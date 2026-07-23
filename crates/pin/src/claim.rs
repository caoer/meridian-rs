//! The `check_claim` evaluator — the claim-site ceiling (23-07 ruling). A
//! declared `check:` names a `def check_claim(t)` starlark predicate that
//! asserts the item's `claim:` over the pinned content. Pin runs it BEFORE the
//! splice; a false (or unevaluable) claim refuses the WHOLE pin atomically — a
//! false claim is not staleness, so re-pinning cannot fix it (refuse-class).
//!
//! # The claim-site surface (23-07 ruling)
//! `t.target` — the pinned item's content; `t.item` — the item's own lock fields
//! as data; `doc(ref)` — the content of another ref declared in the SAME lock
//! (one hop, fail-loud on anything else). The predicate MAY judge; it may never
//! write, reach outward, read the journal, or see the tree — hermetic, exactly
//! like every evaluator run.
//!
//! # Full `EvalLimits`, never fuel alone (`crates/effects/src/kernel.rs` pattern)
//! The load gate ([`effects::validate`]) bounds source size and parse nesting;
//! the metered run bounds fuel (ticks), heap, and call depth. Parse + eval run
//! on a dedicated large stack so pathological nesting cannot overflow the native
//! stack (an uncatchable abort), and a resource-overflow panic is caught and
//! mapped to a fail-closed budget refusal — a bomb terminates, never escapes.

use std::collections::BTreeMap;
use std::panic::AssertUnwindSafe;

use effects::{EvalLimits, Rule};
use starlark::environment::{GlobalsBuilder, Module};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::syntax::{AstModule, Dialect};
use starlark::values::structs::AllocStruct;
use starlark::values::{Heap, Value};

/// The claim-site facts injected as `t` (plus the same-lock `docs` reached by
/// the `doc(ref)` builtin). Every field is content already resolved at the
/// pinned rev — the evaluator sees no tree, no journal, no world.
#[derive(Debug, Clone)]
pub struct ClaimFacts {
    /// `t.target` — the pinned item's own content at the pinned rev.
    pub target: String,
    /// `t.item.ref` — the declared selector.
    pub item_ref: String,
    /// `t.item.to` — the resolved address.
    pub item_to: String,
    /// `t.item.rev` — the pinned rev (empty for a grey declared-only item).
    pub item_rev: String,
    /// `t.item.rev_class` — `content` | `object` | empty (grey).
    pub item_rev_class: String,
    /// `t.item.claim` — the free-text claim being asserted.
    pub item_claim: String,
    /// `doc(ref)` sources — every OTHER ref declared in the same lock mapped to
    /// its content at the pinned rev (one hop; an unknown ref faults loud).
    pub docs: BTreeMap<String, String>,
}

/// The verdict of a `check_claim` run. `Fails` carries the reason — a false
/// return, or the fail-closed fault/budget message — for the refusal render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimOutcome {
    /// The predicate returned truthy — the claim holds; the pin may proceed.
    Holds,
    /// The predicate returned false, or could not be evaluated (fail-closed).
    Fails(String),
}

/// Evaluate a `def check_claim(t)` predicate over `facts` under `limits`. Returns
/// [`ClaimOutcome::Holds`] only when the predicate evaluates cleanly to a truthy
/// value; a false return, a parse/runtime fault, a missing `check_claim`
/// definition, or a budget exhaustion all fail closed to
/// [`ClaimOutcome::Fails`] — a claim the engine cannot confirm true is not
/// pinnable.
#[must_use]
pub fn eval_check_claim(
    predicate_id: &str,
    predicate_src: &str,
    facts: &ClaimFacts,
    limits: EvalLimits,
) -> ClaimOutcome {
    // Load gate: source-size + parse-nesting + parseability (the kernel guards).
    if let Err(e) = effects::validate(&[Rule::new(predicate_id, predicate_src)], limits) {
        return ClaimOutcome::Fails(format!("predicate load refused: {e}"));
    }
    // Metered eval on the large stack (parse re-runs; deep nesting must not
    // overflow the native stack — the kernel::EVAL_STACK_BYTES law, replicated).
    on_eval_stack(|| eval_metered(predicate_id, predicate_src, facts, limits))
}

/// The evaluation thread's stack size — mirrors `effects::kernel::EVAL_STACK_BYTES`
/// (128 MiB virtual, ample over the bounded parse + call-stack worst case).
const EVAL_STACK_BYTES: usize = 128 * 1024 * 1024;

/// Run `f` on a dedicated large-stack thread (parse + eval must not overflow the
/// native stack on pathological nesting — the kernel law, replicated here so pin
/// stays a leaf that does not reach the kernel's `pub(crate)` helper).
fn on_eval_stack<T, F>(f: F) -> T
where
    F: FnOnce() -> T + Send,
    T: Send,
{
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("pin-check-claim".to_owned())
            .stack_size(EVAL_STACK_BYTES)
            .spawn_scoped(scope, f)
            .expect("spawn pin-check-claim thread")
            .join()
            .expect("pin-check-claim thread must not panic (eval catches its own)")
    })
}

/// The metered evaluation: inject `t`, run `check_claim(t)`, read its truthiness
/// under the fuel/heap/call-depth ceilings, with a resource-overflow panic
/// caught and folded to a fail-closed budget refusal.
fn eval_metered(id: &str, src: &str, facts: &ClaimFacts, limits: EvalLimits) -> ClaimOutcome {
    let store = ClaimStore {
        docs: facts.docs.clone(),
    };
    let dialect = Dialect {
        enable_load: false,
        ..Dialect::Standard
    };
    let globals = GlobalsBuilder::standard().with(claim_api).build();
    let step_guard = limits.fuel.max(1);
    let mem_guard = usize::try_from(limits.mem).unwrap_or(usize::MAX).max(1);

    let evaluated = std::panic::catch_unwind(AssertUnwindSafe(|| {
        Module::with_temp_heap(|module| {
            let t = alloc_t(module.heap(), facts);
            let mut eval = Evaluator::new(&module);
            if let Err(e) = eval.set_max_tick_count(step_guard) {
                return ClaimOutcome::Fails(format!("evaluator setup failed: {e}"));
            }
            if let Err(e) = eval.set_max_heap_size(mem_guard) {
                return ClaimOutcome::Fails(format!("evaluator setup failed: {e}"));
            }
            if let Err(e) = eval.set_max_callstack_size(limits.max_call_depth.max(1)) {
                return ClaimOutcome::Fails(format!("evaluator setup failed: {e}"));
            }
            eval.extra = Some(&store);

            let ast = match AstModule::parse(id, src.to_string(), &dialect) {
                Ok(ast) => ast,
                Err(e) => return ClaimOutcome::Fails(format!("predicate parse fault: {e}")),
            };
            if let Err(e) = eval.eval_module(ast, &globals) {
                return ClaimOutcome::Fails(format!("predicate load fault: {e}"));
            }
            let Some(hook) = module.get("check_claim") else {
                return ClaimOutcome::Fails(
                    "predicate defines no `check_claim(t)` function".to_owned(),
                );
            };
            let value = match eval.eval_function(hook, &[t], &[]) {
                Ok(v) => v,
                Err(e) => return ClaimOutcome::Fails(format!("check_claim faulted: {e}")),
            };
            // A non-looping over-allocation completes but still overruns the mem
            // bound — refuse it fail-closed, mirroring the kernel's exact accounting.
            let used_mem = heap_bytes(module.heap());
            if eval.get_total_tick_count() > limits.fuel || used_mem > limits.mem {
                return ClaimOutcome::Fails("check_claim exhausted its budget".to_owned());
            }
            if value.to_bool() {
                ClaimOutcome::Holds
            } else {
                ClaimOutcome::Fails("check_claim asserted false".to_owned())
            }
        })
    }));
    // The only panic reachable inside the metered eval is a resource-overflow
    // assert (a terminated bomb) — fail closed.
    evaluated.unwrap_or_else(|_| ClaimOutcome::Fails("check_claim exhausted its budget".to_owned()))
}

/// Peak eval-heap bytes as `u64` (saturating) — the quantity `set_max_heap_size`
/// guards, monotonic so a post-eval GC cannot deflate it.
fn heap_bytes(heap: Heap<'_>) -> u64 {
    u64::try_from(heap.peak_allocated_bytes()).unwrap_or(u64::MAX)
}

/// Allocate the injected `t`: `target` (string) + `item` (the lock fields as a
/// struct). Same-lock reads go through the `doc(ref)` global, not a `t` field.
fn alloc_t<'v>(heap: Heap<'v>, facts: &ClaimFacts) -> Value<'v> {
    let item = heap.alloc(AllocStruct([
        ("ref", heap.alloc(facts.item_ref.as_str())),
        ("to", heap.alloc(facts.item_to.as_str())),
        ("rev", heap.alloc(facts.item_rev.as_str())),
        ("rev_class", heap.alloc(facts.item_rev_class.as_str())),
        ("claim", heap.alloc(facts.item_claim.as_str())),
    ]));
    heap.alloc(AllocStruct([
        ("target", heap.alloc(facts.target.as_str())),
        ("item", item),
    ]))
}

/// The same-lock content store reached via `eval.extra` by the `doc(ref)`
/// builtin — the one-hop capability, fail-loud on any ref not in the lock.
#[derive(starlark::any::ProvidesStaticType)]
struct ClaimStore {
    docs: BTreeMap<String, String>,
}

/// The claim-site builtin surface — ONLY `doc(ref)` over the same-lock content.
/// Standard globals carry no I/O (research-starlark-mcp §1); this adds one pure
/// reader and nothing else — no write, no world, no journal.
#[starlark_module]
fn claim_api(builder: &mut GlobalsBuilder) {
    /// `doc(ref)` — the content of another ref declared in the SAME lock (one
    /// hop). A ref not in the lock faults loud (never a silent empty read).
    fn doc(
        #[starlark(require = pos)] reference: String,
        eval: &mut Evaluator<'_, '_, '_>,
    ) -> anyhow::Result<String> {
        let store = eval
            .extra
            .and_then(|e| e.downcast_ref::<ClaimStore>())
            .ok_or_else(|| anyhow::anyhow!("check_claim: doc() invoked without a claim store"))?;
        store.docs.get(&reference).cloned().ok_or_else(|| {
            anyhow::anyhow!(
                "check_claim: doc('{reference}') is not a declared lock ref (one hop only)"
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(target: &str) -> ClaimFacts {
        ClaimFacts {
            target: target.to_string(),
            item_ref: "B#Law".to_string(),
            item_to: "B#Law".to_string(),
            item_rev: "deadbeefdeadbeef".to_string(),
            item_rev_class: "content".to_string(),
            item_claim: "the law is present".to_string(),
            docs: BTreeMap::new(),
        }
    }

    #[test]
    fn true_predicate_holds() {
        let src = "def check_claim(t):\n    return 'law' in t.target\n";
        assert_eq!(
            eval_check_claim("A#^c", src, &facts("this law text"), EvalLimits::default()),
            ClaimOutcome::Holds
        );
    }

    #[test]
    fn false_predicate_fails() {
        let src = "def check_claim(t):\n    return 'law' in t.target\n";
        let out = eval_check_claim(
            "A#^c",
            src,
            &facts("no keyword here"),
            EvalLimits::default(),
        );
        assert!(matches!(out, ClaimOutcome::Fails(_)));
    }

    #[test]
    fn item_fields_are_readable() {
        let src = "def check_claim(t):\n    return t.item.rev_class == 'content'\n";
        assert_eq!(
            eval_check_claim("A#^c", src, &facts("x"), EvalLimits::default()),
            ClaimOutcome::Holds
        );
    }

    #[test]
    fn one_hop_doc_reads_same_lock_ref() {
        let mut f = facts("x");
        f.docs
            .insert("C#Body".to_string(), "converged text".to_string());
        let src = "def check_claim(t):\n    return 'converged' in doc('C#Body')\n";
        assert_eq!(
            eval_check_claim("A#^c", src, &f, EvalLimits::default()),
            ClaimOutcome::Holds
        );
    }

    #[test]
    fn doc_of_unknown_ref_fails_loud() {
        let src = "def check_claim(t):\n    return doc('Z#nope') != ''\n";
        let out = eval_check_claim("A#^c", src, &facts("x"), EvalLimits::default());
        assert!(matches!(out, ClaimOutcome::Fails(_)), "unknown ref faults");
    }

    #[test]
    fn missing_definition_fails_closed() {
        let src = "x = 1\n";
        let out = eval_check_claim("A#^c", src, &facts("x"), EvalLimits::default());
        assert!(matches!(out, ClaimOutcome::Fails(_)));
    }
}
