//! The `^expect` evaluation bridge for the `mrd test` scenario runner (U1.2).
//!
//! A scenario's `^expect` block is fenced Starlark defining `def expect(t):`.
//! The runner injects the post-write world as `t` — `t.result` (the write
//! outcome), `t.doc(path)` (a mounted document's text, read AFTER the writes),
//! and `t.journal` (the reserved journal page text) — and calls `expect(t)`.
//! An assertion that does not hold, an unknown `t.` attribute, a runaway loop,
//! or a source-level `DoS` all fault LOUD: the scenario fails, never silently.
//!
//! # Full `EvalLimits`, never fuel alone (§4 preamble law)
//! The block runs under the SAME five-guard envelope the effect kernel enforces:
//! source-size + parse + nesting (reused verbatim through [`effects::validate`],
//! so the harness never forks the tested guard), then tick + heap + call-depth
//! metering on a dedicated large stack with panic containment. A `not not …`
//! chain, a `[[[…` nest, a `while True` loop, or a multi-GiB allocation each
//! terminates as a typed fault — the eval can neither hang nor crash the CLI.
//!
//! # The `t` surface is closed
//! [`Tester`] is a custom Starlark value whose ONLY attributes are `result` /
//! `journal` and whose ONLY method is `doc`. Any other name (`t.reslt`,
//! `t.doc2`) resolves to nothing and Starlark faults LOUD — the harness maps
//! that to a scenario failure naming the missing attribute (taxonomy row 23,
//! `bad_request` on the harness surface).

use std::fmt;
use std::path::{Path, PathBuf};

use effects::{EvalError, EvalLimits, Rule};
use starlark::environment::{
    Globals, GlobalsBuilder, Methods, MethodsBuilder, MethodsStatic, Module,
};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::syntax::{AstModule, Dialect};
use starlark::values::none::NoneType;
use starlark::values::structs::AllocStruct;
use starlark::values::{AllocValue, Heap, StarlarkValue, Value, ValueLike};

use crate::test_cmd::{PutOutcome, confine};

/// The eval-thread stack (mirrors the effect kernel): even with nesting bounded,
/// deep parse frames plus the bounded runtime call-stack exceed a default thread
/// stack, so parse + eval run on a dedicated 128 MiB virtual stack.
const EXPECT_STACK_BYTES: usize = 128 * 1024 * 1024;

/// The limits every `^expect` block runs under — the effect kernel's default
/// envelope (fuel + mem + call-depth + source-size + the cascade cap it ignores).
/// Shared by the [`effects::validate`] pre-guard and the metered eval so the two
/// agree by construction.
fn expect_limits() -> EvalLimits {
    EvalLimits::default()
}

/// Why an `^expect` block did not hold. Every variant is a scenario failure; the
/// message is rendered into the gate report. `Budget` is the terminated-bomb
/// case (a runaway loop / allocation hit the fuel or heap ceiling).
#[derive(Debug)]
pub(crate) enum ExpectError {
    /// The block would not parse, or its nesting/source size tripped the `DoS`
    /// guard (both surface through [`effects::validate`]).
    Source(String),
    /// The block parsed but defines no `def expect(t)` entry.
    NoEntry,
    /// A runaway loop / recursion / allocation hit the fuel or heap ceiling.
    Budget,
    /// The block ran and faulted: a failed `want(...)`, an unknown `t.` attribute
    /// (loud), a `t.doc` mount-escape, a raised error, or a wrong-typed call.
    Failed(String),
}

impl ExpectError {
    fn from_eval(e: EvalError) -> Self {
        match e {
            EvalError::SourceTooLarge { bytes, limit, .. } => ExpectError::Source(format!(
                "^expect source is {bytes} bytes, over the {limit}-byte parse limit"
            )),
            EvalError::Parse { reason, .. } => {
                ExpectError::Source(format!("^expect starlark parse error: {reason}"))
            }
            other => ExpectError::Source(other.to_string()),
        }
    }
}

impl fmt::Display for ExpectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExpectError::Source(m) | ExpectError::Failed(m) => write!(f, "{m}"),
            ExpectError::NoEntry => write!(f, "^expect block defines no `def expect(t)` entry"),
            ExpectError::Budget => write!(
                f,
                "^expect exhausted its budget (fuel/heap ceiling) — evaluation terminated"
            ),
        }
    }
}

/// Run one scenario's `^expect` block against the post-write world. `Ok(())` iff
/// every assertion held; otherwise the typed failure.
pub(crate) fn run_expect(
    source: &str,
    mount: &Path,
    result: &PutOutcome,
    journal: &str,
) -> Result<(), ExpectError> {
    let limits = expect_limits();

    // (1) The DoS pre-guards — source-size + nesting + parse — reused verbatim
    // from the tested effect kernel (`effects::validate`), so the harness runs
    // the SAME guard the door does, never a fork.
    let probe = Rule::new("expect", source.to_string());
    effects::validate(std::slice::from_ref(&probe), limits).map_err(ExpectError::from_eval)?;

    // (2) The metered eval on the dedicated large stack (scoped: the borrowed
    // inputs need no `'static` bound; the thread always joins before return).
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("expect-eval".to_owned())
            .stack_size(EXPECT_STACK_BYTES)
            .spawn_scoped(scope, || {
                eval_expect(source, mount, result, journal, limits)
            })
            .expect("spawn expect-eval thread")
            .join()
            .expect("expect-eval thread must not panic (eval catches its own)")
    })
}

/// The metered `expect(t)` call — mirrors the effect kernel's contract: the three
/// `set_max_*` guards abort a runaway at coarse boundaries, the EXACT post-eval
/// accounting is the authoritative bound, and [`std::panic::catch_unwind`] turns a
/// resource-overflow assert inside the engine into a clean budget fault.
fn eval_expect(
    source: &str,
    mount: &Path,
    result: &PutOutcome,
    journal: &str,
    limits: EvalLimits,
) -> Result<(), ExpectError> {
    let globals = expect_globals();
    let step_guard = limits.fuel.max(1);
    let mem_guard = usize::try_from(limits.mem).unwrap_or(usize::MAX).max(1);

    let evaluated = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        Module::with_temp_heap(|module| {
            let heap = module.heap();
            let tester = heap.alloc(Tester {
                mount: mount.to_path_buf(),
                result_ok: result.ok,
                code: result.code.clone(),
                message: result.message.clone(),
                verdicts: result.verdicts.clone(),
                journal: journal.to_owned(),
            });

            let mut eval = Evaluator::new(&module);
            if let Err(e) = eval.set_max_tick_count(step_guard) {
                return Err(ExpectError::Failed(e.to_string()));
            }
            if let Err(e) = eval.set_max_heap_size(mem_guard) {
                return Err(ExpectError::Failed(e.to_string()));
            }
            if let Err(e) = eval.set_max_callstack_size(limits.max_call_depth.max(1)) {
                return Err(ExpectError::Failed(e.to_string()));
            }

            let ast = match AstModule::parse("expect", source.to_owned(), &expect_dialect()) {
                Ok(ast) => ast,
                Err(e) => return Err(ExpectError::Source(e.to_string())),
            };

            let mut aborted = false;
            let mut missing = false;
            let mut fault: Option<String> = None;
            match eval.eval_module(ast, &globals) {
                Ok(_) => match module.get("expect") {
                    Some(func) => {
                        if let Err(e) = eval.eval_function(func, &[tester], &[]) {
                            aborted = true;
                            fault = Some(e.to_string());
                        }
                    }
                    None => missing = true,
                },
                Err(e) => {
                    aborted = true;
                    fault = Some(e.to_string());
                }
            }

            let used_steps = eval.get_total_tick_count();
            let used_mem = heap_bytes(module.heap());
            drop(eval);

            if missing {
                return Err(ExpectError::NoEntry);
            }
            if aborted {
                if used_steps > limits.fuel || used_mem > limits.mem {
                    return Err(ExpectError::Budget);
                }
                return Err(ExpectError::Failed(fault.unwrap_or_default()));
            }
            Ok(())
        })
    }));

    match evaluated {
        Ok(inner) => inner,
        // The only reachable panic inside the metered eval is a resource-overflow
        // assert (a terminated bomb) — reported as budget, never a crash.
        Err(_panic) => Err(ExpectError::Budget),
    }
}

/// Peak eval-heap bytes as `u64` (saturating) — the memory meter reads (matches
/// the quantity `set_max_heap_size` guards; a post-eval GC cannot deflate it).
fn heap_bytes(heap: Heap<'_>) -> u64 {
    u64::try_from(heap.peak_allocated_bytes()).unwrap_or(u64::MAX)
}

/// The expect dialect: the standard grammar with `load` DISABLED — an `^expect`
/// block is one self-contained `def expect(t)`, never a module loader.
fn expect_dialect() -> Dialect {
    Dialect {
        enable_load: false,
        ..Dialect::Standard
    }
}

/// The closed `^expect` globals: the Starlark standard library plus the `want`
/// assertion verb. No file / net / os / clock — the injected world is `t` alone.
fn expect_globals() -> Globals {
    GlobalsBuilder::standard().with(expect_api).build()
}

/// The assertion verb the `^expect` block reaches for.
#[starlark_module]
fn expect_api(builder: &mut GlobalsBuilder) {
    /// `want(cond, msg = None)` — assert `cond` is truthy. A falsy `cond` fails
    /// the expectation LOUD (the scenario fails), carrying `msg` when given.
    fn want<'v>(
        #[starlark(require = pos)] cond: Value<'v>,
        #[starlark(require = pos)] msg: Option<String>,
    ) -> anyhow::Result<NoneType> {
        if cond.to_bool() {
            Ok(NoneType)
        } else {
            Err(anyhow::anyhow!(
                "want failed: {}",
                msg.unwrap_or_else(|| "expectation not met".to_owned())
            ))
        }
    }
}

/// The injected `t` value: the post-write world a scenario asserts over. A closed
/// custom Starlark value — `result` / `journal` attributes and the `doc` method,
/// and NOTHING else; an unknown attribute faults LOUD (taxonomy row 23).
#[derive(
    Debug, starlark::any::ProvidesStaticType, starlark::values::NoSerialize, allocative::Allocative,
)]
struct Tester {
    /// The scenario's real tmpdir mount — `doc(path)` reads within it, confined.
    #[allocative(skip)]
    mount: PathBuf,
    /// Whether the `^put` sequence committed (`t.result.ok`).
    result_ok: bool,
    /// The refusal code when refused, else empty (`t.result.code`).
    code: String,
    /// The refusal message when refused, else empty (`t.result.message`).
    message: String,
    /// The verdict rule names the write raised (`t.result.verdicts`).
    verdicts: Vec<String>,
    /// The reserved journal page text after the writes (`t.journal`).
    journal: String,
}

impl fmt::Display for Tester {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<tester ok={}>", self.result_ok)
    }
}

#[starlark::values::starlark_value(type = "tester")]
impl<'v> StarlarkValue<'v> for Tester {
    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        match attribute {
            "result" => {
                let verdicts: Vec<Value<'v>> = self
                    .verdicts
                    .iter()
                    .map(|v| heap.alloc(v.as_str()))
                    .collect();
                Some(heap.alloc(AllocStruct([
                    ("ok", heap.alloc(self.result_ok)),
                    ("refused", heap.alloc(!self.result_ok)),
                    ("code", heap.alloc(self.code.as_str())),
                    ("message", heap.alloc(self.message.as_str())),
                    ("verdicts", heap.alloc(verdicts)),
                ])))
            }
            "journal" => Some(heap.alloc(self.journal.as_str())),
            // Any other attribute is outside the closed surface — return None so
            // Starlark faults LOUD ("has no attribute `…`"), never a silent nil.
            _ => None,
        }
    }

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("tester", tester_methods);
        Some(RES.methods())
    }
}

impl<'v> AllocValue<'v> for Tester {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_simple(self)
    }
}

/// `t`'s one method.
#[starlark_module]
fn tester_methods(builder: &mut MethodsBuilder) {
    /// `t.doc(path)` — the mounted document's full text AFTER the writes. `path`
    /// is canonicalized within the mount: an absolute path or a `..`/`.` segment
    /// is REFUSED LOUD (mount confinement, taxonomy row 22), never read outside.
    fn doc<'v>(
        #[starlark(this)] this: Value<'v>,
        #[starlark(require = pos)] path: String,
        heap: Heap<'v>,
    ) -> anyhow::Result<Value<'v>> {
        let tester = this
            .downcast_ref::<Tester>()
            .ok_or_else(|| anyhow::anyhow!("t.doc: receiver is not a tester value"))?;
        let rel = confine(&path).map_err(|e| anyhow::anyhow!("t.doc({path:?}): {e}"))?;
        let full = tester.mount.join(&rel);
        let text = std::fs::read_to_string(&full)
            .map_err(|e| anyhow::anyhow!("t.doc({path:?}): cannot read within the mount: {e}"))?;
        Ok(heap.alloc(text))
    }
}
