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

use starlark::environment::{Globals, GlobalsBuilder, Module};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::syntax::{AstModule, Dialect};
use starlark::values::Heap;
use starlark::values::Value;
use starlark::values::dict::AllocDict;
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
    Emit(EmitStore),
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

struct EmitStore {
    rule_id: String,
    provenance: Provenance,
    depth: u32,
    effects: RefCell<Vec<Effect>>,
    next_seq: Cell<u32>,
    /// The phase gate. [`Fire`](EffectPhase::Fire) for every shipped plane,
    /// so nothing existing changes behavior.
    phase: EffectPhase,
}

impl EmitStore {
    fn new(rule_id: &str, provenance: Provenance, depth: u32) -> Self {
        Self {
            rule_id: rule_id.to_owned(),
            provenance,
            depth,
            effects: RefCell::new(Vec::new()),
            next_seq: Cell::new(0),
            // The shipped planes are single-phase and effectful throughout.
            phase: EffectPhase::Fire,
        }
    }

    /// The same store, gated to a phase — the load half of the run entry's
    /// two-phase evaluation.
    fn in_phase(mut self, phase: EffectPhase) -> Self {
        self.phase = phase;
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
fn store<'a>(
    eval: &'a Evaluator<'_, '_, '_>,
    builtin: &'static str,
) -> anyhow::Result<&'a EmitStore> {
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
    fn create(
        #[starlark(require = named)] path: String,
        #[starlark(require = named)] body: String,
        #[starlark(require = named)] base: Option<String>,
        #[starlark(require = named)] message: Option<String>,
        eval: &mut Evaluator<'_, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let mut args = BTreeMap::new();
        args.insert("path".to_owned(), ArgValue::Str(path));
        args.insert("body".to_owned(), ArgValue::Str(body));
        insert_opt(&mut args, "base", base);
        insert_opt(&mut args, "message", message);
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

/// Evaluate one starlark block's TOP LEVEL in the load phase.
///
/// No hook is looked up and none is called: publishing what a block declares
/// IS evaluating its top level, exactly as on the script plane. Every effect
/// builtin the top level reaches refuses `effect_at_load` at the accessor
/// gate, so a block that tries to act while declaring says so at its own
/// line rather than acting.
#[must_use]
pub fn load_block(source: &str, ctx: &RunCtx, limits: EvalLimits) -> BlockLoad {
    let block = Rule::new(&ctx.task, source.to_owned());
    let globals = effect_globals();
    // Same large stack as every other entry: pathological nesting must fault,
    // never abort the process.
    let run = on_eval_stack(|| metered_eval(&globals, &block, &EvalEntry::RunLoad(ctx), limits));
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
        effects,
        fault,
        fuel_used: run.fuel_used,
        mem_used: run.mem_used,
    }
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

            let EntryRun {
                aborted,
                fault_class,
                depth_overflow,
                fault,
                fault_line,
                missing,
                wrong_plane,
            } = dispatch_entry(&mut eval, &module, entry, ast, globals, arg_value);

            let used_steps = eval.get_total_tick_count();
            let used_mem = heap_bytes(module.heap());
            drop(eval);
            let effects = match &store {
                PlaneStore::Emit(store) => store.effects.take(),
                // The script plane arms no `md.*` descriptors (plan decision 7).
                PlaneStore::Script(_) => Vec::new(),
            };

            let over_budget = used_steps > limits.fuel || used_mem > limits.mem;
            let outcome = classify_outcome(
                rule,
                entry,
                effects,
                &Aborted {
                    aborted,
                    depth_overflow,
                    over_budget,
                    missing,
                    wrong_plane,
                    fault,
                    fault_line,
                },
                limits,
            );
            RuleRun {
                fuel_used: used_steps,
                mem_used: used_mem,
                outcome,
                // Budget is the kernel's OWN accounting, not starlark's word.
                fault_class: if over_budget || depth_overflow {
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

/// How one entry's evaluation ended, before it is turned into an outcome.
/// A struct rather than seven positional arguments — the booleans are all
/// the same type and swapping two of them would compile.
struct Aborted {
    aborted: bool,
    depth_overflow: bool,
    over_budget: bool,
    missing: bool,
    wrong_plane: Option<&'static str>,
    fault: Option<String>,
    fault_line: Option<u32>,
}

/// The typed outcome of one metered evaluation. Extracted from
/// [`metered_eval`] so the eval body stays readable; the precedence is the
/// shipped one, unchanged: a missing entry first, then abort (budget before
/// genuine fault), then an over-budget completion.
fn classify_outcome(
    rule: &Rule,
    entry: &EvalEntry<'_>,
    effects: Vec<Effect>,
    end: &Aborted,
    limits: EvalLimits,
) -> Result<Vec<Effect>, EvalError> {
    if end.missing {
        return Err(EvalError::MissingEntry {
            rule_id: rule.id.clone(),
            expected: entry.hook().unwrap_or_default(),
            wrong_plane: end.wrong_plane,
        });
    }
    if end.aborted {
        // over_budget / StackOverflow → budget; else genuine fault.
        return if end.over_budget || end.depth_overflow {
            Err(budget(limits))
        } else {
            Err(EvalError::Runtime {
                rule_id: rule.id.clone(),
                reason: end.fault.clone().unwrap_or_default(),
                line: end.fault_line,
            })
        };
    }
    if end.over_budget {
        // Completed without abort but exact mem still over — budget.
        return Err(budget(limits));
    }
    Ok(effects)
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
    fn probe_store(phase: EffectPhase) -> EmitStore {
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
    fn load_module(
        source: &str,
        store: &PlaneStore<'_>,
    ) -> Result<starlark::environment::FrozenModule, String> {
        Module::with_temp_heap(|module| {
            let ast = AstModule::parse("probe", source.to_owned(), &rule_dialect())
                .map_err(|e| format!("parse: {e}"))?;
            let mut eval = Evaluator::new(&module);
            eval.extra = Some(store);
            let outcome = eval.eval_module(ast, &effect_globals());
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
        (EffectKind::SetField, r#"set_field(field = "s", value = "v")"#),
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
        let (class, line) = classify("\ncreate(path = \"p.md\", body = \"b\")\n", EffectPhase::Load);
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

    /// The three entries hold their surfaces separately, and the script
    /// builtins never join the hooked planes' — otherwise `on_change` rules
    /// would gain live reads and the change plane would stop being hermetic by
    /// construction.
    #[test]
    fn the_three_entries_have_separate_global_surfaces() {
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
