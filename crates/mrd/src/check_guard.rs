//! Process guards for `mrd check`: one-at-a-time non-blocking slot lock and
//! session-lifetime / wall-clock watchdog (`docs/status.md` § `mrd check`).
//!
//! 1. **Slot lock (`acquire_slot`)** — a non-blocking exclusive `flock(2)` so
//!    two `mrd check` runs never overlap on one host. Precedence:
//!    - `MRD_CHECK_LOCK` when set and non-empty;
//!    - `$HOME/.local/share/ucc/work/fleet-mutex/mrd-nyc1.lock` when that file
//!      already exists on disk (the devstation-nyc-1 host slot, shared with
//!      callers that wrap `flock`);
//!    - `<socket-stem>.check.lock` beside the daemon socket (per cache root, so
//!      every other host and every isolated test sandbox gets a slot without
//!      setup).
//!
//!    When another holder owns the lock, `check` refuses `busy` (exit 2) and
//!    exits immediately — it never queues.
//! 2. **Session-lifetime & wall-clock watchdog (`Watchdog`)** — `mrd check`
//!    never detaches (`setsid`/`nohup`). It dies with its caller on `SIGHUP`,
//!    `SIGTERM`, `SIGINT`, direct parent death (`PR_SET_PDEATHSIG` on Linux +
//!    `getppid` change), parent-shell reparenting to PID 1 (an orphaned remote
//!    `ssh … bash -s` whose `sshd` exited), stdout pipe peer-close (`POLLERR`),
//!    or the wall-clock bound (`MRD_CHECK_TIMEOUT_SECS`; default 60 s for a
//!    path-scoped check and 300 s for an unscoped check), terminating any
//!    direct child processes it spawned before exit.

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

/// Relative path under `$HOME` of the fleet's nyc-1 `mrd` flock file.
const NYC1_LOCK_REL: &str = ".local/share/ucc/work/fleet-mutex/mrd-nyc1.lock";

/// Environment override for `mrd check`'s wall-clock bound, in whole seconds.
const CHECK_TIMEOUT_ENV: &str = "MRD_CHECK_TIMEOUT_SECS";

/// Default wall-clock bound for a path-scoped check (`-- PATH…`), sized to a
/// one-card check with headroom for slow disk or git I/O.
const SCOPED_TIMEOUT_SECS: u64 = 60;

/// Default wall-clock bound for an unscoped whole-corpus check.
const UNSCOPED_TIMEOUT_SECS: u64 = 300;

/// Watchdog polling granularity for parent-session death, pipe hangup, and
/// wall-clock expiry.
const WATCHDOG_POLL: Duration = Duration::from_millis(100);

/// Exit status on wall-clock timeout (GNU `timeout` convention).
const EXIT_TIMEOUT: i32 = 124;

/// Exit status when the parent or SSH session disappears (`128 + SIGHUP`).
const EXIT_HANGUP: i32 = 129;

/// Exit status when stdout's pipe peer closes (`128 + SIGPIPE`).
const EXIT_PIPE: i32 = 141;

/// Signal recorded by our async-signal-safe handler (0 = none).
static TERM_SIGNAL: AtomicI32 = AtomicI32::new(0);

extern "C" fn on_term_signal(sig: libc::c_int) {
    TERM_SIGNAL.store(sig, Ordering::SeqCst);
}

/// Exclusive hold on `mrd check`'s one-at-a-time slot; released on drop (and by
/// the kernel on any process exit).
pub(crate) struct CheckSlot {
    _file: File,
}

/// Resolve the lock file path for this host/environment.
fn resolve_lock_path() -> Option<PathBuf> {
    if let Some(raw) = std::env::var_os(CHECK_LOCK_ENV)
        && !raw.is_empty()
    {
        return Some(PathBuf::from(raw));
    }
    if let Some(home) = std::env::var_os("HOME")
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
/// [`Fail`] exit 2 with `busy` when another process holds the slot — never
/// queues.
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
        let detail = format!(
            "busy — another mrd check holds the slot ({}); exiting without queuing",
            path.display()
        );
        if matches!(format, Format::Json) {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "workspace": workspace.display().to_string(),
                    "error": {
                        "code": "busy",
                        "message": detail,
                        "lock": path.display().to_string(),
                    },
                }))
                .expect("json")
            );
        }
        return Err(Fail::tool(format!("check refused: {detail}")));
    }
    Ok(None)
}

/// Wall-clock bound for `mrd check`: `MRD_CHECK_TIMEOUT_SECS` when set, else
/// 60 s for a path-scoped check and 300 s for an unscoped check.
pub(crate) fn wall_clock_bound(scoped: bool) -> Duration {
    if let Ok(raw) = std::env::var(CHECK_TIMEOUT_ENV)
        && let Ok(secs) = raw.trim().parse::<u64>()
        && secs > 0
    {
        return Duration::from_secs(secs);
    }
    Duration::from_secs(if scoped {
        SCOPED_TIMEOUT_SECS
    } else {
        UNSCOPED_TIMEOUT_SECS
    })
}

/// Active session-lifetime and wall-clock guard for one `mrd check` run.
/// Dropping stops and joins the watchdog thread cleanly.
pub(crate) struct Watchdog {
    stop: Arc<(Mutex<bool>, Condvar)>,
    done: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Watchdog {
    /// Arm `PR_SET_PDEATHSIG`, `SIGHUP`/`SIGTERM`/`SIGINT` handlers, parent
    /// session monitoring, stdout pipe-hangup detection, and the wall-clock
    /// bound.
    pub(crate) fn arm(timeout: Duration) -> Self {
        TERM_SIGNAL.store(0, Ordering::SeqCst);

        // SAFETY: `getppid(2)` has no preconditions.
        let initial_ppid = unsafe { libc::getppid() };
        let grandparent_pid = parent_pid_of(initial_ppid);
        let stdout_is_stream = fd_is_pipe_or_socket(libc::STDOUT_FILENO);

        install_signal_handlers();
        arm_pdeathsig(initial_ppid);

        let stop = Arc::new((Mutex::new(false), Condvar::new()));
        let done = Arc::new(AtomicBool::new(false));
        let stop_bg = Arc::clone(&stop);
        let done_bg = Arc::clone(&done);
        let deadline = Instant::now() + timeout;

        let handle = thread::Builder::new()
            .name("mrd-check-guard".to_owned())
            .spawn(move || {
                watchdog_loop(
                    &stop_bg,
                    &done_bg,
                    initial_ppid,
                    grandparent_pid,
                    stdout_is_stream,
                    deadline,
                    timeout,
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
}

#[cfg(target_os = "linux")]
fn arm_pdeathsig(initial_ppid: libc::pid_t) {
    // SAFETY: `PR_SET_PDEATHSIG` takes a single signal integer; checking
    // `getppid()` immediately afterwards closes the race where our parent died
    // before `prctl` executed.
    unsafe {
        let _ = libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
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

fn watchdog_loop(
    stop: &(Mutex<bool>, Condvar),
    done: &AtomicBool,
    initial_ppid: libc::pid_t,
    grandparent_pid: Option<libc::pid_t>,
    stdout_is_stream: bool,
    deadline: Instant,
    timeout: Duration,
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
        // SAFETY: `getppid(2)` has no preconditions.
        let cur_ppid = unsafe { libc::getppid() };
        if initial_ppid > 1 && (cur_ppid == 1 || cur_ppid != initial_ppid) {
            abort_now(
                EXIT_HANGUP,
                "mrd check: aborted — parent process exited (reparented)",
            );
        }
        // Remote `ssh … bash -s` shape: `sshd` exits first, reparenting `bash -s`
        // to PID 1 while `bash -s` waits on us. Detect that transition directly.
        if let Some(gppid) = grandparent_pid
            && gppid > 1
            && matches!(parent_pid_of(initial_ppid), Some(1) | None)
        {
            abort_now(
                EXIT_HANGUP,
                "mrd check: aborted — ssh/parent session ended (parent shell reparented to pid 1)",
            );
        }
        if stdout_is_stream && stdout_peer_closed() {
            abort_now(
                EXIT_PIPE,
                "mrd check: aborted — stdout pipe closed by caller",
            );
        }
        if Instant::now() >= deadline {
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
