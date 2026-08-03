//! The sealed Starlark evaluation bridge: build the closed globals (standard
//! library + the effect-descriptor builtins, and NOTHING with ambient I/O),
//! inject the [`ChangeEvent`] as a `event` struct, run one rule's
//! `on_change(event)` metered under the fuel/mem budget, and record the effect
//! descriptors it constructs.
//!
//! Everything here is `pub(crate)`; the public surface is the types + `eval` in
//! `lib.rs`. The design mirrors the proven `policy` embedding: `eval.extra`
//! carries the emit store, `set_max_tick_count` / `set_max_heap_size` meter the
//! budget with EXACT post-eval accounting, and [`std::panic::catch_unwind`]
//! converts a resource-overflow panic inside the engine into a clean budget
//! error so a bomb can never escape and crash the caller.
//!
//! # Purity
//! The globals are `GlobalsBuilder::standard().with(effect_api)`. The Starlark
//! standard library has zero ambient I/O — no file, net, os, clock, or random
//! (research-starlark-mcp §1: "the only capabilities a rule gets are the builtins
//! you register"). The builtins registered here construct inert descriptors and
//! read nothing. A rule that names `open`, `print`, `read`, `now`, … reaches a
//! name the sandbox never bound and faults with [`EvalError::Runtime`].

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

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

use crate::{
    ArgValue, ChangeEvent, ChangeFact, Effect, EffectKind, EvalError, EvalLimits, EventFacts,
    Provenance, Rule, RunCtx,
};

/// An optional string as a Starlark value: the string, or `None`. Absence stays
/// absence — a missing frontmatter value must not arrive as `""`, or a predicate
/// cannot tell "the key was empty" from "the key was not there".
fn opt_str<'v>(heap: Heap<'v>, o: Option<&str>) -> Value<'v> {
    match o {
        Some(s) => heap.alloc(s),
        None => Value::new_none(),
    }
}

/// Max parser NESTING depth — brackets `([{` or a run of consecutive unary
/// operators (`not`, `-`, `+`, `~`). The Starlark recursive-descent parser
/// recurses once per nesting level; a deep chain (issue #66: `not not not …`)
/// overflows the native stack and ABORTS the process — `catch_unwind` cannot
/// catch a stack overflow, so it must be PREVENTED. Bounding nesting depth BEFORE
/// parsing is O(1) stack and independent of the parser's (large, debug-inflated)
/// frame size — the robust guard. 500 is orders of magnitude beyond any real
/// rule (nesting depth, NOT total size — a large but shallow rule is fine).
pub(crate) const MAX_NESTING_DEPTH: usize = 500;

/// The evaluation thread's stack size. Even with nesting bounded to
/// [`MAX_NESTING_DEPTH`], 500 debug parse frames plus the bounded runtime
/// call-stack ([`crate::EvalLimits::max_call_depth`]) exceed a default 2 MiB
/// thread stack, so evaluation runs on a dedicated 128 MiB stack (virtual
/// address space, not physically committed until touched) — ample headroom over
/// the ~10 MiB worst case the depth guard admits.
pub(crate) const EVAL_STACK_BYTES: usize = 128 * 1024 * 1024;

/// Run `f` on a dedicated large-stack thread and return its result. The parse +
/// eval must run here so pathologically nested source cannot overflow the native
/// stack and abort the process (see [`EVAL_STACK_BYTES`]). Scoped, so borrowed
/// `rules`/`event` need no `'static` bound; the thread always joins before return.
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

/// Build the closed rule-language globals: the Starlark standard library plus the
/// effect-descriptor constructors. This is the ENTIRE capability surface a rule
/// sees. `standard()` carries no ambient I/O and (verified in tests) no `print` /
/// `pprint` / `debug` stream builtins.
pub(crate) fn effect_globals() -> Globals {
    GlobalsBuilder::standard().with(effect_api).build()
}

/// The rule dialect: the standard grammar with `load` DISABLED. A rule is a
/// single self-contained `on_change` definition; module loading (`load(...)`)
/// would let a rule pull in external symbols — a capability the pure kernel does
/// not grant. Disabling `enable_load` makes `load` a parse error, so a rule
/// cannot even express it. (`enable_load_reexport` is moot once load is off, so
/// it is left at the Standard default.)
fn rule_dialect() -> Dialect {
    Dialect {
        enable_load: false,
        ..Dialect::Standard
    }
}

/// Parse-check every rule without running it — the load gate. Catches authoring
/// faults (over-long source, unparseable Starlark) once, up front, so per-event
/// [`crate::eval`] failures are genuine per-event faults.
///
/// # Errors
/// [`EvalError::SourceTooLarge`] or [`EvalError::Parse`] for the first bad rule.
pub fn validate(rules: &[Rule], limits: EvalLimits) -> Result<(), EvalError> {
    // Parsing runs on the large evaluation stack for the same reason `eval` does:
    // pathological nesting must not overflow the native stack.
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

/// Refuse a rule whose source exceeds the parse-DoS byte cap before it reaches
/// the parser.
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

/// Refuse a rule whose nesting depth would recurse the parser past
/// [`MAX_NESTING_DEPTH`] and overflow the native stack (an uncatchable abort).
///
/// A single conservative left-to-right byte scan that skips string- and
/// comment-content (so brackets/operators inside a literal do not count) and
/// tracks the running bracket-nesting depth and the length of a consecutive
/// unary-operator run (the issue-#66 vector). A run is only broken by an operand
/// (identifier / number / literal) or a bracket — whitespace between operators
/// does not, so `- - - x` counts as depth 3. Over-approximation is safe: it can
/// only reject the pathological, never a real shallow rule.
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

/// The store the effect constructors record into, reached via `Evaluator::extra`.
/// It carries the emitting rule's id, the plane-typed [`Provenance`] to stamp,
/// and the event depth, so each constructor records a complete [`Effect`], plus
/// a per-rule `seq` counter. The `on_change` path always stamps change-plane
/// provenance (the event's diff fingerprints); a run-plane entry point stamps
/// [`Provenance::Run`] at construction.
#[derive(starlark::any::ProvidesStaticType)]
struct EmitStore {
    rule_id: String,
    provenance: Provenance,
    depth: u32,
    effects: RefCell<Vec<Effect>>,
    next_seq: Cell<u32>,
}

impl EmitStore {
    fn new(rule_id: &str, provenance: Provenance, depth: u32) -> Self {
        Self {
            rule_id: rule_id.to_owned(),
            provenance,
            depth,
            effects: RefCell::new(Vec::new()),
            next_seq: Cell::new(0),
        }
    }

    /// Record one descriptor, stamping the rule id, plane-typed provenance,
    /// depth, and the next per-rule `seq`.
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

/// Downcast the emit store out of `Evaluator::extra`. Absent only if a constructor
/// is somehow reached outside a metered run — a loud fault, never a silent drop.
fn store<'a>(eval: &'a Evaluator<'_, '_, '_>) -> anyhow::Result<&'a EmitStore> {
    eval.extra
        .and_then(|e| e.downcast_ref::<EmitStore>())
        .ok_or_else(|| anyhow::anyhow!("effect-api: constructor invoked without an emit store"))
}

/// Insert an optional scalar argument when present.
fn insert_opt(args: &mut BTreeMap<String, ArgValue>, key: &str, value: Option<String>) {
    if let Some(value) = value {
        args.insert(key.to_owned(), ArgValue::Str(value));
    }
}

/// The effect-descriptor constructors — the ENTIRE rule-language builtin surface.
/// Every argument is named (`require = named`) so a rule reads as keyword calls
/// and a positional-shape mistake faults loudly. Each returns `None`; the effect
/// is the recorded descriptor, not a value. NO constructor performs or exposes
/// I/O.
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
        store(eval)?.push(EffectKind::SetField, args);
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
        store(eval)?.push(EffectKind::AppendSection, args);
        Ok(NoneType)
    }

    /// `daemon.refresh_view` — mark the resident `view` stale.
    fn refresh_view(
        #[starlark(require = named)] view: String,
        eval: &mut Evaluator<'_, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let mut args = BTreeMap::new();
        args.insert("view".to_owned(), ArgValue::Str(view));
        store(eval)?.push(EffectKind::RefreshView, args);
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
        store(eval)?.push(EffectKind::Send, args);
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
        store(eval)?.push(EffectKind::Remind, args);
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
        store(eval)?.push(EffectKind::Ask, args);
        Ok(NoneType)
    }

    /// `intent(action, target, severity, payload, receipt)` — the reaction
    /// plane's constructor, in the panel's own noun. `action` names the effect
    /// kind: a wire identity (`proto.send`) or the one documented alias
    /// (`notify` ≈ `proto.send`). Every other argument is carried VERBATIM — the
    /// engine never composes a message, resolves a target, or ranks importance;
    /// that is the HOW, and it is Go's business.
    ///
    /// This grants no capability. It can name any kind, including one the
    /// convention did not declare — and that is deliberate: the load ceiling can
    /// only see constructor NAMES, so a kind chosen by a runtime string is caught
    /// downstream by the capability filter, which drops it and REPORTS it. The two
    /// layers are complementary, not redundant.
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
        store(eval)?.push(kind, args);
        Ok(NoneType)
    }

    /// `receipt_addr(path, rev)` — the address an outcome will be written to,
    /// minted BEFORE any delivery is attempted (constraint 8). Returns the
    /// canonical `path#^anchor` string, the contract's own §6.1 spelling.
    ///
    /// `path` passes through untouched: the predicate decides WHERE the receipt
    /// lives. Only the anchor is the engine's, and it is a pure function of
    /// `(path, rev)` — no clock, no counter, no invented directory layout — so the
    /// same change re-evaluated names the same address, which is what makes an
    /// undelivered intent findable rather than merely regrettable.
    fn receipt_addr(
        #[starlark(require = pos)] path: String,
        #[starlark(require = pos)] rev: String,
    ) -> anyhow::Result<String> {
        // A `#` in the path would make `path#^anchor` ambiguous — the address
        // could not be split back into its two halves. Refuse it here rather than
        // mint an address nobody can resolve.
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
        store(eval)?.push(EffectKind::Notice, args);
        Ok(NoneType)
    }
}

/// Peak eval-heap bytes as `u64` (saturating). Peak (not current) is monotonic
/// and matches the quantity `set_max_heap_size` guards, so the guard's abort and
/// this post-eval reading agree, and a post-eval GC cannot deflate it.
fn heap_bytes(heap: Heap<'_>) -> u64 {
    u64::try_from(heap.peak_allocated_bytes()).unwrap_or(u64::MAX)
}

/// The metered outcome of one rule run: its typed result plus the EXACT
/// post-eval resource accounting (`fuel_used` Starlark ticks, `mem_used` peak
/// heap bytes) the result was judged against. The corpus-replay harness reads
/// `fuel_used`/`mem_used` for its per-rule consumption profile; a parse-time or
/// setup-time failure never reached eval, so both read `0` there. A bomb
/// terminated by the fuel/mem ceiling reports the ceiling it hit.
pub(crate) struct RuleRun {
    pub(crate) fuel_used: u64,
    pub(crate) mem_used: u64,
    pub(crate) outcome: Result<Vec<Effect>, EvalError>,
}

impl RuleRun {
    /// A run that never reached eval (source-cap, nesting, or evaluator setup) —
    /// no fuel spent, the typed authoring/setup fault.
    fn failed(outcome: EvalError) -> Self {
        Self {
            fuel_used: 0,
            mem_used: 0,
            outcome: Err(outcome),
        }
    }
}

/// Run ONE rule's `on_change(event)` over the injected event, returning only the
/// typed effect result — the batch-eval path (`eval_with_limits`) uses this and
/// discards the metering. See [`run_rule_metered`] for the full contract.
pub(crate) fn run_rule(
    globals: &Globals,
    rule: &Rule,
    event: &ChangeEvent,
    limits: EvalLimits,
) -> Result<Vec<Effect>, EvalError> {
    run_rule_metered(globals, rule, event, limits).outcome
}

/// Run ONE task's `run(ctx)` over the injected context — the run-plane entry.
/// Identical machinery to the change plane ([`metered_eval`]): same globals,
/// same guards, same metering, same panic containment. Only the entry name,
/// the injected value, and the stamped provenance differ.
pub(crate) fn run_task(
    globals: &Globals,
    task: &Rule,
    ctx: &RunCtx,
    limits: EvalLimits,
) -> Result<Vec<Effect>, EvalError> {
    metered_eval(globals, task, &EvalEntry::Run(ctx), limits).outcome
}

/// The plane an evaluation enters on: which hook is called, what value is
/// injected, and what provenance the emitted effects carry. One entry per
/// plane — the planes never cross (a wrong-plane source is a typed
/// [`EvalError::MissingEntry`]).
pub(crate) enum EvalEntry<'a> {
    /// `on_change(event)` — change-plane provenance from the event's diff.
    Change(&'a ChangeEvent),
    /// `run(ctx)` — Run-plane provenance from the caller-supplied facts.
    Run(&'a RunCtx),
}

impl EvalEntry<'_> {
    /// The hook name this plane calls.
    fn name(&self) -> &'static str {
        match self {
            EvalEntry::Change(_) => "on_change",
            EvalEntry::Run(_) => "run",
        }
    }

    /// The OTHER plane's hook name (for the wrong-plane diagnosis).
    fn other(&self) -> &'static str {
        match self {
            EvalEntry::Change(_) => "run",
            EvalEntry::Run(_) => "on_change",
        }
    }

    /// The emit store this plane stamps: provenance + depth.
    fn store(&self, rule: &Rule) -> EmitStore {
        match self {
            EvalEntry::Change(event) => EmitStore::new(
                &rule.id,
                Provenance::Change {
                    fingerprint_before: event.fingerprint_before.clone(),
                    fingerprint_after: event.fingerprint_after.clone(),
                },
                event.depth,
            ),
            EvalEntry::Run(ctx) => EmitStore::new(
                &ctx.task,
                Provenance::Run {
                    invocation_id: ctx.invocation_id.clone(),
                    root_at_eval: ctx.root_at_eval.clone(),
                },
                0,
            ),
        }
    }

    /// Allocate the injected argument value on the eval heap.
    fn alloc<'v>(&self, heap: Heap<'v>) -> Value<'v> {
        match self {
            EvalEntry::Change(event) => alloc_event(heap, event),
            EvalEntry::Run(ctx) => alloc_ctx(heap, ctx),
        }
    }
}

/// Run ONE rule's `on_change(event)` over the injected event, metered, returning
/// the typed result AND the exact fuel/mem it spent.
///
/// Fuel/mem are enforced two ways, mirroring `policy`: Starlark's own
/// `set_max_*` guards abort a runaway loop/recursion at coarse (~1000-event)
/// boundaries, and the EXACT post-eval accounting (`get_total_tick_count`, peak
/// heap bytes) is the authoritative bound the result is judged against. A
/// non-looping allocation that defeats the coarse guard is still caught by the
/// exact mem accounting; a pathological allocation that trips a `len overflow`
/// assert INSIDE the engine is caught by [`std::panic::catch_unwind`] and mapped
/// to [`EvalError::Budget`] (the only panic reachable inside the metered eval is
/// resource overflow) — so a bomb terminates, never hangs, never crashes.
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

    // AssertUnwindSafe: on panic we discard `store` unread and return a budget
    // fault, so its interior mutability cannot leak an inconsistent value across
    // the boundary.
    let evaluated = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        Module::with_temp_heap(|module| {
            let heap = module.heap();
            let arg_value = entry.alloc(heap);

            let mut eval = Evaluator::new(&module);
            if let Err(e) = eval.set_max_tick_count(step_guard) {
                return RuleRun::failed(runtime(rule, e));
            }
            if let Err(e) = eval.set_max_heap_size(mem_guard) {
                return RuleRun::failed(runtime(rule, e));
            }
            // Bound recursion DEPTH: unbounded Starlark recursion consumes native
            // frames and would overflow even the large evaluation stack before the
            // tick guard fires. The callstack limit stops it at a fixed depth with
            // a typed `StackOverflow` error (classified as budget below).
            if let Err(e) = eval.set_max_callstack_size(limits.max_call_depth.max(1)) {
                return RuleRun::failed(runtime(rule, e));
            }
            eval.extra = Some(&store);

            let ast = match AstModule::parse(&rule.id, rule.source.clone(), &rule_dialect()) {
                Ok(ast) => ast,
                Err(e) => {
                    return RuleRun::failed(EvalError::Parse {
                        rule_id: rule.id.clone(),
                        reason: e.to_string(),
                    });
                }
            };

            let mut aborted = false;
            let mut depth_overflow = false;
            let mut fault: Option<String> = None;
            let mut missing: Option<Option<&'static str>> = None;
            match eval.eval_module(ast, globals) {
                Ok(_) => match module.get(entry.name()) {
                    Some(hook) => {
                        if let Err(e) = eval.eval_function(hook, &[arg_value], &[]) {
                            aborted = true;
                            depth_overflow = is_depth_overflow(&e);
                            fault = Some(e.to_string());
                        }
                    }
                    // No entry for this plane — typed, with the wrong-plane
                    // diagnosis when the OTHER plane's entry is what exists.
                    None => missing = Some(module.get(entry.other()).map(|_| entry.other())),
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
            let effects = store.effects.take();

            let over_budget = used_steps > limits.fuel || used_mem > limits.mem;
            let outcome = if let Some(wrong_plane) = missing {
                Err(EvalError::MissingEntry {
                    rule_id: rule.id.clone(),
                    expected: entry.name(),
                    wrong_plane,
                })
            } else if aborted {
                // The eval errored: a resource limit (⇒ budget) or a genuine
                // fault? The coarse tick/heap guards are visible in the exact
                // accounting; the callstack-depth guard surfaces as a typed
                // `StackOverflow` — either is a terminated bomb, not a rule fault.
                if over_budget || depth_overflow {
                    Err(budget(limits))
                } else {
                    Err(EvalError::Runtime {
                        rule_id: rule.id.clone(),
                        reason: fault.unwrap_or_default(),
                    })
                }
            } else if over_budget {
                // A non-looping allocation can complete WITHOUT aborting yet
                // still overrun the exact mem bound — refuse it as budget.
                Err(budget(limits))
            } else {
                Ok(effects)
            };
            RuleRun {
                fuel_used: used_steps,
                mem_used: used_mem,
                outcome,
            }
        })
    }));

    match evaluated {
        Ok(run) => run,
        // The only panic reachable inside the metered Starlark eval is a
        // resource-overflow assert (a multi-GiB allocation tripping a `len
        // overflow`): a terminated bomb, reported as budget — never a crash.
        // It hit the fuel/mem ceiling, so the profile records the ceiling.
        Err(_panic) => RuleRun {
            fuel_used: limits.fuel,
            mem_used: limits.mem,
            outcome: Err(budget(limits)),
        },
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
        reason: e.to_string(),
    }
}

/// Allocate the injected `event` value: a struct with the 0003 §3 payload
/// fields. Lists inject as Starlark lists of strings.
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

/// Allocate one `{kind, key, old, new, hpath}` change fact. `old`/`new` are
/// `None` when the value did not exist on that side — absence is a fact a
/// predicate must be able to see, so it is `None` and never an empty string.
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

/// Allocate the injected `event.facts`: `{path, fm}` and NOTHING else.
///
/// `fm` is a dict so a predicate reads `event.facts.fm.get("reviewer")`. The
/// absences here are the law, not an omission: no actor, no session, no
/// invocation identity reaches this surface, so an actor-fact WHEN clause is a
/// `NameError` rather than a policy anyone has to remember to enforce.
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

/// Allocate the injected `ctx` value: the run-plane facts a task sees —
/// `page`, `task`, `args` (list of strings), `env` (dict of strings). The
/// provenance facts (`invocation_id`, `root_at_eval`) are deliberately NOT
/// injected (see [`crate::RunCtx`]): a task's output must not depend on its
/// invocation identity.
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

    /// The `EffectKind` ↔ globals tie. A consumer building a capability ceiling
    /// resolves declared caps to constructor names through
    /// [`EffectKind::constructor`]; if that mapping drifted from what this kernel
    /// registers, the ceiling would allow a name no rule can call, or refuse one it
    /// can. Both directions are pinned: every kind's constructor is registered, and
    /// every registered constructor belongs to a kind.
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

        // The other direction: the standard library is not a capability, so the
        // registered set MINUS the standard globals must be exactly the derived
        // set PLUS the reaction vocabulary. `intent` and `receipt_addr` are
        // builtins that grant no kind, so they belong to neither the standard
        // library nor `EffectKind` — both sides read `REACTION_VOCAB`, so this
        // cannot be satisfied by hand-listing the names in two places.
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

    /// `receipt_addr` is a PURE function of `(path, rev)` — the property the
    /// armed-not-delivered mechanism rests on, since an intent's address must be
    /// re-derivable to find out whether its outcome ever landed.
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

    /// A path carrying `#` cannot be made into an unambiguous `path#^anchor`
    /// address, so `receipt_addr` refuses it rather than minting one nobody can
    /// split back apart.
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

    /// `action` resolves the wire identities plus the ONE documented alias, and
    /// nothing else — an unknown action is a fault, never a guess.
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

    /// The closed capability surface: every effect constructor is present, and no
    /// filesystem / network / eval name is — the purity assertion at the source of
    /// truth (the globals themselves), not merely behaviorally.
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
        // rejected — for BOTH the bracket counter and the unary-run counter.
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
        // A string literal must be skipped EXACTLY — real nesting AFTER it still
        // counts. If skip_string ran to end-of-input, the deep brackets below would
        // be swallowed and wrongly admitted.
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
        // Many brackets, all depth 1 — must NOT trip (bounds depth, not count).
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
        // A single 2000-char identifier of all `n`s. The `not` word-boundary guard
        // must keep this ONE operand (unary run 0), not 2000 phantom `not`s — a
        // regression guard on the guard's boundary check.
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
