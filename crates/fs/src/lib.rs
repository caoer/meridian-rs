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

/// The §12 hash domain: [`walk`] narrowed to the files whose bytes enter the
/// merkle root — md-only, dot-segment-ignored, and custom-ignored removed. This
/// is where `walk` gains the [`domain`] filter; the filter gates HASHING, never
/// `load` — an ignored md file is absent here yet still addressable by path
/// (`hash ⊂ addressable`, §12.1).
///
/// # Errors
/// I/O failure traversing the root.
pub fn hash_domain(root: &WorkspaceRoot, domain: &domain::Domain) -> io::Result<Vec<PathBuf>> {
    Ok(walk(root)?
        .into_iter()
        .filter(|rel| domain.contains(rel))
        .collect())
}

/// The domain files of a workspace as `(workspace-relative path, raw bytes)`
/// pairs — the shape [`domain_snapshot`] returns and [`build_corpus`] consumes.
pub type DomainFiles = Vec<(String, Vec<u8>)>;

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
/// [`io::ErrorKind::InvalidInput`] before any byte is written.
///
/// # Errors
/// The seam-contract violations above (`InvalidInput`), or any I/O failure at a
/// tmp-write, fsync, or rename step.
pub fn apply_batch(
    root: &WorkspaceRoot,
    content_path: &Path,
    receipt_path: Option<&Path>,
    batch: &model::ValidatedBatch,
) -> io::Result<()> {
    stage_batch(root, content_path, receipt_path, batch)?.commit()
}

/// A two-file commit staged to temp files (written + fsync'd), awaiting the two
/// renames. Separating staging from the renames is what lets the crash-honesty
/// test drive a kill BETWEEN the renames deterministically (§6.5).
struct StagedCommit {
    content: StagedFile,
    receipt: Option<StagedFile>,
}

/// One file staged for atomic replace: the temp path holding the new bytes
/// (already fsync'd) and the destination it will be renamed onto.
struct StagedFile {
    tmp: PathBuf,
    dst: PathBuf,
}

/// Stage both files: read each pre-batch file, apply its validated span
/// replacements, and write the result to a fsync'd temp beside the destination.
/// No destination is touched here — staging is entirely off to the side, so a
/// failure (or a crash) before [`StagedCommit::commit`] leaves every real file
/// intact (the property gate 2 checks).
fn stage_batch(
    root: &WorkspaceRoot,
    content_path: &Path,
    receipt_path: Option<&Path>,
    batch: &model::ValidatedBatch,
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

    // Content file: read pre-batch bytes, apply the validated span edits verbatim.
    let content_dst = root.0.join(content_path);
    let content_old = fs::read(&content_dst)?;
    let content_new = apply_spans(
        &content_old,
        batch.edits.iter().map(|e| (&e.span, e.text.as_str())),
    );
    let content = stage_file(&content_dst, &content_new)?;

    // Receipt file (when named): read pre-batch bytes (absent ⇒ empty, a create),
    // apply the single pre-rendered append verbatim, stage it. On any failure the
    // already-staged content temp is cleaned up so a failed apply leaves no litter.
    let receipt = match (receipt_path, batch.receipt.as_ref()) {
        (Some(rp), Some(append)) => {
            let receipt_dst = root.0.join(rp);
            match stage_receipt(&receipt_dst, append) {
                Ok(staged) => Some(staged),
                Err(e) => {
                    let _ = fs::remove_file(&content.tmp);
                    return Err(e);
                }
            }
        }
        _ => None,
    };

    Ok(StagedCommit { content, receipt })
}

/// Stage the receipt file: read its pre-batch bytes (missing ⇒ empty) and apply
/// the single append span. Factored out so content-temp cleanup on error has one
/// site.
fn stage_receipt(receipt_dst: &Path, append: &model::ReceiptAppend) -> io::Result<StagedFile> {
    let old = read_or_empty(receipt_dst)?;
    let new = apply_spans(&old, std::iter::once((&append.span, append.text.as_str())));
    stage_file(receipt_dst, &new)
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
    /// Commit both files: rename the content file (which COMMITS it), then the
    /// receipt file. The gap between the two renames is the STATED §6.5 crash
    /// window; nothing here narrows it away — it is honestly the limit.
    fn commit(self) -> io::Result<()> {
        self.rename_content()?;
        // ┄┄ §6.5 crash window: a crash HERE lands content-without-receipt ┄┄
        self.rename_receipt()
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
    use super::{TEMP_SEQ, WorkspaceRoot, apply_batch, stage_batch, temp_path_for, walk};
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
        };
        match model::validate_batch(&doc, None, &req, receipt) {
            model::SpliceVerdict::Validated(vb) => vb,
            other => panic!("fixture batch must validate, got {other:?}"),
        }
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
        let staged = stage_batch(&root, &content_rel(), Some(&receipt_rel()), &vb).unwrap();
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

        let staged = stage_batch(&root, &content_rel(), Some(&receipt_rel()), &vb).unwrap();

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

        apply_batch(&root, &content_rel(), Some(&receipt_rel()), &vb).unwrap();

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

        apply_batch(&root, &content_rel(), None, &vb).unwrap();

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
            apply_batch(&root, &content_rel(), None, &with_receipt)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput,
        );
        // (2) path supplied but the batch has no receipt.
        assert_eq!(
            apply_batch(&root, &content_rel(), Some(&receipt_rel()), &no_receipt)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput,
        );
        // (3) same-file receipt would clobber the content rename.
        assert_eq!(
            apply_batch(&root, &content_rel(), Some(&content_rel()), &with_receipt)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput,
        );
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
}
