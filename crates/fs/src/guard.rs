//! Exec-window detection bracket: residual-compare over the §12 hash domain,
//! the domain-config change bracket, and symlink refusal in the guarded walk.
//!
//! Bash task blocks are `detected`, not prevented: the run layer brackets every
//! bash exec window with a [`StepGuard`] — [`StepGuard::open`] captures the
//! config state and the pre-step domain snapshot, [`StepGuard::close`]
//! re-snapshots and refuses unless the post-step tree is byte-identical to
//! pre-step files + that step's governed edits. These primitives are
//! exec-independent; the run crate wires them around real bash windows.
//!
//! # Residual-compare, not root-compare
//! A "the root changed, but we changed it" check passes the
//! emit-one-honest-descriptor-and-write-elsewhere cheat. The guard instead
//! computes the expected post-step file set — pre-step files overlaid with the
//! step's [`GovernedEdit`]s, domain-filtered — and diffs the actual snapshot
//! against it path-by-path, byte-by-byte.
//!
//! # Naming discipline
//! A residual delta is reported as an "out-of-band change during exec window" —
//! the window and the paths, never an author, since the delta may equally be a
//! human edit racing the run. The wording lives in [`GuardError`]'s `Display`.
//!
//! # Config bracket
//! The detection domain itself is config: a step that rewrites the domain config
//! could widen the ignore list so its next writes fall outside detection. The
//! raw config bytes are captured at open and re-checked at close, before the
//! residual diff so a widened domain never filters it.
//!
//! Both declaration surfaces are bracketed — `meridian/domain.md` and the legacy
//! `mdfs_config.yaml` — because the concern is the domain moving, not one
//! filename. [`StepGuard::config_state`] + [`StepGuard::verify_config`] are the
//! seam for mid-run continuity; this module only provides the predicate.
//!
//! # Symlinks
//! `ln -s <out-of-tree secret> notes/x.md` would make out-of-tree bytes
//! addressable while the plain [`crate::walk`] silently skips the link. The
//! guarded walk refuses any symlink that APPEARED inside the bracket on a
//! non-dot, non-ignored path (file or directory, md or not) — laundering
//! during the window. A link that already existed at open is the WORLD's
//! shape, not this window's act: it is recorded at open and tolerated at
//! close (the no-guard posture, 2026-08-15 — pre-existing state is never
//! accused; one stranger's link must not close the run door for every
//! unrelated run). Soundness is the ignored-directory argument: links never
//! enter the hash domain (both walks exclude them), so nothing behind a
//! pre-existing link can reach a hash, attest or receipt surface. On unix,
//! file reads are `O_NOFOLLOW` so a link racing the walk cannot be read
//! through either, and a symlinked domain CONFIG or RESERVED path (the
//! armed-rules artifact, the attested marker) still refuses at open — the
//! bracket's own instruments must be vouched for, pre-existing or not.
//!
//! # Accepted gaps
//! - Non-md / `.meridian/` / dot-path writes are undetected: the §12 hash domain
//!   is md-only and dot-excluded.
//! - Dot-path symlinks sit in the same gap: not walked, not refused.
//! - A PRE-EXISTING symlink on a walked path is outside detection: recorded
//!   at open, subtracted at close, never hashed. Its in-window removal, or a
//!   change of what it points at, is undetected — the same class as any
//!   out-of-tree write (the honor-system row). Only a link that APPEARED
//!   inside the window refuses.
//! - Ignored directories are not walked and their links are not refused, and a
//!   symlink AT a custom-ignored path is skipped the same way
//!   ([`Domain::skips_symlink`] — the sessions-root shape, where the venv `bin`
//!   dir IS the link). Two things bound this: the ignore list is itself
//!   bracketed, and an ignored file never enters the hash domain, so nothing it
//!   points at can reach a hash, attest or receipt surface. Without the
//!   carve-out, one link anywhere in a vendored subtree would refuse the whole
//!   walk. Reserved paths never skip.
//! - Residual escape window: background children are process-group killed at
//!   step end; a write landing between that kill and the close-snapshot read —
//!   or after close — is outside the bracket.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use crate::WorkspaceRoot;
use crate::digestmemo::DigestMemo;
use crate::domain::{CONFIG_FILE_NAME, DOMAIN_CONFIG_PATH, Domain, RESERVED_PATHS};

/// The detection bracket around one exec window: pre-step domain snapshot +
/// captured config state, consumed by [`StepGuard::close`] (one bracket, one
/// verdict).
///
/// The snapshots hold `path → leaf digest`, not bytes ([`model::leaf_digest`]
/// is the one leaf law, so digest equality IS byte equality and the folds
/// are byte-identical to the old full-byte compare) — the residual verdict
/// is unchanged while the whole corpus no longer rides in memory twice per
/// window. Byte reads are served through a caller-held [`DigestMemo`] where
/// one is offered ([`StepGuard::open_memoized`]): an unmoved member costs a
/// stat, a moved one a guarded `O_NOFOLLOW` read.
#[derive(Debug)]
#[must_use = "an unclosed guard detects nothing — close() renders the verdict"]
pub struct StepGuard {
    root: WorkspaceRoot,
    domain: Domain,
    config: ConfigState,
    /// Keyed by raw name bytes (merkle-spec §4/§9): two names that would
    /// merge under a lossy decode stay two entries, so the residual compare
    /// cannot be blinded by a hostile name. Values are §12.2 leaf digests.
    pre: BTreeMap<Vec<u8>, [u8; 32]>,
    /// Symlinked paths (§9 display form) present at open: the world's
    /// pre-existing links, outside detection. The close subtracts them —
    /// only a link that APPEARED inside the window refuses.
    pre_links: BTreeSet<String>,
}

/// The captured domain-config state: the raw bytes of both declaration
/// surfaces, each present or absent. Byte equality is the bracket predicate,
/// and absent-vs-present is a change like any other. Both surfaces are captured
/// because a bracket watching only `mdfs_config.yaml` while the workspace
/// declares its ignore list in [`DOMAIN_CONFIG_PATH`] would let a step widen the
/// domain mid-window and never notice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigState {
    md: Option<Vec<u8>>,
    yaml: Option<Vec<u8>>,
}

/// One governed change the step is entitled to have made: the full post-edit
/// bytes of one workspace-relative file (create-or-replace; forward-slash
/// path, the [`crate::DomainFiles`] convention). Later edits to the same path
/// win, matching sequential apply order.
#[derive(Debug, Clone)]
pub struct GovernedEdit {
    /// Workspace-relative forward-slash path.
    pub path: String,
    /// The file's full expected post-step bytes.
    pub bytes: Vec<u8>,
}

/// The residual: how the post-step tree differs from pre-step files +
/// governed edits. Lists are sorted; a path appears in exactly one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResidualDelta {
    /// On disk, not in the expected set.
    pub unexpected: Vec<String>,
    /// In the expected set, absent on disk.
    pub missing: Vec<String>,
    /// Present in both, bytes differ.
    pub altered: Vec<String>,
}

impl ResidualDelta {
    fn is_empty(&self) -> bool {
        self.unexpected.is_empty() && self.missing.is_empty() && self.altered.is_empty()
    }
}

/// The report wording, at its single source: the delta names the exec window
/// and the paths, never an author. Downstream reports render through this impl
/// so the discipline cannot drift.
impl fmt::Display for ResidualDelta {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "out-of-band change during exec window —")?;
        let mut first = true;
        for (label, paths) in [
            ("unexpected", &self.unexpected),
            ("missing", &self.missing),
            ("altered", &self.altered),
        ] {
            if paths.is_empty() {
                continue;
            }
            let sep = if first { " " } else { "; " };
            write!(f, "{sep}{label}: {}", paths.join(", "))?;
            first = false;
        }
        Ok(())
    }
}

/// A refusal (or I/O failure) from the detection bracket. Refusals are typed
/// so the run layer maps them to distinct report states; every `Display`
/// string keeps the naming discipline — name the window, never an author.
#[derive(Debug)]
pub enum GuardError {
    /// Underlying I/O failure taking a snapshot — not a detection verdict.
    Io(io::Error),
    /// Symlinked paths that APPEARED inside the bracket (laundering during
    /// the window), a link racing a guarded read, or a symlinked domain
    /// config. Pre-existing corpus links are recorded at open and tolerated
    /// (module docs § Symlinks). The walk completes before refusing, so the
    /// refusal is a COUNT plus the first offender (sorted) — a claim a
    /// caller can size a cleanup or a missing domain shape by, never one
    /// mine per attempt.
    Symlink {
        /// How many offending symlinked paths the bracket met (≥ 1) — at
        /// close, only links that appeared inside the window count.
        count: usize,
        /// Workspace-relative forward-slash path of the first offender, in
        /// sorted order — deterministic whatever order the walk visits.
        first: String,
    },
    /// The domain config changed inside the bracket: the detection domain
    /// itself moved, so no residual verdict is possible.
    ConfigChanged,
    /// The residual delta: the post-step tree is not pre-step files +
    /// governed edits.
    OutOfBand(ResidualDelta),
}

impl fmt::Display for GuardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GuardError::Io(e) => write!(f, "exec-window snapshot I/O failure: {e}"),
            // One link keeps the established wording byte-identical; more
            // become a count plus the first offender.
            GuardError::Symlink { count: 1, first } => {
                write!(f, "symlinked path refused in exec-window snapshot: {first}")
            }
            GuardError::Symlink { count, first } => write!(
                f,
                "{count} symlinked paths refused in exec-window snapshot, first: {first}"
            ),
            GuardError::ConfigChanged => write!(
                f,
                "the domain config ({DOMAIN_CONFIG_PATH} or {CONFIG_FILE_NAME}) changed during exec window — the detection domain itself moved; refusing",
            ),
            GuardError::OutOfBand(delta) => delta.fmt(f),
        }
    }
}

impl std::error::Error for GuardError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            GuardError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for GuardError {
    fn from(e: io::Error) -> Self {
        GuardError::Io(e)
    }
}

impl StepGuard {
    /// The guarantee class this bracket earns a bash block: `detected` —
    /// never `hermetic`. The labeler imports this constant (no `detected`
    /// label without detection actually landed).
    pub const GUARANTEE_CLASS: &'static str = "detected";

    /// Would [`open`](Self::open) refuse this workspace right now?
    ///
    /// The walk half of `open` without the byte reads — same config, same
    /// domain, same guarded traversal, so it cannot drift from the predicate it
    /// answers for. It captures no baseline and returns no guard.
    ///
    /// A caller asks before committing: the bracket opens against the root the
    /// phase-1 receipt commit made, so by the time `open` can refuse, that
    /// receipt is on disk and the never-rollback rule keeps it there. This
    /// narrows the window rather than closing it — a link appearing between the
    /// probe and `open` still refuses after the commit.
    ///
    /// # Errors
    /// The same refusals [`open`](Self::open) would raise:
    /// [`GuardError::Symlink`] on a symlinked domain config (never on a
    /// pre-existing corpus link — those are outside detection), or
    /// [`GuardError::Io`].
    pub fn probe(root: &WorkspaceRoot) -> Result<(), GuardError> {
        let config = read_config(root)?;
        let domain = config.parse_domain()?;
        let (_members, links) = walk_strict(root, &domain)?;
        refuse_reserved_links(&links)?;
        Ok(())
    }

    /// Open the bracket: capture the config state and the pre-step domain
    /// snapshot through the guarded (symlink-refusing) walk.
    ///
    /// # Errors
    /// [`GuardError::Symlink`] on a symlinked domain config or a link racing
    /// the walk's reads — never on a pre-existing corpus link, which is
    /// recorded as outside detection; [`GuardError::Io`] on any read failure
    /// (a non-UTF-8 config included, refused as `InvalidData`).
    pub fn open(root: &WorkspaceRoot) -> Result<StepGuard, GuardError> {
        Self::open_memoized(root, &mut DigestMemo::new())
    }

    /// [`StepGuard::open`] with byte reads served through `memo`: an unmoved
    /// member costs one stat, a moved or unknown one a guarded read that is
    /// recorded back. The verdicts are identical to `open`'s — the memo only
    /// moves WHERE digests come from, never what they are (module docs of
    /// [`crate::digestmemo`] state the stat-evidence standing).
    ///
    /// # Errors
    /// Exactly [`StepGuard::open`]'s.
    pub fn open_memoized(
        root: &WorkspaceRoot,
        memo: &mut DigestMemo,
    ) -> Result<StepGuard, GuardError> {
        let config = read_config(root)?;
        let domain = config.parse_domain()?;
        let (pre, links) = strict_domain_digests(root, &domain, memo)?;
        refuse_reserved_links(&links)?;
        Ok(StepGuard {
            root: root.clone(),
            domain,
            config,
            pre,
            pre_links: links.into_iter().collect(),
        })
    }

    /// [`StepGuard::open`] with the observation served from a resident
    /// [`crate::DomainCache`] — the daemon door's instrument (card
    /// run-observation-unification): listings from the dir memo, digests from
    /// the leaf memo, moved members read through the same guarded `O_NOFOLLOW`
    /// law. The verdicts are identical to `open`'s — the cache only moves
    /// WHERE listings and digests come from, never what they are.
    ///
    /// # Errors
    /// Exactly [`StepGuard::open`]'s.
    pub fn open_cached(
        root: &WorkspaceRoot,
        cache: &mut crate::DomainCache,
    ) -> Result<StepGuard, GuardError> {
        let config = read_config(root)?;
        let domain = config.parse_domain()?;
        let (pre, links) = cache
            .observe(root, &domain, crate::ObserveLaw::Guarded)
            .map_err(observe_refusal)?;
        refuse_reserved_links(&links)?;
        Ok(StepGuard {
            root: root.clone(),
            domain,
            config,
            pre,
            pre_links: links.into_iter().collect(),
        })
    }

    /// The captured config state — the run layer pins step 1's state and
    /// checks every later guard against it via [`StepGuard::verify_config`].
    #[must_use]
    pub fn config_state(&self) -> &ConfigState {
        &self.config
    }

    /// The pre-step root as this guard observed it at open: the fold of the
    /// captured baseline. The run layer cross-checks this against the
    /// flock-computed `root_after_phase1` (the computed root is the
    /// authority; a divergence means the tree moved between the phase-1
    /// commit and the bracket opening — blameless by construction, reported
    /// by default and refused only under the caller's fatal opt-in).
    #[must_use]
    pub fn pre_root(&self) -> model::MerkleRoot {
        let refs: Vec<(&[u8], [u8; 32])> =
            self.pre.iter().map(|(p, d)| (p.as_slice(), *d)).collect();
        crate::served_root(&refs, self.domain.version())
    }

    /// Cross-step continuity (the bracket is mid-RUN, not just mid-step):
    /// compare this guard's captured config against the state an earlier step
    /// captured. A run refuses when the domain moved between steps even if
    /// each step's own bracket was clean.
    ///
    /// # Errors
    /// [`GuardError::ConfigChanged`] when the states differ.
    pub fn verify_config(&self, earlier: &ConfigState) -> Result<(), GuardError> {
        if self.config == *earlier {
            Ok(())
        } else {
            Err(GuardError::ConfigChanged)
        }
    }

    /// Close the bracket: verify the post-step tree is exactly pre-step
    /// files plus `edits`. Order is load-bearing: the config bracket first (a
    /// widened domain must never filter the diff), then the guarded re-walk,
    /// then the residual compare. A clean close returns the verified
    /// post-step root (the fold of exactly the expected bytes).
    ///
    /// # Errors
    /// [`GuardError::ConfigChanged`] on a mid-bracket config change;
    /// [`GuardError::Symlink`] on a symlinked non-dot path that APPEARED
    /// inside the window (pre-existing links are subtracted, not refused);
    /// [`GuardError::OutOfBand`] naming the residual delta;
    /// [`GuardError::Io`] on snapshot read failure.
    pub fn close(self, edits: &[GovernedEdit]) -> Result<model::MerkleRoot, GuardError> {
        self.close_memoized(edits, &mut DigestMemo::new())
    }

    /// [`StepGuard::close`] with byte reads served through `memo` — the same
    /// verdict discipline, one stat per unmoved member instead of one read.
    ///
    /// # Errors
    /// Exactly [`StepGuard::close`]'s.
    pub fn close_memoized(
        self,
        edits: &[GovernedEdit],
        memo: &mut DigestMemo,
    ) -> Result<model::MerkleRoot, GuardError> {
        if read_config(&self.root)? != self.config {
            return Err(GuardError::ConfigChanged);
        }
        let (actual, links) = strict_domain_digests(&self.root, &self.domain, memo)?;
        self.verdict(edits, &actual, links)
    }

    /// [`StepGuard::close`] with the observation served from a resident
    /// [`crate::DomainCache`] — the pair of [`StepGuard::open_cached`]. Same
    /// verdict discipline, same order (config bracket first, then the guarded
    /// observation, then the residual compare).
    ///
    /// # Errors
    /// Exactly [`StepGuard::close`]'s.
    pub fn close_cached(
        self,
        edits: &[GovernedEdit],
        cache: &mut crate::DomainCache,
    ) -> Result<model::MerkleRoot, GuardError> {
        if read_config(&self.root)? != self.config {
            return Err(GuardError::ConfigChanged);
        }
        let (actual, links) = cache
            .observe(&self.root, &self.domain, crate::ObserveLaw::Guarded)
            .map_err(observe_refusal)?;
        self.verdict(edits, &actual, links)
    }

    /// The close verdict, one owner for both observation sources: refuse any
    /// link that APPEARED inside the window (#25, delta-scoped), then overlay
    /// the governed edits onto the captured baseline, residual-compare, and
    /// fold the verified post-step root. Order above (config bracket before
    /// the observation) is the callers' to hold.
    fn verdict(
        self,
        edits: &[GovernedEdit],
        actual: &BTreeMap<Vec<u8>, [u8; 32]>,
        post_links: Vec<String>,
    ) -> Result<model::MerkleRoot, GuardError> {
        // Pre-existing links are the world's shape, outside detection
        // (module docs § Symlinks) — subtracted, never refused. `post_links`
        // arrives sorted, so the first survivor is the sorted first offender.
        let mut new_links: Vec<String> = post_links
            .into_iter()
            .filter(|link| !self.pre_links.contains(link))
            .collect();
        if !new_links.is_empty() {
            return Err(GuardError::Symlink {
                count: new_links.len(),
                first: new_links.remove(0),
            });
        }
        let mut expected = self.pre;
        for edit in edits {
            if self.domain.contains(Path::new(&edit.path)) {
                // A governed edit's path is a String (run-plane input, UTF-8);
                // its UTF-8 bytes are its raw name bytes — identity. The
                // expected value is the edit's own §12.2 leaf.
                expected.insert(
                    edit.path.clone().into_bytes(),
                    model::leaf_digest(&edit.bytes),
                );
            }
        }
        let delta = residual(&expected, actual);
        if !delta.is_empty() {
            return Err(GuardError::OutOfBand(delta));
        }
        let refs: Vec<(&[u8], [u8; 32])> = actual.iter().map(|(p, d)| (p.as_slice(), *d)).collect();
        Ok(crate::served_root(&refs, self.domain.version()))
    }
}

/// Reserved paths are engine substrate (the armed-rules artifact, the
/// attested marker): a symlink there is never the world's tolerable shape —
/// like the domain config, the bracket's own instrument must be vouched for.
/// Open and probe refuse them even pre-existing; an in-window appearance is
/// caught by the close's new-link check like any other link.
fn refuse_reserved_links(links: &[String]) -> Result<(), GuardError> {
    let mut reserved: Vec<&String> = links
        .iter()
        .filter(|link| RESERVED_PATHS.contains(&link.as_str()))
        .collect();
    if reserved.is_empty() {
        return Ok(());
    }
    Err(GuardError::Symlink {
        count: reserved.len(),
        first: reserved.remove(0).clone(),
    })
}

/// A cached observation's refusal in the guard's vocabulary: I/O stays I/O,
/// the symlink refusal keeps its count-plus-first shape — byte-identical
/// wording to the fresh strict walk's.
fn observe_refusal(refusal: crate::ObserveRefusal) -> GuardError {
    match refusal {
        crate::ObserveRefusal::Io(e) => GuardError::Io(e),
        crate::ObserveRefusal::Symlink { count, first } => GuardError::Symlink { count, first },
    }
}

impl ConfigState {
    /// Parse the captured bytes into a [`Domain`] (absent ⇒ default domain).
    /// Non-UTF-8 config is refused as `InvalidData` — the same refusal
    /// [`Domain::load`] makes.
    fn parse_domain(&self) -> Result<Domain, GuardError> {
        // Same precedence and same ambiguity refusal as `Domain::load`. If the
        // guard resolved a config differently from the read path, the bracket
        // would be detecting against a domain nothing else uses.
        match (&self.md, &self.yaml) {
            (Some(_), Some(_)) => Err(GuardError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "two domain configs are present: {DOMAIN_CONFIG_PATH} and {CONFIG_FILE_NAME}. \
                     The exec-window bracket cannot pick one without guessing which domain the \
                     step is detected against. Remedy: keep {DOMAIN_CONFIG_PATH} and delete \
                     {CONFIG_FILE_NAME}."
                ),
            ))),
            (Some(bytes), None) => Ok(Domain::from_markdown(config_text(bytes)?)),
            (None, Some(bytes)) => Ok(Domain::from_config(config_text(bytes)?)),
            (None, None) => Ok(Domain::new()),
        }
    }
}

/// A captured config's bytes as text, refusing non-UTF-8 rather than lossily
/// decoding a file that decides what is attested.
fn config_text(bytes: &[u8]) -> Result<&str, GuardError> {
    std::str::from_utf8(bytes).map_err(|e| {
        GuardError::Io(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("non-UTF-8 content refused: {e}"),
        ))
    })
}

/// Read the domain config without following links: absent ⇒
/// `ConfigState(None)`; a symlinked config refuses (the symlink refusal
/// covers the domain's own definition file too).
fn read_config(root: &WorkspaceRoot) -> Result<ConfigState, GuardError> {
    Ok(ConfigState {
        md: read_config_file(root, DOMAIN_CONFIG_PATH)?,
        yaml: read_config_file(root, CONFIG_FILE_NAME)?,
    })
}

/// One config surface's bytes, or `None` when it is absent. Read `O_NOFOLLOW`
/// like every other guarded read: a symlinked config is a domain declaration
/// whose bytes live somewhere the bracket cannot vouch for.
fn read_config_file(root: &WorkspaceRoot, rel: &str) -> Result<Option<Vec<u8>>, GuardError> {
    match read_nofollow(root, Path::new(rel), rel) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(GuardError::Io(e)) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// The guarded snapshot: [`walk_strict`] narrowed to the hash domain, each
/// member's §12.2 leaf digest served from `memo` when its stat identity is
/// unmoved, else read via [`read_nofollow`] (bytes dropped after hashing —
/// the corpus never rides in memory) and recorded back. Returned as a sorted
/// path→digest map (the residual compare's working shape) plus the walk's
/// symlink list (sorted §9 display paths) for the caller's pre/post compare.
///
/// Misses are read in parallel above [`crate::PARALLEL_READ_FLOOR`], the
/// order-preserving contiguous-chunk pattern of the domain snapshot's sweep:
/// the first refusal is the first failing member in sorted order, never
/// whichever worker lost a race.
fn strict_domain_digests(
    root: &WorkspaceRoot,
    domain: &Domain,
    memo: &mut DigestMemo,
) -> Result<crate::ObservedDomain, GuardError> {
    let (members, links) = walk_strict(root, domain)?;
    let rels: Vec<PathBuf> = members
        .into_iter()
        .filter(|rel| domain.contains(rel))
        .collect();
    let identities = crate::member_identities(&root.0, &rels, crate::PARALLEL_STAT_FLOOR)?;
    let mut files = BTreeMap::new();
    let mut misses: Vec<(PathBuf, crate::StatKey)> = Vec::new();
    for (rel, key) in identities {
        match memo.lookup(&rel, &key) {
            Some(digest) => {
                files.insert(crate::hash_name(&rel).to_vec(), digest);
            }
            None => misses.push((rel, key)),
        }
    }
    let digests = read_and_digest_nofollow(root, &misses)?;
    // Order-preserving by construction: digests[i] belongs to misses[i].
    for ((rel, key), (_, digest)) in misses.into_iter().zip(digests) {
        files.insert(crate::hash_name(&rel).to_vec(), digest);
        memo.record(rel, key, digest);
    }
    Ok((files, links))
}

/// One member's digest row: its rel path and §12.2 leaf digest.
type DigestRow = (PathBuf, [u8; 32]);

/// Read + digest the missed members through the guarded `O_NOFOLLOW` read,
/// parallel in order-preserving contiguous chunks at or above
/// [`crate::PARALLEL_READ_FLOOR`]. Bytes are hashed and dropped inside each
/// worker; the first refusal is the sorted-order first, matching the serial
/// loop byte for byte.
fn read_and_digest_nofollow(
    root: &WorkspaceRoot,
    misses: &[(PathBuf, crate::StatKey)],
) -> Result<Vec<DigestRow>, GuardError> {
    let digest_of = |(rel, _key): &(PathBuf, crate::StatKey)| -> Result<DigestRow, GuardError> {
        let bytes = read_nofollow(root, rel, &crate::display_name(crate::hash_name(rel)))?;
        Ok((rel.clone(), model::leaf_digest(&bytes)))
    };
    if misses.len() < crate::PARALLEL_READ_FLOOR {
        return misses.iter().map(digest_of).collect();
    }
    let workers = std::thread::available_parallelism().map_or(2, |n| n.get().clamp(2, 8));
    let chunk = misses.len().div_ceil(workers);
    let mut rows: Vec<Result<Vec<DigestRow>, GuardError>> = Vec::new();
    std::thread::scope(|scope| {
        let handles: Vec<_> = misses
            .chunks(chunk)
            .map(|c| scope.spawn(move || c.iter().map(&digest_of).collect()))
            .collect();
        for handle in handles {
            match handle.join() {
                Ok(chunk_rows) => rows.push(chunk_rows),
                Err(panic) => std::panic::resume_unwind(panic),
            }
        }
    });
    let mut out = Vec::with_capacity(misses.len());
    for chunk_rows in rows {
        out.extend(chunk_rows?);
    }
    Ok(out)
}

/// The guarded walk: like [`crate::walk`] but (a) COLLECTS every symlink on
/// a non-dot, non-ignored path instead of silently skipping it — the caller
/// judges the list (open records it as the world's shape; close refuses what
/// appeared inside the window) — and (b) does not descend into dot-prefixed
/// entries — those are structurally outside the detection domain (the named
/// dot-path gap), and collecting links there would false-positive on `.git`
/// internals. Both lists come back sorted — deterministic whatever order
/// `read_dir` served.
fn walk_strict(
    root: &WorkspaceRoot,
    domain: &Domain,
) -> Result<(Vec<PathBuf>, Vec<String>), GuardError> {
    let mut out = Vec::new();
    let mut links = Vec::new();
    walk_strict_dir(&root.0, &PathBuf::new(), domain, &mut out, &mut links)?;
    links.sort();
    out.sort();
    Ok((out, links))
}

fn walk_strict_dir(
    abs_dir: &Path,
    rel_dir: &Path,
    domain: &Domain,
    out: &mut Vec<PathBuf>,
    links: &mut Vec<String>,
) -> Result<(), GuardError> {
    for entry in std::fs::read_dir(abs_dir)? {
        let entry = entry?;
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue; // dot-path gap: outside detection, neither walked nor refused
        }
        let rel = rel_dir.join(&name);
        let file_type = entry.file_type()?;
        // An ignored directory is outside the detection domain, so it is
        // neither walked nor refused. Checked before the symlink refusal, or a
        // corpus carrying a vendored tree would be unrunnable: one link
        // anywhere refuses the whole walk and no bracket can open.
        if file_type.is_dir() && domain.prunes_dir(&rel) {
            continue;
        }
        if file_type.is_symlink() {
            // The same carve-out for the link ITSELF: a symlink at a path the
            // domain ignores (a venv `bin` dir, a scratch-named entry) is
            // outside detection — skipped, never descended, never refused.
            if domain.skips_symlink(&rel) {
                continue;
            }
            links.push(crate::display_name(crate::hash_name(&rel)));
            continue; // keep walking: collected for the caller's pre/post compare
        }
        if file_type.is_dir() {
            walk_strict_dir(&entry.path(), &rel, domain, out, links)?;
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

/// Read a file's bytes refusing to follow a symlink — even one created after
/// the walk classified this path (the walk→read race). A link surfaces as
/// [`GuardError::Symlink`], classified via `symlink_metadata` so the refusal
/// is typed, not an opaque errno.
fn read_nofollow(root: &WorkspaceRoot, rel: &Path, display: &str) -> Result<Vec<u8>, GuardError> {
    match open_nofollow(&root.0, rel) {
        Ok(mut f) => {
            let mut buf = Vec::new();
            f.read_to_end(&mut buf)?;
            Ok(buf)
        }
        Err(e) => {
            if std::fs::symlink_metadata(root.0.join(rel)).is_ok_and(|m| m.file_type().is_symlink())
            {
                // The walk→read race mints for the ONE path it caught.
                Err(GuardError::Symlink {
                    count: 1,
                    first: display.to_string(),
                })
            } else {
                Err(GuardError::Io(e))
            }
        }
    }
}

/// `O_NOFOLLOW` open from the root down — the crate-shared primitive
/// ([`crate::open_nofollow`]), re-spelled locally so the read sites here keep
/// their name.
fn open_nofollow(root: &Path, rel: &Path) -> io::Result<File> {
    crate::open_nofollow(root, rel)
}

/// The residual compare: diff the actual post-step snapshot against the
/// expected set, path-by-path, digest-by-digest ([`model::leaf_digest`] is
/// the one leaf law, so digest equality IS byte equality). Sorted output by
/// construction (both maps iterate in path order) — reports are
/// deterministic.
fn residual(
    expected: &BTreeMap<Vec<u8>, [u8; 32]>,
    actual: &BTreeMap<Vec<u8>, [u8; 32]>,
) -> ResidualDelta {
    // The compare runs on raw name bytes; the delta LISTS are report prose,
    // rendered through the §9 display law (identity for UTF-8 names,
    // injective escape otherwise — never a merge).
    let mut delta = ResidualDelta::default();
    for (path, digest) in expected {
        match actual.get(path) {
            None => delta.missing.push(crate::display_name(path)),
            Some(a) if a != digest => delta.altered.push(crate::display_name(path)),
            Some(_) => {}
        }
    }
    for path in actual.keys() {
        if !expected.contains_key(path) {
            delta.unexpected.push(crate::display_name(path));
        }
    }
    delta
}
