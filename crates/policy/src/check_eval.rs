//! Sealed `check_change(change)` evaluator (U1.3) — the CHECK capability's
//! metered predicate runner. [`CheckLimits`] meters tick, heap, call-depth,
//! source-size, and nesting. [`CheckError`] is the typed failure surface.
//! Capability ceiling for CHECK globals is enforced at load; evaluation is
//! pure over the injected change facts.

use std::cell::RefCell;

use starlark::environment::{Globals, GlobalsBuilder, Module};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::syntax::{AstModule, Dialect};
use starlark::values::Heap;
use starlark::values::Value;
use starlark::values::dict::AllocDict;
use starlark::values::none::NoneType;
use starlark::values::structs::AllocStruct;

use crate::change::{Change, DocFacts, Edge, EditFact, NodeFact, TargetFact};
use crate::declaration::Refusal;

/// Deterministic bounds on one `check_change` evaluation — the full limit set:
/// tick + heap + call-depth + source-size, plus the parser nesting cap
/// ([`MAX_NESTING_DEPTH`]). Distinct from [`crate::EvalBudget`] (the `@1`
/// `{steps, mem}` load-gate budget) — the `@2` door surface meters all five
/// guards, never fuel alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckLimits {
    /// Max Starlark step (tick) count.
    pub fuel: u64,
    /// Max eval-heap bytes.
    pub mem: u64,
    /// Max Starlark call-stack depth (recursion-bomb guard).
    pub max_call_depth: usize,
    /// Max CHECK source length in bytes (parse `DoS` guard).
    pub max_source_bytes: usize,
}

impl Default for CheckLimits {
    fn default() -> Self {
        Self {
            fuel: 1_000_000,
            mem: 64 * 1024 * 1024,
            max_call_depth: 1000,
            // 64 KiB is generous for one CHECK page and stays within the
            // parse-recursion budget the fixed evaluation stack provides.
            max_source_bytes: 64 * 1024,
        }
    }
}

/// Max parser NESTING depth — brackets `([{` or a run of consecutive unary
/// operators (`not`, `-`, `+`, `~`). The Starlark recursive-descent parser recurses
/// once per nesting level; a deep chain overflows the native stack and ABORTS the
/// process (`catch_unwind` cannot catch a stack overflow), so it must be PREVENTED.
/// Bounding nesting BEFORE parsing is O(1) stack. 500 is orders of magnitude beyond
/// any real CHECK (nesting depth, not total size).
const MAX_NESTING_DEPTH: usize = 500;

/// The evaluation thread's stack size — a dedicated large stack so pathologically
/// nested source cannot overflow the native stack and abort the process (mirrors
/// `effects::kernel::EVAL_STACK_BYTES`).
const EVAL_STACK_BYTES: usize = 128 * 1024 * 1024;

/// Why a `check_change` evaluation did not produce a verdict. Every hostile input —
/// a runaway bomb, malformed Starlark, an over-long or over-nested source, a
/// missing `check_change`, a reach for a name outside the ceiling — lands as one of
/// these typed errors; nothing panics, nothing hangs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckError {
    /// The predicate exhausted its `{fuel, mem}` budget (loop, recursion, or
    /// allocation). Terminated, never hung.
    Budget { fuel: u64, mem: u64 },
    /// The CHECK source would not parse as Starlark.
    Parse { reason: String },
    /// The CHECK parsed but faulted at eval: a raised error, a wrong-typed argument
    /// to `refuse`, an out-of-range index, or a reach for a name the ceiling does
    /// not grant (`set_field`, `send`, `open`, `now`, …).
    Runtime { reason: String },
    /// The CHECK source exceeded [`CheckLimits::max_source_bytes`] — refused before
    /// parsing (parse `DoS` guard).
    SourceTooLarge { bytes: usize, limit: usize },
    /// The source parsed and evaluated but defines no `check_change(change)` entry.
    MissingEntry,
}

impl std::fmt::Display for CheckError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CheckError::Budget { fuel, mem } => write!(
                f,
                "CHECK budget exhausted (steps>{fuel} or mem>{mem} bytes) — evaluation terminated"
            ),
            CheckError::Parse { reason } => write!(f, "CHECK starlark parse error: {reason}"),
            CheckError::Runtime { reason } => write!(f, "CHECK evaluation error: {reason}"),
            CheckError::SourceTooLarge { bytes, limit } => write!(
                f,
                "CHECK source is {bytes} bytes, over the {limit}-byte parse limit"
            ),
            CheckError::MissingEntry => {
                f.write_str("CHECK defines no `check_change(change)` predicate")
            }
        }
    }
}

impl std::error::Error for CheckError {}

/// One `check_change` evaluation's metered outcome: the [`Refusal`]s it emitted
/// plus the EXACT fuel (Starlark ticks) and peak eval-heap bytes it spent. The
/// `test --corpus` tier (U1.5) reads these for its fuel + heap p50/p99 profile;
/// the plain [`run_check_change`] path drops the telemetry. Mirrors the effect
/// kernel's `RuleTelemetry` so the two harness tiers report the same shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckTelemetry {
    /// The refusals the CHECK emitted, in emission order (empty ⇒ the change passes).
    pub refusals: Vec<Refusal>,
    /// Exact Starlark ticks the evaluation spent.
    pub fuel_used: u64,
    /// Peak eval-heap bytes the evaluation spent.
    pub mem_used: u64,
}

/// Run `f` on a dedicated large-stack thread and return its result — the parse +
/// eval must run here so pathologically nested source cannot overflow the native
/// stack and abort the process. Scoped, so borrowed data needs no `'static` bound;
/// the thread always joins before return.
pub(crate) fn on_eval_stack<T, F>(f: F) -> T
where
    F: FnOnce() -> T + Send,
    T: Send,
{
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("check-eval".to_owned())
            .stack_size(EVAL_STACK_BYTES)
            .spawn_scoped(scope, f)
            .expect("spawn check-eval thread")
            .join()
            .expect("check-eval thread must not panic (run_check_change catches its own)")
    })
}

/// The CHECK dialect: the standard grammar with `load` DISABLED — a CHECK is a
/// single self-contained `check_change` definition; module loading would let it
/// pull external symbols, a capability the ceiling does not grant.
pub(crate) fn check_dialect() -> Dialect {
    Dialect {
        enable_load: false,
        ..Dialect::Standard
    }
}

/// The CHECK ceiling globals: the Starlark standard library plus ONLY `refuse`.
/// This is the ENTIRE capability surface a CHECK sees — no write, no outward reach.
pub(crate) fn check_globals() -> Globals {
    GlobalsBuilder::standard().with(check_api).build()
}

/// Parse-check a CHECK source without running it — the load gate. Applies the
/// source-size and nesting guards, then parses under the CHECK dialect, so a load
/// separates authoring faults (over-long, over-nested, unparseable) from per-change
/// faults.
///
/// # Errors
/// [`CheckError::SourceTooLarge`], [`CheckError::Parse`] (over-nested or
/// unparseable).
pub(crate) fn validate_check_source(source: &str, limits: CheckLimits) -> Result<(), CheckError> {
    on_eval_stack(|| {
        check_source_size(source, limits)?;
        check_nesting_depth(source)?;
        AstModule::parse("CHECK.md", source.to_owned(), &check_dialect())
            .map(|_| ())
            .map_err(|e| CheckError::Parse {
                reason: e.to_string(),
            })
    })
}

/// Run a CHECK's `check_change(change)` over the injected [`Change`] under the FULL
/// limits, returning the [`Refusal`]s it emitted (empty ⇒ the change passes).
///
/// Metering mirrors the `effects` kernel: Starlark's own `set_max_*` guards abort a
/// runaway loop/recursion at coarse boundaries, the EXACT post-eval accounting
/// (`get_total_tick_count`, peak heap bytes) is the authoritative bound, and
/// [`std::panic::catch_unwind`] converts a resource-overflow panic inside the engine
/// into a clean [`CheckError::Budget`] — so a bomb terminates, never crashes.
///
/// # Errors
/// [`CheckError`] — see its variants.
pub(crate) fn run_check_change(
    source: &str,
    change: &Change,
    limits: CheckLimits,
) -> Result<Vec<Refusal>, CheckError> {
    on_eval_stack(|| eval_check(source, change, limits)).map(|t| t.refusals)
}

/// Run a CHECK's `check_change(change)` under the FULL limits and return the
/// [`CheckTelemetry`] — the [`Refusal`]s AND the exact fuel/heap the evaluation
/// spent. Same metered core as [`run_check_change`]; this variant keeps the
/// telemetry the corpus tier's p50/p99 profile needs.
///
/// # Errors
/// [`CheckError`] — see its variants.
pub(crate) fn run_check_change_metered(
    source: &str,
    change: &Change,
    limits: CheckLimits,
) -> Result<CheckTelemetry, CheckError> {
    on_eval_stack(|| eval_check(source, change, limits))
}

/// The metered evaluation body (runs on the large stack via [`run_check_change`]).
fn eval_check(
    source: &str,
    change: &Change,
    limits: CheckLimits,
) -> Result<CheckTelemetry, CheckError> {
    check_source_size(source, limits)?;
    check_nesting_depth(source)?;

    let globals = check_globals();
    let store = EmitStore {
        refusals: RefCell::new(Vec::new()),
    };
    let step_guard = limits.fuel.max(1);
    let mem_guard = usize::try_from(limits.mem).unwrap_or(usize::MAX).max(1);

    // AssertUnwindSafe: on panic we discard `store` unread and return a budget
    // fault, so its interior mutability cannot leak an inconsistent value across the
    // boundary.
    let evaluated = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        Module::with_temp_heap(|module| {
            let heap = module.heap();
            let change_value = alloc_change(heap, change);

            let mut eval = Evaluator::new(&module);
            // GC OFF, load-bearing (mw-splice-engine-eof, 2026-08-18):
            // `change_value` lives in a Rust local across `eval_module` and is
            // NOT a GC root — a GC fired at a top-level statement boundary
            // (heap ≥ starlark's 100KB threshold, which a large doc's change
            // surface crosses alone) collects it, and `eval_function` then
            // segfaults on the dangling Value. `set_max_heap_size` still caps
            // the arena; the temp heap drops whole at scope end.
            eval.disable_gc();
            if let Err(e) = eval.set_max_tick_count(step_guard) {
                return Err(runtime(e));
            }
            if let Err(e) = eval.set_max_heap_size(mem_guard) {
                return Err(runtime(e));
            }
            // Bound recursion DEPTH: unbounded Starlark recursion consumes native
            // frames and would overflow even the large evaluation stack before the
            // tick guard fires.
            if let Err(e) = eval.set_max_callstack_size(limits.max_call_depth.max(1)) {
                return Err(runtime(e));
            }
            eval.extra = Some(&store);

            let ast = match AstModule::parse("CHECK.md", source.to_owned(), &check_dialect()) {
                Ok(ast) => ast,
                Err(e) => {
                    return Err(CheckError::Parse {
                        reason: e.to_string(),
                    });
                }
            };

            let mut aborted = false;
            let mut depth_overflow = false;
            let mut fault: Option<String> = None;
            let mut missing = false;
            match eval.eval_module(ast, &globals) {
                Ok(_) => match module.get("check_change") {
                    Some(hook) => {
                        if let Err(e) = eval.eval_function(hook, &[change_value], &[]) {
                            aborted = true;
                            depth_overflow = is_depth_overflow(&e);
                            fault = Some(e.to_string());
                        }
                    }
                    None => missing = true,
                },
                Err(e) => {
                    aborted = true;
                    depth_overflow = is_depth_overflow(&e);
                    fault = Some(e.to_string());
                }
            }

            let used_steps = eval.get_total_tick_count();
            let used_mem = heap_bytes(module.heap());
            drop(eval);
            let refusals = store.refusals.take();

            let over_budget = used_steps > limits.fuel || used_mem > limits.mem;
            if missing {
                Err(CheckError::MissingEntry)
            } else if aborted {
                // A resource limit (⇒ budget) or a genuine fault? The coarse
                // tick/heap guards are visible in the exact accounting; the
                // callstack-depth guard surfaces as a typed StackOverflow — either
                // is a terminated bomb, not a CHECK fault.
                if over_budget || depth_overflow {
                    Err(budget(limits))
                } else {
                    Err(CheckError::Runtime {
                        reason: fault.unwrap_or_default(),
                    })
                }
            } else if over_budget {
                // A non-looping allocation can complete WITHOUT aborting yet still
                // overrun the exact mem bound — refuse it as budget.
                Err(budget(limits))
            } else {
                Ok(CheckTelemetry {
                    refusals,
                    fuel_used: used_steps,
                    mem_used: used_mem,
                })
            }
        })
    }));

    match evaluated {
        Ok(inner) => inner,
        // The only panic reachable inside the metered eval is a resource-overflow
        // assert (a multi-GiB allocation tripping a `len overflow`): a terminated
        // bomb, reported as budget — never a crash.
        Err(_panic) => Err(budget(limits)),
    }
}

/// Whether a Starlark eval error is a call-stack-depth overflow (the recursion
/// guard tripping) — classified as budget, not a CHECK fault.
pub(crate) fn is_depth_overflow(e: &starlark::Error) -> bool {
    matches!(e.kind(), starlark::ErrorKind::StackOverflow(_))
}

/// The budget-exhaustion error for these limits.
pub(crate) fn budget(limits: CheckLimits) -> CheckError {
    CheckError::Budget {
        fuel: limits.fuel,
        mem: limits.mem,
    }
}

/// A runtime fault carrying the engine's message.
pub(crate) fn runtime(e: impl std::fmt::Display) -> CheckError {
    CheckError::Runtime {
        reason: e.to_string(),
    }
}

/// Refuse a CHECK whose source exceeds the parse-DoS byte cap before it reaches the
/// parser.
pub(crate) fn check_source_size(source: &str, limits: CheckLimits) -> Result<(), CheckError> {
    if source.len() > limits.max_source_bytes {
        return Err(CheckError::SourceTooLarge {
            bytes: source.len(),
            limit: limits.max_source_bytes,
        });
    }
    Ok(())
}

/// Refuse a CHECK whose nesting depth would recurse the parser past
/// [`MAX_NESTING_DEPTH`] and overflow the native stack (an uncatchable abort). A
/// single conservative left-to-right byte scan that skips string- and
/// comment-content and tracks bracket-nesting depth and the length of a consecutive
/// unary-operator run (the deep `not not …` vector). Over-approximation is safe: it
/// can only reject the pathological, never a real shallow CHECK.
pub(crate) fn check_nesting_depth(source: &str) -> Result<(), CheckError> {
    let bytes = source.as_bytes();
    let n = bytes.len();
    let mut i = 0;
    let mut bracket_depth: usize = 0;
    let mut unary_run: usize = 0;
    let over = || CheckError::Parse {
        reason: format!("source nesting depth exceeds {MAX_NESTING_DEPTH} (parse-recursion guard)"),
    };
    while i < n {
        let c = bytes[i];
        match c {
            // Line comment: skip to end of line.
            b'#' => {
                while i < n && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            // String literal (single/double, possibly triple-quoted): skip its
            // content so nothing inside counts.
            b'"' | b'\'' => {
                i = skip_string(bytes, i);
                unary_run = 0;
            }
            b'(' | b'[' | b'{' => {
                bracket_depth += 1;
                if bracket_depth > MAX_NESTING_DEPTH {
                    return Err(over());
                }
                unary_run = 0;
                i += 1;
            }
            b')' | b']' | b'}' => {
                bracket_depth = bracket_depth.saturating_sub(1);
                unary_run = 0;
                i += 1;
            }
            b'-' | b'+' | b'~' => {
                unary_run += 1;
                if unary_run > MAX_NESTING_DEPTH {
                    return Err(over());
                }
                i += 1;
            }
            _ if c.is_ascii_whitespace() => i += 1,
            b'n' if word_at(bytes, i, b"not") => {
                unary_run += 1;
                if unary_run > MAX_NESTING_DEPTH {
                    return Err(over());
                }
                i += 3;
            }
            _ if c.is_ascii_alphanumeric() || c == b'_' => {
                // An operand ends a unary run; consume the whole word/number.
                while i < n && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                unary_run = 0;
            }
            _ => {
                unary_run = 0;
                i += 1;
            }
        }
    }
    Ok(())
}

/// Skip a string literal starting at `open` (`bytes[open]` is a quote). Returns the
/// index just past the closing quote (or end-of-input on an unterminated literal).
fn skip_string(bytes: &[u8], open: usize) -> usize {
    let quote = bytes[open];
    let n = bytes.len();
    let triple = open + 2 < n && bytes[open + 1] == quote && bytes[open + 2] == quote;
    let mut i = open + if triple { 3 } else { 1 };
    while i < n {
        match bytes[i] {
            b'\\' => i += 2, // escape: skip the next byte
            b if b == quote => {
                if triple {
                    if i + 2 < n && bytes[i + 1] == quote && bytes[i + 2] == quote {
                        return i + 3;
                    }
                    i += 1;
                } else {
                    return i + 1;
                }
            }
            b'\n' if !triple => return i, // unterminated single-line string
            _ => i += 1,
        }
    }
    n
}

/// True if the word `needle` sits at `bytes[i..]` with a non-identifier boundary on
/// each side (so `not` matches the keyword, not the start of `notes`).
fn word_at(bytes: &[u8], i: usize, needle: &[u8]) -> bool {
    if bytes.len() - i < needle.len() || &bytes[i..i + needle.len()] != needle {
        return false;
    }
    let before_ok = i == 0 || !is_word_byte(bytes[i - 1]);
    let after = i + needle.len();
    let after_ok = after >= bytes.len() || !is_word_byte(bytes[after]);
    before_ok && after_ok
}

/// Whether a byte can be part of an identifier.
fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Peak eval-heap bytes as `u64` (saturating). Peak (not current) is monotonic and
/// matches the quantity `set_max_heap_size` guards.
pub(crate) fn heap_bytes(heap: Heap<'_>) -> u64 {
    u64::try_from(heap.peak_allocated_bytes()).unwrap_or(u64::MAX)
}

// ── The refuse builtin (the CHECK ceiling's ONLY power) ───────────────────────

/// The store the `refuse` builtin records into, reached via `Evaluator::extra`.
#[derive(starlark::any::ProvidesStaticType)]
struct EmitStore {
    refusals: RefCell<Vec<Refusal>>,
}

/// The CHECK builtins — the ENTIRE rule-language surface a CHECK sees beyond the
/// Starlark standard library. `refuse` is the ONLY power: it records a finding, and
/// it STRUCTURALLY requires the legal path (`passing`), so no refusal can exist
/// without citing the passing scenario (rulings § refusals cite the passing
/// scenario). Every argument is named; nothing here writes or reaches outward.
#[starlark_module]
fn check_api(builder: &mut GlobalsBuilder) {
    /// `refuse(message, passing)` — record one refusal. `message` teaches what is
    /// wrong; `passing` cites the passing scenario (the legal path), REQUIRED so a
    /// refusal always points at the way to do it right.
    fn refuse(
        #[starlark(require = named)] message: String,
        #[starlark(require = named)] passing: String,
        eval: &mut Evaluator<'_, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let store = eval
            .extra
            .and_then(|e| e.downcast_ref::<EmitStore>())
            .ok_or_else(|| anyhow::anyhow!("check-api: refuse() invoked without an emit store"))?;
        store.refusals.borrow_mut().push(Refusal {
            message,
            passing_scenario: passing,
        });
        Ok(NoneType)
    }
}

// ── Change injection (the rulepack-api@2 surface) ─────────────────────────────

/// An optional string as a Starlark value: the string, or `None`.
pub(crate) fn opt_str<'v>(heap: Heap<'v>, o: Option<&str>) -> Value<'v> {
    match o {
        Some(s) => heap.alloc(s),
        None => Value::new_none(),
    }
}

/// Allocate the injected `change` value: the full `@2` change surface as a Starlark
/// struct. Every field is a pure state/evidence fact ([`crate::CHANGE_FACT_VOCAB`]).
fn alloc_change<'v>(heap: Heap<'v>, change: &Change) -> Value<'v> {
    let targets: Vec<Value<'v>> = change
        .targets
        .iter()
        .map(|t| alloc_target(heap, t))
        .collect();
    let edits: Vec<Value<'v>> = change.edits.iter().map(|e| alloc_edit(heap, e)).collect();
    let fields: Vec<Value<'v>> = change
        .fields_changed
        .iter()
        .map(|s| heap.alloc(s.as_str()))
        .collect();
    let sections: Vec<Value<'v>> = change
        .sections_changed
        .iter()
        .map(|hpath| {
            let segs: Vec<Value<'v>> = hpath.iter().map(|s| heap.alloc(s.as_str())).collect();
            heap.alloc(segs)
        })
        .collect();
    heap.alloc(AllocStruct([
        ("op", heap.alloc(change.op.as_str())),
        ("targets", heap.alloc(targets)),
        ("edits", heap.alloc(edits)),
        ("actor", opt_str(heap, change.actor.as_deref())),
        ("force", heap.alloc(change.force)),
        ("before", alloc_docfacts(heap, &change.before)),
        ("doc", alloc_docfacts(heap, &change.doc)),
        ("fields_changed", heap.alloc(fields)),
        ("sections_changed", heap.alloc(sections)),
    ]))
}

/// Allocate one `DocFacts` value: `{path, nodes, frontmatter, edges}`.
pub(crate) fn alloc_docfacts<'v>(heap: Heap<'v>, facts: &DocFacts) -> Value<'v> {
    let nodes: Vec<Value<'v>> = facts.nodes.iter().map(|n| alloc_node(heap, n)).collect();
    let frontmatter = heap.alloc(AllocDict(
        facts
            .frontmatter
            .iter()
            .map(|(k, v)| (heap.alloc(k.as_str()), heap.alloc(v.as_str()))),
    ));
    let edges: Vec<Value<'v>> = facts.edges.iter().map(|e| alloc_edge(heap, e)).collect();
    heap.alloc(AllocStruct([
        ("path", heap.alloc(facts.path.as_str())),
        ("nodes", heap.alloc(nodes)),
        ("frontmatter", frontmatter),
        ("edges", heap.alloc(edges)),
    ]))
}

/// Allocate one injected node struct.
fn alloc_node<'v>(heap: Heap<'v>, node: &NodeFact) -> Value<'v> {
    let hpath: Vec<Value<'v>> = node.hpath.iter().map(|s| heap.alloc(s.as_str())).collect();
    heap.alloc(AllocStruct([
        ("kind", heap.alloc(node.kind)),
        ("level", heap.alloc(node.level)),
        ("text", heap.alloc(node.text.as_str())),
        ("span", heap.alloc((node.span.0, node.span.1))),
        ("node_rev", heap.alloc(node.node_rev.as_str())),
        ("hpath", heap.alloc(hpath)),
    ]))
}

/// Allocate one resolved target fact `{reference, node_rev}` (`node_rev` is `None`
/// when the target was absent before the write — a create).
pub(crate) fn alloc_target<'v>(heap: Heap<'v>, target: &TargetFact) -> Value<'v> {
    heap.alloc(AllocStruct([
        ("reference", heap.alloc(target.reference.as_str())),
        ("node_rev", opt_str(heap, target.node_rev.as_deref())),
    ]))
}

/// Allocate one edit op-shape fact `{target, kind, at}`.
pub(crate) fn alloc_edit<'v>(heap: Heap<'v>, edit: &EditFact) -> Value<'v> {
    let at = match edit.at {
        Some(a) => heap.alloc(a),
        None => Value::new_none(),
    };
    heap.alloc(AllocStruct([
        ("target", heap.alloc(edit.target.as_str())),
        ("kind", heap.alloc(edit.kind)),
        ("at", at),
    ]))
}

/// Allocate one edge `{reference, pinned_rev, current_rev, claim, drifted,
/// target_facts}`. A fresh edge carries its one hop of `target_facts`; a drifted or
/// unresolved edge fails closed — `target_facts` is `None` (rulings one-hop law).
fn alloc_edge<'v>(heap: Heap<'v>, edge: &Edge) -> Value<'v> {
    let target_facts = match &edge.target_facts {
        Some(facts) => alloc_docfacts(heap, facts),
        None => Value::new_none(),
    };
    heap.alloc(AllocStruct([
        ("reference", heap.alloc(edge.reference.as_str())),
        ("pinned_rev", heap.alloc(edge.pinned_rev.as_str())),
        ("current_rev", opt_str(heap, edge.current_rev.as_deref())),
        ("claim", opt_str(heap, edge.claim.as_deref())),
        ("drifted", heap.alloc(edge.drifted)),
        ("target_facts", target_facts),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::change::{ChangeOp, Invocation, derive_change};
    use model::{Document, NodeKind};
    use std::collections::HashSet;

    /// Build a real `model::Document` from markdown, stamping its path (what the
    /// disk edge does; `model::build` leaves the path empty).
    fn doc_of(path: &str, md: &str) -> Document {
        let nodes = syntax::parse(md);
        let mut doc = model::build(md.to_string(), nodes);
        if let NodeKind::Document { path: p, .. } = &mut doc.root.kind {
            *p = path.to_string();
        }
        doc
    }

    fn no_edges(_: &str) -> Option<(String, Document)> {
        None
    }

    /// A self-close change: `owner` is `actor`. Fires the reviewer-not-owner CHECK.
    fn self_close_change(actor: &str) -> Change {
        let before = doc_of(
            "tasks/t.md",
            "---\nowner: agent:alice\nstatus: open\n---\n# T\n\nx\n",
        );
        let after = doc_of(
            "tasks/t.md",
            "---\nowner: agent:alice\nstatus: closed\n---\n# T\n\nx\n",
        );
        derive_change(
            &before,
            &after,
            &[],
            Invocation {
                op: ChangeOp::Splice,
                actor: Some(actor),
                force: false,
            },
            &[],
            &no_edges,
        )
    }

    const REVIEWER_NOT_OWNER: &str = "\
def check_change(change):
    owner = change.doc.frontmatter.get(\"owner\")
    actor = change.actor
    if actor != None and owner != None and actor == owner:
        refuse(
            message = \"reviewer must not be the owner: \" + actor,
            passing = \"scenarios/reviewer-close.md\",
        )
";

    /// Regression (mw-splice-engine-eof, 2026-08-18): a change surface whose
    /// temp-heap allocation crosses starlark's GC threshold (100KB) must
    /// survive evaluation. Pre-fix, `eval_module` fired a GC, collected the
    /// change Value held in a Rust local (not a GC root), and `eval_function`
    /// SEGFAULTED the process — this test dies with SIGSEGV on the unfixed
    /// evaluator.
    #[test]
    fn a_change_past_the_gc_threshold_survives_evaluation() {
        use std::fmt::Write as _;
        let mut md = String::from("---\nowner: alice\nstatus: open\n---\n# Big\n\n");
        for i in 0..500 {
            let _ = write!(
                md,
                "## Section {i:03}\n\nThe hook plane unifies the four trigger families under \
                 one armed row shape; each family keeps its own firing clock but shares the \
                 severity ladder and the receipts contract. Row {i} carries its own receipts \
                 line.\n\n"
            );
        }
        let before = doc_of("tasks/big.md", &md);
        let after = doc_of(
            "tasks/big.md",
            &md.replace("status: open", "status: review"),
        );
        let change = derive_change(
            &before,
            &after,
            &[],
            Invocation {
                op: ChangeOp::Splice,
                actor: Some("agent:770a48c2"),
                force: false,
            },
            &[],
            &no_edges,
        );
        let refusals = run_check_change(
            "def check_change(change):\n    if change.doc.frontmatter.get(\"status\") == \"review\" and change.actor == change.doc.frontmatter.get(\"owner\"):\n        refuse(message = \"self-review\", passing = \"scenarios/reviewer-close.md\")\n",
            &change,
            CheckLimits::default(),
        )
        .expect("a big change surface evaluates cleanly");
        assert!(
            refusals.is_empty(),
            "actor is not the owner — the CHECK passes: {refusals:?}"
        );
    }

    #[test]
    fn ceiling_globals_are_standard_plus_refuse_only() {
        // The capability ceiling IS the globals: `refuse` is present; no write /
        // outward-reach / io name is — enforced at the source of truth.
        let names: HashSet<String> = check_globals()
            .names()
            .map(|n| n.as_str().to_owned())
            .collect();
        assert!(names.contains("refuse"), "the CHECK ceiling grants refuse");
        for forbidden in [
            "set_field",
            "append_section",
            "send",
            "remind",
            "ask",
            "notice",
            "warn",
            "refresh_view",
            "open",
            "read",
            "write",
            "os",
            "load",
            "print",
            "now",
        ] {
            assert!(
                !names.contains(forbidden),
                "CHECK ceiling must not expose `{forbidden}`"
            );
        }
    }

    #[test]
    fn self_close_fires_with_a_refusal_citing_the_passing_scenario() {
        let change = self_close_change("agent:alice");
        let refusals =
            run_check_change(REVIEWER_NOT_OWNER, &change, CheckLimits::default()).unwrap();
        assert_eq!(refusals.len(), 1, "owner closing their own task fires");
        assert!(refusals[0].message.contains("agent:alice"));
        assert_eq!(
            refusals[0].passing_scenario, "scenarios/reviewer-close.md",
            "the refusal cites the passing scenario"
        );
    }

    #[test]
    fn metered_run_reports_fuel_and_is_deterministic() {
        // The metered path returns the same refusals AND the exact fuel/heap the
        // eval spent; a real eval spends fuel, and replaying is byte-identical.
        let change = self_close_change("agent:alice");
        let a = run_check_change_metered(REVIEWER_NOT_OWNER, &change, CheckLimits::default())
            .expect("metered run");
        let b = run_check_change_metered(REVIEWER_NOT_OWNER, &change, CheckLimits::default())
            .expect("metered run");
        assert_eq!(a.refusals.len(), 1, "owner self-close fires");
        assert!(a.fuel_used > 0, "a CHECK that ran spent fuel");
        assert_eq!(a.fuel_used, b.fuel_used, "fuel is deterministic");
        assert_eq!(a.mem_used, b.mem_used, "heap is deterministic");
        // Telemetry refusals match the plain path.
        let plain = run_check_change(REVIEWER_NOT_OWNER, &change, CheckLimits::default()).unwrap();
        assert_eq!(a.refusals, plain, "metered refusals match the plain path");
    }

    #[test]
    fn reviewer_close_passes() {
        // A distinct reviewer (actor != owner) produces no refusal.
        let change = self_close_change("agent:bob");
        let refusals =
            run_check_change(REVIEWER_NOT_OWNER, &change, CheckLimits::default()).unwrap();
        assert!(refusals.is_empty(), "reviewer != owner passes");
    }

    #[test]
    fn reaching_outside_the_ceiling_faults() {
        // A CHECK that names a write/outward builtin reaches an unbound name — the
        // ceiling is enforced, the reach faults (never silently no-ops).
        let src = "def check_change(change):\n    send(to = [\"x\"], message = \"m\")\n";
        let change = self_close_change("agent:alice");
        assert!(matches!(
            run_check_change(src, &change, CheckLimits::default()),
            Err(CheckError::Runtime { .. })
        ));
    }

    #[test]
    fn refuse_requires_the_passing_citation() {
        // `passing` is a required named arg — a refuse() without it faults, so no
        // refusal can exist without citing the legal path.
        let src = "def check_change(change):\n    refuse(message = \"no citation\")\n";
        let change = self_close_change("agent:alice");
        assert!(matches!(
            run_check_change(src, &change, CheckLimits::default()),
            Err(CheckError::Runtime { .. })
        ));
    }

    #[test]
    fn missing_check_change_is_typed() {
        let src = "x = 1\n";
        let change = self_close_change("agent:alice");
        assert!(matches!(
            run_check_change(src, &change, CheckLimits::default()),
            Err(CheckError::MissingEntry)
        ));
    }

    #[test]
    fn runaway_loop_is_budget_not_a_hang() {
        let src = "def check_change(change):\n    for _ in range(10000000):\n        pass\n";
        let change = self_close_change("agent:alice");
        let limits = CheckLimits {
            fuel: 10_000,
            ..CheckLimits::default()
        };
        assert!(matches!(
            run_check_change(src, &change, limits),
            Err(CheckError::Budget { .. })
        ));
    }

    #[test]
    fn recursion_bomb_is_budget() {
        let src = "def check_change(change):\n    check_change(change)\n";
        let change = self_close_change("agent:alice");
        let limits = CheckLimits {
            max_call_depth: 64,
            ..CheckLimits::default()
        };
        assert!(matches!(
            run_check_change(src, &change, limits),
            Err(CheckError::Budget { .. })
        ));
    }

    #[test]
    fn over_long_source_is_refused_before_parse() {
        let src = format!(
            "# {}\ndef check_change(change):\n    pass\n",
            "x".repeat(200)
        );
        let change = self_close_change("agent:alice");
        let limits = CheckLimits {
            max_source_bytes: 64,
            ..CheckLimits::default()
        };
        assert!(matches!(
            run_check_change(&src, &change, limits),
            Err(CheckError::SourceTooLarge { .. })
        ));
    }

    #[test]
    fn deeply_nested_source_is_refused() {
        let src = format!(
            "def check_change(change):\n    x = {}{}\n",
            "(".repeat(MAX_NESTING_DEPTH + 1),
            ")".repeat(MAX_NESTING_DEPTH + 1)
        );
        let change = self_close_change("agent:alice");
        assert!(matches!(
            run_check_change(&src, &change, CheckLimits::default()),
            Err(CheckError::Parse { .. })
        ));
    }

    #[test]
    fn validate_accepts_a_real_check_and_rejects_garbage() {
        assert!(validate_check_source(REVIEWER_NOT_OWNER, CheckLimits::default()).is_ok());
        assert!(matches!(
            validate_check_source("def check_change(:\n", CheckLimits::default()),
            Err(CheckError::Parse { .. })
        ));
    }

    #[test]
    fn full_change_surface_is_readable() {
        // A CHECK that reads op/force/fields_changed/edits/targets/before does not
        // fault — the full @2 surface is injected, not just actor/doc.
        let src = "\
def check_change(change):
    seen = change.op + str(change.force) + str(len(change.fields_changed))
    seen = seen + str(len(change.edits)) + str(len(change.targets))
    seen = seen + change.before.path + str(len(change.doc.nodes))
    if change.op == \"splice\" and \"status\" in change.fields_changed:
        refuse(message = seen, passing = \"scenarios/ok.md\")
";
        let change = self_close_change("agent:alice");
        let refusals = run_check_change(src, &change, CheckLimits::default()).unwrap();
        assert_eq!(refusals.len(), 1, "the full surface is readable and fires");
    }
}
