//! Exec-window detection bracket (run-plane U6b): residual-compare over the
//! §12 hash domain, the domain-config change bracket, and symlink
//! refusal in the guarded walk.
//!
//! # What this is
//! Bash task blocks are `detected`, not prevented (plan #16/#18): the run
//! layer brackets every bash exec window with a [`StepGuard`] —
//! [`StepGuard::open`] captures the config state and the pre-step domain
//! snapshot, [`StepGuard::close`] re-snapshots after the step and REFUSES
//! unless the post-step tree is byte-identical to pre-step files + that
//! step's governed edits. These primitives are exec-independent; the run
//! crate wires them around real bash windows.
//!
//! # Residual-compare, not root-compare (#19)
//! A naive "the root changed, but we changed it" check passes the
//! emit-one-honest-descriptor-and-write-elsewhere cheat. The guard instead
//! computes the EXPECTED post-step file set — pre-step files overlaid with
//! the step's [`GovernedEdit`]s, domain-filtered — and diffs the actual
//! snapshot against it path-by-path, byte-by-byte. Any residual (a file
//! present that should not be, absent that should be, or with different
//! bytes) refuses with the exact delta named.
//!
//! # Naming discipline (S4)
//! A residual delta is reported as an **"out-of-band change during exec
//! window"** — the guard names the window and the paths, never an author.
//! The delta may equally be a human edit racing the run; accusing the block
//! would be invented attribution. The wording lives in [`GuardError`]'s
//! `Display` so downstream reports inherit it.
//!
//! # Config bracket (#20)
//! The detection domain itself is config: a step that rewrites the domain
//! config could widen the ignore list so its next writes fall
//! outside detection. The guard captures the raw config bytes at open and
//! refuses at close — BEFORE the residual diff, so a widened domain never
//! filters it — if they changed. Byte equality subsumes the planned hash
//! compare.
//!
//! BOTH declaration surfaces are bracketed — `meridian/domain.md` and the
//! legacy `mdfs_config.yaml`. #20 is about the DOMAIN moving, not about one
//! filename: a bracket watching a single file while the workspace declares its
//! ignore list in the other would reintroduce exactly the laundering it exists
//! to stop. [`StepGuard::config_state`] + [`StepGuard::verify_config`] are
//! the SEAM for mid-run continuity: the run layer is expected to pin every
//! step to the config the run started under (surfaced there as
//! `ExecBracket::open_pinned`); this module only provides the predicate.
//!
//! # Symlinks (#25)
//! `ln -s <out-of-tree secret> notes/x.md` would make out-of-tree bytes
//! addressable while the plain [`crate::walk`] silently skips the link —
//! laundering that defeats in-domain detection, unlike a plain out-of-tree
//! write. The guarded walk REFUSES any symlink on a non-dot path (file or
//! directory, md or not), at open (no trustworthy baseline over links) and
//! at close (laundering during the window); on unix, file reads are
//! `O_NOFOLLOW` so a link racing the walk cannot be read through either.
//!
//! # Accepted gaps — named, not hidden
//! - **Non-md / `.meridian/` / dot-path writes are UNDETECTED** (#20): the
//!   §12 hash domain is md-only and dot-excluded, so the guard cannot see
//!   them. An explicit S1 gap, distinct from the out-of-tree honor system.
//! - **Dot-path symlinks** sit in the same gap: not walked, not refused.
//! - **Ignored directories are not walked and their links are not refused.**
//!   An ignored path is outside the detection domain by the workspace's own
//!   declaration, so a symlink there can make out-of-tree bytes readable at a
//!   path nothing attests. Two things keep this from being #25 reopened: the
//!   ignore list is itself bracketed (a step cannot widen it mid-window to
//!   manufacture the spot), and an ignored file never enters the hash domain,
//!   so nothing it points at can reach a hash, attest or receipt surface. What
//!   it does NOT buy is unreadability — a bash step could already read any
//!   out-of-tree file directly, with or without a link, because this bracket
//!   detects writes and has never sandboxed reads.
//!   Without this, one link anywhere in a vendored subtree refuses the whole
//!   walk and no bracket can open: field-notes carries 2,670 non-dot symlinks,
//!   2,669 of them under a single ignored asset store.
//! - **Residual escape window** (S3): background children are process-group
//!   killed at step end (U6a); a write landing between that kill and the
//!   close-snapshot read — or after close — is outside the bracket.
//!   Detection holds up to the bracket boundary, no further.

use std::collections::BTreeMap;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use crate::WorkspaceRoot;
use crate::domain::{CONFIG_FILE_NAME, DOMAIN_CONFIG_PATH, Domain};

/// The detection bracket around one exec window: pre-step domain snapshot +
/// captured config state, consumed by [`StepGuard::close`] (one bracket, one
/// verdict).
#[derive(Debug)]
#[must_use = "an unclosed guard detects nothing — close() renders the verdict"]
pub struct StepGuard {
    root: WorkspaceRoot,
    domain: Domain,
    config: ConfigState,
    pre: BTreeMap<String, Vec<u8>>,
}

/// The captured domain-config state: the raw bytes of BOTH declaration
/// surfaces, each present or absent. Byte equality is the #20 bracket
/// predicate — strictly stronger than a hash compare, and absent-vs-present is
/// a change like any other.
///
/// Both surfaces are captured because #20 is about the DOMAIN moving, not
/// about one filename. A bracket watching only `mdfs_config.yaml` while the
/// workspace declares its ignore list in [`DOMAIN_CONFIG_PATH`] would let a
/// step widen the domain mid-window and never notice — the exact laundering
/// #20 exists to stop, reintroduced by the surface change.
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

/// The residual (#19): how the post-step tree differs from pre-step files +
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

/// The S4 report wording, at its single source: the delta names the exec
/// WINDOW and the paths — never an author. Downstream reports (run crate,
/// receipts) render the delta through this impl so the discipline cannot
/// drift.
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
/// string keeps the S4 discipline — name the window, never an author.
#[derive(Debug)]
pub enum GuardError {
    /// Underlying I/O failure taking a snapshot — not a detection verdict.
    Io(io::Error),
    /// A symlinked path component in the guarded walk (#25): refused at open
    /// (untrusted baseline) and at close (laundering during the window).
    Symlink {
        /// Workspace-relative forward-slash path of the refused link.
        path: String,
    },
    /// `mdfs_config.yaml` changed inside the bracket (#20): the detection
    /// domain itself moved, so no residual verdict is possible.
    ConfigChanged,
    /// The residual delta (#19): the post-step tree is not pre-step files +
    /// governed edits.
    OutOfBand(ResidualDelta),
}

impl fmt::Display for GuardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GuardError::Io(e) => write!(f, "exec-window snapshot I/O failure: {e}"),
            GuardError::Symlink { path } => {
                write!(f, "symlinked path refused in exec-window snapshot: {path}")
            }
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
    /// never `hermetic`. The U7 labeler imports this constant (#23's gate:
    /// no `detected` label without detection actually landed).
    pub const GUARANTEE_CLASS: &'static str = "detected";

    /// Open the bracket: capture the config state and the pre-step domain
    /// snapshot through the guarded (symlink-refusing) walk.
    ///
    /// # Errors
    /// [`GuardError::Symlink`] on any symlinked non-dot path — a trustworthy
    /// baseline cannot be established over links; [`GuardError::Io`] on any
    /// read failure (a non-UTF-8 config included, refused as `InvalidData`).
    pub fn open(root: &WorkspaceRoot) -> Result<StepGuard, GuardError> {
        let config = read_config(root)?;
        let domain = config.parse_domain()?;
        let pre = strict_domain_files(root, &domain)?;
        Ok(StepGuard {
            root: root.clone(),
            domain,
            config,
            pre,
        })
    }

    /// The captured config state — the run layer pins step 1's state and
    /// checks every later guard against it via [`StepGuard::verify_config`].
    #[must_use]
    pub fn config_state(&self) -> &ConfigState {
        &self.config
    }

    /// The pre-step root as this guard OBSERVED it at open: the fold of the
    /// captured baseline. The run layer cross-checks this against the
    /// flock-computed `root_after_phase1` (#19 addendum — the COMPUTED root
    /// is the authority; a mismatch means the tree moved between the phase-1
    /// commit and the bracket opening, and the exec must not start).
    #[must_use]
    pub fn pre_root(&self) -> model::MerkleRoot {
        let refs: Vec<(&str, &[u8])> = self
            .pre
            .iter()
            .map(|(p, b)| (p.as_str(), b.as_slice()))
            .collect();
        model::merkle_root(&refs, self.domain.version())
    }

    /// Cross-step continuity (#20 is "mid-RUN", not just mid-step): compare
    /// this guard's captured config against the state an earlier step
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
    /// files plus `edits`. Order is load-bearing: the config bracket first (#20 — a
    /// widened domain must never filter the diff), then the guarded re-walk
    /// (#25), then the residual compare (#19). A clean close returns the
    /// verified post-step root (the fold of exactly the expected bytes).
    ///
    /// # Errors
    /// [`GuardError::ConfigChanged`] on a mid-bracket config change;
    /// [`GuardError::Symlink`] on a symlinked non-dot path;
    /// [`GuardError::OutOfBand`] naming the residual delta;
    /// [`GuardError::Io`] on snapshot read failure.
    pub fn close(self, edits: &[GovernedEdit]) -> Result<model::MerkleRoot, GuardError> {
        if read_config(&self.root)? != self.config {
            return Err(GuardError::ConfigChanged);
        }
        let actual = strict_domain_files(&self.root, &self.domain)?;
        let mut expected = self.pre;
        for edit in edits {
            if self.domain.contains(Path::new(&edit.path)) {
                expected.insert(edit.path.clone(), edit.bytes.clone());
            }
        }
        let delta = residual(&expected, &actual);
        if !delta.is_empty() {
            return Err(GuardError::OutOfBand(delta));
        }
        let refs: Vec<(&str, &[u8])> = actual
            .iter()
            .map(|(p, b)| (p.as_str(), b.as_slice()))
            .collect();
        Ok(model::merkle_root(&refs, self.domain.version()))
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

/// Read `mdfs_config.yaml` without following links: absent ⇒
/// `ConfigState(None)`; a symlinked config refuses (#25 covers the domain's
/// own definition file too).
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
    match read_nofollow(&root.0.join(rel), rel) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(GuardError::Io(e)) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// The guarded snapshot: [`walk_strict`] narrowed to the hash domain, each
/// file read via [`read_nofollow`]. Returned as a sorted path→bytes map (the
/// residual compare's working shape).
fn strict_domain_files(
    root: &WorkspaceRoot,
    domain: &Domain,
) -> Result<BTreeMap<String, Vec<u8>>, GuardError> {
    let mut files = BTreeMap::new();
    for rel in walk_strict(root, domain)? {
        if !domain.contains(&rel) {
            continue;
        }
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        let bytes = read_nofollow(&root.0.join(&rel), &rel_str)?;
        files.insert(rel_str, bytes);
    }
    Ok(files)
}

/// The guarded walk: like [`crate::walk`] but (a) REFUSES any symlink on a
/// non-dot path instead of silently skipping it (#25), and (b) does not
/// descend into dot-prefixed entries — those are structurally outside the
/// detection domain (the named dot-path gap), and refusing links there would
/// false-positive on `.git` internals.
fn walk_strict(root: &WorkspaceRoot, domain: &Domain) -> Result<Vec<PathBuf>, GuardError> {
    let mut out = Vec::new();
    walk_strict_dir(&root.0, &PathBuf::new(), domain, &mut out)?;
    out.sort();
    Ok(out)
}

fn walk_strict_dir(
    abs_dir: &Path,
    rel_dir: &Path,
    domain: &Domain,
    out: &mut Vec<PathBuf>,
) -> Result<(), GuardError> {
    for entry in std::fs::read_dir(abs_dir)? {
        let entry = entry?;
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue; // dot-path gap: outside detection, neither walked nor refused
        }
        let rel = rel_dir.join(&name);
        let file_type = entry.file_type()?;
        // An ignored directory is outside the detection domain by the
        // workspace's own declaration, so its contents are not walked and its
        // links are not refused — the same shape as the dot-path gap, and
        // named alongside it in the module's accepted-gaps list.
        //
        // Checked BEFORE the symlink refusal, which is the whole point: a
        // corpus that carries a vendored tree (extracted archives, asset
        // stores) is otherwise unrunnable, because ONE link anywhere refuses
        // the entire walk and `mrd run` can never open a bracket.
        if file_type.is_dir() && domain.prunes_dir(&rel) {
            continue;
        }
        if file_type.is_symlink() {
            return Err(GuardError::Symlink {
                path: rel.to_string_lossy().replace('\\', "/"),
            });
        }
        if file_type.is_dir() {
            walk_strict_dir(&entry.path(), &rel, domain, out)?;
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
fn read_nofollow(abs: &Path, rel: &str) -> Result<Vec<u8>, GuardError> {
    match open_nofollow(abs) {
        Ok(mut f) => {
            let mut buf = Vec::new();
            f.read_to_end(&mut buf)?;
            Ok(buf)
        }
        Err(e) => {
            if std::fs::symlink_metadata(abs).is_ok_and(|m| m.file_type().is_symlink()) {
                Err(GuardError::Symlink {
                    path: rel.to_string(),
                })
            } else {
                Err(GuardError::Io(e))
            }
        }
    }
}

/// `O_NOFOLLOW` open (unix): opening a symlink fails (`ELOOP`) instead of
/// reading through it.
#[cfg(unix)]
fn open_nofollow(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}

/// Off unix there is no `O_NOFOLLOW`; the guarded walk has already refused
/// symlinks, so only the walk→read race is uncovered here.
#[cfg(not(unix))]
fn open_nofollow(path: &Path) -> io::Result<File> {
    File::open(path)
}

/// The residual compare (#19): diff the actual post-step snapshot against the
/// expected set, path-by-path, byte-by-byte. Sorted output by construction
/// (both maps iterate in path order) — reports are deterministic.
fn residual(
    expected: &BTreeMap<String, Vec<u8>>,
    actual: &BTreeMap<String, Vec<u8>>,
) -> ResidualDelta {
    let mut delta = ResidualDelta::default();
    for (path, bytes) in expected {
        match actual.get(path) {
            None => delta.missing.push(path.clone()),
            Some(a) if a != bytes => delta.altered.push(path.clone()),
            Some(_) => {}
        }
    }
    for path in actual.keys() {
        if !expected.contains_key(path) {
            delta.unexpected.push(path.clone());
        }
    }
    delta
}
