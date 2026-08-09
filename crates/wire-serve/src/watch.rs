//! External-change classifier, shared by both hosts.
//! `WatchState` tracks the world at ring tip; each reconcile is three-way:
//!
//! - snapshot root == watch root → nothing;
//! - snapshot root == ring tip `root_after` → internal: sync silently
//!   (`commit_batch` already emitted);
//! - else → external: one change set = one root advance = one Delta;
//!   `actor`/`now` absent (§7.1); `seq` at detection; one production constructor.
//!
//! **Rename ruling:** within one batch, exactly one removed + exactly one added
//! path with byte-equal content → `renamed` + `from_path`. Else delete+create,
//! `from_path` unwired.
//!
//! **§52 per-file degradation** (node-rev-merkle-spec §3, line 52): a non-UTF-8
//! member's bytes already moved the root (`domain_snapshot` folds raw bytes),
//! so the delta names it — but it serves no spans/nodes, so its entry carries
//! no revs and no node entries, and it feeds no reaction. Siblings in the same
//! change set are served in full. One poison member never refuses the frame:
//! a refused reconcile would starve the subscription on every detect cycle
//! until the file is removed — the corpus-plane incident's watch-leg twin
//! (`fs::build_corpus`'s unserved slot is the corpus twin of the degraded
//! entry).
//!
//! **Stated degrade — line-boundary reconcile only** (the deleted sidecar's
//! mode; no shipping driver runs it): an external write in the same window as
//! an internal commit can break ring-chain contiguity → `root_unknown` resync
//! (§7.3: re-derive, never wrong data). The registry detector snapshots under
//! the write flock, never mid-landing.
//!
//! Shared here: the three-way disposition, the rename ruling, the wire
//! projection. Host driver: the registry, on its subscription detection cycle.

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
    /// Lets a driver decide whether a reconcile has anything to do before
    /// paying for one: the registry's detector compares a freshly folded disk
    /// root against this before taking the workspace write flock.
    #[must_use]
    pub fn root(&self) -> Option<&Root> {
        self.root.as_ref()
    }
}

/// One reconcile step. Returns the emitted external frame, if any — the caller
/// flushes it to subscribers.
///
/// The caller owns the schedule: both drivers are single-producer per ring,
/// which is what keeps the chain contiguous — a second concurrent producer
/// would duplicate a `seq` or break the root chain, and no lock inside this
/// function could repair that. The registry holds the workspace write flock so
/// no batch is observed mid-landing.
///
/// # Errors
/// A snapshot or classification failure. Loop drivers should log and retry next
/// cycle: an unreadable workspace is transient, never a reason to tear down a
/// subscription.
///
/// # Panics
/// If the watcher's baseline is unset on the arm that classifies against it —
/// unreachable by construction: that arm is guarded by `watch.root` being
/// `Some`.
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
            let delta_files = classify_to_wire(&changes);
            // The one production DeltaFrame constructor (§7.3), shared with the
            // commit path. `allocate_seq` rather than `seq() + 1`: the write
            // path can hold an allocation whose frame has not landed yet, and
            // the tip cannot see it.
            let mut frame = crate::write::assemble_delta(
                ring.allocate_seq(),
                watch_root.clone(),
                disk_root.clone(),
                None, // §7.1: actor ABSENT — never invented
                None, // §7.1: now ABSENT — never invented
                delta_files,
            );
            frame.effects = external_effects(ws_root, &changes);
            ring.advance(frame.clone());
            watch.watcher.rebase(&files);
            watch.root = Some(disk_root);
            Ok(Some(frame))
        }
    }
}

/// The reaction feeder on the external arm, through the same `wire-serve` leaf
/// the guarded write uses (`docs/laws.md` Law 3: one implementation, two
/// hosts). The before/after documents are already in hand, so the feeder reads
/// nothing back from disk.
///
/// No actor: an edit made behind the engine's back has no acting caller, so
/// `actor` stays absent rather than invented (§7.1); with no response to carry
/// `armed` feedback, the reaction exists only on the frame.
///
/// Only modified files feed: a `Create`/`Remove` change has no document on its
/// absent side to derive from. A member with a non-UTF-8 side feeds nothing —
/// a hook matches parsed documents, and an unserved member has none (§52
/// degradation, module header).
fn external_effects(
    ws_root: &fs::WorkspaceRoot,
    changes: &fs::WatchChanges,
) -> Vec<wire::EffectEnvelope> {
    let mut effects = Vec::new();
    for (path, before, after) in &changes.modified {
        // `candidate_of_body`, not `doc_of`: a hook matches `paths:` globs
        // against the document's path, which only this mint carries.
        let (Some(b), Some(a)) = (doc_at(path, before), doc_at(path, after)) else {
            continue;
        };
        // A reaction never fails the world: the edit already landed and the
        // Delta must still be emitted. Faults ride the frame as `ArmedFault`
        // findings.
        effects.extend(crate::reaction::feed_landed_change(
            ws_root,
            b.document(),
            a.document(),
            &[],
            policy::ChangeOp::Splice,
            None, // §7.1: no caller made this write — never invent one
        ));
    }
    effects
}

/// Byte-level classification → wire Delta file entries, path-deterministic
/// order, the ruled rename form applied (module header). Infallible: a
/// non-UTF-8 member degrades to its rev-less entry (§52, module header)
/// instead of refusing the frame.
fn classify_to_wire(changes: &fs::WatchChanges) -> Vec<DeltaFile> {
    if changes.removed.len() == 1
        && changes.added.len() == 1
        && changes.removed[0].1 == changes.added[0].1
    {
        let (from_path, bytes) = &changes.removed[0];
        let (to_path, _) = &changes.added[0];
        // A poison rename stays a rename — byte equality needs no parse —
        // but mints no rev (§52: no spans/nodes served).
        let rev = doc_of(bytes).map(|doc| NodeRev(doc.root.node_rev.0));
        let mut files = vec![DeltaFile {
            path: Path(to_path.clone()),
            change: FileChange::Renamed,
            from_path: Some(Path(from_path.clone())),
            // Same bytes, same rev, both tenses; no node entries — content
            // unchanged (§7.1 never-duplicated posture).
            file_rev_before: rev.clone(),
            file_rev_after: rev,
            nodes: vec![],
        }];
        files.extend(modified_entries(changes));
        files.sort_by(|a, b| a.path.0.cmp(&b.path.0));
        return files;
    }
    // Refuse-to-guess: deletes and creates emitted as themselves.
    let mut files = Vec::new();
    for (path, before) in &changes.removed {
        files.push(match doc_of(before) {
            Some(doc) => {
                let fd = model::delta::file_delta(Some(&doc), None).expect("state changed");
                wire_map::project_file_delta(path, &fd)
            }
            None => degraded_entry(path, FileChange::Deleted),
        });
    }
    for (path, after) in &changes.added {
        files.push(match doc_of(after) {
            Some(doc) => {
                let fd = model::delta::file_delta(None, Some(&doc)).expect("state changed");
                wire_map::project_file_delta(path, &fd)
            }
            None => degraded_entry(path, FileChange::Created),
        });
    }
    files.extend(modified_entries(changes));
    files.sort_by(|a, b| a.path.0.cmp(&b.path.0));
    files
}

/// Modified files: full change facts through the existing model mint points.
/// A member with a non-UTF-8 side keeps its entry but degrades it: the rev
/// survives on whichever side still parses, node grain is gone — node entries
/// need both sides, and the served tense is re-readable via `toc` (§7.1
/// never-duplicated posture).
fn modified_entries(changes: &fs::WatchChanges) -> Vec<DeltaFile> {
    let mut out = Vec::new();
    for (path, before, after) in &changes.modified {
        match (doc_of(before), doc_of(after)) {
            (Some(b), Some(a)) => {
                if let Some(fd) = model::delta::file_delta(Some(&b), Some(&a)) {
                    out.push(wire_map::project_file_delta(path, &fd));
                }
            }
            (b, a) => {
                let mut entry = degraded_entry(path, FileChange::Modified);
                entry.file_rev_before = b.map(|doc| NodeRev(doc.root.node_rev.0));
                entry.file_rev_after = a.map(|doc| NodeRev(doc.root.node_rev.0));
                out.push(entry);
            }
        }
    }
    out
}

/// The §52 degraded entry: the member changed on disk and its bytes already
/// moved the root, so the delta names it — but it serves no spans/nodes, so
/// the entry carries no revs and no node entries. A consumer asking for the
/// member itself gets the per-file `invalid_utf8` from the read doors.
fn degraded_entry(path: &str, change: FileChange) -> DeltaFile {
    DeltaFile {
        path: Path(path.to_string()),
        change,
        from_path: None,
        file_rev_before: None,
        file_rev_after: None,
        nodes: vec![],
    }
}

/// Parse one domain file's bytes AT its path, through the production mint, or
/// `None` for a non-UTF-8 member (§52: never lossy-decoded, never parsed).
///
/// The twin of [`doc_of`] for consumers that need the document's own path — a
/// HOOK's `paths:` scope is matched against it, so a path-less document silently
/// matches no rule at all.
fn doc_at(path: &str, bytes: &[u8]) -> Option<model::CandidateDocument> {
    let text = std::str::from_utf8(bytes).ok()?;
    Some(model::candidate_of_body(path, text.to_string()))
}

/// Parse one domain file's bytes, or `None` for a non-UTF-8 member (§52:
/// never lossy-decoded, never parsed — the caller degrades per-file).
fn doc_of(bytes: &[u8]) -> Option<model::Document> {
    let text = std::str::from_utf8(bytes).ok()?;
    let tree = syntax::parse(text);
    Some(model::build(text.to_string(), tree))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §52 inside ONE change set: the poison member degrades to its rev-less,
    /// node-less entry while the healthy sibling in the SAME batch keeps full
    /// node grain — degradation never leaks past the file.
    ///
    /// *Mutation:* restore `doc_of(...)?` — the whole batch refuses and the
    /// sibling's facts are lost with it.
    #[test]
    fn a_mixed_batch_degrades_only_the_poison_member() {
        let changes = fs::WatchChanges {
            removed: vec![],
            added: vec![("poison.md".into(), b"# P\n\xff\xfe\n".to_vec())],
            modified: vec![(
                "plan.md".into(),
                b"# Goals\n\nship by August\n".to_vec(),
                b"# Goals\n\nship by September\n".to_vec(),
            )],
        };
        let files = classify_to_wire(&changes);
        assert_eq!(files.len(), 2, "both members are named: {files:?}");
        let poison = files.iter().find(|f| f.path.0 == "poison.md").unwrap();
        assert_eq!(poison.change, FileChange::Created);
        assert!(
            poison.file_rev_before.is_none()
                && poison.file_rev_after.is_none()
                && poison.nodes.is_empty(),
            "the unserved member carries no revs and no node grain: {poison:?}"
        );
        let plan = files.iter().find(|f| f.path.0 == "plan.md").unwrap();
        assert_eq!(plan.change, FileChange::Modified);
        assert!(
            plan.file_rev_after.is_some() && !plan.nodes.is_empty(),
            "the healthy sibling keeps full node grain: {plan:?}"
        );
    }

    /// A poison side on a MODIFIED member keeps the entry, keeps the rev of
    /// whichever side still parses, and drops node grain — node entries need
    /// both sides.
    #[test]
    fn a_poisoned_modification_keeps_the_parseable_side_rev() {
        let changes = fs::WatchChanges {
            removed: vec![],
            added: vec![],
            modified: vec![(
                "plan.md".into(),
                b"# Goals\n\nship by August\n".to_vec(),
                b"# Goals\n\xff\xfe\n".to_vec(),
            )],
        };
        let files = classify_to_wire(&changes);
        assert_eq!(files.len(), 1, "{files:?}");
        let entry = &files[0];
        assert_eq!(entry.change, FileChange::Modified);
        assert!(
            entry.file_rev_before.is_some(),
            "the still-served tense keeps its rev: {entry:?}"
        );
        assert!(
            entry.file_rev_after.is_none() && entry.nodes.is_empty(),
            "the poison tense mints nothing: {entry:?}"
        );
    }
}
