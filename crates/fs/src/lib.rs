//! Disk truth in, atomic splices out: read/walk/watch feeding the model;
//! tmp+fsync+rename splice execution.
//!
//! # Charter
//! **Owns:** the disk boundary. Reading workspace files (refusing non-UTF-8 —
//! spans must denote exact disk bytes or splice corrupts files), walking the
//! corpus, watching for changes (rung 4), and *executing* validated splices
//! atomically (tmp + fsync + rename — meridian's write discipline relocates
//! verbatim). Feeds `model` and nothing else.
//!
//! **Never does:** writing bytes it didn't splice, caching anything to disk
//! (law 2: no snapshot files, no second database — the moment memory can't be
//! thrown away, the architecture has been violated), interpreting content.
//!
//! # Law enforcement (candidate thesis, this crate's part)
//! Write execution demands `model::ValidatedBatch` — a token only `model`'s CAS
//! validation can mint. An unvalidated write cannot reach disk by construction;
//! the splice pipeline (validate in `model`, execute here) is enforced by types,
//! not review.
//!
//! # Rungs
//! Rung 1: read + walk. Rung 2: atomic splice execution. Rung 4: watch feeds
//! the daemon's change feed.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Write};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub mod domain;
pub mod guard;

/// The workspace root every wire `path` resolves strictly inside. Constructed
/// once at process start; path-escape rejection (`bad_path`) anchors here.
#[derive(Debug, Clone)]
pub struct WorkspaceRoot(pub PathBuf);

/// Read one workspace file into a parsed document (via `syntax` + `model`).
/// Non-UTF-8 files are refused, never lossy-decoded (wire-contract §8 row 1) —
/// the refusal is `ErrorKind::InvalidData`, distinguishable from a missing
/// file (`NotFound`) so the dispatch boundary can split `invalid_utf8` from
/// `file_not_found`.
///
/// # Errors
/// I/O failure reading the file, or non-UTF-8 content (refused).
pub fn load(root: &WorkspaceRoot, rel_path: &Path) -> io::Result<model::Document> {
    let bytes = fs::read(root.0.join(rel_path))?;
    let raw = String::from_utf8(bytes).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("non-UTF-8 content refused: {e}"),
        )
    })?;
    let nodes = syntax::parse(&raw);
    Ok(model::build(raw, nodes))
}

/// Walk the corpus: every markdown file under the root, as root-relative paths,
/// sorted. This is the ADDRESSABLE set — dot-dir md files (`.github/README.md`)
/// are included, since they stay `load`-able even when ignored for hashing
/// (§12.1). Cold rebuild of the whole world model from this walk is the
/// recovery path — measured 2.16 s. Symlinks are not followed.
///
/// # Errors
/// I/O failure traversing the root.
pub fn walk(root: &WorkspaceRoot) -> io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    walk_dir(&root.0, &PathBuf::new(), &mut out)?;
    out.sort();
    Ok(out)
}

fn walk_dir(abs_dir: &Path, rel_dir: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(abs_dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let name = entry.file_name();
        let rel = rel_dir.join(&name);
        if file_type.is_dir() {
            walk_dir(&entry.path(), &rel, out)?;
        } else if file_type.is_file()
            && Path::new(&name)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("md"))
        {
            out.push(rel);
        }
    }
    Ok(())
}

/// The directory user-scope rule pages live under, relative to the user scope.
pub const USER_RULES_DIR: &str = "rules";

/// The USER rung of the registration scope ladder: every rule-page candidate
/// under the user scope, as `(page path relative to the user scope, raw bytes)`.
///
/// `anchor` is the resolved `MERIDIAN.md` path — the config plane's answer
/// (`config::resolve_path`), never a guess made here. **The user scope is the
/// directory containing that file**, and an anchor that is not an existing file
/// yields an EMPTY user layer.
///
/// # Why the anchor, and why `rules/` alone (ruled 2026-08-01)
/// A workspace bounds its own rule candidates with a declared hash domain; the
/// user scope has no such declaration, and on a real machine it is `$HOME` —
/// where a recursive markdown walk would read the operator's whole home
/// directory to answer a read-only question. So the user rung is bounded two
/// ways, and both are laws rather than optimisations:
///
/// 1. **The anchor must exist.** No `MERIDIAN.md` ⇒ no user scope ⇒ no user-layer
///    rules. The absent anchor is never widened into "walk `$HOME` and see" — a
///    machine that never declared a user scope has not implicitly declared all
///    of it.
/// 2. **Only [`USER_RULES_DIR`].** `<user-scope>/rules/**.md` is the layout
///    folder the mount law already names for this scope (a page directly inside
///    a directory named `rules` mounts at that directory's PARENT, so
///    `~/rules/x.md` governs the user scope at depth 0). Registration is still
///    by TAG — this bounds where the engine LOOKS in a directory that is not a
///    corpus, and decides nothing about what registers.
///
/// The md-only + dot-segment floor of the hash domain (§12.1) applies:
/// non-markdown files never register, and a dot-prefixed segment is outside the
/// domain at any depth. Symlinks are not followed. Paths are returned
/// `rules/…`-prefixed and sorted, so tagging them as the `policy` registration
/// layer `ScopeLayer::User` is the only thing the caller adds.
///
/// # Errors
/// I/O failure reading the `rules/` tree once the anchor and the directory are
/// both present. An absent anchor and an absent `rules/` directory are answers,
/// not failures.
pub fn user_rule_pages(anchor: &Path) -> io::Result<DomainFiles> {
    if !anchor.is_file() {
        return Ok(Vec::new());
    }
    let Some(user_scope) = anchor.parent() else {
        return Ok(Vec::new());
    };
    let rules_dir = user_scope.join(USER_RULES_DIR);
    if !rules_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut rels = Vec::new();
    walk_user_rules_dir(&rules_dir, Path::new(USER_RULES_DIR), &mut rels)?;
    rels.sort();
    let mut pages = Vec::with_capacity(rels.len());
    for rel in rels {
        let bytes = fs::read(user_scope.join(&rel))?;
        pages.push((rel.to_string_lossy().replace('\\', "/"), bytes));
    }
    Ok(pages)
}

/// The user rung's traversal: markdown files under `rules/`, dot-segments
/// declined at any depth, symlinks not followed.
fn walk_user_rules_dir(abs_dir: &Path, rel_dir: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(abs_dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let rel = rel_dir.join(&name);
        if file_type.is_dir() {
            walk_user_rules_dir(&entry.path(), &rel, out)?;
        } else if file_type.is_file()
            && Path::new(&name)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("md"))
        {
            out.push(rel);
        }
    }
    Ok(())
}

/// The §12 hash domain: the files whose bytes enter the merkle root — md-only,
/// dot-segment-ignored, and custom-ignored removed. The filter gates HASHING,
/// never `load` — an ignored md file is absent here yet still addressable by
/// path (`hash ⊂ addressable`, §12.1).
///
/// # Why this is its own traversal and not `walk().filter()`
/// Ignore-for-corpus and addressable-on-demand are DIFFERENT SETS, so they get
/// different walks. [`walk`] must keep descending everywhere — that is what
/// makes `.github/README.md` addressable (§12.1) — while this walk may decline
/// to descend at all, because nothing under a soundly-pruned directory can
/// reach the root.
///
/// Filtering after a full walk pays `stat` for every entry it then discards;
/// on a real vault the discarded majority IS the cost (the walk is
/// syscall-bound, not parse-bound). Pruning is sound only where re-inclusion
/// is impossible — [`domain::Domain::prunes_dir`] carries that proof, and
/// declines whenever a `!` rule could reach beneath.
///
/// Dot-directories are pruned structurally: [`domain::Domain::contains`] holds
/// the dot rule ABOVE custom rules precisely so no `!` can lift a dot path,
/// which makes not descending them equivalent to filtering them out.
///
/// # Errors
/// I/O failure traversing the root.
pub fn hash_domain(root: &WorkspaceRoot, domain: &domain::Domain) -> io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    walk_domain_dir(&root.0, &PathBuf::new(), domain, &mut out)?;
    out.sort();
    Ok(out)
}

fn walk_domain_dir(
    abs_dir: &Path,
    rel_dir: &Path,
    domain: &domain::Domain,
    out: &mut Vec<PathBuf>,
) -> io::Result<()> {
    for entry in fs::read_dir(abs_dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let name = entry.file_name();
        let rel = rel_dir.join(&name);
        if file_type.is_dir() {
            // Dot-segment: structurally outside the hash domain at any depth.
            if name.to_string_lossy().starts_with('.') {
                continue;
            }
            if domain.prunes_dir(&rel) {
                continue;
            }
            walk_domain_dir(&entry.path(), &rel, domain, out)?;
        } else if file_type.is_file() && domain.contains(&rel) {
            out.push(rel);
        }
    }
    Ok(())
}

/// The domain files of a workspace as `(workspace-relative path, raw bytes)`
/// pairs — the shape [`domain_snapshot`] returns and [`build_corpus`] consumes.
pub type DomainFiles = Vec<(String, Vec<u8>)>;

/// Full corpus folds run by [`domain_snapshot`] in this process.
static FOLD_COUNT: AtomicU64 = AtomicU64::new(0);

/// How many full-corpus folds [`domain_snapshot`] has run in this process.
///
/// **An instrument, not a cache** — it counts folds, it never skips one. It is
/// always on because a fold is milliseconds at corpus scale against one relaxed
/// increment, and because a host's per-request fold budget is a CONTRACT (the
/// sidecar's demand law, `sidecar::watch::observes_ring`): a budget nothing
/// asserts regresses silently back to the per-read rescan it replaced, and a
/// stopwatch cannot tell a fold that was skipped from a machine that was fast.
///
/// Process-global and monotonic: read it before and after the work under test
/// and assert the DIFFERENCE. Tests asserting exact counts must not run
/// concurrently with other folding work in the same process.
#[must_use]
pub fn fold_count() -> u64 {
    FOLD_COUNT.load(Ordering::Relaxed)
}

/// The §12 hash-domain snapshot: every domain file's bytes (as
/// `(workspace-relative path, raw bytes)`) plus the corpus [`model::MerkleRoot`]
/// folded over exactly those bytes — one read, one fold, so a consumer parses
/// the same bytes the root describes and the answer cannot drift from its stamp.
///
/// This is the CHEAP half of a resident rebuild: it reads and folds but does not
/// parse. The daemon uses the returned root as the corpus content hash — the
/// warm-engine reuse key (decision 0002 risk R5: the corpus content hash, not
/// the workspace-identity Merkle). Pass the returned files to [`build_corpus`]
/// for the parse (they are the same bytes the root folded — no second read).
///
/// # Errors
/// I/O failure loading the domain config, traversing the root, or reading a file.
pub fn domain_snapshot(root: &WorkspaceRoot) -> io::Result<(DomainFiles, model::MerkleRoot)> {
    FOLD_COUNT.fetch_add(1, Ordering::Relaxed);
    let domain = domain::Domain::load(root)?;
    let rels = hash_domain(root, &domain)?;
    let mut files = Vec::with_capacity(rels.len());
    for rel in rels {
        let bytes = fs::read(root.0.join(&rel))?;
        files.push((rel.to_string_lossy().replace('\\', "/"), bytes));
    }
    let entries: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(p, b)| (p.as_str(), b.as_slice()))
        .collect();
    let folded = model::merkle_root(&entries, domain.version());
    Ok((files, folded))
}

/// [`domain_snapshot`] over a DIFFERENT INTERVAL: the worktree snapshot with an
/// overlay of the bytes another interval carries, folded by the same domain
/// filter and the same fold.
///
/// # Why this exists — a byte check is only as wide as the interval it spans
/// [`domain_snapshot`] reads the WORKTREE. A pre-commit fence is asked about the
/// INDEX, and the two are different intervals: staging a forged file and then
/// restoring the worktree leaves a snapshot that describes bytes no commit will
/// record, and a check over it cannot speak about what is being committed. The
/// overlay is how a caller that HAS the other interval's bytes folds them
/// through the one fold instead of a second one of its own.
///
/// `overlay` is `(workspace-relative path, content)`: `Some(bytes)` replaces or
/// adds a file, `None` removes one. Entries outside the hash domain are ignored
/// here — they are not hashed in either interval — so a caller may pass whatever
/// its producer reported without filtering it first. **The reserved journal is
/// one of them** (root-excluded by named law): an interval's journal bytes are
/// read from the same overlay by the caller, never from this fold.
///
/// # Ordering is the fold's correctness, not a detail
/// The files are re-keyed by [`PathBuf`] so the emitted order is byte-for-byte
/// the order [`walk`] produces (it sorts `PathBuf`s, and so does the map). A
/// snapshot folded in any other order would hash the same content to a different
/// root, and every baseline compare against it would refuse a tree that is
/// actually current.
#[must_use]
pub fn overlay_snapshot(
    worktree: &DomainFiles,
    overlay: &[(String, Option<Vec<u8>>)],
    domain: &domain::Domain,
) -> (DomainFiles, model::MerkleRoot) {
    let mut keyed: BTreeMap<PathBuf, Vec<u8>> = worktree
        .iter()
        .map(|(rel, bytes)| (PathBuf::from(rel), bytes.clone()))
        .collect();
    for (rel, content) in overlay {
        let path = PathBuf::from(rel);
        if !domain.contains(&path) {
            continue;
        }
        match content {
            Some(bytes) => {
                keyed.insert(path, bytes.clone());
            }
            None => {
                keyed.remove(&path);
            }
        }
    }
    let files: DomainFiles = keyed
        .into_iter()
        .map(|(path, bytes)| (path.to_string_lossy().replace('\\', "/"), bytes))
        .collect();
    let entries: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(p, b)| (p.as_str(), b.as_slice()))
        .collect();
    let folded = model::merkle_root(&entries, domain.version());
    (files, folded)
}

/// Parse a [`domain_snapshot`] into the corpus name index + document map — the
/// EXPENSIVE half of a resident rebuild, and the only parser. Non-UTF-8 content
/// is refused, never lossy-decoded (§8 row 1 — the same refusal [`load`] makes):
/// the error is `ErrorKind::InvalidData`, so a caller can split `invalid_utf8`
/// from other I/O. `files` is consumed (the bytes become the documents' `raw`).
///
/// # Errors
/// Non-UTF-8 content in any file (refused).
pub fn build_corpus(
    files: DomainFiles,
) -> io::Result<(model::CorpusIndex, BTreeMap<String, model::Document>)> {
    let mut index = model::CorpusIndex::new();
    let mut docs = BTreeMap::new();
    for (rel, bytes) in files {
        let text = String::from_utf8(bytes).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("non-UTF-8 content refused: {e}"),
            )
        })?;
        let doc = model::build(text.clone(), syntax::parse(&text));
        index.insert(&rel, &doc);
        docs.insert(rel, doc);
    }
    Ok((index, docs))
}

/// A process-unique suffix source for staging paths (combined with the pid and
/// a nanosecond stamp) so concurrent or retried commits never collide on a temp
/// file name.
static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// The typed write-conflict marker (M1 TOCTOU-gap fix, D8): the splice
/// target's live disk bytes no longer equal the validated pre-image — an
/// out-of-band writer landed between validate and commit. Carried inside an
/// [`io::Error`] (via [`write_conflict`]); callers split it from ordinary I/O
/// failure with [`is_write_conflict`] and map it to their typed refusal.
#[derive(Debug)]
pub struct WriteConflict {
    /// The workspace-relative (or staged destination) path that drifted.
    pub path: PathBuf,
}

impl std::fmt::Display for WriteConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "write conflict: {} changed on disk between validate and commit — \
             refusing to splice validated spans into drifted bytes; re-read and retry",
            self.path.display()
        )
    }
}

impl std::error::Error for WriteConflict {}

/// Mint the write-conflict [`io::Error`] for `path` — the ONE constructor, so
/// the [`is_write_conflict`] split cannot drift from the mint.
#[must_use]
pub fn write_conflict(path: &Path) -> io::Error {
    io::Error::other(WriteConflict {
        path: path.to_path_buf(),
    })
}

/// The cross-process WRITE lock (M1 D9, xproc-race fix): an exclusive advisory
/// `flock(2)` on `.meridian/write.lock`, held by the wire write choke-point
/// across its whole critical section (pre-batch read → validate → verify →
/// renames), so two cooperating meridian writers — sidecar process, resident
/// registry daemon, `mrd` — can never interleave read→rename (the lost-update
/// window the in-memory CAS guards cannot see). `LOCK_NB` acquire: a held lock
/// is [`io::ErrorKind::WouldBlock`], surfaced by the caller as the fast typed
/// `workspace_busy` refusal — it never waits, so a hung holder can never make
/// callers hang. Released on drop — by an EXPLICIT unlock, not by the fd close
/// (see [`WriteLock`]'s Drop: relying on the close leaks the lock into any
/// concurrently forking subprocess).
///
/// STATED residuals: out-of-band writers (editors, git, bash) do not take this
/// lock — they are covered by DETECTION (the D8 pre-rename verify →
/// `write_conflict`), not prevention (G3). The run plane serializes on its own
/// `.meridian/run.lock`; run applies and wire splices do not serialize against
/// each other until the two planes unify on one lock file (G4, named).
///
/// `flock` locks belong to the open file description, so two independent
/// acquires contend even within one process — in-process concurrent writers
/// refuse `workspace_busy` exactly like cross-process ones.
#[derive(Debug)]
pub struct WriteLock {
    // Held open for its fd; released by the explicit `flock(LOCK_UN)` in Drop.
    file: File,
}

/// Release the lock EXPLICITLY, before the fd closes.
///
/// # Why the fd close is not enough (measured, not theoretical)
/// A `flock` lock belongs to the open file DESCRIPTION, and a `fork` duplicates
/// every descriptor. Any other thread in this process spawning any subprocess —
/// `git`, a bash task, anything — transiently holds a copy of this fd between
/// its fork and its exec, even with `FD_CLOEXEC` set (CLOEXEC acts at exec, not
/// at fork). If this guard dropped in that window, closing our fd would NOT
/// release the lock: the child's copy keeps the description alive until it
/// execs, and every other writer meanwhile refuses `workspace_busy` for a
/// critical section that already finished.
///
/// `LOCK_UN` acts on the description itself, so one unlock here releases the
/// lock no matter how many copies of the fd exist. Measured before the fix:
/// 12/60 unrelated writes refused spuriously while a sibling thread spawned
/// short-lived children. The refusal was never WRONG (`workspace_busy` is
/// contractually the Retry class), but it was avoidable noise on a door that
/// should only close for a real concurrent writer.
impl Drop for WriteLock {
    fn drop(&mut self) {
        // SAFETY: flock on a valid open fd we own; the fd outlives the call.
        unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_UN) };
    }
}

impl WriteLock {
    /// Try to acquire the exclusive write lock, creating `.meridian/` and the
    /// lockfile on first use. Never blocks: a held lock returns
    /// [`io::ErrorKind::WouldBlock`] immediately.
    ///
    /// # Errors
    /// [`io::ErrorKind::WouldBlock`] when another writer holds the lock; any
    /// other I/O failure creating or locking the lockfile (the caller maps it
    /// to a typed engine error — G2: never unwrap).
    pub fn acquire(root: &WorkspaceRoot) -> io::Result<Self> {
        let dir = root.0.join(".meridian");
        fs::create_dir_all(&dir)?;
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join("write.lock"))?;
        // SAFETY: flock on a valid open fd; the fd outlives the call.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { file })
    }
}

/// Is this I/O error the typed write-conflict refusal ([`write_conflict`])?
#[must_use]
pub fn is_write_conflict(e: &io::Error) -> bool {
    e.get_ref()
        .is_some_and(|inner| inner.downcast_ref::<WriteConflict>().is_some())
}

/// Execute a validated batch as a two-file atomic commit (contract §6.5, §6.1).
///
/// The batch is the sealed [`model::ValidatedBatch`] — the ONLY entry to disk.
/// Its private `_sealed` field means only `model`'s CAS validation mints one, so
/// an unvalidated write is unconstructable at this call site (the token demand
/// is compile-enforced, not reviewed). `content_path` receives the content edits
/// (`batch.edits`); when `batch.receipt` is `Some`, `receipt_path` MUST also be
/// `Some` and names the (distinct) receipt file that receives the append. Paths
/// are threaded separately because the seal is deliberately path-less (M4) —
/// this is the seam D4 consumes when it folds the receipt address onto the
/// batch.
///
/// # The validated pre-image — the TOCTOU-gap fix (M1 D8)
/// `expected_content` is the content file's EXACT bytes the caller validated
/// the batch against (the bytes whose offsets `batch.edits` spans index). The
/// splice SOURCE is these bytes — this function never re-reads the file to
/// splice into, so validated spans can never land in drifted bytes. Before the
/// renames commit anything, the live destination is compared against this
/// pre-image (and the receipt file against its stage-time read): a mismatch —
/// an out-of-band writer landed between validate and commit — refuses with the
/// typed [`write_conflict`] error and no file is touched. The residual window
/// (verify → rename) is stated: cooperating engine writers are serialized by
/// the write flock; out-of-band writers in that gap are a detectable-at-next-
/// read lost update, never a torn or corrupted file.
///
/// # Commit discipline — the atomic-write law (§6.5 + §14)
/// Every byte reaches disk via **tmp + fsync + rename**; no in-place write path
/// exists. Both temp files are fully written and fsync'd FIRST, then the content
/// file is renamed (committing it), then the receipt file is renamed — each
/// rename made durable by an fsync of its parent directory.
///
/// # Crash window — a STATED limit (§6.5 / §13 item 6, risk R3)
/// A crash BETWEEN the two renames lands content-without-receipt: the content
/// commit is durable, the receipt's is not yet. This window is not hidden or
/// papered over. Because each file is replaced by an atomic rename, no file is
/// ever torn — recovery is re-derive (a cold rebuild yields the correct root of
/// whatever landed, never wrong data) and the orphaned intent (content edited,
/// receipt missing) is exactly what a receipt lint finds. Multi-file atomicity
/// is a rung-3 amendment candidate, not assumed here.
///
/// # Seam contract (enforced fail-loud for D4)
/// `receipt_path` presence MUST match `batch.receipt` presence, and
/// `receipt_path` MUST differ from `content_path` — a same-file receipt would
/// let the second rename clobber the first, and the frozen text models two
/// distinct files (§6.5 "two files", §6.1 "both files"). Both violations return
/// [`io::ErrorKind::InvalidInput`] before any byte is written. The receipt
/// append must be an empty span (an EOF append) — a replacing receipt span is
/// the same `InvalidInput` refusal.
///
/// # The candidate is DEMANDED, and it must be the bytes that land (U31)
/// `candidate` is the sealed [`model::CandidateDocument`] the caller gated —
/// the document this commit is about to produce. Only `model`'s mints build
/// one, so a door that lands bytes without ever building a candidate does not
/// compile. Unlike the whole-file primitives, this one's bytes are COMPUTED
/// (batch applied to pre-image) rather than supplied, so the tie is checked:
/// a candidate whose bytes differ from the splice result is `InvalidInput`
/// before any temp is written. Without that check the parameter would be a
/// token a caller may satisfy with an unrelated document — R5's ignorable
/// helper wearing a type's clothes.
///
/// # Errors
/// The seam-contract violations above (`InvalidInput`), a candidate that is not
/// the splice result (`InvalidInput`), the typed [`write_conflict`] refusal
/// (live bytes ≠ validated pre-image — nothing landed), or any I/O failure at a
/// tmp-write, fsync, or rename step.
pub fn apply_batch(
    root: &WorkspaceRoot,
    content_path: &Path,
    receipt_path: Option<&Path>,
    batch: &model::ValidatedBatch,
    expected_content: &[u8],
    candidate: &model::CandidateDocument,
) -> io::Result<()> {
    stage_batch(
        root,
        content_path,
        receipt_path,
        batch,
        expected_content,
        candidate,
    )?
    .commit()
}

/// Birth one file: write the sealed `candidate`'s bytes to `rel_path`
/// atomically (tmp+fsync+rename, the crate's one write discipline — never in
/// place), refusing if the destination is already occupied (the `if_absent` CAS
/// at file grain, d2 §2.5 C3). Parent directories are created first — a birth
/// may name a fresh subtree.
///
/// # The candidate is DEMANDED (U31)
/// The bytes ARE [`model::CandidateDocument::raw`], so the document the caller
/// gated and the bytes that land are the same object by construction. Only
/// `model` mints a candidate, so a birth door that never built one does not
/// compile.
///
/// # `if_absent` is a logical CAS, not a hardware one (stated limit)
/// The occupancy check (`symlink_metadata`, so a symlink or a dangling link
/// counts as occupied) runs BEFORE staging, then the rename lands the bytes.
/// A concurrent writer creating the path between the check and the rename is
/// the same serialized-write-path window every CAS on this engine assumes
/// (splice's `if_root`/`if_node_rev` are checked-then-applied too); the guard
/// is a precondition, not a lock.
///
/// # Errors
/// [`io::ErrorKind::AlreadyExists`] when the destination is occupied (the
/// `if_absent` violation) — no byte is staged; any I/O failure at mkdir,
/// tmp-write, fsync, or rename.
pub fn create_file(
    root: &WorkspaceRoot,
    rel_path: &Path,
    candidate: &model::CandidateDocument,
) -> io::Result<()> {
    let dst = root.0.join(rel_path);
    if fs::symlink_metadata(&dst).is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "create refused: the path is already occupied (if_absent)",
        ));
    }
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    commit_rename(&stage_file(&dst, candidate.raw().as_bytes())?)
}

/// Death of one file: remove `rel_path`, then fsync its parent directory so the
/// deletion survives a crash (the rename-durability discipline, applied to
/// unlink). The rev-CAS (remove-what-you-read, d2 §2.5 C3) is the CALLER's — it
/// compares the read rev against the live file before calling; this only
/// executes the death.
///
/// # Errors
/// [`io::ErrorKind::NotFound`] when the path is already gone; any other I/O
/// failure at unlink or the parent fsync.
pub fn remove_file(root: &WorkspaceRoot, rel_path: &Path) -> io::Result<()> {
    let dst = root.0.join(rel_path);
    fs::remove_file(&dst)?;
    fsync_dir(dst.parent().unwrap_or_else(|| Path::new(".")))
}

/// Overwrite one existing file's whole bytes with the sealed `candidate`'s,
/// atomically (tmp+fsync+rename beside the destination — the crate's one write
/// discipline, never in place). Unlike [`create_file`] this carries NO
/// `if_absent` guard:
/// the caller (the pin lock writer, d2 §2.5) has already CAS-guarded the file's
/// read rev, so the overwrite is the committed edge of a checked write. The
/// destination's parent must exist (a whole-file overwrite never mints a fresh
/// subtree); a missing file is the caller's CAS-drift concern, surfaced here as
/// the rename's own I/O error, never silently created.
///
/// # The candidate is DEMANDED (U31)
/// As with [`create_file`], the bytes ARE
/// [`model::CandidateDocument::raw`] — gated document and landed bytes are one
/// object by construction. This closed three doors that had no candidate at
/// all: the lock writer, the pin's anchor promotion, and `mrd realise
/// --truth file`'s INDEX deploy (which reached disk through a bare
/// `std::fs::write`).
///
/// # Errors
/// Any I/O failure at tmp-write, fsync, or rename.
pub fn replace_file(
    root: &WorkspaceRoot,
    rel_path: &Path,
    candidate: &model::CandidateDocument,
) -> io::Result<()> {
    let dst = root.0.join(rel_path);
    commit_rename(&stage_file(&dst, candidate.raw().as_bytes())?)
}

/// Append one already-rendered `line` at a page's EOF, atomically (tmp+fsync+
/// rename), creating the page and its parent directories when absent. The
/// receipt engine appends journal rows through this: `fs` renders NOTHING
/// (crate charter) — the caller passes the rendered row and this only lands
/// bytes. `line` is written verbatim followed by one `\n` (the appender owns
/// terminators; a rendered row leaf excludes its own).
///
/// # Errors
/// Any I/O failure at mkdir, read, tmp-write, fsync, or rename.
pub fn append_line(root: &WorkspaceRoot, rel_path: &Path, line: &str) -> io::Result<()> {
    let dst = root.0.join(rel_path);
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut bytes = read_or_empty(&dst)?;
    bytes.extend_from_slice(line.as_bytes());
    bytes.push(b'\n');
    commit_rename(&stage_file(&dst, &bytes)?)
}

/// Read the reserved receipt journal page's bytes, or the empty string when the
/// page does not exist yet — **an absent journal IS an empty journal** (a genesis
/// workspace has never written a row). Reads raw text: the row grammar is
/// line-oriented and belongs to `receipt`, so this crate still renders and parses
/// NOTHING (crate charter) — it answers only "where the journal lives" and "what
/// absent means", the two facts it owns as the page's disk home.
///
/// **U35:** every door that appends a row must read this page first (the counter
/// is derived from it — `receipt::journal::next_seq`), so the absent-is-empty rule
/// gets one owner here instead of one copy per door.
///
/// # Errors
/// Any I/O failure other than the page being absent.
pub fn read_journal_page(root: &WorkspaceRoot) -> io::Result<String> {
    let page = root.0.join(domain::RESERVED_JOURNAL_PATH);
    match fs::read_to_string(&page) {
        Ok(text) => Ok(text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e),
    }
}

/// A two-file commit staged to temp files (written + fsync'd), awaiting the two
/// renames. Separating staging from the renames is what lets the crash-honesty
/// test drive a kill BETWEEN the renames deterministically (§6.5). Each staged
/// file carries the pre-image its new bytes were derived from — the D8
/// pre-rename verify compares the live destination against it.
struct StagedCommit {
    content: StagedFile,
    /// The content file's validated pre-image (read#2's bytes — what the
    /// sealed spans index). The live destination must still equal this at
    /// commit, or the commit refuses [`write_conflict`].
    content_expected: Vec<u8>,
    receipt: Option<StagedFile>,
    /// The receipt file's stage-time bytes (absent file ⇒ empty). Verified the
    /// same way; absence at commit still equals an empty pre-image (the first
    /// append creates the file).
    receipt_expected: Option<Vec<u8>>,
}

/// One file staged for atomic replace: the temp path holding the new bytes
/// (already fsync'd) and the destination it will be renamed onto.
struct StagedFile {
    tmp: PathBuf,
    dst: PathBuf,
}

/// Stage both files: apply each file's validated span replacements to its
/// PRE-IMAGE bytes (the content pre-image is the caller's `expected_content` —
/// read#2's validated bytes, never a fresh disk read; the receipt pre-image is
/// read here, reconciled against the append's EOF span) and write the result to
/// a fsync'd temp beside the destination. No destination is touched here —
/// staging is entirely off to the side, so a failure (or a crash) before
/// [`StagedCommit::commit`] leaves every real file intact (the property gate 2
/// checks).
fn stage_batch(
    root: &WorkspaceRoot,
    content_path: &Path,
    receipt_path: Option<&Path>,
    batch: &model::ValidatedBatch,
    expected_content: &[u8],
    candidate: &model::CandidateDocument,
) -> io::Result<StagedCommit> {
    // Seam contract, enforced BEFORE any disk write (fail-loud for D4).
    match (receipt_path, batch.receipt.as_ref()) {
        (Some(rp), Some(_)) if rp == content_path => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "receipt_path equals content_path: the two-file commit would clobber (§6.5)",
            ));
        }
        (Some(_), None) | (None, Some(_)) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "receipt_path presence must match batch.receipt presence",
            ));
        }
        _ => {}
    }

    // Content file: apply the validated span edits verbatim to the VALIDATED
    // pre-image (D8) — the spans index exactly these bytes by construction, so
    // the splice can never land in drifted bytes. The live destination is
    // verified against this pre-image at commit, before any rename.
    let content_dst = root.0.join(content_path);
    let content_new = apply_spans(
        expected_content,
        batch.edits.iter().map(|e| (&e.span, e.text.as_str())),
    );

    // The candidate must BE the splice result (U31): the document the caller
    // gated and the bytes this commit lands are one object, or the commit
    // refuses before staging a temp.
    if content_new != candidate.raw().as_bytes() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "candidate document is not this batch's splice result: the gated \
             document and the landing bytes must be the same object",
        ));
    }

    let content = stage_file(&content_dst, &content_new)?;

    // Receipt file (when named): read pre-batch bytes (absent ⇒ empty, a create),
    // reconcile the append against them, stage. On any failure the already-staged
    // content temp is cleaned up so a failed apply leaves no litter.
    let (receipt, receipt_expected) = match (receipt_path, batch.receipt.as_ref()) {
        (Some(rp), Some(append)) => {
            let receipt_dst = root.0.join(rp);
            match stage_receipt(&receipt_dst, append) {
                Ok((staged, old)) => (Some(staged), Some(old)),
                Err(e) => {
                    let _ = fs::remove_file(&content.tmp);
                    return Err(e);
                }
            }
        }
        _ => (None, None),
    };

    Ok(StagedCommit {
        content,
        content_expected: expected_content.to_vec(),
        receipt,
        receipt_expected,
    })
}

/// Stage the receipt file: read its pre-batch bytes (missing ⇒ empty),
/// reconcile the append span against them, and stage the appended bytes.
/// Returns the staged handle plus the pre-image read (the commit's verify
/// baseline). Factored out so content-temp cleanup on error has one site.
///
/// # The append reconcile (D8, receipt half)
/// The append span was computed as `len..len` when the receipt line was
/// rendered — against the receipt file as it stood THEN. A non-empty span is
/// seam misuse (`InvalidInput`); an empty span whose offset no longer equals
/// the live length means the receipt file moved between render and commit (an
/// out-of-band append or truncation) — the typed [`write_conflict`] refusal,
/// never a blind splice that would misplace the row (or panic on a shrunk
/// file).
fn stage_receipt(
    receipt_dst: &Path,
    append: &model::ReceiptAppend,
) -> io::Result<(StagedFile, Vec<u8>)> {
    if append.span.start != append.span.end {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "receipt append span must be empty (an EOF append), not a replacement",
        ));
    }
    let old = read_or_empty(receipt_dst)?;
    if append.span.start != old.len() {
        return Err(write_conflict(receipt_dst));
    }
    let new = apply_spans(&old, std::iter::once((&append.span, append.text.as_str())));
    let staged = stage_file(receipt_dst, &new)?;
    Ok((staged, old))
}

/// Read a file's bytes, treating a missing file as empty — the receipt file may
/// not exist before its first entry, and the append then creates it.
fn read_or_empty(path: &Path) -> io::Result<Vec<u8>> {
    match fs::read(path) {
        Ok(bytes) => Ok(bytes),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

/// Write `bytes` to a fresh temp file beside `dst` (same directory ⇒ same
/// filesystem, so the eventual rename is atomic), fsync it, and hand back the
/// staged handle. The temp name is non-`.md` and dot-prefixed so `walk` and the
/// §12 hash domain both ignore it. This is the ENTIRE write path — nothing is
/// ever written in place.
fn stage_file(dst: &Path, bytes: &[u8]) -> io::Result<StagedFile> {
    let tmp = temp_path_for(dst);
    if let Err(e) = write_fsync(&tmp, bytes) {
        let _ = fs::remove_file(&tmp); // hygiene: no orphan temp on a failed stage
        return Err(e);
    }
    Ok(StagedFile {
        tmp,
        dst: dst.to_path_buf(),
    })
}

/// Create `tmp`, write all `bytes`, and fsync so the new bytes are durable
/// BEFORE the rename makes them visible (the "fsync" of tmp+fsync+rename).
fn write_fsync(tmp: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut f = File::create(tmp)?;
    f.write_all(bytes)?;
    f.sync_all()
}

/// A staging path beside `dst`: `<dir>/.<name>.<pid>.<nanos>.<seq>.tmp`. Same
/// directory (atomic rename), dot-prefixed with a `.tmp` extension (outside both
/// the `.md` walk and the dot-segment domain ignore), unique per call.
fn temp_path_for(dst: &Path) -> PathBuf {
    let dir = dst.parent().unwrap_or_else(|| Path::new("."));
    let name = dst.file_name().and_then(|n| n.to_str()).unwrap_or("tmp");
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    dir.join(format!(".{name}.{pid}.{nanos}.{seq}.tmp"))
}

/// Apply disjoint, ascending-sorted byte-span replacements to `old`, returning
/// the new bytes. The spans index `old` (the pre-batch bytes); `model` has
/// already validated disjointness and ordering (§4.4), so this is a single
/// linear pass. Bytes are spliced VERBATIM — `fs` never interprets or reformats
/// content (crate charter).
fn apply_spans<'a>(
    old: &[u8],
    edits: impl Iterator<Item = (&'a model::ByteSpan, &'a str)>,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(old.len());
    let mut cursor = 0usize;
    for (span, text) in edits {
        out.extend_from_slice(&old[cursor..span.start]);
        out.extend_from_slice(text.as_bytes());
        cursor = span.end;
    }
    out.extend_from_slice(&old[cursor..]);
    out
}

impl StagedCommit {
    /// Commit both files: verify both live destinations still equal their
    /// pre-images (the D8 final pre-rename check — refuse [`write_conflict`]
    /// on drift, cleaning the staged temps), then rename the content file
    /// (which COMMITS it), then the receipt file. The gap between the two
    /// renames is the STATED §6.5 crash window; nothing here narrows it away —
    /// it is honestly the limit. The verify→rename gap is the D8 residual
    /// window: cooperating writers are serialized by the write flock;
    /// out-of-band writers in that gap lose their update detectably (each file
    /// is still fully-old-or-fully-new — never torn).
    fn commit(self) -> io::Result<()> {
        if let Err(conflict) = self.verify_pre_images() {
            self.discard();
            return Err(conflict);
        }
        self.rename_content()?;
        // ┄┄ §6.5 crash window: a crash HERE lands content-without-receipt ┄┄
        self.rename_receipt()
    }

    /// The D8 verify: the content destination must still hold the validated
    /// pre-image (gone ⇒ conflict too — read#2 saw a real file), and the
    /// receipt destination must still hold its stage-time bytes (absent stays
    /// legal only while the pre-image is empty — the first append creates it).
    /// Both checks run BEFORE the first rename, so a refusal commits nothing.
    fn verify_pre_images(&self) -> io::Result<()> {
        let live = match fs::read(&self.content.dst) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Err(write_conflict(&self.content.dst));
            }
            Err(e) => return Err(e),
        };
        if live != self.content_expected {
            return Err(write_conflict(&self.content.dst));
        }
        if let (Some(staged), Some(expected)) = (&self.receipt, &self.receipt_expected)
            && read_or_empty(&staged.dst)? != *expected
        {
            return Err(write_conflict(&staged.dst));
        }
        Ok(())
    }

    /// Remove the staged temps (hygiene on a refused commit — no litter).
    fn discard(&self) {
        let _ = fs::remove_file(&self.content.tmp);
        if let Some(staged) = &self.receipt {
            let _ = fs::remove_file(&staged.tmp);
        }
    }

    /// Rename the content temp onto its destination (atomic) and fsync the
    /// destination's directory so the rename itself is durable.
    fn rename_content(&self) -> io::Result<()> {
        commit_rename(&self.content)
    }

    /// Rename the receipt temp (when one was staged) onto its destination and
    /// fsync its directory. A batch without a receipt is a no-op here.
    fn rename_receipt(&self) -> io::Result<()> {
        match &self.receipt {
            Some(staged) => commit_rename(staged),
            None => Ok(()),
        }
    }
}

/// Rename a staged temp onto its destination (a POSIX-atomic replace) and fsync
/// the parent directory so the rename survives a crash.
fn commit_rename(staged: &StagedFile) -> io::Result<()> {
    fs::rename(&staged.tmp, &staged.dst)?;
    fsync_dir(staged.dst.parent().unwrap_or_else(|| Path::new(".")))
}

/// Fsync a directory so a just-completed rename (a directory metadata change) is
/// durable. An empty path means the destination carried no parent component (the
/// current directory); there is nothing to sync.
fn fsync_dir(dir: &Path) -> io::Result<()> {
    if dir.as_os_str().is_empty() {
        return Ok(());
    }
    File::open(dir)?.sync_all()
}

/// Filesystem watcher (rung 5, F5-WATCH): the DETECTION primitive — a §12
/// domain baseline plus byte-level change classification against a fresh
/// snapshot. The watcher detects; root folding is `model`'s, Delta emission
/// is the sidecar's, and hook *dispatch* (running agent work on change)
/// stays Go — Rust never executes agent work.
#[derive(Debug)]
pub struct Watcher {
    _root: WorkspaceRoot,
    baseline: Option<BTreeMap<String, Vec<u8>>>,
}

/// One detection cycle's byte-level classification: paths gone from the
/// baseline, paths new on disk, and paths whose bytes moved — each with the
/// bytes both tenses need. Classification only; meaning (rename ruling,
/// Delta facts) is the caller's.
#[derive(Debug, Default)]
pub struct WatchChanges {
    /// `(path, baseline bytes)` — in the baseline, absent from the snapshot.
    pub removed: Vec<(String, Vec<u8>)>,
    /// `(path, snapshot bytes)` — on disk, absent from the baseline.
    pub added: Vec<(String, Vec<u8>)>,
    /// `(path, baseline bytes, snapshot bytes)` — present in both, bytes differ.
    pub modified: Vec<(String, Vec<u8>, Vec<u8>)>,
}

impl Watcher {
    #[must_use]
    pub fn new(root: WorkspaceRoot) -> Self {
        Watcher {
            _root: root,
            baseline: None,
        }
    }

    /// Adopt a snapshot as the new baseline (priming, post-emission, or the
    /// silent internal-commit sync — the caller's disposition).
    pub fn rebase(&mut self, snapshot: &[(String, Vec<u8>)]) {
        self.baseline = Some(snapshot.iter().cloned().collect());
    }

    /// Classify a snapshot against the baseline. `None` until primed — the
    /// epoch's baseline is the first successful snapshot; earlier external
    /// changes are unobservable by construction.
    #[must_use]
    pub fn classify(&self, snapshot: &[(String, Vec<u8>)]) -> Option<WatchChanges> {
        let baseline = self.baseline.as_ref()?;
        let disk: BTreeMap<&str, &[u8]> = snapshot
            .iter()
            .map(|(p, b)| (p.as_str(), b.as_slice()))
            .collect();
        let mut changes = WatchChanges::default();
        for (path, before) in baseline {
            match disk.get(path.as_str()) {
                None => changes.removed.push((path.clone(), before.clone())),
                Some(after) if *after != before.as_slice() => {
                    changes
                        .modified
                        .push((path.clone(), before.clone(), after.to_vec()));
                }
                Some(_) => {}
            }
        }
        for (path, bytes) in snapshot {
            if !baseline.contains_key(path) {
                changes.added.push((path.clone(), bytes.clone()));
            }
        }
        Some(changes)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        TEMP_SEQ, USER_RULES_DIR, WorkspaceRoot, apply_batch, is_write_conflict, stage_batch,
        temp_path_for, user_rule_pages, walk,
    };
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::Ordering;

    // Frozen §0.3 S0 content bytes (the worked fixture). The edit under test is
    // E3's: Q3 "ship by August" → "ship by September".
    const PLAN_S0: &str = "---\ntitle: Plan\n---\n# Goals\n\nShip the contract.\n\n## Q3\n\nship by August\n\n## Q4\n\n- item one\n- see [[2026-07-18]]\n- blocked on [[roadmap]]\n";
    // A small receipt file the append lands into (its own bytes are not
    // contract-pinned — this unit tests the commit mechanics, not §6.3 rendering,
    // which is RC4's gate).
    const RECEIPT_OLD: &str = "# Receipts\n";
    const RECEIPT_LINE: &str = "- splice notes/plan.md edits=1 Goals>Q3 match ^r-000099\n";
    const RECEIPT_ANCHOR: &str = "r-000099";

    fn content_rel() -> PathBuf {
        PathBuf::from("notes/plan.md")
    }
    fn receipt_rel() -> PathBuf {
        PathBuf::from("receipts/log.md")
    }

    /// A temp workspace holding the content + receipt files at known bytes.
    fn workspace() -> (tempfile::TempDir, WorkspaceRoot) {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join("notes")).unwrap();
        fs::create_dir_all(dir.path().join("receipts")).unwrap();
        fs::write(dir.path().join(content_rel()), PLAN_S0).unwrap();
        fs::write(dir.path().join(receipt_rel()), RECEIPT_OLD).unwrap();
        let root = WorkspaceRoot(dir.path().to_path_buf());
        (dir, root)
    }

    fn q3_september() -> model::Edit {
        model::Edit {
            target: model::Ref::Hpath(vec![
                model::HpathSeg {
                    h: "Goals".into(),
                    n: None,
                },
                model::HpathSeg {
                    h: "Q3".into(),
                    n: None,
                },
            ]),
            edit: model::EditKind::Match {
                old: "ship by August".into(),
                new: "ship by September".into(),
            },
            if_node_rev: None,
        }
    }

    fn receipt_append() -> model::ReceiptAppend {
        model::ReceiptAppend {
            span: RECEIPT_OLD.len()..RECEIPT_OLD.len(), // EOF append
            text: RECEIPT_LINE.to_string(),
        }
    }

    /// Obtain a sealed `ValidatedBatch` — the ONLY way is through `model`'s
    /// `validate_batch` (the seal is the sole entry; there is no other
    /// constructor). This is the token-demand chain gate 3 rests on.
    fn validated(receipt: Option<model::ReceiptAppend>) -> model::ValidatedBatch {
        let doc = model::build(PLAN_S0.to_string(), syntax::parse(PLAN_S0));
        let req = model::SpliceRequest {
            if_root: None,
            edits: vec![q3_september()],
            engine: None,
        };
        match model::validate_batch(&doc, None, &req, receipt) {
            model::SpliceVerdict::Validated(vb) => vb,
            other => panic!("fixture batch must validate, got {other:?}"),
        }
    }

    /// The sealed candidate the three byte-landing primitives DEMAND (U31),
    /// for this fixture's batch over `PLAN_S0`. Only `model` mints one — these
    /// tests cannot fabricate a candidate any more than a production door can.
    fn candidate(vb: &model::ValidatedBatch) -> model::CandidateDocument {
        model::candidate_of_batch("notes/plan.md", PLAN_S0, vb)
    }

    /// The receipt lint (§6.5 recovery): does the receipt file record the anchor
    /// a committed batch should have written? A TEST helper only — `fs` never
    /// interprets content (crate charter), and §6.4 puts the production lint in
    /// the policy/Go layer. This is what makes the crash-window orphan LOUD.
    fn receipt_recorded(receipt_bytes: &[u8], anchor: &str) -> bool {
        let needle = format!("^{anchor}");
        std::str::from_utf8(receipt_bytes).is_ok_and(|s| s.contains(&needle))
    }

    /// Cold rebuild: walk the workspace and re-derive the merkle root purely from
    /// on-disk bytes — the §6.5 recovery path (memory is disposable, disk is
    /// truth). Staged `.tmp` files are non-`.md`, so `walk` never sees them.
    fn cold_root(root: &WorkspaceRoot) -> model::MerkleRoot {
        let files: Vec<(String, Vec<u8>)> = walk(root)
            .unwrap()
            .iter()
            .map(|rel| {
                let bytes = fs::read(root.0.join(rel)).unwrap();
                (rel.to_string_lossy().replace('\\', "/"), bytes)
            })
            .collect();
        let refs: Vec<(&str, &[u8])> = files
            .iter()
            .map(|(p, b)| (p.as_str(), b.as_slice()))
            .collect();
        model::merkle_root(&refs, 0)
    }

    fn any_tmp_in(dir: &Path) -> bool {
        fs::read_dir(dir)
            .unwrap()
            .filter_map(Result::ok)
            .any(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
    }

    /// GATE 1 (§6.5 crash honesty): a crash injected BETWEEN the two renames
    /// leaves content-without-receipt; a cold rebuild yields the correct root;
    /// the receipt lint finds the orphan intent.
    #[test]
    fn gate1_crash_between_renames_is_honest() {
        let (dir, root) = workspace();
        let vb = validated(Some(receipt_append()));

        // Stage both temps, commit ONLY the content rename, then "crash" before
        // the receipt rename (drop the staged commit — the receipt is never
        // renamed). This is the §6.5 window, driven deterministically.
        let staged = stage_batch(
            &root,
            &content_rel(),
            Some(&receipt_rel()),
            &vb,
            PLAN_S0.as_bytes(),
            &candidate(&vb),
        )
        .unwrap();
        staged.rename_content().unwrap();
        drop(staged);

        let content_disk = fs::read(dir.path().join(content_rel())).unwrap();
        let receipt_disk = fs::read(dir.path().join(receipt_rel())).unwrap();

        // (a) content COMMITTED, fully — atomic rename, never torn. Independent
        // oracle: std `str::replace`, not this crate's splice code.
        let expected_content = PLAN_S0.replace("ship by August", "ship by September");
        assert_eq!(
            content_disk,
            expected_content.as_bytes(),
            "content file is the full new bytes (atomic rename, never torn)"
        );
        // (b) receipt did NOT land: content-without-receipt (the stated window).
        assert_eq!(
            receipt_disk,
            RECEIPT_OLD.as_bytes(),
            "receipt file untouched — the crash window is content-without-receipt"
        );

        // (c) cold rebuild → correct root, never wrong data: the root re-derived
        // by walking the disk equals the honest root of exactly {content-new,
        // receipt-old}. Each file is fully-old-or-fully-new, so re-derive is honest.
        let honest = model::merkle_root(
            &[
                ("notes/plan.md", expected_content.as_bytes()),
                ("receipts/log.md", RECEIPT_OLD.as_bytes()),
            ],
            0,
        );
        assert_eq!(
            cold_root(&root),
            honest,
            "cold rebuild yields the correct root of the landed state"
        );

        // (d) the receipt lint finds the orphan: content edited, anchor absent.
        assert!(
            !receipt_recorded(&receipt_disk, RECEIPT_ANCHOR),
            "receipt lint finds the orphan intent (anchor missing from the receipt)"
        );
    }

    /// GATE 2 (tmp+fsync+rename, no in-place path): a crash BEFORE the content
    /// rename leaves the destination fully intact — the new bytes live only in a
    /// staged `.tmp`. An in-place writer would show new/partial bytes at the
    /// destination; this proves the write never touches the file in place.
    #[test]
    fn gate2_no_in_place_write_before_rename() {
        let (dir, root) = workspace();
        let vb = validated(Some(receipt_append()));

        let staged = stage_batch(
            &root,
            &content_rel(),
            Some(&receipt_rel()),
            &vb,
            PLAN_S0.as_bytes(),
            &candidate(&vb),
        )
        .unwrap();

        // Destinations untouched while the batch is staged (pre-rename "crash").
        assert_eq!(
            fs::read(dir.path().join(content_rel())).unwrap(),
            PLAN_S0.as_bytes(),
            "content destination UNTOUCHED before rename — no in-place write"
        );
        assert_eq!(
            fs::read(dir.path().join(receipt_rel())).unwrap(),
            RECEIPT_OLD.as_bytes(),
            "receipt destination UNTOUCHED before rename — no in-place write"
        );
        // The new bytes exist, but only in staged temps beside each destination.
        assert!(
            any_tmp_in(&dir.path().join("notes")),
            "content new bytes live in a staged .tmp (tmp+fsync+rename)"
        );
        assert!(
            any_tmp_in(&dir.path().join("receipts")),
            "receipt new bytes live in a staged .tmp (tmp+fsync+rename)"
        );
        drop(staged);
    }

    /// The full two-file commit: both renames land — content edited AND receipt
    /// appended verbatim — and the lint now finds the anchor (no orphan). No
    /// staged temps survive a clean commit.
    #[test]
    fn full_commit_lands_both_files() {
        let (dir, root) = workspace();
        let vb = validated(Some(receipt_append()));

        apply_batch(
            &root,
            &content_rel(),
            Some(&receipt_rel()),
            &vb,
            PLAN_S0.as_bytes(),
            &candidate(&vb),
        )
        .unwrap();

        assert_eq!(
            fs::read(dir.path().join(content_rel())).unwrap(),
            PLAN_S0
                .replace("ship by August", "ship by September")
                .as_bytes(),
        );
        assert_eq!(
            fs::read(dir.path().join(receipt_rel())).unwrap(),
            format!("{RECEIPT_OLD}{RECEIPT_LINE}").as_bytes(),
            "receipt line appended at EOF, verbatim"
        );
        assert!(
            receipt_recorded(
                &fs::read(dir.path().join(receipt_rel())).unwrap(),
                RECEIPT_ANCHOR
            ),
            "after a full commit the receipt records the anchor — no orphan"
        );
        assert!(!any_tmp_in(&dir.path().join("notes")));
        assert!(!any_tmp_in(&dir.path().join("receipts")));
    }

    /// A receipt-less batch commits the content file alone (receipts are
    /// per-request, §6.1) — `receipt_path` `None` pairs with `batch.receipt`
    /// `None`.
    #[test]
    fn content_only_commit() {
        let (dir, root) = workspace();
        let vb = validated(None);

        apply_batch(
            &root,
            &content_rel(),
            None,
            &vb,
            PLAN_S0.as_bytes(),
            &candidate(&vb),
        )
        .unwrap();

        assert_eq!(
            fs::read(dir.path().join(content_rel())).unwrap(),
            PLAN_S0
                .replace("ship by August", "ship by September")
                .as_bytes(),
        );
        // The receipt file is untouched (no receipt in the batch).
        assert_eq!(
            fs::read(dir.path().join(receipt_rel())).unwrap(),
            RECEIPT_OLD.as_bytes()
        );
    }

    /// GATE 3 support + seam contract: the batch reaching `fs` is the sealed
    /// token (obtained only via `validate_batch`), and the presence-mismatch /
    /// same-file guards refuse fail-loud BEFORE any write.
    #[test]
    fn seam_contract_guards_fail_loud() {
        let (_dir, root) = workspace();
        let with_receipt = validated(Some(receipt_append())); // batch.receipt = Some
        let no_receipt = validated(None); // batch.receipt = None

        // (1) batch has a receipt but no path supplied.
        assert_eq!(
            apply_batch(
                &root,
                &content_rel(),
                None,
                &with_receipt,
                PLAN_S0.as_bytes(),
                &candidate(&with_receipt),
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::InvalidInput,
        );
        // (2) path supplied but the batch has no receipt.
        assert_eq!(
            apply_batch(
                &root,
                &content_rel(),
                Some(&receipt_rel()),
                &no_receipt,
                PLAN_S0.as_bytes(),
                &candidate(&no_receipt),
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::InvalidInput,
        );
        // (3) same-file receipt would clobber the content rename.
        assert_eq!(
            apply_batch(
                &root,
                &content_rel(),
                Some(&content_rel()),
                &with_receipt,
                PLAN_S0.as_bytes(),
                &candidate(&with_receipt),
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::InvalidInput,
        );
    }

    /// U31 (the seal is not decoration): `apply_batch`'s candidate must BE the
    /// splice result. The whole-file primitives get this by construction — their
    /// bytes ARE the candidate's — but this one's bytes are computed from the
    /// batch, so a caller could otherwise satisfy the type with any document it
    /// happened to hold and the gates would have judged something else.
    ///
    /// # Both halves, per S3-R8(c)
    /// The refusal is only meaningful next to its acceptance: the SAME call with
    /// the true candidate commits. A guard proven only by what it blocks is
    /// indistinguishable from one that blocks everything.
    #[test]
    fn apply_batch_refuses_a_candidate_that_is_not_the_splice_result() {
        let (dir, root) = workspace();
        let vb = validated(None);

        // An honestly-minted candidate — of the WRONG bytes. `model` built it;
        // nothing about the type is forged. Only the tie is broken.
        let impostor = model::candidate_of_body("notes/plan.md", "# Not this batch\n".to_owned());
        let err = apply_batch(
            &root,
            &content_rel(),
            None,
            &vb,
            PLAN_S0.as_bytes(),
            &impostor,
        )
        .expect_err("a candidate that is not the splice result must refuse");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(!is_write_conflict(&err), "misuse is not the conflict class");
        assert_eq!(
            fs::read(dir.path().join(content_rel())).unwrap(),
            PLAN_S0.as_bytes(),
            "the refusal committed NOTHING — not even a staged temp reached the destination"
        );
        assert!(!any_tmp_in(&dir.path().join("notes")));

        // THE ACCEPTANCE (S3-R8(c)): the identical call with the true candidate
        // lands — so the refusal above discriminates rather than blocks.
        apply_batch(
            &root,
            &content_rel(),
            None,
            &vb,
            PLAN_S0.as_bytes(),
            &candidate(&vb),
        )
        .expect("the true candidate commits");
        assert_eq!(
            fs::read(dir.path().join(content_rel())).unwrap(),
            PLAN_S0
                .replace("ship by August", "ship by September")
                .as_bytes(),
        );
    }

    /// d2 §2.5 C3 birth: `create_file` lands a NEW file's bytes (tmp+fsync+
    /// rename), makes its parent subtree, and `walk` then sees it.
    #[test]
    fn create_file_births_a_new_file() {
        let (dir, root) = workspace();
        super::create_file(
            &root,
            Path::new("births/fresh.md"),
            &model::candidate_of_body("births/fresh.md", "# Fresh\n".to_owned()),
        )
        .unwrap();
        assert_eq!(
            fs::read(dir.path().join("births/fresh.md")).unwrap(),
            b"# Fresh\n",
        );
        assert!(
            walk(&root)
                .unwrap()
                .contains(&PathBuf::from("births/fresh.md")),
            "the born file is addressable via walk"
        );
        // no staged temp survives a clean birth.
        assert!(!any_tmp_in(&dir.path().join("births")));
    }

    /// The `if_absent` CAS at the disk edge: `create_file` on an occupied path
    /// refuses `AlreadyExists` and leaves the existing bytes untouched (no
    /// clobber, no partial write).
    #[test]
    fn create_file_refuses_when_occupied() {
        let (dir, root) = workspace();
        let err = super::create_file(
            &root,
            &content_rel(),
            &model::candidate_of_body("notes/plan.md", "OVERWRITE".to_owned()),
        )
        .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(
            fs::read(dir.path().join(content_rel())).unwrap(),
            PLAN_S0.as_bytes(),
            "the occupant is untouched — the birth refused before any byte",
        );
    }

    /// d2 §2.5 C3 death: `remove_file` deletes the file (walk no longer sees
    /// it); removing an absent path is `NotFound`.
    #[test]
    fn remove_file_deletes_then_missing_is_not_found() {
        let (_dir, root) = workspace();
        super::remove_file(&root, &content_rel()).unwrap();
        assert!(
            !walk(&root).unwrap().contains(&content_rel()),
            "the removed file is gone from walk"
        );
        assert_eq!(
            super::remove_file(&root, &content_rel())
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound,
            "removing what is already gone is NotFound",
        );
    }

    /// `append_line` creates the page on first append (parent dirs and all),
    /// then appends at EOF — verbatim line plus one `\n` terminator each.
    #[test]
    fn append_line_creates_then_appends_at_eof() {
        let (dir, root) = workspace();
        let journal = Path::new("meridian/journal.md");
        super::append_line(&root, journal, "- op=create ^r-000001").unwrap();
        super::append_line(&root, journal, "- op=remove ^r-000002").unwrap();
        assert_eq!(
            fs::read(dir.path().join(journal)).unwrap(),
            b"- op=create ^r-000001\n- op=remove ^r-000002\n",
            "each rendered row is appended verbatim with one terminator",
        );
    }

    // ---- D8 TOCTOU-gap fix: external-writer conflicts, driven ----
    // ---- DETERMINISTICALLY through the stage/commit seam (A-C2: ----
    // ---- the replay harness cannot contain these interleaves).  ----

    /// D8 GATE (external overwrite): an out-of-band writer replaces the content
    /// file between staging (validate) and the rename — the commit refuses the
    /// typed write-conflict, the external bytes SURVIVE untouched (never
    /// clobbered by stale validated spans), the receipt never lands, and no
    /// staged temp litters the tree. Pre-fix behavior: the stale splice landed
    /// over the external bytes silently.
    #[test]
    fn d8_external_overwrite_refuses_write_conflict() {
        const EXTERNAL: &str = "# Rewritten by an external editor\n";
        let (dir, root) = workspace();
        let vb = validated(Some(receipt_append()));
        let staged = stage_batch(
            &root,
            &content_rel(),
            Some(&receipt_rel()),
            &vb,
            PLAN_S0.as_bytes(),
            &candidate(&vb),
        )
        .unwrap();

        // The out-of-band writer lands AFTER validate/stage, BEFORE the rename.
        fs::write(dir.path().join(content_rel()), EXTERNAL).unwrap();

        let err = staged.commit().expect_err("drifted bytes must refuse");
        assert!(
            is_write_conflict(&err),
            "the refusal is the TYPED write-conflict, not a generic io error: {err}"
        );
        assert_eq!(
            fs::read(dir.path().join(content_rel())).unwrap(),
            EXTERNAL.as_bytes(),
            "the external writer's bytes survive — nothing was renamed over them"
        );
        assert_eq!(
            fs::read(dir.path().join(receipt_rel())).unwrap(),
            RECEIPT_OLD.as_bytes(),
            "the receipt never lands on a refused commit (no half-commit)"
        );
        assert!(
            !any_tmp_in(&dir.path().join("notes")) && !any_tmp_in(&dir.path().join("receipts")),
            "a refused commit cleans its staged temps"
        );
    }

    /// D8 GATE (external delete): the content file VANISHES between staging and
    /// the rename. A blind rename would silently resurrect the (stale-derived)
    /// bytes over the deletion — instead the commit refuses the typed conflict
    /// and the path stays deleted.
    #[test]
    fn d8_external_delete_refuses_write_conflict() {
        let (dir, root) = workspace();
        let vb = validated(None);
        let staged = stage_batch(
            &root,
            &content_rel(),
            None,
            &vb,
            PLAN_S0.as_bytes(),
            &candidate(&vb),
        )
        .unwrap();

        fs::remove_file(dir.path().join(content_rel())).unwrap();

        let err = staged.commit().expect_err("a vanished target must refuse");
        assert!(is_write_conflict(&err), "typed write-conflict: {err}");
        assert!(
            !dir.path().join(content_rel()).exists(),
            "the deletion survives — the commit did not resurrect stale bytes"
        );
        assert!(!any_tmp_in(&dir.path().join("notes")));
    }

    /// D8 GATE (receipt moved before staging): the receipt file gained rows
    /// between the append's render (span = len..len against the THEN bytes)
    /// and the commit. Blind application would land the row MID-file; the
    /// reconcile refuses the typed conflict and nothing — content included —
    /// lands.
    #[test]
    fn d8_receipt_grown_before_stage_refuses_write_conflict() {
        let (dir, root) = workspace();
        let vb = validated(Some(receipt_append())); // span pinned to RECEIPT_OLD's EOF

        // An out-of-band append moves the receipt EOF past the rendered span.
        fs::write(
            dir.path().join(receipt_rel()),
            format!("{RECEIPT_OLD}- foreign row ^r-000042\n"),
        )
        .unwrap();

        let err = apply_batch(
            &root,
            &content_rel(),
            Some(&receipt_rel()),
            &vb,
            PLAN_S0.as_bytes(),
            &candidate(&vb),
        )
        .expect_err("a moved receipt must refuse");
        assert!(is_write_conflict(&err), "typed write-conflict: {err}");
        assert_eq!(
            fs::read(dir.path().join(content_rel())).unwrap(),
            PLAN_S0.as_bytes(),
            "content did not land either — the refusal commits NOTHING"
        );
    }

    /// D8 GATE (receipt shrunk): the receipt file was truncated below the
    /// rendered span offset. Pre-fix this PANICKED (slice out of bounds in
    /// `apply_spans`); now it is the same typed conflict refusal.
    #[test]
    fn d8_receipt_shrunk_refuses_write_conflict_not_panic() {
        let (dir, root) = workspace();
        let vb = validated(Some(receipt_append()));
        fs::write(dir.path().join(receipt_rel()), "#").unwrap(); // shorter than span.start

        let err = apply_batch(
            &root,
            &content_rel(),
            Some(&receipt_rel()),
            &vb,
            PLAN_S0.as_bytes(),
            &candidate(&vb),
        )
        .expect_err("a truncated receipt must refuse, not panic");
        assert!(is_write_conflict(&err), "typed write-conflict: {err}");
    }

    /// D8 GATE (receipt drifts between stage and rename): the receipt gains a
    /// row after staging but before the renames. The pre-rename verify catches
    /// it BEFORE the content rename — refusing the whole commit, never landing
    /// content while dropping the foreign receipt row.
    #[test]
    fn d8_receipt_grown_after_stage_refuses_before_content_rename() {
        let (dir, root) = workspace();
        let vb = validated(Some(receipt_append()));
        let staged = stage_batch(
            &root,
            &content_rel(),
            Some(&receipt_rel()),
            &vb,
            PLAN_S0.as_bytes(),
            &candidate(&vb),
        )
        .unwrap();

        let foreign = format!("{RECEIPT_OLD}- foreign row ^r-000042\n");
        fs::write(dir.path().join(receipt_rel()), &foreign).unwrap();

        let err = staged.commit().expect_err("a drifted receipt must refuse");
        assert!(is_write_conflict(&err), "typed write-conflict: {err}");
        assert_eq!(
            fs::read(dir.path().join(content_rel())).unwrap(),
            PLAN_S0.as_bytes(),
            "content was NOT renamed — both verifies run before the first rename"
        );
        assert_eq!(
            fs::read(dir.path().join(receipt_rel())).unwrap(),
            foreign.as_bytes(),
            "the foreign receipt row survives"
        );
    }

    /// D9: the write flock contends across independent acquires (flock is
    /// per-open-file-description — even in one process), refuses fast with
    /// `WouldBlock` (never waits), is re-acquirable after release, and mints
    /// `.meridian/write.lock` on first use.
    #[test]
    fn write_lock_contends_releases_and_creates_sentinel() {
        let (dir, root) = workspace();
        let held = super::WriteLock::acquire(&root).expect("first acquire");
        assert!(
            dir.path().join(".meridian/write.lock").exists(),
            "the lockfile is minted on first use"
        );
        let contend = super::WriteLock::acquire(&root)
            .expect_err("a held write lock refuses a second acquire");
        assert_eq!(
            contend.kind(),
            io::ErrorKind::WouldBlock,
            "contention is WouldBlock (LOCK_NB — a fast refusal, never a wait)"
        );
        drop(held);
        drop(super::WriteLock::acquire(&root).expect("released lock is re-acquirable"));
    }

    /// Seam contract: a REPLACING receipt span (non-empty) is misuse of the
    /// EOF-append discipline — `InvalidInput` fail-loud, distinct from the
    /// conflict class.
    #[test]
    fn receipt_replacing_span_is_invalid_input() {
        let (_dir, root) = workspace();
        let vb = validated(Some(model::ReceiptAppend {
            span: 0..RECEIPT_OLD.len(), // replaces, not appends
            text: RECEIPT_LINE.to_string(),
        }));
        let err = apply_batch(
            &root,
            &content_rel(),
            Some(&receipt_rel()),
            &vb,
            PLAN_S0.as_bytes(),
            &candidate(&vb),
        )
        .expect_err("a replacing receipt span must refuse");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(!is_write_conflict(&err), "misuse is not the conflict class");
    }

    /// The staging path is uniquely named per call (pid + nanos + monotone seq)
    /// so concurrent or retried commits never collide on a temp — and it is
    /// walk-invisible (`.tmp`, not `.md`).
    #[test]
    fn temp_paths_are_unique_and_walk_invisible() {
        let before = TEMP_SEQ.load(Ordering::Relaxed);
        let a = temp_path_for(Path::new("/w/notes/plan.md"));
        let b = temp_path_for(Path::new("/w/notes/plan.md"));
        assert_ne!(a, b, "each staging path is unique");
        assert!(TEMP_SEQ.load(Ordering::Relaxed) > before);
        for p in [&a, &b] {
            let name = p.file_name().unwrap().to_string_lossy();
            assert!(name.starts_with('.') && name.ends_with(".tmp"));
            assert_ne!(
                p.extension().and_then(|e| e.to_str()),
                Some("md"),
                "staging temp must be outside the .md walk"
            );
        }
    }

    // ── the USER rung of the scope ladder ─────────────────────────────────────

    /// A user scope: `MERIDIAN.md` at its root, `rules/` beneath it. Returns the
    /// tempdir (kept alive by the caller) and the anchor path.
    fn user_scope() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let anchor = dir.path().join("MERIDIAN.md");
        fs::write(&anchor, "---\ntype: meridian-config\n---\n").unwrap();
        fs::create_dir_all(dir.path().join(USER_RULES_DIR)).unwrap();
        (dir, anchor)
    }

    fn spellings(pages: &super::DomainFiles) -> Vec<&str> {
        pages.iter().map(|(page, _)| page.as_str()).collect()
    }

    /// **The anchor-absent arm, tested rather than assumed** (ruled 2026-08-01).
    /// No `MERIDIAN.md` ⇒ the user layer is EMPTY. Note what the fixture holds:
    /// a `rules/` tree full of candidates, and a `$HOME`-shaped sibling tree that
    /// a widened walk would have to read. Neither is reached, because the scope
    /// was never declared.
    #[test]
    fn an_absent_anchor_yields_an_empty_user_layer_and_walks_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join("rules")).unwrap();
        fs::write(dir.path().join("rules/notify.md"), "---\nid: x\n---\n").unwrap();
        fs::create_dir_all(dir.path().join("Documents/deep")).unwrap();
        fs::write(dir.path().join("Documents/deep/notes.md"), "# notes\n").unwrap();

        let pages = user_rule_pages(&dir.path().join("MERIDIAN.md")).expect("an answer");
        assert!(
            pages.is_empty(),
            "no anchor ⇒ no user scope ⇒ no user-layer rules, never a home-directory walk"
        );
    }

    /// An anchor that is a DIRECTORY, not a file, is equally not an anchor.
    #[test]
    fn a_directory_where_the_anchor_should_be_is_not_an_anchor() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join("MERIDIAN.md")).unwrap();
        assert!(
            user_rule_pages(&dir.path().join("MERIDIAN.md"))
                .expect("an answer")
                .is_empty()
        );
    }

    /// A declared user scope with no `rules/` directory is an empty layer, not an
    /// I/O failure — a machine may declare a user scope and register nothing.
    #[test]
    fn a_declared_scope_without_a_rules_directory_is_empty_not_an_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let anchor = dir.path().join("MERIDIAN.md");
        fs::write(&anchor, "---\ntype: meridian-config\n---\n").unwrap();
        assert!(user_rule_pages(&anchor).expect("an answer").is_empty());
    }

    /// The layer is `rules/**.md`, spelled relative to the user scope, sorted,
    /// with the bytes verbatim — and nothing outside `rules/` is offered, however
    /// rule-shaped it looks.
    #[test]
    fn the_user_layer_is_the_rules_tree_spelled_from_the_user_scope() {
        let (dir, anchor) = user_scope();
        fs::create_dir_all(dir.path().join("rules/nested")).unwrap();
        fs::write(dir.path().join("rules/notify.md"), "notify bytes\n").unwrap();
        fs::write(dir.path().join("rules/audit.md"), "audit bytes\n").unwrap();
        fs::write(dir.path().join("rules/nested/deep.md"), "deep bytes\n").unwrap();
        // Outside `rules/`: a page at the user scope root, and one in a sibling
        // directory. Neither is a user-layer candidate.
        fs::write(dir.path().join("loose.md"), "---\nid: loose\n---\n").unwrap();
        fs::create_dir_all(dir.path().join("notes")).unwrap();
        fs::write(dir.path().join("notes/other.md"), "---\nid: other\n---\n").unwrap();

        let pages = user_rule_pages(&anchor).expect("the layer");
        assert_eq!(
            spellings(&pages),
            vec!["rules/audit.md", "rules/nested/deep.md", "rules/notify.md"],
            "`rules/**.md` only, sorted, spelled from the user scope"
        );
        assert_eq!(pages[2].1, b"notify bytes\n", "bytes verbatim");
    }

    /// The md-only + dot-segment floor (§12.1) holds on this rung too: a
    /// non-markdown file never registers, and a dot-prefixed file or directory is
    /// outside the domain at any depth.
    #[test]
    fn the_md_only_and_dot_segment_floor_holds_on_the_user_rung() {
        let (dir, anchor) = user_scope();
        fs::write(dir.path().join("rules/real.md"), "real\n").unwrap();
        fs::write(dir.path().join("rules/notes.txt"), "not markdown\n").unwrap();
        fs::write(dir.path().join("rules/.hidden.md"), "dot file\n").unwrap();
        fs::create_dir_all(dir.path().join("rules/.obsidian")).unwrap();
        fs::write(dir.path().join("rules/.obsidian/x.md"), "dot dir\n").unwrap();

        assert_eq!(
            spellings(&user_rule_pages(&anchor).expect("the layer")),
            vec!["rules/real.md"]
        );
    }
}
