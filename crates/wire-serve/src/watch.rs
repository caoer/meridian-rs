//! F5-WATCH — external-change CLASSIFIER, shared by both hosts (U20b).
//! `WatchState` tracks the world at ring tip; each reconcile is three-way:
//!
//! - snapshot root == watch root → nothing;
//! - snapshot root == ring tip `root_after` → INTERNAL: sync silently (no
//!   double emission; `commit_batch` already emitted);
//! - else → EXTERNAL: one change set = one root advance = one Delta;
//!   `actor`/`now` ABSENT (§7.1); `seq` at detection; ONE production constructor.
//!
//!
//!
//!
//!
//!
//! **Rename ruling (as data):** within one batch, EXACTLY one removed + EXACTLY
//! one added path with byte-equal content → `renamed` + `from_path`. Else
//! delete+create, `from_path` unwired. Fact plane is what we serve (delete+create
//! of byte-equal B classifies as rename — indistinguishable in the fact plane).
//!
//!
//!
//!
//!
//! **Stated degrade — SIDECAR ONLY:** external write in the same window as
//! internal commit can break ring-chain contiguity → `root_unknown` resync
//! (§7.3: re-derive, never wrong data). Registry detector snapshots under write
//! flock (never mid-landing). DRIVER differs; classifier is shared.
//!
//!
//!
//! # What is here vs not
//! Shared: three-way disposition, rename ruling, wire projection (what a change
//! MEANS). Per-host DRIVER: sidecar at serve-loop line boundary; registry on
//! subscription detection cycle.
//!
//!
//!
//!
//!
//!
//!

use wire::{DeltaFile, DeltaFrame, ErrorBody, FileChange, NodeRev, Path, Root};

use crate::ring::RootRing;

/// The world at the ring tip: the watcher's baseline + its folded root.
/// Unprimed until the first successful snapshot — the epoch's baseline.
#[derive(Debug)]
pub struct WatchState {
    watcher: fs::Watcher,
    root: Option<Root>,
}

impl WatchState {
    /// An unprimed watcher for `root`: the first reconcile adopts the world as
    /// its baseline and emits nothing.
    #[must_use]
    pub fn new(root: &fs::WorkspaceRoot) -> Self {
        WatchState {
            watcher: fs::Watcher::new(root.clone()),
            root: None,
        }
    }

    /// The baseline root this watcher last rebased on (`None`: unprimed).
    ///
    /// For a driver that wants to know whether a reconcile has anything to do
    /// BEFORE paying for one — the registry's detector compares a freshly folded
    /// disk root against this to decide whether to take the workspace write
    /// flock at all. Comparing here is not a shortcut around the disposition: an
    /// equal root is exactly the disposition's first arm ("a cycle finding
    /// nothing emits nothing"), reached without contending for a lock.
    #[must_use]
    pub fn root(&self) -> Option<&Root> {
        self.root.as_ref()
    }
}

/// One reconcile step. Returns the emitted external frame, if any — the caller
/// flushes it to subscribers.
///
/// **The caller owns the schedule, and the schedule is a correctness argument.**
/// The sidecar runs this on demand at its line boundary (`sidecar::demand`
/// prices each line first). The registry runs it on a subscription's detection
/// cycle, holding the workspace write flock so no batch is observed
/// mid-landing. Both are single-producer per ring, which is what keeps the
/// chain contiguous — a second concurrent producer against one ring would
/// duplicate a `seq` or break the root chain, and no lock inside this function
/// could repair that.
///
/// # Errors
/// A snapshot or classification failure. A caller driving this in a loop should
/// log and retry the next cycle: an unreadable workspace is a transient
/// condition, never a reason to tear down a subscription.
///
/// # Panics
/// If the watcher's baseline is unset on the arm that classifies against it —
/// unreachable by construction, since that arm is guarded by `watch.root` being
/// `Some`. The panic is the assertion, not a failure mode: a watcher that
/// classified against no baseline would emit a Delta describing the whole
/// corpus as new.
pub fn reconcile(
    ws_root: &fs::WorkspaceRoot,
    ring: &mut RootRing,
    watch: &mut WatchState,
) -> Result<Option<DeltaFrame>, Box<ErrorBody>> {
    let (files, disk_root) = crate::domain_snapshot(ws_root)?;
    match &watch.root {
        // Priming: the epoch's baseline is the first successful snapshot.
        None => {
            watch.watcher.rebase(&files);
            watch.root = Some(disk_root);
            Ok(None)
        }
        Some(r) if *r == disk_root => Ok(None),
        // An internal commit moved the world — already emitted; sync silent.
        Some(_) if ring.tip_root() == Some(&disk_root) => {
            watch.watcher.rebase(&files);
            watch.root = Some(disk_root);
            Ok(None)
        }
        Some(watch_root) => {
            let changes = watch
                .watcher
                .classify(&files)
                .expect("primed: watch.root is Some");
            let delta_files = classify_to_wire(&changes)?;
            // The ONE production DeltaFrame constructor (§7.3), shared with the
            // commit path in `wire-serve`. The caller advances the ring.
            // The detector allocates through the ring's ONE issuing point:
            // `assemble_delta` is a pure constructor now (U20b), so the number
            // is decided here, explicitly. `allocate_seq` rather than `seq() + 1`
            // is the whole seam — the write path can hold an allocation whose
            // frame has not landed yet, and the tip cannot see it.
            let mut frame = crate::write::assemble_delta(
                ring.allocate_seq(),
                watch_root.clone(),
                disk_root.clone(),
                None, // §7.1: actor ABSENT — never invented
                None, // §7.1: now ABSENT — never invented
                delta_files,
            );
            frame.effects = external_effects(ws_root, &changes)?;
            ring.advance(frame.clone());
            watch.watcher.rebase(&files);
            watch.root = Some(disk_root);
            Ok(Some(frame))
        }
    }
}

/// C3 — the reaction feeder on the EXTERNAL arm, through the same `wire-serve`
/// leaf the guarded write uses (`docs/laws.md` Law 3: one implementation, two
/// hosts). The before/after documents this needs are already in hand here, so the
/// feeder reads nothing back from disk.
///
/// **No actor.** An edit made behind the engine's back has no acting caller, so
/// `actor` stays absent rather than invented (§7.1) — and by the same fact there
/// is no response to carry `armed` feedback: the reaction exists only on the frame.
///
/// Only MODIFIED files feed. A birth or a death is a `Create`/`Remove` change whose
/// absent side has no document to derive from, and inventing an empty one to fill
/// the seam is a contract question this card does not own.
fn external_effects(
    ws_root: &fs::WorkspaceRoot,
    changes: &fs::WatchChanges,
) -> Result<Vec<wire::EffectEnvelope>, Box<ErrorBody>> {
    let mut effects = Vec::new();
    for (path, before, after) in &changes.modified {
        // Minted through `candidate_of_body`, not `doc_of`: a HOOK answers scope
        // against the changed file's PATH, and only this mint carries it. `doc_of`
        // leaves the document path empty, which every `paths:` glob then misses.
        let (b, a) = (doc_at(path, before)?, doc_at(path, after)?);
        // A reaction never fails the world: the edit already landed on disk and the
        // Delta describing it must still be emitted. Faults are not lost for that —
        // they ride the frame as `ArmedFault` findings, the same channel and the same
        // words the guarded write's call site carries.
        effects.extend(crate::reaction::feed_landed_change(
            ws_root,
            b.document(),
            a.document(),
            &[],
            policy::ChangeOp::Splice,
            None, // §7.1: no caller made this write — never invent one
        ));
    }
    Ok(effects)
}

/// Byte-level classification → wire Delta file entries, path-deterministic
/// order, the ruled rename form applied (module header).
fn classify_to_wire(changes: &fs::WatchChanges) -> Result<Vec<DeltaFile>, Box<ErrorBody>> {
    // The ruled rename predicate: exactly one removed + exactly one added,
    // byte-equal content — a fact, not a similarity guess.
    if changes.removed.len() == 1
        && changes.added.len() == 1
        && changes.removed[0].1 == changes.added[0].1
    {
        let (from_path, bytes) = &changes.removed[0];
        let (to_path, _) = &changes.added[0];
        let rev = NodeRev(doc_of(to_path, bytes)?.root.node_rev.0);
        let mut files = vec![DeltaFile {
            path: Path(to_path.clone()),
            change: FileChange::Renamed,
            from_path: Some(Path(from_path.clone())),
            // Same bytes, same rev, both tenses; no node entries — content
            // unchanged, the inventory is re-readable via toc (§7.1
            // never-duplicated posture).
            file_rev_before: Some(rev.clone()),
            file_rev_after: Some(rev),
            nodes: vec![],
        }];
        files.extend(modified_entries(changes)?);
        files.sort_by(|a, b| a.path.0.cmp(&b.path.0));
        return Ok(files);
    }
    // Refuse-to-guess: deletes and creates emitted as themselves.
    let mut files = Vec::new();
    for (path, before) in &changes.removed {
        let doc = doc_of(path, before)?;
        let fd = model::delta::file_delta(Some(&doc), None).expect("state changed");
        files.push(wire_map::project_file_delta(path, &fd));
    }
    for (path, after) in &changes.added {
        let doc = doc_of(path, after)?;
        let fd = model::delta::file_delta(None, Some(&doc)).expect("state changed");
        files.push(wire_map::project_file_delta(path, &fd));
    }
    files.extend(modified_entries(changes)?);
    files.sort_by(|a, b| a.path.0.cmp(&b.path.0));
    Ok(files)
}

/// Modified files: full change facts through the existing model mint points.
fn modified_entries(changes: &fs::WatchChanges) -> Result<Vec<DeltaFile>, Box<ErrorBody>> {
    let mut out = Vec::new();
    for (path, before, after) in &changes.modified {
        let (b, a) = (doc_of(path, before)?, doc_of(path, after)?);
        if let Some(fd) = model::delta::file_delta(Some(&b), Some(&a)) {
            out.push(wire_map::project_file_delta(path, &fd));
        }
    }
    Ok(out)
}

/// The §12 domain is md/UTF-8 by law — non-UTF-8 bytes refuse loud
/// (`invalid_utf8`), matching `fs::load`'s posture everywhere else.
fn utf8_of<'a>(path: &str, bytes: &'a [u8]) -> Result<&'a str, Box<ErrorBody>> {
    std::str::from_utf8(bytes).map_err(|_| {
        let mut e = ErrorBody::new(wire::ErrorCode::InvalidUtf8);
        e.path = Some(Path(path.to_string()));
        Box::new(e)
    })
}

/// Parse one domain file's bytes AT its path, through the production mint.
///
/// The twin of [`doc_of`] for consumers that need the document's own path — a
/// HOOK's `paths:` scope is matched against it, so a path-less document silently
/// matches no rule at all.
fn doc_at(path: &str, bytes: &[u8]) -> Result<model::CandidateDocument, Box<ErrorBody>> {
    Ok(model::candidate_of_body(
        path,
        utf8_of(path, bytes)?.to_string(),
    ))
}

/// Parse one domain file's bytes. The §12 domain is md/UTF-8 by law —
/// non-UTF-8 bytes refuse loud (`invalid_utf8`), matching `fs::load`'s
/// posture everywhere else.
fn doc_of(path: &str, bytes: &[u8]) -> Result<model::Document, Box<ErrorBody>> {
    let text = utf8_of(path, bytes)?;
    let tree = syntax::parse(text);
    Ok(model::build(text.to_string(), tree))
}
