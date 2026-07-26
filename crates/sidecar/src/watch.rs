//! F5-WATCH — external-change reconciliation at the serve-loop line
//! boundary (single-threaded by charter; no notify thread exists or
//! arrives). `WatchState` tracks the world at the ring tip; each reconcile
//! runs the three-way disposition:
//!
//! - snapshot root == watch root → nothing (a cycle finding nothing emits
//!   nothing — the empty-batch pin generalizes);
//! - snapshot root == ring tip `root_after` → an INTERNAL commit moved the
//!   world: sync silently (`commit_batch` already emitted — no double
//!   emission);
//! - else → EXTERNAL: one detected change set = one root advance = one
//!   Delta, `actor`/`now` ABSENT (§7.1: the engine never invents identity
//!   or time it wasn't given; `seq` is assigned at detection), assembled at
//!   the ONE production constructor site and stored in the ring.
//!
//! **Rename ruling (advisor, f5-watch echo — pinned AS DATA):** within one
//! detection batch, EXACTLY one removed + EXACTLY one added path with
//! byte-equal content classifies `renamed` + `from_path` (blake3/byte
//! equality is a fact in the campaign's identity vocabulary, not a
//! similarity heuristic); every other case refuses to guess and emits
//! delete+create with `from_path` unwired. Ledgered edge, recorded here: a
//! genuine delete of A plus an independent create of byte-equal B
//! classifies as renamed — indistinguishable from a rename in the fact
//! plane, and the fact plane is what we serve (013 posture). STOP tripwire
//! if a frozen worked value ever contradicts.
//!
//! **Stated degrade (no pin):** an external write landing in the same
//! dispatch window as an internal commit can break ring-chain contiguity;
//! `diff` over crossing ranges then answers `root_unknown` → full resync —
//! degrades to re-derive, never to wrong data (§7.3 posture).

use wire::{DeltaFile, DeltaFrame, ErrorBody, FileChange, NodeRev, Op, Path, Root};

use crate::ring::RootRing;

/// **The demand law.** Does this op READ the ring — the reconcile's only
/// product?
///
/// A reconcile produces exactly two things: ring frames (the Delta stream and
/// its `seq`) and the watcher's baseline. It makes NO root fresh — every arm
/// that prints a root folds its own at its own observation point
/// (`wire_serve::ambient_root`), which is why `toc`, `hello` and `read` print a
/// root here and still owe no reconcile. So the axis is the RING, not the
/// printed root: an op owes a reconcile exactly when it reads `seq` or the
/// retained batches. Ops that do not (`cat`, `extract`, `check_write`, …) cost
/// O(target) instead of two full corpus folds for a value they never read.
///
/// Freshness is unchanged because no observer's tense moves: every root a
/// client holds is still folded at the op that handed it over, and every ring
/// reader still reconciles immediately before reading the ring. The one tense
/// that does move is the epoch BASELINE — it is primed at the first ring
/// observation instead of the first line, so a `diff` anchored on a root the
/// client learned from a ring-blind op (`toc`, `hello`, `read`) before that
/// point answers `root_unknown` → resync. That is the §7.1 late law's existing
/// category and the ruled degrade direction: re-derive, never wrong data.
///
/// Exhaustive by construction — no wildcard arm. A new op does not compile
/// until someone classifies it, which is the point: a misclassification here is
/// a freshness regression no timing test would catch.
pub(crate) const fn observes_ring(op: &Op) -> bool {
    match op {
        // Ring readers: `epoch.seq()` / the retained batches ARE their answer.
        //
        // The write ops are here for a second reason — they read `epoch.seq()`
        // to number their own Delta and chain it onto the tip, so an
        // unreconciled external change must be emitted FIRST, or the two roots
        // do not meet and `diff` over the crossing range degrades to
        // `root_unknown` (the §7.3 posture, module header).
        Op::Root
        | Op::Diff { .. }
        | Op::Links { .. }
        | Op::Sub { .. }
        | Op::Splice { .. }
        | Op::Create { .. } => true,
        // Ring-blind: O(target) document reads that print no ring fact, or
        // print a root they fold themselves. `resolve` walks the corpus on its
        // own (the §4.5 walk plane — a different corpus from the §12 hash
        // domain, and not the ring). `view_path` refuses `daemon_only` before
        // touching anything.
        Op::Hello { .. }
        | Op::Toc { .. }
        | Op::Cat { .. }
        | Op::Extract { .. }
        | Op::Read { .. }
        | Op::CheckWrite { .. }
        | Op::Resolve { .. }
        | Op::ViewPath { .. } => false,
    }
}

/// Does this op MOVE the world? Only a write leaves the watcher's baseline
/// behind its own commit, and only the post-dispatch reconcile rebases it —
/// without that the next external delta chains from the PRE-commit root, which
/// is the contiguity break the module header records as a stated degrade.
/// Conservative by design: a `dry` splice moves nothing and still pays the
/// rebase, because dry-ness is a field, not an op.
pub(crate) const fn advances_ring(op: &Op) -> bool {
    match op {
        Op::Splice { .. } | Op::Create { .. } => true,
        Op::Hello { .. }
        | Op::Toc { .. }
        | Op::Cat { .. }
        | Op::Extract { .. }
        | Op::Read { .. }
        | Op::CheckWrite { .. }
        | Op::Resolve { .. }
        | Op::ViewPath { .. }
        | Op::Root
        | Op::Diff { .. }
        | Op::Links { .. }
        | Op::Sub { .. } => false,
    }
}

/// The world at the ring tip: the watcher's baseline + its folded root.
/// Unprimed until the first successful snapshot — the epoch's baseline.
pub(crate) struct WatchState {
    watcher: fs::Watcher,
    root: Option<Root>,
}

impl WatchState {
    pub(crate) fn new(root: &fs::WorkspaceRoot) -> Self {
        WatchState {
            watcher: fs::Watcher::new(root.clone()),
            root: None,
        }
    }
}

/// One reconcile step, run on DEMAND: before every line that reads the ring
/// ([`observes_ring`]), after every line that moved the world
/// ([`advances_ring`]), and on both sides of every line of a subscribed
/// session — a live subscription is a standing observer of the delta stream.
/// Returns the emitted external frame, if any — the caller flushes
/// subscriptions.
pub(crate) fn reconcile(
    ws_root: &fs::WorkspaceRoot,
    ring: &mut RootRing,
    watch: &mut WatchState,
) -> Result<Option<DeltaFrame>, Box<ErrorBody>> {
    let (files, disk_root) = wire_serve::domain_snapshot(ws_root)?;
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
            // commit path in `wire-serve` — `seq` is this epoch ring's current
            // counter, so the frame carries `seq + 1`; the caller advances.
            let frame = wire_serve::write::assemble_delta(
                ring.seq(),
                watch_root.clone(),
                disk_root.clone(),
                None, // §7.1: actor ABSENT — never invented
                None, // §7.1: now ABSENT — never invented
                delta_files,
            );
            ring.advance(frame.clone());
            watch.watcher.rebase(&files);
            watch.root = Some(disk_root);
            Ok(Some(frame))
        }
    }
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

/// Parse one domain file's bytes. The §12 domain is md/UTF-8 by law —
/// non-UTF-8 bytes refuse loud (`invalid_utf8`), matching `fs::load`'s
/// posture everywhere else.
fn doc_of(path: &str, bytes: &[u8]) -> Result<model::Document, Box<ErrorBody>> {
    let text = std::str::from_utf8(bytes).map_err(|_| {
        let mut e = ErrorBody::new(wire::ErrorCode::InvalidUtf8);
        e.path = Some(Path(path.to_string()));
        Box::new(e)
    })?;
    let tree = syntax::parse(text);
    Ok(model::build(text.to_string(), tree))
}
