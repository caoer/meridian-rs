//! Sealed Starlark evaluation bridge: closed globals (stdlib + effect-descriptor
//! builtins, no ambient I/O), inject event/`ctx`, run metered under fuel/mem,
//! record constructed descriptors. `pub(crate)`; public surface is `lib.rs`.
//!
//! Metering mirrors `policy`: `eval.extra` holds the emit store;
//! `set_max_tick_count` / `set_max_heap_size` with exact post-eval accounting;
//! [`std::panic::catch_unwind`] maps resource-overflow panics to budget errors
//! so a bomb cannot crash the caller.
//!
//! # Purity
//! Globals are `GlobalsBuilder::standard().with(effect_api)`. Stdlib has no
//! file/net/os/clock/random; builtins construct inert descriptors only. Unbound
//! names (`open`, `print`, …) fault as [`EvalError::Runtime`].

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};

use starlark::collections::SmallMap;
use starlark::environment::{FrozenModule, Globals, GlobalsBuilder, Module};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::syntax::{AstModule, Dialect};
use starlark::values::Heap;
use starlark::values::UnpackValue;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::values::dict::AllocDict;
use starlark::values::function::FUNCTION_TYPE;
use starlark::values::list::UnpackList;
use starlark::values::none::NoneType;
use starlark::values::structs::AllocStruct;
use starlark_syntax::syntax::ast::{AssignTarget, AstExpr, AstStmt, Expr, Stmt};
use wire::{PlanEdit, ReadSel};

use crate::script_edit::{ArmRefusal, SectionArg, plan_items, segment_of_entry};
use crate::{
    ArgValue, ArmedEdit, ChangeEvent, ChangeFact, Effect, EffectKind, EvalError, EvalLimits,
    EventFacts, Provenance, ReadFace, ReadFault, ReadPosition, ReadRecord, Rule, RunCtx, ScriptCtx,
    ScriptEval, ScriptFacts, ScriptHost, ScriptLimits, ScriptRecording, ScriptTelemetry, SecFacts,
    TocFacts,
};

/// Optional string as Starlark: value or `None`. Absence must stay absence —
/// never coerce missing frontmatter to `""`.
fn opt_str<'v>(heap: Heap<'v>, o: Option<&str>) -> Value<'v> {
    match o {
        Some(s) => heap.alloc(s),
        None => Value::new_none(),
    }
}

/// Max parser nesting depth (brackets `([{` or consecutive unary `not`/`-`/`+`/
/// `~`). Deep chains overflow the native stack and abort the process
/// (`catch_unwind` cannot catch stack overflow) — issue #66. Bound before parse;
/// 500 ≫ any real rule (depth, not total size).
pub(crate) const MAX_NESTING_DEPTH: usize = 500;

/// Eval-thread stack size. [`MAX_NESTING_DEPTH`] debug frames +
/// [`crate::EvalLimits::max_call_depth`] exceed a default 2 MiB stack; 128 MiB
/// virtual (not fully committed) covers the depth guard's worst case.
pub(crate) const EVAL_STACK_BYTES: usize = 128 * 1024 * 1024;

/// Run `f` on a dedicated large-stack thread (see [`EVAL_STACK_BYTES`]). Scoped
/// so borrows need no `'static`; always joins before return.
pub(crate) fn on_eval_stack<T, F>(f: F) -> T
where
    F: FnOnce() -> T + Send,
    T: Send,
{
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("rules-eval".to_owned())
            .stack_size(EVAL_STACK_BYTES)
            .spawn_scoped(scope, f)
            .expect("spawn rules-eval thread")
            .join()
            .expect("rules-eval thread must not panic (run_rule catches its own)")
    })
}

/// Closed rule-language globals: Starlark stdlib + effect-descriptor
/// constructors. Entire capability surface; no ambient I/O, no stream builtins.
pub(crate) fn effect_globals() -> Globals {
    GlobalsBuilder::standard().with(effect_api).build()
}

/// Rule dialect: standard grammar with `load` disabled — parse error if used
/// (no external symbols; pure kernel grants no load capability).
fn rule_dialect() -> Dialect {
    Dialect {
        enable_load: false,
        ..Dialect::Standard
    }
}

/// Script dialect: the rule dialect plus top-level statements. The script
/// entry's module top level IS the program, so `if` / `for` at the top level
/// are the ordinary case there — while a rule or task, which must define a
/// hook, keeps the stricter grammar. `load` stays disabled on both.
fn script_dialect() -> Dialect {
    Dialect {
        enable_top_level_stmt: true,
        ..rule_dialect()
    }
}

/// Load gate: parse-check every rule without running. Separates authoring
/// faults from per-event [`crate::eval`] faults.
///
/// # Errors
/// [`EvalError::SourceTooLarge`] or [`EvalError::Parse`] for the first bad rule.
pub fn validate(rules: &[Rule], limits: EvalLimits) -> Result<(), EvalError> {
    // Same large stack as eval — pathological nesting must not abort.
    on_eval_stack(|| {
        for rule in rules {
            check_source_size(rule, limits)?;
            check_nesting_depth(rule)?;
            AstModule::parse(&rule.id, rule.source.clone(), &rule_dialect()).map_err(|e| {
                EvalError::Parse {
                    rule_id: rule.id.clone(),
                    reason: e.to_string(),
                }
            })?;
        }
        Ok(())
    })
}

/// Refuse source over the parse-DoS byte cap before the parser.
fn check_source_size(rule: &Rule, limits: EvalLimits) -> Result<(), EvalError> {
    if rule.source.len() > limits.max_source_bytes {
        return Err(EvalError::SourceTooLarge {
            rule_id: rule.id.clone(),
            bytes: rule.source.len(),
            limit: limits.max_source_bytes,
        });
    }
    Ok(())
}

/// Refuse nesting past [`MAX_NESTING_DEPTH`] (uncatchable native-stack abort).
/// Left-to-right scan skips string/comment content; tracks bracket depth and
/// consecutive unary runs (issue #66). Whitespace does not break a unary run
/// (`- - - x` = depth 3). Over-approx is safe: only rejects pathological.
fn check_nesting_depth(rule: &Rule) -> Result<(), EvalError> {
    let bytes = rule.source.as_bytes();
    let n = bytes.len();
    let mut i = 0;
    let mut bracket_depth: usize = 0;
    let mut unary_run: usize = 0;
    let over = |rule: &Rule| EvalError::Parse {
        rule_id: rule.id.clone(),
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
                    return Err(over(rule));
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
                    return Err(over(rule));
                }
                i += 1;
            }
            _ if c.is_ascii_whitespace() => i += 1,
            b'n' if word_at(bytes, i, b"not") => {
                unary_run += 1;
                if unary_run > MAX_NESTING_DEPTH {
                    return Err(over(rule));
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

/// Skip a string literal starting at `open` (`bytes[open]` is a quote). Returns
/// the index just past the closing quote (or end-of-input on an unterminated
/// literal — the parser then reports the real syntax error).
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

/// True if the word `needle` sits at `bytes[i..]` with a non-identifier boundary
/// on each side (so `not` matches the keyword, not the start of `notes`).
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

/// What `Evaluator::extra` carries — one type, one downcast, per-plane content.
/// The hooked planes carry the [`EmitStore`]; the script plane carries its
/// injected host seam. A builtin asking for the wrong one is a loud fault.
#[derive(starlark::any::ProvidesStaticType)]
enum PlaneStore<'h> {
    /// Change / run planes: effect descriptors, provenance-stamped.
    Emit(EmitStore<'h>),
    /// Script plane: the one effectful seam plus its recorded reads.
    Script(&'h ScriptEntry<'h>),
}

/// Emit store via `Evaluator::extra`: rule id, plane-typed [`Provenance`],
/// depth, per-rule `seq`. Change path stamps fingerprints; run path stamps
/// [`Provenance::Run`].
/// Which phase of a two-phase (`load` → freeze → `fire`) evaluation is
/// running (hook-support design § 2.2, as amended 2026-08-23).
///
/// **Why this exists as a runtime phase rather than as two globals sets.**
/// § 2.2 specified the load environment as the fire environment MINUS the
/// effect builtins, so that a top-level effect would be an unbound-name
/// fault. starlark-rust `=0.14.2` resolves global names at MODULE-COMPILE
/// time (`starlark-0.14.2/src/eval.rs:82-89` — the `Globals` handed to
/// `eval_module` go straight into `ScopeResolverGlobals`), so a block whose
/// `def run(event)` body merely MENTIONS `create()` would fail to compile
/// under such a globals set — every effectful hook would be unloadable and
/// its declarations unreadable. Freezing does not rebind globals either, so
/// "frozen module ∪ the effect builtins" is not constructible. Measured;
/// probe kept with the card. The boundary therefore lives HERE.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectPhase {
    /// The load phase: the module's top level runs to publish its
    /// `declare()` data. Recording an effect is refused.
    Load,
    /// The fire phase (and every shipped single-phase plane): effects
    /// record normally.
    Fire,
}

struct EmitStore<'h> {
    rule_id: String,
    provenance: Provenance,
    depth: u32,
    effects: RefCell<Vec<Effect>>,
    next_seq: Cell<u32>,
    /// The phase gate. [`Fire`](EffectPhase::Fire) for every shipped plane,
    /// so nothing existing changes behavior.
    phase: EffectPhase,
    /// What `declare()` collected during the LOAD phase, in call order. Empty
    /// on every shipped plane — `declare` is a hook-plane builtin and is not
    /// registered on [`effect_globals`].
    declarations: RefCell<Vec<Declaration>>,
    /// The seam `bash()` reaches during the FIRE phase. `None` on every
    /// shipped plane and on the load phase, where the phase gate refuses
    /// first — so an absent seam is only ever reached by a fire the caller
    /// built without one, and that is a loud fault, never a silent no-op.
    host: Option<&'h dyn FireHost>,
}

impl<'h> EmitStore<'h> {
    fn new(rule_id: &str, provenance: Provenance, depth: u32) -> Self {
        Self {
            rule_id: rule_id.to_owned(),
            provenance,
            depth,
            effects: RefCell::new(Vec::new()),
            next_seq: Cell::new(0),
            // The shipped planes are single-phase and effectful throughout.
            phase: EffectPhase::Fire,
            declarations: RefCell::new(Vec::new()),
            host: None,
        }
    }

    /// The same store, gated to a phase — the load half of the run entry's
    /// two-phase evaluation.
    fn in_phase(mut self, phase: EffectPhase) -> Self {
        self.phase = phase;
        self
    }

    /// The same store with the fire phase's process seam bound.
    fn with_host(mut self, host: &'h dyn FireHost) -> Self {
        self.host = Some(host);
        self
    }

    /// Record one descriptor, stamping the rule id, plane-typed provenance,
    /// depth, and the next per-rule `seq`.
    ///
    /// Infallible: the phase gate lives one step earlier, at the ACCESSOR
    /// ([`store`]) every effect builtin must call to reach this store at all
    /// (design § Amendments / A1). Gating there rather than here is what
    /// makes the law hold for builtins that are effectful WITHOUT recording
    /// an [`Effect`] — `bash` executes a process and pushes no descriptor,
    /// so a gate on this method alone would miss it.
    fn push(&self, kind: EffectKind, args: BTreeMap<String, ArgValue>) {
        let seq = self.next_seq.get();
        self.next_seq.set(seq + 1);
        self.effects.borrow_mut().push(Effect {
            kind,
            rule_id: self.rule_id.clone(),
            seq,
            depth: self.depth,
            provenance: self.provenance.clone(),
            args,
        });
    }
}

/// An effect builtin was called during the LOAD phase (design § Amendments /
/// A1). Typed on purpose: it travels as `ErrorKind::Native` and is
/// classified by DOWNCAST, never by matching the message string — a class
/// read off prose breaks the first time the prose is improved.
///
/// Distinct from `name_error`, which is `ErrorKind::Scope`: an identifier
/// bound nowhere (`dney`, `declare(impl = chck_stop)`). Here the name IS
/// bound — that is precisely what makes an effectful block loadable — and
/// the refusal is about the phase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectAtLoad {
    /// The builtin as the author spelled it, e.g. `create`, `bash`.
    pub builtin: &'static str,
}

impl std::fmt::Display for EffectAtLoad {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "`{}` does not act at load: the load phase evaluates a block's top \
             level to publish what it declares, and applies nothing. Move the \
             call inside the entry the block declares — effects are the fire \
             phase's.",
            self.builtin
        )
    }
}

impl std::error::Error for EffectAtLoad {}

/// A load-phase builtin (`declare`, `exec`) was called during the FIRE phase
/// — A1's mirror of [`EffectAtLoad`], same typed treatment. Declaring at
/// fire is not a harmless no-op: it is a block asserting its shape after the
/// shape was already published and cached, so it refuses rather than
/// silently doing nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclareAtFire {
    /// The builtin as the author spelled it, e.g. `declare`, `exec`.
    pub builtin: &'static str,
}

impl std::fmt::Display for DeclareAtFire {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "`{}` only runs at load: a block declares its shape once, when the \
             page is loaded, and the declaration is cached at that block's rev. \
             Calling it from the entry would assert a shape nobody reads.",
            self.builtin
        )
    }
}

impl std::error::Error for DeclareAtFire {}

/// `declare(impl = …)` was handed something that is not an entry — A1's
/// third typed refusal, and for the same reason as the other two: the fault
/// class a caller matches on must come from a DOWNCAST, never from reading
/// the prose.
///
/// It used to be a bare `anyhow!`, which reaches starlark as an untyped
/// `ErrorKind::Native` and classified as `runtime`. Both `wire-contract.md`
/// and `run-plane.md` published `impl_type` in the union a caller matches on,
/// and it could never be emitted; the one test asserted the reason STRING, so
/// CI could not see it. (PR 195 review, e9f1ae35, F7.)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImplType {
    /// The starlark type the author actually passed, e.g. `int`.
    pub got: String,
}

impl std::fmt::Display for ImplType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "declare: `impl` is {} — it must be a function defined in this block, or an \
             `exec(...)` value; nothing else is an entry",
            self.got
        )
    }
}

impl std::error::Error for ImplType {}

/// What one `declare()` call published, as the load phase collected it.
///
/// `data` is the **uninterpreted** dict the author passed, verbatim (design
/// § 2.4: *"`declarations` is the uninterpreted dict `declare()` collected —
/// evaluation's data, published verbatim; the caller owns every key's
/// meaning"*). The engine reads exactly one key out of it, `impl`, and only
/// to know WHAT to call — never why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    /// Every key the author passed except `impl`, verbatim.
    pub data: serde_json::Value,
    /// What this block's entry is.
    pub entry: DeclaredEntry,
}

/// A declared block's entry — the `entry_kind` a load row publishes.
///
/// `impl` is resolved at the `declare()` CALL, while the value is still in
/// hand: a callable is the evaluated entry, an [`exec`](ExecSpec) value is a
/// process entry, anything else is the `impl_type` fault. Resolving it there
/// rather than after the freeze is why no custom starlark type is minted for
/// `exec()` — the one bit that must be distinguished is distinguished where
/// the distinction exists, and the spec travels onward as data because the
/// load row publishes it as data anyway (§ 2.2 Response).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeclaredEntry {
    /// `declare(impl = fn)` (or a bare `def run(event)`): the frozen module's
    /// binding of this name is called at fire.
    Evaluated {
        /// The def's own name — what `frozen.get(..)` is asked for at fire.
        name: String,
    },
    /// `declare(impl = exec(block = "check"))`: a process entry, run through
    /// the exec bracket rather than evaluated.
    Exec(ExecSpec),
}

impl DeclaredEntry {
    /// The `entry_kind` word a load row carries.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            DeclaredEntry::Evaluated { .. } => "evaluated",
            DeclaredEntry::Exec(_) => "exec",
        }
    }
}

/// A process entry as `exec()` declared it (§ 1.4):
/// `exec(interpreter, cmd=|block=, args=[], env={})`. Data, not code — a new
/// language is `argv[0]`, never a concept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecSpec {
    /// `argv[0]` — `"bash"`, `"bun"`, anything on the path.
    pub interpreter: String,
    /// The program: exactly one of these is `Some`, enforced at the call.
    pub program: ExecProgram,
    /// Arguments after the program.
    pub args: Vec<String>,
    /// Declared environment pairs, overlaid on the target's own `env` (the
    /// run-env ruling's shadowing rule).
    pub env: BTreeMap<String, String>,
}

/// Where an exec'd entry's bytes come from — exactly one, never both, never
/// neither.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecProgram {
    /// `cmd = "test -f allow-stop"` — the bytes inline in the declaration.
    Cmd(String),
    /// `block = "check"` — the `^id` anchor of a block on the same page. The
    /// anchor is resolved by the LOAD caller, which is the layer that holds
    /// the page: a dangling anchor is the block's load fault `no_block`, a
    /// duplicate the typed `ambiguous_anchor` (§ 1.4). The kernel knows no
    /// pages, so it carries the name and judges nothing.
    Block(String),
}

/// One `bash()` call as the program spelled it — the seam's whole input
/// (§ 1.3: `bash(cmd=, block=, cwd=, env={}, timeout=, stdin=)`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BashCall {
    /// The program: exactly one of `cmd` / `block`, enforced at the call.
    pub program: ExecProgram,
    /// `cwd = "…"` — workspace-relative working directory.
    pub cwd: Option<String>,
    /// Environment pairs overlaid on the bracket's inherited environment.
    pub env: BTreeMap<String, String>,
    /// Wall-clock ceiling in seconds; the bracket's configured one when
    /// absent.
    pub timeout_s: Option<u64>,
    /// What to write on the process's stdin. A string rides verbatim; a dict
    /// or list arrives here already serialized as compact JSON, because the
    /// kernel owns the starlark→JSON edge and the plane owns the pipe.
    pub stdin: Option<String>,
    /// The 1-based source line of the call, for the row it produces.
    pub line: u32,
}

/// The fire phase's process seam: what `bash()` reaches when a fire binds one.
///
/// A trait rather than a closure for the same reason [`ScriptEntry`] carries
/// its host as one — the run plane owns the bracket (timeout, setsid, log,
/// the `exec[]` row), and the kernel must not learn any of it. The kernel
/// knows only that a call goes out and a JSON row comes back.
pub trait FireHost: Sync {
    /// Run one `bash()` call through the plane's bracket and answer the row
    /// the program sees — and that the fire row's `exec[]` carries.
    ///
    /// § 1.3's law: **`bash` never raises.** An unstartable process is
    /// `exit: 127`, a timeout `timed_out: true` with `exit: 137`. An `Err`
    /// here is for the cases that are not the process's at all (a seam that
    /// cannot reach its bracket), and it faults the program loudly.
    ///
    /// # Errors
    /// The plane's own words, carried whole; the program faults with them. A
    /// `String` rather than a typed error so the seam imposes no error crate
    /// on its implementors — the run plane is the only layer with anything to
    /// say here, and the kernel does nothing but relay it.
    fn bash(&self, call: &BashCall) -> Result<serde_json::Value, String>;
}

/// How a two-phase evaluation fault is classified on a `run` row (design
/// § 2.2 Response, as amended by A1). Only the classes THIS layer can decide
/// live here — the addressing and shape classes (`no_block`, `not_declared`,
/// `reply_shape`, …) belong to the layers that own those facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultClass {
    /// The block did not parse.
    Parse,
    /// An identifier bound nowhere — a typo (`dney`,
    /// `declare(impl = chck_stop)`). starlark's `ErrorKind::Scope`.
    NameError,
    /// An effect builtin was called during the load phase.
    EffectAtLoad,
    /// A load-phase builtin (`declare`, `exec`) was called at fire.
    DeclareAtFire,
    /// `declare(impl = …)` named something that is not an entry — neither a
    /// callable nor an `exec(...)` value.
    ImplType,
    /// The caller's `prelude` is invalid — its code faulted, **or** it carried
    /// CONSENT MATERIAL (a `declare()` or an `exec()` value).
    ///
    /// One class, not two (advisor `ea317a27`, 2026-08-23, A10 — an earlier
    /// `consent_in_prelude` was withdrawn before it was written): the caller's
    /// remedy is the same in both cases — fix your prelude — and the REASON
    /// string names which invalidity it was. Consent is page-authored by law,
    /// and a prelude is caller source: a declaration there would make every
    /// anchored fence on every addressed page a fire target the page never
    /// consented to.
    PreludeInvalid,
    /// The entry ran and returned something outside the admitted set — the
    /// PROGRAM is fine, its answer is not, and saying `runtime` would send
    /// the author looking for a bug that is not there.
    ReplyShape,
    /// Fuel, memory, or call depth exhausted.
    Budget,
    /// Anything else the evaluator raised.
    Runtime,
}

impl FaultClass {
    /// The wire spelling — ONE owner, so a row's `fault.class`, the docs and
    /// the tests cannot drift apart.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            FaultClass::Parse => "parse",
            FaultClass::NameError => "name_error",
            FaultClass::EffectAtLoad => "effect_at_load",
            FaultClass::DeclareAtFire => "declare_at_fire",
            FaultClass::ImplType => "impl_type",
            FaultClass::PreludeInvalid => "prelude_invalid",
            FaultClass::ReplyShape => "reply_shape",
            FaultClass::Budget => "budget",
            FaultClass::Runtime => "runtime",
        }
    }
}

/// Classify one starlark error for a `run` row.
///
/// **By downcast, never by message** (A1: *"`effect_at_load` is
/// `ErrorKind::Native` carrying `EffectAtLoad`; never classified by
/// string"*). A class read off prose gets reclassified by the next prose
/// edit, silently — and these classes are what a caller branches on.
///
/// The split that matters: `ErrorKind::Scope` is an identifier bound NOWHERE
/// (`name_error`); a phase refusal arrives as `ErrorKind::Native` carrying
/// one of this module's typed errors. They describe opposite situations — a
/// typo versus a correct name called in the wrong phase — and collapsing
/// them would send an author hunting a misspelling that is not there.
#[must_use]
pub fn classify_starlark_fault(error: &starlark::Error) -> FaultClass {
    match error.kind() {
        starlark::ErrorKind::Parser(_) => FaultClass::Parse,
        starlark::ErrorKind::Scope(_) => FaultClass::NameError,
        starlark::ErrorKind::StackOverflow(_) => FaultClass::Budget,
        starlark::ErrorKind::Native(inner) => {
            if inner.downcast_ref::<EffectAtLoad>().is_some() {
                FaultClass::EffectAtLoad
            } else if inner.downcast_ref::<DeclareAtFire>().is_some() {
                FaultClass::DeclareAtFire
            } else if inner.downcast_ref::<ImplType>().is_some() {
                FaultClass::ImplType
            } else {
                FaultClass::Runtime
            }
        }
        _ => FaultClass::Runtime,
    }
}

/// The 1-based source line a starlark error points at, when it carried a
/// span — the `fault.line` a row publishes so an author goes straight to the
/// call. 1-based for a reader; the resolver is 0-based.
#[must_use]
pub fn starlark_fault_line(error: &starlark::Error) -> Option<u32> {
    error
        .span()
        .map(|span| u32::try_from(span.resolve_span().begin.line + 1).unwrap_or(u32::MAX))
}

/// Downcast the plane store out of `Evaluator::extra`. Absent only if a builtin
/// is somehow reached outside a metered run — a loud fault, never a silent drop.
fn plane<'a, 'e>(eval: &'a Evaluator<'_, '_, 'e>) -> anyhow::Result<&'a PlaneStore<'e>> {
    eval.extra
        .and_then(|e| e.downcast_ref::<PlaneStore<'e>>())
        .ok_or_else(|| anyhow::anyhow!("kernel: builtin invoked without a plane store"))
}

/// The emit store, or a loud fault if a descriptor constructor ran on the
/// script plane (where it is not registered — the surfaces are separate).
///
/// **This is the phase gate** (design § Amendments / A1). It is the ONE
/// accessor every effect builtin passes to reach its channel, so gating here
/// makes an ungated effect builtin unconstructible: a builtin that never
/// calls this cannot emit, execute, or touch the corpus, and one that does
/// call it is gated whether or not its author remembered to be. A
/// per-builtin check would be a convention each new constructor could
/// forget; this is a choke point.
///
/// `builtin` is the name as the AUTHOR spelled it, because it is what the
/// reader of the fault has to go find in their source.
///
/// # Errors
/// [`EffectAtLoad`] on a `Load`-phase store; the script-plane fault as before.
fn store<'a, 'e>(
    eval: &'a Evaluator<'_, '_, 'e>,
    builtin: &'static str,
) -> anyhow::Result<&'a EmitStore<'e>> {
    match plane(eval)? {
        PlaneStore::Emit(store) => {
            if store.phase == EffectPhase::Load {
                return Err(anyhow::Error::new(EffectAtLoad { builtin }));
            }
            Ok(store)
        }
        PlaneStore::Script(_) => Err(anyhow::anyhow!(
            "effect-api: constructor invoked on the script plane, which registers none"
        )),
    }
}

/// The emit store for a LOAD-phase builtin — A1's mirror of [`store`].
///
/// `declare()` and `exec()` publish what a block IS; that happens once, at
/// load, and the answer is cached at the block's rev. Calling one at fire
/// would assert a shape nobody reads, so it refuses instead of silently doing
/// nothing.
///
/// # Errors
/// [`DeclareAtFire`] on a `Fire`-phase store; the script-plane fault as for
/// [`store`].
fn load_store<'a, 'e>(
    eval: &'a Evaluator<'_, '_, 'e>,
    builtin: &'static str,
) -> anyhow::Result<&'a EmitStore<'e>> {
    match plane(eval)? {
        PlaneStore::Emit(store) => {
            if store.phase == EffectPhase::Fire {
                return Err(anyhow::Error::new(DeclareAtFire { builtin }));
            }
            Ok(store)
        }
        PlaneStore::Script(_) => Err(anyhow::anyhow!(
            "hook-api: `{builtin}` invoked on the script plane, which registers none"
        )),
    }
}

/// The script entry, or a loud fault if a script builtin ran on a hooked plane
/// (which would mean `script_api` leaked into `effect_globals`).
fn script<'e>(eval: &Evaluator<'_, '_, 'e>) -> anyhow::Result<&'e ScriptEntry<'e>> {
    match plane(eval)? {
        PlaneStore::Script(entry) => Ok(entry),
        PlaneStore::Emit(_) => Err(anyhow::anyhow!(
            "script-api: `read`/`me` invoked on a hooked plane, which grants no live reads"
        )),
    }
}

/// Insert an optional scalar argument when present.
fn insert_opt(args: &mut BTreeMap<String, ArgValue>, key: &str, value: Option<String>) {
    if let Some(value) = value {
        args.insert(key.to_owned(), ArgValue::Str(value));
    }
}

/// `create(props = {…})` → the descriptor's map argument (D6). A dict of
/// string keys to strings or lists of strings, carried INERT: the constructor
/// judges only the SHAPE — the create door owns key grammar, quoting and the
/// list spelling, so the whole escaping law lives at one door instead of in
/// every record-birthing block.
///
/// Absent, `None`, or empty is `Ok(None)` — a birth with no frontmatter of its
/// own, exactly as before this argument existed.
fn props_arg(value: Option<Value<'_>>) -> anyhow::Result<Option<BTreeMap<String, ArgValue>>> {
    let Some(value) = value else { return Ok(None) };
    if value.is_none() {
        return Ok(None);
    }
    let Some(dict) = starlark::values::dict::DictRef::from_value(value) else {
        anyhow::bail!(
            "create(props=…) takes a frontmatter dict — string keys to strings or lists of \
             strings; got {}",
            value.to_repr()
        );
    };
    let mut props: BTreeMap<String, ArgValue> = BTreeMap::new();
    for (key, val) in dict.iter() {
        let Some(key) = key.unpack_str() else {
            anyhow::bail!(
                "create(props=…) keys are frontmatter keys (strings); got {}",
                key.to_repr()
            );
        };
        if let Some(text) = val.unpack_str() {
            props.insert(key.to_owned(), ArgValue::Str(text.to_owned()));
            continue;
        }
        let Some(list) = starlark::values::list::ListRef::from_value(val) else {
            anyhow::bail!(
                "create(props=…) value for {key:?} is a string or a list of strings — a \
                 frontmatter value is a scalar or a one-level list; got {}",
                val.to_repr()
            );
        };
        let mut items = Vec::with_capacity(list.len());
        for item in list.iter() {
            let Some(text) = item.unpack_str() else {
                anyhow::bail!(
                    "create(props=…) list member for {key:?} is a string; got {}",
                    item.to_repr()
                );
            };
            items.push(text.to_owned());
        }
        props.insert(key.to_owned(), ArgValue::List(items));
    }
    Ok((!props.is_empty()).then_some(props))
}

/// Effect-descriptor constructors — entire rule-language builtin surface.
/// Named args only; each returns `None` (effect is the recorded descriptor).
/// No constructor performs or exposes I/O.
#[starlark_module]
fn effect_api(builder: &mut GlobalsBuilder) {
    /// `md.set_field` — set frontmatter `field` to `value`, with an optional
    /// advisory `message`.
    fn set_field(
        #[starlark(require = named)] field: String,
        #[starlark(require = named)] value: String,
        #[starlark(require = named)] message: Option<String>,
        eval: &mut Evaluator<'_, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let mut args = BTreeMap::new();
        args.insert("field".to_owned(), ArgValue::Str(field));
        args.insert("value".to_owned(), ArgValue::Str(value));
        insert_opt(&mut args, "message", message);
        store(eval, "set_field")?.push(EffectKind::SetField, args);
        Ok(NoneType)
    }

    /// `md.append_section` — append `content` to the section at `section`, with an
    /// optional advisory `message`.
    fn append_section(
        #[starlark(require = named)] section: String,
        #[starlark(require = named)] content: String,
        #[starlark(require = named)] message: Option<String>,
        eval: &mut Evaluator<'_, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let mut args = BTreeMap::new();
        args.insert("section".to_owned(), ArgValue::Str(section));
        args.insert("content".to_owned(), ArgValue::Str(content));
        insert_opt(&mut args, "message", message);
        store(eval, "append_section")?.push(EffectKind::AppendSection, args);
        Ok(NoneType)
    }

    /// `md.create` — birth the file at `path` with `body` as its whole bytes,
    /// with an optional advisory `message`. Realized through the create door
    /// (occupied path refuses, armed middleware stamps, checks) — the birth
    /// cap for declared tasks (SCHEMA §5, ruled 2026-08-18).
    ///
    /// `path` is the RELATIVE landing coordinate as declared — the string
    /// the `md.create` capability glob judges — and admits no rooted
    /// spelling. Targeting rides the optional `base` (ZT ruling 2026-08-19
    /// #2: the boundary is data, never a glued string): a rooted
    /// `root:<dir>` ref or a confined workspace-relative directory the path
    /// resolves under. Absent `base`, the caller's ambient directory is the
    /// default base; absent both, the path lands workspace-root-relative.
    /// `props` (optional, D6) is the newborn's FRONTMATTER as a dict — keys to
    /// strings or lists of strings. **The create door serializes it**: quoting,
    /// escaping and list spelling are the door's, so no record-birthing block
    /// hand-rolls a YAML escaper and no caller value can forge a key. A body
    /// that already opens its own frontmatter fence refuses at the door rather
    /// than land two spellings of one block.
    fn create<'v>(
        #[starlark(require = named)] path: String,
        #[starlark(require = named)] body: String,
        #[starlark(require = named)] base: Option<String>,
        #[starlark(require = named)] message: Option<String>,
        #[starlark(require = named)] props: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let mut args = BTreeMap::new();
        args.insert("path".to_owned(), ArgValue::Str(path));
        args.insert("body".to_owned(), ArgValue::Str(body));
        insert_opt(&mut args, "base", base);
        insert_opt(&mut args, "message", message);
        if let Some(props) = props_arg(props)? {
            args.insert("props".to_owned(), ArgValue::Map(props));
        }
        store(eval, "create")?.push(EffectKind::Create, args);
        Ok(NoneType)
    }

    /// `daemon.refresh_view` — mark the resident `view` stale.
    fn refresh_view(
        #[starlark(require = named)] view: String,
        eval: &mut Evaluator<'_, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let mut args = BTreeMap::new();
        args.insert("view".to_owned(), ArgValue::Str(view));
        store(eval, "refresh_view")?.push(EffectKind::RefreshView, args);
        Ok(NoneType)
    }

    /// `proto.send` — deliver `message` to agent target(s) `to`.
    fn send(
        #[starlark(require = named)] to: UnpackList<String>,
        #[starlark(require = named)] message: String,
        eval: &mut Evaluator<'_, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let mut args = BTreeMap::new();
        args.insert("to".to_owned(), ArgValue::List(to.items));
        args.insert("message".to_owned(), ArgValue::Str(message));
        store(eval, "send")?.push(EffectKind::Send, args);
        Ok(NoneType)
    }

    /// `proto.remind` — schedule an advisory reminder `message`, optionally `at` a
    /// time hint (opaque to the kernel).
    fn remind(
        #[starlark(require = named)] message: String,
        #[starlark(require = named)] at: Option<String>,
        eval: &mut Evaluator<'_, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let mut args = BTreeMap::new();
        args.insert("message".to_owned(), ArgValue::Str(message));
        insert_opt(&mut args, "at", at);
        store(eval, "remind")?.push(EffectKind::Remind, args);
        Ok(NoneType)
    }

    /// `proto.ask` — pose `message` back to the writer, with optional `options`.
    fn ask(
        #[starlark(require = named)] message: String,
        #[starlark(require = named)] options: Option<UnpackList<String>>,
        eval: &mut Evaluator<'_, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let mut args = BTreeMap::new();
        args.insert("message".to_owned(), ArgValue::Str(message));
        if let Some(options) = options {
            args.insert("options".to_owned(), ArgValue::List(options.items));
        }
        store(eval, "ask")?.push(EffectKind::Ask, args);
        Ok(NoneType)
    }

    /// `intent(action, …)` — reaction-plane constructor. `action` is a wire
    /// identity or the alias `notify` ≡ `proto.send`; other args carried
    /// verbatim (engine neither composes messages nor ranks severity).
    /// Grants no capability: load ceilings see constructor names only; a
    /// runtime-chosen kind is filtered downstream and reported.
    fn intent(
        #[starlark(require = named)] action: String,
        #[starlark(require = named)] target: Option<String>,
        #[starlark(require = named)] severity: Option<String>,
        #[starlark(require = named)] payload: Option<String>,
        #[starlark(require = named)] receipt: Option<String>,
        eval: &mut Evaluator<'_, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let kind = crate::action_kind(&action).ok_or_else(|| {
            anyhow::anyhow!(
                "intent: unknown action {action:?} — name an effect kind ({}) or the alias `notify`",
                EffectKind::ALL
                    .iter()
                    .map(|k| k.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;
        let mut args = BTreeMap::new();
        args.insert("action".to_owned(), ArgValue::Str(action));
        insert_opt(&mut args, "target", target);
        insert_opt(&mut args, "severity", severity);
        insert_opt(&mut args, "payload", payload);
        insert_opt(&mut args, "receipt", receipt);
        store(eval, "intent")?.push(kind, args);
        Ok(NoneType)
    }

    /// `receipt_addr(path, rev)` → `path#^anchor` (§6.1), minted before delivery.
    /// `path` is caller-chosen; anchor is pure in `(path, rev)` (no clock/counter)
    /// so re-eval names the same address.
    fn receipt_addr(
        #[starlark(require = pos)] path: String,
        #[starlark(require = pos)] rev: String,
    ) -> anyhow::Result<String> {
        // `#` in path makes `path#^anchor` unsplittable — refuse.
        if path.is_empty() || path.contains('#') {
            return Err(anyhow::anyhow!(
                "receipt_addr: path {path:?} is empty or contains `#`, which would make the \
                 `path#^anchor` address ambiguous"
            ));
        }
        Ok(crate::receipt_address(&path, &rev))
    }

    /// `proto.notice` — a low-severity advisory `message`.
    fn notice(
        #[starlark(require = named)] message: String,
        eval: &mut Evaluator<'_, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let mut args = BTreeMap::new();
        args.insert("message".to_owned(), ArgValue::Str(message));
        store(eval, "notice")?.push(EffectKind::Notice, args);
        Ok(NoneType)
    }
}

/// The hook plane's own three builtins (design § 1.3, § 1.4): the two
/// load-phase constructors that publish what a block IS, and the fire-phase
/// process seam.
///
/// They join [`effect_globals`]' surface to make [`hook_globals`] — ONE
/// closed set for compile, freeze and fire (A1). What differs between the
/// phases is not which names exist but what they DO, and that difference
/// lives in the two accessors ([`store`], [`load_store`]).
#[starlark_module]
fn hook_api(builder: &mut GlobalsBuilder) {
    /// `declare(on = …, match = …, impl = fn, …)` — publish what this block
    /// is. Every keyword is carried **verbatim and uninterpreted** (§ 2.4);
    /// the engine reads exactly one of them, `impl`, and only to know what to
    /// call.
    ///
    /// `impl` is *callable | exec-value*, resolved here while the value is
    /// still in hand: a callable names the frozen module's binding the fire
    /// calls, an [`exec`](ExecSpec) value is a process entry. Omitting it
    /// means the conventional `def run(event)`.
    fn declare<'v>(
        #[starlark(require = named, default = NoneType)] r#impl: Value<'v>,
        #[starlark(kwargs)] rest: SmallMap<String, Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let store = load_store(eval, "declare")?;
        let entry = declared_entry(r#impl)?;
        let mut data = serde_json::Map::new();
        for (key, value) in rest {
            data.insert(key, json_of_value(value)?);
        }
        store.declarations.borrow_mut().push(Declaration {
            data: serde_json::Value::Object(data),
            entry,
        });
        Ok(NoneType)
    }

    /// `exec(interpreter, cmd = | block = , args = [], env = {})` — a pure
    /// typed constructor for a process entry (§ 1.4). Legal at load, refused
    /// at fire; runs nothing itself.
    ///
    /// The value it returns is a plain dict — the spec as data. No custom
    /// starlark type is minted because the one bit that must be
    /// distinguished, *callable vs exec-value*, is distinguished at the
    /// `declare()` call where both are still in hand, and the spec travels
    /// onward as data anyway: a load row publishes it in `declarations`.
    fn exec<'v>(
        #[starlark(require = pos)] interpreter: String,
        #[starlark(require = named)] cmd: Option<String>,
        #[starlark(require = named)] block: Option<String>,
        #[starlark(require = named)] args: Option<UnpackList<String>>,
        #[starlark(require = named)] env: Option<SmallMap<String, String>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Value<'v>> {
        // The phase gate first: a refusal must not depend on the arguments
        // being right, or a wrong-phase call with a typo would teach the typo.
        load_store(eval, "exec")?;
        let program = one_program("exec", cmd, block)?;
        let spec = ExecSpec {
            interpreter,
            program,
            args: args.map(|a| a.items).unwrap_or_default(),
            env: env.map(|e| e.into_iter().collect()).unwrap_or_default(),
        };
        let heap = eval.heap();
        Ok(alloc_json(heap, &spec.to_json()))
    }

    /// `bash(cmd = | block = , cwd = , env = {}, timeout = , stdin = )` →
    /// `{command, exit, stdout, stderr, timed_out, dry}` (§ 1.3).
    ///
    /// **Acts at fire only** — bound at every phase, so a top-level `bash()`
    /// is the `effect_at_load` fault at its own line rather than an unbound
    /// name at compile time (A1). It never raises for the process's sake:
    /// unstartable is `exit: 127`, a timeout is `timed_out: True`.
    fn bash<'v>(
        #[starlark(require = named)] cmd: Option<String>,
        #[starlark(require = named)] block: Option<String>,
        #[starlark(require = named)] cwd: Option<String>,
        #[starlark(require = named)] env: Option<SmallMap<String, String>>,
        #[starlark(require = named)] timeout: Option<u64>,
        #[starlark(require = named)] stdin: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Value<'v>> {
        // The phase gate is the ACCESSOR, exactly as for every md
        // constructor — `bash` records no descriptor, so a gate on the emit
        // funnel alone would miss it entirely (A1).
        let store = store(eval, "bash")?;
        let program = one_program("bash", cmd, block)?;
        let line =
            call_site(eval).map_or(0, |(line, _)| u32::try_from(line + 1).unwrap_or(u32::MAX));
        // A string rides verbatim; anything else is the compact JSON § 1.3
        // promises, serialized here because the kernel owns this edge.
        let stdin = match stdin {
            None => None,
            Some(value) if value.is_none() => None,
            Some(value) => Some(match value.unpack_str() {
                Some(text) => text.to_owned(),
                None => serde_json::to_string(&json_of_value(value)?)?,
            }),
        };
        let call = BashCall {
            program,
            cwd,
            env: env.map(|e| e.into_iter().collect()).unwrap_or_default(),
            timeout_s: timeout,
            stdin,
            line,
        };
        let host = store.host.ok_or_else(|| {
            anyhow::anyhow!(
                "`bash` has no process seam on this lane: the fire was built without one. \
                 This is an engine defect, not an authoring fault — the call is legal."
            )
        })?;
        let row = host.bash(&call).map_err(|reason| anyhow::anyhow!(reason))?;
        let heap = eval.heap();
        Ok(alloc_json(heap, &row))
    }
}

/// Exactly one of `cmd` / `block`, named in the refusal as the author spelled
/// them — both and neither are the same authoring mistake seen from two sides.
fn one_program(
    builtin: &str,
    cmd: Option<String>,
    block: Option<String>,
) -> anyhow::Result<ExecProgram> {
    match (cmd, block) {
        (Some(cmd), None) => Ok(ExecProgram::Cmd(cmd)),
        (None, Some(block)) => Ok(ExecProgram::Block(block)),
        (Some(_), Some(_)) => Err(anyhow::anyhow!(
            "{builtin}: `cmd` and `block` are exclusive — inline bytes or an anchored block, \
             not both"
        )),
        (None, None) => Err(anyhow::anyhow!(
            "{builtin}: pass `cmd = \"…\"` for inline bytes or `block = \"<id>\"` for an \
             anchored block on this page"
        )),
    }
}

/// Resolve a `declare(impl = …)` value into the entry it names.
///
/// # Errors
/// The `impl_type` fault when the value is neither callable nor an
/// [`exec`](ExecSpec) value.
fn declared_entry(value: Value<'_>) -> anyhow::Result<DeclaredEntry> {
    if value.is_none() {
        // No `impl`: the conventional entry. Whether the module actually
        // defines it is the freeze's business (`missing_entry`), not this
        // call's — the block may legitimately declare before it defines.
        return Ok(DeclaredEntry::Evaluated {
            name: DEFAULT_HOOK_ENTRY.to_owned(),
        });
    }
    if value.get_type() == FUNCTION_TYPE {
        // A def's `Display` is `ParametersSpec::signature()`, which renders
        // `function_name` and nothing else — and starlark QUALIFIES that name
        // with the module's: a `def check_stop` in module `probe` displays as
        // `probe.check_stop`. Measured, not assumed; asserting the bare name
        // is what `a_declared_impl_names_the_def_the_fire_calls` caught, and
        // an unstripped qualifier would have sent every explicit-`impl` fire
        // to an entry `FrozenModule::get` cannot find.
        //
        // The last dot-segment is the module binding's own name, which is
        // what a fire asks the frozen module for. A def that is not a module
        // binding (a nested one) yields a name the module does not carry, and
        // the fire says `missing_entry` — loud, at the right layer.
        let rendered = value.to_string();
        let name = rendered.rsplit('.').next().unwrap_or(&rendered).to_owned();
        return Ok(DeclaredEntry::Evaluated { name });
    }
    if let Some(spec) = ExecSpec::from_value(value) {
        return Ok(DeclaredEntry::Exec(spec));
    }
    Err(anyhow::Error::new(ImplType {
        got: value.get_type().to_string(),
    }))
}

/// The conventional entry name when `declare()` names none (§ 1.3).
pub const DEFAULT_HOOK_ENTRY: &str = "run";

/// The key `exec()` stamps on the dict it returns, so [`ExecSpec::from_value`]
/// recognizes its own construction rather than guessing from shape.
const EXEC_MARK: &str = "exec";

impl ExecSpec {
    /// The spec as the dict `exec()` returns and a load row publishes.
    fn to_json(&self) -> serde_json::Value {
        let (key, value) = match &self.program {
            ExecProgram::Cmd(cmd) => ("cmd", cmd),
            ExecProgram::Block(block) => ("block", block),
        };
        serde_json::json!({
            EXEC_MARK: self.interpreter,
            key: value,
            "args": self.args,
            "env": self.env,
        })
    }

    /// Read an `exec()` value back out of the dict it returned, or `None`
    /// when the value is not one.
    fn from_value(value: Value<'_>) -> Option<Self> {
        let json = json_of_value(value).ok()?;
        let map = json.as_object()?;
        let interpreter = map.get(EXEC_MARK)?.as_str()?.to_owned();
        let program = match (map.get("cmd"), map.get("block")) {
            (Some(cmd), None) => ExecProgram::Cmd(cmd.as_str()?.to_owned()),
            (None, Some(block)) => ExecProgram::Block(block.as_str()?.to_owned()),
            _ => return None,
        };
        let args = map
            .get("args")?
            .as_array()?
            .iter()
            .map(|a| a.as_str().map(ToOwned::to_owned))
            .collect::<Option<Vec<_>>>()?;
        let env = map
            .get("env")?
            .as_object()?
            .iter()
            .map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_owned())))
            .collect::<Option<BTreeMap<_, _>>>()?;
        Some(ExecSpec {
            interpreter,
            program,
            args,
            env,
        })
    }
}

/// One starlark value as JSON — the fire's return, `declare()`'s data, and
/// `bash(stdin=)`'s payload all cross here.
///
/// The admitted set is § 2.2's, exactly: dict / list / str / int / float /
/// bool / None. Anything else is the `reply_shape` fault, named by type so
/// the author knows what they returned — never coerced to its `repr`, which
/// would turn a wrong answer into a plausible string.
///
/// # Errors
/// A value outside the admitted set, at whatever depth it sits.
pub fn json_of_value(value: Value<'_>) -> anyhow::Result<serde_json::Value> {
    use starlark::values::dict::DictRef;
    use starlark::values::list::ListRef;

    if value.is_none() {
        return Ok(serde_json::Value::Null);
    }
    if let Some(b) = value.unpack_bool() {
        return Ok(serde_json::Value::Bool(b));
    }
    if let Some(i) = value.unpack_i32() {
        return Ok(serde_json::Value::Number(i64::from(i).into()));
    }
    if let Some(s) = value.unpack_str() {
        return Ok(serde_json::Value::String(s.to_owned()));
    }
    if let Some(list) = ListRef::from_value(value) {
        return list
            .iter()
            .map(json_of_value)
            .collect::<anyhow::Result<Vec<_>>>()
            .map(serde_json::Value::Array);
    }
    if let Some(dict) = DictRef::from_value(value) {
        let mut out = serde_json::Map::new();
        for (key, item) in dict.iter() {
            let key = key.unpack_str().ok_or_else(|| {
                anyhow::anyhow!(
                    "a reply dict's keys must be strings; this one is {}",
                    key.get_type()
                )
            })?;
            out.insert(key.to_owned(), json_of_value(item)?);
        }
        return Ok(serde_json::Value::Object(out));
    }
    // Floats and big ints last: they are rarer than the above and their
    // unpacking is the one that can lose precision, so it is worth being
    // explicit about the fallback.
    if let Some(f) = value.downcast_ref::<starlark::values::float::StarlarkFloat>() {
        return serde_json::Number::from_f64(f.0)
            .map(serde_json::Value::Number)
            .ok_or_else(|| anyhow::anyhow!("a reply carried {} , which has no JSON form", f.0));
    }
    if let Ok(i) = i64::unpack_value_err(value) {
        return Ok(serde_json::Value::Number(i.into()));
    }
    Err(anyhow::anyhow!(
        "a reply may carry dict, list, str, int, float, bool or None; this is {}",
        value.get_type()
    ))
}

/// The hook plane's globals: the effect surface plus [`hook_api`]'s three.
///
/// ONE closed set for compile, freeze and fire (A1) — the load/fire boundary
/// is a phase gate at the accessors, never a difference in which names exist,
/// because starlark-rust resolves globals at module-compile time and a frozen
/// def cannot be rebound.
#[must_use]
pub fn hook_globals() -> Globals {
    GlobalsBuilder::standard()
        .with(effect_api)
        .with(hook_api)
        .build()
}

/// The banned suppression spelling `_ = read(…)`, located syntactically on the
/// parsed module — 1-based line of the first occurrence, or `None`.
///
/// `run-plane.md` § The statement-position rule: "Suppression syntax does not
/// exist in v1 (`_ = read(…)` is rejected permanently)". Echo and quiet are a
/// POSITION property the kernel reads off the AST; a spelling that claims to
/// suppress a read would be a second, non-positional grammar for the same
/// thing. The check is syntactic like the rule it defends — the name `read` and
/// the target `_` as written, at any nesting depth, never a value-level test.
fn banned_suppression(ast: &AstModule) -> Option<u32> {
    fn walk(stmt: &AstStmt, found: &mut Vec<starlark_syntax::codemap::Span>) {
        if let Stmt::Assign(assign) = &**stmt
            && let AssignTarget::Identifier(ident) = &*assign.lhs
            && ident.node.ident == "_"
            && let Expr::Call(callee, _) = &*assign.rhs
            && let Expr::Identifier(name) = &***callee
            && name.node.ident == "read"
        {
            found.push(stmt.span);
        }
        stmt.visit_stmt(|child| walk(child, found));
    }

    let mut found = Vec::new();
    walk(ast.statement(), &mut found);
    found.first().map(|span| {
        let begin = ast.file_span(*span).resolve_span().begin;
        u32::try_from(begin.line + 1).unwrap_or(u32::MAX)
    })
}

/// Script-plane globals: Starlark stdlib + `read` / `put` / `me`. A SEPARATE
/// surface — `effect_api` is deliberately absent, and these builtins never
/// join [`effect_globals`]: sharing one surface would give `on_change` rules
/// live reads and end the change plane's hermeticity-by-construction.
///
/// Effects mode (script-effects ruling, 2026-08-13): the admitted effect
/// builtins join the surface EXACTLY when named — a pure submission never
/// holds `run` at all, which is what "provably pure by default" means.
pub(crate) fn script_globals(effects: &[String]) -> Globals {
    let mut builder = GlobalsBuilder::standard().with(script_api);
    if effects.iter().any(|e| e == "run") {
        builder = builder.with(script_run_api);
    }
    if effects.iter().any(|e| e == "token_count") {
        builder = builder.with(script_token_count_api);
    }
    builder.build()
}

/// The script entry's per-attempt state: the one effectful seam (`host`), the
/// inert caller inputs, the echo positions read off the AST, and the recorded
/// responses. Lives behind `Evaluator::extra` as [`PlaneStore::Script`].
pub(crate) struct ScriptEntry<'h> {
    host: RefCell<&'h mut dyn ScriptHost>,
    actor: String,
    args: BTreeMap<String, String>,
    files: Vec<String>,
    max_reads: usize,
    /// Resolved 0-based `(line, column)` of every `read(…)` call sitting in
    /// echo position — a top-level statement, not nested.
    echo_at: RefCell<BTreeSet<(usize, usize)>>,
    /// Names whose LAST assignment is a top-level `name = read(…)` — the
    /// capture law's read half: their value already rides the trace as the
    /// read's own `echo` entry, so [`ScriptEntry::capture_bindings`] leaves
    /// them out (one fact, one carrier). Maintained in source order by the
    /// same walk that notes echo positions.
    read_bound: RefCell<BTreeSet<String>>,
    reads: RefCell<Vec<ReadRecord>>,
    /// The host's own refusal, kept typed for the consumer plane.
    fault: RefCell<Option<ReadFault>>,
    /// Set when the read ceiling refused, so the abort is classified as a
    /// budget refusal rather than a script fault.
    over_read_budget: Cell<bool>,
    /// Live `run()` calls admitted so far (effects mode), against `max_runs`.
    runs_seen: Cell<usize>,
    max_runs: usize,
    /// Set when the run ceiling refused — classified like the read ceiling.
    over_run_budget: Cell<bool>,
    bindings: RefCell<BTreeMap<String, String>>,
    /// Wire plan-edit items armed by `put()`, in execution order. Inert: no I/O
    /// happens at arm time and nothing here is applied until the consumer plane
    /// commits the whole list in ONE guarded splice.
    armed: RefCell<Vec<ArmedEdit>>,
    max_armed_edits: usize,
    /// The arm-time law's refusal, kept typed so the abort is classified as a
    /// consumer-plane refusal rather than a script fault.
    arm_refusal: RefCell<Option<ArmRefusal>>,
    /// Effects mode (script-effects ruling): `true` switches `put()` from
    /// arming to the host's live apply, and marks the attempt as the LIVE
    /// model. The `run` builtin's availability is the globals' (it joins only
    /// when admitted), not this flag's.
    live: bool,
}

/// The inert input names the script plane binds before eval. They are inputs,
/// never results, so [`ScriptFacts::bindings`] excludes them.
const SCRIPT_INPUTS: [&str; 2] = ["args", "files"];

impl<'h> ScriptEntry<'h> {
    fn new(ctx: &ScriptCtx, limits: ScriptLimits, host: &'h mut dyn ScriptHost) -> Self {
        let actor = host.actor().to_owned();
        Self {
            host: RefCell::new(host),
            actor,
            args: ctx.args.clone(),
            files: ctx.files.clone(),
            max_reads: limits.max_reads,
            echo_at: RefCell::new(BTreeSet::new()),
            read_bound: RefCell::new(BTreeSet::new()),
            reads: RefCell::new(Vec::new()),
            fault: RefCell::new(None),
            over_read_budget: Cell::new(false),
            runs_seen: Cell::new(0),
            max_runs: limits.max_runs,
            over_run_budget: Cell::new(false),
            bindings: RefCell::new(BTreeMap::new()),
            armed: RefCell::new(Vec::new()),
            max_armed_edits: limits.max_armed_edits,
            arm_refusal: RefCell::new(None),
            live: !ctx.effects.is_empty(),
        }
    }

    /// Effects mode: apply one `put()` NOW through the host — no arm, no CAS.
    fn put_live(&self, path: &str, items: Vec<PlanEdit>, line: u32) -> anyhow::Result<()> {
        self.host
            .borrow_mut()
            .put_live(path, items, line)
            .map_err(|e| anyhow::anyhow!("put: {}", e.reason))
    }

    /// Effects mode: execute one `run()` NOW through the host; the row comes
    /// back as JSON for the caller to shape into a Starlark value.
    ///
    /// The run ceiling binds HERE, at admission: a run's own execution is
    /// metered on the run plane's budget rather than the script clock, so the
    /// COUNT is what the kernel bounds. The refusal is typed
    /// ([`EvalError::RunBudget`]); the runs already executed stand.
    fn run_live(
        &self,
        page: &str,
        task: Option<&str>,
        args: Vec<String>,
        env: BTreeMap<String, String>,
        dry: bool,
        line: u32,
    ) -> anyhow::Result<serde_json::Value> {
        if self.runs_seen.get() >= self.max_runs {
            self.over_run_budget.set(true);
            anyhow::bail!("run budget of {} runs per attempt reached", self.max_runs);
        }
        self.runs_seen.set(self.runs_seen.get() + 1);
        self.host
            .borrow_mut()
            .run_live(page, task, args, env, dry, line)
            .map_err(|e| anyhow::anyhow!("run: {}", e.reason))
    }

    /// Effects mode: measure one `token_count()` NOW through the host.
    fn token_count_live(&self, text: &str) -> anyhow::Result<i64> {
        self.host
            .borrow_mut()
            .token_count_live(text)
            .map_err(|e| anyhow::anyhow!("token_count: {}", e.reason))
    }

    /// Arm one `put()` call's plan items — PURE: no host call, no I/O. The
    /// arm-time law runs first and, when it refuses, arms NOTHING from this call
    /// and aborts the script.
    ///
    /// The armed list may span N content paths: the commit is the §4.4 SET
    /// form (`splice.set`), sealed across the set (run-plane.md § One COMMIT
    /// per attempt — the arm-time `multi_file_write_set` refusal is retired
    /// with the set-commit machinery that replaced it). The receipt companion
    /// is not a content path — it rides the splice request's own `receipt`
    /// field in the same batch (§6.1), never this list.
    fn arm(&self, path: &str, items: Vec<PlanEdit>, line: u32, depth: u32) -> anyhow::Result<()> {
        let mut armed = self.armed.borrow_mut();
        // The ceiling refuses the edit that would cross it; the list already
        // armed stays whole — never truncated.
        if armed.len() + items.len() > self.max_armed_edits {
            *self.arm_refusal.borrow_mut() = Some(ArmRefusal::ArmedBudget {
                line,
                limit: self.max_armed_edits,
            });
            return Err(anyhow::anyhow!(
                "armed-edit budget of {} edits per attempt reached",
                self.max_armed_edits
            ));
        }
        armed.extend(items.into_iter().map(|edit| ArmedEdit {
            path: path.to_owned(),
            edit,
            line,
            depth,
        }));
        Ok(())
    }

    /// Read the echo positions off the parsed module: a `read(…)` call is an
    /// echo exactly when it is the whole right-hand side of a top-level
    /// assignment, or a top-level expression statement. Everything else —
    /// comprehensions, conditions, loop bodies, function bodies — is quiet.
    ///
    /// The same source-order walk maintains [`ScriptEntry::read_bound`]: a
    /// top-level `name = read(…)` marks the name read-bound, and ANY later
    /// rebinding of that name — a top-level reassignment, an `x += …`, a loop
    /// target, an assignment inside an `if` or `for` body — unmarks it,
    /// because the name no longer holds the face the echo entry carries. A
    /// `def` body is a local scope, so nothing inside one touches the set.
    fn note_echo_positions(&self, ast: &AstModule) {
        /// Is this expression a bare `read(…)` call?
        fn is_read_call(expr: &AstExpr) -> bool {
            let Expr::Call(callee, _) = &**expr else {
                return false;
            };
            let Expr::Identifier(ident) = &**callee.as_ref() else {
                return false;
            };
            // The bindings skip-list (script-result-echo § Border edge): a
            // name whose last top-level RHS is a bare `read()` is carried by
            // its echo entry; a bare `run()` (effects mode) is carried by its
            // `ran` entry's row — neither repeats in the bindings block.
            ident.node.ident == "read" || ident.node.ident == "run"
        }

        /// Every plain name a target binds, at any nesting inside tuples.
        fn target_names<'s>(
            target: &'s starlark_syntax::syntax::ast::AstAssignTarget,
            out: &mut Vec<&'s str>,
        ) {
            match &**target {
                AssignTarget::Identifier(ident) => out.push(&ident.node.ident),
                AssignTarget::Tuple(items) => {
                    for item in items {
                        target_names(item, out);
                    }
                }
                // `d["k"] = …` and `s.f = …` mutate a value; the NAME keeps
                // its binding, so the read-bound mark is untouched.
                AssignTarget::Index(_) | AssignTarget::Dot(_, _) => {}
            }
        }

        fn unbind(
            target: &starlark_syntax::syntax::ast::AstAssignTarget,
            read_bound: &mut BTreeSet<String>,
        ) {
            let mut names = Vec::new();
            target_names(target, &mut names);
            for name in names {
                read_bound.remove(name);
            }
        }

        fn walk<'s>(
            stmt: &'s AstStmt,
            top: bool,
            echo_candidates: &mut Vec<&'s AstExpr>,
            read_bound: &mut BTreeSet<String>,
        ) {
            match &**stmt {
                Stmt::Statements(stmts) => {
                    for s in stmts {
                        walk(s, top, echo_candidates, read_bound);
                    }
                }
                Stmt::Assign(assign) => {
                    if top {
                        echo_candidates.push(&assign.rhs);
                        if let AssignTarget::Identifier(ident) = &*assign.lhs
                            && is_read_call(&assign.rhs)
                        {
                            read_bound.insert(ident.node.ident.clone());
                            return;
                        }
                    }
                    unbind(&assign.lhs, read_bound);
                }
                Stmt::AssignModify(target, _, rhs) => {
                    if top {
                        echo_candidates.push(rhs);
                    }
                    // `x += read(…)` binds the SUM, not the face — the echo
                    // (position rule) still renders the read; the name is no
                    // longer its carrier.
                    unbind(target, read_bound);
                }
                Stmt::Expression(expr) => {
                    if top {
                        echo_candidates.push(expr);
                    }
                }
                Stmt::For(for_) => {
                    unbind(&for_.var, read_bound);
                    walk(&for_.body, false, echo_candidates, read_bound);
                }
                Stmt::If(_, body) => walk(body, false, echo_candidates, read_bound),
                Stmt::IfElse(_, arms) => {
                    walk(&arms.0, false, echo_candidates, read_bound);
                    walk(&arms.1, false, echo_candidates, read_bound);
                }
                // A `def` body binds locals; module names are untouched.
                _ => {}
            }
        }

        let mut candidates = Vec::new();
        let mut read_bound = self.read_bound.borrow_mut();
        walk(ast.statement(), true, &mut candidates, &mut read_bound);
        let mut echo = self.echo_at.borrow_mut();
        for expr in candidates {
            if !is_read_call(expr) {
                continue;
            }
            let Expr::Call(callee, _) = &**expr else {
                unreachable!("is_read_call admitted a non-call");
            };
            // The evaluator reports a call site as one of these two spans
            // depending on how the frame was pushed; both name the same call,
            // so matching either is exact, never approximate.
            for span in [expr.span, callee.span] {
                let begin = ast.file_span(span).resolve_span().begin;
                echo.insert((begin.line, begin.column));
            }
        }
    }

    /// Serve one `read(…)`: enforce the ceiling, ask the host, record the
    /// response in call order with its line and echo/quiet position.
    fn read(
        &self,
        path: &str,
        section: Option<&ReadSel>,
        site: Option<(usize, usize)>,
    ) -> anyhow::Result<ReadFace> {
        if self.reads.borrow().len() >= self.max_reads {
            self.over_read_budget.set(true);
            return Err(anyhow::anyhow!(
                "read budget of {} reads per attempt reached",
                self.max_reads
            ));
        }
        let answered = {
            // The program's own armed list rides every read — the
            // read-your-own-writes seam. A live-read host ignores it; the
            // entry-world host overlays it. Separate RefCells, so the host
            // borrow and the armed borrow never conflict.
            let armed = self.armed.borrow();
            let mut host = self.host.borrow_mut();
            match section {
                Some(section) => host.cat(path, section, &armed).map(ReadFace::Section),
                None => host.toc(path, &armed).map(ReadFace::Toc),
            }
        };
        let face = match answered {
            Ok(face) => face,
            Err(fault) => {
                let message = fault.to_string();
                *self.fault.borrow_mut() = Some(fault);
                return Err(anyhow::anyhow!(message));
            }
        };
        let position = match site {
            Some(site) if self.echo_at.borrow().contains(&site) => ReadPosition::Echo,
            _ => ReadPosition::Quiet,
        };
        self.reads.borrow_mut().push(ReadRecord {
            path: path.to_owned(),
            section: section.cloned(),
            // Source lines are 1-based for a reader; the resolver is 0-based.
            line: site.map_or(0, |(line, _)| u32::try_from(line + 1).unwrap_or(u32::MAX)),
            position,
            face: face.clone(),
        });
        Ok(face)
    }

    /// Snapshot the module's top-level bindings as Starlark reprs — the
    /// capture law. Three names stay out: the inert inputs (inputs are not
    /// results), function bindings (a `def` is not a value the run computed),
    /// and read-bound names ([`ScriptEntry::read_bound`] — the read's own
    /// `echo` entry already carries that value, and one fact gets one
    /// carrier).
    fn capture_bindings(&self, module: &Module<'_>) {
        let mut bindings = self.bindings.borrow_mut();
        let read_bound = self.read_bound.borrow();
        for name in module.names() {
            let name = name.as_str();
            if SCRIPT_INPUTS.contains(&name) || read_bound.contains(name) {
                continue;
            }
            if let Some(value) = module.get(name) {
                if value.get_type() == "function" {
                    continue;
                }
                bindings.insert(name.to_owned(), value.to_repr());
            }
        }
    }

    /// Everything the host answered this attempt. Expansion rows are entry
    /// facts the CALLER stamps after eval (the kernel never expands); the
    /// `files` binding is the kernel's own entry fact, recorded here so the
    /// trace prints it (order-bind ruling).
    fn recording(&self) -> ScriptRecording {
        ScriptRecording {
            actor: self.actor.clone(),
            reads: self.reads.borrow().clone(),
            expansions: Vec::new(),
            files: self.files.clone(),
        }
    }
}

/// Where the current builtin was called from, as a resolved 0-based
/// `(line, column)`. `None` when the call did not come from source.
fn call_site(eval: &Evaluator<'_, '_, '_>) -> Option<(usize, usize)> {
    let span = eval.call_stack_top_location()?;
    let begin = span.resolve_span().begin;
    Some((begin.line, begin.column))
}

/// The `section=` boundary, shared by `read` and `put` — one address grammar,
/// one door (run-plane.md § One address grammar, one parser). Two spellings:
///
/// - a **string** — the joined selector coat, handed on for [`ReadSel::parse`]
///   exactly as ever (D-1: the coat splits on `/` and is never widened);
/// - a **list** — the §2.1 segment array, one `{h, n?}` object per heading,
///   raw text taken verbatim. The wire's own machine form, so the engine's
///   section-miss teaching ("feed the row back as an hpath array") is
///   executable on this plane, and a heading whose raw text carries `/` is
///   addressable.
///
/// # Errors
/// Anything outside the two spellings, in the D-1 line's first arm: what can
/// never exist refuses at the boundary — a bare string in the list (the
/// retired v1 spelling, the wire's own single-sourced refusal), an empty
/// list, an out-of-shape member — and the refusal names only forms this
/// plane accepts.
fn section_arg(builtin: &str, value: Option<Value<'_>>) -> anyhow::Result<Option<SectionArg>> {
    let Some(value) = value else { return Ok(None) };
    if value.is_none() {
        return Ok(None);
    }
    if let Some(s) = value.unpack_str() {
        return Ok(Some(SectionArg::Coat(s.to_owned())));
    }
    let Some(list) = starlark::values::list::ListRef::from_value(value) else {
        anyhow::bail!(
            "{builtin}(section=…) takes the joined heading path as a string, or the §2.1 \
             segment array — a list of {{h, n?}} objects, one per heading, raw text; got {}",
            value.to_repr()
        );
    };
    if list.is_empty() {
        anyhow::bail!(
            "{builtin}(section=[]) addresses nothing — a segment list names at least one \
             {{\"h\": …}} segment"
        );
    }
    let mut segs = Vec::with_capacity(list.len());
    for member in list.iter() {
        segs.push(list_segment(member).map_err(|reason| anyhow::anyhow!(reason))?);
    }
    Ok(Some(SectionArg::Segments(segs)))
}

/// One member of a `section=[…]` list → its §2.1 segment, through the one
/// validator ([`segment_of_entry`]) so every refusal is single-sourced.
fn list_segment(member: Value<'_>) -> Result<wire::HpathSeg, String> {
    if let Some(s) = member.unpack_str() {
        return segment_of_entry(None, None, None, Some(s), &member.to_repr());
    }
    let Some(dict) = starlark::values::dict::DictRef::from_value(member) else {
        return Err(format!(
            "a segment is a {{h, n?}} object — one per heading, raw text; got {}",
            member.to_repr()
        ));
    };
    let describe = member.to_repr();
    let mut h: Option<String> = None;
    let mut n: Option<i64> = None;
    for (key, val) in dict.iter() {
        match key.unpack_str() {
            Some("h") => {
                let Some(text) = val.unpack_str() else {
                    return Err(format!(
                        "a segment's `h` is raw heading text (a string); got {} in {describe}",
                        val.to_repr()
                    ));
                };
                h = Some(text.to_owned());
            }
            Some("n") => {
                let Some(k) = val.unpack_i32() else {
                    return Err(format!(
                        "a segment's `n` is a 1-based occurrence among same-text siblings \
                         (an int); got {} in {describe}",
                        val.to_repr()
                    ));
                };
                n = Some(i64::from(k));
            }
            _ => return segment_of_entry(None, None, Some(&key.to_str()), None, &describe),
        }
    }
    segment_of_entry(h.as_deref(), n, None, None, &describe)
}

/// The script plane's entire builtin surface: one effectful reader and the
/// caller's identity. No descriptor constructors (they arrive at U2), no exec,
/// no enumeration — decision #17 stands permanently.
#[starlark_module]
fn script_api(builder: &mut GlobalsBuilder) {
    /// `read(path)` → the toc face `{rev, fm, toc}` (§4.1);
    /// `read(path, section=…)` → the cat face `{text, rev}` (§4.2). The only
    /// effectful builtin; every response is recorded, which is what makes
    /// replay byte-identical. There is no whole-file body.
    ///
    /// `section` is the joined selector string, or the §2.1 segment array —
    /// the same two spellings the toc face publishes (`section` / `hpath` per
    /// row), so any row feeds back verbatim.
    fn read<'v>(
        #[starlark(require = pos)] path: String,
        #[starlark(require = named)] section: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Value<'v>> {
        let selector = match section_arg("read", section)? {
            None => None,
            Some(SectionArg::Coat(s)) => Some(ReadSel::parse(&s)),
            Some(SectionArg::Segments(segs)) => Some(ReadSel::Hpath { hpath: segs }),
        };
        let site = call_site(eval);
        let heap = eval.heap();
        let face = script(eval)?.read(&path, selector.as_ref(), site)?;
        Ok(alloc_read_face(heap, &face))
    }

    /// `put(path, props={…})` / `put(path, section=…, append=…)` → arms wire
    /// `splice.plan_edits[]` items and returns nothing.
    ///
    /// PURE: it performs no I/O and consults no host at call time. The armed
    /// items are inert until the consumer plane commits the whole list in ONE
    /// guarded splice. `props` arms one `set_property` per key, keys sorted;
    /// `append` arms one section-addressed `append`. Ruling (B′): these are the
    /// wire's second edit dialect, spoken verbatim — no third grammar.
    ///
    /// `section` as on `read`: the joined string, or the §2.1 segment array —
    /// the array is how a heading whose raw text carries `/` is addressed
    /// (D-1: the joined spelling splits, and the coat is never widened).
    fn put<'v>(
        #[starlark(require = pos)] path: String,
        #[starlark(require = named)] props: Option<BTreeMap<String, String>>,
        #[starlark(require = named)] section: Option<Value<'v>>,
        #[starlark(require = named)] append: Option<String>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        // Keys sorted: the same group order `wire-serve::plan::lower` and the
        // MCP `put` face build, so the armed list aligns 1:1 with the commit.
        let props: Vec<(String, String)> = props.unwrap_or_default().into_iter().collect();
        let section = section_arg("put", section)?;
        let items = plan_items(&props, section.as_ref(), append.as_deref())
            .map_err(|reason| anyhow::anyhow!("{reason}"))?;
        let line =
            call_site(eval).map_or(0, |(line, _)| u32::try_from(line + 1).unwrap_or(u32::MAX));
        let entry = script(eval)?;
        // Effects mode: the same grammar applies NOW through the host — no
        // arm, no CAS (script-effects ruling; the arm-time laws are the pure
        // TRANSACTION's and do not reach this model).
        if entry.live {
            entry.put_live(&path, items, line)?;
            return Ok(NoneType);
        }
        // Two frames are always on the stack at a top-level arm — the module's
        // own and this builtin's — so a top-level arm reports depth 0 and each
        // enclosing `def` adds one. Recorded for the trace only: an applied
        // effect renders at ANY depth (there is no suppression in v1).
        let depth = u32::try_from(eval.call_stack_count().saturating_sub(2)).unwrap_or(u32::MAX);
        entry.arm(&path, items, line, depth)?;
        Ok(NoneType)
    }

    /// `me()` → the caller's own identity, threaded in by the host (§9). The
    /// engine mints no identity.
    fn me(eval: &mut Evaluator<'_, '_, '_>) -> anyhow::Result<String> {
        Ok(script(eval)?.actor.clone())
    }
}

/// The effects-mode surface (script-effects ruling, 2026-08-13): joins the
/// globals EXACTLY when the submission admits the builtin by name — a pure
/// script never holds it, which is what keeps #17 standing on the pure path.
#[starlark_module]
fn script_run_api(builder: &mut GlobalsBuilder) {
    /// `run(page, task=None, args=[], env={}, dry=False)` → executes the
    /// addressed task NOW through the run plane and returns its § A.8 row as
    /// a value — state, exit code, stdout observable in-program;
    /// run-then-decide works. Plane refusals RETURN as rows (branchable);
    /// only shape errors fault the program.
    fn run<'v>(
        #[starlark(require = pos)] page: String,
        #[starlark(require = named)] task: Option<String>,
        #[starlark(require = named)] args: Option<UnpackList<String>>,
        #[starlark(require = named)] env: Option<BTreeMap<String, String>>,
        #[starlark(require = named)] dry: Option<bool>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Value<'v>> {
        let line =
            call_site(eval).map_or(0, |(line, _)| u32::try_from(line + 1).unwrap_or(u32::MAX));
        let row = script(eval)?.run_live(
            &page,
            task.as_deref(),
            args.map(|l| l.items).unwrap_or_default(),
            env.unwrap_or_default(),
            dry.unwrap_or(false),
            line,
        )?;
        let heap = eval.heap();
        Ok(alloc_json(heap, &row))
    }
}

/// The effects-mode measurement surface (`token_count` ruling leg B,
/// 2026-08-13): joins the globals EXACTLY when the submission admits it by
/// name. A measurement, not a mutation — the count is the program's value
/// and nothing is journaled, so a top-level `n = token_count(…)` rides the
/// bindings echo like any computed name.
#[starlark_module]
fn script_token_count_api(builder: &mut GlobalsBuilder) {
    /// `token_count(text)` → the real token cost of `text` as an int,
    /// measured NOW through the host's bound harness endpoint (a
    /// `count_tokens` API call — the engine never counts tokens itself).
    /// ONE law: the string is measured VERBATIM — the tool face's `{text}`
    /// arm; the builtin resolves no refs and no sections, so a program
    /// measures exactly what its own `read()` served or what it built
    /// (the stored/served split is the tool face's, structurally absent
    /// here). A lane with no endpoint refuses "unbound"; the endpoint's
    /// own refusal faults the program with its words carried whole.
    fn token_count(
        #[starlark(require = pos)] text: String,
        eval: &mut Evaluator<'_, '_, '_>,
    ) -> anyhow::Result<i64> {
        script(eval)?.token_count_live(&text)
    }
}

/// Allocate a JSON value as its natural Starlark value — dicts, lists,
/// strings, ints, bools, None. Numbers outside `i64` fall back to their
/// string form rather than inventing a float the row never carried.
fn alloc_json<'v>(heap: Heap<'v>, value: &serde_json::Value) -> Value<'v> {
    match value {
        serde_json::Value::Null => Value::new_none(),
        serde_json::Value::Bool(b) => Value::new_bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                heap.alloc(i)
            } else {
                heap.alloc(n.to_string())
            }
        }
        serde_json::Value::String(s) => heap.alloc(s.as_str()),
        serde_json::Value::Array(items) => {
            let values: Vec<Value<'v>> = items.iter().map(|v| alloc_json(heap, v)).collect();
            heap.alloc(values)
        }
        serde_json::Value::Object(map) => heap.alloc(AllocDict(
            map.iter()
                .map(|(k, v)| (heap.alloc(k.as_str()), alloc_json(heap, v))),
        )),
    }
}

/// Allocate a recorded read response as its Starlark face.
fn alloc_read_face<'v>(heap: Heap<'v>, face: &ReadFace) -> Value<'v> {
    match face {
        ReadFace::Toc(facts) => alloc_toc(heap, facts),
        ReadFace::Section(facts) => alloc_section(heap, facts),
    }
}

/// The toc face: `{rev, fm, toc, words}`. A frontmatter key the page does not
/// carry is absent from `fm` — absence stays absence, never a synthesized `""`.
/// `words` is the wire toc face's own `words_total`: a script may see it
/// because the wire answered it (ruling 2026-08-07).
fn alloc_toc<'v>(heap: Heap<'v>, facts: &TocFacts) -> Value<'v> {
    let fm = heap.alloc(AllocDict(
        facts
            .fm
            .iter()
            .map(|(k, v)| (heap.alloc(k.as_str()), heap.alloc(v.as_str()))),
    ));
    let toc: Vec<Value<'v>> = facts
        .toc
        .iter()
        .map(|entry| {
            let mut row = vec![
                (heap.alloc("section"), heap.alloc(entry.section.as_str())),
                (heap.alloc("anchor"), opt_str(heap, entry.anchor.as_deref())),
                (heap.alloc("rev"), heap.alloc(entry.rev.as_str())),
            ];
            // The feedable machine address (D-1): the raw §2.1 segments behind
            // `section`, publishable straight back into `section=` on either
            // builtin. Absent on an anchor-only row — absence stays absence.
            if !entry.hpath.is_empty() {
                let segs: Vec<Value<'v>> = entry
                    .hpath
                    .iter()
                    .map(|seg| {
                        let mut fields = vec![(heap.alloc("h"), heap.alloc(seg.h.as_str()))];
                        if let Some(n) = seg.n {
                            fields.push((heap.alloc("n"), heap.alloc(i64::from(n))));
                        }
                        heap.alloc(AllocDict(fields))
                    })
                    .collect();
                row.push((heap.alloc("hpath"), heap.alloc(segs)));
            }
            heap.alloc(AllocDict(row))
        })
        .collect();
    heap.alloc(AllocDict([
        (heap.alloc("rev"), heap.alloc(facts.rev.as_str())),
        (heap.alloc("fm"), fm),
        (heap.alloc("toc"), heap.alloc(toc)),
        (
            heap.alloc("words"),
            heap.alloc(i32::try_from(facts.words).unwrap_or(i32::MAX)),
        ),
    ]))
}

/// The cat face: the section TEXT itself, a plain string (read alignment,
/// script-effects ruling — *"`read()` returns actual VALUES the agent computes
/// with"*; `"x" in read(p, section=s)` is a legal program). The section's rev
/// still rides the recording, where the threading law reads it.
fn alloc_section<'v>(heap: Heap<'v>, facts: &SecFacts) -> Value<'v> {
    heap.alloc(facts.text.as_str())
}

/// Peak eval-heap bytes as `u64` (saturating). Peak matches `set_max_heap_size`;
/// post-eval GC cannot deflate it.
fn heap_bytes(heap: Heap<'_>) -> u64 {
    u64::try_from(heap.peak_allocated_bytes()).unwrap_or(u64::MAX)
}

/// Metered outcome of one rule run: typed result + exact post-eval fuel/mem.
/// Never-reached eval → `0`/`0`; bomb at ceiling reports that ceiling.
pub(crate) struct RuleRun {
    pub(crate) fuel_used: u64,
    pub(crate) mem_used: u64,
    pub(crate) outcome: Result<Vec<Effect>, EvalError>,
    /// The starlark-level class of the fault, captured WHERE THE STARLARK
    /// ERROR STILL EXISTS.
    ///
    /// [`EvalError`] cannot carry it: by the time an error becomes one, the
    /// `ErrorKind` is gone and only a message string survives — so a
    /// downstream classifier would have to match prose, which is exactly
    /// what design § Amendments / A1 forbids ("never classified by string").
    /// Captured here instead, the phase classes (`effect_at_load`,
    /// `declare_at_fire`) reach a row intact and a misclassification becomes
    /// impossible rather than unlikely.
    pub(crate) fault_class: Option<FaultClass>,
}

impl RuleRun {
    /// A run that never reached eval (source-cap, nesting, or evaluator setup) —
    /// no fuel spent, the typed authoring/setup fault.
    fn failed(outcome: EvalError) -> Self {
        Self {
            fuel_used: 0,
            mem_used: 0,
            outcome: Err(outcome),
            // Nothing was evaluated, so no starlark error exists to class.
            fault_class: None,
        }
    }
}

/// Run one rule's `on_change(event)`; batch path discards metering. See
/// [`run_rule_metered`].
pub(crate) fn run_rule(
    globals: &Globals,
    rule: &Rule,
    event: &ChangeEvent,
    limits: EvalLimits,
) -> Result<Vec<Effect>, EvalError> {
    run_rule_metered(globals, rule, event, limits).outcome
}

/// Run one task's `run(ctx)` — same machinery as change plane ([`metered_eval`]);
/// only entry name, injected value, and provenance differ.
pub(crate) fn run_task(
    globals: &Globals,
    task: &Rule,
    ctx: &RunCtx,
    limits: EvalLimits,
) -> Result<Vec<Effect>, EvalError> {
    metered_eval(globals, task, &EvalEntry::Run(ctx), limits).outcome
}

/// What one block's LOAD phase produced (design § 2.2, as amended by A1).
///
/// The load phase evaluates a block's module top level to publish what the
/// block declares, and applies nothing. `effects` is therefore empty on
/// success — returned rather than asserted away, so a caller can PROVE
/// purity from the outcome instead of trusting a comment.
#[derive(Debug)]
pub struct BlockLoad {
    /// What `declare()` published, in call order. Empty when the block
    /// declares nothing — which is the consent gate's whole subject: such a
    /// block is not a target (§ 2.2, `not_declared`).
    pub declarations: Vec<Declaration>,
    /// The frozen module, present exactly when the load succeeded. This is
    /// what a fire calls; caching it by block rev is what makes a warm fire
    /// one function call (§ 2.2 Price).
    pub module: Option<FrozenModule>,
    /// Empty on success. Anything here would mean the phase gate let
    /// something through — a defect to surface, not a state to handle.
    pub effects: Vec<Effect>,
    /// The classified fault, when the top level refused. `effect_at_load`
    /// here is an authoring fault: the block tried to ACT while declaring.
    pub fault: Option<BlockFault>,
    /// Interpreter steps spent.
    pub fuel_used: u64,
    /// Peak heap bytes.
    pub mem_used: u64,
}

/// One classified evaluation fault, ready for a `run` row's `fault` object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockFault {
    /// The wire class (`effect_at_load`, `name_error`, `parse`, …), decided
    /// by downcast while the starlark error still existed.
    pub class: FaultClass,
    /// The evaluator's own text, verbatim — never reworded, because the
    /// author has to match it against their own source.
    pub reason: String,
    /// 1-based call-site line, when the error carried a span.
    pub line: Option<u32>,
}

/// Evaluate one starlark block's TOP LEVEL in the load phase, then FREEZE it.
///
/// No hook is looked up and none is called: publishing what a block declares
/// IS evaluating its top level, exactly as on the script plane. Every effect
/// builtin the top level reaches refuses `effect_at_load` at the accessor
/// gate, so a block that tries to act while declaring says so at its own
/// line rather than acting.
///
/// `prelude` is § 2.2's caller source, evaluated **into the same module**
/// before the block's own top level, so its bindings are frozen with the
/// module. Two `eval_module` calls rather than one concatenated source: each
/// carries its own spans, so a block's `fault.line` is a line in the BLOCK —
/// the one number an author navigates by — instead of an offset into a text
/// they never wrote.
#[must_use]
pub fn load_block(
    source: &str,
    prelude: Option<&str>,
    ctx: &RunCtx,
    limits: EvalLimits,
) -> BlockLoad {
    let block = Rule::new(&ctx.task, source.to_owned());
    // Same large stack as every other entry: pathological nesting must fault,
    // never abort the process. The store is built INSIDE the closure because
    // it is full of `Cell`/`RefCell` and therefore not `Sync` — the shipped
    // planes do the same.
    let (run, module, declarations) =
        on_eval_stack(|| eval_and_freeze(&block, prelude, ctx, limits));
    let (effects, fault) = match run.outcome {
        Ok(effects) => (effects, None),
        Err(e) => (
            Vec::new(),
            Some(BlockFault {
                // The class was taken by downcast at the starlark boundary;
                // `EvalError` no longer carries the kind, so falling back to
                // a guess here would be the string-matching A1 forbids.
                class: run.fault_class.unwrap_or(FaultClass::Runtime),
                reason: eval_error_reason(&e),
                line: eval_error_line(&e),
            }),
        ),
    };
    BlockLoad {
        declarations,
        // A faulted load has nothing callable to hand on, whatever the
        // freeze itself did.
        module: fault.is_none().then_some(module).flatten(),
        effects,
        fault,
        fuel_used: run.fuel_used,
        mem_used: run.mem_used,
    }
}

/// The load phase's emit store: run provenance, gated to [`EffectPhase::Load`].
fn load_emit_store(ctx: &RunCtx) -> EmitStore<'static> {
    EmitStore::new(
        &ctx.task,
        Provenance::Run {
            invocation_id: ctx.invocation_id.clone(),
            root_at_eval: ctx.root_at_eval.clone(),
        },
        0,
    )
    .in_phase(EffectPhase::Load)
}

/// Evaluate a module's top level under `store` and freeze it, metered and
/// panic-caught exactly as [`metered_eval`] is.
///
/// A path of its own rather than a flag on `metered_eval`, because the two
/// differ in the one thing that matters: this one's module must OUTLIVE the
/// evaluation. `metered_eval` owns its module inside
/// [`Module::with_temp_heap`] and drops it, which is right for every plane
/// that answers with effects and wrong for the only plane that answers with a
/// callable.
fn eval_and_freeze(
    rule: &Rule,
    prelude: Option<&str>,
    ctx: &RunCtx,
    limits: EvalLimits,
) -> (RuleRun, Option<FrozenModule>, Vec<Declaration>) {
    if let Err(e) = check_source_size(rule, limits) {
        return (RuleRun::failed(e), None, Vec::new());
    }
    if let Err(e) = check_nesting_depth(rule) {
        return (RuleRun::failed(e), None, Vec::new());
    }
    let step_guard = limits.fuel.max(1);
    let mem_guard = usize::try_from(limits.mem).unwrap_or(usize::MAX).max(1);
    let store = PlaneStore::Emit(load_emit_store(ctx));
    let globals = hook_globals();

    // AssertUnwindSafe: the panic path discards the store unread → budget.
    let evaluated = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        Module::with_temp_heap(|module| {
            let mut eval = Evaluator::new(&module);
            if let Err(e) = arm_guards(&mut eval, rule, step_guard, mem_guard, limits) {
                return (e, None);
            }
            eval.extra = Some(&store);
            // The prelude first, into the SAME module — its bindings are the
            // block's to use and the freeze's to keep — then the block's own
            // top level. Each carries its own spans, so the block's
            // `fault.line` is a line in the block.
            let mut outcome = Ok(());
            for (id, source) in [
                Some(("prelude", prelude)),
                Some((&*rule.id, Some(&*rule.source))),
            ]
            .into_iter()
            .flatten()
            .filter_map(|(id, source)| source.map(|s| (id, s)))
            {
                if outcome.is_err() {
                    break;
                }
                match AstModule::parse(id, source.to_owned(), &script_dialect()) {
                    Ok(ast) => outcome = eval.eval_module(ast, &globals).map(|_| ()),
                    Err(e) => {
                        return (
                            RuleRun::failed(EvalError::Parse {
                                rule_id: id.to_owned(),
                                reason: e.to_string(),
                            }),
                            None,
                        );
                    }
                }
            }
            let used_steps = eval.get_total_tick_count();
            let used_mem = heap_bytes(module.heap());
            drop(eval);
            let effects = match &store {
                PlaneStore::Emit(store) => store.effects.take(),
                PlaneStore::Script(_) => Vec::new(),
            };
            let over_budget = used_steps > limits.fuel || used_mem > limits.mem;
            let (fault_class, ending) = ending_of(outcome.err().as_ref());
            let faulted = !matches!(ending, Ending::Completed);
            let outcome = classify_outcome(
                rule,
                &EvalEntry::RunLoad(&NO_CTX),
                effects,
                &Ended {
                    ending,
                    over_budget,
                },
                limits,
            );
            // Freezing a module whose top level faulted would hand a caller a
            // half-built environment; the fault is the answer instead.
            let frozen = if faulted { None } else { module.freeze().ok() };
            (
                RuleRun {
                    fuel_used: used_steps,
                    mem_used: used_mem,
                    outcome,
                    fault_class: if over_budget {
                        Some(FaultClass::Budget)
                    } else {
                        fault_class
                    },
                },
                frozen,
            )
        })
    }));

    // The declarations survive the panic boundary because the store outlives
    // the closure — a bomb that dies mid-block still says what it declared
    // before it died, which is data a caller can act on.
    let declarations = match &store {
        PlaneStore::Emit(store) => store.declarations.take(),
        PlaneStore::Script(_) => Vec::new(),
    };
    match evaluated {
        Ok((run, frozen)) => (run, frozen, declarations),
        // Only reachable panic: resource-overflow assert → budget at ceiling.
        Err(_panic) => (
            RuleRun {
                fuel_used: limits.fuel,
                mem_used: limits.mem,
                outcome: Err(budget(limits)),
                fault_class: Some(FaultClass::Budget),
            },
            None,
            declarations,
        ),
    }
}

/// One starlark error (or its absence) as the classified pair every two-phase
/// path needs: the fault CLASS, taken by downcast while the error still
/// exists, and the [`Ending`] the outcome is built from. One owner, so the
/// load and fire paths cannot classify the same error differently.
fn ending_of(error: Option<&starlark::Error>) -> (Option<FaultClass>, Ending) {
    match error {
        None => (None, Ending::Completed),
        Some(e) => (
            Some(classify_starlark_fault(e)),
            Ending::Faulted {
                depth_overflow: is_depth_overflow(e),
                fault: Some(e.to_string()),
                fault_line: starlark_fault_line(e),
            },
        ),
    }
}

/// A `RunCtx` used only to name the entry kind in [`classify_outcome`]'s
/// diagnosis. The load phase calls no hook, so the ctx's own facts are never
/// read on that path — one static beats threading a borrow through purely to
/// satisfy a match arm that ignores it.
static NO_CTX: std::sync::LazyLock<RunCtx> = std::sync::LazyLock::new(RunCtx::default);

/// Check the caller's `prelude` on its own, before any block is looked at —
/// § 2.2's *`prelude_invalid` refuses before any block*.
///
/// It is evaluated in the LOAD phase like a block, which is what makes
/// "nothing but pure functions" enforced rather than requested: a prelude
/// whose top level calls an effect builtin faults `effect_at_load` here.
///
/// **A prelude may not DECLARE.** The consent gate is the page's — *`run`
/// executes what the page declares* — and the prelude is CALLER source that
/// evaluates into the same module, in the load phase, before the block's own
/// top level. `declare()` and `exec()` are load-phase builtins, so they are
/// admitted there; and a fire takes `declarations.first()`. A caller could
/// therefore ship
/// `declare(impl = exec("bash", cmd = "…"))` as its prelude and make EVERY
/// anchored starlark fence on EVERY addressed page a fire target running
/// caller-authored process bytes — including a fence that declares nothing,
/// which is the exact case `not_declared` exists to refuse. That is not a
/// sandbox escape; it is page-authored consent replaced by caller-authored
/// consent, and hook-11/13 arm this fleet-wide. (PR 195 review, `fa5da9ec`,
/// S3.)
///
/// The guard was armed against the wrong set: this function read only
/// `.fault`, and `declare`/`exec` are not effect builtins, so `effect_at_load`
/// never fires for them. It now refuses a prelude that produced ANY
/// declaration.
///
/// **No new fault class is needed** — the caller sees the published
/// `prelude_invalid`, which is what the row already renders for anything this
/// function returns.
///
/// Returns the fault, or `None` when the prelude is sound.
#[must_use]
pub fn check_prelude(source: &str, ctx: &RunCtx, limits: EvalLimits) -> Option<BlockFault> {
    let loaded = load_block(source, None, ctx, limits);
    if let Some(fault) = loaded.fault {
        return Some(fault);
    }
    // Declarations AND the exec values a prelude sank — both are consent
    // material, and the refusal names WHICH was found so the remedy is one
    // line. It refuses REGARDLESS of whether the page declares: silently
    // dropping a caller's declaration would be its own defect class.
    let execs = loaded
        .declarations
        .iter()
        .filter(|d| matches!(d.entry, DeclaredEntry::Exec(_)))
        .count();
    if !loaded.declarations.is_empty() {
        let found = if execs > 0 {
            format!("{} declaration(s), {execs} of them an `exec(...)` value", loaded.declarations.len())
        } else {
            format!("{} declaration(s)", loaded.declarations.len())
        };
        return Some(BlockFault {
            class: FaultClass::PreludeInvalid,
            reason: format!(
                "the prelude carried consent material — {found}. Consent is the PAGE's: \
                 `run` executes what the page declares, a `task.<name>` binding in \
                 frontmatter or a `declare(...)` in the block. Move the declaration into \
                 the block it belongs to; a prelude carries shared helpers, not entries"
            ),
            line: None,
        });
    }
    None
}

/// What one FIRE produced (design § 2.2 Evaluation — fire).
#[derive(Debug)]
pub struct BlockFire {
    /// The entry's return value as JSON — § 1.3's law, *the entry's return is
    /// the answer; `None` is no answer*.
    pub value: Option<serde_json::Value>,
    /// The md effect descriptors the entry emitted, in call order. The
    /// CALLER realizes them through the ordinary doors under the page's
    /// `caps:` — the kernel emits descriptors and opens no door, here as
    /// everywhere.
    pub effects: Vec<Effect>,
    /// The classified fault, when the entry refused.
    pub fault: Option<BlockFault>,
    /// Interpreter steps spent.
    pub fuel_used: u64,
    /// Peak heap bytes.
    pub mem_used: u64,
}

/// Call one frozen entry with a JSON input, under a `Fire` store.
///
/// The seam is [`Heap::access_owned_frozen_value`] — the crossing point that
/// lets a value frozen under one evaluator be called by a fresh one.
/// (`Module::import_public_symbols` is NOT: it imports under
/// `Visibility::Private` and `Module::get` then answers `None`. Measured; it
/// cost a red test, and the note is here so it costs no one a second.)
///
/// # Errors
/// [`EvalError::MissingEntry`] when the frozen module carries no binding of
/// that name. Everything the ENTRY itself does — a fault, a budget refusal, an
/// unserializable answer — rides the returned [`BlockFire`], because those are
/// results of a run that happened, not reasons it could not start.
pub fn fire_entry(
    module: &FrozenModule,
    entry: &str,
    input: &serde_json::Value,
    ctx: &RunCtx,
    host: &dyn FireHost,
    limits: EvalLimits,
) -> Result<BlockFire, EvalError> {
    let Ok(entry_value) = module.get(entry) else {
        return Err(EvalError::MissingEntry {
            rule_id: ctx.task.clone(),
            expected: "run",
            wrong_plane: None,
        });
    };
    let rule = Rule::new(&ctx.task, String::new());
    let step_guard = limits.fuel.max(1);
    let mem_guard = usize::try_from(limits.mem).unwrap_or(usize::MAX).max(1);

    // The store is built inside the eval thread — it is `Cell`/`RefCell`
    // throughout and therefore not `Sync`, exactly like every shipped plane's.
    let fired = on_eval_stack(|| {
        let store = PlaneStore::Emit(
            EmitStore::new(
                &ctx.task,
                Provenance::Run {
                    invocation_id: ctx.invocation_id.clone(),
                    root_at_eval: ctx.root_at_eval.clone(),
                },
                0,
            )
            .with_host(host),
        );
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            Module::with_temp_heap(|fresh| {
                fresh.frozen_heap().add_reference(entry_value.owner());
                let callable = fresh.heap().access_owned_frozen_value(&entry_value);
                let arg = alloc_json(fresh.heap(), input);
                let mut eval = Evaluator::new(&fresh);
                if let Err(e) = arm_guards(&mut eval, &rule, step_guard, mem_guard, limits) {
                    return (e, None);
                }
                eval.extra = Some(&store);
                let called = eval.eval_function(callable, &[arg], &[]);
                // The value must cross the heap boundary as JSON HERE: the
                // heap dies with this closure.
                let rendered = called.map(|v| json_of_value(v));
                let used_steps = eval.get_total_tick_count();
                let used_mem = heap_bytes(fresh.heap());
                drop(eval);
                fire_outcome(&rule, &store, rendered, used_steps, used_mem, limits)
            })
        }))
    });

    let (run, value) = match fired {
        Ok(pair) => pair,
        Err(_panic) => (
            RuleRun {
                fuel_used: limits.fuel,
                mem_used: limits.mem,
                outcome: Err(budget(limits)),
                fault_class: Some(FaultClass::Budget),
            },
            None,
        ),
    };
    let (effects, fault) = match run.outcome {
        Ok(effects) => (effects, None),
        Err(e) => (
            Vec::new(),
            Some(BlockFault {
                class: run.fault_class.unwrap_or(FaultClass::Runtime),
                reason: eval_error_reason(&e),
                line: eval_error_line(&e),
            }),
        ),
    };
    Ok(BlockFire {
        value,
        effects,
        fault,
        fuel_used: run.fuel_used,
        mem_used: run.mem_used,
    })
}

/// Turn one fire's raw call result into the metered outcome plus the JSON
/// answer. Split out so [`fire_entry`]'s closure stays readable.
fn fire_outcome(
    rule: &Rule,
    store: &PlaneStore<'_>,
    called: Result<anyhow::Result<serde_json::Value>, starlark::Error>,
    used_steps: u64,
    used_mem: u64,
    limits: EvalLimits,
) -> (RuleRun, Option<serde_json::Value>) {
    let effects = match store {
        PlaneStore::Emit(store) => store.effects.take(),
        PlaneStore::Script(_) => Vec::new(),
    };
    let over_budget = used_steps > limits.fuel || used_mem > limits.mem;
    let (fault_class, ending, value) = match called {
        // Ran, and its answer serializes: the ordinary case. A starlark
        // `None` return collapses to NO value, never to a JSON `null` —
        // § 1.3's law is *the entry's return is the answer; `None` is no
        // answer*, and a row carrying `"value": null` would be an answer that
        // says nothing rather than the absence of one. Nothing is lost:
        // starlark has exactly one none, so the two cases were never
        // distinguishable at the source.
        Ok(Ok(serde_json::Value::Null)) => (None, Ending::Completed, None),
        Ok(Ok(value)) => (None, Ending::Completed, Some(value)),
        // Ran, but returned something outside § 2.2's admitted set. The
        // run is NOT a starlark fault — the program is fine and the answer
        // is not — so it classes `reply_shape` at the caller, carried here
        // as a runtime fault with the type in its words.
        Ok(Err(shape)) => (
            Some(FaultClass::ReplyShape),
            Ending::Faulted {
                depth_overflow: false,
                fault: Some(shape.to_string()),
                fault_line: None,
            },
            None,
        ),
        Err(e) => (
            Some(classify_starlark_fault(&e)),
            Ending::Faulted {
                depth_overflow: is_depth_overflow(&e),
                fault: Some(e.to_string()),
                fault_line: starlark_fault_line(&e),
            },
            None,
        ),
    };
    let outcome = classify_outcome(
        rule,
        &EvalEntry::RunLoad(&NO_CTX),
        effects,
        &Ended {
            ending,
            over_budget,
        },
        limits,
    );
    (
        RuleRun {
            fuel_used: used_steps,
            mem_used: used_mem,
            outcome,
            fault_class: if over_budget {
                Some(FaultClass::Budget)
            } else {
                fault_class
            },
        },
        value,
    )
}

/// The evaluator's own words for one fault — verbatim where the error kept
/// them, its `Display` otherwise.
fn eval_error_reason(error: &EvalError) -> String {
    match error {
        EvalError::Parse { reason, .. } | EvalError::Runtime { reason, .. } => reason.clone(),
        other => other.to_string(),
    }
}

/// The 1-based source line, where the fault carried one.
fn eval_error_line(error: &EvalError) -> Option<u32> {
    match error {
        EvalError::Runtime { line, .. } => *line,
        _ => None,
    }
}

/// Eval plane: entry point, injected value, stamped provenance. The two hooked
/// planes take one hook each and never cross (wrong-plane source →
/// [`EvalError::MissingEntry`]); the script plane takes no hook at all — its
/// module top level IS the program.
pub(crate) enum EvalEntry<'a> {
    /// `on_change(event)` — change-plane provenance from the event's diff.
    Change(&'a ChangeEvent),
    /// `run(ctx)` — Run-plane provenance from the caller-supplied facts.
    Run(&'a RunCtx),
    /// The amended `run` entry's LOAD phase (design § 2.2 / A1): evaluate a
    /// block's module top level to publish what it declares, and call NO
    /// hook — the module top level is the whole of this phase, exactly as on
    /// the script plane. The store it builds is phase-gated, so any effect
    /// builtin reached from that top level refuses `effect_at_load`.
    RunLoad(&'a RunCtx),
    /// Inline source, kernel entry #3 — no hook, live `read()` through the
    /// injected host, every response recorded.
    Script(&'a ScriptEntry<'a>),
}

impl<'a> EvalEntry<'a> {
    /// The hook this plane calls, or `None` when the module top level is
    /// already the whole program.
    fn hook(&self) -> Option<&'static str> {
        match self {
            EvalEntry::Change(_) => Some("on_change"),
            EvalEntry::Run(_) => Some("run"),
            // The load phase calls nothing: publishing declarations IS
            // evaluating the top level, so there is no hook to look up.
            EvalEntry::RunLoad(_) | EvalEntry::Script(_) => None,
        }
    }

    /// The OTHER hooked plane's entry name (for the wrong-plane diagnosis).
    fn other(&self) -> Option<&'static str> {
        match self {
            EvalEntry::Change(_) => Some("run"),
            EvalEntry::Run(_) => Some("on_change"),
            EvalEntry::RunLoad(_) | EvalEntry::Script(_) => None,
        }
    }

    /// What this plane puts behind `Evaluator::extra`.
    fn store(&self, rule: &Rule) -> PlaneStore<'a> {
        match self {
            EvalEntry::Change(event) => PlaneStore::Emit(EmitStore::new(
                &rule.id,
                Provenance::Change {
                    fingerprint_before: event.fingerprint_before.clone(),
                    fingerprint_after: event.fingerprint_after.clone(),
                },
                event.depth,
            )),
            EvalEntry::Run(ctx) => PlaneStore::Emit(EmitStore::new(
                &ctx.task,
                Provenance::Run {
                    invocation_id: ctx.invocation_id.clone(),
                    root_at_eval: ctx.root_at_eval.clone(),
                },
                0,
            )),
            // The load phase's store carries the same run provenance and
            // the phase gate — one store type, one gate, no second surface.
            EvalEntry::RunLoad(ctx) => PlaneStore::Emit(
                EmitStore::new(
                    &ctx.task,
                    Provenance::Run {
                        invocation_id: ctx.invocation_id.clone(),
                        root_at_eval: ctx.root_at_eval.clone(),
                    },
                    0,
                )
                .in_phase(EffectPhase::Load),
            ),
            EvalEntry::Script(entry) => PlaneStore::Script(entry),
        }
    }

    /// Allocate the injected argument value on the eval heap. The script plane
    /// calls no hook, so it passes nothing.
    fn alloc<'v>(&self, heap: Heap<'v>) -> Value<'v> {
        match self {
            EvalEntry::Change(event) => alloc_event(heap, event),
            EvalEntry::Run(ctx) => alloc_ctx(heap, ctx),
            // No hook is called, so nothing is passed.
            EvalEntry::RunLoad(_) | EvalEntry::Script(_) => Value::new_none(),
        }
    }

    /// The grammar this plane parses under.
    fn dialect(&self) -> Dialect {
        match self {
            EvalEntry::Change(_) | EvalEntry::Run(_) => rule_dialect(),
            // A declaring block's top level is a program (`declare(...)`,
            // helper defs), so it needs the top-level-statement grammar the
            // script plane uses. `load` stays disabled on every entry.
            EvalEntry::RunLoad(_) | EvalEntry::Script(_) => script_dialect(),
        }
    }

    /// Bind this plane's module-level inputs before eval. The hooked planes
    /// pass their facts as the hook argument and bind nothing; the script
    /// plane has no argument, so its inert inputs arrive as bindings.
    fn bind(&self, module: &Module<'_>) {
        if let EvalEntry::Script(entry) = self {
            let heap = module.heap();
            // `args` is a dict (the `RunCtx::env` shape) — inert data, keys
            // sorted by the BTreeMap it came from.
            let args = heap.alloc(AllocDict(
                entry
                    .args
                    .iter()
                    .map(|(k, v)| (heap.alloc(k.as_str()), heap.alloc(v.as_str()))),
            ));
            let files: Vec<Value<'_>> =
                entry.files.iter().map(|s| heap.alloc(s.as_str())).collect();
            module.set("args", args);
            module.set("files", heap.alloc(files));
        }
    }
}

/// Run one inline script at kernel entry #3 — the same metered machinery as the
/// other two planes, minus the hook lookup. Telemetry is unconditional and the
/// recording is returned whatever the outcome, so a refused attempt still says
/// what it read.
pub(crate) fn run_script(
    source: &str,
    ctx: &ScriptCtx,
    limits: ScriptLimits,
    host: &mut dyn ScriptHost,
) -> ScriptEval {
    let rule = Rule::new(&ctx.id, source.to_owned());
    let started = std::time::Instant::now();
    // The entry lives entirely on the eval stack (its cells are single-threaded
    // by construction); only its harvested facts cross back.
    let (run, recording, bindings, over_read_budget, over_run_budget, armed, arm_refusal) =
        on_eval_stack(|| {
            let entry = ScriptEntry::new(ctx, limits, host);
            let globals = script_globals(&ctx.effects);
            let run = metered_eval(&globals, &rule, &EvalEntry::Script(&entry), limits.eval);
            let bindings = entry.bindings.borrow().clone();
            let armed = entry.armed.borrow().clone();
            let arm_refusal = entry.arm_refusal.borrow().clone();
            (
                run,
                entry.recording(),
                bindings,
                entry.over_read_budget.get(),
                entry.over_run_budget.get(),
                armed,
                arm_refusal,
            )
        });
    let wall_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

    let outcome = match run.outcome {
        Ok(_) => Ok(ScriptFacts { bindings }),
        // The ceiling refuses typed, naming itself — never a generic fault and
        // never a truncated read set.
        Err(_) if over_read_budget => Err(EvalError::ReadBudget {
            rule_id: rule.id.clone(),
            limit: limits.max_reads,
        }),
        // The run ceiling, same posture: typed, naming itself; the runs
        // already executed stand (a live program has no rollback).
        Err(_) if over_run_budget => Err(EvalError::RunBudget {
            rule_id: rule.id.clone(),
            limit: limits.max_runs,
        }),
        // The arm-time law refuses consumer-plane typed — never a §8 code, and
        // never a partial apply: the armed list below is evidence for the face,
        // and a refused attempt commits nothing.
        Err(e) => match arm_refusal {
            Some(ArmRefusal::ArmedBudget { line, limit }) => Err(EvalError::ArmedBudget {
                rule_id: rule.id.clone(),
                line,
                limit,
            }),
            None => Err(e),
        },
    };
    ScriptEval {
        telemetry: ScriptTelemetry {
            fuel_used: run.fuel_used,
            mem_used: run.mem_used,
            reads_used: recording.reads.len(),
            wall_ms,
        },
        armed,
        recording,
        outcome,
    }
}

/// Run one rule's `on_change(event)` metered: typed result + exact fuel/mem.
///
/// Dual enforcement (mirrors `policy`): coarse `set_max_*` guards abort
/// runaways; exact post-eval ticks/peak-heap is authoritative. Non-looping
/// oversize alloc still fails exact mem; engine `len overflow` panics map via
/// [`std::panic::catch_unwind`] to [`EvalError::Budget`]. Bombs terminate —
/// never hang, never crash.
pub(crate) fn run_rule_metered(
    globals: &Globals,
    rule: &Rule,
    event: &ChangeEvent,
    limits: EvalLimits,
) -> RuleRun {
    metered_eval(globals, rule, &EvalEntry::Change(event), limits)
}

/// The one metered evaluation both planes share (see [`run_rule_metered`] for
/// the metering/panic contract; [`EvalEntry`] carries the per-plane surface).
fn metered_eval(
    globals: &Globals,
    rule: &Rule,
    entry: &EvalEntry<'_>,
    limits: EvalLimits,
) -> RuleRun {
    if let Err(e) = check_source_size(rule, limits) {
        return RuleRun::failed(e);
    }
    if let Err(e) = check_nesting_depth(rule) {
        return RuleRun::failed(e);
    }

    let store = entry.store(rule);
    let step_guard = limits.fuel.max(1);
    let mem_guard = usize::try_from(limits.mem).unwrap_or(usize::MAX).max(1);

    // AssertUnwindSafe: panic path discards `store` unread → budget fault.
    let evaluated = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        Module::with_temp_heap(|module| {
            let heap = module.heap();
            let arg_value = entry.alloc(heap);

            let mut eval = Evaluator::new(&module);
            if let Err(e) = arm_guards(&mut eval, rule, step_guard, mem_guard, limits) {
                return e;
            }
            eval.extra = Some(&store);
            entry.bind(&module);

            let ast = match AstModule::parse(&rule.id, rule.source.clone(), &entry.dialect()) {
                Ok(ast) => ast,
                Err(e) => {
                    return RuleRun::failed(EvalError::Parse {
                        rule_id: rule.id.clone(),
                        reason: e.to_string(),
                    });
                }
            };

            // The script plane's echo positions are read off this AST — the one
            // parse, no second pass over the source. The banned suppression
            // spelling is refused off the same AST, BEFORE eval: a permanently
            // rejected spelling must not read, bind, or render.
            if let EvalEntry::Script(script) = entry {
                if let Some(line) = banned_suppression(&ast) {
                    return RuleRun::failed(EvalError::Parse {
                        rule_id: rule.id.clone(),
                        reason: format!(
                            "suppression syntax does not exist: `_ = read(…)` at line {line} is \
                             rejected permanently. A read is echoed or quiet by POSITION — bind \
                             it to a name to echo it, or call it inside a comprehension, an `if` \
                             condition, a loop body, or a function body to stay quiet"
                        ),
                    });
                }
                script.note_echo_positions(&ast);
            }

            let run = dispatch_entry(&mut eval, &module, entry, ast, globals, arg_value);

            let used_steps = eval.get_total_tick_count();
            let used_mem = heap_bytes(module.heap());
            drop(eval);
            let effects = match &store {
                PlaneStore::Emit(store) => store.effects.take(),
                // The script plane arms no `md.*` descriptors (plan decision 7).
                PlaneStore::Script(_) => Vec::new(),
            };

            let over_budget = used_steps > limits.fuel || used_mem > limits.mem;
            // A native-frame overflow is a budget verdict whichever side of
            // the classification asks, so it is read out before the ending
            // consumes it.
            let overflowed = run.depth_overflow;
            let fault_class = run.fault_class;
            let ending = run.ending();
            let outcome = classify_outcome(
                rule,
                entry,
                effects,
                &Ended {
                    ending,
                    over_budget,
                },
                limits,
            );
            RuleRun {
                fuel_used: used_steps,
                mem_used: used_mem,
                outcome,
                // Budget is the kernel's OWN accounting, not starlark's word.
                fault_class: if over_budget || overflowed {
                    Some(FaultClass::Budget)
                } else {
                    fault_class
                },
            }
        })
    }));

    match evaluated {
        Ok(run) => run,
        // Only reachable panic: resource-overflow assert → budget at ceiling.
        Err(_panic) => RuleRun {
            fuel_used: limits.fuel,
            mem_used: limits.mem,
            outcome: Err(budget(limits)),
            fault_class: Some(FaultClass::Budget),
        },
    }
}

/// Arm the evaluator's three ceilings: ticks, heap, and call depth.
///
/// Call depth is bounded because unbounded recursion overflows NATIVE frames
/// before the tick guard can fire; the resulting `StackOverflow` is then
/// classed as budget, not as a rule fault.
fn arm_guards(
    eval: &mut Evaluator<'_, '_, '_>,
    rule: &Rule,
    step_guard: u64,
    mem_guard: usize,
    limits: EvalLimits,
) -> Result<(), RuleRun> {
    eval.set_max_tick_count(step_guard)
        .map_err(|e| RuleRun::failed(runtime(rule, e)))?;
    eval.set_max_heap_size(mem_guard)
        .map_err(|e| RuleRun::failed(runtime(rule, e)))?;
    eval.set_max_callstack_size(limits.max_call_depth.max(1))
        .map_err(|e| RuleRun::failed(runtime(rule, e)))?;
    Ok(())
}

/// The three ways one entry's evaluation can end, each carrying only the
/// facts its own case has. An enum rather than a bag of booleans: the states
/// are mutually exclusive, so `missing && aborted` was never representable in
/// the domain and should not be representable in the type.
enum Ending {
    /// A hooked plane found no hook of its own; `wrong_plane` names the OTHER
    /// plane's hook when that is what the source defines instead.
    Missing { wrong_plane: Option<&'static str> },
    /// The entry aborted. `depth_overflow` separates a native-frame overflow
    /// (which is a budget verdict) from a genuine fault.
    Faulted {
        depth_overflow: bool,
        fault: Option<String>,
        fault_line: Option<u32>,
    },
    /// The entry ran to completion.
    Completed,
}

/// How one entry's evaluation ended, before it is turned into an outcome:
/// the [`Ending`], plus the metering verdict — measured from the evaluator
/// AFTER the entry returned, so it is orthogonal to how the entry ended and
/// stays a field of its own.
struct Ended {
    ending: Ending,
    over_budget: bool,
}

/// The typed outcome of one metered evaluation. Extracted from
/// [`metered_eval`] so the eval body stays readable; the precedence is the
/// shipped one, unchanged: a missing entry first, then abort (budget before
/// genuine fault), then an over-budget completion.
fn classify_outcome(
    rule: &Rule,
    entry: &EvalEntry<'_>,
    effects: Vec<Effect>,
    end: &Ended,
    limits: EvalLimits,
) -> Result<Vec<Effect>, EvalError> {
    match &end.ending {
        Ending::Missing { wrong_plane } => Err(EvalError::MissingEntry {
            rule_id: rule.id.clone(),
            expected: entry.hook().unwrap_or_default(),
            wrong_plane: *wrong_plane,
        }),
        // over_budget / StackOverflow → budget; else genuine fault.
        Ending::Faulted {
            depth_overflow,
            fault,
            fault_line,
        } => {
            if end.over_budget || *depth_overflow {
                Err(budget(limits))
            } else {
                Err(EvalError::Runtime {
                    rule_id: rule.id.clone(),
                    reason: fault.clone().unwrap_or_default(),
                    line: *fault_line,
                })
            }
        }
        // Completed without abort but exact mem still over — budget.
        Ending::Completed if end.over_budget => Err(budget(limits)),
        Ending::Completed => Ok(effects),
    }
}

/// How one plane's entry point finished: aborted with a fault, or (hooked
/// planes only) found no hook — with the other plane's hook named when THAT is
/// what the source defines.
struct EntryRun {
    aborted: bool,
    /// The starlark class of the abort, taken by DOWNCAST off the live
    /// `starlark::Error` before it degrades to a string (A1).
    fault_class: Option<FaultClass>,
    depth_overflow: bool,
    fault: Option<String>,
    /// 1-based source line of the fault, when the Starlark error carried a
    /// span. Lines are 1-based for a reader; the resolver is 0-based.
    fault_line: Option<u32>,
    /// The hooked plane found no hook of its own.
    missing: bool,
    /// …and the other plane's hook is what the source defines instead.
    wrong_plane: Option<&'static str>,
}

impl EntryRun {
    /// This run as one [`Ending`], in the shipped precedence: a missing hook
    /// first, then an abort, then a clean completion. The precedence lives
    /// HERE, once, rather than at each caller — it is the part a reader has
    /// to get right and the part a second copy would get wrong.
    fn ending(self) -> Ending {
        if self.missing {
            Ending::Missing {
                wrong_plane: self.wrong_plane,
            }
        } else if self.aborted {
            Ending::Faulted {
                depth_overflow: self.depth_overflow,
                fault: self.fault,
                fault_line: self.fault_line,
            }
        } else {
            Ending::Completed
        }
    }
}

/// Evaluate the module, then enter the plane: the hooked planes look their hook
/// up and call it with the injected facts; the script plane has no hook — its
/// module top level was already the whole program, so entering it is exactly
/// the subtraction of that lookup.
fn dispatch_entry<'v>(
    eval: &mut Evaluator<'v, '_, '_>,
    module: &Module<'v>,
    entry: &EvalEntry<'_>,
    ast: AstModule,
    globals: &Globals,
    arg_value: Value<'v>,
) -> EntryRun {
    let aborted = |e: &starlark::Error| EntryRun {
        aborted: true,
        fault_class: Some(classify_starlark_fault(e)),
        depth_overflow: is_depth_overflow(e),
        fault: Some(e.to_string()),
        // 1-based for a reader; the resolver is 0-based (the read-record
        // convention).
        fault_line: e
            .span()
            .map(|span| u32::try_from(span.resolve_span().begin.line + 1).unwrap_or(u32::MAX)),
        missing: false,
        wrong_plane: None,
    };
    let finished = EntryRun {
        aborted: false,
        fault_class: None,
        depth_overflow: false,
        fault: None,
        fault_line: None,
        missing: false,
        wrong_plane: None,
    };

    if let Err(e) = eval.eval_module(ast, globals) {
        return aborted(&e);
    }
    let Some(hook) = entry.hook() else {
        if let EvalEntry::Script(script) = entry {
            script.capture_bindings(module);
        }
        return finished;
    };
    let Some(hook) = module.get(hook) else {
        // Missing entry; note wrong-plane if the other hook is what exists.
        return EntryRun {
            missing: true,
            wrong_plane: entry.other().filter(|name| module.get(name).is_some()),
            ..finished
        };
    };
    match eval.eval_function(hook, &[arg_value], &[]) {
        Ok(_) => finished,
        Err(e) => aborted(&e),
    }
}

/// Whether a Starlark eval error is a call-stack-depth overflow (the recursion
/// guard tripping) — classified as budget, not a rule fault.
fn is_depth_overflow(e: &starlark::Error) -> bool {
    matches!(e.kind(), starlark::ErrorKind::StackOverflow(_))
}

/// The budget-exhaustion error for these limits.
fn budget(limits: EvalLimits) -> EvalError {
    EvalError::Budget {
        fuel: limits.fuel,
        mem: limits.mem,
    }
}

/// A runtime fault carrying the rule's provenance.
fn runtime(rule: &Rule, e: impl std::fmt::Display) -> EvalError {
    EvalError::Runtime {
        rule_id: rule.id.clone(),
        line: None,
        reason: e.to_string(),
    }
}

/// Allocate injected `event` (0003 §3 payload). Lists → Starlark lists of strings.
fn alloc_event<'v>(heap: Heap<'v>, event: &ChangeEvent) -> Value<'v> {
    let sections: Vec<Value<'v>> = event
        .sections_changed
        .iter()
        .map(|s| heap.alloc(s.as_str()))
        .collect();
    let fields: Vec<Value<'v>> = event
        .fields_changed
        .iter()
        .map(|s| heap.alloc(s.as_str()))
        .collect();
    let changes: Vec<Value<'v>> = event
        .changes
        .iter()
        .map(|c| alloc_change_fact(heap, c))
        .collect();
    heap.alloc(AllocStruct([
        ("file", heap.alloc(event.file.as_str())),
        ("sections_changed", heap.alloc(sections)),
        ("fields_changed", heap.alloc(fields)),
        ("changes", heap.alloc(changes)),
        ("facts", alloc_event_facts(heap, &event.facts)),
        (
            "fingerprint_before",
            heap.alloc(event.fingerprint_before.as_str()),
        ),
        (
            "fingerprint_after",
            heap.alloc(event.fingerprint_after.as_str()),
        ),
        ("depth", heap.alloc(event.depth)),
    ]))
}

/// Allocate one `{kind, key, old, new, hpath}` fact. Absence is `None`, never `""`.
fn alloc_change_fact<'v>(heap: Heap<'v>, fact: &ChangeFact) -> Value<'v> {
    let hpath: Vec<Value<'v>> = fact.hpath.iter().map(|s| heap.alloc(s.as_str())).collect();
    heap.alloc(AllocStruct([
        ("kind", heap.alloc(fact.kind.as_str())),
        ("key", heap.alloc(fact.key.as_str())),
        ("old", opt_str(heap, fact.old.as_deref())),
        ("new", opt_str(heap, fact.new.as_deref())),
        ("hpath", heap.alloc(hpath)),
    ]))
}

/// Allocate `event.facts` as `{path, fm}` only. No actor/session/invocation —
/// actor-fact WHEN is a `NameError`, not a soft policy.
fn alloc_event_facts<'v>(heap: Heap<'v>, facts: &EventFacts) -> Value<'v> {
    let fm = heap.alloc(AllocDict(
        facts
            .frontmatter
            .iter()
            .map(|(k, v)| (heap.alloc(k.as_str()), heap.alloc(v.as_str()))),
    ));
    heap.alloc(AllocStruct([
        ("path", heap.alloc(facts.path.as_str())),
        ("fm", fm),
    ]))
}

/// Allocate run-plane `ctx`: `page`/`task`/`args`/`env` only. Provenance facts
/// deliberately not injected (see [`crate::RunCtx`]).
fn alloc_ctx<'v>(heap: Heap<'v>, ctx: &RunCtx) -> Value<'v> {
    let args: Vec<Value<'v>> = ctx.args.iter().map(|s| heap.alloc(s.as_str())).collect();
    let env = heap.alloc(AllocDict(
        ctx.env
            .iter()
            .map(|(k, v)| (heap.alloc(k.as_str()), heap.alloc(v.as_str()))),
    ));
    heap.alloc(AllocStruct([
        ("page", heap.alloc(ctx.page.as_str())),
        ("task", heap.alloc(ctx.task.as_str())),
        ("args", heap.alloc(args)),
        ("env", env),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// Non-standard names of a globals set — what the plane adds to the stdlib.
    fn plane_surface(globals: &Globals) -> HashSet<String> {
        let standard: HashSet<String> = GlobalsBuilder::standard()
            .build()
            .names()
            .map(|n| n.as_str().to_owned())
            .collect();
        globals
            .names()
            .map(|n| n.as_str().to_owned())
            .filter(|n| !standard.contains(n))
            .collect()
    }

    /// Load purity, structurally — the run entry's load phase can record NO
    /// effect, for every effect that exists and every effect that will.
    ///
    /// § 2.2 stated this law as a set difference over the load globals
    /// (`surface ∩ {bash, md constructors} = ∅`). That mechanism is not
    /// available: starlark-rust resolves global names at MODULE-COMPILE time
    /// (`starlark-0.14.2/src/eval.rs:82-89`), so withholding the builtins
    /// would make every effectful block UNLOADABLE — a `def run(event)` body
    /// merely mentioning `create()` is enough to fail the compile — and the
    /// declarations a load exists to publish would be unreachable for exactly
    /// the blocks that matter. Measured; the probe is kept with card
    /// `hook-01-mrd-run-load-fire`.
    ///
    /// So the law is enforced at the ONE funnel every effect must pass to
    /// exist — [`EmitStore::push`] — and asserted here over the closed
    /// [`EffectKind::ALL`] table rather than a hand-written list. That is
    /// what makes it structural: a new effect kind cannot be added without
    /// appearing in `ALL`, and cannot record without `push`, so it can
    /// escape neither the gate nor this assertion.
    /// A probe store for the phase tests.
    fn probe_store(phase: EffectPhase) -> EmitStore<'static> {
        EmitStore::new(
            "probe",
            Provenance::Run {
                invocation_id: "i".to_owned(),
                root_at_eval: "r".to_owned(),
            },
            0,
        )
        .in_phase(phase)
    }

    /// Parse + evaluate one hook-plane module against a store, freeze it, and
    /// hand back the frozen module — the load half, in miniature.
    fn load_module(source: &str, store: &PlaneStore<'_>) -> Result<FrozenModule, String> {
        Module::with_temp_heap(|module| {
            let ast = AstModule::parse("probe", source.to_owned(), &script_dialect())
                .map_err(|e| format!("parse: {e}"))?;
            let mut eval = Evaluator::new(&module);
            eval.extra = Some(store);
            let outcome = eval.eval_module(ast, &hook_globals());
            drop(eval);
            outcome.map_err(|e| format!("{e}"))?;
            module.freeze().map_err(|e| format!("freeze: {e:?}"))
        })
    }

    /// **THE load-bearing assumption of the whole phase-gate shape**, and it
    /// was asserted only from API types until this test existed (deciding
    /// seat `dd1bb68f`, 2026-08-23: "FIRST TEST you write, before anything
    /// else").
    ///
    /// The shape is: compile and FREEZE a block's module once under a `Load`
    /// store, then later call its frozen entry from a FRESH evaluator whose
    /// `Evaluator::extra` carries a `Fire` store. That only works if a frozen
    /// `def` reads the store DYNAMICALLY at call time — off the evaluator
    /// running it — rather than capturing the one it was compiled under. If
    /// starlark inlined the store the way it inlines a global's value, the
    /// fire would silently emit into the dead load store and every effect a
    /// hook applied would vanish. Nothing about the API types says which it
    /// is; only running it does.
    #[test]
    fn a_frozen_def_sees_the_fire_store_not_the_load_store() {
        let source = "\
def run(event):
    set_field(field = \"status\", value = event)
    return 1
";
        // Load phase: the module compiles and freezes. The def MENTIONS an
        // effect builtin, which is exactly the case A1 exists for — under the
        // superseded design this would not even have loaded.
        let load = PlaneStore::Emit(probe_store(EffectPhase::Load));
        let frozen = load_module(source, &load).expect("the block loads under a Load store");
        let PlaneStore::Emit(load_store) = &load else {
            unreachable!("probe store is an emit store")
        };
        assert!(
            load_store.effects.borrow().is_empty(),
            "the LOAD phase recorded an effect: {:?}",
            load_store.effects.borrow()
        );

        // Fire phase: a fresh evaluator, a fresh store, the SAME frozen def.
        let fire = PlaneStore::Emit(probe_store(EffectPhase::Fire));
        let entry = frozen.get("run").expect("`run` is defined");
        let returned = Module::with_temp_heap(|module| {
            // `Heap::access_owned_frozen_value` is the seam that lets a
            // frozen value cross into a fresh evaluator's lifetime.
            // (`import_public_symbols` does NOT work here: it imports under
            // `Visibility::Private`, and `Module::get` returns `None` for a
            // private name — measured, it cost a red test.)
            module.frozen_heap().add_reference(entry.owner());
            let entry = module.heap().access_owned_frozen_value(&entry);
            let arg = module.heap().alloc("Done");
            let mut eval = Evaluator::new(&module);
            eval.extra = Some(&fire);
            let called = eval.eval_function(entry, &[arg], &[]);
            let rendered = called.map(|v| v.to_string()).map_err(|e| e.to_string());
            drop(eval);
            rendered
        })
        .expect("the frozen entry calls under a Fire store");
        assert_eq!(returned, "1", "the entry's return value crosses back");

        let PlaneStore::Emit(fire_store) = &fire else {
            unreachable!("probe store is an emit store")
        };
        let recorded = fire_store.effects.borrow();
        assert_eq!(
            recorded.len(),
            1,
            "the frozen def emitted into the FIRE store — got {recorded:?}"
        );
        assert_eq!(recorded[0].kind, EffectKind::SetField);
        assert_eq!(
            recorded[0].args.get("value"),
            Some(&ArgValue::Str("Done".to_owned())),
            "the fire's argument reached the effect, so the call really ran"
        );
        // …and the load store stayed empty throughout, so nothing leaked back.
        assert!(
            load_store.effects.borrow().is_empty(),
            "the frozen def emitted into the store it was COMPILED under — the \
             phase-gate shape does not hold"
        );
    }

    /// Load purity, structurally — the load phase can perform NO effect, for
    /// every effect that exists and every effect that will.
    ///
    /// § 2.2 stated this law as a set difference over the load globals
    /// (`surface ∩ {bash, md constructors} = ∅`). That mechanism is not
    /// available: starlark-rust resolves identifiers at MODULE-COMPILE time
    /// and inlines a bound global's value into the def, so withholding the
    /// builtins would make every effectful block UNLOADABLE — a `def
    /// run(event)` body merely mentioning `create()` is enough to fail the
    /// compile — and the declarations a load exists to publish would be
    /// unreachable for exactly the blocks that matter. Measured; design
    /// § Amendments / A1.
    ///
    /// So the law is enforced at the ONE accessor every effect builtin calls
    /// to reach its channel ([`store`]), and asserted here over the closed
    /// [`EffectKind::ALL`] table rather than a hand-written list. That is
    /// what makes it structural: a new effect kind cannot be added without
    /// appearing in `ALL`, and cannot act without the accessor, so it can
    /// escape neither the gate nor this assertion.
    #[test]
    fn no_effect_builtin_acts_during_the_load_phase() {
        for (kind, call) in WELL_FORMED_CALLS {
            let builtin = kind.constructor();
            assert!(
                call.starts_with(builtin),
                "the call table row for `{builtin}` does not call it: {call}"
            );
            let load = PlaneStore::Emit(probe_store(EffectPhase::Load));
            // The call is WELL-FORMED on purpose. starlark validates named
            // parameters before it enters a builtin's body (measured: an
            // arg-less `set_field()` refuses `Missing named-only parameter
            // `field``, an `ErrorKind::Function` fault that never reaches the
            // gate). A malformed call would therefore "pass" this test
            // while proving nothing about the phase.
            let Err(message) = load_module(&format!("{call}\n"), &load) else {
                panic!("`{builtin}` did not refuse at load")
            };
            assert!(
                message.contains("does not act at load"),
                "`{builtin}` refused at load for the WRONG reason: {message}"
            );
            let PlaneStore::Emit(store) = &load else {
                unreachable!()
            };
            assert!(
                store.effects.borrow().is_empty(),
                "`{builtin}` recorded an effect during the load phase"
            );
        }
        println!(
            "POPULATION load-gated effect builtins = {:?}",
            EffectKind::ALL.map(EffectKind::constructor)
        );
    }

    /// One well-formed call per effect kind — the table the load-purity test
    /// drives. Kept beside `EffectKind::ALL` and checked against it, so a new
    /// effect kind fails this module rather than silently going untested.
    const WELL_FORMED_CALLS: [(EffectKind, &str); 8] = [
        (
            EffectKind::SetField,
            r#"set_field(field = "s", value = "v")"#,
        ),
        (
            EffectKind::AppendSection,
            r#"append_section(section = "S", content = "c")"#,
        ),
        (EffectKind::Create, r#"create(path = "p.md", body = "b")"#),
        (EffectKind::RefreshView, r#"refresh_view(view = "v")"#),
        (EffectKind::Send, r#"send(to = ["a"], message = "m")"#),
        (EffectKind::Remind, r#"remind(message = "m")"#),
        (EffectKind::Ask, r#"ask(message = "m")"#),
        (EffectKind::Notice, r#"notice(message = "m")"#),
    ];

    /// The call table covers the closed kind table, in order — the guard that
    /// keeps the load-purity test honest as effects are added.
    #[test]
    fn the_well_formed_call_table_covers_every_effect_kind() {
        let covered: Vec<EffectKind> = WELL_FORMED_CALLS.iter().map(|(k, _)| *k).collect();
        assert_eq!(
            covered,
            EffectKind::ALL.to_vec(),
            "WELL_FORMED_CALLS must mirror EffectKind::ALL — a new effect kind \
             needs a row here or load purity goes untested for it"
        );
    }

    /// The typed class, not the prose. A1: *"`effect_at_load` is
    /// `ErrorKind::Native` carrying `EffectAtLoad`; never classified by
    /// string."* If a class ever has to be matched off the message, it has
    /// stopped being typed, and the next prose edit reclassifies the fault
    /// without anyone noticing.
    #[test]
    fn phase_faults_classify_by_downcast_and_never_absorb_name_error() {
        let classify = |source: &str, phase: EffectPhase| -> (FaultClass, Option<u32>) {
            let store = PlaneStore::Emit(probe_store(phase));
            Module::with_temp_heap(|module| {
                let ast = AstModule::parse("probe", source.to_owned(), &rule_dialect())
                    .expect("the probe parses");
                let mut eval = Evaluator::new(&module);
                eval.extra = Some(&store);
                let outcome = eval.eval_module(ast, &effect_globals());
                let classified = outcome
                    .err()
                    .map(|e| (classify_starlark_fault(&e), starlark_fault_line(&e)));
                drop(eval);
                classified
            })
            .expect("the probe refuses")
        };

        // A well-formed effect call at load: the phase class, by downcast.
        let (class, line) = classify(
            "\ncreate(path = \"p.md\", body = \"b\")\n",
            EffectPhase::Load,
        );
        assert_eq!(class, FaultClass::EffectAtLoad);
        assert_eq!(
            class.as_str(),
            "effect_at_load",
            "the wire spelling is the one the design names"
        );
        assert_eq!(
            line,
            Some(2),
            "the fault carries the CALL-SITE line, 1-based — row 8 publishes it"
        );

        // A misspelled constructor is still `name_error`, at either phase.
        // Without this, the new class would quietly swallow the old one and
        // an author with a typo would be told to move their code.
        for phase in [EffectPhase::Load, EffectPhase::Fire] {
            let (class, _) = classify("dney()\n", phase);
            assert_eq!(
                class,
                FaultClass::NameError,
                "a name bound nowhere stays `name_error` at {phase:?} — it is \
                 ErrorKind::Scope, a different situation entirely"
            );
        }

        // And the same effect call at FIRE does not fault at all.
        let store = PlaneStore::Emit(probe_store(EffectPhase::Fire));
        let ok = Module::with_temp_heap(|module| {
            let ast = AstModule::parse(
                "probe",
                "create(path = \"p.md\", body = \"b\")\n".to_owned(),
                &rule_dialect(),
            )
            .expect("parses");
            let mut eval = Evaluator::new(&module);
            eval.extra = Some(&store);
            let outcome = eval.eval_module(ast, &effect_globals());
            let is_ok = outcome.is_ok();
            drop(eval);
            is_ok
        });
        assert!(ok, "the gate refuses a PHASE, not the call");
    }

    /// The same table, FIRE tense: the gate refuses a PHASE, never an effect.
    /// Without this, a gate that refused everything would pass the test above.
    #[test]
    fn every_effect_kind_records_during_the_fire_phase() {
        let fire = probe_store(EffectPhase::Fire);
        for kind in EffectKind::ALL {
            fire.push(kind, BTreeMap::new());
        }
        assert_eq!(
            fire.effects.borrow().len(),
            EffectKind::ALL.len(),
            "the fire phase records every kind, once each"
        );
    }

    /// Whether a `FrozenModule` may be held in a resident, shared cache — the
    /// § 2.2 per-block-rev module cache lives in the daemon's registry and is
    /// reached from every connection thread, so `Send + Sync` is a
    /// PRECONDITION of that design, not a detail. Asserted at compile time
    /// because a runtime discovery would arrive as an unexplainable borrow
    /// error three layers away.
    #[test]
    fn a_frozen_module_can_live_in_a_shared_cache() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<FrozenModule>();
    }

    /// A probe `RunCtx` — the load/fire paths read only its `task` (as the
    /// rule id) and its provenance strings.
    fn probe_ctx() -> RunCtx {
        RunCtx {
            task: "probe".to_owned(),
            invocation_id: "i".to_owned(),
            root_at_eval: "r".to_owned(),
            ..RunCtx::default()
        }
    }

    /// A `FireHost` that records what `bash()` was asked for and answers a
    /// fixed row — enough to prove the seam is reached and its answer crosses
    /// back, without a process anywhere near a unit test.
    struct ProbeHost {
        calls: std::sync::Mutex<Vec<BashCall>>,
    }

    impl FireHost for ProbeHost {
        fn bash(&self, call: &BashCall) -> Result<serde_json::Value, String> {
            self.calls
                .lock()
                .expect("probe host lock")
                .push(call.clone());
            Ok(serde_json::json!({"exit": 0, "stdout": "hi", "stderr": "", "timed_out": false}))
        }
    }

    /// `declare()` publishes every key VERBATIM and reads exactly one of them.
    ///
    /// The design's law (§ 2.4) is that `declarations` is uninterpreted data —
    /// so this asserts on keys the engine has no opinion about at all, which
    /// is the only way to catch an engine that quietly starts having one.
    #[test]
    fn declare_publishes_its_keys_verbatim_and_reads_only_impl() {
        let source = "\
def run(event):
    return None

declare(on = \"Stop\", match = \"Bash\", weight = 3, nested = {\"a\": [1, True, None]})
";
        let loaded = load_block(source, None, &probe_ctx(), EvalLimits::default());
        assert!(loaded.fault.is_none(), "load faulted: {:?}", loaded.fault);
        assert_eq!(loaded.declarations.len(), 1);
        let declaration = &loaded.declarations[0];
        assert_eq!(
            declaration.data,
            serde_json::json!({
                "on": "Stop",
                "match": "Bash",
                "weight": 3,
                "nested": {"a": [1, true, null]},
            }),
            "every key the author passed rides verbatim, and `impl` is not among them"
        );
        assert_eq!(
            declaration.entry,
            DeclaredEntry::Evaluated {
                name: "run".to_owned()
            },
            "no `impl` means the conventional entry"
        );
        assert_eq!(declaration.entry.kind(), "evaluated");
    }

    /// An explicit `declare(impl = f)` names the def the fire will call.
    ///
    /// This pins an INTERNAL of starlark's rendering: a def's `Display` is
    /// `ParametersSpec::signature()`, which writes the function name and
    /// nothing else. The whole explicit-`impl` path resolves the entry that
    /// way, so a silent change upstream would send every such fire to the
    /// wrong function — or to none — and nothing else would notice.
    #[test]
    fn a_declared_impl_names_the_def_the_fire_calls() {
        let source = "\
def check_stop(event):
    return {\"deny\": \"no\"}

declare(on = \"Stop\", impl = check_stop)
";
        let loaded = load_block(source, None, &probe_ctx(), EvalLimits::default());
        assert!(loaded.fault.is_none(), "load faulted: {:?}", loaded.fault);
        assert_eq!(
            loaded.declarations[0].entry,
            DeclaredEntry::Evaluated {
                name: "check_stop".to_owned()
            }
        );
        // …and the name really resolves in the frozen module: naming it is
        // worth nothing if the fire cannot find it.
        let module = loaded.module.expect("a clean load freezes");
        let host = ProbeHost {
            calls: std::sync::Mutex::new(Vec::new()),
        };
        let fired = fire_entry(
            &module,
            "check_stop",
            &serde_json::json!({"name": "Stop"}),
            &probe_ctx(),
            &host,
            EvalLimits::default(),
        )
        .expect("the named entry exists");
        assert_eq!(fired.value, Some(serde_json::json!({"deny": "no"})));
    }

    /// `declare(impl = …)` takes a callable or an `exec()` value, and says so
    /// by TYPE for anything else — the `impl_type` fault.
    #[test]
    fn an_impl_that_is_neither_callable_nor_exec_refuses_by_type() {
        let loaded = load_block(
            "declare(on = \"Stop\", impl = 7)\n",
            None,
            &probe_ctx(),
            EvalLimits::default(),
        );
        let fault = loaded.fault.expect("an int is not an entry");
        // The CLASS, not the prose. Asserting the reason string alone is what
        // let `impl_type` sit in two published fault unions while the engine
        // could only ever emit `runtime` — the exact shape A1's
        // "typed, never classified by string" argument warns against.
        assert_eq!(
            fault.class,
            FaultClass::ImplType,
            "the fault a caller matches on must be `impl_type`: {fault:?}"
        );
        assert_eq!(fault.class.as_str(), "impl_type");
        assert!(
            fault.reason.contains("`impl` is int"),
            "the refusal must name what the author actually passed: {}",
            fault.reason
        );
    }

    /// **S3 — a prelude may not DECLARE** (PR 195 review, `fa5da9ec`).
    ///
    /// The one-liner the reviewer used: a caller prelude declaring a process
    /// entry. Before the guard it produced a declaration that outranked the
    /// page's, so every anchored fence on every addressed page became a fire
    /// target running caller-authored bytes.
    #[test]
    fn a_prelude_that_declares_is_refused_before_any_block() {
        let prelude = "declare(impl = exec(\"bash\", cmd = \"id\"))\n";
        // It still EVALUATES — the builtins are load-phase legal — which is
        // why reading `.fault` alone could never catch it.
        let loaded = load_block(prelude, None, &probe_ctx(), EvalLimits::default());
        assert!(loaded.fault.is_none(), "it evaluates cleanly: {loaded:?}");
        assert_eq!(loaded.declarations.len(), 1, "and it DECLARES");

        // The guard refuses it, and the row renders `prelude_invalid`.
        let fault = check_prelude(prelude, &probe_ctx(), EvalLimits::default())
            .expect("a declaring prelude must refuse");
        assert_eq!(
            fault.class,
            FaultClass::PreludeInvalid,
            "the class is the remedy a caller branches on: {fault:?}"
        );
        assert_eq!(fault.class.as_str(), "prelude_invalid");
        assert!(
            fault.reason.contains("consent material"),
            "the refusal must name what it found: {}",
            fault.reason
        );

        // A prelude of pure helpers is untouched.
        assert!(
            check_prelude("def helper(x):\n    return x\n", &probe_ctx(), EvalLimits::default())
                .is_none(),
            "a pure prelude is what a prelude is FOR"
        );
    }

    /// `exec()` declares a process entry: legal at load, carried as data.
    #[test]
    fn exec_declares_a_process_entry_as_data() {
        let source = "\
declare(on = \"Stop\", impl = exec(\"bash\", block = \"check\", args = [\"--quiet\"]))
";
        let loaded = load_block(source, None, &probe_ctx(), EvalLimits::default());
        assert!(loaded.fault.is_none(), "load faulted: {:?}", loaded.fault);
        assert_eq!(
            loaded.declarations[0].entry,
            DeclaredEntry::Exec(ExecSpec {
                interpreter: "bash".to_owned(),
                program: ExecProgram::Block("check".to_owned()),
                args: vec!["--quiet".to_owned()],
                env: BTreeMap::new(),
            })
        );
        assert_eq!(loaded.declarations[0].entry.kind(), "exec");
    }

    /// Exactly one of `cmd` / `block`, on both builtins that take the pair.
    #[test]
    fn exec_takes_exactly_one_program() {
        for (source, why) in [
            (
                "declare(impl = exec(\"bash\", cmd = \"true\", block = \"c\"))\n",
                "exclusive",
            ),
            ("declare(impl = exec(\"bash\"))\n", "neither"),
        ] {
            let loaded = load_block(source, None, &probe_ctx(), EvalLimits::default());
            let fault = loaded
                .fault
                .unwrap_or_else(|| panic!("{why}: the call should have refused"));
            assert!(
                fault.reason.contains("exec:"),
                "{why}: the refusal must name the builtin: {}",
                fault.reason
            );
        }
    }

    /// `declare()` and `exec()` are the LOAD phase's — A1's mirror gate.
    #[test]
    fn declare_and_exec_refuse_during_the_fire_phase() {
        for (builtin, call) in [
            ("declare", "declare(on = \"Stop\")"),
            ("exec", "exec(\"bash\", cmd = \"true\")"),
        ] {
            let source = format!("def run(event):\n    {call}\n    return None\n");
            let loaded = load_block(&source, None, &probe_ctx(), EvalLimits::default());
            assert!(
                loaded.fault.is_none(),
                "`{builtin}` inside a def must LOAD fine — the refusal is about the \
                 phase, not the name: {:?}",
                loaded.fault
            );
            let module = loaded.module.expect("a clean load freezes");
            let host = ProbeHost {
                calls: std::sync::Mutex::new(Vec::new()),
            };
            let fired = fire_entry(
                &module,
                "run",
                &serde_json::Value::Null,
                &probe_ctx(),
                &host,
                EvalLimits::default(),
            )
            .expect("`run` exists");
            let fault = fired
                .fault
                .unwrap_or_else(|| panic!("`{builtin}` did not refuse at fire"));
            assert_eq!(
                fault.class,
                FaultClass::DeclareAtFire,
                "`{builtin}` at fire is `declare_at_fire`, by downcast"
            );
            assert!(
                fault.reason.contains("only runs at load"),
                "`{builtin}` refused at fire for the wrong reason: {}",
                fault.reason
            );
        }
    }

    /// A top-level `bash()` is the `effect_at_load` fault at its own line —
    /// gate row 8's subject. `bash` is not an `EffectKind`, so the
    /// table-driven purity test above cannot reach it and this one must.
    #[test]
    fn a_top_level_bash_is_effect_at_load_with_its_line() {
        let source = "\
x = 1
bash(cmd = \"true\")
";
        let loaded = load_block(source, None, &probe_ctx(), EvalLimits::default());
        let fault = loaded.fault.expect("`bash` at load must refuse");
        assert_eq!(fault.class, FaultClass::EffectAtLoad);
        assert_eq!(fault.class.as_str(), "effect_at_load");
        assert_eq!(
            fault.line,
            Some(2),
            "the fault points at the CALL, not at the block: {fault:?}"
        );
        assert!(loaded.module.is_none(), "a faulted load freezes nothing");
    }

    /// A `bash()` inside the entry reaches the process seam at fire, and its
    /// answer crosses back into the program as a value.
    #[test]
    fn bash_reaches_the_process_seam_at_fire() {
        let source = "\
def run(event):
    out = bash(cmd = \"echo hi\", stdin = event)
    return {\"exit\": out[\"exit\"], \"said\": out[\"stdout\"]}
";
        let loaded = load_block(source, None, &probe_ctx(), EvalLimits::default());
        assert!(loaded.fault.is_none(), "load faulted: {:?}", loaded.fault);
        let module = loaded.module.expect("a clean load freezes");
        let host = ProbeHost {
            calls: std::sync::Mutex::new(Vec::new()),
        };
        let fired = fire_entry(
            &module,
            "run",
            &serde_json::json!({"name": "Stop"}),
            &probe_ctx(),
            &host,
            EvalLimits::default(),
        )
        .expect("`run` exists");
        assert!(fired.fault.is_none(), "fire faulted: {:?}", fired.fault);
        assert_eq!(
            fired.value,
            Some(serde_json::json!({"exit": 0, "said": "hi"})),
            "the seam's row reached the program and the program's answer came back"
        );
        let calls = host.calls.lock().expect("probe host lock");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].program, ExecProgram::Cmd("echo hi".to_owned()));
        assert_eq!(
            calls[0].stdin.as_deref(),
            Some(r#"{"name":"Stop"}"#),
            "a dict `stdin` is serialized as COMPACT JSON before it reaches the pipe"
        );
    }

    /// The prelude binds into the block's module — and the block keeps its
    /// OWN line numbers.
    ///
    /// The obvious implementation concatenates the prelude ahead of the source,
    /// which passes the first half of this test and fails the second: every
    /// `fault.line` would be offset by the prelude's length, and `fault.line`
    /// is the one number an author navigates by. Both halves are asserted here
    /// because only the pair distinguishes the two implementations.
    #[test]
    fn a_prelude_binds_into_the_block_and_leaves_its_line_numbers_alone() {
        let prelude = "\
def deny(reason):
    return {\"deny\": reason}

def allow(reason):
    return {\"allow\": reason}
";
        let source = "\
def run(event):
    return deny(\"no stash\")
";
        let loaded = load_block(source, Some(prelude), &probe_ctx(), EvalLimits::default());
        assert!(loaded.fault.is_none(), "load faulted: {:?}", loaded.fault);
        let module = loaded.module.expect("a clean load freezes");
        let host = ProbeHost {
            calls: std::sync::Mutex::new(Vec::new()),
        };
        let fired = fire_entry(
            &module,
            "run",
            &serde_json::Value::Null,
            &probe_ctx(),
            &host,
            EvalLimits::default(),
        )
        .expect("`run` exists");
        assert_eq!(
            fired.value,
            Some(serde_json::json!({"deny": "no stash"})),
            "a prelude binding is callable from the block and survives the freeze"
        );

        // The second half: a fault in the BLOCK reports the block's line.
        let faulty = "\
x = 1
bash(cmd = \"true\")
";
        let loaded = load_block(faulty, Some(prelude), &probe_ctx(), EvalLimits::default());
        let fault = loaded.fault.expect("`bash` at load must refuse");
        assert_eq!(
            fault.line,
            Some(2),
            "the prelude's 5 lines must not move the block's line 2: {fault:?}"
        );
    }

    /// A prelude that is not pure refuses on its own, before any block —
    /// § 2.2's `prelude_invalid`.
    #[test]
    fn a_prelude_that_acts_refuses_before_any_block() {
        let fault = check_prelude(
            "create(path = \"a.md\", body = \"x\")\n",
            &probe_ctx(),
            EvalLimits::default(),
        )
        .expect("an effectful prelude must refuse");
        assert_eq!(fault.class, FaultClass::EffectAtLoad);
        assert!(
            check_prelude(
                "def deny(r):\n    return r\n",
                &probe_ctx(),
                EvalLimits::default()
            )
            .is_none(),
            "a prelude of pure functions is sound"
        );
    }

    /// The fire's md effects land in the FIRE store and cross back as
    /// descriptors for the caller's doors — the kernel opens none itself.
    #[test]
    fn a_fire_emits_descriptors_for_the_caller_to_realize() {
        let source = "\
def run(event):
    create(path = event[\"path\"], body = \"born\")
    return None
";
        let loaded = load_block(source, None, &probe_ctx(), EvalLimits::default());
        assert!(loaded.fault.is_none(), "load faulted: {:?}", loaded.fault);
        assert!(
            loaded.effects.is_empty(),
            "the LOAD phase applied something: {:?}",
            loaded.effects
        );
        let module = loaded.module.expect("a clean load freezes");
        let host = ProbeHost {
            calls: std::sync::Mutex::new(Vec::new()),
        };
        let fired = fire_entry(
            &module,
            "run",
            &serde_json::json!({"path": "agents/x/x.md"}),
            &probe_ctx(),
            &host,
            EvalLimits::default(),
        )
        .expect("`run` exists");
        assert!(fired.fault.is_none(), "fire faulted: {:?}", fired.fault);
        assert_eq!(
            fired.value, None,
            "falling off the end is `None` — no answer, and that is legal"
        );
        assert_eq!(fired.effects.len(), 1);
        assert_eq!(fired.effects[0].kind, EffectKind::Create);
        assert_eq!(
            fired.effects[0].args.get("path"),
            Some(&ArgValue::Str("agents/x/x.md".to_owned()))
        );
    }

    /// An answer outside § 2.2's admitted set is `reply_shape`, named by
    /// type — not `runtime`, which would send the author hunting a bug that
    /// is not in their program.
    #[test]
    fn a_reply_outside_the_admitted_set_is_reply_shape() {
        let source = "\
def run(event):
    return run
";
        let loaded = load_block(source, None, &probe_ctx(), EvalLimits::default());
        assert!(loaded.fault.is_none(), "load faulted: {:?}", loaded.fault);
        let module = loaded.module.expect("a clean load freezes");
        let host = ProbeHost {
            calls: std::sync::Mutex::new(Vec::new()),
        };
        let fired = fire_entry(
            &module,
            "run",
            &serde_json::Value::Null,
            &probe_ctx(),
            &host,
            EvalLimits::default(),
        )
        .expect("`run` exists");
        let fault = fired.fault.expect("a function is not a reply");
        assert_eq!(fault.class, FaultClass::ReplyShape);
        assert_eq!(fault.class.as_str(), "reply_shape");
        assert!(
            fault.reason.contains("function"),
            "the refusal names what was returned: {}",
            fault.reason
        );
    }

    /// A fire naming an entry the module does not define is `MissingEntry` —
    /// typed, before any evaluation.
    #[test]
    fn a_fire_on_an_undefined_entry_is_missing_entry() {
        let loaded = load_block("x = 1\n", None, &probe_ctx(), EvalLimits::default());
        let module = loaded.module.expect("a clean load freezes");
        let host = ProbeHost {
            calls: std::sync::Mutex::new(Vec::new()),
        };
        let outcome = fire_entry(
            &module,
            "run",
            &serde_json::Value::Null,
            &probe_ctx(),
            &host,
            EvalLimits::default(),
        );
        assert!(
            matches!(outcome, Err(EvalError::MissingEntry { .. })),
            "a missing entry is typed, not a runtime fault: {outcome:?}"
        );
    }

    /// Every entry holds its surface separately, and the script
    /// builtins never join the hooked planes' — otherwise `on_change` rules
    /// would gain live reads and the change plane would stop being hermetic by
    /// construction.
    #[test]
    fn every_entry_has_its_own_global_surface() {
        let hooked = plane_surface(&effect_globals());
        let script = plane_surface(&script_globals(&[]));
        println!("POPULATION hooked-plane surface = {hooked:?}");
        println!("POPULATION script-plane surface = {script:?}");

        // `on_change` and `run` share one surface — the shipped design, and it
        // is byte-unchanged by the arrival of the script entry.
        let expected_hooked: HashSet<String> = EffectKind::ALL
            .iter()
            .map(|k| k.constructor().to_owned())
            .chain(crate::REACTION_VOCAB.iter().map(|s| (*s).to_owned()))
            .collect();
        assert_eq!(hooked, expected_hooked, "the hooked planes are unchanged");

        let expected_script: HashSet<String> = ["read", "me", "put"]
            .into_iter()
            .map(ToOwned::to_owned)
            .collect();
        assert_eq!(
            script, expected_script,
            "the PURE script plane adds read/me/put only — provably pure by \
             default (script-effects ruling: #17 stands on this path)"
        );

        // A1 assertion (1): the hook plane is the hooked surface plus exactly
        // three names. Written as a set equality over `EffectKind::ALL` rather
        // than a literal list so a new effect kind cannot join the globals
        // without this test noticing — the structural half of "an effect
        // builtin without the gate cannot exist".
        let hook = plane_surface(&hook_globals());
        println!("POPULATION hook-plane surface = {hook:?}");
        let expected_hook: HashSet<String> = expected_hooked
            .iter()
            .cloned()
            .chain(
                ["declare", "exec", "bash"]
                    .into_iter()
                    .map(ToOwned::to_owned),
            )
            .collect();
        assert_eq!(
            hook, expected_hook,
            "the hook plane is the effect surface ∪ {{declare, exec, bash}} — ONE closed \
             set for compile, freeze and fire (A1); a name here that is not in the table \
             is a builtin nothing gates"
        );

        // Effects mode: the admitted builtin joins EXACTLY when named — the
        // sanctioned exec surface (#17 overturned on the flagged path only).
        let live = plane_surface(&script_globals(&["run".to_owned()]));
        let expected_live: HashSet<String> = ["read", "me", "put", "run"]
            .into_iter()
            .map(ToOwned::to_owned)
            .collect();
        assert_eq!(
            live, expected_live,
            "effects:[\"run\"] adds exactly the admitted builtin"
        );
        let measured = plane_surface(&script_globals(&["token_count".to_owned()]));
        let expected_measured: HashSet<String> = ["read", "me", "put", "token_count"]
            .into_iter()
            .map(ToOwned::to_owned)
            .collect();
        assert_eq!(
            measured, expected_measured,
            "effects:[\"token_count\"] adds exactly the admitted builtin — never run"
        );
        let both = plane_surface(&script_globals(&[
            "run".to_owned(),
            "token_count".to_owned(),
        ]));
        let expected_both: HashSet<String> = ["read", "me", "put", "run", "token_count"]
            .into_iter()
            .map(ToOwned::to_owned)
            .collect();
        assert_eq!(
            both, expected_both,
            "the admitted set composes — each name admits its own builtin"
        );
        assert!(
            !hooked.contains("put"),
            "the script arming surface must never reach the change or run plane"
        );

        assert!(
            hooked.is_disjoint(&script),
            "the two surfaces must not intersect: {:?}",
            hooked.intersection(&script).collect::<Vec<_>>()
        );
        assert!(
            !hooked.contains("read"),
            "a live read must never reach the change or run plane"
        );
        for forbidden in [
            "exec",
            "subprocess",
            "os",
            "open",
            "load",
            "eval",
            "print",
            "glob",
        ] {
            assert!(
                !script.contains(forbidden),
                "the script sandbox must not expose `{forbidden}` (decision #17)"
            );
        }
    }

    /// Pin `EffectKind::constructor` ↔ registered globals both ways.
    #[test]
    fn every_effect_kind_constructor_is_registered() {
        let names: HashSet<String> = effect_globals()
            .names()
            .map(|n| n.as_str().to_owned())
            .collect();
        let derived: HashSet<String> = EffectKind::ALL
            .iter()
            .map(|k| k.constructor().to_owned())
            .collect();
        println!("POPULATION derived constructors = {derived:?}");

        for kind in EffectKind::ALL {
            assert!(
                names.contains(kind.constructor()),
                "{} names constructor `{}`, which is not registered",
                kind.as_str(),
                kind.constructor()
            );
            assert_eq!(
                EffectKind::from_wire_name(kind.as_str()),
                Some(kind),
                "wire identity must round-trip"
            );
        }

        // non-standard globals == EffectKind constructors ∪ REACTION_VOCAB.
        let standard: HashSet<String> = GlobalsBuilder::standard()
            .build()
            .names()
            .map(|n| n.as_str().to_owned())
            .collect();
        let registered_ctors: HashSet<String> = names.difference(&standard).cloned().collect();
        let expected: HashSet<String> = derived
            .iter()
            .cloned()
            .chain(crate::REACTION_VOCAB.iter().map(|s| (*s).to_string()))
            .collect();
        println!("POPULATION registered non-standard globals = {registered_ctors:?}");
        println!("POPULATION reaction vocab = {:?}", crate::REACTION_VOCAB);
        assert_eq!(
            registered_ctors, expected,
            "a global exists that neither an EffectKind nor REACTION_VOCAB names (or vice versa)"
        );
    }

    /// `receipt_addr` pure in `(path, rev)` — address re-derivable after delivery.
    #[test]
    fn receipt_address_is_pure_and_collision_separated() {
        let a = crate::receipt_address("tasks/t.md", "abc123");
        println!("POPULATION receipt_address = {a}");
        assert_eq!(a, crate::receipt_address("tasks/t.md", "abc123"), "pure");
        assert!(a.starts_with("tasks/t.md#^r-"), "path passes through: {a}");
        assert_ne!(
            a,
            crate::receipt_address("tasks/t.md", "abc124"),
            "rev matters"
        );
        assert_ne!(
            a,
            crate::receipt_address("tasks/u.md", "abc123"),
            "path matters"
        );
        // The separator earns its place: without it these two would collide.
        assert_ne!(
            crate::receipt_address("a", "bc"),
            crate::receipt_address("ab", "c"),
            "the \\0 separator prevents a concatenation collision"
        );
        let anchor = a.rsplit("#^").next().unwrap();
        assert!(
            anchor
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-'),
            "the anchor stays inside the one block-id charset: {anchor}"
        );
    }

    /// Paths with `#` (or empty) refused — would make `path#^anchor` ambiguous.
    #[test]
    fn receipt_addr_refuses_a_path_that_would_make_the_address_ambiguous() {
        let event = ChangeEvent::new("f.md", "a", "b");
        for bad in ["", "notes/plan.md#Goals"] {
            let src = format!("def on_change(event):\n    receipt_addr({bad:?}, \"rev\")\n");
            let err = crate::eval(&[Rule::new("r", src)], &event)
                .expect_err("an ambiguous receipt path must fault");
            println!("POPULATION receipt_addr({bad:?}) -> {err}");
        }
        // The control: a normal path still mints an address, so the refusals above
        // are about `#` and emptiness, not about the builtin being unreachable.
        let ok = crate::eval(
            &[Rule::new(
                "r",
                "def on_change(event):\n    receipt_addr(\"tasks/t.md\", \"rev\")\n",
            )],
            &event,
        );
        assert!(ok.is_ok(), "a normal path mints an address: {ok:?}");
    }

    /// Wire identities + one alias; unknown action is a fault.
    #[test]
    fn action_kind_resolves_wire_names_and_the_one_alias() {
        assert_eq!(crate::action_kind("notify"), Some(EffectKind::Send));
        for kind in EffectKind::ALL {
            assert_eq!(crate::action_kind(kind.as_str()), Some(kind));
        }
        for unknown in ["send", "shout", "", "proto.telepathy", "NOTIFY"] {
            assert_eq!(
                crate::action_kind(unknown),
                None,
                "{unknown} must not resolve"
            );
        }
    }

    /// Closed surface: every effect constructor present; no I/O/eval names.
    #[test]
    fn globals_surface_is_the_closed_capability_set() {
        let globals = effect_globals();
        let names: HashSet<String> = globals.names().map(|n| n.as_str().to_owned()).collect();

        for ctor in [
            "set_field",
            "append_section",
            "refresh_view",
            "send",
            "remind",
            "ask",
            "notice",
        ] {
            assert!(names.contains(ctor), "missing constructor `{ctor}`");
        }
        for forbidden in [
            "open",
            "read",
            "write",
            "read_file",
            "write_file",
            "os",
            "sys",
            "socket",
            "load",
            "eval",
            "exec",
            "import",
            "__import__",
            "print",
            "pprint",
            "debug",
        ] {
            assert!(
                !names.contains(forbidden),
                "sandbox must not expose I/O name `{forbidden}`"
            );
        }
    }

    fn rule(src: &str) -> Rule {
        Rule::new("t", src)
    }

    #[test]
    fn nesting_guard_rejects_deep_brackets() {
        let src = format!("x = {}{}", "[".repeat(600), "]".repeat(600));
        assert!(matches!(
            check_nesting_depth(&rule(&src)),
            Err(EvalError::Parse { .. })
        ));
    }

    #[test]
    fn nesting_guard_rejects_deep_unary() {
        let src = format!("x = {}1", "-".repeat(600));
        assert!(matches!(
            check_nesting_depth(&rule(&src)),
            Err(EvalError::Parse { .. })
        ));
    }

    #[test]
    fn nesting_guard_rejects_deep_not_chain() {
        let src = format!("x = {}True", "not ".repeat(600));
        assert!(matches!(
            check_nesting_depth(&rule(&src)),
            Err(EvalError::Parse { .. })
        ));
    }

    #[test]
    fn nesting_guard_threshold_is_exclusive() {
        // The cap is `>`: exactly MAX_NESTING_DEPTH is admitted, one deeper is
        // rejected — for both the bracket and unary-run counters.
        let at_brackets = format!(
            "{}{}",
            "[".repeat(MAX_NESTING_DEPTH),
            "]".repeat(MAX_NESTING_DEPTH)
        );
        assert!(
            check_nesting_depth(&rule(&at_brackets)).is_ok(),
            "exactly at the cap is allowed"
        );
        let over_brackets = format!(
            "{}{}",
            "[".repeat(MAX_NESTING_DEPTH + 1),
            "]".repeat(MAX_NESTING_DEPTH + 1)
        );
        assert!(
            matches!(
                check_nesting_depth(&rule(&over_brackets)),
                Err(EvalError::Parse { .. })
            ),
            "one over the cap is rejected"
        );

        let at_unary = format!("x = {}1", "-".repeat(MAX_NESTING_DEPTH));
        assert!(check_nesting_depth(&rule(&at_unary)).is_ok());
        let over_unary = format!("x = {}1", "-".repeat(MAX_NESTING_DEPTH + 1));
        assert!(matches!(
            check_nesting_depth(&rule(&over_unary)),
            Err(EvalError::Parse { .. })
        ));

        // The `not`-keyword unary arm has its own threshold check.
        let at_not = format!("x = {}True", "not ".repeat(MAX_NESTING_DEPTH));
        assert!(check_nesting_depth(&rule(&at_not)).is_ok());
        let over_not = format!("x = {}True", "not ".repeat(MAX_NESTING_DEPTH + 1));
        assert!(matches!(
            check_nesting_depth(&rule(&over_not)),
            Err(EvalError::Parse { .. })
        ));
    }

    #[test]
    fn skip_string_consumes_only_the_string_not_the_rest() {
        // A string literal must be skipped exactly — real nesting after it
        // still counts.
        let src = format!(
            "a = \"x\"\nb = {}{}\n",
            "(".repeat(MAX_NESTING_DEPTH + 1),
            ")".repeat(MAX_NESTING_DEPTH + 1)
        );
        assert!(matches!(
            check_nesting_depth(&rule(&src)),
            Err(EvalError::Parse { .. })
        ));
    }

    #[test]
    fn nesting_guard_admits_shallow_but_wide() {
        // Many brackets, all depth 1 — must not trip (bounds depth, not count).
        use std::fmt::Write as _;
        let mut src = String::new();
        for i in 0..2000 {
            let _ = writeln!(src, "_{i} = (1, 2)");
        }
        assert!(check_nesting_depth(&rule(&src)).is_ok());
    }

    #[test]
    fn nesting_guard_ignores_brackets_inside_strings() {
        // 600 open brackets, but inside a string literal — not real nesting.
        let inner = "[".repeat(600);
        let src = format!("x = \"{inner}\"\n");
        assert!(check_nesting_depth(&rule(&src)).is_ok());
    }

    #[test]
    fn nesting_guard_ignores_brackets_in_triple_string() {
        let inner = "(".repeat(600);
        let src = format!("x = \"\"\"{inner}\"\"\"\n");
        assert!(check_nesting_depth(&rule(&src)).is_ok());
    }

    #[test]
    fn nesting_guard_ignores_operators_in_comments() {
        let run = "-".repeat(600);
        let src = format!("x = 1  # {run}\n");
        assert!(check_nesting_depth(&rule(&src)).is_ok());
    }

    #[test]
    fn word_at_respects_identifier_boundaries() {
        assert!(word_at(b"not x", 0, b"not"));
        assert!(word_at(b"a not b", 2, b"not"));
        // `not` as a prefix of `notes` is not the keyword.
        assert!(!word_at(b"notes", 0, b"not"));
        assert!(!word_at(b"cannot", 3, b"not"));
    }

    #[test]
    fn not_prefix_identifier_does_not_count_as_unary() {
        // `notes` repeated is identifiers, not a unary chain.
        let mut src = String::from("x = ");
        for _ in 0..600 {
            src.push_str("notes ");
        }
        src.push('1');
        assert!(check_nesting_depth(&rule(&src)).is_ok());
    }

    #[test]
    fn long_n_run_is_one_identifier_not_a_not_chain() {
        // A single 2000-char identifier of all `n`s must count as one operand,
        // not 2000 phantom `not`s.
        let src = format!("x = {}\n", "n".repeat(2000));
        assert!(check_nesting_depth(&rule(&src)).is_ok());
    }

    #[test]
    fn skip_string_handles_escaped_quote() {
        // `"a\"b"` — the escaped quote does not close the string.
        let s = b"\"a\\\"b\" rest";
        let end = skip_string(s, 0);
        assert_eq!(&s[end..], b" rest");
    }

    #[test]
    fn skip_string_stops_at_newline_for_unterminated_single_line() {
        // An unterminated single-line string ends at the newline (the parser then
        // reports the real syntax error); the scanner does not run off the end.
        let s = b"\"unterminated\nnext";
        let end = skip_string(s, 0);
        assert_eq!(s[end], b'\n');
    }

    #[test]
    fn nesting_guard_admits_a_real_rule_with_brackets_in_strings() {
        // A realistic rule whose strings contain brackets and dashes must pass.
        let src = "def on_change(event):\n    notice(message = \"a - b (see [1])\")\n";
        assert!(check_nesting_depth(&rule(src)).is_ok());
    }
}
