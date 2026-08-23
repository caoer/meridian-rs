//! Pure Starlark effect kernel: [`Rule`]s + [`ChangeEvent`] → deterministic
//! `Vec<`[`Effect`]`>` descriptors. Zero I/O, zero integration; advisory-only
//! (0003). Fuel-limited, depth-capped, panic-safe.
//!
//! **Owns:** evaluate fenced Starlark over a semantic change event. Rule surface
//! is the descriptor constructors in `kernel` plus the Starlark stdlib — no
//! file, net, clock, or os.
//!
//! **Never does:** disk, sockets, apply effects, watch trees, daemon/wire I/O.
//! Effects are inert data a consumer executes. No correctness path; does not
//! touch the resident-daemon spine.
//!
//! # Advisory law (0003 §6)
//! Effects are latency/UX, never correctness. Undelivered effect = lost latency,
//! not lost truth. Disk and fingerprints are correctness; nothing here mutates
//! disk.
//!
//! # Cursor-replay limitation (0003 §4)
//! At-least-once delivery is fingerprint-cursor replay, not a queue: reconnect
//! re-emits `diff(cursor, live)`. Intermediate transitions collapse
//! (`todo→review→done` → `todo→done`) by design. Escape: write history into the
//! tree at write time (`append_section` / `md.*`) — disk holds history, wire does
//! not.

use std::collections::{BTreeMap, BTreeSet};

use wire::PlanEdit;

use serde::{Deserialize, Serialize};

pub mod digest;
mod kernel;
mod script_edit;
pub mod trace;

pub use kernel::validate;
pub use script_edit::{ArmedEdit, hpath_addresses, segs_address, sel_addresses};

/// One rule's metered outcome: typed result plus exact fuel (Starlark ticks)
/// and peak-heap bytes. Never-reached eval → `0`/`0` + authoring fault; bomb
/// at ceiling → reports that ceiling.
#[derive(Debug, Clone)]
pub struct RuleTelemetry {
    /// The rule this run measured — its `id`.
    pub rule_id: String,
    /// Exact Starlark ticks spent (`0` if eval was never reached).
    pub fuel_used: u64,
    /// Peak eval-heap bytes (`0` if eval was never reached).
    pub mem_used: u64,
    /// The rule's typed result: the effects it emitted, or why it faulted.
    pub outcome: Result<Vec<Effect>, EvalError>,
}

/// Evaluate each rule independently over one `event` under `limits`, returning
/// metered outcomes in slice order. Unlike [`eval_with_limits`], never aborts
/// the batch on the first fault — every rule reports its own outcome + fuel/mem
/// (corpus-replay contract). Pure function of `(rule, event, limits)`.
/// Cascade depth cap matches [`eval_with_limits`]: at/beyond `max_depth`,
/// cascading `md.*` effects are suppressed.
#[must_use]
pub fn eval_telemetry(
    rules: &[Rule],
    event: &ChangeEvent,
    limits: EvalLimits,
) -> Vec<RuleTelemetry> {
    kernel::on_eval_stack(|| {
        let globals = kernel::effect_globals();
        rules
            .iter()
            .map(|rule| {
                let run = kernel::run_rule_metered(&globals, rule, event, limits);
                let outcome = run.outcome.map(|mut effects| {
                    if event.depth >= limits.max_depth {
                        effects.retain(|e| e.kind.domain() != Domain::Md);
                    }
                    effects
                });
                RuleTelemetry {
                    rule_id: rule.id.clone(),
                    fuel_used: run.fuel_used,
                    mem_used: run.mem_used,
                    outcome,
                }
            })
            .collect()
    })
}

/// The executor domain that applies an effect (0003 §5). New domains are new
/// consumers with zero core changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Domain {
    /// The engine applies it to the markdown tree, actor `rule:<id>`,
    /// depth-capped (these are the cascading effects).
    Md,
    /// Resident daemon powers (view refresh, schedule, watch).
    Daemon,
    /// Wire clients — advisory feedback delivered to agents.
    Proto,
}

/// Namespaced effect-descriptor kind. Consumers route by capability; the string
/// form (`md.set_field`, `proto.send`, …) is the wire/snapshot identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EffectKind {
    /// `md.set_field` — set a frontmatter field. Authorizes under the
    /// `md.edit` cap verb (caps-redesign ruling 2026-08-19: the cap plane
    /// speaks create/edit/delete; the descriptor plane stays per-op).
    SetField,
    /// `md.append_section` — append content to a section. Authorizes under
    /// the `md.edit` cap verb (caps-redesign ruling 2026-08-19).
    AppendSection,
    /// `md.create` — birth a file through the create door (the declared-task
    /// birth cap, SCHEMA §5 verb-fold map / create-task-page ruling
    /// 2026-08-18: option 1, engine birth cap). Realized by the run executor
    /// via `wire_serve::write::create` — occupied-path refusal, armed
    /// middleware stamps, and checks are the DOOR's, never re-implemented.
    /// Args: `path` (the declared RELATIVE landing coordinate — the string
    /// the `md.create` cap glob judges), `body`, optional `base` (the
    /// resolution base — ZT ruling 2026-08-19 #2: targeting is data, carried
    /// beside the path, never pre-joined into it), optional `message`.
    Create,
    /// `daemon.refresh_view` — mark a resident view stale.
    RefreshView,
    /// `proto.send` — deliver a message to agent target(s).
    Send,
    /// `proto.remind` — schedule an advisory reminder.
    Remind,
    /// `proto.ask` — pose a question back to the writer.
    Ask,
    /// `proto.notice` — a low-severity advisory notice.
    Notice,
}

impl EffectKind {
    /// Every descriptor kind, stable order — closed surface source of truth.
    pub const ALL: [EffectKind; 8] = [
        EffectKind::SetField,
        EffectKind::AppendSection,
        EffectKind::Create,
        EffectKind::RefreshView,
        EffectKind::Send,
        EffectKind::Remind,
        EffectKind::Ask,
        EffectKind::Notice,
    ];

    /// The namespaced wire / snapshot identity.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            EffectKind::SetField => "md.set_field",
            EffectKind::AppendSection => "md.append_section",
            EffectKind::Create => "md.create",
            EffectKind::RefreshView => "daemon.refresh_view",
            EffectKind::Send => "proto.send",
            EffectKind::Remind => "proto.remind",
            EffectKind::Ask => "proto.ask",
            EffectKind::Notice => "proto.notice",
        }
    }

    /// The executor domain (the namespace before the `.`).
    #[must_use]
    pub fn domain(self) -> Domain {
        match self {
            EffectKind::SetField | EffectKind::AppendSection | EffectKind::Create => Domain::Md,
            EffectKind::RefreshView => Domain::Daemon,
            EffectKind::Send | EffectKind::Remind | EffectKind::Ask | EffectKind::Notice => {
                Domain::Proto
            }
        }
    }

    /// Starlark constructor name (`proto.send` → `send`). Load-time capability
    /// ceilings must use this mapping (pinned by
    /// `every_effect_kind_constructor_is_registered`) — never re-derive by
    /// splitting [`EffectKind::as_str`].
    #[must_use]
    pub fn constructor(self) -> &'static str {
        match self {
            EffectKind::SetField => "set_field",
            EffectKind::AppendSection => "append_section",
            EffectKind::Create => "create",
            EffectKind::RefreshView => "refresh_view",
            EffectKind::Send => "send",
            EffectKind::Remind => "remind",
            EffectKind::Ask => "ask",
            EffectKind::Notice => "notice",
        }
    }

    /// Wire identity → kind (`"proto.send"` → [`EffectKind::Send`]), or `None`
    /// if unknown — closed surface; never guess.
    #[must_use]
    pub fn from_wire_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == name)
    }
}

/// Reaction-plane builtins that are not capabilities and grant no effect kind.
/// Capability ceilings must admit these unconditionally (like the Starlark
/// stdlib): `intent` emits a descriptor the caps still gate; `receipt_addr`
/// computes an address. Single source pinned by
/// `every_effect_kind_constructor_is_registered`.
pub const REACTION_VOCAB: [&str; 2] = ["intent", "receipt_addr"];

/// `intent(action = …)` → kind: wire identity (`"proto.send"`) or the one alias
/// `"notify"` ≡ `proto.send`. Unknown action is a fault, never a guess.
#[must_use]
pub fn action_kind(action: &str) -> Option<EffectKind> {
    match action {
        "notify" => Some(EffectKind::Send),
        other => EffectKind::from_wire_name(other),
    }
}

/// Receipt address for `(path, rev)` as `path#^anchor` (§6.1). Pure in inputs
/// (no clock/counter): same change re-eval → same address. Anchor is `r-` +
/// first 16 hex of `blake3(path \0 rev)` (`[A-Za-z0-9-]`); `\0` separator so
/// `("a","bc")` ≠ `("ab","c")`.
#[must_use]
pub fn receipt_address(path: &str, rev: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(path.as_bytes());
    hasher.update(b"\0");
    hasher.update(rev.as_bytes());
    let digest = hasher.finalize().to_hex();
    let anchor = &digest.as_str()[..16];
    format!("{path}#^r-{anchor}")
}

impl Serialize for EffectKind {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

/// Descriptor argument: scalar string, list of strings, or a ONE-LEVEL map of
/// those two (`create(props = {…})` — D6). Closed shape — no numbers, no
/// deeper nesting; inert data throughout. The map arm exists because a
/// frontmatter block IS a map: flattening it into `props.<key>` args would
/// make every reader re-parse a name, and the door that serializes it needs
/// the keys and the values apart to quote them (`yaml_safe_value` /
/// `yaml_safe_flow`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ArgValue {
    /// A scalar string argument.
    Str(String),
    /// A list-of-strings argument (e.g. `send(to = [...])`).
    List(Vec<String>),
    /// A map argument whose values are scalars or lists — one level, never a
    /// map inside a map (the constructor that builds it refuses deeper).
    Map(BTreeMap<String, ArgValue>),
}

/// Typed provenance of an effect. Planes carry different facts and cannot
/// impersonate each other: change-plane carries diff fingerprints (cursor
/// coords); run-plane carries invocation id + root at eval. Serialized with an
/// explicit `plane` tag — never overload fingerprint fields with run values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "plane", rename_all = "lowercase")]
pub enum Provenance {
    /// Change-plane: `on_change` over a semantic diff (0003 §3).
    Change {
        /// Pre-change fingerprint.
        fingerprint_before: String,
        /// Post-change fingerprint — the cursor coordinate.
        fingerprint_after: String,
    },
    /// Run-plane: explicit task invocation. No diff/cursor — re-run is new
    /// intent, never a replay.
    Run {
        /// Caller-supplied invocation id (engine mints none).
        invocation_id: String,
        /// The workspace root fingerprint this invocation observed. Stamped
        /// verbatim from [`RunCtx`]; a caller that folds lazily writes it
        /// here after the eval instead (see that type).
        root_at_eval: String,
    },
}

/// One effect descriptor — inert data a consumer executes: routing `kind`, flat
/// `args`, emitting `rule_id`, plane-typed [`Provenance`], per-rule `seq`,
/// cascade `depth`. Dedup key `(rule_id, fingerprint_after, seq)` (0003 §4) is
/// change-plane only — see [`Effect::idempotency_key`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Effect {
    /// The routing kind (`md.set_field`, `proto.send`, …).
    pub kind: EffectKind,
    /// The rule that emitted this descriptor.
    pub rule_id: String,
    /// Emission index within the emitting rule for this event (0-based).
    pub seq: u32,
    /// The cascade depth of the event this was produced from.
    pub depth: u32,
    /// The plane this descriptor was produced on, with that plane's facts.
    #[serde(flatten)]
    pub provenance: Provenance,
    /// Flat, canonical (sorted-key) descriptor arguments.
    pub args: BTreeMap<String, ArgValue>,
}

impl Effect {
    /// Dedup key `(rule_id, fingerprint_after, seq)` (0003 §4) — `Some` only on
    /// the change plane (stable for cursor replay). Run-plane has none: re-run
    /// is new intent; suppressing it would drop requested work.
    #[must_use]
    pub fn idempotency_key(&self) -> Option<IdempotencyKey> {
        match &self.provenance {
            Provenance::Change {
                fingerprint_after, ..
            } => Some(IdempotencyKey {
                rule_id: self.rule_id.clone(),
                fingerprint_after: fingerprint_after.clone(),
                seq: self.seq,
            }),
            Provenance::Run { .. } => None,
        }
    }
}

/// Executor-dedup key of a change-plane [`Effect`] (0003 §4). Run-plane effects
/// have none (see [`Effect::idempotency_key`]).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct IdempotencyKey {
    /// The emitting rule.
    pub rule_id: String,
    /// The post-change fingerprint (the cursor coordinate).
    pub fingerprint_after: String,
    /// The per-rule emission index.
    pub seq: u32,
}

/// What one [`ChangeFact`] is about — closed two-value vocabulary, not a free
/// string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeFactKind {
    /// A frontmatter key whose value changed. Carries `key`, `old`, `new`.
    Frontmatter,
    /// A section whose content changed. Carries `hpath`.
    Section,
}

impl ChangeFactKind {
    /// The string a predicate compares against.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            ChangeFactKind::Frontmatter => "frontmatter",
            ChangeFactKind::Section => "section",
        }
    }
}

/// One change with values — what `fields_changed` (names only) cannot express.
/// Frontmatter: `key` + `old`/`new` (`None` = absence). Section: `hpath` only
/// — **no body text** (bodies are unbounded; content is read by consumers).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChangeFact {
    /// Whether this is a frontmatter key or a section.
    pub kind: ChangeFactKind,
    /// The frontmatter key. Empty for a section fact.
    pub key: String,
    /// The value before, when there was one. Always `None` for a section fact.
    pub old: Option<String>,
    /// The value after, when there is one. Always `None` for a section fact.
    pub new: Option<String>,
    /// The section's heading path. Empty for a frontmatter fact.
    pub hpath: Vec<String>,
}

/// World-model facts a reaction may read: the changed document as it now stands.
/// Deliberately absent: actor, session, invocation identity — the engine must
/// not observe the observer; actor-fact WHEN clauses are refused.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct EventFacts {
    /// The document's path.
    pub path: String,
    /// Its frontmatter properties in document order, AFTER the change.
    pub frontmatter: Vec<(String, String)>,
}

/// Semantic change event — one `on_change` payload (0003 §3). Rules filter on
/// sections/fields, not "file changed"; single hook (no `on_section_change`).
/// `fields_changed` = which keys; `changes` = from/to values; `facts` = document
/// as it now stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChangeEvent {
    /// The changed file's workspace path.
    pub file: String,
    /// Section paths whose content changed (sections-as-files addressing).
    pub sections_changed: Vec<String>,
    /// Frontmatter field names whose value changed.
    pub fields_changed: Vec<String>,
    /// What changed, with values — frontmatter keys and sections.
    pub changes: Vec<ChangeFact>,
    /// The changed document's facts as it now stands.
    pub facts: EventFacts,
    /// The pre-change fingerprint.
    pub fingerprint_before: String,
    /// The post-change fingerprint.
    pub fingerprint_after: String,
    /// Cascade depth: `0` for a user-originated change, `n` for the `n`-th
    /// generation produced by applying an `md.*` effect (0003 §5 depth cap).
    pub depth: u32,
}

impl ChangeEvent {
    /// Depth-0 (user-originated) event.
    #[must_use]
    pub fn new(
        file: impl Into<String>,
        fingerprint_before: impl Into<String>,
        fingerprint_after: impl Into<String>,
    ) -> Self {
        Self {
            file: file.into(),
            sections_changed: Vec::new(),
            fields_changed: Vec::new(),
            changes: Vec::new(),
            facts: EventFacts::default(),
            fingerprint_before: fingerprint_before.into(),
            fingerprint_after: fingerprint_after.into(),
            depth: 0,
        }
    }
}

/// Run-plane context — one `run(ctx)` invocation's inert facts, all
/// caller-supplied (§9: engine invents no identity/time). Sandbox sees
/// `page`/`task`/`args`/`env` only. `invocation_id`/`root_at_eval` stamp
/// [`Provenance::Run`] but are not injected — task output must not depend on
/// invocation identity (same inputs → byte-identical payloads).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunCtx {
    /// The addressed page's workspace path.
    pub page: String,
    /// The task name (also the emitting `rule_id` on this plane).
    pub task: String,
    /// Positional args, contract-validated by the caller before eval.
    pub args: Vec<String>,
    /// Declared env, contract-validated by the caller before eval.
    pub env: BTreeMap<String, String>,
    /// The caller-supplied invocation id (Run-plane provenance).
    pub invocation_id: String,
    /// The workspace root fingerprint this invocation observed (Run-plane
    /// provenance), stamped onto every emitted effect.
    ///
    /// Supplied at eval time when the caller already holds a fold. A caller
    /// that folds LAZILY — the starlark run leg, behind every caller of
    /// `run::runner::run` / `rehearse` (the `mrd run` CLI, the wire `run` op,
    /// `realise`), which folds only when THAT TENSE'S output will show the
    /// token (`docs/run-plane.md` § The run plane) — passes it empty and
    /// rewrites the emitted effects' provenance afterwards, or leaves it
    /// empty when no reader exists. Where a fold happens, both spellings name
    /// the same domain: the entry is hermetic by construction, so an eval
    /// cannot move the corpus under itself, and the sandbox cannot read this
    /// field (above), so no output can depend on which spelling ran.
    pub root_at_eval: String,
}

/// A rule: caller-assigned `id` (stamped on every emitted effect) + Starlark
/// `source` (`def on_change(event):` or `def run(ctx):`). Loading files is the
/// caller's job — this crate takes source only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    /// Rule provenance — the effect `rule_id`. Also the idempotency-key prefix.
    pub id: String,
    /// The Starlark source defining `on_change(event)`.
    pub source: String,
}

impl Rule {
    /// Build a rule from its id and source.
    #[must_use]
    pub fn new(id: impl Into<String>, source: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            source: source.into(),
        }
    }
}

/// Effect kinds a consumer declared executable (0003 §2). Engine emits all
/// effects deterministically; this is the downstream routing filter — kept
/// separate so eval stays pure in `(rules, event)`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapabilitySet(BTreeSet<EffectKind>);

impl CapabilitySet {
    /// An empty set — executes nothing.
    #[must_use]
    pub fn none() -> Self {
        Self(BTreeSet::new())
    }

    /// Every kind — a fully-capable executor.
    #[must_use]
    pub fn all() -> Self {
        Self(EffectKind::ALL.into_iter().collect())
    }

    /// Add one kind (builder-style).
    #[must_use]
    pub fn with(mut self, kind: EffectKind) -> Self {
        self.0.insert(kind);
        self
    }

    /// Add every kind in a domain (builder-style).
    #[must_use]
    pub fn with_domain(mut self, domain: Domain) -> Self {
        self.0
            .extend(EffectKind::ALL.into_iter().filter(|k| k.domain() == domain));
        self
    }

    /// Whether this consumer can execute the effect's kind.
    #[must_use]
    pub fn admits(&self, effect: &Effect) -> bool {
        self.0.contains(&effect.kind)
    }

    /// Partition into `(admitted, rejected)` by capability, order preserved.
    /// Filter only — never changes which effects were produced.
    #[must_use]
    pub fn route(&self, effects: Vec<Effect>) -> (Vec<Effect>, Vec<Effect>) {
        effects.into_iter().partition(|e| self.admits(e))
    }
}

impl FromIterator<EffectKind> for CapabilitySet {
    fn from_iter<I: IntoIterator<Item = EffectKind>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}

/// Deterministic eval bounds (0003 §1). Exceeding `fuel` / `mem` /
/// `max_call_depth` → [`EvalError::Budget`] (no hang). `max_depth` suppresses
/// cascading `md.*` at/beyond the ceiling. `max_source_bytes` is the parse-DoS
/// guard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvalLimits {
    /// Max Starlark step (tick) count per rule.
    pub fuel: u64,
    /// Max eval-heap bytes per rule.
    pub mem: u64,
    /// Max Starlark call-stack depth per rule (recursion-bomb guard).
    pub max_call_depth: usize,
    /// Cascade depth cap — `md.*` effects are suppressed at/beyond this depth.
    pub max_depth: u32,
    /// Max rule source length in bytes (parse `DoS` guard).
    pub max_source_bytes: usize,
}

impl Default for EvalLimits {
    fn default() -> Self {
        Self {
            fuel: 1_000_000,
            mem: 64 * 1024 * 1024,
            max_call_depth: 1000,
            max_depth: 8,
            // Sized for the fixed eval stack (`kernel::EVAL_STACK_BYTES`);
            // raising this needs a proportionally larger stack.
            max_source_bytes: 64 * 1024,
        }
    }
}

/// Why eval produced no effects. Hostile input lands as a typed error — no
/// panic, no hang.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvalError {
    /// Per-eval `{fuel, mem}` exhausted — terminated, never hung.
    Budget {
        /// The fuel bound that was exceeded (or is the ceiling).
        fuel: u64,
        /// The mem bound (bytes).
        mem: u64,
    },
    /// The rule source would not parse as Starlark.
    Parse {
        /// The rule whose source failed.
        rule_id: String,
        /// The parser's message.
        reason: String,
    },
    /// Parsed but faulted at eval (raised error, bad arg type, unbound name, …).
    Runtime {
        /// The rule that faulted.
        rule_id: String,
        /// The fault message.
        reason: String,
        /// 1-based source line of the fault, when the evaluator attributed a
        /// span. Absence is absence, never a synthesized line 0.
        line: Option<u32>,
    },
    /// Source exceeded [`EvalLimits::max_source_bytes`] — refused before parse.
    SourceTooLarge {
        /// The offending rule.
        rule_id: String,
        /// Its source length in bytes.
        bytes: usize,
        /// The configured limit.
        limit: usize,
    },
    /// The script plane's read ceiling was reached — refused, never truncated.
    ReadBudget {
        /// The script that asked for one read too many.
        rule_id: String,
        /// The ceiling it hit (`ScriptLimits::max_reads`).
        limit: usize,
    },
    /// The effects-mode run ceiling was reached. The refusal is typed and the
    /// runs already executed STAND — a live program has no rollback; the trace
    /// records how far it got. The ceiling exists because a live `run()`'s
    /// execution is metered on the run plane's own budget, not the script
    /// clock, so without a count ceiling a run loop would be bounded by
    /// nothing.
    RunBudget {
        /// The script that asked for one run too many.
        rule_id: String,
        /// The ceiling it hit (`ScriptLimits::max_runs`).
        limit: usize,
    },
    /// The armed-edit ceiling was reached — refused, never truncated.
    ArmedBudget {
        /// The script that armed one edit too many.
        rule_id: String,
        /// 1-based source line of the refused `put()`.
        line: u32,
        /// The ceiling it hit (`ScriptLimits::max_armed_edits`).
        limit: usize,
    },
    /// No hook for the addressed plane, or the other plane's hook is present.
    /// One hook per hooked plane, and they never cross — the script entry has
    /// no hook at all (its module top level IS the program), so it never
    /// raises this.
    MissingEntry {
        /// The offending rule/task.
        rule_id: String,
        /// The entry this eval required (`run` or `on_change`).
        expected: &'static str,
        /// The other plane's entry, when that is what the source defines.
        wrong_plane: Option<&'static str>,
    },
}

impl std::fmt::Display for EvalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvalError::Budget { fuel, mem } => write!(
                f,
                "budget exhausted (steps>{fuel} or mem>{mem} bytes) — evaluation terminated"
            ),
            EvalError::Parse { rule_id, reason } => {
                write!(f, "rule '{rule_id}' starlark parse error: {reason}")
            }
            EvalError::Runtime {
                rule_id, reason, ..
            } => {
                write!(f, "rule '{rule_id}' evaluation error: {reason}")
            }
            EvalError::SourceTooLarge {
                rule_id,
                bytes,
                limit,
            } => write!(
                f,
                "rule '{rule_id}' source is {bytes} bytes, over the {limit}-byte parse limit"
            ),
            EvalError::ReadBudget { rule_id, limit } => write!(
                f,
                "script '{rule_id}' exceeded the read budget of {limit} reads per attempt — \
                 refused, never truncated\n  → the ceiling is per attempt, not per corpus: \
                 when your set is larger, run one attempt per slice — files[0:{limit}] first, \
                 files[{limit}:{}] next, until done",
                limit * 2
            ),
            EvalError::RunBudget { rule_id, limit } => write!(
                f,
                "script '{rule_id}' exceeded the run budget of {limit} runs per attempt — \
                 the runs already executed stand; a live program has no rollback\n  → continue \
                 in a fresh attempt starting at the first run this one never made — the {limit} \
                 that ran are done, and re-running them would run them twice"
            ),
            EvalError::ArmedBudget { line, limit, .. } => write!(
                f,
                "refused at line {line} · budget — the armed-edit ceiling of {limit} edits \
                 per attempt was reached; refused, never truncated\n  → the ceiling is per \
                 attempt, not per corpus: when your edit set is larger, slice the targets and \
                 run one attempt per slice of at most {limit} edits"
            ),
            EvalError::MissingEntry {
                rule_id,
                expected,
                wrong_plane: Some(found),
            } => write!(
                f,
                "'{rule_id}' defines `{found}` — wrong plane: this eval requires `{expected}`"
            ),
            EvalError::MissingEntry {
                rule_id, expected, ..
            } => write!(f, "'{rule_id}' defines no `{expected}` entry"),
        }
    }
}

impl std::error::Error for EvalError {}

/// Evaluate `rules` over one `event` under [`EvalLimits::default`]. See
/// [`eval_with_limits`].
///
/// # Errors
/// First failing rule's typed [`EvalError`].
pub fn eval(rules: &[Rule], event: &ChangeEvent) -> Result<Vec<Effect>, EvalError> {
    eval_with_limits(rules, event, EvalLimits::default())
}

/// Evaluate `rules` over one `event` under explicit `limits`.
///
/// Determinism: rules in slice order; per-rule `seq` in execution order. Pure
/// function of `(rules, event, limits)` — no clock/random/consumer observation;
/// cursor replay (0003 §4) is byte-identical.
///
/// Depth cap: at `event.depth >= limits.max_depth`, cascading `md.*` are
/// suppressed; terminal `daemon.*`/`proto.*` still emit. Rule still runs and
/// spends fuel.
///
/// # Errors
/// First offending rule: [`EvalError::Parse`], [`EvalError::SourceTooLarge`],
/// [`EvalError::Runtime`], or [`EvalError::Budget`]. Load-gate with [`validate`]
/// to separate authoring from per-event faults.
pub fn eval_with_limits(
    rules: &[Rule],
    event: &ChangeEvent,
    limits: EvalLimits,
) -> Result<Vec<Effect>, EvalError> {
    // Large-stack thread: pathologically nested source must not overflow the
    // native stack (uncatchable abort; issue #66). No effect on determinism.
    kernel::on_eval_stack(|| {
        let globals = kernel::effect_globals();
        let mut out = Vec::new();
        for rule in rules {
            out.extend(kernel::run_rule(&globals, rule, event, limits)?);
        }
        // Depth cap (0003 §5): withhold cascading md.* past the ceiling.
        if event.depth >= limits.max_depth {
            out.retain(|e| e.kind.domain() != Domain::Md);
        }
        Ok(out)
    })
}

/// Evaluate one task's `run(ctx)` under `limits` in the same sealed sandbox as
/// `on_change` (same globals, fuel/mem/depth metering, panic containment). No
/// process spawn, clock, or I/O.
///
/// Effects carry [`Provenance::Run`], `rule_id = ctx.task`, `depth = 0`, no
/// idempotency key (re-run is new intent). Pure in `(task, ctx, limits)`;
/// provenance is stamped not injected, so payload depends only on
/// `(page, task, args, env)`.
///
/// # Errors
/// Same surface as [`eval_with_limits`], plus [`EvalError::MissingEntry`] when
/// no `run(ctx)` (wrong-plane detail if `on_change` is defined instead).
pub fn eval_run(task: &Rule, ctx: &RunCtx, limits: EvalLimits) -> Result<Vec<Effect>, EvalError> {
    // One identity per invocation: task.id (errors) must equal ctx.task
    // (effects). Divergence refused, never reconciled.
    if task.id != ctx.task {
        return Err(EvalError::Runtime {
            rule_id: task.id.clone(),
            reason: format!(
                "task id '{}' != ctx.task '{}' — one identity per invocation",
                task.id, ctx.task
            ),
            line: None,
        });
    }
    kernel::on_eval_stack(|| {
        let globals = kernel::effect_globals();
        kernel::run_task(&globals, task, ctx, limits)
    })
}

// ---------------------------------------------------------------------------
// The script entry (kernel entry #3) — `docs/run-plane.md` § The script entry.
// ---------------------------------------------------------------------------

/// Inert script-plane inputs (the [`RunCtx`] precedent): all caller-supplied,
/// none minted here. `files` carries **paths only, in call order** —
/// `files[i]` is the i-th path the caller named (order-bind ruling; patterns
/// expand in place upstream). Content enters only through `read()`, so it is
/// recorded and replay stays byte-identical. There is no `glob()` builtin.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScriptCtx {
    /// Script identity, used for error provenance only.
    pub id: String,
    /// The caller's arguments, injected as the inert `args` binding — a **dict**
    /// (`RunCtx::env`'s shape), because a caller names its inputs rather than
    /// counting them. Inert: string keys, string values, no callables, no host
    /// reach.
    pub args: BTreeMap<String, String>,
    /// Host-enumerated paths, injected as the inert `files` binding.
    pub files: Vec<String>,
    /// Effects mode (script-effects ruling, 2026-08-13): the effect builtins
    /// the program may use. Empty = the pure model, today's law word for
    /// word. Non-empty = the LIVE program model — `put()` dispatches to the
    /// host and applies NOW; the named builtins (closed set: `run`,
    /// `token_count`) join the globals. The kernel validates nothing here
    /// beyond membership — the decode wall upstream owns the combination laws.
    pub effects: Vec<String>,
}

/// Script-plane budgets: [`EvalLimits`] verbatim plus the read ceiling — the
/// I/O-amplification axis the other two entries do not have (they read nothing).
/// Every ceiling is a typed refusal, never silent truncation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScriptLimits {
    /// Fuel / mem / call depth / source bytes, shared with the other entries.
    pub eval: EvalLimits,
    /// Max host reads per attempt. Exceeding it → [`EvalError::ReadBudget`].
    pub max_reads: usize,
    /// Max armed edits per attempt. Exceeding it → [`EvalError::ArmedBudget`].
    /// The value is the host's (`run-plane.md` § The script entry lists it among
    /// the budget overrides); the kernel enforces it at arm time, because a
    /// ceiling counted after eval could only truncate, and truncation is what
    /// this plane refuses.
    pub max_armed_edits: usize,
    /// Max live `run()` calls per attempt (effects mode only — the pure lane
    /// never holds the builtin). Exceeding it → [`EvalError::RunBudget`].
    /// The count ceiling that keeps a run loop bounded now that a run's own
    /// execution is metered on the run plane's budget, not the script clock.
    pub max_runs: usize,
}

impl Default for ScriptLimits {
    fn default() -> Self {
        Self {
            eval: EvalLimits::default(),
            max_reads: 64,
            max_armed_edits: 64,
            max_runs: 64,
        }
    }
}

/// The one effectful seam of the script entry. Two production implementors —
/// the wire client (reads lower to `toc`/`cat` through the one door, live) and
/// the § A.7 in-process entry-world host (reads serve from the pinned entry
/// state plus the program's own armed overlay); tests and replay implement it
/// too, which is what makes the replay law testable.
///
/// The kernel calls `actor` once per attempt, before eval — identity is
/// threaded (§9), never minted.
///
/// `Send` because eval runs on the kernel's own large-stack thread (the
/// parse-recursion guard); the host is moved onto it and back, never shared.
pub trait ScriptHost: Send {
    /// The §4.1 toc face for `path`.
    ///
    /// `armed` is the program's OWN armed list so far, in arm order — the
    /// read-your-own-writes seam (run-plane § the entry world, 2026-08-12).
    /// It rides the READ call, never the arm, so `put()` stays pure. A host
    /// serving live reads (the wire client) ignores it; the entry-world host
    /// composes the overlay from it; replay ignores it too, because the
    /// recording already carries the served face.
    ///
    /// # Errors
    /// A [`ReadFault`] when the host cannot serve the page; the script aborts.
    fn toc(&mut self, path: &str, armed: &[ArmedEdit]) -> Result<TocFacts, ReadFault>;

    /// The §4.2 cat face for one `section` of `path`. There is no whole-file
    /// body: read one section, not the disk. `armed` as on [`ScriptHost::toc`].
    ///
    /// `section` arrives as the parsed selector: the one human-string→selector
    /// door ([`wire::ReadSel::parse`]) sits at the kernel's `section=` boundary
    /// — never inward of it — so a host serves segments and cannot re-split a
    /// heading whose raw text carries `/`.
    ///
    /// # Errors
    /// A [`ReadFault`] when the host cannot serve the section; the script aborts.
    fn cat(
        &mut self,
        path: &str,
        section: &wire::ReadSel,
        armed: &[ArmedEdit],
    ) -> Result<SecFacts, ReadFault>;

    /// The caller's own identity, returned by the `me()` builtin.
    fn actor(&self) -> &str;

    /// Effects mode only: apply one `put()` NOW — no rev, no snapshot, no CAS
    /// (script-effects ruling; the write flock and structural validation are
    /// the host's own). The default refuses: a lane that has not built the
    /// live model cannot half-serve it.
    ///
    /// # Errors
    /// [`EffectFault`] — the host's typed reason; the script aborts.
    fn put_live(&mut self, path: &str, items: Vec<PlanEdit>, line: u32) -> Result<(), EffectFault> {
        let _ = (path, items, line);
        Err(EffectFault {
            reason: "live effects are not supported on this lane".to_owned(),
        })
    }

    /// Effects mode only: execute one `run()` NOW through the run plane and
    /// answer its § A.8 row as a JSON value — observable results,
    /// run-then-decide. Plane refusals RETURN as rows (branchable); only a
    /// host that cannot run at all errors here. The default refuses.
    ///
    /// # Errors
    /// [`EffectFault`] — the host's typed reason; the script aborts.
    fn run_live(
        &mut self,
        page: &str,
        task: Option<&str>,
        args: Vec<String>,
        env: BTreeMap<String, String>,
        dry: bool,
        line: u32,
    ) -> Result<serde_json::Value, EffectFault> {
        let _ = (page, task, args, env, dry, line);
        Err(EffectFault {
            reason: "live effects are not supported on this lane".to_owned(),
        })
    }

    /// Effects mode only: measure one `token_count()` NOW and answer the
    /// count. The engine holds no tokenizer and no credentials — counting is
    /// a harness `count_tokens` call, so only the lane handed a measuring
    /// endpoint (§ A.7 `token_count_endpoint`) can serve it. The default
    /// refuses "unbound": that covers the pure/entry-world host and the CLI
    /// wire client, neither of which carries an endpoint.
    ///
    /// # Errors
    /// [`EffectFault`] — the host's typed reason; the script aborts.
    fn token_count_live(&mut self, text: &str) -> Result<i64, EffectFault> {
        let _ = text;
        Err(EffectFault {
            reason: "token_count is unbound on this lane — no measuring endpoint was carried to it"
                .to_owned(),
        })
    }
}

/// A live-effect refusal from the host — the effects-mode sibling of
/// [`ReadFault`]. Aborts the script as a runtime fault naming the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectFault {
    /// The host's own words.
    pub reason: String,
}

/// The script entry's default wall clock — §5.3-class host policy with ONE
/// spelling: the CLI lane and the § A.7 in-process serve both derive their
/// deadline from here, so "7 seconds" cannot drift into two values.
pub const DEFAULT_WALL_CLOCK: std::time::Duration = std::time::Duration::from_secs(7);

/// Why a host could not serve a read. Rendered by the consumer plane; the
/// script aborts whole (nothing partial ever lands).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReadFault {
    /// The path that was asked for.
    pub path: String,
    /// The section, when the cat face was asked for.
    pub section: Option<String>,
    /// The host's reason, carried verbatim.
    pub reason: String,
}

impl std::fmt::Display for ReadFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.section {
            Some(section) => write!(
                f,
                "read({:?}, section={section:?}): {}",
                self.path, self.reason
            ),
            None => write!(f, "read({:?}): {}", self.path, self.reason),
        }
    }
}

/// The toc face (§4.1): the page rev, its frontmatter, its section map, and the
/// page's word count. A frontmatter key the page does not carry is **absent
/// from `fm`** — never a synthesized empty value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TocFacts {
    /// The page rev at read time.
    pub rev: String,
    /// Frontmatter as read; absence is absence.
    pub fm: BTreeMap<String, String>,
    /// The section map, in document order.
    pub toc: Vec<TocEntry>,
    /// The whole-file word count — the toc response's own `words_total`.
    /// `read(path)` IS the wire toc face 1:1, so this is a delivered fact the
    /// host carries; nothing here counts words (ruling 2026-08-07).
    pub words: usize,
}

/// One toc row: the section, its optional `^anchor`, and its node rev.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TocEntry {
    /// The section heading — the `/`-joined display spelling.
    pub section: String,
    /// The block anchor, when the row carries one. Absence stays absence.
    pub anchor: Option<String>,
    /// The node rev of this section.
    pub rev: String,
    /// The raw §2.1 segments behind `section` — one `{h, n?}` per heading,
    /// exactly as the wire published them. This is the feedable address: the
    /// joined `section` splits on `/`, so a heading whose raw text carries one
    /// is round-trippable only through this field (`docs/laws.md` D-1). Empty
    /// on an anchor-only row, and on rows recorded before the field existed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hpath: Vec<wire::HpathSeg>,
}

impl TocEntry {
    /// Does this row address the same node as an armed `hpath`? Segment-true
    /// when the row carries its segments; the joined-string parse otherwise
    /// (pre-field rows) — the one matcher every threading lane uses, so a
    /// licensed row cannot depend on which spelling the toc arrived in.
    #[must_use]
    pub fn addresses(&self, hpath: &[wire::HpathSeg]) -> bool {
        if self.hpath.is_empty() {
            return hpath_addresses(&self.section, hpath);
        }
        segs_address(&self.hpath, hpath)
    }
}

/// The cat face (§4.2): one section's text and its rev.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecFacts {
    /// The section text.
    pub text: String,
    /// The node rev of that section.
    pub rev: String,
}

/// Which face a recorded read carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReadFace {
    /// `read(path)` — the toc face.
    Toc(TocFacts),
    /// `read(path, section=…)` — the cat face.
    Section(SecFacts),
}

/// Where a read call sat in the source. The face renders `Echo` reads and stays
/// quiet about `Quiet` ones; both are recorded either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReadPosition {
    /// A top-level statement read — `card = read(…)` binds and echoes.
    Echo,
    /// A nested read — loop bodies, comprehensions, conditions, functions.
    Quiet,
}

/// One recorded host response, in call order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReadRecord {
    /// The path read.
    pub path: String,
    /// The parsed section selector, when the cat face was asked for — the
    /// STRUCTURE, not a spelling: a display string would collide the
    /// one-segment `A/B` with the two-segment path, and replay divergence
    /// plus rev threading both compare this field. The trace derives its
    /// display spelling from here ([`wire::ReadSel::display`]).
    pub section: Option<wire::ReadSel>,
    /// 1-based source line of the call.
    pub line: u32,
    /// Whether the call sat at module top level or nested.
    pub position: ReadPosition,
    /// The response, recorded verbatim — this is what replay serves.
    pub face: ReadFace,
}

/// One § A.7 `files[]` pattern expansion, recorded at entry: the pattern
/// member and the paths the entry world matched it to. Eval is a pure
/// function of `(script, args, files, read-responses)` — with patterns the
/// `files` binding depends on the expansion, so the rows are recorded and
/// [`replay_script`] replays them exactly as it replays reads. An empty
/// `matched` is data (the zero contributes zero paths), never silent.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExpansionRecord {
    /// The pattern member, verbatim.
    pub pattern: String,
    /// The matched workspace-relative paths, sorted.
    pub matched: Vec<String>,
}

/// Everything the host answered during one attempt. Replay is a pure function
/// of `(script, args, files, recording)`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ScriptRecording {
    /// The actor the host reported, recorded so `me()` replays too.
    pub actor: String,
    /// Every read response, in call order.
    pub reads: Vec<ReadRecord>,
    /// § A.7 pattern expansions, in member order — empty when `files[]`
    /// carried no pattern, and skipped then so pattern-less trace bytes are
    /// unchanged.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expansions: Vec<ExpansionRecord>,
    /// The `files` binding as the program saw it, in binding order — the
    /// kernel records it at entry so the trace can print one `bound` row per
    /// member (order-bind ruling: the binding is visible, never inferred).
    /// Empty when the attempt bound no files, and skipped then so file-less
    /// trace bytes are unchanged.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
}

/// Measurement of one attempt. Unconditional — reported even on failure
/// (the [`RuleTelemetry`] precedent).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptTelemetry {
    /// Exact Starlark ticks spent.
    pub fuel_used: u64,
    /// Peak eval-heap bytes.
    pub mem_used: u64,
    /// Host reads performed.
    pub reads_used: usize,
    /// Elapsed monotonic milliseconds (a duration, not a clock reading).
    pub wall_ms: u64,
}

/// What one script evaluation produced: the module's top-level bindings, in
/// name order, rendered as Starlark reprs. The inert inputs (`args`, `files`)
/// are inputs, not results, and are excluded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScriptFacts {
    /// Top-level bindings the script left behind.
    pub bindings: BTreeMap<String, String>,
}

/// One script attempt: its typed outcome, everything the host answered, and
/// unconditional telemetry.
#[derive(Debug, Clone)]
pub struct ScriptEval {
    /// The eval facts, or why the attempt refused.
    pub outcome: Result<ScriptFacts, EvalError>,
    /// The edits `put()` armed, in execution order — present whether the attempt
    /// succeeded or not, because a refused attempt still renders the arms that
    /// landed before it as `[not committed]` (golden scenario 2a). They are
    /// inert: only a successful outcome may be committed.
    pub armed: Vec<ArmedEdit>,
    /// The recorded reads — present whether the attempt succeeded or not.
    pub recording: ScriptRecording,
    /// Always present.
    pub telemetry: ScriptTelemetry,
}

impl ScriptEval {
    /// The distinct content paths the armed edits write, in arm order — the
    /// §4.4 set form's member list when it spans more than one (run-plane.md
    /// § One COMMIT per attempt); the receipt companion is not here (§6.4).
    #[must_use]
    pub fn content_paths(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for armed in &self.armed {
            if !out.contains(&armed.path) {
                out.push(armed.path.clone());
            }
        }
        out
    }
}

/// Evaluate inline `script` at kernel entry #3: **the module top level IS the
/// program** — no hook lookup, so a script with no `def` is legal and a script
/// defining `def run` simply leaves it unused.
///
/// `read()` is the one effectful builtin and reaches `host`; every response is
/// recorded. The law: eval is a pure function of `(script, args, files,
/// read-response sequence)`, so [`replay_script`] against the recording is
/// byte-identical. The change and run planes are untouched — the script
/// builtins do not join their globals.
#[must_use]
pub fn eval_script(
    script: &str,
    ctx: &ScriptCtx,
    limits: ScriptLimits,
    host: &mut dyn ScriptHost,
) -> ScriptEval {
    kernel::run_script(script, ctx, limits, host)
}

/// Re-evaluate `script` against a `recording` instead of a live host — the
/// replay law made callable. Consults nothing: the recording IS the world.
///
/// A script that asks for something the recording does not hold (a different
/// path, a different face, one read too many) refuses rather than serving a
/// substitute — a replay that forged a response would forge determinism.
#[must_use]
pub fn replay_script(
    script: &str,
    ctx: &ScriptCtx,
    limits: ScriptLimits,
    recording: &ScriptRecording,
) -> ScriptEval {
    // A live program is not a pure function of recorded inputs — its writes
    // and runs happened in the world. Refusing here keeps the replay law's
    // word exact instead of forging a world that was live (§ Effects mode).
    if !ctx.effects.is_empty() {
        return ScriptEval {
            outcome: Err(EvalError::Runtime {
                rule_id: ctx.id.clone(),
                reason: "replay refuses an effects-mode context — a live program \
                         is not a pure function of its recording"
                    .to_owned(),
                line: None,
            }),
            armed: Vec::new(),
            recording: recording.clone(),
            telemetry: ScriptTelemetry {
                fuel_used: 0,
                mem_used: 0,
                reads_used: 0,
                wall_ms: 0,
            },
        };
    }
    // Recorded § A.7 expansions replay exactly as recorded reads do: each
    // pattern member substitutes its recorded match list IN PLACE, then a
    // path already bound earlier is dropped — the one merge law (order-bind
    // ruling: first occurrence wins, never a global sort). A caller passing
    // the ORIGINAL files[] (patterns included) reconstructs the exact binding
    // the live attempt evaluated under.
    let ctx = if recording.expansions.is_empty() {
        ctx.clone()
    } else {
        let mut files: Vec<String> = Vec::new();
        for member in &ctx.files {
            match recording
                .expansions
                .iter()
                .find(|row| row.pattern == *member)
            {
                Some(row) => files.extend(row.matched.iter().cloned()),
                None => files.push(member.clone()),
            }
        }
        let mut seen = std::collections::HashSet::new();
        files.retain(|path| seen.insert(path.clone()));
        ScriptCtx {
            files,
            ..ctx.clone()
        }
    };
    let mut host = RecordedHost::new(recording);
    kernel::run_script(script, &ctx, limits, &mut host)
}

/// A [`ScriptHost`] that serves a [`ScriptRecording`] in call order. Backs
/// [`replay_script`]; also usable directly to re-run one attempt's world.
pub struct RecordedHost<'r> {
    recording: &'r ScriptRecording,
    next: usize,
}

impl<'r> RecordedHost<'r> {
    /// Serve `recording` from its first read.
    #[must_use]
    pub fn new(recording: &'r ScriptRecording) -> Self {
        Self { recording, next: 0 }
    }

    /// Take the next recorded response, refusing on divergence. Selectors
    /// compare STRUCTURALLY — two spellings of one address diverge or match
    /// exactly as the live kernel would have parsed them.
    fn next_face(
        &mut self,
        path: &str,
        section: Option<&wire::ReadSel>,
    ) -> Result<&'r ReadFace, ReadFault> {
        let fault = |reason: String| ReadFault {
            path: path.to_owned(),
            section: section.map(wire::ReadSel::display),
            reason,
        };
        let record = self
            .recording
            .reads
            .get(self.next)
            .ok_or_else(|| fault("the recording holds no further read".to_owned()))?;
        if record.path != path || record.section.as_ref() != section {
            return Err(fault(format!(
                "diverges from the recording, which holds read #{} of {:?} section={:?}",
                self.next + 1,
                record.path,
                record.section.as_ref().map(wire::ReadSel::display)
            )));
        }
        self.next += 1;
        Ok(&record.face)
    }
}

impl ScriptHost for RecordedHost<'_> {
    fn toc(&mut self, path: &str, _armed: &[ArmedEdit]) -> Result<TocFacts, ReadFault> {
        match self.next_face(path, None)? {
            ReadFace::Toc(facts) => Ok(facts.clone()),
            ReadFace::Section(_) => Err(ReadFault {
                path: path.to_owned(),
                section: None,
                reason: "the recording holds a section face here, not a toc face".to_owned(),
            }),
        }
    }

    fn cat(
        &mut self,
        path: &str,
        section: &wire::ReadSel,
        _armed: &[ArmedEdit],
    ) -> Result<SecFacts, ReadFault> {
        match self.next_face(path, Some(section))? {
            ReadFace::Section(facts) => Ok(facts.clone()),
            ReadFace::Toc(_) => Err(ReadFault {
                path: path.to_owned(),
                section: Some(section.display()),
                reason: "the recording holds a toc face here, not a section face".to_owned(),
            }),
        }
    }

    fn actor(&self) -> &str {
        &self.recording.actor
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_a_namespaced_string_matching_its_domain() {
        for kind in EffectKind::ALL {
            let s = kind.as_str();
            let (ns, _) = s.split_once('.').expect("kind string is namespaced");
            let expected = match kind.domain() {
                Domain::Md => "md",
                Domain::Daemon => "daemon",
                Domain::Proto => "proto",
            };
            assert_eq!(ns, expected, "{s} namespace vs domain");
        }
    }

    #[test]
    fn kind_strings_are_unique() {
        let mut seen = BTreeSet::new();
        for kind in EffectKind::ALL {
            assert!(
                seen.insert(kind.as_str()),
                "duplicate kind string {}",
                kind.as_str()
            );
        }
        assert_eq!(seen.len(), EffectKind::ALL.len());
    }

    fn effect(kind: EffectKind, rule_id: &str, fp_after: &str, seq: u32) -> Effect {
        Effect {
            kind,
            rule_id: rule_id.to_owned(),
            seq,
            depth: 0,
            provenance: Provenance::Change {
                fingerprint_before: "before".to_owned(),
                fingerprint_after: fp_after.to_owned(),
            },
            args: BTreeMap::new(),
        }
    }

    fn run_effect(kind: EffectKind, rule_id: &str, seq: u32) -> Effect {
        Effect {
            kind,
            rule_id: rule_id.to_owned(),
            seq,
            depth: 0,
            provenance: Provenance::Run {
                invocation_id: "inv-1".to_owned(),
                root_at_eval: "root".to_owned(),
            },
            args: BTreeMap::new(),
        }
    }

    #[test]
    fn change_plane_idempotency_key_is_rule_fingerprint_seq() {
        let e = effect(EffectKind::Send, "r1", "fpA", 3);
        let k = e
            .idempotency_key()
            .expect("change-plane effects carry a key");
        assert_eq!(k.rule_id, "r1");
        assert_eq!(k.fingerprint_after, "fpA");
        assert_eq!(k.seq, 3);
    }

    #[test]
    fn run_plane_effect_has_no_idempotency_key() {
        let e = run_effect(EffectKind::SetField, "task", 0);
        assert_eq!(
            e.idempotency_key(),
            None,
            "a re-run is new intent — never deduped"
        );
    }

    #[test]
    fn provenance_serializes_with_an_explicit_plane_tag() {
        // Plane-typed JSON: fingerprints vs invocation identity — never mixed.
        let change = serde_json::to_value(effect(EffectKind::Notice, "r", "fpA", 0)).unwrap();
        assert_eq!(change["plane"], "change");
        assert_eq!(change["fingerprint_before"], "before");
        assert_eq!(change["fingerprint_after"], "fpA");
        assert!(change.get("invocation_id").is_none());

        let run = serde_json::to_value(run_effect(EffectKind::Notice, "t", 0)).unwrap();
        assert_eq!(run["plane"], "run");
        assert_eq!(run["invocation_id"], "inv-1");
        assert_eq!(run["root_at_eval"], "root");
        assert!(run.get("fingerprint_after").is_none());
    }

    #[test]
    fn capability_admits_only_declared_kinds() {
        let caps = CapabilitySet::none().with(EffectKind::Send);
        assert!(caps.admits(&effect(EffectKind::Send, "r", "f", 0)));
        assert!(!caps.admits(&effect(EffectKind::Notice, "r", "f", 0)));
    }

    #[test]
    fn all_capability_covers_every_kind() {
        let caps = CapabilitySet::all();
        for kind in EffectKind::ALL {
            assert!(
                caps.admits(&effect(kind, "r", "f", 0)),
                "{} not admitted",
                kind.as_str()
            );
        }
    }

    #[test]
    fn with_domain_adds_exactly_that_domain() {
        let caps = CapabilitySet::none().with_domain(Domain::Md);
        for kind in EffectKind::ALL {
            assert_eq!(
                caps.admits(&effect(kind, "r", "f", 0)),
                kind.domain() == Domain::Md,
                "{}",
                kind.as_str()
            );
        }
    }

    #[test]
    fn errors_display_their_provenance() {
        let parse = EvalError::Parse {
            rule_id: "myrule".into(),
            reason: "bad".into(),
        };
        assert!(parse.to_string().contains("myrule"));

        let budget = EvalError::Budget { fuel: 10, mem: 20 };
        assert!(budget.to_string().contains("10") && budget.to_string().contains("20"));

        let runtime = EvalError::Runtime {
            rule_id: "r".into(),
            reason: "boom".into(),
            line: None,
        };
        assert!(runtime.to_string().contains('r') && runtime.to_string().contains("boom"));

        let too_large = EvalError::SourceTooLarge {
            rule_id: "big".into(),
            bytes: 999,
            limit: 100,
        };
        let s = too_large.to_string();
        assert!(s.contains("big") && s.contains("999") && s.contains("100"));
    }

    #[test]
    fn change_event_new_is_depth_zero_empty_payload() {
        let e = ChangeEvent::new("f.md", "a", "b");
        assert_eq!(e.depth, 0);
        assert!(e.sections_changed.is_empty());
        assert!(e.fields_changed.is_empty());
        assert_eq!(e.fingerprint_before, "a");
        assert_eq!(e.fingerprint_after, "b");
    }

    #[test]
    fn telemetry_reports_positive_fuel_for_a_firing_rule() {
        let rule = Rule::new(
            "fires",
            "def on_change(event):\n    notice(message = \"hi\")\n",
        );
        let event = ChangeEvent::new("f.md", "a", "b");
        let tel = eval_telemetry(&[rule], &event, EvalLimits::default());
        assert_eq!(tel.len(), 1);
        assert_eq!(tel[0].rule_id, "fires");
        assert!(tel[0].fuel_used > 0, "a rule that ran spent fuel");
        let effects = tel[0].outcome.as_ref().expect("ok");
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0].kind, EffectKind::Notice);
    }

    #[test]
    fn telemetry_marks_a_never_firing_rule_as_ok_empty() {
        let rule = Rule::new(
            "dead",
            "def on_change(event):\n    if \"nope\" in event.fields_changed:\n        notice(message = \"x\")\n",
        );
        let event = ChangeEvent::new("f.md", "a", "b");
        let tel = eval_telemetry(&[rule], &event, EvalLimits::default());
        // The dead-rule signal is `Ok(vec![])`, not fuel: Starlark tick
        // accounting is coarse, so a cheap false branch can read 0.
        assert!(tel[0].outcome.as_ref().expect("ok").is_empty());
    }

    #[test]
    fn telemetry_isolates_a_faulting_rule_from_the_rest() {
        // The batch path aborts on the first bad rule; telemetry must not —
        // replay needs every rule's outcome.
        let good = Rule::new(
            "good",
            "def on_change(event):\n    notice(message = \"w\")\n",
        );
        let bad = Rule::new("bad", "def on_change(event):\n    fail(\"boom\")\n");
        let good2 = Rule::new(
            "good2",
            "def on_change(event):\n    notice(message = \"n\")\n",
        );
        let event = ChangeEvent::new("f.md", "a", "b");
        let tel = eval_telemetry(&[good, bad, good2], &event, EvalLimits::default());
        assert_eq!(tel.len(), 3);
        assert_eq!(tel[0].outcome.as_ref().expect("good ok").len(), 1);
        assert!(matches!(tel[1].outcome, Err(EvalError::Runtime { .. })));
        assert_eq!(tel[2].outcome.as_ref().expect("good2 ok").len(), 1);
    }

    #[test]
    fn telemetry_is_deterministic_across_runs() {
        let rule = Rule::new(
            "r",
            "def on_change(event):\n    send(to = [\"a\", \"b\"], message = event.file)\n",
        );
        let event = ChangeEvent::new("doc.md", "before", "after");
        let a = eval_telemetry(std::slice::from_ref(&rule), &event, EvalLimits::default());
        let b = eval_telemetry(std::slice::from_ref(&rule), &event, EvalLimits::default());
        assert_eq!(a[0].fuel_used, b[0].fuel_used);
        assert_eq!(
            a[0].outcome.as_ref().expect("ok"),
            b[0].outcome.as_ref().expect("ok")
        );
    }
}
