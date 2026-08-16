//! The workspace authority state machine — the authority half of
//! construction step 6 (merged-plan §4.7, RULED B: daemon-routed).
//!
//! Law: the amended authority contract
//! `solve-codex-cross-process-authority.md`, pin `95f7c248101e6ff6…` (§15
//! merge capsule). The merkle spec delegates the write plane's own law to
//! that contract (`docs/node-rev-merkle-spec.md` § write-plane note).
//!
//! What this module IS: one exclusive workspace authority lease on a stable
//! directory inode (§2.1); the takeover/recovery ORDER as a typestate —
//! lease → recovery EX → recovery work → current root → ready → EX released
//! → admissions open (§2.3); `LOCK_EX` on `.meridian/write.lock` demoted to
//! takeover/recovery only (a [`Takeover`] is this module's only
//! [`fs::WriteLock`] consumer, and a [`PublishPermit`] takes NO flock); and
//! the wedged-but-alive posture (§2.1.1): heartbeat progress, an external
//! non-fd-inheriting supervisor that turns a hang into process death, typed
//! `authority_unavailable{wedged, retry_after}` while takeover runs.
//!
//! What this module is NOT (the other halves of step 6, and the cutover):
//! IPC routing and the §2.1 authenticated handshake are the daemon face's,
//! wired at the cutover — nothing here opens a socket. Reservations, durable
//! intents, and the contiguous root chain are the parallel-commits half; the
//! recovery WORK between [`AuthorityLease::begin_takeover`] and
//! [`Takeover::ready`] is the caller's, this machine enforces only its
//! position in the order. The §2.4 downgrade fence is B-03's, activated at
//! the cutover. Until that cutover the live write door keeps its interim
//! flock law — nothing here rewires it.
//!
//! Contract law 11, claimed exactly: the lease excludes COOPERATING Meridian
//! publishers only. Editors, git, and arbitrary bash do not honor it and
//! remain the stated external-race residual (`crates/fs/src/lib.rs`, the
//! `WriteLock` residual note). No API here says "fenced" about them.
//!
//! §15 capsule conformance rows for this half are recorded in the session
//! deliverable (`results/authority-lease-conformance.md`) against the tests
//! in this file and `tests/authority_process.rs`.

use std::fs::File;
use std::io;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// The authority-lock inode: a stable directory under `.meridian/`, flocked
/// by fd (§2.1). A directory because its inode is never swapped by a
/// tmp+rename — the `crates/cache` drawer-lock precedent.
pub const AUTHORITY_DIR: &str = ".meridian/authority";

/// The owner record beside the lock — a ROUTING HINT only (§2.1): it never
/// proves authority; the kernel lock does.
const OWNER_RECORD: &str = "owner.json";

/// The supervisor's wedged declaration + diagnostic (§2.1.1). Hint-grade,
/// like the owner record; removed when a successor reaches ready.
const WEDGED_MARKER: &str = "wedged.json";

/// The heartbeat progress file the ready authority advances and the external
/// supervisor observes. Content-compared, never mtime-compared, so
/// filesystem timestamp granularity cannot fake a hang.
const PROGRESS_FILE: &str = "progress";

// ── identities (§2.2) ──────────────────────────────────────────────────────

/// The ephemeral authority epoch — minted new for each authority lifetime,
/// never persisted across one (§2.2). It is what lets replies, reservations,
/// and intent ownership from a dead process be rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthorityEpoch(pub u128);

impl AuthorityEpoch {
    /// Mint a fresh epoch from kernel entropy. Entropy, not a clock: a crash
    /// loop inside one clock tick must still mint distinct lifetimes.
    fn mint() -> io::Result<Self> {
        let mut bytes = [0u8; 16];
        // SAFETY: getentropy fills exactly `bytes.len()` (≤ 256) bytes of a
        // valid, owned buffer; the buffer outlives the call.
        if unsafe { libc::getentropy(bytes.as_mut_ptr().cast(), bytes.len()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(u128::from_le_bytes(bytes)))
    }

    /// The canonical hex spelling carried by records.
    #[must_use]
    pub fn hex(&self) -> String {
        format!("{:032x}", self.0)
    }
}

/// The owner record's parsed hint fields. A stale socket or owner record
/// never proves authority (§2.1) — this type exists for refusal teaching and
/// supervisor routing, and for nothing load-bearing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerRecord {
    /// The holder's pid at acquisition.
    pub pid: i32,
    /// The holder's authority epoch, hex.
    pub epoch_hex: String,
    /// The canonical workspace the lease was taken on.
    pub workspace: String,
}

impl OwnerRecord {
    fn to_json(&self) -> String {
        serde_json::json!({
            "pid": self.pid,
            "epoch": self.epoch_hex,
            "workspace": self.workspace,
        })
        .to_string()
    }

    /// Parse a record; `None` on any deviation — a hint that cannot parse is
    /// no hint, never a refusal of its own.
    fn from_json(raw: &str) -> Option<Self> {
        let v: serde_json::Value = serde_json::from_str(raw).ok()?;
        Some(Self {
            pid: i32::try_from(v.get("pid")?.as_i64()?).ok()?,
            epoch_hex: v.get("epoch")?.as_str()?.to_string(),
            workspace: v.get("workspace")?.as_str()?.to_string(),
        })
    }
}

// ── typed refusals ─────────────────────────────────────────────────────────

/// Why authority could not be acquired — the `authority_unavailable` class
/// (§11 row 1, §2.1.1). A client holding this NEVER writes directly (§3.4);
/// there is no in-process fallback arm to reach for.
#[derive(Debug)]
pub enum AuthorityUnavailable {
    /// The lease is held by a live authority. `hint` is the owner record —
    /// routing teaching only, never proof.
    Held {
        /// The owner record beside the lock, when one parsed.
        hint: Option<OwnerRecord>,
    },
    /// The supervisor declared the holder wedged; takeover is in progress
    /// (§2.1.1). Retry after the report's budget.
    Wedged {
        /// The takeover-window hint carried by the report.
        retry_after: Duration,
        /// The supervisor's declaration, verbatim.
        report: WedgedReport,
    },
    /// The lock inode could not be created, opened, or locked for a reason
    /// other than contention.
    Io(io::Error),
}

impl std::fmt::Display for AuthorityUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthorityUnavailable::Held { hint } => {
                write!(
                    f,
                    "authority_unavailable: the workspace authority lease is held"
                )?;
                match hint {
                    Some(h) => write!(f, " (owner hint: pid {}, epoch {})", h.pid, h.epoch_hex),
                    None => write!(f, " (no readable owner hint)"),
                }
            }
            AuthorityUnavailable::Wedged { retry_after, .. } => write!(
                f,
                "authority_unavailable{{wedged}}: the holder was declared wedged and takeover \
                 is in progress; retry_after {}ms",
                retry_after.as_millis()
            ),
            AuthorityUnavailable::Io(e) => write!(f, "authority lock inode: {e}"),
        }
    }
}

impl std::error::Error for AuthorityUnavailable {}

/// Why a takeover could not complete its recovery-EX step.
#[derive(Debug)]
pub enum TakeoverError {
    /// Recovery EX was still held past the deadline. Under the interim
    /// coexistence law a cooperating direct writer may hold `write.lock`
    /// briefly; past the deadline the takeover refuses loudly rather than
    /// waiting forever.
    RecoveryExTimeout {
        /// How long the takeover waited.
        waited: Duration,
    },
    /// Acquiring recovery EX failed for a non-contention reason.
    Io(io::Error),
}

impl std::fmt::Display for TakeoverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TakeoverError::RecoveryExTimeout { waited } => write!(
                f,
                "recovery EX still held after {}ms — takeover refuses rather than waits forever",
                waited.as_millis()
            ),
            TakeoverError::Io(e) => write!(f, "recovery EX: {e}"),
        }
    }
}

impl std::error::Error for TakeoverError {}

// ── the lease (§2.1) ───────────────────────────────────────────────────────

/// The exclusive workspace authority lease: `flock(LOCK_EX | LOCK_NB)` on
/// the stable [`AUTHORITY_DIR`] directory fd, held for the entire time this
/// process may admit, recover, or answer writes (§2.1).
///
/// Kernel auto-release on process death is the janitor (§0.2): `kill -9`
/// drops the lease, and a successor's [`AuthorityLease::acquire`] is the
/// kernel-confirmed release check — no owner-record scavenging.
///
/// Holding a lease does NOT open admissions: a fresh lease can only
/// [`AuthorityLease::begin_takeover`], and only [`Takeover::ready`] mints
/// the state that publishes. Write admissions are closed by construction
/// between lease acquisition and ready (§2.3 steps 1–2).
///
/// The lease excludes cooperating Meridian publishers only (law 11).
#[derive(Debug)]
pub struct AuthorityLease {
    // Held open for its fd; released by the explicit `flock(LOCK_UN)` in
    // Drop — a concurrent fork's fd copy would otherwise keep a finished
    // lease held (`FD_CLOEXEC` acts at exec, not fork; §0.2).
    dir: File,
    epoch: AuthorityEpoch,
    root: fs::WorkspaceRoot,
}

/// Explicit unlock before the fd closes, exactly as [`fs::WriteLock`] and
/// the cache drawer lock: `LOCK_UN` acts on the open file description, so
/// one unlock releases the lease no matter how many forked fd copies exist.
/// Mandated by the contract for both authority fds (§0.2).
impl Drop for AuthorityLease {
    fn drop(&mut self) {
        // SAFETY: flock on a valid open fd we own; the fd outlives the call.
        unsafe { libc::flock(self.dir.as_raw_fd(), libc::LOCK_UN) };
    }
}

impl AuthorityLease {
    /// Try to become the workspace authority: create the stable lock inode
    /// on first use, take the exclusive flock without blocking, mint a fresh
    /// epoch, and land the owner routing hint.
    ///
    /// A refusal is TYPED, never a wait: [`AuthorityUnavailable::Wedged`]
    /// when a supervisor's declaration stands beside the held lock,
    /// [`AuthorityUnavailable::Held`] otherwise.
    ///
    /// # Errors
    /// [`AuthorityUnavailable`] per its variants.
    pub fn acquire(root: &fs::WorkspaceRoot) -> Result<Self, AuthorityUnavailable> {
        let dir_path = root.0.join(AUTHORITY_DIR);
        std::fs::create_dir_all(&dir_path).map_err(AuthorityUnavailable::Io)?;
        let dir = File::open(&dir_path).map_err(AuthorityUnavailable::Io)?;
        // SAFETY: flock on a valid open fd we own; the fd outlives the call.
        if unsafe { libc::flock(dir.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::WouldBlock {
                return Err(refusal(&dir_path));
            }
            return Err(AuthorityUnavailable::Io(e));
        }
        let epoch = AuthorityEpoch::mint().map_err(AuthorityUnavailable::Io)?;
        let lease = Self {
            dir,
            epoch,
            root: root.clone(),
        };
        lease
            .write_owner_record()
            .map_err(AuthorityUnavailable::Io)?;
        Ok(lease)
    }

    /// This lifetime's epoch (§2.2).
    #[must_use]
    pub fn epoch(&self) -> AuthorityEpoch {
        self.epoch
    }

    /// The workspace the lease is held on — taken from the lease so a lease
    /// and a root cannot be paired wrongly (the [`fs::WriteLock`] pattern).
    #[must_use]
    pub fn root(&self) -> &fs::WorkspaceRoot {
        &self.root
    }

    /// The owner routing hint beside a workspace's lock inode, or `None`
    /// when absent or unreadable. Never proof of anything (§2.1).
    #[must_use]
    pub fn owner_hint(root: &fs::WorkspaceRoot) -> Option<OwnerRecord> {
        let raw = std::fs::read_to_string(root.0.join(AUTHORITY_DIR).join(OWNER_RECORD)).ok()?;
        OwnerRecord::from_json(&raw)
    }

    /// Step 3 of the takeover interval (§2.3): acquire recovery EX — the
    /// demoted `.meridian/write.lock`, this module's ONLY flock on any
    /// publication-adjacent path. Bounded wait: under the interim law a
    /// cooperating direct writer may hold it briefly, so the `LOCK_NB`
    /// acquire retries until `deadline`, then refuses loudly.
    ///
    /// Consumes the lease: between lease and ready there is no state that
    /// can publish, which IS "write admissions closed" (§2.3 step 2).
    ///
    /// # Errors
    /// [`TakeoverError::RecoveryExTimeout`] past the deadline; any
    /// non-contention I/O failure as [`TakeoverError::Io`].
    pub fn begin_takeover(self, deadline: Duration) -> Result<Takeover, TakeoverError> {
        let started = Instant::now();
        loop {
            match fs::WriteLock::acquire(&self.root) {
                Ok(recovery) => {
                    return Ok(Takeover {
                        lease: self,
                        recovery,
                    });
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    if started.elapsed() >= deadline {
                        return Err(TakeoverError::RecoveryExTimeout {
                            waited: started.elapsed(),
                        });
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) => return Err(TakeoverError::Io(e)),
            }
        }
    }

    fn authority_dir(&self) -> PathBuf {
        self.root.0.join(AUTHORITY_DIR)
    }

    /// Land the owner routing hint, tmp+rename so a reader never sees a
    /// torn record. Overwrites any stale predecessor's record — staleness
    /// is expected after `kill -9`, and harmless because the record proves
    /// nothing.
    fn write_owner_record(&self) -> io::Result<()> {
        let record = OwnerRecord {
            // SAFETY: getpid never fails and has no preconditions.
            pid: unsafe { libc::getpid() },
            epoch_hex: self.epoch.hex(),
            workspace: self.root.0.to_string_lossy().into_owned(),
        };
        write_atomic(
            &self.authority_dir(),
            OWNER_RECORD,
            record.to_json().as_bytes(),
        )
    }
}

/// Type the flock refusal from the hints beside the lock: a standing wedged
/// declaration types it `Wedged`; otherwise `Held` with whatever owner hint
/// parses. Corrupt hint files degrade the teaching, never the refusal.
fn refusal(dir_path: &Path) -> AuthorityUnavailable {
    if let Ok(raw) = std::fs::read_to_string(dir_path.join(WEDGED_MARKER))
        && let Some(report) = WedgedReport::from_json(&raw)
    {
        return AuthorityUnavailable::Wedged {
            retry_after: report.retry_after,
            report,
        };
    }
    let hint = std::fs::read_to_string(dir_path.join(OWNER_RECORD))
        .ok()
        .and_then(|raw| OwnerRecord::from_json(&raw));
    AuthorityUnavailable::Held { hint }
}

/// Write `bytes` at `dir/name` atomically (same-directory tmp + rename).
fn write_atomic(dir: &Path, name: &str, bytes: &[u8]) -> io::Result<()> {
    let tmp = dir.join(format!("{name}.tmp"));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, dir.join(name))
}

// ── the takeover window (§2.3 steps 2–7) ───────────────────────────────────

/// The takeover/recovery window: authority lease held, recovery EX held,
/// admissions closed. The recovery WORK — classify and recover durable
/// intents, validate the restart checkpoint, bring the feed through a
/// barrier (§2.3 steps 4–5) — is the caller's, performed while this value
/// lives; those subsystems are the parallel-commits and checkpoint halves
/// of step 6, not this machine's. What this type enforces is the ORDER:
/// no recovery EX without the lease (the only constructor consumes an
/// [`AuthorityLease`]), no publication until [`Takeover::ready`].
#[derive(Debug)]
pub struct Takeover {
    lease: AuthorityLease,
    recovery: fs::WriteLock,
}

impl Takeover {
    /// The recovery fence — the demoted `LOCK_EX`, alive only inside this
    /// window. Callers hand it to intent recovery when that half lands.
    #[must_use]
    pub fn recovery_ex(&self) -> &fs::WriteLock {
        &self.recovery
    }

    /// This lifetime's epoch (§2.2).
    #[must_use]
    pub fn epoch(&self) -> AuthorityEpoch {
        self.lease.epoch
    }

    /// Steps 6–8 of the interval (§2.3): record the current root the
    /// caller's recovery established, explicitly release recovery EX, close
    /// the wedged episode if one stood, and open admissions — in exactly
    /// that order. The returned state is the only permit minter.
    ///
    /// # Errors
    /// Removing a standing wedged declaration failed. Recovery EX is
    /// already released when this refuses; re-run `ready` on the same
    /// arguments is not possible (the window is consumed), so the caller
    /// escalates — a marker that cannot be removed is a workspace-health
    /// fault, not a serving state.
    pub fn ready(self, current_root: wire::Root) -> io::Result<ReadyAuthority> {
        let Takeover { lease, recovery } = self;
        // (7) recovery EX explicitly unlocked — WriteLock's Drop is the
        // explicit LOCK_UN — BEFORE admissions open.
        drop(recovery);
        // The wedged episode ends only now: the marker stood through the
        // takeover so refusals stayed `wedged{retry_after}` while recovery
        // ran (§2.1.1 "while takeover is in progress").
        match std::fs::remove_file(lease.authority_dir().join(WEDGED_MARKER)) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        // (8) admissions open.
        Ok(ReadyAuthority {
            lease,
            root_at_ready: current_root,
            beats: 0,
        })
    }
}

// ── the ready authority (§2.3 step 8 onward) ───────────────────────────────

/// A ready workspace authority: lease held, recovery EX RELEASED, admissions
/// open. Ordinary publications take no flock — [`ReadyAuthority::permit`]
/// performs zero lock acquisitions, which is the `LOCK_EX` demotion stated
/// as a type: while this value serves, `.meridian/write.lock` is free.
#[derive(Debug)]
pub struct ReadyAuthority {
    lease: AuthorityLease,
    root_at_ready: wire::Root,
    beats: u64,
}

impl ReadyAuthority {
    /// Mint a publication permit. No flock, no syscall — proof of authority
    /// is the living lease this permit borrows, and the epoch it carries is
    /// what publications and (later) reservations bind to.
    #[must_use]
    pub fn permit(&self) -> PublishPermit<'_> {
        PublishPermit {
            epoch: self.lease.epoch,
            _lease: std::marker::PhantomData,
        }
    }

    /// This lifetime's epoch (§2.2).
    #[must_use]
    pub fn epoch(&self) -> AuthorityEpoch {
        self.lease.epoch
    }

    /// The current root recovery established at ready (§2.3 step 6).
    #[must_use]
    pub fn root_at_ready(&self) -> &wire::Root {
        &self.root_at_ready
    }

    /// The workspace this authority serves.
    #[must_use]
    pub fn root(&self) -> &fs::WorkspaceRoot {
        self.lease.root()
    }

    /// Advance the out-of-process heartbeat (§2.1.1): the event loop calls
    /// this at least once per [`HeartbeatPolicy::beat`] while ready. The
    /// progress file is content-advanced and lands by tmp+rename, so the
    /// supervisor never reads a torn beat and never depends on timestamp
    /// granularity.
    ///
    /// # Errors
    /// Any I/O failure landing the progress file — a holder that cannot
    /// advance progress WILL be declared wedged, so the caller should treat
    /// this as loud, not advisory.
    pub fn beat(&mut self) -> io::Result<()> {
        self.beats += 1;
        write_atomic(
            &self.lease.authority_dir(),
            PROGRESS_FILE,
            self.beats.to_string().as_bytes(),
        )
    }
}

/// Proof that one publication was admitted by the live authority. Carries
/// the epoch; holds NO lock — the demotion, in the type. The borrow ties it
/// to the authority's lifetime: a permit cannot outlive the lease.
#[derive(Debug, Clone, Copy)]
pub struct PublishPermit<'a> {
    epoch: AuthorityEpoch,
    _lease: std::marker::PhantomData<&'a AuthorityLease>,
}

impl PublishPermit<'_> {
    /// The authority epoch this publication binds to.
    #[must_use]
    pub fn epoch(&self) -> AuthorityEpoch {
        self.epoch
    }
}

// ── the wedged posture (§2.1.1) ────────────────────────────────────────────

/// The supervisor's operating constants. Defaults are the contract's
/// PROPOSED policy constants — 1 s beat, 30 misses, 5 s termination budget —
/// stated as policy, not measured receipts; deployment may tighten them only
/// after measuring the event-loop heartbeat under load (§2.1.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeartbeatPolicy {
    /// The holder advances progress at least once per this interval, and
    /// the supervisor samples at this cadence.
    pub beat: Duration,
    /// Consecutive missing/invalid samples that declare the holder wedged.
    pub misses_to_wedged: u32,
    /// The graceful-termination budget between termination and `SIGKILL`.
    pub term_grace: Duration,
}

impl Default for HeartbeatPolicy {
    fn default() -> Self {
        Self {
            beat: Duration::from_secs(1),
            misses_to_wedged: 30,
            term_grace: Duration::from_secs(5),
        }
    }
}

/// One supervisor observation of the holder's progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Observation {
    /// Progress advanced since the last sample (or a first baseline landed).
    Progressing,
    /// No progress; the count of consecutive misses so far.
    Missing(u32),
    /// The miss budget is spent: the holder is declared wedged.
    Wedged,
}

/// The supervisor's wedged declaration — the captured diagnostic (§2.1.1),
/// landed beside the lock as [`WEDGED_MARKER`] so refusals can teach
/// `wedged{retry_after}` while takeover runs. Hint-grade, like every file
/// beside the lock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WedgedReport {
    /// The wedged holder's pid.
    pub pid: i32,
    /// Consecutive missed samples at declaration.
    pub missed: u32,
    /// The client retry hint: the termination budget ahead.
    pub retry_after: Duration,
    /// Declaration wall time, unix milliseconds. Diagnostic, not law.
    pub declared_unix_ms: u128,
    /// The human-readable diagnostic line.
    pub diagnostic: String,
}

impl WedgedReport {
    fn to_json(&self) -> String {
        serde_json::json!({
            "pid": self.pid,
            "missed": self.missed,
            "retry_after_ms": u64::try_from(self.retry_after.as_millis()).unwrap_or(u64::MAX),
            "declared_unix_ms": self.declared_unix_ms.to_string(),
            "diagnostic": self.diagnostic,
        })
        .to_string()
    }

    /// Parse a declaration; `None` on any deviation — an unreadable marker
    /// falls back to the plain `Held` refusal rather than inventing a
    /// wedged claim.
    #[must_use]
    pub fn from_json(raw: &str) -> Option<Self> {
        let v: serde_json::Value = serde_json::from_str(raw).ok()?;
        Some(Self {
            pid: i32::try_from(v.get("pid")?.as_i64()?).ok()?,
            missed: u32::try_from(v.get("missed")?.as_u64()?).ok()?,
            retry_after: Duration::from_millis(v.get("retry_after_ms")?.as_u64()?),
            declared_unix_ms: v.get("declared_unix_ms")?.as_str()?.parse().ok()?,
            diagnostic: v.get("diagnostic")?.as_str()?.to_string(),
        })
    }
}

/// How a wedged episode ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WedgedOutcome {
    /// The holder was already dead when supervision looked — the kernel
    /// janitor's case, nothing to enforce.
    AlreadyDead,
    /// The holder exited within the graceful-termination budget.
    DiedOnTerm(WedgedReport),
    /// The holder survived the budget and was `SIGKILL`ed.
    RequiredKill(WedgedReport),
    /// `SIGKILL` did not end it — an uninterruptible kernel state. Host or
    /// operator recovery; there is no safe userspace lease-steal escape
    /// (§2.1.1). Loud by construction.
    StillAliveAfterKill(WedgedReport),
}

/// The external daemon supervisor (§2.1.1): observes the holder's heartbeat
/// progress, declares a hang after the miss budget, captures the diagnostic,
/// and turns the hang into process death — termination, a graceful budget,
/// then `SIGKILL`. It never steals the lease: only kernel-confirmed lock
/// release (a successor's [`AuthorityLease::acquire`] succeeding) lets
/// anyone serve.
///
/// MUST run in a process that did not inherit either authority fd — a
/// forked copy of the lease fd would keep the dead holder's lease held and
/// wedge the takeover this supervisor exists to unblock. Spawn it
/// independently (fork+exec closes the CLOEXEC fds), never fork it from a
/// serving daemon.
#[derive(Debug)]
pub struct Watchdog {
    progress: PathBuf,
    wedged: PathBuf,
    pid: libc::pid_t,
    policy: HeartbeatPolicy,
    last: Option<Vec<u8>>,
    misses: u32,
}

impl Watchdog {
    /// A supervisor for the authority of `root`, whose daemon is `pid`. The
    /// pid comes from the supervisor's own spawn bookkeeping — never from
    /// the owner record, which proves nothing.
    #[must_use]
    pub fn new(root: &fs::WorkspaceRoot, pid: i32, policy: HeartbeatPolicy) -> Self {
        let dir = root.0.join(AUTHORITY_DIR);
        Self {
            progress: dir.join(PROGRESS_FILE),
            wedged: dir.join(WEDGED_MARKER),
            pid,
            policy,
            last: None,
            misses: 0,
        }
    }

    /// The pure observation step: fold one sampled progress content into
    /// the miss counter. `None` (absent/unreadable) is a miss — missing and
    /// invalid heartbeats count alike (§2.1.1). Deterministic so the
    /// counting law is testable without a clock or a disk.
    pub fn note(&mut self, content: Option<Vec<u8>>) -> Observation {
        match content {
            Some(bytes) if self.last.as_deref() != Some(bytes.as_slice()) => {
                self.last = Some(bytes);
                self.misses = 0;
                Observation::Progressing
            }
            // Unchanged AND absent/unreadable both count as misses, and an
            // absence does not forget the last seen content: a progress file
            // deleted and recreated with the same bytes is still no progress.
            _ => {
                self.misses += 1;
                if self.misses >= self.policy.misses_to_wedged {
                    Observation::Wedged
                } else {
                    Observation::Missing(self.misses)
                }
            }
        }
    }

    /// One disk sample folded through [`Watchdog::note`].
    pub fn sample(&mut self) -> Observation {
        let content = std::fs::read(&self.progress).ok();
        self.note(content)
    }

    /// Sample at the policy cadence until the miss budget declares the
    /// holder wedged, then land the declaration marker and return the
    /// report. Returns `None` if the holder died on its own first — the
    /// kernel janitor already did the job.
    ///
    /// # Errors
    /// Landing the wedged marker failed.
    pub fn watch_until_wedged(&mut self) -> io::Result<Option<WedgedReport>> {
        loop {
            std::thread::sleep(self.policy.beat);
            if !process_alive(self.pid) {
                return Ok(None);
            }
            if self.sample() == Observation::Wedged {
                let report = WedgedReport {
                    pid: self.pid,
                    missed: self.misses,
                    retry_after: self.policy.term_grace,
                    declared_unix_ms: unix_ms(),
                    diagnostic: format!(
                        "no heartbeat progress in {} consecutive samples at {}ms cadence",
                        self.misses,
                        self.policy.beat.as_millis()
                    ),
                };
                let dir = self.wedged.parent().unwrap_or(Path::new("."));
                write_atomic(dir, WEDGED_MARKER, report.to_json().as_bytes())?;
                return Ok(Some(report));
            }
        }
    }

    /// Turn the declared hang into process death: send termination, wait
    /// the graceful budget, escalate to `SIGKILL`, and report which leg
    /// ended it. Never touches the lease — release is the kernel's.
    #[must_use]
    pub fn enforce(&self, report: WedgedReport) -> WedgedOutcome {
        // SAFETY: signalling a pid this supervisor owns the lifecycle of.
        unsafe { libc::kill(self.pid, libc::SIGTERM) };
        if wait_death(self.pid, self.policy.term_grace) {
            return WedgedOutcome::DiedOnTerm(report);
        }
        // SAFETY: as above.
        unsafe { libc::kill(self.pid, libc::SIGKILL) };
        if wait_death(self.pid, self.policy.term_grace) {
            return WedgedOutcome::RequiredKill(report);
        }
        WedgedOutcome::StillAliveAfterKill(report)
    }

    /// The whole §2.1.1 posture in one call: watch, declare, enforce.
    ///
    /// # Errors
    /// Landing the wedged marker failed.
    pub fn supervise(&mut self) -> io::Result<WedgedOutcome> {
        match self.watch_until_wedged()? {
            None => Ok(WedgedOutcome::AlreadyDead),
            Some(report) => Ok(self.enforce(report)),
        }
    }
}

/// Is `pid` alive (signal 0 probe)? `EPERM` counts as alive — it exists.
fn process_alive(pid: libc::pid_t) -> bool {
    // SAFETY: kill with signal 0 performs only the existence/permission
    // check; no signal is delivered.
    if unsafe { libc::kill(pid, 0) } == 0 {
        return true;
    }
    io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

/// Poll `pid` for death up to `budget`. A zombie still holds no flock —
/// kernel release happens at death, not at reaping — but `kill(pid, 0)`
/// answers alive for zombies, so the successor's lease acquire (not this
/// poll) remains the release authority; this poll only paces enforcement.
fn wait_death(pid: libc::pid_t, budget: Duration) -> bool {
    let deadline = Instant::now() + budget;
    loop {
        // Reap if it is our child; ignore errors — it may not be ours.
        let mut status: libc::c_int = 0;
        // SAFETY: non-blocking waitpid into a local; a pid that is not our
        // child fails harmlessly with ECHILD.
        unsafe { libc::waitpid(pid, &raw mut status, libc::WNOHANG) };
        if !process_alive(pid) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws() -> (tempfile::TempDir, fs::WorkspaceRoot) {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = fs::WorkspaceRoot(dir.path().to_path_buf());
        (dir, root)
    }

    /// Acceptance gate 1's in-process face: exactly one authority per
    /// workspace, the loser's refusal typed and non-blocking, the lease
    /// free again after an explicit release.
    #[test]
    fn election_exactly_one_authority_and_a_typed_refusal() {
        let (_dir, root) = ws();
        let held = AuthorityLease::acquire(&root).expect("first acquire wins");
        match AuthorityLease::acquire(&root) {
            Err(AuthorityUnavailable::Held { hint }) => {
                let hint = hint.expect("the winner landed an owner hint");
                // SAFETY: getpid never fails and has no preconditions.
                assert_eq!(hint.pid, unsafe { libc::getpid() });
                assert_eq!(hint.epoch_hex, held.epoch().hex());
            }
            other => panic!("a held lease refuses typed Held, got {other:?}"),
        }
        drop(held);
        AuthorityLease::acquire(&root).expect("free again after explicit release");
    }

    /// §2.1: a stale owner record never proves authority and never blocks a
    /// successor — the kernel lock is the only truth.
    #[test]
    fn a_stale_owner_record_never_proves_authority() {
        let (_dir, root) = ws();
        let dir = root.0.join(AUTHORITY_DIR);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let stale = OwnerRecord {
            pid: 999_999_999,
            epoch_hex: "f".repeat(32),
            workspace: "somewhere/else".into(),
        };
        std::fs::write(dir.join(OWNER_RECORD), stale.to_json()).expect("plant stale record");

        let lease = AuthorityLease::acquire(&root)
            .expect("a stale record with a free kernel lock must not refuse");
        let now = AuthorityLease::owner_hint(&root).expect("fresh record landed");
        assert_ne!(now.epoch_hex, stale.epoch_hex, "the record was re-minted");
        assert_eq!(now.epoch_hex, lease.epoch().hex());
    }

    /// §2.2: the epoch is new for each authority lifetime.
    #[test]
    fn the_epoch_is_new_for_each_lifetime() {
        let (_dir, root) = ws();
        let first = AuthorityLease::acquire(&root)
            .expect("first lifetime")
            .epoch();
        let second = AuthorityLease::acquire(&root)
            .expect("second lifetime")
            .epoch();
        assert_ne!(first, second, "two lifetimes, two epochs");
    }

    /// §2.3 as one in-process pass: recovery EX is held exactly for the
    /// takeover window; ready releases it BEFORE admissions open; the
    /// permit takes no flock (write.lock is free while publishing).
    #[test]
    fn takeover_holds_recovery_ex_and_ready_releases_it_before_admissions() {
        let (_dir, root) = ws();
        let lease = AuthorityLease::acquire(&root).expect("lease");
        let epoch = lease.epoch();

        let takeover = lease
            .begin_takeover(Duration::from_secs(2))
            .expect("recovery EX free");
        assert_eq!(
            fs::WriteLock::acquire(&root)
                .expect_err("EX held by the takeover")
                .kind(),
            io::ErrorKind::WouldBlock,
            "the recovery fence is the demoted write.lock, held through recovery"
        );
        assert_eq!(takeover.recovery_ex().root().0, root.0);

        let mut ready = takeover
            .ready(wire::Root("b3:test-root".into()))
            .expect("ready");
        let _free_again = fs::WriteLock::acquire(&root)
            .expect("recovery EX explicitly released at ready (§2.3 step 7)");

        // Ordinary publication takes no flock: the permit mints while
        // write.lock is held by someone else entirely.
        let permit = ready.permit();
        assert_eq!(permit.epoch(), epoch);
        assert_eq!(ready.root_at_ready().0, "b3:test-root");
        ready.beat().expect("heartbeat lands");
    }

    /// The bounded recovery-EX wait: a cooperating holder that releases is
    /// waited out; one that never releases is a loud timeout, not a hang.
    #[test]
    fn begin_takeover_waits_out_a_brief_holder_and_times_out_on_a_stuck_one() {
        let (_dir, root) = ws();

        let held = fs::WriteLock::acquire(&root).expect("cooperating holder");
        let lease = AuthorityLease::acquire(&root).expect("lease");
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(80));
            drop(held);
        });
        let takeover = lease
            .begin_takeover(Duration::from_secs(5))
            .expect("waited out the brief holder");
        releaser.join().expect("releaser thread");

        // Stuck case: hold EX and give the takeover a tiny deadline.
        let ready = takeover.ready(wire::Root("b3:x".into())).expect("ready");
        drop(ready);
        let _stuck = fs::WriteLock::acquire(&root).expect("stuck holder");
        let lease = AuthorityLease::acquire(&root).expect("lease again");
        match lease.begin_takeover(Duration::from_millis(60)) {
            Err(TakeoverError::RecoveryExTimeout { waited }) => {
                assert!(waited >= Duration::from_millis(60));
            }
            other => panic!("a stuck EX must time out loudly, got {other:?}"),
        }
    }

    /// §2.1.1: while a wedged declaration stands beside a held lock, the
    /// refusal is typed `wedged{retry_after}`; a corrupt declaration falls
    /// back to plain Held rather than inventing a wedged claim.
    #[test]
    fn acquire_types_the_wedged_refusal_from_the_standing_declaration() {
        let (_dir, root) = ws();
        let held = AuthorityLease::acquire(&root).expect("holder");
        let dir = root.0.join(AUTHORITY_DIR);
        let report = WedgedReport {
            pid: 4242,
            missed: 30,
            retry_after: Duration::from_secs(5),
            declared_unix_ms: 1_700_000_000_000,
            diagnostic: "no heartbeat progress in 30 consecutive samples".into(),
        };
        std::fs::write(dir.join(WEDGED_MARKER), report.to_json()).expect("plant declaration");

        match AuthorityLease::acquire(&root) {
            Err(AuthorityUnavailable::Wedged {
                retry_after,
                report: got,
            }) => {
                assert_eq!(retry_after, Duration::from_secs(5));
                assert_eq!(got, report);
            }
            other => panic!("expected the wedged typing, got {other:?}"),
        }

        std::fs::write(dir.join(WEDGED_MARKER), "not json").expect("corrupt it");
        match AuthorityLease::acquire(&root) {
            Err(AuthorityUnavailable::Held { .. }) => {}
            other => panic!("a corrupt declaration is no wedged claim, got {other:?}"),
        }
        drop(held);
    }

    /// A successor's ready closes the wedged episode: the marker is removed
    /// so the next refusal is Held, not a stale wedged teaching.
    #[test]
    fn ready_removes_the_standing_wedged_marker() {
        let (_dir, root) = ws();
        let dir = root.0.join(AUTHORITY_DIR);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let report = WedgedReport {
            pid: 7,
            missed: 30,
            retry_after: Duration::from_secs(5),
            declared_unix_ms: 0,
            diagnostic: "d".into(),
        };
        std::fs::write(dir.join(WEDGED_MARKER), report.to_json()).expect("plant");

        let lease = AuthorityLease::acquire(&root).expect("successor lease");
        let takeover = lease
            .begin_takeover(Duration::from_secs(2))
            .expect("takeover");
        let _ready = takeover.ready(wire::Root("b3:r".into())).expect("ready");
        assert!(
            !dir.join(WEDGED_MARKER).exists(),
            "ready ends the wedged episode (§2.1.1)"
        );
        match AuthorityLease::acquire(&root) {
            Err(AuthorityUnavailable::Held { .. }) => {}
            other => panic!("post-ready refusals are Held, got {other:?}"),
        }
    }

    /// The miss-counting law, driven deterministically: absent and unchanged
    /// both count; the budget declares wedged at exactly the policy count;
    /// any progress resets the counter.
    #[test]
    fn the_watchdog_counting_law_is_exact() {
        let (_dir, root) = ws();
        let policy = HeartbeatPolicy {
            beat: Duration::from_millis(1),
            misses_to_wedged: 3,
            term_grace: Duration::from_millis(1),
        };
        let mut dog = Watchdog::new(&root, 1, policy);

        assert_eq!(dog.note(Some(b"1".to_vec())), Observation::Progressing);
        assert_eq!(dog.note(Some(b"1".to_vec())), Observation::Missing(1));
        assert_eq!(
            dog.note(None),
            Observation::Missing(2),
            "invalid counts like missing"
        );
        assert_eq!(
            dog.note(None),
            Observation::Wedged,
            "the third miss spends the budget"
        );
        assert_eq!(
            dog.note(Some(b"1".to_vec())),
            Observation::Wedged,
            "a file recreated with the SAME bytes is still no progress"
        );

        // Progress resets: a fresh baseline, then the full budget again.
        assert_eq!(dog.note(Some(b"2".to_vec())), Observation::Progressing);
        assert_eq!(dog.note(Some(b"2".to_vec())), Observation::Missing(1));
        assert_eq!(dog.note(Some(b"3".to_vec())), Observation::Progressing);
    }

    /// The hint and declaration records round-trip, and garbage parses to
    /// `None` — never a refusal of their own.
    #[test]
    fn hint_records_round_trip_and_garbage_is_no_hint() {
        let record = OwnerRecord {
            pid: 123,
            epoch_hex: "0".repeat(32),
            workspace: "/w".into(),
        };
        assert_eq!(OwnerRecord::from_json(&record.to_json()), Some(record));
        assert_eq!(OwnerRecord::from_json("{"), None);
        assert_eq!(OwnerRecord::from_json("{\"pid\": \"x\"}"), None);

        let report = WedgedReport {
            pid: 9,
            missed: 30,
            retry_after: Duration::from_secs(5),
            declared_unix_ms: 42,
            diagnostic: "d".into(),
        };
        assert_eq!(WedgedReport::from_json(&report.to_json()), Some(report));
        assert_eq!(WedgedReport::from_json("[]"), None);
    }
}
