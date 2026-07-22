//! The shared executor — the ONE write path both dispatch paths (starlark U5,
//! bash U6) converge on (plan decision #4; verdict ruling 2). md.* effect
//! descriptors → block-capability validation AT THE CHOKE POINT → one atomic
//! `if_root`-pinned splice batch → receipt in the same commit → apply→event
//! synthesis with REAL post-apply fingerprints (the phase-2 cascade adopts
//! this module; it imports no runner).
//!
//! # Laws enforced here
//! - **Choke point (decision #13):** every descriptor is validated against the
//!   block's [`CapSet`] before ANY I/O; one violation refuses the whole batch.
//! - **One batch (ruling 2):** all edits + the receipt ride ONE
//!   `model::validate_batch` → `fs::apply_batch` commit; a refusal applies
//!   nothing. There is NO rollback path — the executor never un-writes
//!   (rollback would be a second write path with invented authority).
//! - **`if_root` pin + gate #19:** the batch pins `pin_root` and validates
//!   against `live_root`; BOTH are required `MerkleRoot`s at this API, so the
//!   silent enforcement-off (`live_root = None` skips the guard,
//!   `model::validate_batch` §5.1) is unrepresentable. The caller threads the
//!   COMPUTED root — never re-read around a bash step.
//! - **Self-guards (decision #9):** every edit carries `if_node_rev` from the
//!   load-time resolve; load → validate → commit runs under the workspace
//!   flock ([`WorkspaceLock`], `LOCK_NB` — a held lock is the fast typed
//!   [`ExecError::WorkspaceBusy`] refusal, never a wait), closing the
//!   cross-process read→rename TOCTOU without ever taking callers hostage.
//! - **Foreign-edit law (decision #26, ZT):** CAS only covers concurrent
//!   races — a re-run reads the edited state, so its token matches and a
//!   replace would silently destroy a newer HUMAN edit. The executor anchors
//!   on receipts: if a target has a prior run receipt and its current rev is
//!   not that receipt's after-rev, the batch refuses with a typed
//!   [`ExecError::ForeignEdit`] carrying the three-way frame (target, the
//!   last governed after-rev, the current rev). Overwrite only via the
//!   explicit `takeover` flag.
//! - **§9 identity/time:** `invocation_id` and `now` are caller-supplied
//!   strings; nothing here reads a clock or mints an id.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

use model::{
    Document, Edit, EditKind, HpathSeg, MerkleRoot, NodeKind, NodeRev, PutAt, ReceiptAppend, Ref,
    SpliceRequest, SpliceVerdict, delta,
};
use rules::{ArgValue, ChangeEvent, Domain, Effect, EffectKind};
use serde::{Deserialize, Serialize};

use crate::caps::CapSet;
use crate::record::{ExecRecord, ExecRecordSink};

/// Where the run receipt lands: a workspace-relative file (appended) and the
/// pre-minted block anchor for the line. Address policy is the CALLER's (U5
/// convention); the executor renders and folds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiptAddr {
    /// Workspace-relative receipt file path (created on first append).
    pub path: String,
    /// The line's block anchor id (e.g. `r-000042`), caller-minted.
    pub anchor: String,
}

/// One apply request: the md.* descriptors of one generation, their governing
/// caps, and the root pins. See the module docs for the laws each field serves.
#[derive(Debug)]
pub struct ApplyRequest<'a> {
    /// The page the effects apply to (workspace-relative).
    pub page: &'a str,
    /// The task name — the receipt actor is `run:<task>`.
    pub task: &'a str,
    /// The addressed task block's `node_rev` at eval/address time — the
    /// procedure-hash the receipt attests (WHICH code ran, not just the
    /// mutable task NAME). The caller threads it from the resolved block.
    pub task_rev: &'a str,
    /// Caller-supplied invocation id (§9).
    pub invocation_id: &'a str,
    /// Caller-supplied time fact (§9); absent stays absent, never invented.
    pub now: Option<&'a str>,
    /// The md.* effect descriptors to apply, in emission order.
    pub effects: &'a [Effect],
    /// The block's resolved capability set — the choke point validates
    /// against exactly this.
    pub caps: &'a CapSet,
    /// The root the effects were produced against — pinned as the batch's
    /// `if_root` (root-at-eval, or `root_after_phase1` on the bash path).
    pub pin_root: &'a MerkleRoot,
    /// The current computed corpus root the pin validates against (gate #19:
    /// required, never `None`, never re-read around a bash step).
    pub live_root: &'a MerkleRoot,
    /// Receipt address; `None` skips the receipt AND the foreign-edit anchor
    /// (no provenance to check against — dispatch paths always pass one).
    pub receipt: Option<ReceiptAddr>,
    /// Explicit foreign-edit takeover (decision #26): overwrite a target whose
    /// current rev diverged from its last governed after-rev.
    pub takeover: bool,
    /// The cascade generation of the effects being applied (`0` for the run
    /// itself); the synthesized event carries `depth + 1`.
    pub depth: u32,
}

/// A committed apply: what landed and the facts the runner reports.
#[derive(Debug, Clone, PartialEq)]
pub struct Applied {
    /// How many descriptors were applied (all of them — the batch is atomic).
    pub applied: usize,
    /// The apply→event synthesis: the semantic change this batch caused, with
    /// REAL post-apply fingerprints (`None` when the batch was a no-op).
    pub event: Option<ChangeEvent>,
    /// The receipt line that rode the commit (`None` without an address).
    pub receipt_line: Option<String>,
    /// The page's post-apply file rev.
    pub file_rev_after: String,
}

/// Why the executor refused. Every variant applied NOTHING — the batch is
/// atomic and there is no partial state and no rollback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecError {
    /// A non-md descriptor reached the executor — a dispatch bug, refused loud.
    NonMdEffect { kind: String },
    /// A descriptor's kind/target is not admitted by the block's caps.
    CapDenied { kind: String, target: String },
    /// A descriptor argument is missing or wrongly shaped (kernel constructors
    /// make this unreachable; hand-built descriptors fault here).
    BadDescriptor { kind: String, reason: String },
    /// `md.append_section` names a section absent from the page.
    SectionNotFound { section: String },
    /// `md.append_section` names a heading appearing more than once.
    SectionAmbiguous { section: String, count: usize },
    /// Decision #26: the target was edited outside the run plane since its
    /// last governed write — the three-way frame (target, last governed
    /// after-rev, current rev). Overwrite requires `takeover`.
    ForeignEdit {
        target: String,
        last_governed: String,
        current: String,
    },
    /// Another run holds the workspace lock (decision #9: `LOCK_NB` — a fast
    /// typed refusal, never a wait; a hung holder can never make callers hang).
    WorkspaceBusy,
    /// The pinned root does not match the live root — out-of-band change;
    /// the declared effects refuse (ruling 2).
    RootMismatch { expected: String, actual: String },
    /// Any other typed validation refusal (CAS, no-match, would-corrupt,
    /// overlap, …) — carried as the verdict's debug shape.
    Refused { verdict: String },
    /// Page load failure (missing, non-UTF-8, I/O).
    Page { path: String, reason: String },
    /// Lock, receipt-scan, or commit I/O failure.
    Io { reason: String },
}

impl std::fmt::Display for ExecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecError::NonMdEffect { kind } => {
                write!(f, "executor applies md.* only, got '{kind}'")
            }
            ExecError::CapDenied { kind, target } => {
                write!(f, "capability denied: {kind} on '{target}'")
            }
            ExecError::BadDescriptor { kind, reason } => {
                write!(f, "bad {kind} descriptor: {reason}")
            }
            ExecError::SectionNotFound { section } => write!(f, "no section '{section}'"),
            ExecError::SectionAmbiguous { section, count } => {
                write!(f, "section '{section}' appears {count} times (ambiguous)")
            }
            ExecError::ForeignEdit {
                target,
                last_governed,
                current,
            } => write!(
                f,
                "foreign edit on {target}: last governed rev {last_governed}, current {current} — refusing to overwrite (use takeover to override)"
            ),
            ExecError::WorkspaceBusy => write!(
                f,
                "workspace busy: another run holds the lock — retry when it exits"
            ),
            ExecError::RootMismatch { expected, actual } => write!(
                f,
                "root mismatch: pinned {expected}, live {actual} — out-of-band change, nothing applied"
            ),
            ExecError::Refused { verdict } => write!(f, "batch refused: {verdict}"),
            ExecError::Page { path, reason } => write!(f, "page {path}: {reason}"),
            ExecError::Io { reason } => write!(f, "io: {reason}"),
        }
    }
}

impl std::error::Error for ExecError {}

/// The workspace run lock (decision #9): an exclusive advisory `flock(2)` on
/// `.meridian/run.lock`, held from page load through the atomic rename, so two
/// local runs cannot interleave read→rename (the lost-update TOCTOU intra-
/// process CAS guards cannot see). `LOCK_NB` acquire — a held lock is
/// [`io::ErrorKind::WouldBlock`], surfaced as the fast typed
/// [`ExecError::WorkspaceBusy`] refusal; it never waits, so a hung holder can
/// never make callers hang (review C4). Released on drop.
#[derive(Debug)]
pub struct WorkspaceLock {
    // Held open for its fd; flock releases when the fd closes on drop.
    _file: File,
}

impl WorkspaceLock {
    /// Try to acquire the exclusive workspace run lock, creating `.meridian/`
    /// and the lockfile on first use. Never blocks: a held lock returns
    /// [`io::ErrorKind::WouldBlock`] immediately (decision #9).
    ///
    /// # Errors
    /// [`io::ErrorKind::WouldBlock`] when another run holds the lock; any
    /// other I/O failure creating or locking the lockfile.
    pub fn acquire(workspace_root: &Path) -> io::Result<Self> {
        let dir = workspace_root.join(".meridian");
        std::fs::create_dir_all(&dir)?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join("run.lock"))?;
        // SAFETY: flock on a valid open fd; the fd outlives the call.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { _file: file })
    }
}

/// The machine-re-readable body of one run receipt line — what the
/// decision-#26 check parses back. Serialized as one compact JSON object
/// inside the markdown line (`- run {json} ^anchor`); the format is
/// run-plane-local (review S10), not the wire receipt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiptFacts {
    /// The page the batch applied to.
    pub page: String,
    /// The task that ran.
    pub task: String,
    /// Caller-supplied invocation id.
    pub invocation: String,
    /// `run:<task>`.
    pub actor: String,
    /// Caller-supplied time fact; absent stays absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub now: Option<String>,
    /// The root the batch was pinned to.
    pub root_pin: String,
    /// The addressed task block's `node_rev` — the procedure-hash (attestation
    /// roadmap): the receipt names WHICH code ran, not just the mutable task
    /// NAME. Stamped at eval/address time, frozen into the receipt here.
    pub task_rev: String,
    /// Per-edit facts: target identity + rev transition.
    pub edits: Vec<ReceiptEdit>,
    /// The bash step's exec facts (U8), adopted post-hoc via
    /// [`ExecRecordSink`]; absent on the hermetic path (no child process).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub exec: Option<ExecRecord>,
}

/// The U8→U4 record edge (review S10): the receipt owner adopts the sealed
/// exec facts by taking them into its optional `exec` field. Defined here so
/// the bash path can fill the record without editing this file's construction.
impl ExecRecordSink for ReceiptFacts {
    fn fill_exec(&mut self, exec: ExecRecord) {
        self.exec = Some(exec);
    }
}

/// One edit's receipt fact: which node, and its rev transition. `after` is
/// the foreign-edit anchor the NEXT run compares against.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiptEdit {
    /// The target's identity.
    pub target: EditTarget,
    /// The node rev the edit was validated against.
    pub before: String,
    /// The node rev after the commit — the decision-#26 anchor.
    pub after: String,
}

/// A run-plane edit target identity — structured, join-string-free (mirrors
/// the wire's no-join-string law).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EditTarget {
    /// A frontmatter key.
    #[serde(rename = "fm")]
    FmKey(String),
    /// A section heading chain, root → governing heading.
    #[serde(rename = "sec")]
    Section(Vec<String>),
}

impl EditTarget {
    fn describe(&self) -> String {
        match self {
            EditTarget::FmKey(k) => format!("fm:{k}"),
            EditTarget::Section(segs) => format!("section:{}", segs.join("#")),
        }
    }
}

/// One planned edit: the model edit plus the identity/rev facts the receipt
/// and the foreign-edit check need.
struct PlannedEdit {
    edit: Edit,
    identity: EditTarget,
    before_rev: NodeRev,
}

/// Apply one generation of md.* effects to `page` as ONE atomic batch. See the
/// module docs for the full law set; the flow is: choke-point cap validation →
/// flock → load → plan edits (self-guarded) → foreign-edit check (#26) →
/// validate (`if_root` pinned) → receipt → the single `fs::apply_batch`
/// commit → apply→event synthesis.
///
/// # Errors
/// [`ExecError`] — in every case NOTHING was applied.
pub fn apply(root: &fs::WorkspaceRoot, req: &ApplyRequest<'_>) -> Result<Applied, ExecError> {
    // Serialize local runs (decision #9: LOCK_NB — busy is a fast typed
    // refusal, never a wait).
    let lock = WorkspaceLock::acquire(&root.0).map_err(|e| {
        if e.kind() == io::ErrorKind::WouldBlock {
            ExecError::WorkspaceBusy
        } else {
            ExecError::Io {
                reason: format!("workspace lock: {e}"),
            }
        }
    })?;
    apply_under(&lock, root, req)
}

/// [`apply`] under a CALLER-held [`WorkspaceLock`] — the U6a two-phase seam
/// (u4-gate addendum on #19): the bash dispatcher must commit phase 1 and
/// compute `root_after_phase1` inside ONE locked window, so it holds the lock
/// across both and threads it here. The lock parameter is the proof-of-lock —
/// flock cannot re-acquire on a second fd, so a self-locking call under a
/// held lock would refuse itself as busy.
///
/// # Errors
/// [`ExecError`] — in every case NOTHING was applied.
pub fn apply_under(
    _lock: &WorkspaceLock,
    root: &fs::WorkspaceRoot,
    req: &ApplyRequest<'_>,
) -> Result<Applied, ExecError> {
    // 1. THE CHOKE POINT — before any I/O: md.* only, each admitted by the
    // block's caps (kind + target, so target-scoped caps bind for real).
    for effect in req.effects {
        let (kind, target) = descriptor_surface(effect)?;
        if !req.caps.admits(kind, Some(&target)) {
            return Err(ExecError::CapDenied {
                kind: kind.to_owned(),
                target,
            });
        }
    }

    // 2. Load under the lock.
    let doc = fs::load(root, Path::new(req.page)).map_err(|e| ExecError::Page {
        path: req.page.to_owned(),
        reason: e.to_string(),
    })?;

    // 3. Plan edits, self-guarded with load-time revs.
    let mut planned = Vec::with_capacity(req.effects.len());
    for effect in req.effects {
        planned.push(plan_edit(&doc, effect)?);
    }

    // 4. Foreign-edit law (#26): receipt-anchored, refused unless takeover.
    if let Some(addr) = &req.receipt
        && !req.takeover
    {
        check_foreign_edits(root, addr, req.page, &planned)?;
    }

    // 5. Validate — mints the sealed batch; the `if_root` pin runs against
    // the REQUIRED live root (gate #19).
    let batch = SpliceRequest {
        if_root: Some(req.pin_root.clone()),
        edits: planned.iter().map(|p| p.edit.clone()).collect(),
    };
    let sealed = match model::validate_batch(&doc, Some(req.live_root), &batch, None) {
        SpliceVerdict::Validated(b) => b,
        SpliceVerdict::RootMismatch { expected, actual } => {
            return Err(ExecError::RootMismatch {
                expected: expected.0,
                actual: actual.0,
            });
        }
        refused => {
            return Err(ExecError::Refused {
                verdict: format!("{refused:?}"),
            });
        }
    };

    // 6. Armed facts: dry-apply the sealed edits in memory — the SAME bytes
    // fs will write — and read each target's post-apply rev off the reparse.
    let mut after_raw = doc.raw.clone();
    for edit in sealed.edits.iter().rev() {
        after_raw.replace_range(edit.span.clone(), &edit.text);
    }
    let after_nodes = syntax::parse(&after_raw);
    let after_doc = model::build(after_raw, after_nodes);
    let after_revs: Vec<NodeRev> = planned
        .iter()
        .map(|p| after_rev(&after_doc, &p.edit.target))
        .collect::<Result<_, _>>()?;

    // 7. Receipt (rides the same sealed commit — §6.1).
    let receipt = match &req.receipt {
        Some(addr) => Some(render_receipt(root, addr, req, &planned, &after_revs)?),
        None => None,
    };
    let receipt_line = receipt.as_ref().map(|(_, _, line)| line.clone());
    let sealed = match model::validate_batch(
        &doc,
        Some(req.live_root),
        &batch,
        receipt.as_ref().map(|(_, append, _)| append.clone()),
    ) {
        SpliceVerdict::Validated(b) => b,
        refused => {
            // Same inputs as step 5 plus an EOF append — unreachable refusal.
            return Err(ExecError::Refused {
                verdict: format!("{refused:?}"),
            });
        }
    };

    // 8. THE commit — the only write, atomic, two files (§6.5 crash window
    // accepted: content-without-receipt recovers by re-derive + lint).
    fs::apply_batch(
        root,
        Path::new(req.page),
        receipt
            .as_ref()
            .map(|(path, _, _)| Path::new(path.as_str())),
        &sealed,
    )
    .map_err(|e| ExecError::Io {
        reason: e.to_string(),
    })?;

    // 9. Apply→event synthesis with REAL post-apply fingerprints (the dry
    // bytes ARE the committed bytes — no re-read).
    let event = synthesize_event(req.page, &doc, &after_doc, req.depth);
    Ok(Applied {
        applied: req.effects.len(),
        event,
        receipt_line,
        file_rev_after: after_doc.root.node_rev.0.clone(),
    })
}

/// A descriptor's choke-point surface: its namespaced kind string and its
/// capability target (the field / section it touches).
fn descriptor_surface(effect: &Effect) -> Result<(&'static str, String), ExecError> {
    if effect.kind.domain() != Domain::Md {
        return Err(ExecError::NonMdEffect {
            kind: effect.kind.as_str().to_owned(),
        });
    }
    let target = match effect.kind {
        EffectKind::SetField => str_arg(effect, "field")?,
        EffectKind::AppendSection => str_arg(effect, "section")?,
        _ => unreachable!("md.* kinds are SetField | AppendSection"),
    };
    Ok((effect.kind.as_str(), target))
}

/// A required scalar string argument off a descriptor.
fn str_arg(effect: &Effect, key: &str) -> Result<String, ExecError> {
    match effect.args.get(key) {
        Some(ArgValue::Str(s)) => Ok(s.clone()),
        _ => Err(ExecError::BadDescriptor {
            kind: effect.kind.as_str().to_owned(),
            reason: format!("missing scalar '{key}'"),
        }),
    }
}

/// Plan one md.* descriptor as a self-guarded model edit.
fn plan_edit(doc: &Document, effect: &Effect) -> Result<PlannedEdit, ExecError> {
    match effect.kind {
        EffectKind::SetField => {
            let field = str_arg(effect, "field")?;
            let value = str_arg(effect, "value")?;
            let before = model::fm_upsert_before(doc, &field);
            Ok(PlannedEdit {
                edit: Edit {
                    target: Ref::FmKey(field.clone()),
                    edit: EditKind::Put {
                        at: PutAt::Upsert,
                        text: value,
                    },
                    if_node_rev: Some(before.node_rev.clone()),
                },
                identity: EditTarget::FmKey(field),
                before_rev: before.node_rev,
            })
        }
        EffectKind::AppendSection => {
            let section = str_arg(effect, "section")?;
            let content = str_arg(effect, "content")?;
            let (segs, span_end_byte, node_rev) = find_section(doc, &section)?;
            // Append as a LINE: exactly one trailing newline; a leading one
            // only when the section's last byte is not already a terminator.
            let mut text = String::new();
            if span_end_byte != Some(b'\n') {
                text.push('\n');
            }
            text.push_str(content.trim_end_matches('\n'));
            text.push('\n');
            Ok(PlannedEdit {
                edit: Edit {
                    target: Ref::Hpath(
                        segs.iter()
                            .map(|h| HpathSeg {
                                h: h.clone(),
                                n: None,
                            })
                            .collect(),
                    ),
                    edit: EditKind::Put {
                        at: PutAt::End,
                        text,
                    },
                    if_node_rev: Some(node_rev.clone()),
                },
                identity: EditTarget::Section(segs),
                before_rev: node_rev,
            })
        }
        _ => Err(ExecError::NonMdEffect {
            kind: effect.kind.as_str().to_owned(),
        }),
    }
}

/// Find the UNIQUE section whose governing heading text is `heading`; returns
/// its full hpath chain, its last raw byte, and its load-time rev. Zero → not
/// found; two-plus → ambiguous (the mint plane never silently picks).
fn find_section(
    doc: &Document,
    heading: &str,
) -> Result<(Vec<String>, Option<u8>, NodeRev), ExecError> {
    fn collect<'a>(node: &'a model::Node, heading: &str, out: &mut Vec<&'a model::Node>) {
        if matches!(&node.kind, NodeKind::Section { heading_text, .. } if heading_text == heading) {
            out.push(node);
        }
        for c in &node.children {
            collect(c, heading, out);
        }
    }
    let mut hits: Vec<&model::Node> = Vec::new();
    collect(&doc.root, heading, &mut hits);
    match hits.as_slice() {
        [] => Err(ExecError::SectionNotFound {
            section: heading.to_owned(),
        }),
        [only] => {
            let segs = only
                .hpath
                .clone()
                .unwrap_or_else(|| vec![heading.to_owned()]);
            let last_byte = doc
                .raw
                .as_bytes()
                .get(only.span.end.wrapping_sub(1))
                .copied();
            Ok((segs, last_byte, only.node_rev.clone()))
        }
        many => Err(ExecError::SectionAmbiguous {
            section: heading.to_owned(),
            count: many.len(),
        }),
    }
}

/// Decision #26: refuse any planned target whose current rev diverged from its
/// last governed after-rev (per the newest matching receipt line).
fn check_foreign_edits(
    root: &fs::WorkspaceRoot,
    addr: &ReceiptAddr,
    page: &str,
    planned: &[PlannedEdit],
) -> Result<(), ExecError> {
    let anchors = last_governed_revs(root, addr, page)?;
    for p in planned {
        if let Some(last) = anchors.get(&p.identity.describe())
            && *last != p.before_rev.0
        {
            return Err(ExecError::ForeignEdit {
                target: p.identity.describe(),
                last_governed: last.clone(),
                current: p.before_rev.0.clone(),
            });
        }
    }
    Ok(())
}

/// The newest governed after-rev per target for `page`, scanned from every
/// receipt file beside the addressed one (receipts roll by date — the prior
/// run's line may sit in an older file). Later files and later lines win.
fn last_governed_revs(
    root: &fs::WorkspaceRoot,
    addr: &ReceiptAddr,
    page: &str,
) -> Result<BTreeMap<String, String>, ExecError> {
    let io_err = |e: io::Error| ExecError::Io {
        reason: format!("receipt scan: {e}"),
    };
    let abs = root.0.join(&addr.path);
    let dir: PathBuf = abs
        .parent()
        .map_or_else(|| root.0.clone(), Path::to_path_buf);
    let mut out = BTreeMap::new();
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(io_err(e)),
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("md")))
        .collect();
    files.sort();
    for file in files {
        let text = std::fs::read_to_string(&file).map_err(io_err)?;
        for line in text.lines() {
            let Some(facts) = parse_receipt_line(line) else {
                continue;
            };
            if facts.page != page {
                continue;
            }
            for edit in facts.edits {
                out.insert(edit.target.describe(), edit.after);
            }
        }
    }
    Ok(out)
}

/// Parse one `- run {json} ^anchor` receipt line; `None` for any other line.
fn parse_receipt_line(line: &str) -> Option<ReceiptFacts> {
    let body = line.strip_prefix("- run ")?;
    let json = match body.rsplit_once(" ^") {
        Some((json, _anchor)) => json,
        None => body,
    };
    serde_json::from_str(json).ok()
}

/// Render the receipt line and its EOF append for this batch.
fn render_receipt(
    root: &fs::WorkspaceRoot,
    addr: &ReceiptAddr,
    req: &ApplyRequest<'_>,
    planned: &[PlannedEdit],
    after_revs: &[NodeRev],
) -> Result<(String, ReceiptAppend, String), ExecError> {
    let io_err = |e: io::Error| ExecError::Io {
        reason: format!("receipt: {e}"),
    };
    let facts = ReceiptFacts {
        page: req.page.to_owned(),
        task: req.task.to_owned(),
        invocation: req.invocation_id.to_owned(),
        actor: format!("run:{}", req.task),
        now: req.now.map(str::to_owned),
        root_pin: req.pin_root.0.clone(),
        task_rev: req.task_rev.to_owned(),
        edits: planned
            .iter()
            .zip(after_revs)
            .map(|(p, after)| ReceiptEdit {
                target: p.identity.clone(),
                before: p.before_rev.0.clone(),
                after: after.0.clone(),
            })
            .collect(),
        // No child exec on this path. The bash path's sealed record is adopted
        // through the `ExecRecordSink` seam (`fill_exec`); wiring it into the
        // committed line is the runner's receipt-composition step. Absent here.
        exec: None,
    };
    let json = serde_json::to_string(&facts).map_err(|e| ExecError::Io {
        reason: format!("receipt encode: {e}"),
    })?;
    let line = format!("- run {json} ^{}", addr.anchor);
    let abs = root.0.join(&addr.path);
    if let Some(parent) = abs.parent() {
        std::fs::create_dir_all(parent).map_err(io_err)?;
    }
    let len = match std::fs::read(&abs) {
        Ok(bytes) => bytes.len(),
        Err(e) if e.kind() == io::ErrorKind::NotFound => 0,
        Err(e) => return Err(io_err(e)),
    };
    Ok((
        addr.path.clone(),
        ReceiptAppend {
            span: len..len,
            text: format!("{line}\n"),
        },
        line,
    ))
}

/// A planned target's post-apply rev, read off the reparsed after-document.
fn after_rev(after_doc: &Document, target: &Ref) -> Result<NodeRev, ExecError> {
    match target {
        // The upserted key exists after the batch by construction.
        Ref::FmKey(key) => model::resolve(after_doc, &Ref::FmKey(key.clone()))
            .map(|t| t.node_rev)
            .map_err(|e| ExecError::Refused {
                verdict: format!("post-apply resolve of fm:{key} failed: {e:?}"),
            }),
        other => model::resolve(after_doc, other)
            .map(|t| t.node_rev)
            .map_err(|e| ExecError::Refused {
                verdict: format!("post-apply resolve failed: {e:?}"),
            }),
    }
}

/// The apply→event synthesis (decision #4): the semantic change THIS batch
/// caused, from the real before/after documents — the primitive the phase-2
/// resident cascade adopts. Deterministic, duplicate-free change sets;
/// fingerprints are the real file revs; `depth` is the applied generation
/// plus one.
#[must_use]
pub fn synthesize_event(
    page: &str,
    before: &Document,
    after: &Document,
    applied_depth: u32,
) -> Option<ChangeEvent> {
    let fd = delta::file_delta(Some(before), Some(after))?;
    let mut sections = Vec::new();
    let mut fields = Vec::new();
    for nd in &fd.nodes {
        match &nd.target {
            Ref::Hpath(segs) => sections.push(render_hpath(segs)),
            Ref::Anchor(id) => sections.push(format!("^{id}")),
            Ref::FmKey(key) => fields.push(key.clone()),
        }
    }
    sections.sort();
    sections.dedup();
    fields.sort();
    fields.dedup();
    Some(ChangeEvent {
        file: page.to_owned(),
        sections_changed: sections,
        fields_changed: fields,
        fingerprint_before: fd.file_rev_before.map(|r| r.0).unwrap_or_default(),
        fingerprint_after: fd.file_rev_after.map(|r| r.0).unwrap_or_default(),
        depth: applied_depth + 1,
    })
}

/// Render an hpath for the event payload (`A#B`, `%n` occurrence) — the same
/// spelling `mrd rules replay` uses.
fn render_hpath(segs: &[HpathSeg]) -> String {
    segs.iter()
        .map(|s| match s.n {
            Some(n) => format!("{}%{n}", s.h),
            None => s.h.clone(),
        })
        .collect::<Vec<_>>()
        .join("#")
}
