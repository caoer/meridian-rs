//! Disk truth in, atomic splices out: read/walk/watch feeding the model; tmp+fsync+rename
//! splice execution.
//!
//! # Charter
//! **Owns:** the disk boundary. Reading workspace files (refusing non-UTF-8 — spans must
//! denote exact disk bytes or splice corrupts files), walking the corpus, watching for
//! changes, and *executing* validated splices atomically (tmp + fsync + rename).
//! Feeds `model` and nothing else.
//!
//! **Never does:** writing bytes it didn't splice, caching anything to disk (law 2: no
//! snapshot files, no second database), interpreting content.
//!
//! # Law enforcement
//! Write execution demands `model::ValidatedBatch` — a token only `model`'s CAS
//! validation can mint. An unvalidated write cannot reach disk by construction; the
//! splice pipeline (validate in `model`, execute here) is enforced by types, not review.

use std::collections::{BTreeMap, VecDeque};
use std::fs::{self, File};
use std::io::{self, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::PoisonError;
use std::sync::atomic::{AtomicU64, Ordering};

pub mod base;
pub mod digestmemo;
pub mod domain;
pub mod fence;
pub mod forest;
pub mod guard;
pub mod radix;
pub mod resident;
pub mod stable;

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

/// The canonical workspace-relative spelling of `path` — absolute or
/// root-relative argv alike — when it resolves inside `root`; `None` when it
/// resolves outside the root (no such spelling exists) or cannot be resolved
/// at all. The one respell computation: the write door's teaching
/// (`wire_serve::write::relative_respelling`) and the run doors' §2.1 receipt
/// keys both delegate here, so a taught spelling and a receipted spelling
/// cannot drift.
///
/// Both sides canonicalize, so symlinked prefixes (`/tmp` vs `/private/tmp`)
/// and `.`/`..`-bearing spellings resolve to one form; a missing leaf
/// resolves through its parent, so a not-yet-born file still gets its
/// spelling. The root itself (empty rel) is no page spelling — `None`.
#[must_use]
pub fn workspace_relative(root: &WorkspaceRoot, path: &str) -> Option<String> {
    let p = Path::new(path);
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        root.0.join(p)
    };
    let canonical = fs::canonicalize(&abs).ok().or_else(|| {
        let parent = fs::canonicalize(abs.parent()?).ok()?;
        Some(parent.join(abs.file_name()?))
    })?;
    let base = fs::canonicalize(&root.0).unwrap_or_else(|_| root.0.clone());
    let rel = canonical.strip_prefix(&base).ok()?.to_str()?;
    (!rel.is_empty()).then(|| rel.to_owned())
}

/// Walk the corpus: every markdown file under the root, as root-relative paths,
/// sorted. This is the ADDRESSABLE set — dot-dir md files (`.github/README.md`)
/// are included, since they stay `load`-able even when ignored for hashing
/// (§12.1). Cold rebuild of the whole world model from this walk is the
/// recovery path. Symlinks are not followed.
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

/// The markdown the domain DECLINES yet the vault still shows: every md file
/// on a dot-free path that the §12 filter excludes — the custom-ignore class,
/// exactly, as sorted workspace-relative paths.
///
/// This is the enumerator for voices about declined pages (`mrd rules`'s
/// `not offered to registration` / `cannot be answered` blocks), and it walks
/// by the projection's own dir law: a dot-prefixed segment is never entered
/// and never reported ([`domain::dot_segment`] — the same spelling
/// [`hash_domain`]'s walk skips by), so a face fed from here can never caveat
/// a path the record projection refuses to serve (dogfood F11). Custom-ignored
/// DIRECTORIES are entered, never pruned: their files are exactly what this
/// walk exists to find. Contrast [`walk`], the ADDRESSABLE set, which keeps
/// dot-dir md files because they stay `load`-able.
///
/// A non-UTF-8 NAME is skipped — wire paths are UTF-8, so such a file is
/// unservable and unnameable alike.
///
/// # Errors
/// I/O failure loading the domain config or traversing the root.
pub fn declined_markdown(root: &WorkspaceRoot) -> io::Result<Vec<String>> {
    let domain = domain::Domain::load(root)?;
    let mut out = Vec::new();
    declined_dir(&root.0, "", &domain, &mut out)?;
    out.sort();
    Ok(out)
}

fn declined_dir(
    abs_dir: &Path,
    rel_dir: &str,
    domain: &domain::Domain,
    out: &mut Vec<String>,
) -> io::Result<()> {
    for entry in fs::read_dir(abs_dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        // Before the is_dir branch, so a dot FILE and a dot DIRECTORY are
        // declined by the same line — the user rung's own discipline.
        if domain::dot_segment(name) {
            continue;
        }
        let rel = if rel_dir.is_empty() {
            name.to_owned()
        } else {
            format!("{rel_dir}/{name}")
        };
        if file_type.is_dir() {
            declined_dir(&entry.path(), &rel, domain, out)?;
        } else if file_type.is_file()
            && matches!(
                domain.exclusion(Path::new(&rel)),
                Some(domain::ExclusionReason::CustomIgnore)
            )
        {
            out.push(rel);
        }
    }
    Ok(())
}

/// The disk behind the ambient root, as the question `model` asks — one
/// predicate, one owner, the same division [`domain::Domain`] answers
/// [`model::HashDomain`] under.
///
/// The colour plane needs it because **absence outranks domain membership**
/// (`wire-contract.md` §12.1, verdict-plane clause; session decision 0049): the
/// corpus map holds no out-of-domain path whether that path is on disk or
/// deleted, so only a read separates *present but unhashable* (grey) from
/// *genuinely gone* (red). `model` cannot name a filesystem, so it names the
/// trait and this answers it. [`WorkspaceRoot`] itself is the implementor —
/// the root IS the disk the ambient corpus was built from, and a second type
/// would be a second answer to one fact.
impl model::AmbientDisk for WorkspaceRoot {
    /// The path law first, then one `stat`.
    ///
    /// A path that does not spell a location strictly inside the root answers
    /// `None` — never `false`. `false` is a MEASURED absence that the colour
    /// plane renders as red `file-not-found`, and a path the engine refused to
    /// read is not a path the engine measured.
    fn exists(&self, rel: &str) -> Option<bool> {
        let path = Path::new(rel);
        if rel.is_empty()
            || !path
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_)))
        {
            return None;
        }
        Some(self.0.join(path).is_file())
    }
}

/// The directory user-scope rule pages live under, relative to the user scope.
pub const USER_RULES_DIR: &str = "rules";

/// The USER rung of the registration scope ladder: every rule-page candidate
/// under the user scope, as `(page path relative to the user scope, raw bytes)`.
///
/// `anchor` is the resolved `MERIDIAN.md` path (`config::resolve_path`), never
/// a guess made here. The user scope is the directory containing that file, and
/// an anchor that is not an existing file yields an empty user layer.
///
/// The user scope has no declared hash domain and on a real machine it is
/// `$HOME`, so the rung is bounded twice: no anchor ⇒ no user layer (never
/// widened into "walk `$HOME` and see"), and only [`USER_RULES_DIR`] is looked
/// at. Registration itself is still by tag.
///
/// The md-only + dot-segment floor of the hash domain (§12.1) applies:
/// non-markdown files never register, and a dot-prefixed segment is outside the
/// domain at any depth. Symlinks are not followed. Paths are returned
/// `rules/…`-prefixed and sorted.
///
/// # Errors
/// I/O failure reading the `rules/` tree once the anchor and the directory are
/// both present. An absent anchor and an absent `rules/` directory are answers,
/// not failures.
pub fn user_rule_pages(anchor: &Path) -> io::Result<DomainFiles> {
    let Some((rels, _declined)) = user_rules_traversal(anchor)? else {
        return Ok(Vec::new());
    };
    let Some(user_scope) = anchor.parent() else {
        return Ok(Vec::new());
    };
    let mut pages = Vec::with_capacity(rels.len());
    for rel in rels {
        // A rule page whose name has no UTF-8 spelling must not register
        // under a rewritten name (silent policy drift) — refuse loud instead
        // (merkle-spec §9 name truthfulness; the display form is escaped).
        let Some(rel_str) = rel.to_str() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "user rule page {} has a non-UTF-8 name and cannot be truthfully registered",
                    display_name(hash_name(&rel))
                ),
            ));
        };
        let bytes = fs::read(user_scope.join(&rel))?;
        pages.push((rel_str.to_owned(), bytes));
    }
    Ok(pages)
}

/// The markdown under the user `rules/` tree that the rung DECLINED to offer —
/// every page a dot-prefixed segment kept out, spelled `rules/…` exactly as
/// [`user_rule_pages`] spells what it keeps.
///
/// This exists because the rung's exclusion is SILENT at every face that reads
/// it: `mrd rules` printed `(no rules in effect)` at exit 0 with an empty stderr
/// for a rule page whose only defect was a dot in its path. An enumerator MAY
/// exclude what it cannot attest; it may never exclude SILENTLY (session
/// decision 0017). A face that wants to voice the drop needs the dropped
/// population, and it may not re-derive it — a second traversal beside this
/// one is two answers to "what did the rung decline", which is the defect one
/// level up.
///
/// ⛔ The dot test in [`walk_user_rules_dir`] sits BEFORE the `is_dir` branch,
/// so a dot-prefixed FILE and a dot-prefixed DIRECTORY are declined by the same
/// line and are ONE member of this population, not two.
///
/// # Errors
/// As [`user_rule_pages`]: I/O failure once the anchor and the directory are
/// both present. An absent anchor or `rules/` directory is an empty answer.
pub fn user_rule_pages_declined(anchor: &Path) -> io::Result<Vec<String>> {
    let Some(declined) = user_rules_traversal(anchor)?.map(|(_kept, declined)| declined) else {
        return Ok(Vec::new());
    };
    let mut out: Vec<String> = declined
        .iter()
        .filter_map(|rel| rel.to_str().map(str::to_owned))
        .collect();
    out.sort();
    Ok(out)
}

/// The ONE traversal of the user rung, returning both views: what registered
/// and what a dot segment declined. `None` when there is no rung to walk at all
/// (no anchor, or no `rules/` beside it) — which is an answer, not a failure,
/// and is deliberately distinguishable from a rung that walked and found
/// nothing.
fn user_rules_traversal(anchor: &Path) -> io::Result<Option<(Vec<PathBuf>, Vec<PathBuf>)>> {
    if !anchor.is_file() {
        return Ok(None);
    }
    let Some(user_scope) = anchor.parent() else {
        return Ok(None);
    };
    let rules_dir = user_scope.join(USER_RULES_DIR);
    if !rules_dir.is_dir() {
        return Ok(None);
    }
    let mut kept = Vec::new();
    let mut declined = Vec::new();
    walk_user_rules_dir(
        &rules_dir,
        Path::new(USER_RULES_DIR),
        &mut kept,
        &mut declined,
    )?;
    kept.sort();
    declined.sort();
    Ok(Some((kept, declined)))
}

/// The user rung's traversal: markdown files under `rules/`, dot-segments
/// declined at any depth, symlinks not followed.
///
/// `declined` collects the markdown a dot segment kept out, so the rung can be
/// asked what it dropped without a second traversal disagreeing with this one.
/// A dot-prefixed DIRECTORY is descended for this purpose only — its pages are
/// still declined, and nothing beneath it can ever be re-included.
fn walk_user_rules_dir(
    abs_dir: &Path,
    rel_dir: &Path,
    out: &mut Vec<PathBuf>,
    declined: &mut Vec<PathBuf>,
) -> io::Result<()> {
    for entry in fs::read_dir(abs_dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let name = entry.file_name();
        let rel = rel_dir.join(&name);
        let is_markdown = Path::new(&name)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("md"));
        if name.to_string_lossy().starts_with('.') {
            // The declining line, unchanged in effect: this page does not
            // register. What is new is that it is RECORDED rather than dropped
            // on the floor.
            if file_type.is_dir() {
                collect_declined_markdown(&entry.path(), &rel, declined)?;
            } else if file_type.is_file() && is_markdown {
                declined.push(rel);
            }
            continue;
        }
        if file_type.is_dir() {
            walk_user_rules_dir(&entry.path(), &rel, out, declined)?;
        } else if file_type.is_file() && is_markdown {
            out.push(rel);
        }
    }
    Ok(())
}

/// Every markdown page beneath a declined directory. Nothing here can register
/// — the dot segment above it declines the whole subtree — so this walk only
/// ever feeds the declined population.
fn collect_declined_markdown(
    abs_dir: &Path,
    rel_dir: &Path,
    declined: &mut Vec<PathBuf>,
) -> io::Result<()> {
    for entry in fs::read_dir(abs_dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let name = entry.file_name();
        let rel = rel_dir.join(&name);
        if file_type.is_dir() {
            collect_declined_markdown(&entry.path(), &rel, declined)?;
        } else if file_type.is_file()
            && Path::new(&name)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("md"))
        {
            declined.push(rel);
        }
    }
    Ok(())
}

/// The §12 hash domain: the files whose bytes enter the merkle root — md-only,
/// dot-segment-ignored, and custom-ignored removed. The filter gates HASHING,
/// never `load` — an ignored md file is absent here yet still addressable by
/// path (`hash ⊂ addressable`, §12.1).
///
/// This is its own traversal rather than `walk().filter()`: [`walk`] must keep
/// descending everywhere to keep `.github/README.md` addressable (§12.1), while
/// this walk may prune whole directories. Pruning is sound only where
/// re-inclusion is impossible — [`domain::Domain::prunes_dir`] declines
/// whenever a `!` rule could reach beneath. Dot-directories are pruned
/// structurally, since [`domain::Domain::contains`] holds the dot rule above
/// custom rules so no `!` can lift a dot path.
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
            if domain::dot_segment(&name.to_string_lossy()) {
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

/// A cheap, stat-only fingerprint of the hash domain: the same walk
/// [`hash_domain`] runs, folded over each file's `(relative path, mtime, size)`
/// instead of its bytes. Never a correctness signal — a change detector that
/// pays one `stat` per domain file and reads no content.
///
/// Equality means "no file's path, size, or mtime moved" — evidence of an
/// unchanged corpus, not proof (a write restoring the previous size within the
/// filesystem's mtime granularity is invisible). So it may only gate work that
/// is pure latency, such as skipping a pre-warm sweep; it must never stand in
/// for the content root, which stays the warm-engine reuse key and the only
/// thing a served answer is stamped with.
///
/// # Errors
/// I/O failure loading the domain config or traversing the root.
pub fn domain_stat_signature(root: &WorkspaceRoot) -> io::Result<u64> {
    let domain = domain::Domain::load(root)?;
    let rels = hash_domain(root, &domain)?;
    // FNV-1a, not the production merkle fold: a non-cryptographic fold over
    // stat metadata cannot be mistaken for a second, weaker root.
    let mut fold = FNV_OFFSET;
    let mut eat = |bytes: &[u8]| {
        for byte in bytes {
            fold ^= u64::from(*byte);
            fold = fold.wrapping_mul(FNV_PRIME);
        }
    };
    eat(&(rels.len() as u64).to_le_bytes());
    for rel in &rels {
        let meta = fs::symlink_metadata(root.0.join(rel))?;
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX));
        eat(hash_name(rel));
        eat(&mtime.to_le_bytes());
        eat(&meta.len().to_le_bytes());
    }
    Ok(fold)
}

/// FNV-1a 64-bit offset basis and prime, for [`domain_stat_signature`]'s fold.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// The identity one file must keep for a cached §12.2 leaf digest to be reused:
/// `(device, inode, size, mtime, ctime)`, each at the resolution the filesystem
/// reports.
///
/// `ctime` is what makes this worth trusting. `mtime` alone is settable
/// (`utimes(2)`), so a writer can restore it; `ctime` is bumped by the kernel on
/// every inode change and no API sets it, so a same-size in-place rewrite that
/// forges an unchanged `mtime` still moves `ctime`. `dev`/`ino` catch the path
/// being re-pointed at a different file.
///
/// Same standing as [`domain_stat_signature`]: evidence, not proof. It gates
/// re-READING a file whose content is already hashed — never what a committed
/// answer is stamped with. The write path folds from bytes
/// ([`domain_snapshot`]), so a memo that ever disagreed with disk refuses the
/// commit instead of landing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatKey {
    dev: u64,
    ino: u64,
    size: u64,
    mtime: (i64, i64),
    ctime: (i64, i64),
}

impl StatKey {
    /// The identity of `meta`.
    #[must_use]
    pub fn of(meta: &fs::Metadata) -> StatKey {
        use std::os::unix::fs::MetadataExt as _;
        StatKey {
            dev: meta.dev(),
            ino: meta.ino(),
            size: meta.size(),
            mtime: (meta.mtime(), meta.mtime_nsec()),
            ctime: (meta.ctime(), meta.ctime_nsec()),
        }
    }

    /// The raw identity fields, for [`digestmemo`]'s line codec. Crate-only:
    /// the fields stay private so no consumer grows an opinion about them.
    pub(crate) fn raw_parts(&self) -> (u64, u64, u64, (i64, i64), (i64, i64)) {
        (self.dev, self.ino, self.size, self.mtime, self.ctime)
    }

    /// Rebuild an identity from [`StatKey::raw_parts`]'s fields (the memo's
    /// deserialization arm).
    pub(crate) fn from_raw_parts(
        dev: u64,
        ino: u64,
        size: u64,
        mtime: (i64, i64),
        ctime: (i64, i64),
    ) -> StatKey {
        StatKey {
            dev,
            ino,
            size,
            mtime,
            ctime,
        }
    }

    /// The deliberately-spoiled memo identity (merkle-spec §6.2: "or its
    /// memo entry deliberately spoiled"): matches no file any filesystem
    /// reports, so the next observation re-reads the member instead of
    /// trusting a stat raced by the write that minted the entry.
    pub(crate) fn spoiled() -> StatKey {
        StatKey {
            dev: u64::MAX,
            ino: u64::MAX,
            size: u64::MAX,
            mtime: (i64::MIN, i64::MIN),
            ctime: (i64::MIN, i64::MIN),
        }
    }

    /// The identity of the file at `abs`, or `None` when nothing is there.
    /// Symlinks are not followed, matching the domain walk.
    ///
    /// # Errors
    /// I/O failure other than the file being absent.
    pub fn of_path(abs: &Path) -> io::Result<Option<StatKey>> {
        match fs::symlink_metadata(abs) {
            Ok(meta) => Ok(Some(StatKey::of(&meta))),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
}

/// A resident memo of the hash domain's §12.2 leaf digests, keyed by
/// [`StatKey`] — the daemon's currency check.
///
/// The corpus root is re-derived far more often than the corpus changes: every
/// wire round trip needs to know the warm engine is still current, and the only
/// honest answer was to re-read and re-fold every domain byte. This holds
/// `blake3(raw)` per file instead, so a currency pass costs one `stat` per
/// domain member and reads only the members whose identity moved — O(corpus) in
/// `stat`s, O(changed) in bytes.
///
/// It is a cache of a pure function ([`model::leaf_digest`]) and holds exactly
/// one generation. It retains no history and answers no as-of question: there
/// is nothing here to select a version from.
///
/// # The §6.3 stamp plane
/// Each resident directory node carries `last_seq` — the highest journal seq
/// beneath it — maintained HERE, by the same guarded write path that
/// maintains the digests, so the hash instrument audits the stamp instrument
/// and the two cannot drift silently (the ZFS `hole_birth` lesson). Once
/// [`bind_stamps`](Self::bind_stamps) installs a journal binding, every tree
/// advance — own-write overlay, feed apply, an observation absorbing foreign
/// change — stamps the touched ancestor chains with `clock() + 1`: one past
/// the journal tip at the instant of the advance, which is exactly the seq a
/// choke-point commit's frame will carry, and for an unjournaled advance a
/// value strictly greater than every token minted before it — the compare
/// can only degrade, never false-pass. [`stamp_untouched`](Self::stamp_untouched)
/// answers the fast-path fact; the EVENT-STREAM vouch that makes a stamp
/// answer legal (merkle-spec §6.3/§6.4 — cookie barrier, loss-free feed) is
/// the caller's to establish, never assumed here.
#[derive(Debug, Default)]
pub struct DomainCache {
    leaves: BTreeMap<PathBuf, LeafSeen>,
    dirs: DirMemo,
    /// The resident tree (merkle-spec §6.1): the interior folds, kept.
    /// Updated by every successful observation's generation delta and by the
    /// own-write overlay; law-2 from birth, serving nothing before the
    /// cutover.
    tree: resident::ResidentTree,
    /// The §6.3 stamp plane's journal binding: the tree instance id and the
    /// journal tip clock every stamp mints against. `None` — the unbound
    /// plane — stamps nothing and answers nothing (every query degrades to
    /// the content-fold compare). Bound by the cache's owner
    /// ([`Self::bind_stamps`]); the write paths below stamp through it.
    stamps: Option<StampSource>,
    /// The served law-1 root under the interim law (merged plan §6 step 3):
    /// recomputed only when the tree advances or the domain version moves —
    /// `(domain version, root)`.
    served: Option<(u32, model::MerkleRoot)>,
    /// The domain as of the last observation — the overlay's membership and
    /// version law (the overlay composes against the OBSERVED generation,
    /// never a fresher config read).
    domain_seen: Option<domain::Domain>,
    /// The §6.2 timestamp-granularity calibration, probed lazily at the
    /// first observation (the cache's workspace open). `None` = not yet
    /// probed.
    calibration: Option<stable::Calibration>,
    /// The shared feed-generation cell: the event-feed watcher advances it,
    /// the observation path fences byte reads with it, and its loss count
    /// feeds [`Self::guard_currency`].
    feed: stable::FeedGen,
    /// Feed losses absorbed by the last COMPLETED observation — a loss at or
    /// below this count has been re-derived by a full pass.
    acked_losses: u64,
    reads: u64,
    listings: u64,
    flat_folds: u64,
    watermark_rereads: u64,
    suspect_reads: u64,
    fenced_reads: u64,
    /// Corpus observation passes attempted — one half of the §7(d)
    /// quiet-workspace counter pair (codex gate 10). A quiet workspace after
    /// baseline leaves this unmoved, which is what proves no timer exists.
    sweeps: u64,
    /// Member identities `stat`ed across all sweeps — the pair's other half.
    member_stats: u64,
}

/// The journal tip clock a stamp plane mints against: answers the current
/// journal seq (the workspace ring's tip) at the instant of a tree advance.
pub type StampClock = std::sync::Arc<dyn Fn() -> u64 + Send + Sync>;

/// The §6.3 stamp binding: which journal epoch the tree's stamps belong to.
///
/// `instance` is the tree instance id (B-01 epoch identity — ring seq is
/// per-daemon-epoch and rings are idle-reaped, so a seq is meaningful only
/// within one instance). A query under any OTHER instance answers `None` —
/// the degrade the §7 restart/reap row demands. Rebinding after an epoch
/// change keeps old stamp values: max-only stamps from a dead epoch can only
/// read as "touched" against a young chain's tokens — conservatism that
/// heals as the new chain grows, never a false pass.
struct StampSource {
    instance: String,
    clock: StampClock,
}

impl std::fmt::Debug for StampSource {
    /// The clock is an opaque closure; the instance is the whole public truth.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StampSource")
            .field("instance", &self.instance)
            .finish_non_exhaustive()
    }
}

/// One remembered leaf: the identity its digest was byte-derived under and
/// the observation watermark of that record. `seen: None` is the deliberate
/// spoil (merkle-spec §6.2 row 1): the entry never trusts, whatever its key.
#[derive(Debug, Clone)]
struct LeafSeen {
    key: StatKey,
    digest: [u8; 32],
    seen: Option<stable::FsStamp>,
}

/// The remembered listings: each directory's [`StatKey`] at enumeration
/// time, the observation watermark of that record (`None` = spoiled — a
/// same-quantum entry change would be invisible to the key compare, exactly
/// the leaf memo's racy window), and the entry set it held then.
type DirMemo = BTreeMap<PathBuf, DirSeen>;

/// One remembered directory listing with its trust record (see [`DirMemo`]).
#[derive(Debug, Clone)]
struct DirSeen {
    key: StatKey,
    seen: Option<stable::FsStamp>,
    entries: Vec<DirEntryKind>,
}

/// One remembered directory entry: its name and its `read_dir` file type — the
/// only facts [`hash_domain`]'s walk takes from an enumeration, so remembering
/// them is remembering the listing.
///
/// All three flags are kept because `read_dir`'s type is `lstat`-shaped: a
/// symlink is neither `is_dir` nor `is_file`, and the domain walk descends
/// only the first and admits only the second. Deriving one from another would
/// quietly start following symlinks into the hash domain. `is_symlink` is
/// what lets the GUARDED observation ([`DomainCache::observe`]) refuse links
/// from remembered listings exactly as the fresh strict walk does — a link's
/// creation or removal moves its directory's own timestamps, so an unmoved
/// listing still carries the truth about it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DirEntryKind {
    name: std::ffi::OsString,
    is_dir: bool,
    is_file: bool,
    is_symlink: bool,
}

impl DomainCache {
    /// An empty memo — the first [`root`](Self::root) reads every member.
    #[must_use]
    pub fn new() -> DomainCache {
        DomainCache::default()
    }

    /// An empty memo sharing an existing feed-generation cell: the registry
    /// rebuilds a memo through this so the event-feed watcher and the
    /// observation fence ride ONE instrument — the watcher's `advance` /
    /// `note_loss` land on the cell this cache brackets reads with.
    #[must_use]
    pub fn with_feed(feed: stable::FeedGen) -> DomainCache {
        DomainCache {
            feed,
            ..DomainCache::default()
        }
    }

    /// How many domain members this memo has READ (not `stat`ed) for their
    /// bytes, over its whole life.
    ///
    /// The companion of [`fold_count`]: that counts full folds, this counts how
    /// much of the corpus each currency pass actually touched. Per-memo rather
    /// than process-global precisely so an exact-count assertion does not
    /// depend on nothing else folding at the same time.
    #[must_use]
    pub fn leaves_read(&self) -> u64 {
        self.reads
    }

    /// The domain's current [`model::MerkleRoot`], reading only what moved.
    ///
    /// The walk and the `stat`s are live: the member SET is observed now, and
    /// every member's identity is checked now. Vanished members are dropped, so
    /// the memo cannot outlive its corpus.
    ///
    /// The root is byte-identical to [`domain_snapshot`]'s over the same tree —
    /// both fold through [`model::merkle_root_of_leaves`], and a member whose
    /// digest is not memoized is read and hashed by [`model::leaf_digest`], the
    /// same leaf the other path uses.
    ///
    /// # Errors
    /// I/O failure loading the domain config, traversing the root, `stat`ing a
    /// member, or reading a member whose identity moved.
    pub fn root(&mut self, root: &WorkspaceRoot) -> io::Result<model::MerkleRoot> {
        let domain = domain::Domain::load(root)?;
        let rows = self
            .observe(root, &domain, ObserveLaw::Plain)
            .map_err(plain_refusal)?;
        let version = domain.version();
        // The interim served-token law (merged plan §6 step 3): the served
        // value stays law-1, derived from the resident structure's current
        // leaves, recomputed only when the root advances. The observation
        // above invalidated the cache iff the tree moved.
        if let Some((v, cached)) = &self.served
            && *v == version
        {
            return Ok(cached.clone());
        }
        self.flat_folds += 1;
        // Raw name bytes into the fold (merkle-spec §4/§9) — the same names
        // `domain_snapshot` folds, so the byte-identity holds on any corpus.
        let leaves: Vec<(&[u8], [u8; 32])> = rows
            .iter()
            .map(|(name, digest)| (name.as_slice(), *digest))
            .collect();
        let folded = model::merkle_root_of_leaves(&leaves, version);
        self.served = Some((version, folded.clone()));
        Ok(folded)
    }

    /// The locked-window observation of the run plane, served from this memo:
    /// the domain's current [`DomainLeaves`] — plain-walk semantics, byte
    /// reads only for movers. Value-identical to [`domain_leaves_memoized`]
    /// over the same tree; what changes is where listings and digests come
    /// from (the resident dir + leaf memos instead of a fresh walk and the
    /// caller's drawer memo).
    ///
    /// # Errors
    /// I/O failure loading the domain config, traversing the root, `stat`ing
    /// a member, or reading a member whose identity moved.
    pub fn domain_leaves(&mut self, root: &WorkspaceRoot) -> io::Result<DomainLeaves> {
        let domain = domain::Domain::load(root)?;
        let leaves = self
            .observe(root, &domain, ObserveLaw::Plain)
            .map_err(plain_refusal)?;
        Ok(DomainLeaves { leaves, domain })
    }

    /// One observation of the domain through the resident memos — the shared
    /// core of [`root`](Self::root), [`domain_leaves`](Self::domain_leaves),
    /// and the guarded bracket observations ([`guard::StepGuard::open_cached`]
    /// / [`guard::StepGuard::close_cached`]).
    ///
    /// The walk and the `stat`s are live: the member SET is observed now, and
    /// every member's identity is checked now. Listings are served from the
    /// dir memo for unmoved directories; digests from the leaf memo for
    /// unmoved members; moved members are re-read under the §6.2 stable-read
    /// law ([`stable::read_settled`]: `O_NOFOLLOW`, fd identity fstatted
    /// before and after the bytes, feed-generation fence). On success the
    /// leaf memo holds exactly the observed generation — vanished members
    /// are dropped, so the memo cannot outlive its corpus.
    ///
    /// Reuse is gated by the watermark trust close (merkle-spec §6.2), never
    /// by `StatKey` equality alone: an entry whose stamps sit inside the
    /// calibrated racy window of its record watermark is re-read even when
    /// its key is byte-identical — the same-quantum in-place write is
    /// exactly what the key compare cannot see.
    pub(crate) fn observe(
        &mut self,
        root: &WorkspaceRoot,
        domain: &domain::Domain,
        law: ObserveLaw,
    ) -> Result<BTreeMap<Vec<u8>, [u8; 32]>, ObserveRefusal> {
        // Losses counted before the pass are re-derived by the pass (a
        // completed observation IS the full sweep the rescan ladder floors
        // at); losses landing mid-pass stay unabsorbed. Counted at entry so
        // an aborted sweep still shows on the §7(d) counter.
        self.sweeps += 1;
        let losses_at_start = self.feed.losses();
        let trust = self.trust_context(root);
        let (mut rels, mut offenders, fresh_dirs, listings) =
            Self::walk_tree(&self.dirs, &root.0, domain, law, trust)?;
        // The listings are facts about the tree either way — recorded even
        // when the guarded law refuses below, exactly as a fresh walk's
        // enumeration cost is paid before its verdict.
        self.dirs = fresh_dirs;
        self.listings += listings;
        if !offenders.is_empty() {
            // The walk completed; the refusal is a count plus the first
            // offender in sorted order (the strict walk's discipline).
            offenders.sort();
            return Err(ObserveRefusal::Symlink {
                count: offenders.len(),
                first: offenders.remove(0),
            });
        }
        rels.sort();
        let identities = member_identities(&root.0, &rels, PARALLEL_STAT_FLOOR)?;
        self.member_stats += identities.len() as u64;
        let mut fresh: BTreeMap<PathBuf, LeafSeen> = BTreeMap::new();
        // Name-keyed rows (merkle-spec §4/§9 raw name bytes) — the shape every
        // consumer folds or compares in, built once here so no observation
        // pays a second per-member map conversion.
        let mut rows: BTreeMap<Vec<u8>, [u8; 32]> = BTreeMap::new();
        for (rel, key) in identities {
            let entry = self.observe_member(root, &rel, key, law, trust)?;
            rows.insert(hash_name(&rel).to_vec(), entry.digest);
            fresh.insert(rel, entry);
        }
        // The resident tree follows the observed generation (merkle-spec
        // §6.1): removals first — a same-name kind swap across generations
        // must never compose a transient §4.4 collision — then idempotent
        // set for the survivors (an unmoved member re-hashes nothing). Every
        // absorbed change stamps its chain in the same act (§6.3): one
        // clock read serves the pass — every stamped value is one past the
        // tip as of the pass, which stays strictly greater than any token
        // minted before the change was absorbed.
        let mut advanced = false;
        {
            let stamp_seq = self.stamps.as_ref().map(|s| (s.clock)() + 1);
            let (leaves, tree) = (&self.leaves, &mut self.tree);
            for rel in leaves.keys() {
                if !fresh.contains_key(rel) {
                    let removed = tree.remove_leaf(rel);
                    if removed && let Some(seq) = stamp_seq {
                        tree.stamp_chain(rel, seq);
                    }
                    advanced |= removed;
                }
            }
            for (rel, entry) in &fresh {
                let set = tree.set_leaf(rel, entry.digest);
                if set && let Some(seq) = stamp_seq {
                    tree.stamp_chain(rel, seq);
                }
                advanced |= set;
            }
        }
        if advanced {
            self.served = None;
        }
        self.domain_seen = Some(domain.clone());
        self.leaves = fresh;
        self.acked_losses = losses_at_start;
        Ok(rows)
    }

    /// One member's leaf under the §6.2 trust decision: a memoized digest
    /// serves only when the key matches AND the watermark law clears the
    /// record — a matching key inside the racy window re-reads, because the
    /// same-quantum in-place write is exactly what the key compare cannot
    /// see (codex gate 17: `StatKey` equality alone never passes).
    fn observe_member(
        &mut self,
        root: &WorkspaceRoot,
        rel: &Path,
        key: StatKey,
        law: ObserveLaw,
        trust: stable::TrustCtx,
    ) -> Result<LeafSeen, ObserveRefusal> {
        let key_matched = match self.leaves.get(rel) {
            Some(prior) if prior.key == key && trust.trusts(&key, prior.seen) => {
                return Ok(LeafSeen {
                    key,
                    digest: prior.digest,
                    seen: prior.seen,
                });
            }
            Some(prior) => prior.key == key,
            None => false,
        };
        if key_matched {
            // The watermark's own re-read — the trust close's instrument.
            self.watermark_rereads += 1;
        }
        self.reads += 1;
        // The event-generation fence brackets the read (§6.2 row 4): a feed
        // event landing inside re-classifies the record as spoiled, never a
        // torn observation trusted.
        let bracket = self.feed.bracket();
        let read = read_member(root, rel, law)?;
        let fence_clean = self.feed.clean(bracket);
        if !read.settled {
            // A still-open in-place writer (§6.2 row 5): the leaf is
            // SUSPECT — served this pass, never trusted.
            self.suspect_reads += 1;
        }
        if !fence_clean {
            self.fenced_reads += 1;
        }
        let seen = if read.settled && fence_clean {
            trust.record_seen()
        } else {
            None
        };
        // The fd-true identity of the bytes actually hashed — never the
        // walk's path stat, which the minting write can race.
        Ok(LeafSeen {
            key: read.key,
            digest: model::leaf_digest(&read.bytes),
            seen,
        })
    }

    /// The trust context this pass runs under: calibrate lazily at the first
    /// observation (the cache's workspace open — merkle-spec §6.2 row 2),
    /// then capture the pass watermark from the probe file's own clock. Any
    /// gap — probe unavailable, watermark capture failure — degrades to the
    /// untrusted floor (reuse nothing, spoil every record), LOUDLY, never to
    /// silent trust.
    fn trust_context(&mut self, root: &WorkspaceRoot) -> stable::TrustCtx {
        let dir = stable::meridian_dir(root);
        let calibration = self
            .calibration
            .get_or_insert_with(|| stable::calibrate(&dir));
        let granule_ns = match calibration {
            stable::Calibration::Measured { granule_ns } => *granule_ns,
            stable::Calibration::Unavailable { .. } => return stable::TrustCtx::untrusted(),
        };
        match stable::watermark(&dir) {
            Ok(w) => stable::TrustCtx {
                granule_ns: Some(granule_ns),
                watermark: Some(w),
            },
            Err(e) => {
                eprintln!(
                    "merkle: observation watermark capture failed ({e}) — this pass trusts \
                     no stat identity and spoils its records (merkle-spec 6.2)"
                );
                stable::TrustCtx::untrusted()
            }
        }
    }

    /// [`hash_domain`]'s traversal, with the listing of an unmoved directory
    /// taken from memory instead of from a `read_dir`, parallel across
    /// subtrees.
    ///
    /// A directory's own `mtime`/`ctime` move when an entry is created,
    /// removed, or renamed inside it — that is what a directory's timestamps
    /// ARE — so an unmoved directory has the entry set it had last time. It
    /// says nothing about the CONTENT of the files in it, which is why every
    /// member is still `stat`ed by the caller: this memo skips enumeration, and
    /// never a member's own currency check.
    ///
    /// Same evidence-not-proof standing as [`StatKey`], and it fails the safe
    /// way: an unreadable directory stat re-enumerates rather than trusting
    /// what it remembers.
    ///
    /// The fan-out (ported from arm-A `f84c1912`, re-derived on this memo
    /// walk): even fully memoized, the sweep pays one directory `stat` per
    /// tree node, ~25k serial syscalls at production 2x scale, and the kernel
    /// answers eight at once — so every directory is a work item on a shared
    /// queue (2–8 scoped workers), termination witnessed by
    /// queue-empty-and-none-active, first scan error refusing the sweep. The
    /// caller sorts the member list and folds through ordered maps, so collect
    /// order — the only thing parallelism changes — is invisible to the fold;
    /// which error is named when SEVERAL directories fail at once is the one
    /// nondeterminism, disclosed in the fuse ledger. Trees with no
    /// subdirectories never see a thread. Liveness is untouched: one
    /// synchronous, kernel-fresh metadata pass per call — the constant moved,
    /// not the semantics.
    ///
    /// Returns `(member files, symlink offenders, fresh dir memo, enumerations
    /// run)`. Offenders are non-empty only under [`ObserveLaw::Guarded`]; the
    /// walk COMPLETES before the caller refuses on them, matching the strict
    /// walk's count-plus-first-offender discipline.
    fn walk_tree(
        prior: &DirMemo,
        root: &Path,
        domain: &domain::Domain,
        law: ObserveLaw,
        trust: stable::TrustCtx,
    ) -> io::Result<(Vec<PathBuf>, Vec<String>, DirMemo, u64)> {
        struct Shared {
            /// Rel dirs awaiting a scan.
            todo: VecDeque<PathBuf>,
            /// Scans in flight — with `todo`, the termination witness: the
            /// walk is done when the queue is empty AND no worker holds one.
            active: usize,
            files: Vec<PathBuf>,
            offenders: Vec<String>,
            dirs: Vec<(PathBuf, DirSeen)>,
            listings: u64,
            /// The first scan failure; the sweep fails with it (a tree that
            /// cannot be walked refuses the whole pass, unchanged).
            err: Option<io::Error>,
        }

        let mut files = Vec::new();
        let mut offenders = Vec::new();
        let mut fresh_dirs = BTreeMap::new();
        let mut listings = 0u64;

        // The root's own scan runs serially — flat trees never see a thread.
        let scan = scan_dir(prior, root, Path::new(""), trust)?;
        listings += u64::from(scan.enumerated);
        let (mut root_files, subdirs, mut root_offenders) =
            classify(&scan.entries, Path::new(""), domain, law);
        if let Some(seen) = scan.into_seen() {
            fresh_dirs.insert(PathBuf::new(), seen);
        }
        files.append(&mut root_files);
        offenders.append(&mut root_offenders);
        if subdirs.is_empty() {
            return Ok((files, offenders, fresh_dirs, listings));
        }

        let shared = std::sync::Mutex::new(Shared {
            todo: subdirs.into(),
            active: 0,
            files: Vec::new(),
            offenders: Vec::new(),
            dirs: Vec::new(),
            listings: 0,
            err: None,
        });
        let idle = std::sync::Condvar::new();
        let workers = std::thread::available_parallelism().map_or(2, |n| n.get().clamp(2, 8));

        std::thread::scope(|scope| {
            for _ in 0..workers {
                scope.spawn(|| {
                    loop {
                        let rel = {
                            let mut s = shared.lock().unwrap_or_else(PoisonError::into_inner);
                            loop {
                                if s.err.is_some() {
                                    return;
                                }
                                if let Some(rel) = s.todo.pop_front() {
                                    s.active += 1;
                                    break rel;
                                }
                                if s.active == 0 {
                                    return; // queue drained and nobody holds a scan
                                }
                                s = idle.wait(s).unwrap_or_else(PoisonError::into_inner);
                            }
                        };
                        let scanned = scan_dir(prior, root, &rel, trust).map(|scan| {
                            let split = classify(&scan.entries, &rel, domain, law);
                            (scan, split)
                        });
                        let mut s = shared.lock().unwrap_or_else(PoisonError::into_inner);
                        s.active -= 1;
                        match scanned {
                            Ok((scan, (mut new_files, new_subdirs, mut new_offenders))) => {
                                s.listings += u64::from(scan.enumerated);
                                s.files.append(&mut new_files);
                                s.offenders.append(&mut new_offenders);
                                s.todo.extend(new_subdirs);
                                if let Some(seen) = scan.into_seen() {
                                    s.dirs.push((rel, seen));
                                }
                                // Every push can wake every sleeper: workers
                                // outnumber the queue's contents at the fringe.
                                idle.notify_all();
                            }
                            Err(e) => {
                                s.err.get_or_insert(e);
                                idle.notify_all();
                                return;
                            }
                        }
                    }
                });
            }
        });

        let mut s = shared.into_inner().unwrap_or_else(PoisonError::into_inner);
        if let Some(err) = s.err {
            return Err(err);
        }
        files.append(&mut s.files);
        offenders.append(&mut s.offenders);
        fresh_dirs.extend(s.dirs);
        listings += s.listings;
        Ok((files, offenders, fresh_dirs, listings))
    }

    /// How many directories this memo has ENUMERATED (`read_dir`) over its
    /// life, as against re-used from the listing memo. The walk's counterpart
    /// to [`leaves_read`](Self::leaves_read).
    #[must_use]
    pub fn listings(&self) -> u64 {
        self.listings
    }

    /// The memo's current leaf set — member → §12.2 leaf digest, as of the
    /// last [`root`](Self::root) pass. The incremental rebuild arm's delta
    /// input ([`update_corpus`]): each digest was byte-derived when its
    /// [`StatKey`] last moved, the same evidence grade the reuse check runs
    /// on. A clone, so callers take it only on the rebuild path — never per
    /// currency pass (the hot path stays allocation-free).
    #[must_use]
    pub fn leaf_digests(&self) -> BTreeMap<PathBuf, [u8; 32]> {
        self.leaves
            .iter()
            .map(|(rel, entry)| (rel.clone(), entry.digest))
            .collect()
    }

    /// Own-write overlay, insert/update half (merkle-spec §6.1): the commit
    /// KNOWS the bytes it wrote — replace that leaf and re-fold the ancestor
    /// chain in the resident tree, synchronously. `root_after` then comes
    /// from [`Self::overlay_root`], never a second corpus read: the overlay
    /// is MORE correct than a re-read, because a foreign write racing the
    /// commit never silently enters the folded baseline
    /// ([`DomainLeaves::overlay`]'s own doc law — that overlay stays the
    /// correctness law; this is the same law through the resident structure).
    ///
    /// Membership follows the OBSERVED domain generation: a path outside it
    /// is ignored (`Ok(false)`), exactly [`DomainLeaves::overlay`]'s filter.
    /// The path's memo entry is deliberately spoiled (§6.2's own word): the
    /// next observation re-reads that member once instead of trusting a stat
    /// raced by the write that minted it.
    ///
    /// Returns whether the tree advanced.
    ///
    /// # Errors
    /// No observation has landed yet — an overlay without a baseline is a
    /// caller-order defect, refused rather than guessed around.
    pub fn overlay_leaf(&mut self, rel: &Path, digest: [u8; 32]) -> io::Result<bool> {
        let domain = self.overlay_domain()?;
        if !domain.contains(rel) {
            return Ok(false);
        }
        if !self.tree.set_leaf(rel, digest) {
            return Ok(false);
        }
        self.stamp_advance(rel);
        self.leaves.insert(
            rel.to_path_buf(),
            LeafSeen {
                key: StatKey::spoiled(),
                digest,
                seen: None,
            },
        );
        self.served = None;
        Ok(true)
    }

    /// Own-write overlay, removal half: a governed `remove` unlinked the
    /// leaf — the next fold composes the tree without it (merkle-spec §8,
    /// death Delta; no tombstone). Same domain filter, same spoiling
    /// posture (the memo entry simply leaves), same advance report as
    /// [`Self::overlay_leaf`].
    ///
    /// # Errors
    /// No observation has landed yet (as [`Self::overlay_leaf`]).
    pub fn overlay_remove(&mut self, rel: &Path) -> io::Result<bool> {
        let domain = self.overlay_domain()?;
        if !domain.contains(rel) {
            return Ok(false);
        }
        if !self.tree.remove_leaf(rel) {
            return Ok(false);
        }
        self.stamp_advance(rel);
        self.leaves.remove(rel);
        self.served = None;
        Ok(true)
    }

    /// The served root over the current overlay state — the interim law-1
    /// value (merged plan §6 step 3), folded from the resident structure's
    /// current leaves with NO walk, NO stat, NO byte read. Recomputed only
    /// when the tree advanced since the last serve (lane C class when it
    /// did, a clone when it did not).
    ///
    /// # Errors
    /// No observation has landed yet (as [`Self::overlay_leaf`]).
    pub fn overlay_root(&mut self) -> io::Result<model::MerkleRoot> {
        let version = self.overlay_domain()?.version();
        if let Some((v, cached)) = &self.served
            && *v == version
        {
            return Ok(cached.clone());
        }
        self.flat_folds += 1;
        let leaves: Vec<(&[u8], [u8; 32])> = self
            .leaves
            .iter()
            .map(|(rel, entry)| (hash_name(rel), entry.digest))
            .collect();
        let folded = model::merkle_root_of_leaves(&leaves, version);
        self.served = Some((version, folded.clone()));
        Ok(folded)
    }

    /// Resolve one scope against the resident tree (merkle-spec §7 scope
    /// rows): root, folder, file leaf, `absent` — or the tree's two
    /// `scope_unresolved` refusals (§4.4 collision, kind conflict). Law-2
    /// values, engine-internal until the cutover: no wire surface mints or
    /// serves from this before merged plan §6 step 7.
    ///
    /// # Errors
    /// [`resident::ScopeRefusal`] naming the refusing path.
    pub fn fold_at(&mut self, scope: &Path) -> Result<resident::ScopeFold, resident::ScopeRefusal> {
        self.tree.fold_at(scope)
    }

    /// The law-2 workspace fingerprint of the resident tree (merkle-spec
    /// §4.2.3). Engine-internal until the cutover — the interim SERVED value
    /// is [`Self::root`] / [`Self::overlay_root`]'s law-1 token.
    pub fn law2_fingerprint(&mut self) -> [u8; 32] {
        self.tree.fingerprint()
    }

    /// Live §4.4 collision keys, display-spelled — the loud lint's
    /// queryable face.
    #[must_use]
    pub fn collision_paths(&self) -> Vec<String> {
        self.tree.collision_paths()
    }

    /// Resident-tree instrument totals (probe surface).
    #[must_use]
    pub fn resident_stats(&self) -> resident::ResidentStats {
        self.tree.stats()
    }

    /// Bind the §6.3 stamp plane to a journal epoch: `instance` is the tree
    /// instance id (the workspace ring's B-01 epoch identity), `clock`
    /// answers that ring's current tip. From this call on, every tree
    /// advance stamps its touched chains with `clock() + 1`.
    ///
    /// Rebinding after an epoch change (idle-reap, restart) swaps the
    /// binding and keeps existing stamp values: stamps are max-only, so a
    /// dead epoch's leftovers can only read as "touched" against the young
    /// chain — the compare degrades to the content-fold floor and heals as
    /// the new chain grows past them. Nothing is reset, nothing false-passes.
    pub fn bind_stamps(&mut self, instance: &str, clock: StampClock) {
        self.stamps = Some(StampSource {
            instance: instance.to_owned(),
            clock,
        });
    }

    /// The bound stamp epoch's tree instance id; `None` while the plane is
    /// unbound (nothing stamps, every query degrades).
    #[must_use]
    pub fn stamp_instance(&self) -> Option<&str> {
        self.stamps.as_ref().map(|s| s.instance.as_str())
    }

    /// The §6.3 fast-path fact for one scope against a stamp-bearing token:
    /// `Some(true)` — no journaled change beneath `scope` after the token's
    /// seq (untouched); `Some(false)` — the subtree moved (or a dead
    /// epoch's conservative leftover says so); `None` — stamps cannot
    /// answer: the plane is unbound, the token's instance is not the bound
    /// epoch (restart or reap — §7 row), or the path holds no current node
    /// or leaf (stamps never answer for the dead).
    ///
    /// A `Some(true)` is legal to ACT on only while the event stream can
    /// vouch for this cache (merkle-spec §6.3/§6.4: cookie barrier returned,
    /// dirty set applied, no collapse, [`Self::guard_currency`] trusted) —
    /// that vouch is the caller's fact. Everything else is the
    /// §6.2-governed extent refresh floor: re-derive, never trust a stamp.
    #[must_use]
    pub fn stamp_untouched(&self, instance: &str, seq: u64, scope: &Path) -> Option<bool> {
        let source = self.stamps.as_ref()?;
        if source.instance != instance {
            return None;
        }
        let stamp = self.tree.stamp_at(scope)?;
        Some(stamp <= seq)
    }

    /// Stamp one advanced leaf's ancestor chain, when the plane is bound
    /// (§6.3): `clock() + 1` — one past the journal tip at this instant. For
    /// a choke-point overlay that is exactly the seq the commit's frame
    /// allocates; for an unjournaled advance (feed apply) it is strictly
    /// greater than every token minted before the advance, so the compare
    /// can only degrade, never false-pass.
    fn stamp_advance(&mut self, rel: &Path) {
        let Some(seq) = self.stamps.as_ref().map(|s| (s.clock)() + 1) else {
            return;
        };
        self.tree.stamp_chain(rel, seq);
    }

    /// How many law-1 flat folds ([`model::merkle_root_of_leaves`]) this
    /// cache has run for its served root — the interim law's own instrument:
    /// under it, this advances only when the root advances, never per pass.
    #[must_use]
    pub fn flat_folds(&self) -> u64 {
        self.flat_folds
    }

    /// The §6.2 timestamp-granularity calibration this cache runs under —
    /// the probe's queryable face (the test matrix records it per backend).
    /// `None` until the first observation probes it.
    #[must_use]
    pub fn calibration(&self) -> Option<&stable::Calibration> {
        self.calibration.as_ref()
    }

    /// A handle on the shared feed-generation cell. The event-feed watcher
    /// clones this to advance generations and report loss; the observation
    /// path fences every byte read with the same cell.
    #[must_use]
    pub fn feed_gen(&self) -> stable::FeedGen {
        self.feed.clone()
    }

    /// Guard currency as this cache can vouch for it (merkle-spec §6.2
    /// row 6): LOUD untrusted on no baseline, unknown capability
    /// (calibration unavailable), or unabsorbed event loss — never silent
    /// trust. A COMPLETED observation absorbs losses reported before it
    /// started (a full pass is the rescan ladder's own floor); calibration
    /// unavailability never heals within the workspace open.
    #[must_use]
    pub fn guard_currency(&self) -> stable::GuardCurrency {
        if self.domain_seen.is_none() {
            return stable::GuardCurrency::Untrusted {
                reason: "no observation has landed".to_owned(),
            };
        }
        if let Some(stable::Calibration::Unavailable { reason }) = &self.calibration {
            return stable::GuardCurrency::Untrusted {
                reason: reason.clone(),
            };
        }
        let losses = self.feed.losses();
        if losses > self.acked_losses {
            return stable::GuardCurrency::Untrusted {
                reason: format!(
                    "event loss reported ({} unabsorbed) — a full observation must re-baseline",
                    losses - self.acked_losses
                ),
            };
        }
        stable::GuardCurrency::Trusted
    }

    /// Members re-read because the §6.2 watermark refused their record while
    /// their `StatKey` matched byte-for-byte — the trust close's own
    /// instrument (codex gate 17: key equality alone never passes).
    #[must_use]
    pub fn watermark_rereads(&self) -> u64 {
        self.watermark_rereads
    }

    /// Reads whose fd identity was still moving after the retry budget — a
    /// still-open in-place writer classified SUSPECT (§6.2 row 5); served
    /// that pass, recorded spoiled.
    #[must_use]
    pub fn suspect_reads(&self) -> u64 {
        self.suspect_reads
    }

    /// Reads whose event-generation fence caught a feed event landing inside
    /// the read bracket (§6.2 row 4); recorded spoiled.
    #[must_use]
    pub fn fenced_reads(&self) -> u64 {
        self.fenced_reads
    }

    /// Corpus observation passes attempted over this memo's life — the §7(d)
    /// quiet-workspace counter (codex gate 10): after baseline, a quiet
    /// workspace advances neither this nor [`member_stats`](Self::member_stats).
    #[must_use]
    pub fn sweeps(&self) -> u64 {
        self.sweeps
    }

    /// Member identities `stat`ed across all sweeps — the §7(d) counter
    /// pair's other half. Counted per member per completed identity pass.
    #[must_use]
    pub fn member_stats(&self) -> u64 {
        self.member_stats
    }

    /// The overlay's domain: the OBSERVED generation, refused when none has
    /// landed (composing an overlay against a fresher config read than the
    /// tree's own generation would mix two worlds).
    fn overlay_domain(&self) -> io::Result<domain::Domain> {
        self.domain_seen.clone().ok_or_else(|| {
            io::Error::other("overlay before any observation: the resident tree has no baseline")
        })
    }
}

/// One directory's scan during [`DomainCache`]'s walk: its identity, its
/// entries (remembered, or freshly enumerated when the identity moved), and
/// whether an enumeration actually ran — the walk's unit of work.
struct DirScan {
    key: Option<StatKey>,
    /// The watermark this scan's record rides: a reused listing keeps its
    /// original record watermark; a fresh enumeration takes the pass's
    /// ([`stable::TrustCtx::record_seen`] — `None` under the untrusted
    /// floor, so a later pass re-enumerates).
    seen: Option<stable::FsStamp>,
    entries: Vec<DirEntryKind>,
    enumerated: bool,
}

impl DirScan {
    /// This scan's memo record — `None` when the directory vanished under
    /// the stat (nothing to remember).
    fn into_seen(self) -> Option<DirSeen> {
        self.key.map(|key| DirSeen {
            key,
            seen: self.seen,
            entries: self.entries,
        })
    }
}

/// Scan one directory for [`DomainCache::walk_tree`]: `stat` it, take its
/// listing from `prior` when the identity is unmoved AND the §6.2 watermark
/// law clears the record (a directory whose entry set changed in the same
/// stamp quantum as its remembered scan is exactly the leaf memo's racy
/// window — an unmoved-looking key hiding a moved listing), `read_dir`
/// otherwise. An unreadable directory stat re-enumerates rather than
/// trusting what the memo remembers — the serial walk's own failure posture.
fn scan_dir(
    prior: &DirMemo,
    root: &Path,
    rel_dir: &Path,
    trust: stable::TrustCtx,
) -> io::Result<DirScan> {
    let abs_dir = if rel_dir.as_os_str().is_empty() {
        root.to_path_buf()
    } else {
        root.join(rel_dir)
    };
    let key = StatKey::of_path(&abs_dir)?;
    let remembered = key.and_then(|key| {
        prior
            .get(rel_dir)
            .filter(|seen| seen.key == key && trust.trusts(&key, seen.seen))
            .map(|seen| (seen.entries.clone(), seen.seen))
    });
    let (entries, seen, enumerated) = if let Some((entries, seen)) = remembered {
        (entries, seen, false)
    } else {
        let mut entries = Vec::new();
        for entry in fs::read_dir(&abs_dir)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            entries.push(DirEntryKind {
                is_dir: file_type.is_dir(),
                is_file: file_type.is_file(),
                is_symlink: file_type.is_symlink(),
                name: entry.file_name(),
            });
        }
        (entries, trust.record_seen(), true)
    };
    Ok(DirScan {
        key,
        seen,
        entries,
        enumerated,
    })
}

/// Which walk law a cached observation runs under ([`DomainCache::observe`]).
///
/// The two laws share the listing memo and the leaf memo; they differ only in
/// what a symlink means and how a moved member's bytes are read — exactly the
/// difference between [`hash_domain`]'s plain walk and the guarded walk of
/// [`guard::StepGuard`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ObserveLaw {
    /// The plain domain walk: symlinks are silently outside the domain, byte
    /// reads follow the ordinary read path ([`hash_domain`] semantics).
    Plain,
    /// The guarded walk: any symlink on a non-dot, non-ignored path refuses
    /// the whole observation (count + sorted first offender), and moved
    /// members are read `O_NOFOLLOW` — [`guard::StepGuard`]'s law.
    Guarded,
}

/// Why a cached observation refused — the crate-internal shape
/// [`guard::StepGuard`] maps onto [`guard::GuardError`]. The plain law only
/// ever constructs `Io`.
pub(crate) enum ObserveRefusal {
    /// Underlying I/O failure (config, listing, stat, or member read).
    Io(io::Error),
    /// Guarded law only: symlinked non-dot paths in the walk, refused as a
    /// count plus the first offender in sorted order.
    Symlink {
        /// How many symlinked paths the walk met (≥ 1).
        count: usize,
        /// Workspace-relative forward-slash path of the first offender.
        first: String,
    },
}

impl From<io::Error> for ObserveRefusal {
    fn from(e: io::Error) -> Self {
        ObserveRefusal::Io(e)
    }
}

/// Split a scanned directory's entries into the files the domain admits, the
/// subdirectories worth descending, and — under the guarded law — the symlink
/// offenders. Plain classification applies the same two pruning rules
/// [`walk_domain_dir`] applies (dot-segment structurally outside the domain,
/// [`domain::Domain::prunes_dir`] where re-inclusion is impossible); the
/// guarded law replays [`guard`]'s strict walk over the remembered listing:
/// dot-prefixed entries of ANY kind are skipped first, a symlink at a path the
/// domain's own rules exclude is skipped ([`domain::Domain::skips_symlink`]),
/// and every other symlink is an offender.
fn classify(
    entries: &[DirEntryKind],
    rel_dir: &Path,
    domain: &domain::Domain,
    law: ObserveLaw,
) -> (Vec<PathBuf>, Vec<PathBuf>, Vec<String>) {
    let mut files = Vec::new();
    let mut subdirs = Vec::new();
    let mut offenders = Vec::new();
    for entry in entries {
        let rel = rel_dir.join(&entry.name);
        if law == ObserveLaw::Guarded && entry.name.to_string_lossy().starts_with('.') {
            continue; // strict law: dot paths are outside detection, any kind
        }
        if entry.is_dir {
            if entry.name.to_string_lossy().starts_with('.') {
                continue;
            }
            if domain.prunes_dir(&rel) {
                continue;
            }
            subdirs.push(rel);
        } else if law == ObserveLaw::Guarded && entry.is_symlink {
            if domain.skips_symlink(&rel) {
                continue;
            }
            offenders.push(display_name(hash_name(&rel)));
        } else if entry.is_file && domain.contains(&rel) {
            files.push(rel);
        }
    }
    (files, subdirs, offenders)
}

/// A plain-law [`ObserveRefusal`] back into the `io::Result` world. The plain
/// walk skips symlinks (matching [`hash_domain`]) so it never constructs the
/// symlink arm; the mapping still answers it typed rather than panicking.
fn plain_refusal(refusal: ObserveRefusal) -> io::Error {
    match refusal {
        ObserveRefusal::Io(e) => e,
        ObserveRefusal::Symlink { first, .. } => io::Error::other(format!(
            "symlink refusal on a plain observation (defect — the plain walk skips links): {first}"
        )),
    }
}

/// One moved member's bytes under the §6.2 stable-read law
/// ([`stable::read_settled`]: `O_NOFOLLOW`, fd identity fstatted before and
/// after the bytes — BOTH observation laws; spec §6.2 row 3 is
/// unconditional). What differs per law is the refusal spelling: plain reads
/// keep the corpus-scoped refusal shape ([`DomainCache::root`]'s law) — a
/// member swapped for a symlink between walk and read now refuses there
/// instead of being read through, strictly stricter; guarded reads spell a
/// link racing the walk as the symlink refusal ([`guard`]'s law).
fn read_member(
    root: &WorkspaceRoot,
    rel: &Path,
    law: ObserveLaw,
) -> Result<stable::SettledRead, ObserveRefusal> {
    let abs = root.0.join(rel);
    match stable::read_settled(&abs) {
        Ok(read) => Ok(read),
        Err(e) => match law {
            ObserveLaw::Plain => Err(ObserveRefusal::Io(corpus_member_refusal(
                e.kind(),
                &display_name(hash_name(rel)),
                format!("cannot be read ({e})"),
            ))),
            ObserveLaw::Guarded => {
                if fs::symlink_metadata(&abs).is_ok_and(|m| m.file_type().is_symlink()) {
                    // The walk→read race mints for the ONE path it caught.
                    Err(ObserveRefusal::Symlink {
                        count: 1,
                        first: display_name(hash_name(rel)),
                    })
                } else {
                    Err(ObserveRefusal::Io(e))
                }
            }
        },
    }
}

/// `O_NOFOLLOW` open (unix): opening a symlink fails (`ELOOP`) instead of
/// reading through it. The guarded read primitive, shared by the strict walk
/// ([`guard`]) and the guarded cached observation ([`DomainCache::observe`]).
#[cfg(unix)]
pub(crate) fn open_nofollow(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}

/// Off unix there is no `O_NOFOLLOW`; the guarded walk has already refused
/// symlinks, so only the walk→read race is uncovered here.
#[cfg(not(unix))]
pub(crate) fn open_nofollow(path: &Path) -> io::Result<File> {
    File::open(path)
}

/// Members below this count take the serial stat loop — thread spawn only
/// pays for itself on big domains (floor carried from arm-A `c0d0c8ba`,
/// measured on this hardware class).
const PARALLEL_STAT_FLOOR: usize = 4096;

/// Every member's [`StatKey`] in `rels` order — the currency pass's stat
/// sweep, parallel in ORDER-PRESERVING contiguous chunks at or above `floor`
/// (ported from arm-A `c0d0c8ba`, re-derived on this memo's fold).
///
/// Each scoped worker owns one contiguous chunk of the sorted `rels`; the
/// merge walks chunks in spawn order, so both the row order and the FIRST
/// refusal are identical to the serial loop's: the member named is the first
/// failing one in sorted order, never whichever worker lost a race. The fold
/// itself cannot see the difference either way — the digests land in ordered
/// maps — so parallelism here moves wall time and nothing else. A vanished
/// member refuses corpus-scoped, naming itself, exactly as the serial loop
/// did.
fn member_identities(
    root: &Path,
    rels: &[PathBuf],
    floor: usize,
) -> io::Result<Vec<(PathBuf, StatKey)>> {
    let identity_of = |rel: &PathBuf| -> io::Result<(PathBuf, StatKey)> {
        let key = StatKey::of_path(&root.join(rel))?.ok_or_else(|| {
            // Walked a moment ago and gone now: a corpus-scoped refusal
            // names its member, like every other one here.
            corpus_member_refusal(
                io::ErrorKind::NotFound,
                &display_name(hash_name(rel)),
                "vanished between the domain walk and its stat".to_owned(),
            )
        })?;
        Ok((rel.clone(), key))
    };
    if rels.is_empty() || rels.len() < floor {
        return rels.iter().map(identity_of).collect();
    }
    let workers = std::thread::available_parallelism().map_or(2, |n| n.get().clamp(2, 8));
    let chunk = rels.len().div_ceil(workers);
    let mut rows: Vec<io::Result<Vec<(PathBuf, StatKey)>>> = Vec::new();
    std::thread::scope(|scope| {
        let handles: Vec<_> = rels
            .chunks(chunk)
            .map(|c| scope.spawn(move || c.iter().map(&identity_of).collect()))
            .collect();
        for handle in handles {
            match handle.join() {
                Ok(chunk_rows) => rows.push(chunk_rows),
                Err(panic) => std::panic::resume_unwind(panic),
            }
        }
    });
    let mut out = Vec::with_capacity(rels.len());
    for chunk_rows in rows {
        out.extend(chunk_rows?);
    }
    Ok(out)
}

/// The domain files of a workspace as `(workspace-relative path, raw bytes)`
/// pairs — the shape [`domain_snapshot`] returns and [`build_corpus`] consumes.
pub type DomainFiles = Vec<(String, Vec<u8>)>;

/// Full corpus folds run by [`domain_snapshot`] in this process.
static FOLD_COUNT: AtomicU64 = AtomicU64::new(0);

/// How many full-corpus folds [`domain_snapshot`] has run in this process.
///
/// An instrument, not a cache — it counts folds, it never skips one, so a
/// host's per-request fold budget is assertable rather than timed.
///
/// Process-global and monotonic: read it before and after the work under test
/// and assert the difference. Tests asserting exact counts must not run
/// concurrently with other folding work in the same process.
#[must_use]
pub fn fold_count() -> u64 {
    FOLD_COUNT.load(Ordering::Relaxed)
}

/// The §12 hash-domain snapshot: every SERVABLE domain file's bytes (as
/// `(workspace-relative path, raw bytes)`) plus the corpus [`model::MerkleRoot`]
/// folded over the WHOLE domain — one read, one fold, so a consumer parses
/// the same bytes the root describes and the answer cannot drift from its stamp.
///
/// **Name truthfulness (merkle-spec §4/§9):** the fold carries each member's
/// raw name bytes ([`hash_name`]) — never a lossy decode, never a separator
/// rewrite. A member whose NAME is not valid UTF-8 enters the root with its
/// exact bytes yet does not appear in the returned files: wire paths are UTF-8,
/// so such a member is integrity-covered but unservable — the §3 analog of
/// non-UTF-8 CONTENT (hashed, serves no spans). On any corpus this engine can
/// serve at all, the two sets coincide and the name conversion is identity.
///
/// This is the CHEAP half of a resident rebuild: it reads and folds but does
/// not parse. The daemon uses the returned root as the corpus content hash —
/// the warm-engine reuse key. Pass the returned files to [`build_corpus`] for
/// the parse (they are the same bytes the root folded — no second read).
///
/// # Errors
/// I/O failure loading the domain config, traversing the root, or reading a file.
pub fn domain_snapshot(root: &WorkspaceRoot) -> io::Result<(DomainFiles, model::MerkleRoot)> {
    let (files, _leaves, folded) = domain_snapshot_with_leaves(root)?;
    Ok((files, folded))
}

/// [`domain_snapshot`] with the per-member leaf set alongside the fold — the
/// form a resident engine records so a later incremental pass
/// ([`update_corpus`]) has a delta baseline. Same walk, same read, same fold;
/// the leaves are the very digests the returned root folded.
///
/// # Errors
/// I/O failure loading the domain config, traversing the root, or reading a file.
pub fn domain_snapshot_with_leaves(
    root: &WorkspaceRoot,
) -> io::Result<(DomainFiles, BTreeMap<PathBuf, [u8; 32]>, model::MerkleRoot)> {
    FOLD_COUNT.fetch_add(1, Ordering::Relaxed);
    let domain = domain::Domain::load(root)?;
    let rels = hash_domain(root, &domain)?;
    let members = read_and_digest_members(root, &rels, PARALLEL_READ_FLOOR)?;
    let mut files = Vec::with_capacity(rels.len());
    let mut leaves: BTreeMap<PathBuf, [u8; 32]> = BTreeMap::new();
    for (rel, (bytes, digest)) in rels.iter().zip(members) {
        leaves.insert(rel.clone(), digest);
        if let Some(rel_str) = rel.to_str() {
            files.push((rel_str.to_owned(), bytes));
        }
    }
    let leaf_refs: Vec<(&[u8], [u8; 32])> =
        leaves.iter().map(|(rel, d)| (hash_name(rel), *d)).collect();
    let folded = model::merkle_root_of_leaves(&leaf_refs, domain.version());
    Ok((files, leaves, folded))
}

/// One domain observation's leaves — `raw name bytes → §12.2 leaf digest` —
/// foldable ([`DomainLeaves::root`]) and overlayable
/// ([`DomainLeaves::overlay`]). The overlay is how a caller that KNOWS the
/// one member a commit changed folds the post-commit root without a second
/// sweep: replace that member's leaf, refold. The result is byte-identical
/// to what [`domain_snapshot`] would fold over the post-commit tree, and
/// more computed than a re-read — a foreign write racing the commit never
/// silently enters the folded baseline.
#[derive(Debug)]
pub struct DomainLeaves {
    leaves: BTreeMap<Vec<u8>, [u8; 32]>,
    domain: domain::Domain,
}

impl DomainLeaves {
    /// The fold of these leaves ([`model::merkle_root_of_leaves`] — the same
    /// tree, encoding, and root as [`domain_snapshot`]'s over the same
    /// bytes).
    #[must_use]
    pub fn root(&self) -> model::MerkleRoot {
        let refs: Vec<(&[u8], [u8; 32])> = self
            .leaves
            .iter()
            .map(|(n, d)| (n.as_slice(), *d))
            .collect();
        model::merkle_root_of_leaves(&refs, self.domain.version())
    }

    /// Replace (or insert) one member's leaf digest. A path outside the hash
    /// domain is ignored — the fold never grows a member the domain walk
    /// would not serve (the same filter [`guard::StepGuard::close`] applies
    /// to governed edits).
    pub fn overlay(&mut self, rel: &Path, digest: [u8; 32]) {
        if self.domain.contains(rel) {
            self.leaves.insert(hash_name(rel).to_vec(), digest);
        }
    }
}

/// The domain's current leaves with bytes read only for members whose stat
/// identity moved — [`domain_snapshot`]'s observation served through a
/// caller-held [`digestmemo::DigestMemo`]. An unmoved member reuses its
/// recorded [`model::leaf_digest`]; a moved one is re-read through the same
/// leaf law. Walk semantics are the domain walk's (dot-dirs pruned at
/// descent, symlinks silently skipped); a caller needing the guarded walk
/// uses [`guard::StepGuard`].
///
/// Not counted by [`fold_count`], which counts FULL folds — the number the
/// registry's quiet-cycle gates budget.
///
/// # Errors
/// I/O failure loading the domain config, traversing the root, `stat`ing a
/// member, or reading a member whose identity moved.
pub fn domain_leaves_memoized(
    root: &WorkspaceRoot,
    memo: &mut digestmemo::DigestMemo,
) -> io::Result<DomainLeaves> {
    let domain = domain::Domain::load(root)?;
    let rels = hash_domain(root, &domain)?;
    let identities = member_identities(&root.0, &rels, PARALLEL_STAT_FLOOR)?;
    let mut leaves: BTreeMap<Vec<u8>, [u8; 32]> = BTreeMap::new();
    let mut misses: Vec<(PathBuf, StatKey)> = Vec::new();
    for (rel, key) in identities {
        if let Some(digest) = memo.lookup(&rel, &key) {
            leaves.insert(hash_name(&rel).to_vec(), digest);
        } else {
            misses.push((rel, key));
        }
    }
    let miss_rels: Vec<PathBuf> = misses.iter().map(|(rel, _)| rel.clone()).collect();
    let read = read_and_digest_members(root, &miss_rels, PARALLEL_READ_FLOOR)?;
    for ((rel, key), (_, digest)) in misses.into_iter().zip(read) {
        leaves.insert(hash_name(&rel).to_vec(), digest);
        memo.record(rel, key, digest);
    }
    Ok(DomainLeaves { leaves, domain })
}

/// Below this member count the read+digest sweep stays serial: a fold that
/// small finishes before threads would pay for themselves. Reads move bytes
/// (unlike [`PARALLEL_STAT_FLOOR`]'s stats), so the floor sits much lower.
const PARALLEL_READ_FLOOR: usize = 64;

/// One member's bytes + leaf digest, refusal-shaped exactly as the serial
/// loop always was: a member that cannot be read refuses the whole snapshot,
/// naming the member (`CorpusMemberError`) — the raw OS error carries no path.
fn read_and_digest_member(root: &WorkspaceRoot, rel: &Path) -> io::Result<(Vec<u8>, [u8; 32])> {
    let bytes = fs::read(root.0.join(rel)).map_err(|e| {
        corpus_member_refusal(
            e.kind(),
            &display_name(hash_name(rel)),
            format!("cannot be read ({e})"),
        )
    })?;
    let digest = model::leaf_digest(&bytes);
    Ok((bytes, digest))
}

/// The snapshot's read+digest sweep, parallel above `floor` — the byte half
/// of the git-class pass (§ A.7 measurement companion; the stat half landed
/// with [`DomainCache::walk_tree`] / [`member_identities`], this is the same
/// discipline applied where the bytes are).
///
/// Order-preserving contiguous chunks on 2–8 scoped workers, merged in spawn
/// order — so the returned rows AND the first refusal match the serial loop
/// exactly: chunks are ordered slices, each chunk stops at its own first
/// refusal, and the merge takes the earliest chunk's. A worker panic resumes
/// on the caller (the [`member_identities`] posture).
fn read_and_digest_members(
    root: &WorkspaceRoot,
    rels: &[PathBuf],
    floor: usize,
) -> io::Result<Vec<(Vec<u8>, [u8; 32])>> {
    if rels.is_empty() || rels.len() < floor {
        return rels
            .iter()
            .map(|rel| read_and_digest_member(root, rel))
            .collect();
    }
    // Clamped LOWER than the stat sweep's (2, 8): reads move bytes through
    // the kernel's per-file open/read path, which CONTENDS under wide
    // fan-out — measured on the 24k/146MB corpus, an 8-way `cat` sweep is
    // SLOWER than serial (0.77s vs 0.52s wall, 7x the system CPU) while
    // 3-way is the knee (0.29s). Four threads keep digest work overlapped
    // with reads without tipping into that contention.
    let workers = std::thread::available_parallelism().map_or(2, |n| n.get().clamp(2, 4));
    let chunk = rels.len().div_ceil(workers);
    std::thread::scope(|scope| {
        let handles: Vec<_> = rels
            .chunks(chunk)
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .map(|rel| read_and_digest_member(root, rel))
                        .collect::<io::Result<Vec<_>>>()
                })
            })
            .collect();
        let mut rows = Vec::with_capacity(rels.len());
        for handle in handles {
            match handle.join() {
                Ok(chunk_rows) => rows.extend(chunk_rows?),
                Err(panic) => std::panic::resume_unwind(panic),
            }
        }
        Ok(rows)
    })
}

/// [`domain_snapshot`] over a DIFFERENT INTERVAL: the worktree snapshot with an
/// overlay of the bytes another interval carries, folded by the same domain
/// filter and the same fold.
///
/// [`domain_snapshot`] reads the worktree, while a pre-commit fence is asked
/// about the index: staging a forged file and restoring the worktree leaves a
/// snapshot that describes bytes no commit will record. The overlay is how a
/// caller holding the other interval's bytes folds them through the one fold.
///
/// `overlay` is `(workspace-relative path, content)`: `Some(bytes)` replaces or
/// adds a file, `None` removes one. Entries outside the hash domain are ignored
/// here — they are not hashed in either interval — so a caller may pass whatever
/// its producer reported without filtering it first.
///
/// Names stay the exact strings the inputs carry (merkle-spec §4/§9: a `&str`
/// name folds as its UTF-8 bytes — identity, never a rewrite). The map key is
/// a [`PathBuf`] for ORDER only — component-wise, the order [`walk`] and
/// [`hash_domain`] emit — so the returned list is order-identical to
/// [`domain_snapshot`]'s over the same tree; the emitted NAME is the input
/// string verbatim, carried beside the key, never re-derived from it. Both
/// inputs are UTF-8-named by type, so this fold covers the SERVABLE
/// interval — the same set [`domain_snapshot`] returns.
#[must_use]
pub fn overlay_snapshot(
    worktree: &DomainFiles,
    overlay: &[(String, Option<Vec<u8>>)],
    domain: &domain::Domain,
) -> (DomainFiles, model::MerkleRoot) {
    let mut keyed: BTreeMap<PathBuf, (String, Vec<u8>)> = worktree
        .iter()
        .map(|(rel, bytes)| (PathBuf::from(rel), (rel.clone(), bytes.clone())))
        .collect();
    for (rel, content) in overlay {
        let path = PathBuf::from(rel);
        if !domain.contains(&path) {
            continue;
        }
        match content {
            Some(bytes) => {
                keyed.insert(path, (rel.clone(), bytes.clone()));
            }
            None => {
                keyed.remove(&path);
            }
        }
    }
    let files: DomainFiles = keyed.into_values().collect();
    let entries: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(p, b)| (p.as_str(), b.as_slice()))
        .collect();
    let folded = model::merkle_root(&entries, domain.version());
    (files, folded)
}

/// The raw name bytes a workspace-relative path contributes to a merkle fold
/// (merkle-spec §4/§9 name truthfulness): the `OsStr` bytes verbatim — `/` as
/// segment separator, every other byte a name byte. Never a decode, never a
/// separator rewrite. The ONE conversion point from walk output to fold input,
/// so no hash path can re-grow a lossy spelling.
#[must_use]
pub fn hash_name(rel: &Path) -> &[u8] {
    rel.as_os_str().as_bytes()
}

/// Display spelling for a member name in refusal prose (merkle-spec §9 display
/// law): a valid-UTF-8 name verbatim (identity — two-way, zero loss); a name
/// with no UTF-8 spelling escaped injectively — `\` doubled, each invalid byte
/// as `\xNN` — so a refusal can never name the wrong member. Takes the raw
/// name bytes ([`hash_name`]'s output).
#[must_use]
pub fn display_name(name: &[u8]) -> String {
    if let Ok(s) = std::str::from_utf8(name) {
        return s.to_owned();
    }
    let mut bytes = name;
    let mut out = String::with_capacity(bytes.len() + 8);
    let push_valid = |out: &mut String, s: &str| {
        for c in s.chars() {
            if c == '\\' {
                out.push_str("\\\\");
            } else {
                out.push(c);
            }
        }
    };
    loop {
        match std::str::from_utf8(bytes) {
            Ok(s) => {
                push_valid(&mut out, s);
                return out;
            }
            Err(e) => {
                let (valid, rest) = bytes.split_at(e.valid_up_to());
                // `valid` is valid UTF-8 by `valid_up_to`'s contract.
                push_valid(&mut out, std::str::from_utf8(valid).unwrap_or(""));
                let bad = e.error_len().unwrap_or(rest.len());
                for b in &rest[..bad] {
                    use std::fmt::Write as _;
                    let _ = write!(out, "\\x{b:02X}"); // infallible on String
                }
                bytes = &rest[bad..];
            }
        }
    }
}

/// The typed corpus-scoped refusal: the corpus cannot be served because ONE
/// member fails a condition. A corpus-scoped condition reported without its
/// locus makes every file look individually corrupt — the caller pins the
/// refusal on whatever file they asked for and has nothing to fix. Scope,
/// member, and condition ride together so every face can name the poison file.
/// Carried inside an [`io::Error`] whose kind is the mint's choice (the raw
/// OS kind for a read failure), so existing kind splits keep working; faces
/// that want the locus structurally use [`corpus_member_error`].
#[derive(Debug)]
pub struct CorpusMemberError {
    /// The mint's `io::ErrorKind` — the one discriminator, so the churn
    /// teaching below and the wire's `corpus_race` mapping read the same
    /// fact rather than two.
    pub kind: io::ErrorKind,
    /// The workspace-relative path of the offending member.
    pub member: String,
    /// What the member fails, human-stated (`is not UTF-8 (…)`).
    pub condition: String,
}

impl CorpusMemberError {
    /// The reason alone, without the recovery line. A face that carries the
    /// recovery class STRUCTURALLY — the wire frame's own `recovery` field —
    /// uses this; a text face uses [`Display`](std::fmt::Display), which
    /// teaches the class in words because it has nowhere else to put it.
    #[must_use]
    pub fn reason(&self) -> String {
        format!(
            "the corpus cannot be served: {} {}",
            self.member, self.condition
        )
    }
}

impl std::fmt::Display for CorpusMemberError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.reason())?;
        // A member that is simply GONE is ordinary corpus churn — an
        // unrelated writer moved or deleted a file the caller never named,
        // and the next read derives from the corpus as it is now. Only that
        // family gets the line: a permission fault or a poison member
        // persists, and promising a re-issue would be false. Class per §8's
        // `corpus_race` binding (retry), the same one the wire seam mints.
        if self.kind == io::ErrorKind::NotFound {
            write!(
                f,
                "\n  → the member left the corpus while it was being read, and \
                 nothing you named is wrong — re-issue the call (recovery: retry)"
            )?;
        }
        Ok(())
    }
}

impl std::error::Error for CorpusMemberError {}

/// Mint the corpus-member refusal for `member` — the ONE constructor, so the
/// [`corpus_member_error`] split cannot drift from the mint.
fn corpus_member_refusal(kind: io::ErrorKind, member: &str, condition: String) -> io::Error {
    io::Error::new(
        kind,
        CorpusMemberError {
            kind,
            member: member.to_string(),
            condition,
        },
    )
}

/// The offending corpus member inside an I/O error, when the error is a
/// corpus-scoped refusal ([`CorpusMemberError`]).
#[must_use]
pub fn corpus_member_error(e: &io::Error) -> Option<&CorpusMemberError> {
    e.get_ref()
        .and_then(|inner| inner.downcast_ref::<CorpusMemberError>())
}

/// Parse a [`domain_snapshot`] into the corpus name index + document map — the
/// EXPENSIVE half of a resident rebuild, and the only parser. `files` is
/// consumed (the bytes become the documents' `raw`).
///
/// **Degradation is per-file** (node-rev-merkle-spec §3, "Files that are not
/// valid UTF-8"): a non-UTF-8 member is never lossy-decoded (§8 row 1 — the
/// same refusal [`load`] makes) and never parsed — it serves no spans/nodes —
/// but it does not poison the corpus. Its bytes already participated in the
/// root ([`domain_snapshot`] folds raw bytes), so integrity coverage and span
/// service stay independent properties. The skipped members come back in the
/// third slot as member → condition, so a face asked for one directly can
/// mint the per-file `invalid_utf8` naming it.
#[must_use]
pub fn build_corpus(
    files: DomainFiles,
) -> (
    model::CorpusIndex,
    BTreeMap<String, model::Document>,
    BTreeMap<String, String>,
) {
    let mut docs = BTreeMap::new();
    let mut unserved = BTreeMap::new();
    for (rel, bytes) in files {
        match String::from_utf8(bytes) {
            Ok(text) => {
                let doc = model::build(text.clone(), syntax::parse(&text));
                docs.insert(rel, doc);
            }
            Err(e) => {
                unserved.insert(rel, format!("is not UTF-8 ({})", e.utf8_error()));
            }
        }
    }
    (corpus_index_of(&docs), docs, unserved)
}

/// The corpus name index over `docs`, in the docs map's own (path) order —
/// the ONE index constructor, shared by [`build_corpus`] and
/// [`update_corpus`] so the two build paths cannot disagree on multi-candidate
/// order.
fn corpus_index_of(docs: &BTreeMap<String, model::Document>) -> model::CorpusIndex {
    let mut index = model::CorpusIndex::new();
    for (rel, doc) in docs {
        index.insert(rel, doc);
    }
    index
}

/// One incremental corpus pass's product ([`update_corpus`]): the same parts a
/// from-scratch [`build_corpus`] yields, plus the leaf set and fold they were
/// built at, plus how many documents this pass actually parsed.
#[derive(Debug)]
pub struct CorpusUpdate {
    /// The rebuilt name index — constructed over the FINAL docs map, so it
    /// cannot drift from a from-scratch build over the same tree.
    pub index: model::CorpusIndex,
    /// The updated documents, keyed by workspace-relative path.
    pub docs: BTreeMap<String, model::Document>,
    /// The updated unserved map (non-UTF-8 CONTENT members).
    pub unserved: BTreeMap<String, String>,
    /// The leaf set the fold describes — the next pass's delta baseline.
    pub leaves: BTreeMap<PathBuf, [u8; 32]>,
    /// The fold of `leaves` — what the engine is stamped with.
    pub root: model::MerkleRoot,
    /// Documents parsed by THIS pass (movers only) — the [`build_corpus`]
    /// zero-parse proof, kept per-pass.
    pub parsed: usize,
}

/// Rebuild a corpus INCREMENTALLY against a prior build. `fresh` is the
/// current §12.2 leaf set (a [`DomainCache::leaf_digests`] pass); `prior_*`
/// are the parts and leaf set of the build being updated.
///
/// Per member of `fresh`: an unmoved leaf (digest equal to `prior_leaves`)
/// carries its parsed document — or its unserved condition — forward, bytes
/// untouched; a moved, added, or unaccounted-for member is read NOW, its
/// digest re-derived from the very bytes parsed, so the per-member
/// stamp==bytes atomicity of [`domain_snapshot`] is preserved for everything
/// this pass parses. A member absent from `fresh` is vanished and drops. A
/// mover that vanishes between the leaf pass and the read drops the same way
/// (the fold describes what was actually built); any other read failure
/// refuses the pass whole — the Law A-3c posture, unchanged.
///
/// The returned root folds the resulting leaf set through
/// [`model::merkle_root_of_leaves`] — byte-identical to what
/// [`domain_snapshot`] folds over the same tree state.
///
/// # Errors
/// I/O failure loading the domain config or reading a moved member.
pub fn update_corpus(
    root: &WorkspaceRoot,
    prior_docs: &BTreeMap<String, model::Document>,
    prior_unserved: &BTreeMap<String, String>,
    prior_leaves: &BTreeMap<PathBuf, [u8; 32]>,
    fresh: &BTreeMap<PathBuf, [u8; 32]>,
) -> io::Result<CorpusUpdate> {
    let domain = domain::Domain::load(root)?;
    let mut docs: BTreeMap<String, model::Document> = BTreeMap::new();
    let mut unserved: BTreeMap<String, String> = BTreeMap::new();
    let mut leaves: BTreeMap<PathBuf, [u8; 32]> = BTreeMap::new();
    let mut parsed = 0usize;
    for (rel, fresh_digest) in fresh {
        let carried = prior_leaves.get(rel) == Some(fresh_digest)
            && match rel.to_str() {
                // A UTF-8-named member of the prior build is in exactly one
                // of docs/unserved; a hole means the prior parts do not
                // describe this member — re-read it rather than guess.
                Some(rel_str) => {
                    prior_docs.contains_key(rel_str) || prior_unserved.contains_key(rel_str)
                }
                // Non-UTF-8 NAME: integrity-covered, never servable — the
                // leaf alone is the whole carry.
                None => true,
            };
        if carried {
            leaves.insert(rel.clone(), *fresh_digest);
            if let Some(rel_str) = rel.to_str() {
                if let Some(doc) = prior_docs.get(rel_str) {
                    docs.insert(rel_str.to_owned(), doc.clone());
                } else if let Some(why) = prior_unserved.get(rel_str) {
                    unserved.insert(rel_str.to_owned(), why.clone());
                }
            }
            continue;
        }
        let bytes = match fs::read(root.0.join(rel)) {
            Ok(bytes) => bytes,
            // Vanished since the leaf pass: not part of what this pass
            // builds, exactly as a fresh walk would report it.
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => {
                return Err(corpus_member_refusal(
                    e.kind(),
                    &display_name(hash_name(rel)),
                    format!("cannot be read ({e})"),
                ));
            }
        };
        leaves.insert(rel.clone(), model::leaf_digest(&bytes));
        if let Some(rel_str) = rel.to_str() {
            match String::from_utf8(bytes) {
                Ok(text) => {
                    let doc = model::build(text.clone(), syntax::parse(&text));
                    docs.insert(rel_str.to_owned(), doc);
                    parsed += 1;
                }
                Err(e) => {
                    unserved.insert(
                        rel_str.to_owned(),
                        format!("is not UTF-8 ({})", e.utf8_error()),
                    );
                }
            }
        }
    }
    let leaf_refs: Vec<(&[u8], [u8; 32])> =
        leaves.iter().map(|(rel, d)| (hash_name(rel), *d)).collect();
    let folded = model::merkle_root_of_leaves(&leaf_refs, domain.version());
    Ok(CorpusUpdate {
        index: corpus_index_of(&docs),
        docs,
        unserved,
        leaves,
        root: folded,
        parsed,
    })
}

/// A process-unique suffix source for staging paths (combined with the pid and
/// a nanosecond stamp) so concurrent or retried commits never collide on a temp
/// file name.
static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// The typed write-conflict marker: the splice target's live disk bytes no
/// longer equal the validated pre-image — an out-of-band writer landed
/// between validate and commit. Carried inside an [`io::Error`] (via
/// [`write_conflict`]); callers split it from ordinary I/O failure with
/// [`is_write_conflict`] and map it to their typed refusal.
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

/// The cross-process WRITE lock: an exclusive advisory `flock(2)` on
/// `.meridian/write.lock`, held by the wire write choke-point across its
/// whole critical section (pre-batch read → validate → verify → renames), so
/// two cooperating meridian writers — resident registry daemon, `mrd` — can
/// never interleave read→rename (the lost-update window
/// the in-memory CAS guards cannot see). `LOCK_NB` acquire: a held lock is
/// [`io::ErrorKind::WouldBlock`], surfaced by the caller as the fast typed
/// `workspace_busy` refusal — it never waits, so a hung holder can never make
/// callers hang. Released on drop — by an EXPLICIT unlock, not by the fd
/// close (see [`WriteLock`]'s Drop).
///
/// Stated residuals: out-of-band writers (editors, git, bash) do not take this
/// lock — they are covered by detection (the pre-rename verify →
/// `write_conflict`), not prevention. The run plane serializes on its own
/// `.meridian/run.lock`; run applies and wire splices do not serialize
/// against each other — EXCEPT the daemon-side delta-mint bracket
/// (§ A.8 run-delta ruling): a run apply with an armed frame sink holds
/// THIS lock across its commit and ring advance, so the detector cannot
/// classify a governed run commit as external change.
///
/// `flock` locks belong to the open file description, so two independent
/// acquires contend even within one process — in-process concurrent writers
/// refuse `workspace_busy` exactly like cross-process ones.
#[derive(Debug)]
pub struct WriteLock {
    // Held open for its fd; released by the explicit `flock(LOCK_UN)` in Drop.
    file: File,
    /// The workspace this lock was acquired on. The write door takes its
    /// workspace identity from the lock, so a lock and a root cannot be
    /// paired wrongly.
    root: WorkspaceRoot,
}

/// Release the lock explicitly, before the fd closes: a `flock` lock belongs to
/// the open file description, and a concurrent `fork` in any other thread holds
/// a copy of this fd until its exec (`FD_CLOEXEC` acts at exec, not at fork).
/// Closing our fd in that window would leave the lock held by the child's copy,
/// so every other writer refuses `workspace_busy` for a critical section that
/// already finished. `LOCK_UN` acts on the description itself.
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
    /// to a typed engine error, never unwraps).
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
        Ok(Self {
            file,
            root: root.clone(),
        })
    }

    /// The workspace this lock is held on. A door that needs both a lock and
    /// the workspace being written takes the workspace from HERE instead of
    /// accepting it as a second parameter — two values that must agree become
    /// one value that cannot disagree.
    #[must_use]
    pub fn root(&self) -> &WorkspaceRoot {
        &self.root
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
/// Its private `_sealed` field means only `model`'s CAS validation mints one,
/// so an unvalidated write is unconstructable at this call site. `content_path`
/// receives the content edits (`batch.edits`); when `batch.receipt` is `Some`,
/// `receipt_path` MUST also be `Some` and names the (distinct) receipt file
/// that receives the append. Paths are threaded separately because the seal is
/// deliberately path-less.
///
/// # The validated pre-image
/// `expected_content` is the content file's EXACT bytes the caller validated
/// the batch against (the bytes whose offsets `batch.edits` spans index). The
/// splice SOURCE is these bytes — this function never re-reads the file to
/// splice into, so validated spans can never land in drifted bytes. Before the
/// renames commit anything, the live destination is compared against this
/// pre-image (and the receipt file against its stage-time read): a mismatch
/// refuses with the typed [`write_conflict`] error and no file is touched. The
/// residual window (verify → rename) is stated: cooperating engine writers are
/// serialized by the write flock; out-of-band writers in that gap are a
/// detectable-at-next-read lost update, never a torn or corrupted file.
///
/// # Commit discipline — the atomic-write law (§6.5 + §14)
/// Every byte reaches disk via **tmp + fsync + rename**; no in-place write
/// path exists. Both temp files are fully written and fsync'd FIRST, then the
/// content file is renamed (committing it), then the receipt file — each
/// rename made durable by an fsync of its parent directory.
///
/// # Crash window — a STATED limit (§6.5)
/// A crash BETWEEN the two renames lands content-without-receipt: the content
/// commit is durable, the receipt's is not yet. Because each file is replaced
/// by an atomic rename, no file is ever torn — recovery is re-derive (a cold
/// rebuild yields the correct root of whatever landed) and the orphaned
/// intent is exactly what a receipt lint finds.
///
/// # Seam contract (enforced fail-loud)
/// `receipt_path` presence MUST match `batch.receipt` presence, and
/// `receipt_path` MUST differ from `content_path` — a same-file receipt would
/// let the second rename clobber the first (§6.5 "two files"). Both
/// violations return [`io::ErrorKind::InvalidInput`] before any byte is
/// written. The receipt append must be an empty span (an EOF append) — a
/// replacing receipt span is the same `InvalidInput` refusal.
///
/// # The candidate
/// `candidate` is the sealed [`model::CandidateDocument`] the caller gated —
/// only `model` mints one, so a door that lands bytes without building a
/// candidate does not compile. This primitive's bytes are computed (batch
/// applied to pre-image) rather than supplied, so the tie is checked: a
/// candidate whose bytes differ from the splice result is `InvalidInput`
/// before any temp is written.
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
/// place), refusing if the destination is already occupied (the `if_absent`
/// CAS at file grain). Parent directories are created first — a birth may
/// name a fresh subtree.
///
/// # The candidate
/// The bytes ARE [`model::CandidateDocument::raw`], so the document the caller
/// gated and the bytes that land are the same object by construction.
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

/// Death of one file: remove `rel_path`, then fsync its parent directory so
/// the deletion survives a crash (the rename-durability discipline, applied
/// to unlink). The rev-CAS (remove-what-you-read) is the CALLER's — it
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
/// atomically (tmp+fsync+rename beside the destination — the crate's one
/// write discipline, never in place). Unlike [`create_file`] this carries NO
/// `if_absent` guard: the caller has already CAS-guarded the file's read rev,
/// so the overwrite is the committed edge of a checked write. The
/// destination's parent must exist (a whole-file overwrite never mints a
/// fresh subtree); a missing file is the caller's CAS-drift concern, surfaced
/// here as the rename's own I/O error, never silently created.
///
/// As with [`create_file`], the bytes ARE [`model::CandidateDocument::raw`] —
/// gated document and landed bytes are one object by construction.
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
/// rename), creating the page and its parent directories when absent. `fs`
/// renders NOTHING (crate charter) — the caller passes the rendered line and
/// this only lands bytes. `line` is written verbatim followed by one `\n` (the
/// appender owns terminators; a rendered leaf excludes its own).
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

/// A two-file commit staged to temp files (written + fsync'd), awaiting the
/// two renames. Separating staging from the renames is what lets the
/// crash-honesty test drive a kill BETWEEN the renames deterministically
/// (§6.5). Each staged file carries the pre-image its new bytes were derived
/// from — the pre-rename verify compares the live destination against it.
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
/// [`StagedCommit::commit`] leaves every real file intact.
fn stage_batch(
    root: &WorkspaceRoot,
    content_path: &Path,
    receipt_path: Option<&Path>,
    batch: &model::ValidatedBatch,
    expected_content: &[u8],
    candidate: &model::CandidateDocument,
) -> io::Result<StagedCommit> {
    // Seam contract, enforced BEFORE any disk write (fail-loud).
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
    // pre-image — the spans index exactly these bytes by construction, so
    // the splice can never land in drifted bytes. The live destination is
    // verified against this pre-image at commit, before any rename.
    let content_dst = root.0.join(content_path);
    let content_new = apply_spans(
        expected_content,
        batch.edits.iter().map(|e| (&e.span, e.text.as_str())),
    );

    // The candidate must BE the splice result: the document the caller
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
/// # The append reconcile (receipt half)
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
    /// pre-images (refuse [`write_conflict`] on drift, cleaning the staged
    /// temps), then rename the content file (which COMMITS it), then the
    /// receipt file. The gap between the two renames is the STATED §6.5 crash
    /// window. The verify→rename gap is the stated residual window:
    /// cooperating writers are serialized by the write flock; out-of-band
    /// writers in that gap lose their update detectably (each file is still
    /// fully-old-or-fully-new — never torn).
    fn commit(self) -> io::Result<()> {
        if let Err(conflict) = self.verify_pre_images() {
            self.discard();
            return Err(conflict);
        }
        self.rename_content()?;
        // ┄┄ §6.5 crash window: a crash HERE lands content-without-receipt ┄┄
        self.rename_receipt()
    }

    /// The pre-rename verify: the content destination must still hold the validated
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

// ---------------------------------------------------------------------------
// The set commit (§4.4 set form): N content files + one receipt, sealed
// ---------------------------------------------------------------------------

/// One member of a set commit: a content file, its sealed batch, the validated
/// pre-image the batch's spans index, and the candidate the caller gated.
/// The same per-file contract as [`apply_batch`]'s content half; the receipt
/// never rides a member — it is set-level (one receipt entry names every file).
pub struct SetMember<'a> {
    /// The content file this member edits (workspace-relative).
    pub content_path: &'a Path,
    /// The sealed batch — `receipt` MUST be `None` (set-level receipt only).
    pub batch: &'a model::ValidatedBatch,
    /// The exact pre-image bytes the batch's spans index (read#2's bytes).
    pub expected_content: &'a [u8],
    /// The sealed candidate — must BE this member's splice result.
    pub candidate: &'a model::CandidateDocument,
}

/// Commit a SET of content files plus one optional receipt append in one
/// sealed batch: stage all, verify all pre-images, then rename member order,
/// receipt LAST. Validation and verification run whole-set-first, so any
/// refusal lands NOTHING (the §4.4 set law: validate-all-then-apply).
///
/// # Rollback — in-memory pre-images, no journal (ruling 2026-08-14)
/// A rename FAILURE mid-sequence (process alive) restores every member already
/// renamed from its held pre-image bytes — the same tmp+fsync+rename
/// discipline, run backwards — and the error names the member that failed.
/// There is NO journal file: "effect-less script should be only in memory
/// state, simple is better" (the ruling superseding the §6.5 set-journal
/// draft). A CRASH mid-rename-sequence is therefore an accepted rare window —
/// stated like the single-commit §6.5 window: every file is still
/// fully-old-or-fully-new (atomic renames, never torn), a cold rebuild yields
/// the correct root of whatever landed, and receipt-rename-LAST keeps §6.6
/// honest — a resolvable receipt anchor still implies the whole set landed.
///
/// # Seam contract (enforced fail-loud, `InvalidInput` before any byte)
/// Two or more members; content paths pairwise distinct; the receipt path
/// distinct from every content path; every member's `batch.receipt` is `None`
/// (the set receipt is passed set-level, never per member); each member's
/// candidate is its batch's splice result.
///
/// # Errors
/// Seam violations (`InvalidInput`), the typed [`write_conflict`] refusal
/// (any live destination ≠ its pre-image — nothing landed), or I/O failure at
/// a stage, fsync, or rename step (with the restore outcome named).
pub fn apply_set(
    root: &WorkspaceRoot,
    members: &[SetMember<'_>],
    receipt: Option<(&Path, &model::ReceiptAppend)>,
) -> io::Result<()> {
    stage_set(root, members, receipt)?.commit()
}

/// A staged set commit: every content temp plus the optional receipt temp,
/// each carrying the pre-image its verify compares against. Separated from
/// the renames so the rollback/crash tests can drive failure between staging
/// and each rename deterministically (the [`StagedCommit`] precedent).
struct StagedSet {
    /// `(staged file, validated pre-image)` per member, in member order.
    contents: Vec<(StagedFile, Vec<u8>)>,
    /// The receipt temp and its stage-time pre-image (absent file ⇒ empty).
    receipt: Option<(StagedFile, Vec<u8>)>,
}

/// Stage a whole set: seam contract first, then apply each member's sealed
/// spans to its pre-image and stage the result, then stage the receipt
/// append. A failure anywhere discards every temp already staged — staging is
/// entirely off to the side, so no destination is touched.
fn stage_set(
    root: &WorkspaceRoot,
    members: &[SetMember<'_>],
    receipt: Option<(&Path, &model::ReceiptAppend)>,
) -> io::Result<StagedSet> {
    if members.len() < 2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a set commit takes two or more content members; one file is apply_batch's path",
        ));
    }
    for (i, m) in members.iter().enumerate() {
        if m.batch.receipt.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a set member's batch must not carry a receipt append: the set receipt is \
                 set-level (one receipt entry names every file)",
            ));
        }
        if members[..i]
            .iter()
            .any(|p| p.content_path == m.content_path)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "set content paths must be pairwise distinct: `{}` appears twice",
                    m.content_path.display()
                ),
            ));
        }
        if let Some((rp, _)) = receipt
            && rp == m.content_path
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "receipt_path equals a set content path: the receipt rename would clobber (§6.5)",
            ));
        }
    }

    let mut contents: Vec<(StagedFile, Vec<u8>)> = Vec::with_capacity(members.len());
    let discard_staged = |contents: &[(StagedFile, Vec<u8>)]| {
        for (staged, _) in contents {
            let _ = fs::remove_file(&staged.tmp);
        }
    };
    for m in members {
        let new_bytes = apply_spans(
            m.expected_content,
            m.batch.edits.iter().map(|e| (&e.span, e.text.as_str())),
        );
        if new_bytes != m.candidate.raw().as_bytes() {
            discard_staged(&contents);
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "candidate document is not this batch's splice result for `{}`: the gated \
                     document and the landing bytes must be the same object",
                    m.content_path.display()
                ),
            ));
        }
        match stage_file(&root.0.join(m.content_path), &new_bytes) {
            Ok(staged) => contents.push((staged, m.expected_content.to_vec())),
            Err(e) => {
                discard_staged(&contents);
                return Err(e);
            }
        }
    }

    let receipt = match receipt {
        Some((rp, append)) => match stage_receipt(&root.0.join(rp), append) {
            Ok((staged, old)) => Some((staged, old)),
            Err(e) => {
                discard_staged(&contents);
                return Err(e);
            }
        },
        None => None,
    };

    Ok(StagedSet { contents, receipt })
}

impl StagedSet {
    /// Commit the set: verify every live destination still equals its
    /// pre-image (refuse [`write_conflict`] with nothing landed), then rename
    /// member order, receipt last. A rename failure restores every member
    /// already renamed from its held pre-image (in-memory rollback — no
    /// journal, ruling 2026-08-14); a crash mid-sequence is the stated set
    /// window (see [`apply_set`]).
    fn commit(self) -> io::Result<()> {
        if let Err(conflict) = self.verify_pre_images() {
            self.discard();
            return Err(conflict);
        }
        for i in 0..self.contents.len() {
            if let Err(e) = commit_rename(&self.contents[i].0) {
                let restore = self.restore_renamed(i);
                self.discard();
                return Err(rollback_error(
                    &e,
                    &self.contents[i].0.dst,
                    "content rename",
                    &restore,
                ));
            }
        }
        // ┄┄ stated set window: a crash between here and the receipt rename
        // lands content-without-receipt for the whole set — §6.6 stays honest
        // (no resolvable anchor ⇒ the caller cannot mistake it for landed) ┄┄
        if let Some((staged, _)) = &self.receipt
            && let Err(e) = commit_rename(staged)
        {
            let restore = self.restore_renamed(self.contents.len());
            self.discard();
            return Err(rollback_error(&e, &staged.dst, "receipt rename", &restore));
        }
        Ok(())
    }

    /// The pre-rename verify, whole set: every content destination must still
    /// hold its validated pre-image (gone ⇒ conflict — read#2 saw a real
    /// file), and the receipt destination its stage-time bytes. All checks run
    /// BEFORE the first rename, so a refusal commits nothing.
    fn verify_pre_images(&self) -> io::Result<()> {
        for (staged, expected) in &self.contents {
            let live = match fs::read(&staged.dst) {
                Ok(bytes) => bytes,
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    return Err(write_conflict(&staged.dst));
                }
                Err(e) => return Err(e),
            };
            if live != *expected {
                return Err(write_conflict(&staged.dst));
            }
        }
        if let Some((staged, expected)) = &self.receipt
            && read_or_empty(&staged.dst)? != *expected
        {
            return Err(write_conflict(&staged.dst));
        }
        Ok(())
    }

    /// Restore the first `renamed` members to their held pre-images — stage
    /// the pre-image bytes and rename them back, the same atomic discipline.
    /// Best-effort: a member whose restore itself fails is reported, never
    /// silently left ambiguous.
    fn restore_renamed(&self, renamed: usize) -> RestoreOutcome {
        let mut restored = Vec::new();
        let mut failed = Vec::new();
        for (staged, pre_image) in self.contents.iter().take(renamed).rev() {
            let outcome = stage_file(&staged.dst, pre_image).and_then(|s| commit_rename(&s));
            match outcome {
                Ok(()) => restored.push(staged.dst.display().to_string()),
                Err(e) => failed.push(format!("{} ({e})", staged.dst.display())),
            }
        }
        RestoreOutcome { restored, failed }
    }

    /// Remove every staged temp still on disk (hygiene — no litter). Renamed
    /// temps are already gone; `remove_file` on them is a no-op error, ignored.
    fn discard(&self) {
        for (staged, _) in &self.contents {
            let _ = fs::remove_file(&staged.tmp);
        }
        if let Some((staged, _)) = &self.receipt {
            let _ = fs::remove_file(&staged.tmp);
        }
    }
}

/// What an in-memory rollback managed to undo, for the loud error.
struct RestoreOutcome {
    restored: Vec<String>,
    failed: Vec<String>,
}

/// The loud rollback error: names the rename that failed, what was restored,
/// and — when a restore itself failed — exactly which files remain in their
/// NEW state, so recovery is a statement, never a guess.
fn rollback_error(
    cause: &io::Error,
    dst: &Path,
    step: &str,
    restore: &RestoreOutcome,
) -> io::Error {
    use std::fmt::Write as _;
    let mut msg = format!(
        "set commit failed at the {step} for `{}`: {cause}. ",
        dst.display()
    );
    if restore.restored.is_empty() && restore.failed.is_empty() {
        msg.push_str("No member had renamed yet — nothing landed.");
    } else {
        if !restore.restored.is_empty() {
            let _ = write!(
                msg,
                "Rolled back to pre-images: {}. ",
                restore.restored.join(", ")
            );
        }
        if !restore.failed.is_empty() {
            let _ = write!(
                msg,
                "ROLLBACK INCOMPLETE — these files hold the NEW bytes and their restore \
                 failed: {}. Restore them from the receipt-less new state or re-run the \
                 set once the cause clears.",
                restore.failed.join(", ")
            );
        }
    }
    io::Error::new(cause.kind(), msg)
}

/// Filesystem watcher: the DETECTION primitive — a §12
/// domain baseline plus byte-level change classification against a fresh
/// snapshot. The watcher detects; root folding is `model`'s, Delta emission
/// is the serving host's, and hook *dispatch* (running agent work on change)
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
        SetMember, TEMP_SEQ, USER_RULES_DIR, WorkspaceRoot, apply_batch, apply_set,
        is_write_conflict, stage_batch, stage_set, temp_path_for, user_rule_pages, walk,
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

    /// Obtain a sealed `ValidatedBatch` — the only way is through `model`'s
    /// `validate_batch`.
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

    /// The sealed candidate the byte-landing primitives demand, for this
    /// fixture's batch over `PLAN_S0`.
    fn candidate(vb: &model::ValidatedBatch) -> model::CandidateDocument {
        model::candidate_of_batch("notes/plan.md", PLAN_S0, vb)
    }

    /// The receipt lint (§6.5 recovery): does the receipt file record the anchor
    /// a committed batch should have written? A test helper only — `fs` never
    /// interprets content (crate charter), and §6.4 puts the production lint in
    /// the policy/Go layer.
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

    // ── the set commit (§4.4 set form) ──────────────────────────────────────

    /// Shared pre-image for every set-fixture content file.
    const SET_S0: &str = "# Goals\n\nship by August\n";

    fn set_edit(new_text: &str) -> model::Edit {
        model::Edit {
            target: model::Ref::Hpath(vec![model::HpathSeg {
                h: "Goals".into(),
                n: None,
            }]),
            edit: model::EditKind::Match {
                old: "ship by August".into(),
                new: new_text.into(),
            },
            if_node_rev: None,
        }
    }

    /// A sealed one-edit batch over [`SET_S0`], receipt-less (set-level receipt).
    fn set_validated(new_text: &str) -> model::ValidatedBatch {
        let doc = model::build(SET_S0.to_string(), syntax::parse(SET_S0));
        let req = model::SpliceRequest {
            if_root: None,
            edits: vec![set_edit(new_text)],
            engine: None,
        };
        match model::validate_batch(&doc, None, &req, None) {
            model::SpliceVerdict::Validated(vb) => vb,
            other => panic!("set fixture batch must validate, got {other:?}"),
        }
    }

    /// Three content files in three separate directories (so a permission
    /// sabotage hits ONE member's rename) plus the receipt file.
    fn set_workspace() -> (tempfile::TempDir, WorkspaceRoot, Vec<PathBuf>) {
        let dir = tempfile::tempdir().expect("tempdir");
        let rels: Vec<PathBuf> = (1..=3)
            .map(|i| PathBuf::from(format!("notes/d{i}/f{i}.md")))
            .collect();
        for rel in &rels {
            fs::create_dir_all(dir.path().join(rel.parent().unwrap())).unwrap();
            fs::write(dir.path().join(rel), SET_S0).unwrap();
        }
        fs::create_dir_all(dir.path().join("receipts")).unwrap();
        fs::write(dir.path().join(receipt_rel()), RECEIPT_OLD).unwrap();
        let root = WorkspaceRoot(dir.path().to_path_buf());
        (dir, root, rels)
    }

    /// `(batch, candidate)` per member — owned so members can borrow them.
    fn set_batches(rels: &[PathBuf]) -> Vec<(model::ValidatedBatch, model::CandidateDocument)> {
        rels.iter()
            .enumerate()
            .map(|(i, rel)| {
                let vb = set_validated(&format!("ship by September-{}", i + 1));
                let cand = model::candidate_of_batch(&rel.to_string_lossy(), SET_S0, &vb);
                (vb, cand)
            })
            .collect()
    }

    fn set_members<'a>(
        rels: &'a [PathBuf],
        owned: &'a [(model::ValidatedBatch, model::CandidateDocument)],
    ) -> Vec<SetMember<'a>> {
        rels.iter()
            .zip(owned)
            .map(|(rel, (vb, cand))| SetMember {
                content_path: rel,
                batch: vb,
                expected_content: SET_S0.as_bytes(),
                candidate: cand,
            })
            .collect()
    }

    /// SET GATE 1: an N-file set plus receipt lands whole — every member holds
    /// its new bytes, the receipt appended, no staged litter anywhere.
    #[test]
    fn set_commit_lands_every_member_receipt_last() {
        let (dir, root, rels) = set_workspace();
        let owned = set_batches(&rels);
        let members = set_members(&rels, &owned);
        let append = receipt_append();

        apply_set(&root, &members, Some((&receipt_rel(), &append))).expect("the set lands whole");

        for (i, rel) in rels.iter().enumerate() {
            assert_eq!(
                fs::read(dir.path().join(rel)).unwrap(),
                SET_S0
                    .replace("ship by August", &format!("ship by September-{}", i + 1))
                    .as_bytes(),
                "member {} holds its new bytes",
                i + 1
            );
            assert!(
                !any_tmp_in(&dir.path().join(rel.parent().unwrap())),
                "no staged litter beside member {}",
                i + 1
            );
        }
        assert_eq!(
            fs::read(dir.path().join(receipt_rel())).unwrap(),
            format!("{RECEIPT_OLD}{RECEIPT_LINE}").as_bytes(),
            "receipt appended (last rename)"
        );
    }

    /// SET GATE 2 (validate-all-then-apply): pre-image drift on ANY member
    /// refuses the WHOLE set with the typed write-conflict — no member's bytes
    /// move, no staged litter survives.
    #[test]
    fn set_verify_drift_refuses_whole_nothing_landed() {
        let (dir, root, rels) = set_workspace();
        let owned = set_batches(&rels);
        let members = set_members(&rels, &owned);
        let append = receipt_append();

        let staged = stage_set(&root, &members, Some((&receipt_rel(), &append))).unwrap();
        // Out-of-band writer moves member 2 between stage and commit.
        let drifted = "# Goals\n\nship by NEVER\n";
        fs::write(dir.path().join(&rels[1]), drifted).unwrap();

        let err = staged.commit().expect_err("drift refuses the whole set");
        assert!(is_write_conflict(&err), "typed write-conflict, got {err:?}");
        assert_eq!(
            fs::read(dir.path().join(&rels[0])).unwrap(),
            SET_S0.as_bytes(),
            "member 1 untouched — nothing landed"
        );
        assert_eq!(
            fs::read(dir.path().join(&rels[1])).unwrap(),
            drifted.as_bytes(),
            "the out-of-band bytes stand (never clobbered)"
        );
        assert_eq!(
            fs::read(dir.path().join(&rels[2])).unwrap(),
            SET_S0.as_bytes(),
            "member 3 untouched — nothing landed"
        );
        assert_eq!(
            fs::read(dir.path().join(receipt_rel())).unwrap(),
            RECEIPT_OLD.as_bytes(),
            "receipt untouched"
        );
        for rel in &rels {
            assert!(
                !any_tmp_in(&dir.path().join(rel.parent().unwrap())),
                "refusal leaves no staged litter"
            );
        }
    }

    /// SET GATE 3 (in-memory rollback, no journal — ruling 2026-08-14): a
    /// rename FAILURE mid-sequence restores every already-renamed member to
    /// its pre-image, and the error names the failing member.
    #[test]
    #[cfg(unix)]
    fn set_rename_failure_rolls_back_previous_renames() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, root, rels) = set_workspace();
        let owned = set_batches(&rels);
        let members = set_members(&rels, &owned);

        let staged = stage_set(&root, &members, None).unwrap();
        // Member 3's parent goes read-only AFTER staging: verify still reads,
        // the rename fails EACCES — members 1 and 2 have already renamed.
        let locked = dir.path().join(rels[2].parent().unwrap());
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).unwrap();

        let err = staged.commit().expect_err("member 3's rename fails");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();

        let msg = err.to_string();
        assert!(
            msg.contains("f3.md") && msg.contains("Rolled back"),
            "error names the failing member and the rollback: {msg}"
        );
        for (i, rel) in rels.iter().enumerate() {
            assert_eq!(
                fs::read(dir.path().join(rel)).unwrap(),
                SET_S0.as_bytes(),
                "member {} back at its pre-image (in-memory rollback)",
                i + 1
            );
        }
    }

    /// SET GATE 4 (receipt-rename-last is load-bearing): a receipt rename
    /// failure restores ALL content members — in every reachable non-crash
    /// state, a resolvable receipt anchor implies the whole set landed (§6.6).
    #[test]
    #[cfg(unix)]
    fn set_receipt_rename_failure_restores_all_members() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, root, rels) = set_workspace();
        let owned = set_batches(&rels);
        let members = set_members(&rels, &owned);
        let append = receipt_append();

        let staged = stage_set(&root, &members, Some((&receipt_rel(), &append))).unwrap();
        let locked = dir.path().join("receipts");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).unwrap();

        let err = staged.commit().expect_err("the receipt rename fails");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();

        assert!(
            err.to_string().contains("receipt rename"),
            "error names the receipt step: {err}"
        );
        for (i, rel) in rels.iter().enumerate() {
            assert_eq!(
                fs::read(dir.path().join(rel)).unwrap(),
                SET_S0.as_bytes(),
                "member {} restored — no content landed without its receipt",
                i + 1
            );
        }
        assert_eq!(
            fs::read(dir.path().join(receipt_rel())).unwrap(),
            RECEIPT_OLD.as_bytes(),
            "receipt file untouched"
        );
    }

    /// SET GATE 5: the seam contract refuses fail-loud before any byte —
    /// fewer than two members, duplicate paths, a member-level receipt, a
    /// receipt path colliding with a content path.
    #[test]
    fn set_seam_contract_refuses_before_any_byte() {
        let (dir, root, rels) = set_workspace();
        let owned = set_batches(&rels);
        let append = receipt_append();

        // Fewer than two members.
        let one = set_members(&rels[..1], &owned[..1]);
        let err = apply_set(&root, &one, None).expect_err("one member refuses");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);

        // Duplicate content paths.
        let dup_rels = vec![rels[0].clone(), rels[0].clone()];
        let dup = set_members(&dup_rels, &owned[..2]);
        let err = apply_set(&root, &dup, None).expect_err("duplicate path refuses");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);

        // A member-level receipt append (must be set-level).
        let with_receipt = validated(Some(receipt_append()));
        let cand = model::candidate_of_batch(&rels[0].to_string_lossy(), PLAN_S0, &with_receipt);
        let bad = vec![
            SetMember {
                content_path: &rels[0],
                batch: &with_receipt,
                expected_content: PLAN_S0.as_bytes(),
                candidate: &cand,
            },
            SetMember {
                content_path: &rels[1],
                batch: &owned[1].0,
                expected_content: SET_S0.as_bytes(),
                candidate: &owned[1].1,
            },
        ];
        let err = apply_set(&root, &bad, None).expect_err("member-level receipt refuses");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);

        // Receipt path collides with a content path.
        let two = set_members(&rels[..2], &owned[..2]);
        let err = apply_set(&root, &two, Some((&rels[0], &append)))
            .expect_err("receipt==content refuses");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);

        // Nothing anywhere moved.
        for rel in &rels {
            assert_eq!(
                fs::read(dir.path().join(rel)).unwrap(),
                SET_S0.as_bytes(),
                "seam refusals touch nothing"
            );
        }
    }

    /// GATE 1 (§6.5 crash honesty): a crash injected BETWEEN the two renames
    /// leaves content-without-receipt; a cold rebuild yields the correct root;
    /// the receipt lint finds the orphan intent.
    #[test]
    fn gate1_crash_between_renames_is_honest() {
        let (dir, root) = workspace();
        let vb = validated(Some(receipt_append()));

        // Stage both temps, commit only the content rename, then "crash" before
        // the receipt rename — the §6.5 window, driven deterministically.
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

    /// Seam contract: the batch reaching `fs` is the sealed
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

    /// `apply_batch`'s candidate must BE the splice result: its bytes are
    /// computed from the batch, so a caller could otherwise satisfy the type
    /// with any document it happened to hold. Both halves are asserted — the
    /// same call with the true candidate commits.
    #[test]
    fn apply_batch_refuses_a_candidate_that_is_not_the_splice_result() {
        let (dir, root) = workspace();
        let vb = validated(None);

        // An honestly-minted candidate of the wrong bytes — only the tie is
        // broken, nothing about the type is forged.
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

        // The identical call with the true candidate lands, so the refusal
        // above discriminates rather than blocks.
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
        let page = Path::new("meridian/appended.md");
        super::append_line(&root, page, "- first").unwrap();
        super::append_line(&root, page, "- second").unwrap();
        assert_eq!(
            fs::read(dir.path().join(page)).unwrap(),
            b"- first\n- second\n",
            "each rendered line is appended verbatim with one terminator",
        );
    }

    // ── TOCTOU external-writer conflicts, driven deterministically ──────────
    // ── through the stage/commit seam ───────────────────────────────────────

    /// External overwrite: an out-of-band writer replaces the content
    /// file between staging (validate) and the rename — the commit refuses the
    /// typed write-conflict, the external bytes SURVIVE untouched (never
    /// clobbered by stale validated spans), the receipt never lands, and no
    /// staged temp litters the tree.
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

    /// External delete: the content file VANISHES between staging and
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

    /// Receipt moved before staging: the receipt file gained rows
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

    /// Receipt shrunk: the receipt file was truncated below the rendered span
    /// offset — the same typed conflict refusal, never a panic (a blind
    /// `apply_spans` would slice out of bounds).
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

    /// Receipt drifts between stage and rename: the receipt gains a
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

    /// The write flock contends across independent acquires (flock is
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

    /// No `MERIDIAN.md` ⇒ an empty user layer. The fixture holds both a
    /// `rules/` tree of candidates and a `$HOME`-shaped sibling tree a widened
    /// walk would read; neither is reached.
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

/// Design tests for [`domain_stat_signature`]: the cheap change signal that
/// replaced a per-second read of the whole corpus.
#[cfg(test)]
mod stat_signature_tests {
    use super::{WorkspaceRoot, domain_snapshot, domain_stat_signature};
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    fn workspace(files: &[(&str, &str)]) -> (tempfile::TempDir, WorkspaceRoot) {
        let tmp = tempfile::tempdir().unwrap();
        for (rel, body) in files {
            let path = tmp.path().join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, body).unwrap();
        }
        let root = WorkspaceRoot(fs::canonicalize(tmp.path()).unwrap());
        (tmp, root)
    }

    /// The gate the pre-warm skip rests on: an untouched corpus signs the same
    /// twice, so a sweep can tell "nothing moved" from "something did".
    #[test]
    fn an_untouched_corpus_signs_the_same_twice() {
        let (_tmp, root) = workspace(&[("a.md", "# A\n"), ("sub/b.md", "# B\n")]);
        let first = domain_stat_signature(&root).unwrap();
        assert_eq!(
            first,
            domain_stat_signature(&root).unwrap(),
            "a quiet corpus must sign identically, or the skip never fires"
        );
    }

    /// And the other half: a signal that never changes would freeze pre-warm
    /// permanently. Size, path set, and mtime each move it.
    #[test]
    fn every_kind_of_corpus_change_moves_the_signature() {
        let (tmp, root) = workspace(&[("a.md", "# A\n")]);
        let base = domain_stat_signature(&root).unwrap();

        fs::write(tmp.path().join("a.md"), "# A grown longer\n").unwrap();
        let grown = domain_stat_signature(&root).unwrap();
        assert_ne!(base, grown, "a changed file size must move the signature");

        fs::write(tmp.path().join("b.md"), "# B\n").unwrap();
        let added = domain_stat_signature(&root).unwrap();
        assert_ne!(grown, added, "a new domain file must move the signature");

        fs::remove_file(tmp.path().join("b.md")).unwrap();
        let removed = domain_stat_signature(&root).unwrap();
        assert_ne!(
            added, removed,
            "a removed domain file must move the signature"
        );
        assert_eq!(grown, removed, "and removal must return to the prior shape");
    }

    /// The signal reads no file content: a domain file with no read permission
    /// is invisible to `domain_snapshot` (it fails) and fully visible here.
    #[test]
    fn the_signature_stats_the_corpus_without_reading_a_byte_of_it() {
        let (tmp, root) = workspace(&[("a.md", "# A\n")]);
        let readable = domain_stat_signature(&root).unwrap();

        let secret = tmp.path().join("a.md");
        fs::set_permissions(&secret, fs::Permissions::from_mode(0o000)).unwrap();

        assert!(
            domain_snapshot(&root).is_err(),
            "the content fold must need read permission — otherwise this test proves nothing"
        );
        assert_eq!(
            domain_stat_signature(&root).unwrap(),
            readable,
            "the stat signal must not depend on reading the bytes"
        );

        fs::set_permissions(&secret, fs::Permissions::from_mode(0o644)).unwrap();
    }
}

/// Design tests for the two degradation grains. A member the snapshot cannot
/// READ refuses the whole corpus and names its scope and offending member
/// ([`CorpusMemberError`]) — no bytes, no leaf hash, nothing to degrade to. A
/// member that reads but is not UTF-8 degrades PER-FILE (node-rev-merkle-spec
/// §3): the parse skips it and reports it, and the corpus serves.
#[cfg(test)]
mod corpus_refusal_tests {
    use super::{WorkspaceRoot, build_corpus, corpus_member_error, domain_snapshot};
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    fn workspace(files: &[(&str, &[u8])]) -> (tempfile::TempDir, WorkspaceRoot) {
        let tmp = tempfile::tempdir().unwrap();
        for (rel, bytes) in files {
            let path = tmp.path().join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }
        let root = WorkspaceRoot(fs::canonicalize(tmp.path()).unwrap());
        (tmp, root)
    }

    /// One poison member is skipped and REPORTED, never fatal: the healthy
    /// member parses, the poison member serves no spans/nodes, and the
    /// unserved slot names member and condition so a face asked for it
    /// directly can mint the per-file `invalid_utf8`.
    #[test]
    fn a_poison_member_is_skipped_and_reported_not_fatal() {
        let (_tmp, root) = workspace(&[
            ("healthy.md", b"# Healthy\n".as_slice()),
            ("notes/poison.md", b"# P\n\xff\xfe\n".as_slice()),
        ]);
        let (files, _) = domain_snapshot(&root).unwrap();
        let (_index, docs, unserved) = build_corpus(files);

        assert!(docs.contains_key("healthy.md"), "the healthy member parses");
        assert!(
            !docs.contains_key("notes/poison.md"),
            "the poison member serves no spans/nodes"
        );
        let condition = unserved
            .get("notes/poison.md")
            .expect("the skipped member is reported, keyed by its path");
        assert!(condition.contains("UTF-8"), "condition: {condition}");
    }

    /// The other corpus-scoped class: a member the snapshot cannot READ refuses
    /// the whole snapshot, so that refusal names the member too — the raw OS
    /// error carries no path at all.
    #[test]
    fn an_unreadable_member_is_named_by_the_snapshot_refusal() {
        let (tmp, root) = workspace(&[
            ("healthy.md", b"# Healthy\n".as_slice()),
            ("sub/secret.md", b"# S\n".as_slice()),
        ]);
        let secret = tmp.path().join("sub/secret.md");
        fs::set_permissions(&secret, fs::Permissions::from_mode(0o000)).unwrap();

        let err = domain_snapshot(&root).unwrap_err();
        let member = corpus_member_error(&err).expect("the typed locus rides the error");
        assert_eq!(member.member, "sub/secret.md");
        assert!(
            err.to_string().contains("sub/secret.md"),
            "the message names the member: {err}"
        );

        fs::set_permissions(&secret, fs::Permissions::from_mode(0o644)).unwrap();
    }

    /// A healthy corpus is untouched by the refusal plumbing: it parses, and
    /// nothing is reported unserved.
    #[test]
    fn a_healthy_corpus_still_parses() {
        let (_tmp, root) = workspace(&[("a.md", b"# A\n".as_slice())]);
        let (files, _) = domain_snapshot(&root).unwrap();
        let (_index, docs, unserved) = build_corpus(files);
        assert_eq!(docs.len(), 1);
        assert!(unserved.is_empty());
    }
}

#[cfg(test)]
mod domain_cache_tests {
    //! The currency memo: same root as the byte-derived fold, reading only what
    //! moved (`docs/run-plane.md` § What an entry costs).

    use super::{DomainCache, WorkspaceRoot, domain_snapshot};
    use std::fs;

    fn workspace(files: &[(&str, &[u8])]) -> (tempfile::TempDir, WorkspaceRoot) {
        let tmp = tempfile::tempdir().unwrap();
        for (rel, bytes) in files {
            let path = tmp.path().join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }
        let root = WorkspaceRoot(fs::canonicalize(tmp.path()).unwrap());
        (tmp, root)
    }

    /// Write `bytes` to `rel` and force a distinguishable mtime — a test that
    /// rewrites a file inside one filesystem timestamp tick would be asserting
    /// the memo's blind spot, not its behaviour.
    fn rewrite(root: &WorkspaceRoot, rel: &str, bytes: &[u8]) {
        std::thread::sleep(std::time::Duration::from_millis(10));
        fs::write(root.0.join(rel), bytes).unwrap();
    }

    /// The whole point: the memo's root IS the byte-derived root, at every
    /// state of the tree. If these two ever disagreed the daemon would stamp
    /// answers with a token the commit guard refuses.
    #[test]
    fn the_memo_root_equals_the_byte_derived_root_through_every_change() {
        let (_tmp, root) = workspace(&[
            ("a.md", b"# A\n".as_slice()),
            ("notes/b.md", b"# B\n".as_slice()),
            ("notes/deep/c.md", b"# C\n".as_slice()),
        ]);
        let mut cache = DomainCache::new();

        let agrees = |cache: &mut DomainCache, at: &str| {
            let memo = cache.root(&root).unwrap();
            let bytes = domain_snapshot(&root).unwrap().1;
            assert_eq!(memo, bytes, "memo and byte fold disagree {at}");
            memo
        };

        let r0 = agrees(&mut cache, "at rest");

        rewrite(&root, "notes/b.md", b"# B changed\n");
        let r1 = agrees(&mut cache, "after a modify");
        assert_ne!(r0, r1, "a modified member moves the root");

        fs::write(root.0.join("notes/d.md"), b"# D\n").unwrap();
        let r2 = agrees(&mut cache, "after an add");
        assert_ne!(r1, r2, "an added member moves the root");

        fs::remove_file(root.0.join("notes/d.md")).unwrap();
        let r3 = agrees(&mut cache, "after a delete");
        assert_ne!(r2, r3, "a removed member moves the root");
        assert_eq!(r1, r3, "removing the addition restores the earlier root");

        fs::remove_file(root.0.join("notes/deep/c.md")).unwrap();
        agrees(&mut cache, "after emptying a subtree");
    }

    /// The cost claim, measured rather than timed: a currency pass over an
    /// unchanged corpus reads ZERO members, and one over a corpus with one
    /// changed member reads exactly that one.
    #[test]
    fn a_currency_pass_reads_only_what_moved() {
        let (_tmp, root) = workspace(&[
            ("a.md", b"# A\n".as_slice()),
            ("b.md", b"# B\n".as_slice()),
            ("c.md", b"# C\n".as_slice()),
        ]);
        let mut cache = DomainCache::new();

        cache.root(&root).unwrap();
        assert_eq!(
            cache.leaves_read(),
            3,
            "a cold memo reads every member once"
        );

        cache.root(&root).unwrap();
        cache.root(&root).unwrap();
        assert_eq!(
            cache.leaves_read(),
            3,
            "an unchanged corpus is re-folded without reading a single byte"
        );

        rewrite(&root, "b.md", b"# B changed\n");
        cache.root(&root).unwrap();
        assert_eq!(
            cache.leaves_read(),
            4,
            "one moved member costs one read, not a corpus"
        );
    }

    /// The listing memo's own cost claim, and its safety claim, in one test:
    /// an unchanged tree is not re-enumerated, and a directory that GAINED a
    /// member is — because that is what a directory's own timestamps record.
    ///
    /// The safety half is the one that matters. A listing memo that missed a
    /// new file would leave it out of the fold, and the daemon would serve a
    /// root describing a corpus that no longer exists.
    #[test]
    fn an_unchanged_directory_is_not_re_enumerated_but_a_grown_one_is() {
        let (_tmp, root) = workspace(&[
            ("a.md", b"# A\n".as_slice()),
            ("notes/b.md", b"# B\n".as_slice()),
            ("notes/deep/c.md", b"# C\n".as_slice()),
        ]);
        let mut cache = DomainCache::new();

        cache.root(&root).unwrap();
        assert_eq!(
            cache.listings(),
            3,
            "a cold memo enumerates every directory: root, notes, notes/deep"
        );

        let quiet = cache.root(&root).unwrap();
        assert_eq!(
            cache.listings(),
            3,
            "an unchanged tree is re-folded without a single read_dir"
        );

        // A new member moves its OWN directory's timestamps and no other's.
        rewrite(&root, "notes/new.md", b"# New\n");
        let grown = cache.root(&root).unwrap();
        assert_ne!(quiet, grown, "the new member is in the fold");
        assert_eq!(
            grown,
            domain_snapshot(&root).unwrap().1,
            "and the fold still agrees with the byte-derived root"
        );
        assert_eq!(
            cache.listings(),
            4,
            "exactly one directory was re-enumerated — the one that grew"
        );
    }

    /// A same-size rewrite is the case a size-only check would miss. It must
    /// move the root: this is the property the whole mechanism rests on.
    #[test]
    fn a_same_size_rewrite_still_moves_the_root() {
        let (_tmp, root) = workspace(&[("a.md", b"# AAA\n".as_slice())]);
        let mut cache = DomainCache::new();
        let before = cache.root(&root).unwrap();

        rewrite(&root, "a.md", b"# BBB\n");
        assert_eq!(
            fs::metadata(root.0.join("a.md")).unwrap().len(),
            6,
            "the rewrite is the same size"
        );

        let after = cache.root(&root).unwrap();
        assert_ne!(before, after, "a same-size rewrite is not invisible");
        assert_eq!(
            after,
            domain_snapshot(&root).unwrap().1,
            "and it agrees with the byte fold"
        );
    }

    /// The memo holds one generation and never outlives its corpus: a member
    /// that is deleted is dropped, so re-creating it with different content
    /// cannot be answered from the digest it used to have.
    #[test]
    fn a_deleted_member_is_dropped_not_remembered() {
        let (_tmp, root) = workspace(&[("a.md", b"# A\n".as_slice())]);
        let mut cache = DomainCache::new();
        let with_a = cache.root(&root).unwrap();

        fs::remove_file(root.0.join("a.md")).unwrap();
        let without_a = cache.root(&root).unwrap();
        assert_ne!(with_a, without_a, "the deletion is observed");

        rewrite(&root, "a.md", b"# A different\n");
        let reborn = cache.root(&root).unwrap();
        assert_ne!(reborn, with_a, "the reborn file is not the remembered one");
        assert_eq!(
            reborn,
            domain_snapshot(&root).unwrap().1,
            "and it agrees with the byte fold"
        );
    }
}

#[cfg(test)]
mod domain_cache_parallel_tests {
    //! Fuse-authored gates over the arm-A-derived parallel mechanism
    //! (`wave1/wall-clock-e33b553a` commits `f84c1912` + `c0d0c8ba`,
    //! re-derived onto this memo walk). arm-A shipped the mechanism with
    //! measurements and zero tests, so nothing could be transplanted: these
    //! gates assert the claims the port rests on — bit-identical folds, exact
    //! counters at fan-out width, and refusals that survive the fan-out —
    //! rather than inheriting them.

    use super::{DomainCache, WorkspaceRoot, domain_snapshot};
    use std::fs;

    fn write(root: &std::path::Path, rel: &str, contents: &str) {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, contents).unwrap();
    }

    /// A tree wide enough to put work on every worker: 3 top-level dirs × 4
    /// subdirs × 3 files, plus a dot-dir and a pruned dir that must never be
    /// scanned. The serial walk's own guarantees, asserted at fan-out width:
    /// the memo root equals the byte-derived root cold and warm, the listing
    /// count is exact (no directory scanned twice, none skipped), and a grown
    /// directory costs exactly one re-enumeration.
    #[test]
    fn parallel_walk_is_bit_identical_at_fan_out_width() {
        let tmp = tempfile::tempdir().unwrap();
        let root_path = tmp.path();
        write(root_path, "top.md", "# Top\n");
        for a in 0..3 {
            for b in 0..4 {
                for f in 0..3 {
                    write(
                        root_path,
                        &format!("d{a}/s{b}/f{f}.md"),
                        &format!("# {a}-{b}-{f}\n"),
                    );
                }
            }
        }
        write(root_path, ".hidden/x.md", "outside the domain\n");
        write(root_path, "assets/skip.md", "pruned\n");
        write(
            root_path,
            "mdfs_config.yaml",
            "version: 1\nignore:\n  - \"assets/**\"\n",
        );
        let root = WorkspaceRoot(fs::canonicalize(root_path).unwrap());

        let mut cache = DomainCache::new();
        let cold = cache.root(&root).unwrap();
        assert_eq!(
            cold,
            domain_snapshot(&root).unwrap().1,
            "cold parallel walk agrees with the byte-derived root"
        );
        // Scanned: root + d0..d2 + 12 subdirs = 16. The dot-dir and the
        // pruned dir are never entered, so they must not count.
        assert_eq!(
            cache.listings(),
            16,
            "every directory enumerated exactly once"
        );

        let warm = cache.root(&root).unwrap();
        assert_eq!(warm, cold, "warm parallel walk returns the same root");
        assert_eq!(
            cache.listings(),
            16,
            "an unchanged tree re-enumerates nothing"
        );

        std::thread::sleep(std::time::Duration::from_millis(10));
        write(root_path, "d1/s2/new.md", "# New\n");
        let grown = cache.root(&root).unwrap();
        assert_eq!(
            grown,
            domain_snapshot(&root).unwrap().1,
            "the grown tree still agrees with the byte-derived root"
        );
        assert_eq!(
            cache.listings(),
            17,
            "exactly one directory re-enumerated — the one that grew"
        );
    }

    /// A tree whose collection order can never equal its sorted order — a
    /// subdirectory's file sorts BETWEEN its parent's own files, and the walk
    /// always collects the parent's files before the subdirectory is scanned.
    /// Any ordering leak from the parallel walk into the fold therefore
    /// changes the root and reddens this equality; it cannot pass by luck.
    #[test]
    fn collection_order_is_invisible_to_the_fold() {
        let tmp = tempfile::tempdir().unwrap();
        let root_path = tmp.path();
        // Sorted: a/b/x.md < a/m.md — but a/m.md is collected first.
        write(root_path, "a/m.md", "# M\n");
        write(root_path, "a/b/x.md", "# X\n");
        write(root_path, "z/a.md", "# ZA\n");
        write(root_path, "z/y/q.md", "# Q\n");
        let root = WorkspaceRoot(fs::canonicalize(root_path).unwrap());

        let mut cache = DomainCache::new();
        assert_eq!(
            cache.root(&root).unwrap(),
            domain_snapshot(&root).unwrap().1,
            "collect order must be invisible to the fold"
        );
    }

    /// A subdirectory that cannot be scanned refuses the whole sweep — the
    /// first-error law survives the fan-out — and the refusal clears when the
    /// directory does.
    #[cfg(unix)]
    #[test]
    fn an_unreadable_directory_refuses_the_parallel_sweep() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let root_path = tmp.path();
        write(root_path, "ok/a.md", "# A\n");
        write(root_path, "sealed/b.md", "# B\n");
        let root = WorkspaceRoot(fs::canonicalize(root_path).unwrap());

        let sealed = root.0.join("sealed");
        fs::set_permissions(&sealed, fs::Permissions::from_mode(0o000)).unwrap();
        let mut cache = DomainCache::new();
        let refused = cache.root(&root);
        fs::set_permissions(&sealed, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(
            refused.is_err(),
            "an unscannable directory refuses the pass"
        );

        assert_eq!(
            cache.root(&root).unwrap(),
            domain_snapshot(&root).unwrap().1,
            "the pass recovers once the directory does"
        );
    }

    /// The chunked stat pass returns byte-identical rows to the serial loop —
    /// same members, same order, same identities — over a fixture with
    /// distinct per-member metadata, so a chunking or merge-order break
    /// cannot cancel out.
    #[test]
    fn chunked_stat_pass_matches_the_serial_loop() {
        let tmp = tempfile::tempdir().unwrap();
        let root_path = tmp.path();
        for i in 0..40 {
            write(
                root_path,
                &format!("m{i:02}.md"),
                &format!("# {}\n", "x".repeat(i + 1)),
            );
        }
        let root = WorkspaceRoot(fs::canonicalize(root_path).unwrap());
        let domain = super::domain::Domain::load(&root).unwrap();
        let rels = super::hash_domain(&root, &domain).unwrap();

        let serial = super::member_identities(&root.0, &rels, usize::MAX).unwrap();
        let parallel = super::member_identities(&root.0, &rels, 0).unwrap();
        assert_eq!(serial, parallel, "rows must be bit-identical, in order");
    }

    /// With TWO members vanished, the refusal names the sorted-FIRST one on
    /// both the serial and the chunked path: the merge walks chunks in spawn
    /// order, so the member named never depends on which worker finished
    /// first.
    #[test]
    fn the_chunked_refusal_names_the_sorted_first_vanished_member() {
        let tmp = tempfile::tempdir().unwrap();
        let root_path = tmp.path();
        for i in 0..50 {
            write(root_path, &format!("m{i:02}.md"), "# M\n");
        }
        let root = WorkspaceRoot(fs::canonicalize(root_path).unwrap());
        let domain = super::domain::Domain::load(&root).unwrap();
        let rels = super::hash_domain(&root, &domain).unwrap();

        fs::remove_file(root.0.join("m10.md")).unwrap();
        fs::remove_file(root.0.join("m40.md")).unwrap();

        for floor in [usize::MAX, 0] {
            let err = super::member_identities(&root.0, &rels, floor).unwrap_err();
            let msg = err.to_string();
            assert!(
                msg.contains("m10.md"),
                "the sorted-first vanished member is named (floor {floor}): {msg}"
            );
            assert!(
                !msg.contains("m40.md"),
                "the later vanished member is never named (floor {floor}): {msg}"
            );
        }
    }
}

#[cfg(test)]
mod read_digest_parallel_tests {
    //! Gates for the BYTE half of the git-class pass
    //! ([`super::read_and_digest_members`]) — production concurrency on the
    //! commit path (the fold every splice pays 3×), so serial parity, refusal
    //! order, and the floor boundary are asserted rather than inherited from
    //! the construction argument. The stat half's gates directly above are
    //! the template; these hold the same claims where the bytes are.

    use super::{PARALLEL_READ_FLOOR, WorkspaceRoot, read_and_digest_members};
    use std::fs;
    use std::path::PathBuf;

    fn corpus(n: usize) -> (tempfile::TempDir, WorkspaceRoot, Vec<PathBuf>) {
        let tmp = tempfile::tempdir().unwrap();
        let mut rels = Vec::with_capacity(n);
        for i in 0..n {
            let rel = PathBuf::from(format!("m{i:04}.md"));
            fs::write(
                tmp.path().join(&rel),
                format!("# {i}\n\n{}\n", "x".repeat(i % 97)),
            )
            .unwrap();
            rels.push(rel);
        }
        let root = WorkspaceRoot(fs::canonicalize(tmp.path()).unwrap());
        (tmp, root, rels)
    }

    /// Serial parity: the parallel sweep's rows are bit-identical to the
    /// serial path's, in member order — bytes and digests both. The member
    /// count is deliberately not a multiple of any worker count, so a chunk
    /// boundary sits mid-list.
    #[test]
    fn the_parallel_sweep_matches_the_serial_rows_exactly() {
        let (_tmp, root, rels) = corpus(97);
        let serial = read_and_digest_members(&root, &rels, usize::MAX).unwrap();
        let parallel = read_and_digest_members(&root, &rels, 0).unwrap();
        assert_eq!(
            serial, parallel,
            "rows must be bit-identical, in member order"
        );
    }

    /// With TWO members unreadable, the refusal names the FIRST one in member
    /// order on both paths: each chunk stops at its own first refusal and the
    /// merge walks chunks in spawn order, so the member named never depends
    /// on which worker finished first.
    #[test]
    fn the_first_refusal_in_member_order_wins() {
        let (_tmp, root, mut rels) = corpus(96);
        // Missing members refuse at fs::read — one early, one late, in
        // different chunks at every worker count.
        rels.insert(20, PathBuf::from("gone-early.md"));
        rels.push(PathBuf::from("gone-late.md"));

        for floor in [usize::MAX, 0] {
            let err = read_and_digest_members(&root, &rels, floor).unwrap_err();
            let msg = err.to_string();
            assert!(
                msg.contains("gone-early.md"),
                "the first unreadable member is named (floor {floor}): {msg}"
            );
            assert!(
                !msg.contains("gone-late.md"),
                "the later unreadable member is never named (floor {floor}): {msg}"
            );
        }
    }

    /// The floor boundary: `len == floor` engages the parallel path,
    /// `len < floor` stays serial, and the rows are identical either side —
    /// the boundary is a scheduling fact, never a content fact.
    #[test]
    fn the_floor_boundary_changes_nothing_about_the_rows() {
        let (_tmp, root, rels) = corpus(PARALLEL_READ_FLOOR);
        let reference = read_and_digest_members(&root, &rels, usize::MAX).unwrap();

        let at_floor = read_and_digest_members(&root, &rels, PARALLEL_READ_FLOOR).unwrap();
        assert_eq!(at_floor, reference, "len == floor: parallel, same rows");

        let below = &rels[..PARALLEL_READ_FLOOR - 1];
        let serial_below = read_and_digest_members(&root, below, PARALLEL_READ_FLOOR).unwrap();
        assert_eq!(
            serial_below,
            reference[..PARALLEL_READ_FLOOR - 1],
            "len < floor: serial, the same leading rows"
        );
    }
}

/// Design tests for [`workspace_relative`]: the one respell computation the
/// door teachings and the §2.1 receipt keys share.
#[cfg(test)]
mod workspace_relative_tests {
    use super::{WorkspaceRoot, workspace_relative};
    use std::fs;

    fn workspace() -> (tempfile::TempDir, WorkspaceRoot) {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("a/b")).unwrap();
        fs::write(tmp.path().join("a/b/page.md"), "# P\n").unwrap();
        let root = WorkspaceRoot(tmp.path().to_owned());
        (tmp, root)
    }

    #[test]
    fn every_inside_spelling_resolves_to_the_one_relative_form() {
        let (tmp, root) = workspace();
        let abs = tmp.path().join("a/b/page.md");
        for spelling in [
            "a/b/page.md".to_owned(),
            "./a/b/page.md".to_owned(),
            "a/../a/b/page.md".to_owned(),
            abs.to_str().unwrap().to_owned(),
        ] {
            assert_eq!(
                workspace_relative(&root, &spelling).as_deref(),
                Some("a/b/page.md"),
                "spelling {spelling:?}"
            );
        }
    }

    #[test]
    fn a_symlinked_spelling_collapses_onto_the_physical_page() {
        let (tmp, root) = workspace();
        std::os::unix::fs::symlink(tmp.path().join("a/b"), tmp.path().join("alias")).unwrap();
        assert_eq!(
            workspace_relative(&root, "alias/page.md").as_deref(),
            Some("a/b/page.md"),
            "two spellings of one file are one key"
        );
    }

    #[test]
    fn an_outside_path_has_no_spelling() {
        let (_tmp, root) = workspace();
        let other = tempfile::tempdir().unwrap();
        let outside = other.path().join("evil.md");
        fs::write(&outside, "# E\n").unwrap();
        assert_eq!(workspace_relative(&root, outside.to_str().unwrap()), None);
        assert_eq!(workspace_relative(&root, "../evil.md"), None);
    }

    #[test]
    fn the_root_itself_is_no_page_spelling() {
        let (tmp, root) = workspace();
        assert_eq!(
            workspace_relative(&root, tmp.path().to_str().unwrap()),
            None
        );
        assert_eq!(workspace_relative(&root, "."), None);
    }

    #[test]
    fn a_missing_leaf_resolves_through_its_parent() {
        let (_tmp, root) = workspace();
        assert_eq!(
            workspace_relative(&root, "a/b/unborn.md").as_deref(),
            Some("a/b/unborn.md"),
            "a not-yet-born inside file still gets its spelling"
        );
    }
}

/// The §6.2 hermetic trust-close gates (card stable-read-protocol; codex
/// gate 17). These live INSIDE the crate because the row under test — a
/// leaf-memo entry whose `StatKey` equals the file's current identity while
/// its digest is stale — is the same-quantum in-place edit: constructible on
/// a 1–2 s-quantum backend by racing the clock, unconstructible
/// deterministically on a nanosecond one. The harness implants the row
/// directly; production records ride `observe` only.
#[cfg(test)]
mod stable_trust_tests {
    use std::path::{Path, PathBuf};

    use super::{DomainCache, LeafSeen, StatKey, WorkspaceRoot, stable};

    fn workspace(files: &[(&str, &[u8])]) -> (tempfile::TempDir, WorkspaceRoot) {
        let dir = tempfile::tempdir().expect("tempdir");
        for (rel, bytes) in files {
            let abs = dir.path().join(rel);
            if let Some(parent) = abs.parent() {
                std::fs::create_dir_all(parent).expect("mkdir");
            }
            std::fs::write(abs, bytes).expect("write fixture");
        }
        let root = WorkspaceRoot(dir.path().to_path_buf());
        (dir, root)
    }

    /// Implant the hermetic row: the file's CURRENT on-disk identity paired
    /// with a stale digest, recorded under `seen`.
    fn implant(
        cache: &mut DomainCache,
        root: &WorkspaceRoot,
        rel: &str,
        stale: [u8; 32],
        seen: Option<stable::FsStamp>,
    ) {
        let key = StatKey::of_path(&root.0.join(rel))
            .expect("stat")
            .expect("fixture present");
        cache.leaves.insert(
            PathBuf::from(rel),
            LeafSeen {
                key,
                digest: stale,
                seen,
            },
        );
    }

    /// Codex gate 17, both directions. The hermetic row's key compare PASSES
    /// by construction — what decides is the watermark law alone:
    ///
    /// - recorded inside its own racy window (`seen` == the file's mtime,
    ///   the same-quantum shape) ⇒ the member is re-read despite the
    ///   matching key, and the served root folds the DISK bytes;
    /// - recorded comfortably clear of the window (`seen` far ahead of the
    ///   stamps) ⇒ the memo serves, proving the re-read above was the
    ///   watermark's refusal and not an always-read.
    #[test]
    fn statkey_equality_alone_never_passes_the_watermark() {
        const OLD: &[u8] = b"# v1 body bytes\n";
        const NEW: &[u8] = b"# v2 body bytes\n"; // same size, in-place shape
        let (_tmp, root) = workspace(&[("x.md", NEW), ("other.md", b"bystander\n")]);
        let stale = model::leaf_digest(OLD);

        // Racy arm: the record watermark sits AT the file's own stamps —
        // exactly what a same-quantum record looks like. The key matches
        // the disk byte-for-byte; the watermark refuses anyway.
        let mut cache = DomainCache::new();
        cache.root(&root).expect("calibrate + baseline");
        let (_, _, _, mtime, _) = StatKey::of_path(&root.0.join("x.md"))
            .expect("stat")
            .expect("present")
            .raw_parts();
        implant(&mut cache, &root, "x.md", stale, Some(mtime));
        let rereads_before = cache.watermark_rereads();
        let served = cache.root(&root).expect("observe");
        let (_, disk) = super::domain_snapshot(&root).expect("snapshot");
        assert_eq!(served, disk, "the racy row must be re-read from disk");
        assert!(
            cache.watermark_rereads() > rereads_before,
            "the re-read must be the watermark's own (key equality held)"
        );

        // Trusted arm: same stale row, recorded far clear of the racy
        // window — the memo SERVES it, so the arm above was decided by the
        // watermark, not by paranoia.
        let mut cache = DomainCache::new();
        cache.root(&root).expect("calibrate + baseline");
        let far_future = (mtime.0 + 1_000_000, mtime.1);
        implant(&mut cache, &root, "x.md", stale, Some(far_future));
        let served = cache.root(&root).expect("observe");
        assert_ne!(served, disk, "the trusted row must serve the memo digest");
        let mut oracle = DomainCache::new();
        oracle.root(&root).expect("observe");
        let expected = {
            // Fold the oracle's leaves with x.md's digest swapped for the
            // stale one — the root the memo-served pass must answer.
            let mut leaves = oracle.leaf_digests();
            leaves.insert(PathBuf::from("x.md"), stale);
            let refs: Vec<(&[u8], [u8; 32])> = leaves
                .iter()
                .map(|(rel, d)| (super::hash_name(rel), *d))
                .collect();
            model::merkle_root_of_leaves(
                &refs,
                super::domain::Domain::load(&root)
                    .expect("domain")
                    .version(),
            )
        };
        assert_eq!(served, expected);
    }

    /// A spoiled record (`seen: None`) never trusts, whatever its key — the
    /// deliberate-spoil half of §6.2 row 1 (the overlay's posture, and every
    /// suspect/fenced read's).
    #[test]
    fn a_spoiled_record_is_re_read_despite_a_matching_key() {
        const OLD: &[u8] = b"stale bytes\n";
        const NEW: &[u8] = b"fresh bytes\n";
        let (_tmp, root) = workspace(&[("x.md", NEW)]);
        let mut cache = DomainCache::new();
        cache.root(&root).expect("baseline");
        implant(&mut cache, &root, "x.md", model::leaf_digest(OLD), None);
        let served = cache.root(&root).expect("observe");
        let (_, disk) = super::domain_snapshot(&root).expect("snapshot");
        assert_eq!(served, disk, "a spoiled record must fail toward reading");
    }

    /// Gate 2 (card stable-read-protocol): fixture-induced event loss lands
    /// in the LOUD untrusted state, visibly — and a completed observation
    /// re-baselines past it, while a virgin cache and an unabsorbed loss
    /// both report untrusted.
    #[test]
    fn event_loss_lands_in_the_loud_untrusted_state() {
        let (_tmp, root) = workspace(&[("a.md", b"# A\n")]);
        let mut cache = DomainCache::new();
        assert!(
            matches!(
                cache.guard_currency(),
                stable::GuardCurrency::Untrusted { .. }
            ),
            "a virgin cache vouches for nothing"
        );

        cache.root(&root).expect("baseline");
        assert_eq!(cache.guard_currency(), stable::GuardCurrency::Trusted);

        // The fixture-induced loss: the watcher's handle reports overflow.
        cache.feed_gen().note_loss("fixture-induced overflow");
        let stable::GuardCurrency::Untrusted { reason } = cache.guard_currency() else {
            panic!("event loss must drop guard currency, visibly");
        };
        assert!(
            reason.contains("event loss"),
            "the untrusted state names its cause: {reason}"
        );

        // A completed full observation absorbs the loss (the pass IS the
        // rescan ladder's full-sweep floor).
        cache.root(&root).expect("re-baseline");
        assert_eq!(cache.guard_currency(), stable::GuardCurrency::Trusted);
    }

    /// Unknown capability: a workspace whose `.meridian/` cannot be probed
    /// serves — correctly — but trusts NO stat identity (every pass re-reads
    /// every member) and reports untrusted for the whole workspace open.
    #[cfg(unix)]
    #[test]
    fn an_unprobeable_backend_is_loud_and_never_trusts() {
        use std::os::unix::fs::PermissionsExt as _;
        let (_tmp, root) = workspace(&[("a.md", b"# A\n"), ("b.md", b"# B\n")]);
        // An unwritable pre-existing .meridian: the probe cannot create its
        // file, so calibration is Unavailable for this workspace open.
        let meridian = root.0.join(".meridian");
        std::fs::create_dir(&meridian).expect("mkdir");
        std::fs::set_permissions(&meridian, std::fs::Permissions::from_mode(0o555)).expect("chmod");

        let mut cache = DomainCache::new();
        let r1 = cache.root(&root).expect("serving continues");
        let reads_cold = cache.leaves_read();
        assert!(
            matches!(
                cache.calibration(),
                Some(stable::Calibration::Unavailable { .. })
            ),
            "the probe failure is recorded, not guessed around"
        );
        assert!(
            matches!(
                cache.guard_currency(),
                stable::GuardCurrency::Untrusted { .. }
            ),
            "unknown capability is untrusted for the whole open"
        );

        // Never silent trust: a quiet pass re-reads every member.
        let r2 = cache.root(&root).expect("quiet pass");
        assert_eq!(r1, r2);
        assert_eq!(
            cache.leaves_read(),
            reads_cold * 2,
            "an uncalibrated backend trusts no stat identity"
        );

        // Restore write access so the tempdir can clean up.
        std::fs::set_permissions(&meridian, std::fs::Permissions::from_mode(0o755))
            .expect("chmod back");
    }

    /// The dir-listing memo rides the same trust close: a listing recorded
    /// inside its racy window re-enumerates despite an unmoved directory
    /// key, so a same-quantum entry change can never hide behind the
    /// listing memo (the "bytes outside the fold" hazard).
    #[test]
    fn a_racy_dir_listing_is_re_enumerated() {
        let (_tmp, root) = workspace(&[("notes/a.md", b"# A\n")]);
        let mut cache = DomainCache::new();
        cache.root(&root).expect("baseline");
        let listings_warm = cache.listings();

        // Spoil the notes/ listing record: same key, seen at its own mtime.
        let key = StatKey::of_path(&root.0.join("notes"))
            .expect("stat")
            .expect("present");
        let seen = {
            let (_, _, _, mtime, _) = key.raw_parts();
            Some(mtime)
        };
        let prior = cache
            .dirs
            .get(Path::new("notes"))
            .expect("memoized")
            .clone();
        cache.dirs.insert(
            PathBuf::from("notes"),
            super::DirSeen {
                key,
                seen,
                entries: prior.entries,
            },
        );
        cache.root(&root).expect("observe");
        assert!(
            cache.listings() > listings_warm,
            "a racy listing record must re-enumerate"
        );
    }
}
