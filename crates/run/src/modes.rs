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

/// How many exec logs a workspace keeps (§ 2.2's stated ceiling).
const LOG_RETENTION: usize = 50;

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
    let loaded = match load_cached(world, block, &ctx_for(target, invocation), eval_limits(target)) {
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
    let ctx = ctx_for(target, invocation);
    let loaded = load_cached(world, &block, &ctx, eval_limits(target))
        .map_err(|fault| fault_row(target, invocation, &block, &fault))?;
    let Some(declaration) = loaded.declarations.first() else {
        return Err(refused_row(
            target,
            invocation,
            "not_declared",
            &match &block.task {
                Some(task) => format!(
                    "^{anchor} is bound to task `{task}`: address it as a task target \
                     (`task`), not as a fire — the two addressings are exclusive"
                ),
                None => format!(
                    "^{anchor} declares nothing: `run` executes what the page declares — a \
                     `task.<name>` binding in frontmatter, or `declare(...)` in the block"
                ),
            },
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
    let process = match seam.exec(
        spec,
        stdin.as_deref(),
        invocation,
        &target.page,
        &block.anchor,
        target,
    ) {
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
                    row["reason"] =
                        json!("the birth lane completed; the failure is after it");
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
            calls.push(self.published(call, &seen));
        }
        Ok(seen)
    }
}

impl ProcessSeam<'_> {
    /// The `exec[]` rows this fire produced, in call order.
    fn rows(&self) -> Vec<Value> {
        self.calls.lock().map(|c| c.clone()).unwrap_or_default()
    }

    /// Keep the newest [`LOG_RETENTION`] exec logs and drop the rest.
///
/// § 2.2 names the ceiling — *"retention is the last 50 per page, none under
/// `dry`, named so that tool-call cadence has a ceiling"* — and a log
/// directory that only ever grows is the unbounded cost with a longer fuse.
/// Best-effort by construction: a log that cannot be removed is not a reason
/// to fail a fire that already ran.
fn prune_logs(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut logs: Vec<(std::time::SystemTime, std::path::PathBuf)> = entries
        .filter_map(Result::ok)
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.starts_with("exec-") && n.ends_with(".log"))
        })
        .filter_map(|e| {
            let modified = e.metadata().ok()?.modified().ok()?;
            Some((modified, e.path()))
        })
        .collect();
    if logs.len() <= LOG_RETENTION {
        return;
    }
    logs.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, path) in logs.drain(LOG_RETENTION..) {
        let _ = std::fs::remove_file(path);
    }
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
    /// `log` is written under `.meridian/runs/`, never in the tree, and never
    /// under `dry` — a rehearsal spawns nothing, so there is nothing to record.
    fn published(&self, call: &effects::BashCall, seen: &Value) -> Value {
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
        if !dry && !(stdout.is_empty() && stderr.is_empty()) {
            if let Some(path) = self.write_log(stdout, stderr) {
                row["log"] = json!(path);
            }
        }
        row
    }

    /// Write one call's streams to an out-of-tree log, and answer the
    /// workspace-relative path. `None` when the write fails: a row that cannot
    /// point at a log says nothing rather than pointing at a path that is not
    /// there.
    fn write_log(&self, stdout: &str, stderr: &str) -> Option<String> {
        let dir = self.root.join(".meridian/runs");
        std::fs::create_dir_all(&dir).ok()?;
        let digest = blake3::hash(format!("{stdout}\u{0}{stderr}").as_bytes());
        let name = format!("exec-{}.log", &digest.to_hex()[..16]);
        let body = if stderr.is_empty() {
            stdout.to_owned()
        } else {
            format!("{stdout}\n--- stderr ---\n{stderr}")
        };
        std::fs::write(dir.join(&name), body).ok()?;
        Self::prune_logs(&dir);
        Some(format!(".meridian/runs/{name}"))
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
            source: &source,
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
    fn exec(
        &self,
        spec: &effects::ExecSpec,
        stdin: Option<&str>,
        invocation: &str,
        page: &str,
        block_anchor: &str,
        target: &wire::RunTarget,
    ) -> Result<Value, (&'static str, String)> {
        let source = match &spec.program {
            effects::ExecProgram::Cmd(cmd) => Ok(cmd.clone()),
            effects::ExecProgram::Block(anchor) => blocks::block(self.doc, anchor)
                .map(|block| block.source)
                .map_err(|e| e.to_string()),
        };
        // A dangling `exec(block=)` anchor is a DECLARATION defect, and § 2.2
        // resolves it at LOAD — `addressed_entry` does that now, so reaching
        // this arm means the page changed under us. It is still an inability
        // to start, never a script's exit: it answers a typed refusal that
        // keeps the anchor error's own words.
        let source = match source {
            Ok(source) => source,
            Err(reason) => return Err(("no_block", reason)),
        };
        if self.dry {
            return Ok(json!({
                "interpreter": spec.interpreter, "exit": 0,
                "stdout_tail": "", "stderr_tail": "", "timed_out": false, "dry": true,
            }));
        }
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
            source: &source,
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
                Ok(json!({
                    "interpreter": spec.interpreter,
                    "exit": exit,
                    "stdout_tail": String::from_utf8_lossy(&result.stdout),
                    "stderr_tail": String::from_utf8_lossy(&result.stderr),
                    "timed_out": timed_out,
                }))
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

    fn empty_doc() -> Document {
        let raw = "# probe\n".to_owned();
        let nodes = syntax::parse(&raw);
        model::build(raw, nodes)
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

        let first = load_cached(&world, &block, &ctx, EvalLimits::default()).expect("the block loads");
        let second = load_cached(&world, &block, &ctx, EvalLimits::default()).expect("and loads again");

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
