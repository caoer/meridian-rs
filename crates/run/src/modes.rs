//! The `run` op's mode-bearing rows — `mode:"load"` and `mode:"fire"`
//! (hook-support design § 2.2, as amended by § Amendments / A1).
//!
//! Two modes, one verb. **`load`** evaluates a page's starlark blocks' top
//! levels in the load phase and publishes what each one declares.
//! **`fire`** calls one declared block's frozen entry with a JSON input and
//! answers its return plus the md effects it applied through the ordinary
//! doors.
//!
//! Laws held here:
//!
//! - **The consent gate.** `run` executes what the page DECLARES —
//!   `task.<name>` in frontmatter or `declare()` in the block, never an
//!   undeclared block. A fire naming a bare anchored fence refuses
//!   `not_declared` at the door.
//! - **Recording by declaration kind.** A fire row writes no receipt and takes
//!   no lock beyond the batch's own; a task row is unchanged. There is no
//!   caller flag — the engine reads the page.
//! - **The entry world is PINNED, never folded.** These paths take the
//!   resident snapshot the script op takes (an `Arc` clone) and never run the
//!   `domain_snapshot` fold, which is what keeps a fire off the task path's
//!   per-run corpus walk.

use std::collections::BTreeMap;
use std::path::Path;

use effects::{
    BlockFault, DeclaredEntry, Effect, EvalLimits, FireHost, RunCtx, fire_entry, load_block,
};
use model::{Document, MerkleRoot};
use serde_json::{Value, json};

use crate::blocks::{self, AnchoredBlock, BlockError};
use crate::caps::{self, Authority};
use crate::executor::{self, Applied, ApplyRequest, DeltaSink, ExecError};

/// What one mode-bearing target runs against.
///
/// The DOC is a parameter, not a lookup: the two callers hold it differently
/// and both are right. The daemon hands the page out of its PINNED resident
/// snapshot (an `Arc` clone — never the `domain_snapshot` fold); the CLI has
/// no resident engine and hands the page it just loaded. Putting the world
/// behind a borrow here is what lets ONE implementation of the rows serve
/// both lanes, so the two cannot answer differently.
pub struct ModeWorld<'a> {
    /// The page, as the caller's world serves it.
    pub doc: &'a Document,
    /// The workspace root the effects apply against.
    pub root: &'a fs::WorkspaceRoot,
    /// The declaring root — whose conventions ceiling narrows the page's
    /// `caps:`. `None` when nothing is entitled to declare policy.
    pub declaring_root: Option<&'a Path>,
    /// The corpus root the fire observed. Receipt provenance only; this door
    /// holds no world pin (the 2026-08-15 no-guard-on-effects ruling).
    pub observed_root: &'a MerkleRoot,
    /// The caller's `prelude` (cap `run.mode`), shared by every mode-bearing
    /// target in the call.
    pub prelude: Option<&'a str>,
    /// The door-side facilities a realized effect rides.
    pub doors: Doors<'a>,
    /// The per-block-rev module cache, when the caller keeps one. `None` on
    /// the CLI: a fresh process per invocation has nothing to keep it in.
    pub cache: Option<&'a dyn ModuleCache>,
}

/// The door-side facilities one fire's md effects ride — the same four the
/// task path threads, with the same meanings, so a fire's writes are
/// indistinguishable from any other door write. All optional: the CLI is a
/// separate process with no ring in reach, exactly as on the task path.
#[derive(Default)]
pub struct Doors<'a> {
    /// Delta honesty: the host's frame mint. `None` on the CLI.
    pub delta: Option<&'a dyn DeltaSink>,
    /// The workspace ring as the create door's `SeqSink`. `None` on the CLI.
    pub birth_seq: Option<&'a dyn wire_serve::seq::SeqSink>,
    /// § A.2.1 passthrough onto every birth's `ctx.fields`.
    pub fields: Option<&'a BTreeMap<String, String>>,
    /// The caller's ambient directory, workspace-relative.
    pub ambient: Option<&'a str>,
}

/// How many exec logs a PAGE keeps (§ 2.2's stated ceiling).
const LOG_RETENTION: usize = 50;

/// Where a fire's logs live, out of tree. One directory per page below it
/// (`<RUNS_DIR>/<page-path>/`), which is what makes the ceiling above *per
/// page* and keeps the fire path out of the top level, where the task path's
/// receipted logs are.
const RUNS_DIR: &str = ".meridian/runs";

/// Where an exec'd entry's bytes are staged, keyed by the block's rev — the
/// design's *"staged bytes cached by block rev"*. Out of tree, like the logs.
const STAGED_DIR: &str = ".meridian/staged";

/// How many staged programs a workspace keeps. Same ceiling as the logs, same
/// reason: a cache keyed by every rev ever fired grows without bound.
const STAGED_RETENTION: usize = 50;

/// How many bytes of each stream the `process` object publishes.
///
/// The field is named `stdout_tail`/`stderr_tail` and § 2.2 says *tails*, but
/// it carried the WHOLE stream: a chatty exec'd entry put every byte it wrote
/// on the wire, in the daemon's journal and in an agent's context on every
/// fire — the unbounded per-fire cost PR 195's F8 removed from `exec[]` rows,
/// still open on the process row. The tail is the LAST bytes (a failure
/// explains itself at the end, not at the beginning); the whole stream is in
/// the log the row points at, and `stdout_bytes`/`stderr_bytes` say how much
/// was cut.
const TAIL_BYTES: usize = 4096;

/// The last [`TAIL_BYTES`] of a stream, on a char boundary, plus its true
/// length.
fn tail(bytes: &[u8]) -> (String, usize) {
    let len = bytes.len();
    let from = len.saturating_sub(TAIL_BYTES);
    (String::from_utf8_lossy(&bytes[from..]).into_owned(), len)
}

/// Empty fields for a caller with no frame passthrough in reach.
static NO_FIELDS: std::sync::LazyLock<BTreeMap<String, String>> =
    std::sync::LazyLock::new(BTreeMap::new);

/// One block as the load phase left it: what it declares, and the frozen
/// module a fire calls.
///
/// Cached by the block's own rev (§ 2.2 step 3), which is what makes a warm
/// fire ONE function call instead of a parse, an evaluation and a freeze.
pub struct LoadedBlock {
    /// What `declare()` published, in call order.
    pub declarations: Vec<effects::Declaration>,
    /// The frozen module. `Send + Sync` — asserted at compile time in the
    /// kernel, because a resident cache is reached from every connection
    /// thread and discovering otherwise at integration time would arrive as
    /// an unexplainable borrow error three layers away.
    pub module: effects::FrozenModule,
}

/// The per-block-rev module cache (§ 2.2 step 3).
///
/// A trait, not a concrete map, because only ONE caller has anywhere to keep
/// it: the daemon, whose registry is resident. The CLI is a fresh process per
/// invocation and passes `None` — a cache there would be built, used once and
/// dropped, which is a cost with no benefit rather than a small one.
pub trait ModuleCache: Sync {
    /// The block at this key, if it is still the same bytes under the same
    /// prelude.
    fn get(&self, key: &str) -> Option<std::sync::Arc<LoadedBlock>>;
    /// Keep this block under this key.
    fn put(&self, key: String, loaded: std::sync::Arc<LoadedBlock>);
}

/// The cache key: the block's own rev, plus a digest of the prelude.
///
/// The prelude is part of the key because it is part of the MODULE — its
/// bindings are frozen with the block's (§ 2.2). Two calls with different
/// preludes produce different modules from identical block bytes, and keying
/// on the rev alone would serve one caller the other's environment.
fn cache_key(block_rev: &str, prelude: Option<&str>) -> String {
    match prelude {
        None => format!("{block_rev}:-"),
        Some(prelude) => format!(
            "{block_rev}:{}",
            &blake3::hash(prelude.as_bytes()).to_hex()[..16]
        ),
    }
}

/// Load one block, through the cache when the caller brought one.
///
/// A FAULTED load is never cached: a fault is about the attempt, and the next
/// caller deserves to see it happen rather than be handed a stored refusal
/// whose cause may already be fixed.
fn load_cached(
    world: &ModeWorld<'_>,
    block: &AnchoredBlock,
    ctx: &RunCtx,
    limits: EvalLimits,
) -> Result<std::sync::Arc<LoadedBlock>, BlockFault> {
    let key = cache_key(&block.rev, world.prelude);
    if let Some(cache) = world.cache
        && let Some(hit) = cache.get(&key)
    {
        return Ok(hit);
    }
    let loaded = load_block(&block.source, world.prelude, ctx, limits);
    if let Some(fault) = loaded.fault {
        return Err(fault);
    }
    let Some(module) = loaded.module else {
        return Err(BlockFault {
            class: effects::FaultClass::Runtime,
            reason: "the block loaded but did not freeze".to_owned(),
            line: None,
        });
    };
    let entry = std::sync::Arc::new(LoadedBlock {
        declarations: loaded.declarations,
        module,
    });
    if let Some(cache) = world.cache {
        cache.put(key, std::sync::Arc::clone(&entry));
    }
    Ok(entry)
}

/// The caller's evaluation ceilings, applied — **effective = min(declared,
/// ceiling)** (§ 2.2, the formula `wire-contract.md` § A.8 publishes and
/// `Op::Run`'s doc comment repeats).
///
/// A caller NARROWS; nothing a caller sends can raise the engine's own
/// ceiling, so this is a `min` in both axes and never an assignment. Absent
/// fields leave the engine ceiling standing.
///
/// This exists because `budget` was decoded, validated (positive integers, a
/// closed `{steps, mem}` object, an empty-budget refusal) and then read zero
/// times: every eval site passed `EvalLimits::default()`, so a hook declaring
/// `{"steps":10000,"mem":4194304}` ran at 100× the steps and 16× the memory
/// it asked for, while `telemetry` reported the real consumption and made the
/// row look instrumented. (PR 195 review, e9f1ae35, F1.)
fn eval_limits(target: &wire::RunTarget) -> EvalLimits {
    let mut limits = EvalLimits::default();
    if let Some(budget) = target.budget.as_ref() {
        if let Some(steps) = budget.steps {
            limits.fuel = limits.fuel.min(steps);
        }
        if let Some(mem) = budget.mem {
            limits.mem = limits.mem.min(mem);
        }
    }
    limits
}

/// The process wall-clock ceiling for this target: the root's configured
/// ceiling, narrowed by the caller's `timeout_ms`. Same law as
/// [`eval_limits`] — `min`, never an assignment.
fn process_timeout(world: &ModeWorld<'_>, target: &wire::RunTarget) -> std::time::Duration {
    let ceiling = crate::exec::configured_timeout(world.declaring_root)
        .unwrap_or(crate::exec::DEFAULT_TIMEOUT);
    match target.timeout_ms {
        Some(ms) => ceiling.min(std::time::Duration::from_millis(ms)),
        None => ceiling,
    }
}

/// One mode-bearing target → one row.
#[must_use]
pub fn mode_row(
    world: &ModeWorld<'_>,
    target: &wire::RunTarget,
    invocation: &str,
    actor: Option<&str>,
    now: Option<&str>,
) -> Value {
    // The prelude is checked ONCE, before any block is looked at, on BOTH
    // modes — § 2.2: `prelude_invalid` refuses before any block runs.
    //
    // It used to be checked in `load_row` alone, and the fire path paid for
    // it: a typo in the DAEMON-supplied prelude faulted inside the block's
    // own evaluation, so every hook on every page answered `name_error`
    // naming that page, that block's rev, and a line number belonging to
    // source the page's author cannot see. `--load` on the same input said
    // `prelude_invalid` and named it correctly, so the two modes disagreed
    // about one broken input and the mode that lied was the one that runs in
    // production. (PR 195 review, e9f1ae35, F9.)
    if let Some(prelude) = world.prelude
        && let Some(fault) =
            effects::check_prelude(prelude, &ctx_for(target, invocation), eval_limits(target))
    {
        return refused_row(
            target,
            invocation,
            "prelude_invalid",
            &fault.reason,
            fault.line,
        );
    }
    // `source` — a page that is not on disk yet (§ 2.2; § 6's row: *"`source`
    // (forces `dry`)"*). The wall forces `dry` and relaxes `page`, so nothing
    // downstream can forget; what was missing is anyone READING it, so a
    // client that negotiated the cap and sent a draft got `bad_path` naming
    // an empty page. Building the document here keeps ONE owner, so the CLI
    // and the daemon cannot answer differently. (PR 195 review, e9f1ae35, F6.)
    if let Some(source) = target.source.as_deref() {
        let drafted = model::build(source.to_owned(), syntax::parse(source));
        let drafted_world = ModeWorld {
            doc: &drafted,
            root: world.root,
            declaring_root: world.declaring_root,
            observed_root: world.observed_root,
            prelude: world.prelude,
            doors: Doors::default(),
            cache: None,
        };
        return match target.mode {
            Some(wire::RunMode::Load) => load_row(&drafted_world, target, invocation),
            // A fire needs a declared block to call. `source` carries no
            // identity a caller can address twice, and its `dry` makes the
            // effects a rehearsal, so a fire on a draft is refused by name
            // rather than half-served.
            Some(wire::RunMode::Fire) | None => refused_row(
                target,
                invocation,
                "bad_request",
                "`source` serves `mode: load` — it answers what a draft page \
                 DECLARES. A fire calls a declared block on a page that exists",
                None,
            ),
        };
    }
    match target.mode {
        Some(wire::RunMode::Load) => load_row(world, target, invocation),
        Some(wire::RunMode::Fire) => fire_row(world, target, invocation, actor, now),
        // `mode_row` is only reached for a mode-bearing target; the shipped
        // task path is chosen before this call.
        None => refused_row(
            target,
            invocation,
            "bad_request",
            "no mode on a mode row",
            None,
        ),
    }
}

/// A `mode:"load"` row: `{page, rev, loaded: [...]}`.
fn load_row(world: &ModeWorld<'_>, target: &wire::RunTarget, invocation: &str) -> Value {
    let doc = world.doc;
    let loaded: Vec<Value> = blocks::anchored_blocks(doc)
        .into_iter()
        .filter_map(|block| match block {
            Ok(block) => loaded_row(world, target, invocation, &block),
            // A duplicated anchor is a page fault the author must be told
            // about; it appears as a row of its own rather than a silence.
            Err(e) => Some(json!({
                "block": anchor_of(&e),
                "result": "fault",
                "fault": {"class": e.class(), "reason": e.to_string()},
            })),
        })
        .collect();
    json!({
        "page": target.page,
        "invocation": invocation,
        "rev": {"file": doc.root.node_rev.0},
        "loaded": loaded,
    })
}

/// One block's load row, or `None` when the block is not a module and not a
/// task — those are exec targets and `bash(block=)` helpers, and a load that
/// listed them would answer with the page's whole prose (§ 2.2 step 1).
fn loaded_row(
    world: &ModeWorld<'_>,
    target: &wire::RunTarget,
    invocation: &str,
    block: &AnchoredBlock,
) -> Option<Value> {
    // A task-bound block is the RUN plane's: reported with `entry_kind: task`
    // and no declarations, never evaluated here (§ 2.2 step 2).
    if let Some(task) = &block.task {
        return Some(json!({
            "block": block.anchor,
            "rev": block.rev,
            "result": "ok",
            "entry_kind": "task",
            "task": task,
            "declarations": [],
        }));
    }
    if !block.is_starlark() {
        return None;
    }
    let loaded = match load_cached(
        world,
        block,
        &ctx_for(target, invocation),
        eval_limits(target),
    ) {
        Ok(loaded) => loaded,
        Err(fault) => {
            return Some(json!({
                "block": block.anchor,
                "rev": block.rev,
                "result": "fault",
                "fault": fault_object(&fault),
            }));
        }
    };
    // § 2.2 step 2: an `exec(block=)` anchor resolves HERE, at load.
    if let Some(fault) = exec_anchor_fault(world.doc, block, &loaded) {
        return Some(fault);
    }
    // A starlark block that declares nothing is not a target. It is not a
    // fault either — a page may carry a helper module — so it is reported
    // with no declarations and the consent gate refuses a fire on it.
    let declarations: Vec<Value> = loaded.declarations.iter().map(|d| d.data.clone()).collect();
    let entry_kind = loaded
        .declarations
        .first()
        .map_or("evaluated", |d| d.entry.kind());
    Some(json!({
        "block": block.anchor,
        "rev": block.rev,
        "result": "ok",
        "entry_kind": entry_kind,
        "declarations": declarations,
    }))
}

/// A `mode:"fire"` row: one declared block's entry, called.
fn fire_row(
    world: &ModeWorld<'_>,
    target: &wire::RunTarget,
    invocation: &str,
    actor: Option<&str>,
    now: Option<&str>,
) -> Value {
    let doc = world.doc;
    let addressed = match addressed_entry(world, target, invocation, doc) {
        Ok(resolved) => resolved,
        Err(row) => return row,
    };
    let ctx = ctx_for(target, invocation);
    let seam = ProcessSeam {
        doc: world.doc,
        root: &world.root.0,
        page: &target.page,
        invocation,
        calls: std::sync::Mutex::new(Vec::new()),
        // The root's ceiling, narrowed by the caller's `timeout_ms`
        // (`process_timeout`): min(declared, ceiling), never an assignment.
        timeout: process_timeout(world, target),
        dry: target.dry.unwrap_or(false),
    };
    let (block, entry, loaded) = match addressed {
        Addressed::Evaluated {
            block,
            entry,
            loaded,
        } => {
            // `env` on an evaluated-entry fire: REFUSED, as the doc says.
            // There is no process to receive it, and a field that is
            // meaningless on a target must refuse rather than be silently
            // dropped — the guard-you-believe-is-armed trap. It cannot refuse
            // at the decode wall: whether a block's entry is evaluated or
            // exec'd is a fact about the PAGE, which the wall does not read.
            // (PR 195 review, e9f1ae35, F2a.)
            if !target.env.is_empty() {
                return refused_row(
                    target,
                    invocation,
                    "bad_request",
                    &format!(
                        "`env` is refused on a fire of ^{} — that block declares an \
                         EVALUATED entry, and no process exists to receive it. `env` \
                         is an exec'd entry's process overlay",
                        block.anchor
                    ),
                    None,
                );
            }
            (block, entry, loaded)
        }
        // An exec'd entry never reaches the evaluator: one process through
        // the bracket, its stdin the input, its raw exit and stderr surfaced
        // (§ 1.4). Caps do not apply to a process, by law — what bounds it is
        // the timeout and the loud result.
        Addressed::Exec { block, spec } => {
            return exec_row(&seam, target, invocation, &block, &spec, doc);
        }
    };
    let started = std::time::Instant::now();
    let fired = match fire_entry(
        &loaded.module,
        &entry,
        target.input.as_ref().unwrap_or(&Value::Null),
        &ctx,
        &seam,
        eval_limits(target),
    ) {
        Ok(fired) => fired,
        Err(e) => {
            return refused_row(target, invocation, "missing_entry", &e.to_string(), None);
        }
    };
    if let Some(fault) = fired.fault {
        return fault_row(target, invocation, &block, &fault);
    }

    // The md effects, realized through the ordinary doors under the PAGE's
    // `caps:` — the new grammar § 2.2 calls for, judged at the lifted admit
    // choke point, before any I/O.
    let (applied, refusal) = realize(
        world,
        target,
        invocation,
        &block,
        &fired.effects,
        actor,
        now,
    );
    let mut row = json!({
        "page": target.page,
        "invocation": invocation,
        "block": block.anchor,
        "rev": {"file": doc.root.node_rev.0, "block": block.rev},
        // A DOOR refusal is the effect row's, never this one's: a hook that
        // answered `{"deny": …}` and also tried a write the armed plane
        // refused must still deliver its verdict, or a daemon that checks
        // `result == "ok"` before reading `value` drops the deny. Only a
        // failure to carry the batch at all refuses here.
        // (PR 195 review, e9f1ae35, F5b; § 2.2 and run-plane.md § the rule.)
        "result": match &refusal {
            Some(Refusal::Row { .. }) => "refused",
            Some(Refusal::Door) | None => "ok",
        },
        "applied": applied,
        "exec": seam.rows(),
        "telemetry": {
            "steps": fired.fuel_used,
            "mem": fired.mem_used,
            // Published by § 2.2 and absent until now. A fire's wall time is
            // the one telemetry number a caller with a deadline can act on:
            // fuel says nothing about a `bash()` that slept.
            "wall_ms": u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        },
    });
    if let Some(value) = fired.value {
        row["value"] = value;
    }
    if let Some(Refusal::Row { class, reason }) = refusal {
        row["fault"] = json!({"class": class, "reason": reason});
    }
    row
}

/// Address the fire's block, load it, and resolve its entry — everything that
/// happens before the call. Returns the row's refusal on the left when any of
/// it refuses, so the caller stays one straight line.
///
/// **The consent gate lives here**: `run` executes what the page DECLARES —
/// a `task.<name>` binding in frontmatter or a `declare()` in the block —
/// never an undeclared one.
fn addressed_entry(
    world: &ModeWorld<'_>,
    target: &wire::RunTarget,
    invocation: &str,
    doc: &Document,
) -> Result<Addressed, Value> {
    let Some(anchor) = target.block.as_deref() else {
        return Err(refused_row(
            target,
            invocation,
            "no_block",
            "a fire addresses a block: pass `block`",
            None,
        ));
    };
    let block = blocks::block(doc, anchor)
        .map_err(|e| refused_row(target, invocation, e.class(), &e.to_string(), None))?;
    if !block.is_starlark() {
        // A non-starlark block is an exec'd entry's BYTES, never a module of
        // its own. It runs when a starlark block DECLARES it — `declare(impl
        // = exec("bash", block = "..."))` — which is the consent gate again:
        // an anchored fence nobody declared is not a target, whatever its
        // language.
        return Err(refused_row(
            target,
            invocation,
            "not_a_module",
            &format!(
                "^{anchor} is not a starlark block, so it is not a module: a non-starlark \
                 block runs as an exec'd entry, addressed through the starlark block that \
                 declares it with `exec(...)`"
            ),
            None,
        ));
    }
    // A TASK-BOUND block is the task path's, and that is a fact about the
    // PAGE — so it is judged here, before the block is even loaded, mirroring
    // the load path's own early return. It used to be judged only inside the
    // `else` of "the block declared nothing", which made it reachable only
    // when no declaration existed: a prelude-supplied declaration would have
    // hijacked a task-bound block into a fire, and a task-bound block has no
    // `declare()` of its own to refuse with. (PR 195 review, `fa5da9ec`, S3
    // B2 — the narrow fix would have left exactly this open.)
    if let Some(task) = block.task.as_deref() {
        return Err(refused_row(
            target,
            invocation,
            "not_declared",
            &format!(
                "^{anchor} is bound to task `{task}`: address it as a task target \
                 (`task`), not as a fire — the two addressings are exclusive"
            ),
            None,
        ));
    }
    let ctx = ctx_for(target, invocation);
    let loaded = load_cached(world, &block, &ctx, eval_limits(target))
        .map_err(|fault| fault_row(target, invocation, &block, &fault))?;
    let Some(declaration) = loaded.declarations.first() else {
        return Err(refused_row(
            target,
            invocation,
            "not_declared",
            // A task-bound block never reaches here — the guard above judges
            // that before the load — so this arm is the one case left: an
            // anchored starlark fence that declares nothing.
            &format!(
                "^{anchor} declares nothing: `run` executes what the page declares — a \
                 `task.<name>` binding in frontmatter, or `declare(...)` in the block"
            ),
            None,
        ));
    };
    let entry = match &declaration.entry {
        DeclaredEntry::Evaluated { name } => name.clone(),
        // An exec'd entry needs no frozen module: the declaration IS the
        // program (§ 1.4). Every other language runs this way — one generic
        // process contract, never a peer evaluator.
        DeclaredEntry::Exec(spec) => {
            // § 2.2 resolves an `exec(block=)` anchor at LOAD: the declaration
            // is the program, so a declaration naming a fence that is not
            // there is broken the moment it is read, not when it is called.
            // Resolving it here — the fire's own load step — is what makes a
            // dangling anchor a `no_block` refusal with the anchor's own
            // words, instead of a fabricated `exit: 127` row.
            if let effects::ExecProgram::Block(anchor) = &spec.program
                && let Err(e) = blocks::block(doc, anchor)
            {
                return Err(refused_row(
                    target,
                    invocation,
                    "no_block",
                    &format!(
                        "^{} declares `exec(block = \"{anchor}\")`, and this page has no \
                         such block: {e}",
                        block.anchor
                    ),
                    None,
                ));
            }
            return Ok(Addressed::Exec {
                block,
                spec: spec.clone(),
            });
        }
    };
    Ok(Addressed::Evaluated {
        block,
        entry,
        loaded,
    })
}

/// One exec'd entry's row: the process, through the same bracket `bash()`
/// uses (§ 1.4 — *one generic execution contract, and a new language is
/// `argv[0]`*).
///
/// **Caps are inert here, by law.** What bounds an exec'd entry is the
/// timeout, the out-of-tree log, and the loud result — never a capability,
/// because a capability cannot gate a process that could `sed -i` its way
/// around it.
fn exec_row(
    seam: &ProcessSeam<'_>,
    target: &wire::RunTarget,
    invocation: &str,
    block: &AnchoredBlock,
    spec: &effects::ExecSpec,
    doc: &Document,
) -> Value {
    // The input is stdin, verbatim compact JSON — that is what makes a
    // `settings.json` script's bytes run unchanged.
    let stdin = target
        .input
        .as_ref()
        .map(|v| serde_json::to_string(v).unwrap_or_default());
    // `MRD_RUN_BLOCK` is the DECLARING block — the one the caller addressed —
    // not the fence `exec(block=)` points at. A script shared by several
    // blocks on a page branches on which block it is running for.
    let process = match seam.exec(spec, stdin.as_deref(), &block.anchor, target) {
        Ok(process) => process,
        // Could not START — an unstartable interpreter. Not a script's answer,
        // so not an `ok` row; the reason is the one the OS gave, never null.
        Err((class, reason)) => {
            return refused_row(target, invocation, class, &reason, None);
        }
    };
    let row = json!({
        "page": target.page,
        "invocation": invocation,
        "block": block.anchor,
        "rev": {"file": doc.root.node_rev.0, "block": block.rev},
        // The row's `result` is an EVALUATION word about the fire, not about
        // the process's exit: a script that exits 1 ran fine and said no.
        // `process.exit` carries what the script said.
        "result": "ok",
        "applied": [],
        "exec": [],
        "process": process,
    });
    // NO exit-code remap. `127` is the shell's own "command not found" and is
    // reachable from any script whose binary is off `PATH`: rewriting that row
    // into `fault`/`no_block` contradicted the doctrine three lines above and
    // destroyed the diagnostic besides — the branch read `process["stderr"]`
    // while every construction site spells `stderr_tail`, so `fault.reason`
    // was JSON `null` on every hit. An entry that could not START says so
    // through `ProcessSeam::exec`'s own Err, below, which keeps its reason.
    // (PR 195 review, e9f1ae35, F3.)
    row
}

/// What a fire's addressing resolved to — the two entry kinds § 1.4 admits.
enum Addressed {
    /// A starlark entry on the block's own frozen module.
    Evaluated {
        block: AnchoredBlock,
        entry: String,
        loaded: std::sync::Arc<LoadedBlock>,
    },
    /// A process entry, declared by `exec(...)`.
    Exec {
        block: AnchoredBlock,
        spec: effects::ExecSpec,
    },
}

/// Realize one fire's md effects. Returns the `applied[]` rows and, when the
/// batch refused, the row's own `(class, reason)`.
#[allow(clippy::too_many_arguments)]
fn realize(
    world: &ModeWorld<'_>,
    target: &wire::RunTarget,
    invocation: &str,
    block: &AnchoredBlock,
    effects: &[Effect],
    actor: Option<&str>,
    now: Option<&str>,
) -> (Vec<Value>, Option<Refusal>) {
    if effects.is_empty() {
        return (Vec::new(), None);
    }
    let authority = match page_authority(world) {
        Ok(authority) => authority,
        Err(e) => {
            return (
                Vec::new(),
                Some(Refusal::Row {
                    class: "cap_denied",
                    reason: e.to_string(),
                }),
            );
        }
    };
    // `dry` lists the effects and applies none — the shipped rehearsal
    // switch, unchanged in meaning on a fire row.
    if target.dry.unwrap_or(false) {
        let rows = effects
            .iter()
            .map(|e| json!({"kind": e.kind.as_str(), "result": "dry", "args": args_of(e)}))
            .collect();
        return (rows, None);
    }
    let request = ApplyRequest {
        page: &target.page,
        // A fire has no task name; the block's anchor is what identifies it,
        // and it is what a reader of any provenance this leaves behind needs.
        task: &block.anchor,
        task_rev: &block.rev,
        invocation_id: invocation,
        now,
        effects,
        authority: &authority,
        observed_root: world.observed_root,
        // **A fire row writes no receipt** — the recording law. The record of
        // a fire is its response and the caller's own journal.
        receipt: None,
        exec: None,
        actor,
        depth: 0,
        delta: world.doors.delta,
        fields: world.doors.fields.unwrap_or(&NO_FIELDS),
        birth_seq: world.doors.birth_seq,
        ambient: world.doors.ambient,
    };
    match executor::apply(world.root, &request) {
        Ok(applied) => {
            let rows = effects
                .iter()
                .map(|e| applied_row(world, e, &applied))
                .collect();
            (rows, None)
        }
        Err(e) => {
            // `cap_denied` keeps its teaching text — the grants it measured
            // and the ceiling that took the cap are what an author fixes.
            let (class, reason) = match &e {
                ExecError::CapDenied {
                    kind,
                    target: coordinate,
                    ceiling,
                    declared,
                    ..
                } => (
                    "cap_denied",
                    format!(
                        "`{kind}` on `{coordinate}` is outside this page's `caps:` \
                         (granted: {}; ceiling: {})",
                        if declared.is_empty() {
                            "none".to_owned()
                        } else {
                            declared.join(", ")
                        },
                        ceiling.clone().unwrap_or_else(|| "none".to_owned())
                    ),
                ),
                other if is_door_refusal(other) => (door_class(other), other.to_string()),
                other => ("runtime", other.to_string()),
            };
            // Nothing landed either way — the batch is atomic. What changes
            // is WHOSE refusal it is: a door's verdict belongs to the
            // descriptor it judged, and the fire row keeps its own result and
            // its `value` (the never-veto law); an infrastructure failure is
            // the row's, because the engine could not carry the batch at all.
            let rows = refused_rows(effects, &e, class, &reason);
            if is_door_refusal(&e) {
                (rows, Some(Refusal::Door))
            } else {
                (rows, Some(Refusal::Row { class, reason }))
            }
        }
    }
}

/// Whose refusal a failed realization is.
enum Refusal {
    /// A door judged one descriptor. Its row carries the verdict; the fire
    /// row stays `ok` and keeps its `value`.
    Door,
    /// The engine could not carry the batch. The fire row refuses.
    Row {
        /// The published fault class.
        class: &'static str,
        /// The door's own words.
        reason: String,
    },
}

/// What an effect that LANDED is called (§ 2.2's row vocabulary, amended —
/// A8).
///
/// § 2.2 spells the triple `born|exists|refused`. Two departures, both
/// deliberate and both on the card: this door has no `exists` arm — an
/// occupied path REFUSES at the create door, so `exists` is never emitted —
/// and an edit is `edited`, a word § 2.2 does not name, because the
/// alternative is calling an edit a birth.
fn result_word(kind: effects::EffectKind) -> &'static str {
    match kind {
        effects::EffectKind::Create => "born",
        _ => "edited",
    }
}

/// Whether this refusal happened BEFORE the birth lane ran.
///
/// Only the workspace lock does: it is taken in `executor::apply` before any
/// descriptor is looked at. Everything else — birth-target resolution, the
/// birth lane, the page load, the splice — runs after, so a refusal from any
/// of them leaves the birth lane's own progress on disk (and, when it is the
/// birth lane, carries the index that says how much).
fn pre_birth(e: &ExecError) -> bool {
    matches!(e, ExecError::WorkspaceBusy)
}

/// Whether this executor refusal is a DOOR saying no, or the engine being
/// unable to carry the batch at all.
///
/// The distinction is the never-veto law made operational (§ 2.2, and
/// `docs/run-plane.md` § A door refusal is that effect's row):
///
/// - a **door refusal** — caps, an occupied birth, an armed-middleware veto,
///   a section that is not there or is there twice, an fp-claim, a verdict —
///   is about ONE descriptor. It becomes that effect's own row, and **the
///   fire row stays `ok` and keeps its `value`**, because a hook's answer
///   must not be discarded because an unrelated write was refused.
/// - an **infrastructure failure** — the workspace lock, I/O, a page that
///   will not load, a non-md descriptor, a malformed descriptor — is not a
///   verdict about a descriptor at all. The engine could not carry the batch,
///   so the fire row refuses and says which class.
fn is_door_refusal(e: &ExecError) -> bool {
    matches!(
        e,
        ExecError::CapDenied { .. }
            | ExecError::BirthRefused { .. }
            | ExecError::ArmedRefusal { .. }
            | ExecError::SectionNotFound { .. }
            | ExecError::SectionAmbiguous { .. }
            | ExecError::FpClaim { .. }
            | ExecError::Refused { .. }
    )
}

/// The class a door refusal publishes.
fn door_class(e: &ExecError) -> &'static str {
    match e {
        ExecError::CapDenied { .. } => "cap_denied",
        ExecError::BirthRefused { .. } => "birth_refused",
        ExecError::ArmedRefusal { .. } => "armed_refusal",
        ExecError::SectionNotFound { .. } => "section_not_found",
        ExecError::SectionAmbiguous { .. } => "section_ambiguous",
        ExecError::FpClaim { .. } => "fp_claim",
        _ => "refused",
    }
}

/// The `applied[]` rows for a batch that REFUSED.
///
/// The words follow the executor's ACTUAL ordering, which is not uniform and
/// which its own header doc used to deny (now corrected there): births
/// realize FIRST, one door call each in emission order, stopping at the first
/// refusal; the page splice — load, plan, seal, the armed gate, commit — runs
/// after them as ONE atomic batch. So, keyed on
/// [`ExecError::descriptor_index`]:
///
/// | descriptor | word | because |
/// |---|---|---|
/// | a create BEFORE the refused index | `born` | it is on disk; decision #14 does not roll it back |
/// | the descriptor AT the refused index | `refused` + class + reason | the door judged this one |
/// | a create AFTER it | `not_applied` | the loop stopped before reaching it |
/// | every edit | `not_applied` | the splice never ran — a refusal from the birth lane or the armed gate is before the commit |
///
/// A refusal naming NO descriptor renders BY STAGE (advisor `1161daf7`,
/// 2026-08-23), because the splice runs after the birth lane and a uniform
/// `refused` would say a file that exists is not there:
///
/// - **pre-birth** (the workspace lock, taken before anything runs) — every
///   row `not_applied`;
/// - **post-birth, no descriptor named** (the page load, splice I/O) — every
///   create `born`, because the birth lane ran to completion (had it refused,
///   the refusal would carry that birth's index), and every edit
///   `not_applied`.
///
/// `refused` stays RESERVED for the descriptor a door judged.
fn refused_rows(effects: &[Effect], e: &ExecError, class: &str, reason: &str) -> Vec<Value> {
    let culprit = e.descriptor_index();
    effects
        .iter()
        .enumerate()
        .map(|(i, effect)| {
            let mut row = json!({
                "kind": effect.kind.as_str(),
                "args": args_of(effect),
            });
            let Some(culprit) = culprit else {
                // NO descriptor is named. Rendering every row `refused` here
                // would re-admit the falsehood the positional rule removes:
                // the page load and the splice run AFTER the birth lane, so a
                // page that will not load fails with every birth ALREADY ON
                // DISK. Render BY STAGE, which the variant knows:
                if pre_birth(e) {
                    // The workspace lock is taken before anything runs.
                    row["result"] = json!("not_applied");
                    row["reason"] = json!("refused before the batch began");
                } else if effect.kind == effects::EffectKind::Create {
                    // realize_births ran to completion — had it refused, the
                    // refusal would carry that birth's index — so every create
                    // landed.
                    row["result"] = json!("born");
                    row["reason"] = json!("the birth lane completed; the failure is after it");
                } else if is_door_refusal(e) {
                    // A door judged the SPLICE as a whole (an armed veto, a
                    // verdict): the edits are what it judged.
                    row["result"] = json!("refused");
                    row["class"] = json!(class);
                    row["reason"] = json!(reason);
                } else {
                    row["result"] = json!("not_applied");
                    row["reason"] = json!("the page splice never ran");
                }
                return row;
            };
            if i == culprit {
                row["result"] = json!("refused");
                row["class"] = json!(class);
                row["reason"] = json!(reason);
            } else if effect.kind == effects::EffectKind::Create && i < culprit {
                // ON DISK. Saying `not_applied` about a record that exists is
                // the falsehood the ordering makes possible, and the reason
                // this table is keyed on the index rather than on the verb.
                row["result"] = json!("born");
                row["reason"] = json!(
                    "committed before the refusal — the birth lane realizes in emission \
                     order and does not roll back"
                );
            } else {
                row["result"] = json!("not_applied");
                row["reason"] = json!(if effect.kind == effects::EffectKind::Create {
                    "the birth lane stopped at the refusal before reaching this descriptor"
                } else {
                    "the page splice never ran — one atomic batch, refused before commit"
                });
            }
            row
        })
        .collect()
}

/// One realized effect as its `applied[]` row.
///
/// `file_rev` names the rev of **the record this row touched** — for a birth
/// that is the BORN file, not the page. The batch's own `file_rev_after` is
/// the page's, and reporting it on a birth row would put a plausible,
/// unrelated hash where a caller expects the thing it just created. The born
/// rev costs one read of a file we just wrote, and a rev nobody can act on
/// costs more.
fn applied_row(world: &ModeWorld<'_>, effect: &Effect, applied: &Applied) -> Value {
    let mut row = json!({
        "kind": effect.kind.as_str(),
        // The effect's OWN word. It used to be the literal "born" for every
        // kind, so a `set_field` — an edit, not a birth — reported `born`
        // while the same function's next arm published the PAGE's rev for it,
        // and the word and the rev disagreed inside one row.
        // (PR 195 review, e9f1ae35, F5a.)
        "result": result_word(effect.kind),
        "args": args_of(effect),
    });
    let born_path = (effect.kind == effects::EffectKind::Create)
        .then(|| effect.args.get("path"))
        .flatten()
        .and_then(|p| match p {
            effects::ArgValue::Str(path) => Some(path.clone()),
            // A birth's `path` is a scalar by the constructor's own contract;
            // the other shapes are not a landing coordinate, and an absent
            // rev is the honest answer for a row we cannot read back.
            effects::ArgValue::List(_) | effects::ArgValue::Map(_) => None,
        });
    match born_path {
        Some(path) => {
            row["path"] = json!(path);
            // Absent rather than wrong: a birth whose rev cannot be read back
            // says nothing about it, which is a fact a caller can handle. A
            // borrowed page rev is one it cannot.
            if let Ok(doc) = crate::address::load_page(world.root, Path::new(&path)) {
                row["file_rev"] = json!(doc.root.node_rev.0);
            }
        }
        // An edit touched the page, so the page's post-apply rev IS this
        // row's rev.
        None => row["file_rev"] = json!(applied.file_rev_after),
    }
    row
}

/// The page's authority: its `caps:` narrowed by the declaring root's
/// conventions.
fn page_authority(world: &ModeWorld<'_>) -> Result<Authority, caps::CapsError> {
    let page_caps = caps::page_caps(world.doc)?;
    let (conventions, _) = caps::load_conventions(world.declaring_root)?;
    Ok(caps::resolve_page_authority(
        page_caps.as_ref(),
        &conventions,
    ))
}

/// The fire phase's process seam: `bash()` through the run plane's own
/// bracket (§ 1.3, § 2.2 step 5).
///
/// **`bash` never raises for the process's sake** — § 1.3's law. An
/// unstartable process is `exit: 127`, a timeout is `timed_out: True` with
/// `exit: 137`, a dangling `block=` anchor is `exit: 127` with `stderr`
/// naming it. The program branches on a row; it does not handle an exception
/// it has no syntax for.
struct ProcessSeam<'a> {
    /// The page — for `bash(block=)`, which resolves an anchor on THIS page.
    doc: &'a Document,
    /// The workspace root: the default cwd, the scratch dir, and
    /// `$MERIDIAN_PROJECT_ROOT`.
    root: &'a Path,
    /// The page's workspace-relative path — the log directory's own key, so
    /// retention is *per page* as § 2.2 states it.
    page: &'a str,
    /// The per-target invocation (`<invocation>-t<index>`, minted by the
    /// caller): the log's NAME, which is what lets a row be joined to the
    /// daemon's journal — the reason § 2.2 gives for `invocation` riding a
    /// fire at all.
    invocation: &'a str,
    /// Every call this fire made, in call order — the fire row's `exec[]`.
    /// A `Mutex` rather than a `RefCell` because [`FireHost`] is `Sync`: the
    /// kernel holds the seam across its evaluation thread. It is never
    /// contended — the evaluator is single-threaded — so the lock costs
    /// nothing and satisfies the bound honestly.
    calls: std::sync::Mutex<Vec<Value>>,
    /// The wall-clock ceiling, resolved once from the declaring root.
    timeout: std::time::Duration,
    /// Where a `dry` fire stops: the stub says so on its own row, and § 1.3
    /// is explicit that a decision reached under `dry` is a rehearsal.
    dry: bool,
}

impl FireHost for ProcessSeam<'_> {
    fn bash(&self, call: &effects::BashCall) -> Result<Value, String> {
        let seen = self.run(call);
        if let Ok(mut calls) = self.calls.lock() {
            // The call's position in the fire — the log name's `-b<n>`, so two
            // calls of one target cannot write one file over the other.
            let index = calls.len();
            calls.push(self.published(call, &seen, index));
        }
        Ok(seen)
    }
}

impl ProcessSeam<'_> {
    /// The `exec[]` rows this fire produced, in call order.
    fn rows(&self) -> Vec<Value> {
        self.calls.lock().map(|c| c.clone()).unwrap_or_default()
    }

    /// Keep the newest `keep` files with this extension in `dir` and drop the
    /// rest — the ceiling § 2.2 names for logs (*"retention is the last 50 per
    /// page, none under `dry`, named so that tool-call cadence has a
    /// ceiling"*), and the same one this implementation puts on staged bytes,
    /// because a cache keyed by an ever-growing set of revs is the same
    /// unbounded cost with a longer fuse.
    ///
    /// `dir` is always a leaf the FIRE path owns — one page's log directory,
    /// or the staged-bytes directory — never `.meridian/runs/` itself: the
    /// task path's own logs live at that top level and a run receipt POINTS
    /// AT them, so a prune that could reach them would delete the evidence a
    /// receipt promises.
    ///
    /// Best-effort by construction: a file that cannot be removed is not a
    /// reason to fail a fire that already ran.
    fn prune(dir: &Path, ext: Option<&str>, keep: usize) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut files: Vec<(std::time::SystemTime, std::path::PathBuf)> = entries
            .filter_map(Result::ok)
            // `None` counts every file in the leaf — the staged directory's
            // members carry the fence's own extension, so filtering on one
            // would leave every other language unbounded.
            .filter(|e| {
                ext.is_none_or(|ext| {
                    e.path()
                        .extension()
                        .is_some_and(|found| found.eq_ignore_ascii_case(ext))
                })
            })
            .filter_map(|e| {
                let modified = e.metadata().ok()?.modified().ok()?;
                Some((modified, e.path()))
            })
            .collect();
        if files.len() <= keep {
            return;
        }
        files.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
        for (_, path) in files.drain(keep..) {
            let _ = std::fs::remove_file(path);
        }
    }

    /// This page's log directory — `.meridian/runs/<page-path>/`.
    ///
    /// The page is a DIRECTORY, not a fragment of a file name, which is what
    /// makes *"the last 50 per page"* implementable at all: retention is
    /// [`Self::prune`] over this one leaf.
    fn log_dir(&self) -> std::path::PathBuf {
        self.root.join(RUNS_DIR).join(self.page)
    }

    /// The PUBLISHED row for one `bash()` call — deliberately **not** the dict
    /// the program saw.
    ///
    /// The program's dict carries `stdout`/`stderr` inline, because a program
    /// branching on its own command's output is the entire reason `bash()`
    /// returns a value (§ 1.4's cadence rule). The published row must not:
    /// § 2.2 states the recording is *"stdout by sha + a log path, not inline
    /// in the trace"*, with logs out of tree, *"named so that tool-call
    /// cadence has a ceiling"*. Publishing the program's dict put a chatty
    /// hook's entire stdout on the wire, into the daemon's journal and into an
    /// agent's context on EVERY fire — the unbounded per-tool-call cost the
    /// ceiling exists to prevent. (PR 195 review, e9f1ae35, F8.)
    ///
    /// `log` is written under `.meridian/runs/<page>/`, never in the tree, and
    /// never under `dry` — a rehearsal spawns nothing, so there is nothing to
    /// record. `index` is this call's position in the fire, which is what
    /// keeps several `bash()` calls of one target from writing one file over
    /// another.
    fn published(&self, call: &effects::BashCall, seen: &Value, index: usize) -> Value {
        let stdout = seen["stdout"].as_str().unwrap_or_default();
        let stderr = seen["stderr"].as_str().unwrap_or_default();
        let dry = seen["dry"].as_bool().unwrap_or(false);
        let mut row = json!({
            "command": seen["command"].clone(),
            "exit": seen["exit"].clone(),
            "stdout_sha256": format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(stdout.as_bytes())),
            "bytes": stdout.len(),
            "timed_out": seen["timed_out"].clone(),
            "dry": dry,
        });
        // `block` when the call named one — the fact that says WHICH fence
        // ran, which a bare command string cannot.
        if let effects::ExecProgram::Block(anchor) = &call.program {
            row["block"] = json!(anchor);
        }
        if !dry
            && (!stdout.is_empty() || !stderr.is_empty())
            && let Some(path) = self.write_log(&format!("{}-b{index}", self.invocation), stdout, stderr)
        {
            row["log"] = json!(path);
        }
        row
    }

    /// Write one process's streams to its out-of-tree log, and answer the
    /// workspace-relative path. `None` when the write fails: a row that cannot
    /// point at a log says nothing rather than pointing at a path that is not
    /// there.
    ///
    /// The name is the caller's `stem` — `<invocation>-t<index>` for an exec'd
    /// entry, `<invocation>-t<index>-b<n>` for the n-th `bash()` call of one
    /// target — under this page's own directory, so a row can be joined to the
    /// daemon's journal by the id it already carries and retention stays per
    /// page.
    fn write_log(&self, stem: &str, stdout: &str, stderr: &str) -> Option<String> {
        let dir = self.log_dir();
        std::fs::create_dir_all(&dir).ok()?;
        let name = format!("{stem}.log");
        let body = if stderr.is_empty() {
            stdout.to_owned()
        } else {
            format!("{stdout}\n--- stderr ---\n{stderr}")
        };
        std::fs::write(dir.join(&name), body).ok()?;
        Self::prune(&dir, Some("log"), LOG_RETENTION);
        Some(format!("{RUNS_DIR}/{}/{name}", self.page))
    }

    /// Stage an exec'd entry's bytes to a file and answer its path — the
    /// design's *"staged bytes cached by block rev"*.
    ///
    /// `key` is the block's own rev (or, for an inline `cmd`, a digest of the
    /// bytes), so the staged file IS the cache: a rev names one immutable set
    /// of bytes, and a second fire of an unchanged block finds the file
    /// already there and writes nothing. `ext` is the fence's own info-string
    /// token when it has one, because a loader-by-extension interpreter (bun,
    /// deno) reads the language off the name — `fence.rs` is untouched (D5):
    /// this reads the FIRST token, the one classifier that already exists.
    ///
    /// # Errors
    /// The staging write failed — reported as the engine's own failure, never
    /// as an exit code the program did not produce.
    fn stage(&self, source: &str, key: &str, ext: Option<&str>) -> Result<std::path::PathBuf, String> {
        let dir = self.root.join(STAGED_DIR);
        std::fs::create_dir_all(&dir).map_err(|e| format!("stage {STAGED_DIR}: {e}"))?;
        let name = match ext {
            Some(ext) => format!("{key}.{ext}"),
            None => key.to_owned(),
        };
        let path = dir.join(&name);
        // Write only when it is not already there: the key is the content's
        // own identity, so an existing file has the bytes we were about to
        // write. This is the cache — it removes the write, and (with the
        // pinned page in memory) the read; § 1.4 is explicit that it never
        // removes the spawn.
        if !path.exists() {
            std::fs::write(&path, source).map_err(|e| format!("stage {name}: {e}"))?;
        }
        Self::prune(&dir, None, STAGED_RETENTION);
        Ok(path)
    }

    /// One `bash()` call → its row. Never `Err`: every failure mode of a
    /// process is a value the program can read.
    fn run(&self, call: &effects::BashCall) -> Value {
        // Under `dry` nothing is spawned and the row SAYS so, so a program
        // that branched on it cannot be mistaken for one that decided.
        if self.dry {
            return json!({
                "command": self.command_of(call).unwrap_or_default(),
                "exit": 0, "stdout": "", "stderr": "",
                "timed_out": false, "dry": true,
            });
        }
        let source = match self.command_of(call) {
            Ok(source) => source,
            // A dangling `block=` anchor is the "could not start" class, by
            // § 1.3's own words — not a fault, not a silent skip.
            Err(reason) => {
                return json!({
                    "command": "", "exit": 127, "stdout": "", "stderr": reason,
                    "timed_out": false, "dry": false,
                });
            }
        };
        let scratch = self.root.join(".meridian/scratch");
        let _ = std::fs::create_dir_all(&scratch);
        // § 2.2's cwd rule: the call's own `cwd` when it named one, else the
        // page's root — the official contract runs a hook in the project dir.
        let cwd = call
            .cwd
            .as_ref()
            .map_or_else(|| self.root.to_path_buf(), |c| self.root.join(c));
        let timeout = call
            .timeout_s
            // A caller ceiling NARROWS; it never raises the root's.
            .map_or(self.timeout, |s| {
                self.timeout.min(std::time::Duration::from_secs(s))
            });
        let spec = crate::exec::ExecSpec {
            // `bash()` IS bash by its own name, so `-c` is honest here and the
            // staged-file convention buys nothing: the interpreter is not a
            // parameter of this call (§ 1.3).
            program: crate::exec::Program::Inline(&source),
            args: &[],
            env: &call.env,
            scratch: &scratch,
            project_root: self.root,
            timeout,
            step_cwd: Some(&cwd),
            interpreter: crate::exec::BASH,
            stdin: call.stdin.as_deref(),
        };
        match crate::exec::exec(&spec) {
            Ok(result) => {
                let (exit, timed_out) = match result.status {
                    crate::exec::ExecStatus::Exited { code } => (code, false),
                    // A signal is not an exit code; 128+n is the shell's own
                    // convention and the one a script author reads.
                    crate::exec::ExecStatus::Signaled { signal } => (128 + signal, false),
                    crate::exec::ExecStatus::TimedOut { .. } => (137, true),
                };
                json!({
                    "command": source,
                    "exit": exit,
                    "stdout": String::from_utf8_lossy(&result.stdout),
                    "stderr": String::from_utf8_lossy(&result.stderr),
                    "timed_out": timed_out,
                    "dry": false,
                })
            }
            // Unstartable is 127 — the "could not start" class, never a raise.
            Err(e) => json!({
                "command": source, "exit": 127, "stdout": "",
                "stderr": e.to_string(), "timed_out": false, "dry": false,
            }),
        }
    }

    /// One declared exec'd entry → its `process` object (§ 1.4).
    ///
    /// The `interpreter` is `argv[0]`, the `args` follow the program, and the
    /// **raw exit** is surfaced: the run plane collapses 1 and 2 into `state:
    /// partial`, and the official hook contract needs them apart — *exit 2 →
    /// stderr's first line is the reason* is unconstructible otherwise.
    ///
    /// The bytes are **staged to a file** and the process is `<interpreter>
    /// <staged-file> <args…>` ([`crate::exec::Program::Staged`]) — the one
    /// convention every interpreter honours, and the only way § 1.4's *"a new
    /// language is `argv[0]`"* is true of anything but a shell.
    fn exec(
        &self,
        spec: &effects::ExecSpec,
        stdin: Option<&str>,
        block_anchor: &str,
        target: &wire::RunTarget,
    ) -> Result<Value, (&'static str, String)> {
        let invocation = self.invocation;
        let page = self.page;
        // Judged FIRST: a rehearsal spawns nothing, so it also stages nothing
        // and writes no log — `dry` must leave the disk as it found it.
        if self.dry {
            return Ok(json!({
                "interpreter": spec.interpreter, "exit": 0,
                "stdout_tail": "", "stderr_tail": "",
                "stdout_bytes": 0, "stderr_bytes": 0,
                "timed_out": false, "dry": true,
            }));
        }
        // A dangling `exec(block=)` anchor is a DECLARATION defect, and § 2.2
        // resolves it at LOAD — both `load_row` and `addressed_entry` do that
        // now, so reaching this arm means the page changed under us. It is
        // still an inability to start, never a script's exit: it answers a
        // typed refusal that keeps the anchor error's own words.
        let staged = match &spec.program {
            // An inline `cmd` has no rev of its own, so the cache is keyed on
            // the bytes; a block is keyed on the rev the read face publishes.
            effects::ExecProgram::Cmd(cmd) => {
                self.stage(cmd, &blake3::hash(cmd.as_bytes()).to_hex()[..16], None)
            }
            effects::ExecProgram::Block(anchor) => match blocks::block(self.doc, anchor) {
                Ok(block) => {
                    let ext = fence_extension(&block);
                    self.stage(&block.source, &block.rev, ext)
                }
                Err(e) => return Err(("no_block", e.to_string())),
            },
        };
        let staged = match staged {
            Ok(path) => path,
            // Nothing ran, and no exit code is invented for it: staging is the
            // ENGINE's step, so its failure is the engine's to report.
            Err(reason) => return Err(("runtime", reason)),
        };
        let scratch = self.root.join(".meridian/scratch").join(invocation);
        let _ = std::fs::create_dir_all(&scratch);
        // The layering § 2.2 states, in its order: the TARGET's `env` is the
        // base — that is where the daemon's `CCC_HOOK_*` scalars ride, and the
        // engine carries them opaque — and the DECLARED `exec(env=)` pairs
        // overlay it. The target half used to be dropped on the floor: `env`
        // was read zero times, so a `settings.json`-shaped hook reading
        // `$CCC_HOOK_EVENT` saw it unset, took its default branch, and
        // answered `ok`/`exit 0` with nothing anywhere saying a variable had
        // gone missing. (PR 195 review, e9f1ae35, F2.)
        let mut env: BTreeMap<String, String> = target.env.clone();
        env.extend(spec.env.clone());
        // The engine's own two facts. `MRD_RUN_BLOCK` is the BLOCK — it used
        // to carry the invocation, so a script shared by several blocks on one
        // page (the reason the pair exists) branched on a token that differed
        // every call and matched no block name, falling through to its default
        // every time. The invocation is worth publishing too, and now does so
        // under its own honest name. (F4.)
        env.insert("MRD_RUN_PAGE".to_owned(), page.to_owned());
        env.insert("MRD_RUN_BLOCK".to_owned(), block_anchor.to_owned());
        env.insert("MRD_RUN_INVOCATION".to_owned(), invocation.to_owned());
        // § 2.2's cwd rule for an exec'd entry, the same one `bash()` keeps:
        // the input's own `cwd` when it named one, else the page's root — the
        // official contract runs a hook in the project directory. `step_cwd`
        // used to be the root unconditionally, so a hook that asked to run
        // somewhere else was silently run somewhere else than it asked.
        let cwd = target
            .input
            .as_ref()
            .and_then(|input| input.get("cwd"))
            .and_then(Value::as_str)
            .map_or_else(|| self.root.to_path_buf(), |c| self.root.join(c));
        let exec_spec = crate::exec::ExecSpec {
            program: crate::exec::Program::Staged(&staged),
            args: &spec.args,
            env: &env,
            scratch: &scratch,
            project_root: self.root,
            timeout: self.timeout,
            step_cwd: Some(&cwd),
            interpreter: &spec.interpreter,
            stdin,
        };
        let answer = match crate::exec::exec(&exec_spec) {
            Ok(result) => {
                let (exit, timed_out) = match result.status {
                    crate::exec::ExecStatus::Exited { code } => (code, false),
                    crate::exec::ExecStatus::Signaled { signal } => (128 + signal, false),
                    crate::exec::ExecStatus::TimedOut { .. } => (137, true),
                };
                // Tails, and the whole streams in the log the row points at —
                // § 1.4 names the out-of-tree log as one of the three things
                // that BOUND an exec'd entry, and until now it had none.
                let (stdout_tail, stdout_bytes) = tail(&result.stdout);
                let (stderr_tail, stderr_bytes) = tail(&result.stderr);
                let mut process = json!({
                    "interpreter": spec.interpreter,
                    "exit": exit,
                    "stdout_tail": stdout_tail,
                    "stderr_tail": stderr_tail,
                    "stdout_bytes": stdout_bytes,
                    "stderr_bytes": stderr_bytes,
                    "timed_out": timed_out,
                });
                if stdout_bytes + stderr_bytes > 0
                    && let Some(path) = self.write_log(
                        invocation,
                        &String::from_utf8_lossy(&result.stdout),
                        &String::from_utf8_lossy(&result.stderr),
                    )
                {
                    process["log"] = json!(path);
                }
                Ok(process)
            }
            // The process never started. That is the engine's failure to
            // report, not an exit code to invent: a fabricated `127` here is
            // indistinguishable from a script whose binary was off `PATH`.
            Err(e) => Err(("no_block", e.to_string())),
        };
        let _ = std::fs::remove_dir_all(&scratch);
        answer
    }

    /// The bytes to run: inline `cmd`, or the source of the `^id` block on
    /// this page.
    fn command_of(&self, call: &effects::BashCall) -> Result<String, String> {
        match &call.program {
            effects::ExecProgram::Cmd(cmd) => Ok(cmd.clone()),
            effects::ExecProgram::Block(anchor) => blocks::block(self.doc, anchor)
                .map(|block| block.source)
                .map_err(|e| e.to_string()),
        }
    }
}

/// One block's fault as a row.
fn fault_row(
    target: &wire::RunTarget,
    invocation: &str,
    block: &AnchoredBlock,
    fault: &BlockFault,
) -> Value {
    json!({
        "page": target.page,
        "invocation": invocation,
        "block": block.anchor,
        "rev": {"block": block.rev},
        "result": "fault",
        "fault": fault_object(fault),
    })
}

/// A classified fault as the row's `fault` object.
fn fault_object(fault: &BlockFault) -> Value {
    let mut out = json!({"class": fault.class.as_str(), "reason": fault.reason});
    if let Some(line) = fault.line {
        out["line"] = json!(line);
    }
    out
}

/// A refusal row: the target's addressing plus the typed cause.
fn refused_row(
    target: &wire::RunTarget,
    invocation: &str,
    class: &str,
    reason: &str,
    line: Option<u32>,
) -> Value {
    let mut fault = json!({"class": class, "reason": reason});
    if let Some(line) = line {
        fault["line"] = json!(line);
    }
    let mut row = json!({
        "page": target.page,
        "invocation": invocation,
        "result": "refused",
        "fault": fault,
    });
    if let Some(block) = &target.block {
        row["block"] = json!(block);
    }
    row
}

/// A load fault for a declaration whose `exec(block=)` addresses no block —
/// **§ 2.2 step 2: the anchor resolves at LOAD**, dangling → `no_block`,
/// duplicate → `ambiguous_anchor`, load faults both.
///
/// The declaration IS the program (§ 1.4), so a declaration naming a fence
/// that is not there is broken the moment it is READ, and a resolver loading a
/// page to decide what to arm must learn that from the load rather than from
/// the first fire. Until this card it was resolved on the FIRE door alone, so
/// a load answered `result: "ok"` for a hook that could never run — while
/// `docs/run-plane.md` and `effects::kernel::ExecProgram::Block`'s own doc
/// both already stated the load rule. A doc claiming a refusal the tree does
/// not make is the F2 class.
fn exec_anchor_fault(doc: &Document, block: &AnchoredBlock, loaded: &LoadedBlock) -> Option<Value> {
    for declaration in &loaded.declarations {
        let DeclaredEntry::Exec(spec) = &declaration.entry else {
            continue;
        };
        let effects::ExecProgram::Block(anchor) = &spec.program else {
            continue;
        };
        let Err(e) = blocks::block(doc, anchor) else {
            continue;
        };
        return Some(json!({
            "block": block.anchor,
            "rev": block.rev,
            "result": "fault",
            "entry_kind": "exec",
            "fault": {
                "class": e.class(),
                // The anchor error's OWN words: "no such block" and "is minted
                // N times, so it addresses none of them" are different
                // repairs, and a reason that flattened them would send the
                // author looking for the wrong one.
                "reason": format!(
                    "^{} declares `exec(block = \"{anchor}\")`, which addresses no block: {e}",
                    block.anchor
                ),
            },
        }));
    }
    None
}

/// The staged file's extension — the fence's own info-string token.
///
/// bun and deno choose a loader from the file NAME, so a `ts` block must stage
/// as `.ts` or the interpreter refuses bytes it could have run. `fence.rs` is
/// untouched (D5): this reads the FIRST token, the classifier that already
/// exists, and invents no second-token label mechanism.
fn fence_extension(block: &AnchoredBlock) -> Option<&str> {
    match &block.lang {
        Ok(crate::fence::TaskLanguage::Bash) => Some("bash"),
        Ok(crate::fence::TaskLanguage::Starlark) => Some("star"),
        // The classifier refused, and its refusal CARRIES the token: a page
        // may fence `ts`, `py`, `rb` — the plane dispatches on none of them,
        // and an exec'd entry names its own interpreter anyway.
        Err(crate::fence::FenceError::UnknownLanguage { lang }) => extension_token(lang),
        Err(_) => None,
    }
}

/// A token safe to hang on a file name: ASCII alphanumeric, at most 12 bytes.
/// Anything else stages without an extension rather than putting a page's
/// bytes into a path component nobody vetted.
fn extension_token(token: &str) -> Option<&str> {
    (!token.is_empty() && token.len() <= 12 && token.bytes().all(|b| b.is_ascii_alphanumeric()))
        .then_some(token)
}

/// The anchor a block refusal is about.
fn anchor_of(error: &BlockError) -> &str {
    match error {
        BlockError::NoBlock { anchor }
        | BlockError::AmbiguousAnchor { anchor, .. }
        | BlockError::NotACodeBlock { anchor } => anchor,
    }
}

/// One descriptor's arguments as JSON — what the `applied[]` row shows so a
/// caller can see WHICH write landed, not just that one did.
fn args_of(effect: &Effect) -> Value {
    let mut out = serde_json::Map::new();
    for (key, value) in &effect.args {
        // `ArgValue` serializes untagged — a scalar as a string, a list as
        // an array — so the row shows the argument as the author wrote it.
        out.insert(key.clone(), json!(value));
    }
    Value::Object(out)
}

/// The eval context one mode-bearing target runs under.
fn ctx_for(target: &wire::RunTarget, invocation: &str) -> RunCtx {
    RunCtx {
        page: target.page.clone(),
        // A declared block has no task name; the block anchor identifies it,
        // and the kernel uses this only as the module id and the emitting
        // rule id.
        task: target.block.clone().unwrap_or_else(|| target.page.clone()),
        args: Vec::new(),
        env: BTreeMap::new(),
        invocation_id: invocation.to_owned(),
        root_at_eval: String::new(),
    }
}

/// Whether this target takes the mode path at all.
#[must_use]
pub fn is_mode_target(target: &wire::RunTarget) -> bool {
    target.mode.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One md descriptor, for the row-rendering tests.
    fn effect(kind: effects::EffectKind, path: &str) -> Effect {
        let mut args = BTreeMap::new();
        args.insert("path".to_owned(), effects::ArgValue::Str(path.to_owned()));
        Effect {
            kind,
            rule_id: "probe".to_owned(),
            seq: 0,
            depth: 0,
            provenance: effects::Provenance::Run {
                invocation_id: "probe".to_owned(),
                root_at_eval: String::new(),
            },
            args,
        }
    }

    /// **The by-stage rendering, post-birth half** (advisor `1161daf7`,
    /// 2026-08-23; PR 195 pass-2 fixture 3).
    ///
    /// The page load runs at `apply_under` step 2 — AFTER the birth lane — so
    /// a page that will not load fails with every birth already on disk. The
    /// rows must say `born` for those creates, not `refused` and not
    /// `not_applied`: both would say a file that exists is not there.
    ///
    /// Aperture: this drives the RENDERER with the executor's own `Page`
    /// variant, not a live page-load failure — the CLI loads the page before
    /// the apply, so a test cannot make step 2 fail without racing it. What it
    /// proves is the mapping from variant to words; what it does not prove is
    /// that `apply_under` returns `Page` there, which the executor's own
    /// ordering (and its corrected doc) states.
    #[test]
    fn a_post_birth_failure_naming_no_descriptor_still_says_born() {
        let effects = vec![
            effect(effects::EffectKind::Create, "born/one.md"),
            effect(effects::EffectKind::SetField, "page.md"),
            effect(effects::EffectKind::Create, "born/two.md"),
        ];
        let e = ExecError::Page {
            path: "probe.md".to_owned(),
            reason: "no such file".to_owned(),
        };
        assert_eq!(e.descriptor_index(), None, "this variant names none");
        let rows = refused_rows(&effects, &e, "runtime", "page load failed");

        assert_eq!(rows[0]["result"], "born", "{rows:#?}");
        assert_eq!(rows[2]["result"], "born", "{rows:#?}");
        assert_eq!(rows[1]["result"], "not_applied", "the splice never ran");
        assert!(
            rows.iter().all(|r| r["result"] != "refused"),
            "`refused` is reserved for the descriptor a door judged: {rows:#?}"
        );
    }

    /// **The by-stage rendering, pre-birth half**: the workspace lock is taken
    /// before any descriptor is looked at, so nothing ran and nothing landed.
    #[test]
    fn a_pre_birth_failure_says_not_applied_for_everything() {
        let effects = vec![
            effect(effects::EffectKind::Create, "born/one.md"),
            effect(effects::EffectKind::SetField, "page.md"),
        ];
        let rows = refused_rows(
            &effects,
            &ExecError::WorkspaceBusy,
            "runtime",
            "another run holds the lock",
        );
        assert!(
            rows.iter().all(|r| r["result"] == "not_applied"),
            "{rows:#?}"
        );
    }

    /// A probe cache that counts what it was asked to keep.
    #[derive(Default)]
    struct ProbeCache {
        blocks: std::sync::Mutex<BTreeMap<String, std::sync::Arc<LoadedBlock>>>,
        misses: std::sync::atomic::AtomicUsize,
    }

    impl ModuleCache for ProbeCache {
        fn get(&self, key: &str) -> Option<std::sync::Arc<LoadedBlock>> {
            let hit = self
                .blocks
                .lock()
                .expect("probe cache")
                .get(key)
                .map(std::sync::Arc::clone);
            if hit.is_none() {
                self.misses
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            hit
        }

        fn put(&self, key: String, loaded: std::sync::Arc<LoadedBlock>) {
            self.blocks.lock().expect("probe cache").insert(key, loaded);
        }
    }

    fn probe_block() -> AnchoredBlock {
        AnchoredBlock {
            anchor: "h".to_owned(),
            rev: "rev-1".to_owned(),
            lang: Ok(crate::fence::TaskLanguage::Starlark),
            source: "def run(event):\n    return 1\n\ndeclare(on = \"Stop\")\n".to_owned(),
            task: None,
        }
    }

    fn probe_world<'a>(
        doc: &'a Document,
        root: &'a fs::WorkspaceRoot,
        fp: &'a MerkleRoot,
        prelude: Option<&'a str>,
        cache: Option<&'a dyn ModuleCache>,
    ) -> ModeWorld<'a> {
        ModeWorld {
            doc,
            root,
            declaring_root: None,
            observed_root: fp,
            prelude,
            doors: Doors::default(),
            cache,
        }
    }

    /// A page that really binds a task AND really declares — so a test naming
    /// the task guard exercises the task guard, and a test naming a declaring
    /// page has one. `empty_doc()`'s `# probe\n` has neither, which is what
    /// made two of my fixtures the undeclared-page case wearing other names.
    /// (PR 195 pass 2, `fa5da9ec`, P2-4/P2-5.)
    fn task_bound_doc() -> Document {
        let raw = "\
---
task.arm: \"[[#^armer]]\"
---

# Page

```starlark
def run(ctx):
    pass
```
^armer

```starlark
def run(event):
    return {\"saw\": event[\"name\"]}

declare(on = \"Stop\")
```
^hook
"
        .to_owned();
        let nodes = syntax::parse(&raw);
        model::build(raw, nodes)
    }

    fn empty_doc() -> Document {
        let raw = "# probe\n".to_owned();
        let nodes = syntax::parse(&raw);
        model::build(raw, nodes)
    }

    /// **S3's FIRE-path fixture** (reviewer `fa5da9ec`, semantic (c)): a fire
    /// on a page that declares NOTHING, with a prelude carrying a process
    /// declaration.
    ///
    /// This is the bypass itself. It is closed by the F9 hoist — the prelude
    /// is checked in `mode_row`, which BOTH modes go through, not in
    /// `load_row` where it used to sit — so the fire refuses before
    /// `addressed_entry` ever reads `declarations.first()`, and no process is
    /// started. S3 and F9 are one change.
    #[test]
    fn a_declaring_prelude_cannot_hijack_a_fire_on_an_undeclared_page() {
        let doc = empty_doc();
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = fs::WorkspaceRoot(tmp.path().to_path_buf());
        let fp = MerkleRoot(String::new());
        // The reviewer's own one-liner.
        let prelude = "declare(impl = exec(\"bash\", cmd = \"id\"))\n";
        let world = probe_world(&doc, &root, &fp, Some(prelude), None);
        let target = wire::RunTarget {
            page: "probe.md".to_owned(),
            block: Some("bare".to_owned()),
            mode: Some(wire::RunMode::Fire),
            ..wire::RunTarget::task_target(
                "probe.md".to_owned(),
                None,
                Vec::new(),
                BTreeMap::new(),
                None,
            )
        };

        let row = mode_row(&world, &target, "probe-fire", None, None);
        assert_eq!(row["result"], "refused", "{row:#}");
        assert_eq!(row["fault"]["class"], "prelude_invalid", "{row:#}");
        // Nothing ran: no process row, no effect row, no value.
        assert!(row["process"].is_null(), "a process was started: {row:#}");
        assert!(row["exec"].is_null(), "{row:#}");
        assert!(row["value"].is_null(), "{row:#}");
    }

    /// **S3's third fixture** (reviewer `fa5da9ec` B2): a TASK-BOUND block
    /// plus a DECLARING prelude.
    ///
    /// This is why the prelude guard refuses regardless of what the page
    /// declares. A task-bound block has no `declare()` of its own, so a
    /// narrow guard ("refuse the prelude's declaration only when the page
    /// declares one") would leave a caller free to hijack it into a fire with
    /// a prelude-supplied declaration. Here the prelude is refused before the
    /// page is consulted at all, and nothing fires.
    #[test]
    fn a_declaring_prelude_cannot_hijack_a_task_bound_block() {
        let doc = task_bound_doc();
        // The premise the test rests on: ^armer really IS task-bound. Without
        // this the page has no binding and the test silently becomes the
        // undeclared-page case, which cannot catch the narrow guard.
        let bound = blocks::block(&doc, "armer").expect("^armer is on the page");
        assert_eq!(bound.task.as_deref(), Some("arm"), "the premise");
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = fs::WorkspaceRoot(tmp.path().to_path_buf());
        let fp = MerkleRoot(String::new());
        let prelude = "declare(impl = exec(\"bash\", cmd = \"id\"))\n";
        let world = probe_world(&doc, &root, &fp, Some(prelude), None);
        let target = wire::RunTarget {
            page: "probe.md".to_owned(),
            block: Some("armer".to_owned()),
            mode: Some(wire::RunMode::Fire),
            ..wire::RunTarget::task_target(
                "probe.md".to_owned(),
                None,
                Vec::new(),
                BTreeMap::new(),
                None,
            )
        };

        let row = mode_row(&world, &target, "probe-1", None, None);
        assert_eq!(row["result"], "refused", "{row:#}");
        assert_eq!(
            row["fault"]["class"], "prelude_invalid",
            "the prelude is judged BEFORE the page: {row:#}"
        );
        assert!(
            row["fault"]["reason"]
                .as_str()
                .unwrap_or_default()
                .contains("consent material"),
            "the reason names which invalidity it was: {row:#}"
        );
        assert!(row["applied"].is_null(), "nothing fired: {row:#}");
    }

    /// **P2-5** — a declaring page with a PURE prelude still fires, asserted
    /// where a prelude can actually be passed.
    ///
    /// This lived in the CLI suite, where `prelude` is `None` by construction
    /// (the argv has no spelling for caller source), so it was a plain fire
    /// wearing the name. Here the prelude is real: helpers only, no
    /// declaration — and the fire answers.
    #[test]
    fn a_declaring_page_with_a_pure_prelude_still_fires() {
        let doc = task_bound_doc();
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = fs::WorkspaceRoot(tmp.path().to_path_buf());
        let fp = MerkleRoot(String::new());
        let world = probe_world(
            &doc,
            &root,
            &fp,
            Some("def helper(x):\n    return x\n"),
            None,
        );
        let target = wire::RunTarget {
            page: "probe.md".to_owned(),
            block: Some("hook".to_owned()),
            mode: Some(wire::RunMode::Fire),
            input: Some(json!({"name": "Stop"})),
            ..wire::RunTarget::task_target(
                "probe.md".to_owned(),
                None,
                Vec::new(),
                BTreeMap::new(),
                None,
            )
        };

        let row = mode_row(&world, &target, "probe-pure", None, None);
        assert_eq!(
            row["result"], "ok",
            "a pure prelude is what a prelude is FOR: {row:#}"
        );
        assert_eq!(row["value"]["saw"], "Stop", "{row:#}");
    }

    /// The second load of an unchanged block is SERVED, not re-evaluated —
    /// § 2.2's "an unchanged block is served from cache, never re-evaluated",
    /// which is the whole reason a warm fire is one function call.
    #[test]
    fn an_unchanged_block_is_served_from_cache() {
        let doc = empty_doc();
        let root = fs::WorkspaceRoot(std::path::PathBuf::from("/nonexistent"));
        let fp = MerkleRoot(String::new());
        let cache = ProbeCache::default();
        let world = probe_world(&doc, &root, &fp, None, Some(&cache));
        let block = probe_block();
        let ctx = RunCtx::default();

        let first =
            load_cached(&world, &block, &ctx, EvalLimits::default()).expect("the block loads");
        let second =
            load_cached(&world, &block, &ctx, EvalLimits::default()).expect("and loads again");

        assert_eq!(
            cache.misses.load(std::sync::atomic::Ordering::Relaxed),
            1,
            "the second load must be a HIT — a miss means nothing was cached"
        );
        assert!(
            std::sync::Arc::ptr_eq(&first, &second),
            "the second load returned a different module, so it was re-evaluated"
        );
        assert_eq!(first.declarations.len(), 1);
    }

    /// A faulted load is NEVER cached: the fault is about the attempt, and a
    /// stored refusal would outlive the cause the author has since fixed.
    #[test]
    fn a_faulted_load_is_not_cached() {
        let doc = empty_doc();
        let root = fs::WorkspaceRoot(std::path::PathBuf::from("/nonexistent"));
        let fp = MerkleRoot(String::new());
        let cache = ProbeCache::default();
        let world = probe_world(&doc, &root, &fp, None, Some(&cache));
        let mut block = probe_block();
        block.source = "bash(cmd = \"true\")\n".to_owned();
        let ctx = RunCtx::default();

        assert!(load_cached(&world, &block, &ctx, EvalLimits::default()).is_err());
        assert!(load_cached(&world, &block, &ctx, EvalLimits::default()).is_err());
        assert_eq!(
            cache.misses.load(std::sync::atomic::Ordering::Relaxed),
            2,
            "a fault was cached and served back"
        );
        assert!(cache.blocks.lock().expect("probe").is_empty());
    }

    /// **The TARGET's `env` is the base, and a declared `exec(env=)` pair
    /// shadows it** (§ 2.2 step 4's layering, in its order). The target half
    /// is where a daemon's `CCC_HOOK_*` scalars ride, carried opaque by the
    /// engine, and it was read zero times until PR 195's review — a
    /// `settings.json`-shaped hook saw `$CCC_HOOK_EVENT` unset, took its
    /// default branch and answered `exit 0` with nothing saying a variable had
    /// gone missing. The fix shipped without a fixture; this is it.
    ///
    /// It lives here rather than in the CLI suite because the CLI has no argv
    /// for a target `env` — the wire lane is where it rides, and `mode_row` is
    /// the row builder that lane calls.
    #[test]
    fn the_targets_env_is_the_base_and_a_declared_pair_shadows_it() {
        let raw = "\
# Env probe

```starlark
declare(on = \"Stop\", impl = exec(\"bash\", block = \"p\", env = {\"OVER\": \"declared\"}))
```
^e

```bash
printf '%s|%s' \"$BASE\" \"$OVER\"
```
^p
"
        .to_owned();
        let doc = model::build(raw.clone(), syntax::parse(&raw));
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = fs::WorkspaceRoot(tmp.path().to_path_buf());
        let fp = MerkleRoot(String::new());
        let world = probe_world(&doc, &root, &fp, None, None);
        let env = BTreeMap::from([
            ("BASE".to_owned(), "target".to_owned()),
            ("OVER".to_owned(), "target".to_owned()),
        ]);
        let target = wire::RunTarget {
            page: "env.md".to_owned(),
            block: Some("e".to_owned()),
            mode: Some(wire::RunMode::Fire),
            input: Some(json!({"name": "Stop"})),
            ..wire::RunTarget::task_target("env.md".to_owned(), None, Vec::new(), env, None)
        };

        let row = mode_row(&world, &target, "probe-env-t0", None, None);
        assert_eq!(row["result"], "ok", "{row:#}");
        assert_eq!(
            row["process"]["stdout_tail"], "target|declared",
            "the target env is the base and the declared pair shadows it: {row:#}"
        );
    }

    /// Retention is **per page**, and it can never reach the TASK path's logs.
    ///
    /// `.meridian/runs/` top level is where `run::record` writes a task's log
    /// and a run receipt POINTS AT it. The fire path writes one directory
    /// below, and the prune runs on that leaf only — so a busy page cannot
    /// delete the evidence another lane's receipt promises.
    #[test]
    fn retention_keeps_the_last_fifty_of_one_page_and_never_a_task_log() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let runs = tmp.path().join(RUNS_DIR);
        let page_dir = runs.join("HOOKS.md");
        std::fs::create_dir_all(&page_dir).expect("page dir");
        // A task log at the top level, receipted by construction.
        std::fs::write(runs.join("inv-task-t0.log"), "task").expect("task log");
        for i in 0..60 {
            std::fs::write(page_dir.join(format!("inv-{i}-t0.log")), "fire").expect("fire log");
            // Distinct mtimes: the prune keeps the NEWEST, and a filesystem
            // whose timestamps collide would make the assertion vacuous.
            std::thread::sleep(std::time::Duration::from_millis(2));
        }

        ProcessSeam::prune(&page_dir, Some("log"), LOG_RETENTION);

        let left = std::fs::read_dir(&page_dir).expect("read back").count();
        assert_eq!(left, LOG_RETENTION, "the ceiling is per page");
        assert!(
            runs.join("inv-task-t0.log").exists(),
            "a per-page prune reached the task path's own log"
        );
        assert!(
            page_dir.join("inv-59-t0.log").exists(),
            "the newest must survive"
        );
        assert!(
            !page_dir.join("inv-0-t0.log").exists(),
            "the oldest must go"
        );
    }

    /// The key is the block's rev PLUS the prelude's digest. Both halves
    /// matter: edited bytes are a different block, and the same bytes under a
    /// different prelude are a different MODULE, because the prelude's
    /// bindings are frozen with it. Keying on the rev alone would serve one
    /// caller the other's environment.
    #[test]
    fn the_key_separates_edited_bytes_and_different_preludes() {
        assert_ne!(
            cache_key("rev-1", None),
            cache_key("rev-2", None),
            "an edited block must be a different key"
        );
        assert_ne!(
            cache_key("rev-1", Some("def deny(r):\n    return r\n")),
            cache_key("rev-1", Some("def allow(r):\n    return r\n")),
            "the same bytes under a different prelude are a different module"
        );
        assert_eq!(
            cache_key("rev-1", Some("x = 1\n")),
            cache_key("rev-1", Some("x = 1\n")),
            "the key must be stable for the same inputs"
        );
        assert_ne!(cache_key("rev-1", None), cache_key("rev-1", Some("")));
    }
}
