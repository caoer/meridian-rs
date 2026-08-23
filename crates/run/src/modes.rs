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

/// Empty fields for a caller with no frame passthrough in reach.
static NO_FIELDS: std::sync::LazyLock<BTreeMap<String, String>> =
    std::sync::LazyLock::new(BTreeMap::new);

/// One mode-bearing target → one row.
#[must_use]
pub fn mode_row(
    world: &ModeWorld<'_>,
    target: &wire::RunTarget,
    invocation: &str,
    actor: Option<&str>,
    now: Option<&str>,
) -> Value {
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
) -> Value {
    let doc = world.doc;
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
    doc: &Document,
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
) -> (Vec<Value>, Option<(&'static str, String)>) {
    if effects.is_empty() {
        return (Vec::new(), None);
    }
    let authority = match page_authority(world) {
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
        "result": "born",
        "args": args_of(effect),
    });
    let born_path = (effect.kind == effects::EffectKind::Create)
        .then(|| effect.args.get("path"))
        .flatten()
        .and_then(|p| match p {
            effects::ArgValue::Str(path) => Some(path.clone()),
            effects::ArgValue::List(_) => None,
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
pub fn is_mode_target(target: &wire::RunTarget) -> bool {
    target.mode.is_some()
}
