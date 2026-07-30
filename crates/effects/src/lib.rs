//! The effect kernel — pure Starlark evaluation: rules in, effect descriptors
//! out, **zero I/O, zero integration**, advisory-only (decisions/0003,
//! decision #7 rename-and-demote).
//!
//! # Charter
//! **Owns:** turning a semantic [`ChangeEvent`] plus a set of [`Rule`]s (fenced
//! Starlark) into a deterministic `Vec<`[`Effect`]`>` — a pure function of
//! `(rules, event)`. Fuel-limited, depth-capped, panic-safe. The rule language
//! surface is the effect-descriptor builtins ([`set_field`], [`send`], … — see
//! the `kernel` module) over the Starlark standard library and nothing else: no
//! file, no network, no clock, no os. The only capabilities a rule reaches are
//! the descriptor constructors registered here.
//!
//! **Never does:** read or write disk, open a socket, apply an effect, watch a
//! tree, or talk to the daemon/wire. Effects are DESCRIPTORS — inert data a
//! consumer executes. This crate runs parallel to the resident-daemon spine and
//! touches nothing it owns (no splice choke point, no wire, no daemon) — and
//! never will: the crate is advisory-only by charter (decision #7). The
//! splice-point carve-in is DEAD, not reserved — this kernel will never gain a
//! correctness path. (The wire `effects[]` field remains a reserved later unit,
//! 0003 § reserved.)
//!
//! # The advisory law (0003 §6)
//! Effects are latency / UX, never correctness. An undelivered effect is lost
//! latency, never lost truth. Disk and fingerprints are correctness; the wire is
//! not. Nothing in this crate can change what is on disk.
//!
//! # Acknowledged limitation — cursor replay collapses intermediate transitions
//! At-least-once delivery is fingerprint-cursor replay (0003 §4), not a queue: on
//! reconnect a consumer presents its last fully-executed fingerprint and the
//! engine re-emits `diff(cursor, live)`. An outage between cursor and live
//! replays the **net** diff — `todo→review→done` re-emits as `todo→done`, and the
//! "entered review" transition **never fires**. This is lost by design and
//! accepted under the advisory law. The documented escape: a rule that must
//! record every transition writes its record INTO the tree at write time
//! (`append_section` / an `md.*` effect) — disk carries the history, the wire
//! never does.
//!
//! # Naming is not frozen
//! Descriptor kinds and their argument names are a SETTLED-direction sketch, not
//! a freeze: alignment with 108da20a's fused stage-2 winner may rename them
//! (0003 § reserved). Under delete-don't-migrate that is cheap — do not build a
//! migration layer against these names.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

mod kernel;

pub use kernel::validate;

/// One rule's metered outcome over a single event: its typed result plus the
/// EXACT fuel (Starlark ticks) and peak-heap bytes it spent. The corpus-replay
/// harness reads these for its per-rule consumption profile and dead-rule
/// detection. A rule that never reached eval (unparseable source, over the
/// source cap) reports `fuel_used`/`mem_used` of `0` and the typed authoring
/// fault; a bomb terminated at the ceiling reports the ceiling it hit.
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

/// Evaluate each rule INDEPENDENTLY over one `event` under explicit `limits`,
/// returning a metered outcome per rule (in slice order).
///
/// Unlike [`eval_with_limits`], this NEVER aborts the batch on the first
/// offending rule — every rule runs and reports its own typed outcome plus the
/// exact fuel/mem it spent. This is the corpus-replay contract: one bad rule
/// must not hide the fire counts of the rest, and the fuel profile needs
/// per-rule accounting the batch path discards. Determinism is unchanged (the
/// kernel is a pure function of `(rule, event, limits)`), so replaying the same
/// history twice yields byte-identical telemetry.
///
/// The cascade depth cap matches [`eval_with_limits`]: when
/// `event.depth >= limits.max_depth`, a rule's cascading `md.*` effects are
/// suppressed (only its terminal `daemon.*` / `proto.*` effects survive).
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

/// A namespaced effect descriptor kind. The `kind` a consumer routes by
/// capability. The string form (`md.set_field`, `proto.send`, …) is the wire /
/// snapshot identity.
///
/// NAMING IS NOT FROZEN (see crate docs): fused-winner alignment may rename.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EffectKind {
    /// `md.set_field` — set a frontmatter field.
    SetField,
    /// `md.append_section` — append content to a section.
    AppendSection,
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
    /// `proto.warn` — a warning advisory about the change.
    Warn,
}

impl EffectKind {
    /// Every descriptor kind, in a stable order — the "declare all capabilities"
    /// convenience and the source of truth for the closed surface.
    pub const ALL: [EffectKind; 8] = [
        EffectKind::SetField,
        EffectKind::AppendSection,
        EffectKind::RefreshView,
        EffectKind::Send,
        EffectKind::Remind,
        EffectKind::Ask,
        EffectKind::Notice,
        EffectKind::Warn,
    ];

    /// The namespaced wire / snapshot identity.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            EffectKind::SetField => "md.set_field",
            EffectKind::AppendSection => "md.append_section",
            EffectKind::RefreshView => "daemon.refresh_view",
            EffectKind::Send => "proto.send",
            EffectKind::Remind => "proto.remind",
            EffectKind::Ask => "proto.ask",
            EffectKind::Notice => "proto.notice",
            EffectKind::Warn => "proto.warn",
        }
    }

    /// The executor domain (the namespace before the `.`).
    #[must_use]
    pub fn domain(self) -> Domain {
        match self {
            EffectKind::SetField | EffectKind::AppendSection => Domain::Md,
            EffectKind::RefreshView => Domain::Daemon,
            EffectKind::Send
            | EffectKind::Remind
            | EffectKind::Ask
            | EffectKind::Notice
            | EffectKind::Warn => Domain::Proto,
        }
    }

    /// The Starlark constructor name a rule calls to mint this descriptor — the
    /// wire identity minus its domain prefix (`proto.send` → `send`).
    ///
    /// A consumer that enforces a capability ceiling at LOAD time (the HOOK
    /// declaration in `policy`) resolves declared caps to the exact global names a
    /// predicate may reach for. That set must be the one the kernel actually
    /// registers, so the mapping lives here beside the registration and is pinned
    /// to it by `kernel`'s `every_effect_kind_constructor_is_registered` — never
    /// re-derived by a caller splitting [`EffectKind::as_str`] on the dot.
    #[must_use]
    pub fn constructor(self) -> &'static str {
        match self {
            EffectKind::SetField => "set_field",
            EffectKind::AppendSection => "append_section",
            EffectKind::RefreshView => "refresh_view",
            EffectKind::Send => "send",
            EffectKind::Remind => "remind",
            EffectKind::Ask => "ask",
            EffectKind::Notice => "notice",
            EffectKind::Warn => "warn",
        }
    }

    /// The [`EffectKind`] a wire identity names (`"proto.send"` → [`EffectKind::Send`]),
    /// or `None` when the string is not one of the eight — the closed surface is the
    /// vocabulary, so an unknown name is denied rather than guessed at.
    #[must_use]
    pub fn from_wire_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == name)
    }
}

impl Serialize for EffectKind {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

/// One descriptor argument value: a scalar string or a list of strings. The
/// closed value shape a rule can hand a constructor — no numbers, no nested
/// maps; a descriptor is flat, inert data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ArgValue {
    /// A scalar string argument.
    Str(String),
    /// A list-of-strings argument (e.g. `send(to = [...])`).
    List(Vec<String>),
}

/// The plane an effect descriptor was produced on — its typed provenance. The
/// two planes carry DIFFERENT facts and one can never impersonate the other: a
/// change-plane effect was produced against a semantic diff and carries its
/// fingerprints (the cursor coordinates); a run-plane effect was produced by an
/// explicit invocation (`mrd run`) and carries the invocation identity plus the
/// tree root it evaluated against. Serialized with an explicit `plane` tag —
/// the public JSON surface states the plane from day one rather than
/// overloading fingerprint fields with run-plane values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "plane", rename_all = "lowercase")]
pub enum Provenance {
    /// Change-plane: emitted by `on_change` over a semantic diff (0003 §3).
    Change {
        /// The pre-change fingerprint of the diff.
        fingerprint_before: String,
        /// The post-change fingerprint of the diff — the cursor coordinate.
        fingerprint_after: String,
    },
    /// Run-plane: emitted by an explicit task invocation. There is no diff and
    /// no cursor — a re-run is new intent, never a replay.
    Run {
        /// The caller-supplied invocation id (the engine mints no identity).
        invocation_id: String,
        /// The workspace root fingerprint at evaluation time.
        root_at_eval: String,
    },
}

/// One effect descriptor — inert data a consumer executes. Carries the routing
/// `kind`, its flat `args`, the emitting `rule_id`, its plane-typed
/// [`Provenance`], its per-rule `seq`, and the cascade `depth`.
///
/// The idempotency key for executor dedup is `(rule_id, fingerprint_after, seq)`
/// (0003 §4), defined on the change plane only — see [`Effect::idempotency_key`].
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
    /// The executor-dedup idempotency key `(rule_id, fingerprint_after, seq)`
    /// (0003 §4) — `Some` only on the change plane, stable across re-evaluation
    /// of the same `(rules, event)`. A run-plane effect has NO dedup key: dedup
    /// exists for cursor replay, and a re-run is new intent — suppressing it as
    /// a replay would silently drop requested work.
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

/// The executor-dedup key of a change-plane [`Effect`] (0003 §4): a replayed
/// effect carrying a key the executor already applied is dropped. Run-plane
/// effects have no key (see [`Effect::idempotency_key`]).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct IdempotencyKey {
    /// The emitting rule.
    pub rule_id: String,
    /// The post-change fingerprint (the cursor coordinate).
    pub fingerprint_after: String,
    /// The per-rule emission index.
    pub seq: u32,
}

/// What one [`ChangeFact`] is about. The founding reaction predicate's own
/// discriminator (`delta.kind != "frontmatter"`), so it is a closed two-value
/// vocabulary rather than a free string.
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

/// One thing that changed, WITH its values — the fact `fields_changed` (names
/// only) could not express.
///
/// A frontmatter fact carries `key` plus `old`/`new`; absence is real, so a key
/// that did not exist before has `old: None` and a deleted key has `new: None`.
///
/// A section fact carries `hpath` and **no body text, ever**. A subscription
/// classifies on small values; section bodies are unbounded, and putting one in a
/// reaction payload would make every large edit an expensive event. What changed
/// is addressable; a consumer that wants the content reads it.
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

/// The world-model facts a reaction may read: the changed document as it NOW
/// stands. This is how a reaction reads its target off the card (ZT 24-01 — the
/// card defines the notification target).
///
/// **What is deliberately absent is the point.** There is no actor, no session,
/// no invocation identity here, and there must never be: a WHEN clause naming an
/// actor fact is refused, and the engine must not observe the observer. The
/// derivation drops `actor` even though the change plane carries it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct EventFacts {
    /// The document's path.
    pub path: String,
    /// Its frontmatter properties in document order, AFTER the change.
    pub frontmatter: Vec<(String, String)>,
}

/// The semantic change event — one `on_change` payload (0003 §3). Meridian diffs
/// semantically, so a rule filters on payload granularity (which sections, which
/// fields) rather than "file changed". There is exactly one hook; there is no
/// `on_section_change` kind.
///
/// `changes` and `facts` are the reaction plane's additions: `fields_changed`
/// names WHICH keys moved, `changes` says what they moved FROM and TO, and
/// `facts` carries the document those keys live on. The entry did not change —
/// `on_change(event)` is still the one change-plane entry; its argument grew.
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
    /// A depth-0 (user-originated) event. Convenience for the common case and
    /// for callers that do not model cascade themselves.
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

/// The run-plane context — one `run(ctx)` invocation's inert facts, ALL
/// caller-supplied (§9: the engine invents no identity and no time; nothing
/// here is read from a clock or minted in-kernel).
///
/// The sandbox sees `ctx.page`, `ctx.task`, `ctx.args`, `ctx.env` and nothing
/// else. `invocation_id` / `root_at_eval` are the Run-plane provenance facts
/// stamped onto every emitted effect ([`Provenance::Run`]) — deliberately NOT
/// injected: a task's output must not depend on its invocation identity, so a
/// re-run with the same inputs emits byte-identical effects and only the
/// provenance distinguishes the invocations.
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
    /// The workspace root fingerprint at evaluation time (Run-plane provenance).
    pub root_at_eval: String,
}

/// A rule: a caller-assigned `id` (its provenance — stamped onto every effect it
/// emits) plus its fenced-free Starlark `source` (a `def on_change(event):`
/// definition on the change plane, a `def run(ctx):` definition on the run
/// plane). Loading rule files from disk is the CALLER's job (that is the
/// I/O this crate refuses); the crate takes the loaded source and parses it.
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

/// The effect kinds a consumer declared executable — capability declaration
/// rides `hello` (0003 §2). The engine emits ALL effects deterministically; a
/// [`CapabilitySet`] is the downstream routing filter, kept separate so eval
/// stays a pure function of `(rules, event)` (cursor replay depends on that).
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

    /// Partition effects into `(admitted, rejected)` by capability, preserving
    /// order in each. Routing is a filter over the deterministic effect set —
    /// it never changes which effects were produced.
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

/// Deterministic bounds on one evaluation (0003 §1: fuel-limited, depth-capped).
/// Exceeding `fuel` (Starlark steps), `mem` (eval-heap bytes), or `max_call_depth`
/// (recursion depth) terminates the rule with [`EvalError::Budget`]; a runaway
/// loop, recursion bomb, or huge allocation can never hang. `max_depth` caps the
/// cascade: at or beyond it, the cascading `md.*` effects are suppressed.
/// `max_source_bytes` bounds parse cost (a `DoS` guard against pathological
/// source; the parser recurses per nesting level).
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
            // 64 KiB is generous for a single rule file (real rules are ~1 KiB)
            // and stays within the parse-recursion budget the fixed evaluation
            // stack provides (see `kernel::EVAL_STACK_BYTES`). Raising this
            // requires a proportionally larger evaluation stack.
            max_source_bytes: 64 * 1024,
        }
    }
}

/// Why an evaluation did not produce effects. Every hostile input — a runaway
/// bomb, malformed Starlark, a wrong-typed argument, a forged descriptor — lands
/// as one of these typed errors; nothing panics, nothing hangs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvalError {
    /// The rule exhausted its per-eval `{fuel, mem}` budget (runaway loop,
    /// recursion, or allocation). Terminated, never hung.
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
    /// The rule parsed but faulted at eval: no `on_change(event)` defined, a
    /// raised error, a wrong-typed argument to a constructor, an out-of-range
    /// index, or an attempt to reach a name the sandbox does not provide.
    Runtime {
        /// The rule that faulted.
        rule_id: String,
        /// The fault message.
        reason: String,
    },
    /// The rule source exceeded [`EvalLimits::max_source_bytes`] — refused
    /// before parsing (parse `DoS` guard).
    SourceTooLarge {
        /// The offending rule.
        rule_id: String,
        /// Its source length in bytes.
        bytes: usize,
        /// The configured limit.
        limit: usize,
    },
    /// The source parsed and evaluated but defines no entry for the plane it
    /// was addressed on — or defines the OTHER plane's entry (an `on_change`
    /// rule addressed as a task, or a `run` task fed a change event). One
    /// entry per plane; the planes never cross.
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
            EvalError::Runtime { rule_id, reason } => {
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

/// Evaluate `rules` over one `event`, under [`EvalLimits::default`], returning the
/// deterministic effect descriptor set — the pure kernel function of
/// `(rules, event)`.
///
/// See [`eval_with_limits`] for the full contract; this is the default-limits
/// convenience.
///
/// # Errors
/// The first rule to fail returns its typed [`EvalError`] — see
/// [`eval_with_limits`].
pub fn eval(rules: &[Rule], event: &ChangeEvent) -> Result<Vec<Effect>, EvalError> {
    eval_with_limits(rules, event, EvalLimits::default())
}

/// Evaluate `rules` over one `event` under explicit `limits`.
///
/// Determinism: rules run in slice order; within a rule, effects are recorded in
/// execution order with a per-rule `seq` (0, 1, …). Starlark is deterministic
/// (insertion-ordered dicts, no clock, no random), so the output is byte-identical
/// across runs and thread counts. The result is a pure function of
/// `(rules, event, limits)` — nothing observes the consumer, so cursor replay
/// (0003 §4) reproduces effects exactly.
///
/// Depth cap: when `event.depth >= limits.max_depth`, the cascading `md.*`
/// effects are suppressed (they would re-trigger `on_change`); terminal `daemon.*`
/// / `proto.*` effects still emit. A rule still runs and spends fuel — only its
/// tree-mutating descriptors are withheld past the cap.
///
/// # Errors
/// Fails loud on the FIRST offending rule — [`EvalError::Parse`] (malformed
/// source), [`EvalError::SourceTooLarge`], [`EvalError::Runtime`] (no
/// `on_change`, a raised error, a wrong-typed / forged argument), or
/// [`EvalError::Budget`] (a bomb terminated by the fuel/mem limits). Validate
/// rules once at load with [`validate`] to separate authoring faults from
/// per-event faults.
pub fn eval_with_limits(
    rules: &[Rule],
    event: &ChangeEvent,
    limits: EvalLimits,
) -> Result<Vec<Effect>, EvalError> {
    // The whole evaluation runs on a dedicated large-stack thread: the Starlark
    // recursive-descent parser can otherwise overflow the native stack on
    // pathologically nested source (research-starlark-mcp §1, issue #66) — an
    // uncatchable process abort that `catch_unwind` cannot contain. The large
    // virtual stack plus the source-size cap bounds parse recursion. The thread
    // makes no observable difference to the result (determinism preserved).
    kernel::on_eval_stack(|| {
        let globals = kernel::effect_globals();
        let mut out = Vec::new();
        for rule in rules {
            out.extend(kernel::run_rule(&globals, rule, event, limits)?);
        }
        // Depth cap (0003 §5): past the cascade ceiling, withhold the
        // tree-mutating (cascading) effects so a rule chain cannot loop forever
        // through the engine.
        if event.depth >= limits.max_depth {
            out.retain(|e| e.kind.domain() != Domain::Md);
        }
        Ok(out)
    })
}

/// Evaluate one task's `run(ctx)` — the run-plane entry (verdict rulings 1/8)
/// — under explicit `limits`, in the SAME sealed sandbox as `on_change`: the
/// same [`kernel::effect_globals`] closed builtin surface, the same fuel / mem
/// / call-depth metering, the same panic containment. There is no second
/// sandbox and no wider one — `run(ctx)` cannot spawn a process, read a clock,
/// or touch I/O any more than `on_change` can (starlark-invokes-bash is a
/// permanent no).
///
/// Emitted effects carry [`Provenance::Run`] (from `ctx.invocation_id` /
/// `ctx.root_at_eval`), `rule_id = ctx.task`, `depth = 0`, and NO idempotency
/// key — a re-run is new intent, never a replay. Determinism: the result is a
/// pure function of `(task, ctx, limits)`; the provenance facts are stamped,
/// never injected, so the effect PAYLOAD depends only on
/// `(page, task, args, env)`.
///
/// # Errors
/// The same typed surface as [`eval_with_limits`], plus
/// [`EvalError::MissingEntry`] when the source defines no `run(ctx)` — with
/// the wrong-plane detail when it defines `on_change` instead.
pub fn eval_run(task: &Rule, ctx: &RunCtx, limits: EvalLimits) -> Result<Vec<Effect>, EvalError> {
    // One identity per invocation (f9542789 U3-gate): `task.id` is the error
    // provenance, `ctx.task` the effect provenance — a divergence would let
    // the same failed run report two names. Refused, never reconciled.
    if task.id != ctx.task {
        return Err(EvalError::Runtime {
            rule_id: task.id.clone(),
            reason: format!(
                "task id '{}' != ctx.task '{}' — one identity per invocation",
                task.id, ctx.task
            ),
        });
    }
    kernel::on_eval_stack(|| {
        let globals = kernel::effect_globals();
        kernel::run_task(&globals, task, ctx, limits)
    })
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
        // The public JSON surface is plane-typed from day one: change-plane
        // effects carry fingerprints, run-plane effects carry invocation
        // identity — flattened, tagged, never overloaded into each other.
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
        // A rule that runs on every event but emits nothing is DEAD (never fired)
        // — the corpus-replay dead-rule signal is `Ok(vec![])`.
        let rule = Rule::new(
            "dead",
            "def on_change(event):\n    if \"nope\" in event.fields_changed:\n        notice(message = \"x\")\n",
        );
        let event = ChangeEvent::new("f.md", "a", "b");
        let tel = eval_telemetry(&[rule], &event, EvalLimits::default());
        // The dead-rule signal is `Ok(vec![])` — the rule ran without faulting
        // and emitted nothing. (Fuel is not the signal: Starlark's tick
        // accounting is coarse, so a cheap false branch can legitimately read 0.)
        assert!(tel[0].outcome.as_ref().expect("ok").is_empty());
    }

    #[test]
    fn telemetry_isolates_a_faulting_rule_from_the_rest() {
        // The batch path aborts on the first bad rule; telemetry must NOT — a
        // faulting rule reports its own error while the others still report their
        // effects (replay needs every rule's outcome).
        let good = Rule::new("good", "def on_change(event):\n    warn(message = \"w\")\n");
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
