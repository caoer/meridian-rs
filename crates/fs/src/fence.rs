//! The mechanical downgrade fence — dormant B-03 mechanism.
//!
//! Standing law (ZT on the b03 card, 2026-08-15): B-03 is not a cutover
//! blocker. There are no old-binary users. The tombstone never activates.
//! No-return is the durable B-04 record. This module ships the mechanism
//! built and tested; [`activate`] has no production caller.
//!
//! The mechanism's own citations: `docs/node-rev-merkle-spec.md` §4.2.5 and
//! the authority contract §2.4 (frozen pin
//! `solve-codex-cross-process-authority.md`, sha256 `95f7c248…`).
//!
//! # The shape (what the tests prove)
//!
//! The live writer opens the lockfile with create/write semantics
//! ([`crate::WriteLock::acquire`]) and then flocks it. The fence replaces the
//! `write.lock` PATHNAME with a symlink whose target is a directory
//! (`.meridian/fence.tombstone.d`): `open(O_CREAT|O_WRONLY)` follows the
//! symlink, hits a directory, and fails (`EISDIR`) — before any flock, before
//! any staging. The serving write door still uses this pathname today, which
//! is why the fence stays unactivated: flipping it would refuse the current
//! writer too. rename(2) of the symlink over the lockfile is atomic within
//! `.meridian/`, which gives the (test-only) activation a single commit point.
//!
//! # Activation is dormant
//!
//! Nothing in this module runs on any read or write path. [`activate`] is
//! invoked only from tests. A cutover does not call it: the no-return record
//! is B-04, not this tombstone. The tombstone is irreversible by construction
//! — there is no deactivate — which is why activation is its own verb instead
//! of riding any door, and why it stays uncalled.
//!
//! # The durable order, and why the tests keep it
//!
//! The activation ladder ([`ActivationPhase`]) makes the tombstone directory
//! durable BEFORE the symlink is renamed over the lockfile. The order is not
//! style: the live open carries `O_CREAT`, so a DANGLING symlink at
//! `write.lock` would let a later `WriteLock::acquire` create the missing
//! target through the link and take the lock — the fence would be a hole
//! exactly when a test expected it to hold. Sequencing dir-durability before
//! the rename makes that state unreachable through any crash: after a crash,
//! either the rename is absent (fence inactive, activation idempotently
//! re-runnable) or the rename is present and its target directory is already
//! durable (fence active). The harness constructs the dangling state directly
//! as a counterexample cell.
//!
//! # The stated residual window
//!
//! [`activate`] holds the exclusive flock across installation, so any
//! concurrent `WriteLock::acquire` whose `flock()` lands during activation
//! refuses `WouldBlock` exactly as it would against a concurrent writer. The
//! residual: a caller that called `open()` before the rename, was descheduled
//! across the ENTIRE activation, and calls `flock()` only after the activation
//! holder's fd closes could still lock the orphaned inode. [`ActivatedFence`]
//! therefore keeps that fd open for the life of the process. This is the same
//! class as the contract's stated external-race residual (editors, git,
//! arbitrary bash) — stated, bounded, not silent.

use std::fs::{self, File};
use std::io;
use std::path::Path;

use crate::{WorkspaceRoot, WriteLock};

/// The tombstone directory name inside `.meridian/`, and the symlink's exact
/// relative target. Relative on purpose: a moved workspace keeps its fence.
pub const TOMBSTONE_DIR: &str = "fence.tombstone.d";

/// The marker file inside the tombstone directory. [`status`] requires it to
/// certify a fence as THIS mechanism rather than a stray directory.
pub const TOMBSTONE_MARKER: &str = "TOMBSTONE";

/// The marker's exact contents — written before activation commits, read back
/// by [`status`]. The text names the law so a human finding the directory
/// learns what fenced them and where the law lives.
pub const TOMBSTONE_TEXT: &str = "meridian hash-law cutover downgrade fence (B-03)\n\
     write.lock is retired at this workspace: the hash-law cutover passed its\n\
     no-return boundary and pre-cutover direct binaries must not commit here.\n\
     law: docs/node-rev-merkle-spec.md 4.2.5; authority contract 2.4.\n";

/// The staged symlink's name while activation is in flight. Staging is inert:
/// the legacy writer opens `write.lock`, never this name.
const STAGED_LINK: &str = "write.lock.fence-staged";

/// What [`status`] certifies about a workspace. Two states only — every
/// half-state or foreign state is a loud [`FenceStateError`], never a guess
/// (the same posture B-04 mandates for the cutover record).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FenceStatus {
    /// `write.lock` is absent or a regular file — the unfenced world. A
    /// staged-but-unrenamed symlink also reports this: staging commits nothing.
    NotInstalled,
    /// `write.lock` is a symlink to the durable tombstone directory with the
    /// marker present: the legacy open cannot succeed here.
    Active,
}

/// The activation ladder, in commit order. Each phase names the on-disk state
/// AFTER that step completes; a crash inside a step leaves the previous
/// phase's state or a prefix of the current one. [`activate_until`] constructs
/// each state exactly — the crash-phase harness drives
/// [`crate::WriteLock::acquire`] against every rung.
///
/// The commit point is [`ActivationPhase::Renamed`]: every phase before it
/// leaves the fence INACTIVE (the legacy writer is unaffected and activation
/// re-runs to completion); every phase at or after it leaves the fence ACTIVE.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ActivationPhase {
    /// `.meridian/fence.tombstone.d/` created, marker written and fsynced,
    /// tombstone dir fsynced. Not yet durable in `.meridian` itself.
    TombstoneBuilt,
    /// `.meridian` fsynced — the tombstone directory entry is durable. This
    /// MUST precede the rename (see the module doc's dangling-symlink hazard).
    MetaDirSynced,
    /// The staged symlink `write.lock.fence-staged → fence.tombstone.d`
    /// exists. Inert: nothing opens the staged name.
    Staged,
    /// THE COMMIT POINT: the symlink renamed over `write.lock`. The legacy
    /// open now fails. The rename itself may still be lost to a crash until
    /// the next phase makes it durable — both survivor states are legal rungs.
    Renamed,
    /// `.meridian` fsynced — the rename is durable. Activation is complete.
    ParentSynced,
}

/// A completed activation. Holds the legacy exclusive flock taken before the
/// rename (see the module doc's residual window) — a caller that invoked
/// [`activate`] keeps this alive for the rest of the process; dropping it
/// merely closes the fd on the orphaned lockfile inode.
#[derive(Debug)]
pub struct ActivatedFence {
    /// `None` when [`activate`] found the fence already active (idempotent
    /// re-run): the original inode is gone, there is nothing to hold.
    _legacy_lock: Option<WriteLock>,
}

/// The typed loud error for a `write.lock` state this module did not make and
/// will not guess about: a symlink elsewhere, a dangling symlink, a directory
/// sitting at the lock pathname, a tombstone without its marker. Wrapped in
/// [`io::Error`] (kind [`io::ErrorKind::InvalidData`]); [`is_fence_state_error`]
/// recovers it.
#[derive(Debug)]
pub struct FenceStateError {
    /// What was found, in words a human acts on.
    pub found: String,
}

impl std::fmt::Display for FenceStateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "unrecognized downgrade-fence state at .meridian/write.lock: {} — refusing to guess; \
             repair deliberately (spec 4.2.5, authority contract 2.4)",
            self.found
        )
    }
}

impl std::error::Error for FenceStateError {}

/// Is this I/O error the typed unrecognized-fence-state refusal?
#[must_use]
pub fn is_fence_state_error(e: &io::Error) -> bool {
    e.get_ref()
        .is_some_and(|inner| inner.downcast_ref::<FenceStateError>().is_some())
}

fn state_error(found: impl Into<String>) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        FenceStateError {
            found: found.into(),
        },
    )
}

/// Certify the fence state of `root`. Answers [`FenceStatus::NotInstalled`]
/// for every legal unfenced state (no `.meridian`, no lockfile, a regular
/// lockfile, staging leftovers) and [`FenceStatus::Active`] only for the exact
/// shape [`activate`] commits. Anything else errors loud ([`FenceStateError`])
/// — a fence check that guesses is worse than none.
///
/// # Errors
/// [`FenceStateError`] (via [`io::ErrorKind::InvalidData`]) on an
/// unrecognized state; any I/O failure reading the workspace.
pub fn status(root: &WorkspaceRoot) -> io::Result<FenceStatus> {
    let dir = root.0.join(".meridian");
    let lock_path = dir.join("write.lock");
    let meta = match fs::symlink_metadata(&lock_path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(FenceStatus::NotInstalled),
        Err(e) => return Err(e),
    };
    let ft = meta.file_type();
    if ft.is_file() {
        return Ok(FenceStatus::NotInstalled);
    }
    if ft.is_dir() {
        return Err(state_error(
            "a directory sits directly at the lock pathname",
        ));
    }
    if !ft.is_symlink() {
        return Err(state_error("neither file, directory, nor symlink"));
    }
    let target = fs::read_link(&lock_path)?;
    if target != Path::new(TOMBSTONE_DIR) {
        return Err(state_error(format!(
            "a symlink to {} (expected {TOMBSTONE_DIR})",
            target.display()
        )));
    }
    let tomb = dir.join(TOMBSTONE_DIR);
    match fs::symlink_metadata(&tomb) {
        Ok(m) if m.is_dir() => {}
        Ok(_) => {
            return Err(state_error(
                "the tombstone target exists but is not a directory",
            ));
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Err(state_error(
                "a DANGLING fence symlink — the tombstone directory is missing, and the legacy \
                 O_CREAT open would create through it",
            ));
        }
        Err(e) => return Err(e),
    }
    match fs::read(tomb.join(TOMBSTONE_MARKER)) {
        Ok(bytes) if bytes == TOMBSTONE_TEXT.as_bytes() => Ok(FenceStatus::Active),
        Ok(_) => Err(state_error(
            "the tombstone marker exists with foreign contents",
        )),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            Err(state_error("a tombstone directory without its marker"))
        }
        Err(e) => Err(e),
    }
}

/// Fsync a directory so its entries are durable — the same discipline (and
/// the same ruled fsync class) the splice path applies to destination
/// parents.
fn sync_dir(path: &Path) -> io::Result<()> {
    crate::honest_sync_path(path)
}

/// Run the activation ladder up to and including `until`, WITHOUT taking the
/// exclusive flock and WITHOUT completing the remaining rungs. Test-only:
/// each rung is a post-crash state — a crashed process holds no flock, so the
/// harness must not either. There is no production activation.
///
/// Idempotent per rung: re-running a prefix over its own leftovers is legal —
/// that is what crash recovery does.
///
/// # Errors
/// Any I/O failure; [`FenceStateError`] when the workspace holds a state this
/// mechanism did not make.
pub fn activate_until(root: &WorkspaceRoot, until: ActivationPhase) -> io::Result<()> {
    let dir = root.0.join(".meridian");
    fs::create_dir_all(&dir)?;
    let tomb = dir.join(TOMBSTONE_DIR);

    // TombstoneBuilt: directory + marker, both durable inside the directory.
    match fs::symlink_metadata(&tomb) {
        Ok(m) if m.is_dir() => {}
        Ok(_) => {
            return Err(state_error(
                "the tombstone pathname is occupied by a non-directory",
            ));
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => fs::create_dir(&tomb)?,
        Err(e) => return Err(e),
    }
    let marker = tomb.join(TOMBSTONE_MARKER);
    let mut f = File::create(&marker)?;
    io::Write::write_all(&mut f, TOMBSTONE_TEXT.as_bytes())?;
    crate::honest_sync(&f)?;
    sync_dir(&tomb)?;
    if until == ActivationPhase::TombstoneBuilt {
        return Ok(());
    }

    // MetaDirSynced: the tombstone directory entry is durable in .meridian —
    // the load-bearing barrier before any rename (module doc).
    sync_dir(&dir)?;
    if until == ActivationPhase::MetaDirSynced {
        return Ok(());
    }

    // Staged: the inert symlink beside the lockfile.
    let staged = dir.join(STAGED_LINK);
    match fs::symlink_metadata(&staged) {
        Ok(_) => fs::remove_file(&staged)?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    std::os::unix::fs::symlink(TOMBSTONE_DIR, &staged)?;
    if until == ActivationPhase::Staged {
        return Ok(());
    }

    // Renamed — THE COMMIT POINT.
    fs::rename(&staged, dir.join("write.lock"))?;
    if until == ActivationPhase::Renamed {
        return Ok(());
    }

    // ParentSynced: the rename is durable.
    sync_dir(&dir)
}

/// Install the downgrade fence on `root` — the irreversible tombstone.
///
/// Dormant: no production caller. Tests only. The hash-law cutover does not
/// invoke this; no-return is the B-04 record. Kept so the mechanism stays
/// proven. There is no deactivate.
///
/// Holds the exclusive flock across installation, so a concurrent
/// [`crate::WriteLock::acquire`] refuses `WouldBlock` rather than racing the
/// rename. Idempotent: an already-active fence returns `Ok` without touching
/// anything.
///
/// # Errors
/// [`io::ErrorKind::WouldBlock`] when a writer currently holds the lock
/// (retry after it exits); [`FenceStateError`] when `write.lock` holds a
/// state this mechanism did not make; any other I/O failure.
pub fn activate(root: &WorkspaceRoot) -> io::Result<ActivatedFence> {
    if status(root)? == FenceStatus::Active {
        return Ok(ActivatedFence { _legacy_lock: None });
    }
    let lock = WriteLock::acquire(root)?;
    activate_until(root, ActivationPhase::ParentSynced)?;
    Ok(ActivatedFence {
        _legacy_lock: Some(lock),
    })
}
