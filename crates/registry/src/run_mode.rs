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
use run::blocks::{self, AnchoredBlock, BlockError};
use run::caps::{self, Authority};
use run::executor::{self, ApplyRequest, ExecError};
use serde_json::{Value, json};

use crate::engine::WorkspaceEngine;
use crate::run_op::RunHost;

/// The pinned world one mode-bearing submission runs against, plus the
/// workspace facts every row needs. Built once per submission so the rows of
/// one call cannot disagree about which corpus they read.
pub(crate) struct ModeWorld<'a> {
    pub(crate) world: &'a WorkspaceEngine,
    pub(crate) root: &'a fs::WorkspaceRoot,
    pub(crate) ws: &'a Path,
    /// The caller's `prelude` (cap `run.mode`), shared by every mode-bearing
    /// target in the call.
    pub(crate) prelude: Option<&'a str>,
}

/// One mode-bearing target → one row.
pub(crate) fn mode_row(
    world: &ModeWorld<'_>,
    target: &wire::RunTarget,
    invocation: &str,
    actor: Option<&str>,
    now: Option<&str>,
    host: &RunHost<'_>,
) -> Value {
    match target.mode {
        Some(wire::RunMode::Load) => load_row(world, target, invocation),
        Some(wire::RunMode::Fire) => fire_row(world, target, invocation, actor, now, host),
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
    let Some(doc) = world.world.docs.get(&target.page) else {
        return refused_row(
            target,
            invocation,
            "no_block",
            &format!("no such page in the pinned corpus: {}", target.page),
            None,
        );
    };
    // The prelude is checked ONCE, before any block — § 2.2's
    // `prelude_invalid` refuses before a single block is looked at, so a
    // typo in caller source is never reported as a page's fault.
    if let Some(prelude) = world.prelude
        && let Some(fault) =
            effects::check_prelude(prelude, &ctx_for(target, invocation), EvalLimits::default())
    {
        return refused_row(
            target,
            invocation,
            "prelude_invalid",
            &fault.reason,
            fault.line,
        );
    }
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
    let loaded = load_block(
        &block.source,
        world.prelude,
        &ctx_for(target, invocation),
        EvalLimits::default(),
    );
    if let Some(fault) = loaded.fault {
        return Some(json!({
            "block": block.anchor,
            "rev": block.rev,
            "result": "fault",
            "fault": fault_object(&fault),
        }));
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
    host: &RunHost<'_>,
) -> Value {
    let Some(doc) = world.world.docs.get(&target.page) else {
        return refused_row(
            target,
            invocation,
            "no_block",
            &format!("no such page in the pinned corpus: {}", target.page),
            None,
        );
    };
    let (block, entry, module) = match addressed_entry(world, target, invocation, doc) {
        Ok(resolved) => resolved,
        Err(row) => return row,
    };
    let ctx = ctx_for(target, invocation);
    let seam = NoProcessSeam;
    let fired = match fire_entry(
        &module,
        &entry,
        target.input.as_ref().unwrap_or(&Value::Null),
        &ctx,
        &seam,
        EvalLimits::default(),
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
        host,
    );
    let mut row = json!({
        "page": target.page,
        "invocation": invocation,
        "block": block.anchor,
        "rev": {"file": doc.root.node_rev.0, "block": block.rev},
        "result": if refusal.is_some() { "refused" } else { "ok" },
        "applied": applied,
        "exec": [],
        "telemetry": {"steps": fired.fuel_used, "mem": fired.mem_used},
    });
    if let Some(value) = fired.value {
        row["value"] = value;
    }
    if let Some((class, reason)) = refusal {
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
    doc: &model::Document,
) -> Result<(AnchoredBlock, String, effects::FrozenModule), Value> {
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
        // A non-starlark block is an exec'd entry's bytes, not a module. The
        // exec bracket that runs one is the next increment; until it lands
        // this refuses by NAME rather than pretending the block is a module.
        return Err(refused_row(
            target,
            invocation,
            "not_a_module",
            &format!(
                "^{anchor} is not a starlark block, so it is not a module: a non-starlark \
                 block runs as an exec'd entry, declared with `exec(...)`"
            ),
            None,
        ));
    }
    let ctx = ctx_for(target, invocation);
    let loaded = load_block(&block.source, world.prelude, &ctx, EvalLimits::default());
    if let Some(fault) = loaded.fault {
        return Err(fault_row(target, invocation, &block, &fault));
    }
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
        DeclaredEntry::Exec(_) => {
            return Err(refused_row(
                target,
                invocation,
                "impl_type",
                "this block declares an exec'd entry; the process bracket that runs one \
                 is not built yet",
                None,
            ));
        }
    };
    let Some(module) = loaded.module else {
        return Err(refused_row(
            target,
            invocation,
            "missing_entry",
            "the block loaded but did not freeze",
            None,
        ));
    };
    Ok((block, entry, module))
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
    host: &RunHost<'_>,
) -> (Vec<Value>, Option<(&'static str, String)>) {
    if effects.is_empty() {
        return (Vec::new(), None);
    }
    let authority = match page_authority(world, target) {
        Ok(authority) => authority,
        Err(e) => return (Vec::new(), Some(("cap_denied", e.to_string()))),
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
        observed_root: &world.world.at_fingerprint,
        // **A fire row writes no receipt** — the recording law. The record of
        // a fire is its response and the caller's own journal.
        receipt: None,
        exec: None,
        actor,
        depth: 0,
        delta: Some(host.sink),
        fields: host.fields,
        birth_seq: Some(host.birth_seq),
        ambient: host.ambient,
    };
    match executor::apply(world.root, &request) {
        Ok(applied) => {
            let rows = effects
                .iter()
                .map(|e| {
                    json!({
                        "kind": e.kind.as_str(),
                        "result": "born",
                        "args": args_of(e),
                        "file_rev": applied.file_rev_after,
                    })
                })
                .collect();
            (rows, None)
        }
        // The batch is atomic: nothing landed, so no effect row claims it
        // did. `cap_denied` is the row's fault by name; every other refusal
        // rides the door's own words.
        Err(ExecError::CapDenied {
            kind,
            target: coordinate,
            ceiling,
            declared,
            ..
        }) => (
            Vec::new(),
            Some((
                "cap_denied",
                format!(
                    "`{kind}` on `{coordinate}` is outside this page's `caps:` \
                     (granted: {}; ceiling: {})",
                    if declared.is_empty() {
                        "none".to_owned()
                    } else {
                        declared.join(", ")
                    },
                    ceiling.unwrap_or_else(|| "none".to_owned())
                ),
            )),
        ),
        Err(e) => (Vec::new(), Some(("runtime", e.to_string()))),
    }
}

/// The page's authority: its `caps:` narrowed by the declaring root's
/// conventions.
fn page_authority(
    world: &ModeWorld<'_>,
    target: &wire::RunTarget,
) -> Result<Authority, caps::CapsError> {
    let page_caps = world
        .world
        .docs
        .get(&target.page)
        .map(|doc| caps::page_caps(doc))
        .transpose()?
        .flatten();
    let (conventions, _) = caps::load_conventions(Some(world.ws))?;
    Ok(caps::resolve_page_authority(
        page_caps.as_ref(),
        &conventions,
    ))
}

/// The fire phase's process seam, before the exec bracket exists.
///
/// `bash()` is BOUND on the hook-plane globals at every phase — that is A1's
/// whole shape — so a program may call it and the call must answer something.
/// Until the bracket is factored, it answers a loud fault rather than a
/// plausible row: a stub returning `exit: 0` would let a hook decide on a
/// command that never ran.
struct NoProcessSeam;

impl FireHost for NoProcessSeam {
    fn bash(&self, call: &effects::BashCall) -> Result<Value, String> {
        Err(format!(
            "`bash` is not yet served on this lane (line {}): the process bracket the \
             fire phase runs it through is the next increment. The call is legal and the \
             refusal is the engine's, not the page's.",
            call.line
        ))
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
pub(crate) fn is_mode_target(target: &wire::RunTarget) -> bool {
    target.mode.is_some()
}
