//! External-change classifier, shared by the serving host.
//! `WatchState` tracks the world at ring tip; each reconcile is three-way:
//!
//! - snapshot root == watch root → nothing;
//! - snapshot root == ring tip `root_after` → internal: sync silently
//!   (`commit_batch` already emitted);
//! - else → external: one change set = one root advance = one Delta;
//!   `actor`/`now` absent (§7.1); `seq` at detection; one production constructor.
//!
//! **Rename ruling (sharpened by § A.9):** within one batch, exactly one
//! DISK-TRUE removal (the path is gone from disk) + exactly one added path
//! with byte-equal content, and no re-scope cause → `renamed` + `from_path`.
//! Else delete+create, `from_path` unwired — a still-on-disk origin is a
//! membership leave and never pairs.
//!
//! **§ A.9 re-scope honesty:** a removed path still present on disk mints
//! `unattested` (it left the ATTESTED SET), never `deleted`. When the
//! effective domain config changed between baselines, membership-only
//! changes collapse into the frame's `rescope` summary (cause + counts) and
//! the cause row rides first; the enumeration is bounded at
//! [`MAX_DELTA_FILES`] with an `overflow` count past it.
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

use wire::{DeltaFile, DeltaFrame, ErrorBody, FileChange, NodeRev, Overflow, Path, Rescope, Root};

use crate::ring::RootRing;

/// § A.9: the file-enumeration bound one frame may carry. Sized against the
/// measured r3-f9 storm: ≈158 chars per rendered row, a ≈25k-token consumer
/// cap ≈ 630 rows per drain — 128 rows per frame keeps several frames
/// deliverable in one drain. Rows past the bound (deterministic order: cause
/// first, then path-sorted) drop into `overflow.dropped`.
pub const MAX_DELTA_FILES: usize = 128;

/// The world at the ring tip: the watcher's baseline + its folded root +
/// the effective scope the baseline was taken under (§ A.9 re-scope
/// detection compares scopes semantically, never bytes).
/// Unprimed until the first successful snapshot — the epoch's baseline.
#[derive(Debug)]
pub struct WatchState {
    watcher: fs::Watcher,
    root: Option<Root>,
    scope: Option<fs::domain::Scope>,
}

impl WatchState {
    /// An unprimed watcher for `root`: the first reconcile adopts the world as
    /// its baseline and emits nothing.
    #[must_use]
    pub fn new(root: &fs::WorkspaceRoot) -> Self {
        WatchState {
            watcher: fs::Watcher::new(root.clone()),
            root: None,
            scope: None,
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
            watch.scope = Some(load_scope(ws_root)?);
            Ok(None)
        }
        Some(r) if *r == disk_root => Ok(None),
        // An internal commit moved the world — already emitted; sync silent.
        // The scope refreshes with the baseline: a face write that re-scoped
        // the domain must not be re-claimed as a cause by the NEXT external
        // batch.
        Some(_) if ring.tip_root() == Some(&disk_root) => {
            watch.watcher.rebase(&files);
            watch.root = Some(disk_root);
            watch.scope = Some(load_scope(ws_root)?);
            Ok(None)
        }
        Some(watch_root) => {
            let changes = watch
                .watcher
                .classify(&files)
                .expect("primed: watch.root is Some");
            let scope_now = load_scope(ws_root)?;
            let cause = rescope_cause(watch.scope.as_ref(), &scope_now);
            let classified = classify_to_wire(ws_root, &changes, cause.as_deref());
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
                classified.files,
            );
            frame.rescope = classified.rescope;
            frame.overflow = classified.overflow;
            frame.effects = external_effects(ws_root, &changes);
            ring.advance(frame.clone());
            watch.watcher.rebase(&files);
            watch.root = Some(disk_root);
            watch.scope = Some(scope_now);
            Ok(Some(frame))
        }
    }
}

/// The effective scope as a wire-shaped result — `io_error{cause}`, the
/// same envelope [`crate::domain_snapshot`] answers with (the snapshot this
/// cycle already took proves the config was readable an instant ago, so a
/// refusal here is the same transient class).
fn load_scope(ws_root: &fs::WorkspaceRoot) -> Result<fs::domain::Scope, Box<ErrorBody>> {
    fs::domain::Scope::load(ws_root).map_err(|e| {
        let mut err = ErrorBody::new(wire::ErrorCode::IoError);
        err.cause = Some(e.to_string());
        Box::new(err)
    })
}

/// § A.9 re-scope detection: did the effective domain configuration change
/// between the baselines? Compared as PARSED scope (config identity +
/// `Domain` semantics) — a prose-only config edit re-scopes nothing. The
/// cause is the config file whose change re-scoped the set: on a switch,
/// the file now in effect; on a removal, the departed one.
fn rescope_cause(before: Option<&fs::domain::Scope>, now: &fs::domain::Scope) -> Option<String> {
    let before = before?;
    if before.config != now.config {
        return now.config.or(before.config).map(str::to_string);
    }
    if before.domain != now.domain {
        return now.config.map(str::to_string);
    }
    None
}

/// One classification's wire outcome: the (bounded) file rows plus the § A.9
/// batch summaries.
struct Classified {
    files: Vec<DeltaFile>,
    rescope: Option<Rescope>,
    overflow: Option<Overflow>,
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
/// order (cause first under a re-scope), the ruled rename form applied
/// (module header), the § A.9 honesty laws applied at the one mint point.
/// Infallible: a non-UTF-8 member degrades to its rev-less entry (§52,
/// module header) instead of refusing the frame.
fn classify_to_wire(
    ws_root: &fs::WorkspaceRoot,
    changes: &fs::WatchChanges,
    cause: Option<&str>,
) -> Classified {
    // Disk truth splits the removed list (§ A.9): a path with anything still
    // at it on disk left the ATTESTED SET; only a path gone from disk was
    // deleted. Probed, never inferred — `symlink_metadata` so a symlink now
    // squatting the path (itself outside the domain) still counts as present.
    type Removed<'a> = Vec<&'a (String, Vec<u8>)>;
    let (gone, still): (Removed<'_>, Removed<'_>) = changes
        .removed
        .iter()
        .partition(|(path, _)| ws_root.0.join(path).symlink_metadata().is_err());

    let mut files = Vec::new();
    let mut rescope = cause.map(|c| Rescope {
        cause: Path(c.to_string()),
        unattested: 0,
        attested: 0,
    });

    // Rename ruling, sharpened by § A.9: `renamed` asserts the origin LEFT
    // THE DISK, so only a disk-true removal may pair — and never under a
    // re-scope, where an added path may have been on disk all along (entered
    // the set) and "renamed from" would be a guess.
    let mut added: Removed<'_> = changes.added.iter().collect();
    let mut gone = gone;
    if cause.is_none() && gone.len() == 1 && added.len() == 1 && gone[0].1 == added[0].1 {
        let (from_path, bytes) = gone[0];
        let (to_path, _) = added[0];
        // A poison rename stays a rename — byte equality needs no parse —
        // but mints no rev (§52: no spans/nodes served).
        let rev = doc_of(bytes).map(|doc| NodeRev(doc.root.node_rev.0));
        files.push(DeltaFile {
            path: Path(to_path.clone()),
            change: FileChange::Renamed,
            from_path: Some(Path(from_path.clone())),
            // Same bytes, same rev, both tenses; no node entries — content
            // unchanged (§7.1 never-duplicated posture).
            file_rev_before: rev.clone(),
            file_rev_after: rev,
            nodes: vec![],
        });
        gone.clear();
        added.clear();
    }

    // Disk-true deletes: emitted as themselves, full facts.
    for (path, before) in gone {
        files.push(match doc_of(before) {
            Some(doc) => {
                let fd = model::delta::file_delta(Some(&doc), None).expect("state changed");
                wire_map::project_file_delta(path, &fd)
            }
            None => degraded_entry(path, FileChange::Deleted),
        });
    }

    // Membership leaves: collapsed into the count under a re-scope,
    // per-file `unattested` rows otherwise (§ A.9).
    match &mut rescope {
        Some(summary) => summary.unattested = still.len() as u64,
        None => {
            for (path, before) in still {
                files.push(unattested_entry(path, before));
            }
        }
    }

    // Additions: under a re-scope every added path ENTERED the set and
    // collapses into the count — except the cause file's own row, which
    // states a disk fact and rides (config-change-rides-first).
    for (path, after) in added {
        if let Some(summary) = &mut rescope
            && path != &summary.cause.0
        {
            summary.attested += 1;
            continue;
        }
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
    // Config-change-rides-first is law, not sort-order luck (§ A.9).
    if let Some(summary) = &rescope
        && let Some(at) = files.iter().position(|f| f.path.0 == summary.cause.0)
    {
        files[..=at].rotate_right(1);
    }

    // § A.9's bound: the feed limits its own enumeration before any
    // transport layer can truncate it silently. Deterministic prefix rides;
    // the count is complete.
    let overflow = (files.len() > MAX_DELTA_FILES).then(|| {
        let dropped = (files.len() - MAX_DELTA_FILES) as u64;
        files.truncate(MAX_DELTA_FILES);
        Overflow { dropped }
    });

    Classified {
        files,
        rescope,
        overflow,
    }
}

/// The § A.9 `unattested` entry: the path left the attested set while its
/// file remains on disk. Departed rev kept when the departed bytes still
/// parse, no post-state rev (nothing attested exists to hash), no node grain
/// (no content changed — §7.1 never-duplicated posture).
fn unattested_entry(path: &str, before: &[u8]) -> DeltaFile {
    DeltaFile {
        path: Path(path.to_string()),
        change: FileChange::Unattested,
        from_path: None,
        file_rev_before: doc_of(before).map(|doc| NodeRev(doc.root.node_rev.0)),
        file_rev_after: None,
        nodes: vec![],
    }
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

    /// A scratch workspace root for classification tests. Files the disk
    /// probe should find are written by the test; everything else is absent.
    fn scratch_root() -> (tempfile::TempDir, fs::WorkspaceRoot) {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = fs::WorkspaceRoot(dir.path().to_path_buf());
        (dir, root)
    }

    /// §52 inside ONE change set: the poison member degrades to its rev-less,
    /// node-less entry while the healthy sibling in the SAME batch keeps full
    /// node grain — degradation never leaks past the file.
    ///
    /// *Mutation:* restore `doc_of(...)?` — the whole batch refuses and the
    /// sibling's facts are lost with it.
    #[test]
    fn a_mixed_batch_degrades_only_the_poison_member() {
        let (_dir, root) = scratch_root();
        let changes = fs::WatchChanges {
            removed: vec![],
            added: vec![("poison.md".into(), b"# P\n\xff\xfe\n".to_vec())],
            modified: vec![(
                "plan.md".into(),
                b"# Goals\n\nship by August\n".to_vec(),
                b"# Goals\n\nship by September\n".to_vec(),
            )],
        };
        let files = classify_to_wire(&root, &changes, None).files;
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
        let (_dir, root) = scratch_root();
        let changes = fs::WatchChanges {
            removed: vec![],
            added: vec![],
            modified: vec![(
                "plan.md".into(),
                b"# Goals\n\nship by August\n".to_vec(),
                b"# Goals\n\xff\xfe\n".to_vec(),
            )],
        };
        let files = classify_to_wire(&root, &changes, None).files;
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

    // ------------------------------------------------------------------
    // § A.9 re-scope honesty (dogfood r3 f9)
    // ------------------------------------------------------------------

    /// f9's word defect at the file grain: a path that left the snapshot
    /// while its file REMAINS ON DISK left the attested set — it was not
    /// deleted. The row says `unattested`: departed rev kept, no post-state
    /// rev (nothing attested exists to hash), no node grain (no content
    /// changed).
    #[test]
    fn a_removed_path_still_on_disk_is_unattested_never_deleted() {
        let (_dir, root) = scratch_root();
        std::fs::write(root.0.join("kept.md"), b"# K\n").unwrap();
        let changes = fs::WatchChanges {
            removed: vec![("kept.md".into(), b"# K\n".to_vec())],
            added: vec![],
            modified: vec![],
        };
        let out = classify_to_wire(&root, &changes, None);
        assert_eq!(out.files.len(), 1, "{:?}", out.files);
        let entry = &out.files[0];
        assert_eq!(
            entry.change,
            FileChange::Unattested,
            "the file is on disk — `deleted` would be false: {entry:?}"
        );
        assert!(
            entry.file_rev_before.is_some() && entry.file_rev_after.is_none(),
            "departed rev kept, no attested post-state: {entry:?}"
        );
        assert!(entry.nodes.is_empty(), "no content changed: {entry:?}");
        assert!(
            out.rescope.is_none(),
            "no config changed: {:?}",
            out.rescope
        );
    }

    /// The disk-true half stays exactly as it was: a removed path with
    /// nothing at it on disk is `deleted`.
    #[test]
    fn a_removed_path_gone_from_disk_stays_deleted() {
        let (_dir, root) = scratch_root();
        let changes = fs::WatchChanges {
            removed: vec![("gone.md".into(), b"# G\n".to_vec())],
            added: vec![],
            modified: vec![],
        };
        let out = classify_to_wire(&root, &changes, None);
        assert_eq!(out.files.len(), 1, "{:?}", out.files);
        assert_eq!(out.files[0].change, FileChange::Deleted);
    }

    /// The rename ruling sharpened: `renamed` asserts the origin LEFT THE
    /// DISK. A still-on-disk origin (a membership leave) byte-equal to an
    /// added path is two facts — `unattested` + `created` — never one
    /// invented rename.
    #[test]
    fn a_still_on_disk_origin_never_pairs_as_renamed() {
        let (_dir, root) = scratch_root();
        std::fs::write(root.0.join("old.md"), b"# Same\n").unwrap();
        let changes = fs::WatchChanges {
            removed: vec![("old.md".into(), b"# Same\n".to_vec())],
            added: vec![("new.md".into(), b"# Same\n".to_vec())],
            modified: vec![],
        };
        let out = classify_to_wire(&root, &changes, None);
        let kinds: Vec<_> = out
            .files
            .iter()
            .map(|f| (f.path.0.as_str(), f.change))
            .collect();
        assert!(
            !out.files.iter().any(|f| f.change == FileChange::Renamed),
            "a rename claims the origin is gone from disk — it is not: {kinds:?}"
        );
        assert_eq!(
            kinds,
            vec![
                ("new.md", FileChange::Created),
                ("old.md", FileChange::Unattested)
            ],
            "two facts, path-sorted"
        );
    }

    /// f9's flood defect: under a config-change cause the membership bulk
    /// COLLAPSES into the `rescope` counts. Rows that state disk-true or
    /// content facts still ride — the cause file's own row FIRST
    /// (config-change-rides-first is law, not sort-order luck), the true
    /// delete, the modification in full grain. No per-file row for any
    /// counted member.
    #[test]
    fn a_config_change_collapses_membership_into_one_rescope_summary() {
        let (_dir, root) = scratch_root();
        for kept in ["a.md", "b.md", "c.md"] {
            std::fs::write(root.0.join(kept), format!("# {kept}\n")).unwrap();
        }
        std::fs::create_dir_all(root.0.join("meridian")).unwrap();
        let config = b"---\nignore:\n  - \"*.md\"\n---\n# Domain\n".to_vec();
        std::fs::write(root.0.join("meridian/domain.md"), &config).unwrap();
        std::fs::write(root.0.join("d.md"), b"# D\nnew\n").unwrap();
        let changes = fs::WatchChanges {
            removed: vec![
                ("a.md".into(), b"# a.md\n".to_vec()),
                ("b.md".into(), b"# b.md\n".to_vec()),
                ("c.md".into(), b"# c.md\n".to_vec()),
                ("gone.md".into(), b"# G\n".to_vec()),
            ],
            added: vec![
                ("meridian/domain.md".into(), config),
                ("entered.md".into(), b"# E\n".to_vec()),
            ],
            modified: vec![("d.md".into(), b"# D\n".to_vec(), b"# D\nnew\n".to_vec())],
        };
        let out = classify_to_wire(&root, &changes, Some("meridian/domain.md"));
        let rescope = out.rescope.expect("a config change mints the summary");
        assert_eq!(rescope.cause.0, "meridian/domain.md");
        assert_eq!(
            (rescope.unattested, rescope.attested),
            (3, 1),
            "a/b/c left the set on-disk; entered.md entered it: {rescope:?}"
        );
        let kinds: Vec<_> = out
            .files
            .iter()
            .map(|f| (f.path.0.as_str(), f.change))
            .collect();
        assert_eq!(
            kinds,
            vec![
                ("meridian/domain.md", FileChange::Created),
                ("d.md", FileChange::Modified),
                ("gone.md", FileChange::Deleted),
            ],
            "cause first, then path-sorted disk/content facts; no membership rows"
        );
    }

    /// § A.9's bound: rows past `MAX_DELTA_FILES` drop into the counted
    /// overflow marker at ASSEMBLY — before any transport layer can truncate
    /// the enumeration silently. Deterministic: the path-sorted prefix rides.
    #[test]
    fn a_batch_past_the_bound_drops_rows_and_counts_them() {
        let (_dir, root) = scratch_root();
        let removed: Vec<(String, Vec<u8>)> = (0..MAX_DELTA_FILES + 2)
            .map(|i| (format!("gone-{i:04}.md"), b"# G\n".to_vec()))
            .collect();
        let changes = fs::WatchChanges {
            removed,
            added: vec![],
            modified: vec![],
        };
        let out = classify_to_wire(&root, &changes, None);
        assert_eq!(
            out.files.len(),
            MAX_DELTA_FILES,
            "the enumeration is bounded"
        );
        assert_eq!(
            out.overflow,
            Some(Overflow { dropped: 2 }),
            "dropped rows are counted, never silently absent"
        );
        assert_eq!(
            out.files[0].path.0, "gone-0000.md",
            "the path-sorted prefix rides"
        );
    }

    /// The storm end-to-end at the reconcile grain: a domain config landing
    /// on a live root emits ONE frame whose enumeration is the cause row,
    /// with the membership storm collapsed into the summary — never a
    /// 1,010-row `deleted` flood (dogfood r3 f9's live case).
    #[test]
    fn a_domain_config_landing_emits_one_rescope_frame_not_a_deleted_storm() {
        let (_dir, root) = scratch_root();
        std::fs::create_dir_all(root.0.join("notes")).unwrap();
        for i in 0..5 {
            std::fs::write(root.0.join(format!("notes/n{i}.md")), b"# N\n").unwrap();
        }
        std::fs::write(root.0.join("keep.md"), b"# Keep\n").unwrap();
        let mut ring = RootRing::new();
        let mut watch = WatchState::new(&root);
        assert!(
            reconcile(&root, &mut ring, &mut watch)
                .expect("prime")
                .is_none(),
            "priming emits nothing"
        );
        std::fs::create_dir_all(root.0.join("meridian")).unwrap();
        std::fs::write(
            root.0.join("meridian/domain.md"),
            b"---\nignore:\n  - \"notes/**\"\n---\n# Domain\n",
        )
        .unwrap();
        let frame = reconcile(&root, &mut ring, &mut watch)
            .expect("reconcile")
            .expect("the config landing emits one frame");
        let rescope = frame
            .rescope
            .as_ref()
            .expect("the frame carries the summary");
        assert_eq!(rescope.cause.0, "meridian/domain.md");
        assert_eq!(
            (rescope.unattested, rescope.attested),
            (5, 0),
            "the five notes left the set and remain on disk: {rescope:?}"
        );
        let kinds: Vec<_> = frame
            .delta
            .files
            .iter()
            .map(|f| (f.path.0.as_str(), f.change))
            .collect();
        assert_eq!(
            kinds,
            vec![("meridian/domain.md", FileChange::Created)],
            "the cause row is the whole enumeration"
        );
        assert!(frame.overflow.is_none(), "one row cannot overflow");
        // The next cycle is quiet: baseline AND scope rebased together.
        assert!(
            reconcile(&root, &mut ring, &mut watch)
                .expect("quiet")
                .is_none(),
            "the re-scoped world is the new baseline"
        );
    }
}
