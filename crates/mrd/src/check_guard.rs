//! Process guards for `mrd check`: one-at-a-time non-blocking slot lock and
//! session-lifetime / wall-clock watchdog (`docs/status.md` § `mrd check`).
//!
//! 1. **Slot lock (`acquire_slot`)** — a non-blocking exclusive `flock(2)` so
//!    two `mrd check` runs never overlap on one host. Precedence:
//!    - `MRD_CHECK_LOCK` when set and non-empty;
//!    - `$HOME/.local/share/ucc/work/fleet-mutex/mrd-nyc1.lock` when that file
//!      already exists on disk and neither `XDG_CACHE_HOME` nor
//!      `MERIDIAN_CACHE_ROOT` overrides the cache root (the devstation-nyc-1
//!      host slot, shared with callers that wrap `flock`);
//!    - `<socket-stem>.check.lock` beside the daemon socket (per cache root, so
//!      every other host and every isolated test sandbox gets a slot without
//!      setup).
//!
//!    When an ancestor wrapper already holds that lock for this process (e.g.
//!    `flock <lock> mrd check …`), `acquire_slot` detects the inherited locked
//!    descriptor or ancestor PID in `/proc/locks` (or `MRD_CHECK_SLOT_HELD=1`
//!    on non-Linux hosts) and proceeds. When an unrelated process holds the
//!    slot, `check` refuses `busy` (exit [`EXIT_BUSY`], 75 = `EX_TEMPFAIL`) and
//!    exits immediately — it never queues.
//! 2. **Session-lifetime & wall-clock watchdog (`Watchdog`)** — `mrd check`
//!    never detaches (`setsid`/`nohup`). It dies with its caller on `SIGHUP`,
//!    `SIGTERM`, `SIGINT`, direct parent or ancestor-chain death up to the
//!    session leader (including `ssh … bash -s` wrapped in `timeout`, `flock`,
//!    or subshells), stdout pipe peer-close (`POLLERR`/`POLLHUP`), or its
//!    wall-clock bound (`MRD_CHECK_TIMEOUT_SECS`; default 60 s for the bounded
//!    `--commit-gate -- PATH…` lane only, unbounded by default for unscoped or
//!    non-gate runs, and `0` disables the bound), terminating any descendant
//!    processes it spawned before exit. Deliberately backgrounded callers may
//!    opt out of the ancestor/pipe check with `MRD_CHECK_ALLOW_DETACH=1` or by
//!    detaching via `setsid` before invoking `mrd check`.

use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::json;

use crate::{Fail, Format};

/// Environment override naming the lock file for `mrd check`'s one-at-a-time slot.
const CHECK_LOCK_ENV: &str = "MRD_CHECK_LOCK";

/// Environment override telling `mrd check` its caller already holds the slot
/// lock (portable fallback on non-Linux hosts where `/proc` is unavailable).
const CHECK_SLOT_HELD_ENV: &str = "MRD_CHECK_SLOT_HELD";

/// Environment opt-out for deliberately backgrounded callers (e.g. an async
/// `pre-push` hook) that exit before `mrd check` finishes.
const CHECK_ALLOW_DETACH_ENV: &str = "MRD_CHECK_ALLOW_DETACH";

/// Relative path under `$HOME` of the fleet's nyc-1 `mrd` flock file.
const NYC1_LOCK_REL: &str = ".local/share/ucc/work/fleet-mutex/mrd-nyc1.lock";

/// Environment override for `mrd check`'s wall-clock bound, in whole seconds
/// (`0` disables the wall-clock bound).
const CHECK_TIMEOUT_ENV: &str = "MRD_CHECK_TIMEOUT_SECS";

/// Default wall-clock bound for the bounded `--commit-gate -- PATH…` lane,
/// sized to a one-card check with headroom for slow disk or git I/O.
const SCOPED_TIMEOUT_SECS: u64 = 60;

/// Watchdog polling granularity for parent-session death, pipe hangup, and
/// wall-clock expiry.
const WATCHDOG_POLL: Duration = Duration::from_millis(100);

/// Exit status when another unrelated `mrd check` already holds the host slot
/// (`EX_TEMPFAIL` — retry; distinct from 0 green, 1 finding, 2 bad invocation).
pub(crate) const EXIT_BUSY: u8 = 75;

/// Exit status on wall-clock timeout (GNU `timeout` convention).
const EXIT_TIMEOUT: i32 = 124;

/// Exit status when the parent or SSH session disappears (`128 + SIGHUP`).
const EXIT_HANGUP: i32 = 129;

/// Exit status when stdout's pipe peer closes (`128 + SIGPIPE`).
const EXIT_PIPE: i32 = 141;

/// Signal recorded by our async-signal-safe handler (0 = none).
static TERM_SIGNAL: AtomicI32 = AtomicI32::new(0);

/// Set by the `SIGUSR2` (`PR_SET_PDEATHSIG`) handler so the watchdog wakes and
/// verifies `getppid()` without falsely aborting when a parent's worker thread
/// exits while the parent process is still alive.
static PDEATH_SIGNALED: AtomicBool = AtomicBool::new(false);

extern "C" fn on_term_signal(sig: libc::c_int) {
    TERM_SIGNAL.store(sig, Ordering::SeqCst);
}

#[cfg(target_os = "linux")]
extern "C" fn on_pdeath_signal(_sig: libc::c_int) {
    PDEATH_SIGNALED.store(true, Ordering::SeqCst);
}

/// Exclusive hold on `mrd check`'s one-at-a-time slot; released on drop (and by
/// the kernel on any process exit).
pub(crate) struct CheckSlot {
    _file: File,
}

fn env_is_truthy(name: &str) -> bool {
    std::env::var(name).is_ok_and(|raw| {
        matches!(
            raw.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

/// Resolve the lock file path for this host/environment.
fn resolve_lock_path() -> Option<PathBuf> {
    if let Some(raw) = std::env::var_os(CHECK_LOCK_ENV)
        && !raw.is_empty()
    {
        return Some(PathBuf::from(raw));
    }
    let cache_overridden = std::env::var_os("XDG_CACHE_HOME").is_some_and(|v| !v.is_empty())
        || std::env::var_os("MERIDIAN_CACHE_ROOT").is_some_and(|v| !v.is_empty());
    if !cache_overridden
        && let Some(home) = std::env::var_os("HOME")
        && !home.is_empty()
    {
        let nyc1 = PathBuf::from(home).join(NYC1_LOCK_REL);
        if nyc1.exists() {
            return Some(nyc1);
        }
    }
    let client = registry::Client::from_default().ok()?;
    Some(client.socket_path().with_extension("check.lock"))
}

/// Acquire `mrd check`'s non-blocking exclusive flock slot.
///
/// # Errors
/// [`Fail`] exit [`EXIT_BUSY`] (75) with `busy` when another unrelated process
/// holds the slot — never queues.
pub(crate) fn acquire_slot(format: Format, workspace: &Path) -> Result<Option<CheckSlot>, Fail> {
    let Some(path) = resolve_lock_path() else {
        return Ok(None);
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(file) = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
    else {
        return Ok(None);
    };
    // SAFETY: `file` owns a live open descriptor; `flock(2)` with `LOCK_EX |
    // LOCK_NB` is non-blocking and matches `flock(1)` on Linux and macOS.
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc == 0 {
        return Ok(Some(CheckSlot { _file: file }));
    }
    let err = io::Error::last_os_error();
    if err.kind() == io::ErrorKind::WouldBlock {
        // A wrapper such as `flock <lock> mrd check …` or `( flock 9; mrd check ) 9><lock>`
        // already holds the exclusive host slot for this invocation.
        if env_is_truthy(CHECK_SLOT_HELD_ENV) || slot_held_by_caller_or_ancestor(&file) {
            return Ok(None);
        }
        let detail = format!(
            "busy — another mrd check holds the slot ({}); exiting without queuing (retry once it finishes)",
            path.display()
        );
        if matches!(format, Format::Json) {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "workspace": workspace.display().to_string(),
                    "error": {
                        "code": "busy",
                        "exit_code": EXIT_BUSY,
                        "message": detail,
                        "lock": path.display().to_string(),
                    },
                }))
                .expect("json")
            );
        }
        return Err(Fail::with_code(
            EXIT_BUSY,
            format!("check refused: {detail}"),
        ));
    }
    Ok(None)
}

/// Detect whether `file`'s exclusive `flock` is held by an inherited open file
/// description (e.g. `flock <file> mrd check` or `( flock 9; mrd check ) 9><file>`)
/// or by an ancestor process in `/proc/locks` (e.g. `flock -o <file> mrd check`).
#[cfg(target_os = "linux")]
fn slot_held_by_caller_or_ancestor(file: &File) -> bool {
    let target_fd = file.as_raw_fd();
    let Some((target_dev, target_ino)) = fd_dev_ino(target_fd) else {
        return false;
    };

    // 1. Inherited locked descriptor: covers both `flock <file> <cmd>` (fd stays
    //    open across exec) and `( flock 9; <cmd> ) 9><file>` (where `flock 9`
    //    already exited, leaving the locked OFD inherited on fd 9).
    if let Ok(entries) = std::fs::read_dir("/proc/self/fd") {
        for entry in entries.flatten() {
            let Ok(fd) = entry.file_name().to_string_lossy().parse::<libc::c_int>() else {
                continue;
            };
            if fd == target_fd {
                continue;
            }
            if fd_dev_ino(fd) == Some((target_dev, target_ino)) && fdinfo_holds_flock(fd) {
                return true;
            }
        }
    }

    // 2. Ancestor process holding FLOCK WRITE in `/proc/locks` (covers `flock -o`
    //    or wrappers that open the lock file with `O_CLOEXEC`).
    let Ok(locks) = std::fs::read_to_string("/proc/locks") else {
        return false;
    };
    let ancestors = ancestor_pid_set();
    let target_maj = libc::major(target_dev);
    let target_min = libc::minor(target_dev);
    for line in locks.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        // Format: `<n>: FLOCK ADVISORY WRITE <pid> <maj>:<min>:<ino> 0 EOF`
        if fields.len() < 6 {
            continue;
        }
        if fields[1] != "FLOCK" || fields[3] != "WRITE" {
            continue;
        }
        let mut dev_ino = fields[5].split(':');
        let (Some(maj_s), Some(min_s), Some(ino_s)) =
            (dev_ino.next(), dev_ino.next(), dev_ino.next())
        else {
            continue;
        };
        let (Ok(maj), Ok(min), Ok(ino)) = (
            u32::from_str_radix(maj_s, 16),
            u32::from_str_radix(min_s, 16),
            ino_s.parse::<libc::ino_t>(),
        ) else {
            continue;
        };
        // On btrfs/overlayfs/anonymous mounts (`major == 0`), Linux `fstat`
        // reports the subvolume's synthetic minor while `/proc/locks` reports
        // the superblock minor; match `(0, ino)` there, and exact `(maj, min,
        // ino)` for block-backed filesystems.
        let dev_matches = if target_maj == 0 && maj == 0 {
            true
        } else {
            maj == target_maj && min == target_min
        };
        if !dev_matches || ino != target_ino {
            continue;
        }
        if let Ok(holder_pid) = fields[4].parse::<libc::pid_t>()
            && ancestors.contains(&holder_pid)
        {
            return true;
        }
    }
    false
}

#[cfg(not(target_os = "linux"))]
fn slot_held_by_caller_or_ancestor(_file: &File) -> bool {
    false
}

#[cfg(target_os = "linux")]
fn fd_dev_ino(fd: libc::c_int) -> Option<(libc::dev_t, libc::ino_t)> {
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: `fstat` writes into `st` for an open descriptor.
    if unsafe { libc::fstat(fd, &raw mut st) } != 0 {
        return None;
    }
    Some((st.st_dev, st.st_ino))
}

#[cfg(target_os = "linux")]
fn fdinfo_holds_flock(fd: libc::c_int) -> bool {
    let Ok(info) = std::fs::read_to_string(format!("/proc/self/fdinfo/{fd}")) else {
        return false;
    };
    info.lines().any(|line| {
        let Some(rest) = line.strip_prefix("lock:") else {
            return false;
        };
        let fields: Vec<&str> = rest.split_whitespace().collect();
        fields.len() >= 4 && fields[1] == "FLOCK" && fields[3] == "WRITE"
    })
}

#[cfg(target_os = "linux")]
fn ancestor_pid_set() -> HashSet<libc::pid_t> {
    let mut out = HashSet::new();
    if let Ok(self_pid) = libc::pid_t::try_from(std::process::id()) {
        out.insert(self_pid);
    }
    // SAFETY: `getppid(2)` has no preconditions.
    let mut cur = unsafe { libc::getppid() };
    for _ in 0..64 {
        if cur <= 1 || !out.insert(cur) {
            break;
        }
        match parent_pid_of(cur) {
            Some(next) if next > 1 => cur = next,
            _ => break,
        }
    }
    out
}

/// Wall-clock bound for `mrd check`:
/// - `MRD_CHECK_TIMEOUT_SECS=0` disables the bound (`None`);
/// - `MRD_CHECK_TIMEOUT_SECS=N` (`N > 0`) sets an explicit `N`-second bound;
/// - otherwise, 60 s for the bounded `--commit-gate -- PATH…` lane only, and
///   unbounded (`None`) for unscoped or non-gate checks.
pub(crate) fn wall_clock_bound(scoped_gate: bool) -> Option<Duration> {
    let raw = std::env::var(CHECK_TIMEOUT_ENV).ok();
    wall_clock_bound_from(scoped_gate, raw.as_deref())
}

fn wall_clock_bound_from(scoped_gate: bool, raw: Option<&str>) -> Option<Duration> {
    if let Some(raw) = raw
        && let Ok(secs) = raw.trim().parse::<u64>()
    {
        return (secs > 0).then(|| Duration::from_secs(secs));
    }
    scoped_gate.then(|| Duration::from_secs(SCOPED_TIMEOUT_SECS))
}

/// Active session-lifetime and wall-clock guard for one `mrd check` run.
/// Dropping stops and joins the watchdog thread cleanly.
pub(crate) struct Watchdog {
    stop: Arc<(Mutex<bool>, Condvar)>,
    done: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Watchdog {
    /// Arm `SIGHUP`/`SIGTERM`/`SIGINT` handlers, ancestor-chain session
    /// monitoring (unless `MRD_CHECK_ALLOW_DETACH=1`), stdout pipe-hangup
    /// detection, and the optional wall-clock bound.
    pub(crate) fn arm(timeout: Option<Duration>) -> Self {
        TERM_SIGNAL.store(0, Ordering::SeqCst);
        PDEATH_SIGNALED.store(false, Ordering::SeqCst);

        let allow_detach = env_is_truthy(CHECK_ALLOW_DETACH_ENV);
        // SAFETY: `getppid(2)` has no preconditions.
        let initial_ppid = unsafe { libc::getppid() };
        let ancestors = if allow_detach {
            Vec::new()
        } else {
            capture_ancestor_chain(initial_ppid)
        };
        let stdout_is_stream = !allow_detach && fd_is_pipe_or_socket(libc::STDOUT_FILENO);

        install_signal_handlers();
        if !allow_detach {
            arm_pdeathsig(initial_ppid);
        }

        let stop = Arc::new((Mutex::new(false), Condvar::new()));
        let done = Arc::new(AtomicBool::new(false));
        let stop_bg = Arc::clone(&stop);
        let done_bg = Arc::clone(&done);
        let deadline = timeout.map(|t| (Instant::now() + t, t));

        let handle = thread::Builder::new()
            .name("mrd-check-guard".to_owned())
            .spawn(move || {
                watchdog_loop(
                    &stop_bg,
                    &done_bg,
                    allow_detach,
                    initial_ppid,
                    &ancestors,
                    stdout_is_stream,
                    deadline,
                );
            })
            .ok();

        Self { stop, done, handle }
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        self.done.store(true, Ordering::SeqCst);
        let (lock, cv) = &*self.stop;
        if let Ok(mut stopped) = lock.lock() {
            *stopped = true;
            cv.notify_all();
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn install_signal_handlers() {
    // SAFETY: `on_term_signal` performs a single atomic store (async-signal-safe).
    let handler = on_term_signal as *const () as libc::sighandler_t;
    unsafe {
        let _ = libc::signal(libc::SIGHUP, handler);
        let _ = libc::signal(libc::SIGTERM, handler);
        let _ = libc::signal(libc::SIGINT, handler);
    }
    #[cfg(target_os = "linux")]
    {
        let pdeath = on_pdeath_signal as *const () as libc::sighandler_t;
        // SAFETY: `on_pdeath_signal` performs a single atomic store (async-signal-safe).
        unsafe {
            let _ = libc::signal(libc::SIGUSR2, pdeath);
        }
    }
}

#[cfg(target_os = "linux")]
fn arm_pdeathsig(initial_ppid: libc::pid_t) {
    // Use `SIGUSR2` rather than `SIGTERM` for `PR_SET_PDEATHSIG`: on Linux
    // `PDEATHSIG` fires when the parent *thread* that spawned us exits, even if
    // the parent *process* is still alive (e.g. a tokio/Go worker thread idling
    // out). Recording `PDEATH_SIGNALED` lets the watchdog verify `getppid()`
    // before aborting.
    // SAFETY: `PR_SET_PDEATHSIG` takes a single signal integer; checking
    // `getppid()` immediately afterwards closes the race where our parent died
    // before `prctl` executed.
    unsafe {
        let _ = libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGUSR2);
        let now_ppid = libc::getppid();
        if initial_ppid > 1 && (now_ppid == 1 || now_ppid != initial_ppid) {
            abort_now(
                EXIT_HANGUP,
                "mrd check: aborted — parent process exited before check started",
            );
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn arm_pdeathsig(_initial_ppid: libc::pid_t) {}

/// Record `(ancestor_pid, expected_ppid)` from `initial_ppid` up to and
/// including our session leader (`getsid(0)`), skipping any ancestor whose
/// parent is already PID 1 (e.g. an already-detached `setsid` session).
#[cfg(target_os = "linux")]
fn capture_ancestor_chain(initial_ppid: libc::pid_t) -> Vec<(libc::pid_t, libc::pid_t)> {
    // SAFETY: `getsid(0)` queries the calling process's own session id.
    let my_sid = unsafe { libc::getsid(0) };
    let mut chain = Vec::new();
    let mut seen = HashSet::new();
    let mut cur = initial_ppid;
    for _ in 0..32 {
        if cur <= 1 || !seen.insert(cur) {
            break;
        }
        let Some(ppid) = parent_pid_of(cur) else {
            break;
        };
        if ppid <= 1 {
            break;
        }
        chain.push((cur, ppid));
        if my_sid > 0 && cur == my_sid {
            break;
        }
        cur = ppid;
    }
    chain
}

#[cfg(not(target_os = "linux"))]
fn capture_ancestor_chain(_initial_ppid: libc::pid_t) -> Vec<(libc::pid_t, libc::pid_t)> {
    Vec::new()
}

fn watchdog_loop(
    stop: &(Mutex<bool>, Condvar),
    done: &AtomicBool,
    allow_detach: bool,
    initial_ppid: libc::pid_t,
    ancestors: &[(libc::pid_t, libc::pid_t)],
    stdout_is_stream: bool,
    deadline: Option<(Instant, Duration)>,
) {
    let (lock, cv) = stop;
    loop {
        if done.load(Ordering::SeqCst) {
            return;
        }
        let sig = TERM_SIGNAL.load(Ordering::SeqCst);
        if sig != 0 {
            abort_now(
                128 + sig,
                &format!("mrd check: aborted — received signal {sig}"),
            );
        }
        if !allow_detach {
            let _ = PDEATH_SIGNALED.swap(false, Ordering::SeqCst);
            // SAFETY: `getppid(2)` has no preconditions.
            let cur_ppid = unsafe { libc::getppid() };
            if initial_ppid > 1 && (cur_ppid == 1 || cur_ppid != initial_ppid) {
                abort_now(
                    EXIT_HANGUP,
                    "mrd check: aborted — parent process exited (reparented)",
                );
            }
            // Watch the whole ancestor chain up to the session leader so deeper
            // remote ssh shapes (`sshd → bash -s → timeout/flock/subshell → mrd`)
            // abort immediately when any ancestor disappears or is reparented.
            for &(anc_pid, expected_ppid) in ancestors {
                if parent_pid_of(anc_pid) != Some(expected_ppid) {
                    abort_now(
                        EXIT_HANGUP,
                        "mrd check: aborted — ssh/parent session ended (ancestor exited or reparented)",
                    );
                }
            }
            if stdout_is_stream && stdout_peer_closed() {
                abort_now(
                    EXIT_PIPE,
                    "mrd check: aborted — stdout pipe closed by caller",
                );
            }
        }
        if let Some((at, timeout)) = deadline
            && Instant::now() >= at
        {
            abort_now(
                EXIT_TIMEOUT,
                &format!(
                    "mrd check: aborted — wall-clock bound ({}s) exceeded",
                    timeout.as_secs()
                ),
            );
        }

        let Ok(guard) = lock.lock() else {
            return;
        };
        if *guard {
            return;
        }
        let _ = cv.wait_timeout(guard, WATCHDOG_POLL);
    }
}

fn abort_now(code: i32, reason: &str) -> ! {
    kill_direct_children();
    let _ = io::Write::write_all(&mut io::stderr(), format!("{reason}\n").as_bytes());
    std::process::exit(code);
}

fn fd_is_pipe_or_socket(fd: libc::c_int) -> bool {
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: `fstat` writes into `st` for an open descriptor.
    if unsafe { libc::fstat(fd, &raw mut st) } != 0 {
        return false;
    }
    let kind = st.st_mode & libc::S_IFMT;
    kind == libc::S_IFIFO || kind == libc::S_IFSOCK
}

fn stdout_peer_closed() -> bool {
    let mut pfd = libc::pollfd {
        fd: libc::STDOUT_FILENO,
        events: 0,
        revents: 0,
    };
    // SAFETY: `pfd` points to a single valid `pollfd` struct; timeout 0 is
    // non-blocking.
    let rc = unsafe { libc::poll(&raw mut pfd, 1, 0) };
    rc > 0 && (pfd.revents & (libc::POLLERR | libc::POLLHUP)) != 0
}

#[cfg(target_os = "linux")]
fn parent_pid_of(pid: libc::pid_t) -> Option<libc::pid_t> {
    if pid <= 1 {
        return None;
    }
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    parse_linux_stat_ppid(&stat)
}

#[cfg(not(target_os = "linux"))]
fn parent_pid_of(_pid: libc::pid_t) -> Option<libc::pid_t> {
    None
}

/// Extract PPID (field 4) from Linux `/proc/<pid>/stat`, skipping past the last
/// `')'` so process names containing spaces or parentheses parse cleanly.
#[cfg(target_os = "linux")]
fn parse_linux_stat_ppid(stat: &str) -> Option<libc::pid_t> {
    let after_comm = stat.rsplit_once(')')?.1;
    let mut fields = after_comm.split_whitespace();
    let _state = fields.next()?;
    fields.next()?.parse::<libc::pid_t>().ok()
}

/// Collect all live descendant PIDs of `root_pid` from Linux `/proc`.
#[cfg(target_os = "linux")]
fn descendant_pids(root_pid: libc::pid_t) -> Vec<libc::pid_t> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    let mut children_of: std::collections::HashMap<libc::pid_t, Vec<libc::pid_t>> =
        std::collections::HashMap::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(text) = name.to_str() else {
            continue;
        };
        let Ok(pid) = text.parse::<libc::pid_t>() else {
            continue;
        };
        if let Some(ppid) = parent_pid_of(pid) {
            children_of.entry(ppid).or_default().push(pid);
        }
    }
    let mut out = Vec::new();
    let mut stack = vec![root_pid];
    while let Some(cur) = stack.pop() {
        if let Some(kids) = children_of.get(&cur) {
            for &kid in kids {
                if kid != root_pid {
                    out.push(kid);
                    stack.push(kid);
                }
            }
        }
    }
    out
}

/// Terminate every descendant process of this `mrd check` (e.g. a running `git`
/// subprocess or shell helper) so nothing survives an abort.
#[cfg(target_os = "linux")]
fn kill_direct_children() {
    let Ok(self_pid) = libc::pid_t::try_from(std::process::id()) else {
        return;
    };
    let first = descendant_pids(self_pid);
    for &pid in &first {
        // SAFETY: sending `SIGTERM` to a verified descendant of this process.
        unsafe {
            let _ = libc::kill(pid, libc::SIGTERM);
        }
    }
    if !first.is_empty() {
        thread::sleep(Duration::from_millis(50));
    }
    // Re-scan after the grace window so any child spawned mid-abort is caught too.
    for pid in descendant_pids(self_pid) {
        // SAFETY: escalating to `SIGKILL` for any remaining verified descendant.
        unsafe {
            let _ = libc::kill(pid, libc::SIGKILL);
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn kill_direct_children() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wall_clock_bound_applies_only_to_scoped_gate_by_default() {
        // Only the bounded `--commit-gate -- PATH…` lane gets the 60 s default;
        // unscoped and non-gate runs are unbounded unless overridden.
        assert_eq!(
            wall_clock_bound_from(true, None),
            Some(Duration::from_secs(SCOPED_TIMEOUT_SECS))
        );
        assert_eq!(wall_clock_bound_from(false, None), None);
        // Explicit `0` disables the bound even for the scoped lane.
        assert_eq!(wall_clock_bound_from(true, Some("0")), None);
        assert_eq!(wall_clock_bound_from(false, Some("0")), None);
        // Positive integer overrides either lane.
        assert_eq!(
            wall_clock_bound_from(true, Some("15")),
            Some(Duration::from_secs(15))
        );
        assert_eq!(
            wall_clock_bound_from(false, Some("45")),
            Some(Duration::from_secs(45))
        );
    }
}
