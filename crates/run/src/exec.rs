//! Bash process supervision (U6a, plan decision #21, review S3) — exec one
//! task block under the run plane's resource bracket:
//!
//! - **Scratch cwd:** the block runs in a caller-provided scratch directory,
//!   never in the tree. Tree access exists ONLY through the shim fd.
//! - **Own process group:** `setsid()` in the child pre-exec — the block and
//!   everything it spawns share one group the supervisor can address.
//! - **Wall-clock timeout (#21):** past the deadline the WHOLE group is
//!   `SIGKILL`ed; timeout is a distinct typed state ([`ExecStatus::TimedOut`]),
//!   never conflated with a nonzero exit.
//! - **Step-end reaping (S3):** the group is `SIGKILL`ed when the step ends —
//!   however it ends — so a background child cannot outlive the step and
//!   write into the post-step window.
//! - **Env hygiene:** the child env is cleared; it gets `PATH`, `LC_ALL=C`
//!   (framing lengths are BYTE counts — C locale makes bash `${#var}` count
//!   bytes), the contract-validated declared env, and `MD_EFFECT_FD`.
//! - **Shim fd:** fd 3 in the child, also named by `$MD_EFFECT_FD`. The
//!   captured stream feeds [`crate::shim::parse`]; capture past the storage
//!   cap sets the overflow flag and the stream fails closed downstream.
//!
//! This module supervises and captures — it applies nothing and parses
//! nothing. Two-phase apply is `dispatch_bash`; the protocol is `shim`; the
//! stdout consumer rides the [`exec_streaming`] seam (U8's `record` tee plugs
//! in there — stdout is DATA, not effects, ruling 7).

use std::collections::BTreeMap;
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::Path;
use std::process::{ChildStdout, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::shim::ShimStream;

/// The shim descriptor fd number in the child (also exported as
/// `$MD_EFFECT_FD`).
pub const SHIM_FD: i32 = 3;

/// The default wall-clock timeout when `.meridian.toml` does not configure
/// one (`[run] timeout_secs`).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_mins(5);

/// Storage cap for the captured shim stream. The reader keeps DRAINING past
/// the cap (the child is never deadlocked on a full pipe) but stops storing
/// and flags the overflow — the stream then fails closed at parse.
pub const MAX_SHIM_STREAM_BYTES: usize = 8 * 1024 * 1024;

/// One bash step to supervise. Identity-free: the invocation id, receipts,
/// and roots are the dispatcher's business.
#[derive(Debug)]
pub struct ExecSpec<'a> {
    /// The fence's inner source, run as `bash -c <source>`.
    pub source: &'a str,
    /// Contract-validated positional args (`$1`…).
    pub args: &'a [String],
    /// Contract-validated declared env — the ONLY caller env the child sees.
    pub env: &'a BTreeMap<String, String>,
    /// The scratch cwd (caller-created, out-of-tree).
    pub scratch: &'a Path,
    /// The wall-clock ceiling (#21).
    pub timeout: Duration,
}

/// How the step ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecStatus {
    /// The block exited on its own.
    Exited {
        /// Its exit code.
        code: i32,
    },
    /// The block was killed by a signal the supervisor did not send.
    Signaled {
        /// The signal number.
        signal: i32,
    },
    /// The wall-clock ceiling passed — the supervisor `SIGKILL`ed the group.
    /// A DISTINCT report state (#21), never folded into `Exited`.
    TimedOut {
        /// The ceiling that was enforced.
        limit: Duration,
    },
}

impl ExecStatus {
    /// True exactly for a clean `exit 0`.
    #[must_use]
    pub fn success(&self) -> bool {
        matches!(self, ExecStatus::Exited { code: 0 })
    }
}

/// What one supervised step produced: how it ended and everything it wrote.
#[derive(Debug)]
pub struct ExecResult {
    /// How the step ended.
    pub status: ExecStatus,
    /// Captured stdout ([`exec`] path; empty on the [`exec_streaming`] path,
    /// where the consumer owns the bytes).
    pub stdout: Vec<u8>,
    /// Captured stderr (diagnostics for the report).
    pub stderr: Vec<u8>,
    /// The captured shim-fd stream for [`crate::shim::parse`].
    pub shim: ShimStream,
}

/// Run one bash step, capturing stdout in memory — the plain seam.
///
/// # Errors
/// I/O failure spawning or wiring the child — nothing ran.
pub fn exec(spec: &ExecSpec<'_>) -> io::Result<ExecResult> {
    let (mut result, stdout) = exec_streaming(spec, |mut out| {
        let mut buf = Vec::new();
        let _ = out.read_to_end(&mut buf);
        buf
    })?;
    result.stdout = stdout;
    Ok(result)
}

/// Run one bash step with a caller-supplied stdout consumer — the U8 seam.
/// `stdout` runs on a supervisor thread WHILE the child runs (the live tee);
/// the step-end group SIGKILL closes the pipe, so a consumer that reads to
/// EOF always terminates. [`ExecResult::stdout`] is empty on this path — the
/// consumer's return value carries whatever it built (e.g. a sealed
/// `StdoutRecord`).
///
/// # Errors
/// I/O failure spawning or wiring the child — nothing ran and the consumer
/// was never called.
///
/// # Panics
/// If the `stdout` consumer panics, the panic propagates from its join.
pub fn exec_streaming<T, F>(spec: &ExecSpec<'_>, stdout: F) -> io::Result<(ExecResult, T)>
where
    F: FnOnce(ChildStdout) -> T + Send,
    T: Send,
{
    let (shim_read, shim_write) = io::pipe()?;

    let mut cmd = Command::new("bash");
    cmd.arg("-c")
        .arg(spec.source)
        .arg("mrd-task")
        .args(spec.args)
        .current_dir(spec.scratch)
        .env_clear()
        .env(
            "PATH",
            std::env::var_os("PATH").unwrap_or_else(|| "/usr/bin:/bin".into()),
        )
        .env("LC_ALL", "C")
        .envs(spec.env)
        .env("MD_EFFECT_FD", SHIM_FD.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let write_fd = shim_write.as_raw_fd();
    // SAFETY: pre_exec runs post-fork pre-exec in the child; setsid and dup2
    // are async-signal-safe. `write_fd` stays open in the parent until after
    // spawn (dup2 clears CLOEXEC on the duplicate, so fd 3 survives exec).
    unsafe {
        cmd.pre_exec(move || {
            if libc::setsid() < 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::dup2(write_fd, SHIM_FD) < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let mut child = cmd.spawn()?;
    // Close the parent's write end — EOF on the shim pipe then tracks the
    // GROUP's last holder, not the parent.
    drop(shim_write);

    let pid = i32::try_from(child.id()).map_err(|_| io::Error::other("pid out of range"))?;
    let stdout_pipe = child.stdout.take().expect("stdout piped");
    let stderr_pipe = child.stderr.take().expect("stderr piped");

    let (status, stderr, shim, consumed) = thread::scope(|s| {
        let stdout_h = s.spawn(move || stdout(stdout_pipe));
        let stderr_h = s.spawn(move || read_all(stderr_pipe));
        let shim_h = s.spawn(move || read_shim(shim_read));

        // Wall-clock supervision loop (#21): poll, and past the deadline
        // SIGKILL the whole group, then reap the leader.
        let deadline = Instant::now() + spec.timeout;
        let status = loop {
            match child.try_wait() {
                Err(e) => break Err(e),
                Ok(Some(st)) => break Ok(status_of(st)),
                Ok(None) => {}
            }
            if Instant::now() >= deadline {
                kill_group(pid);
                break match child.wait() {
                    Ok(_) => Ok(ExecStatus::TimedOut {
                        limit: spec.timeout,
                    }),
                    Err(e) => Err(e),
                };
            }
            thread::sleep(Duration::from_millis(10));
        };

        // Step-end reaping (S3): the group dies WITH the step, whatever the
        // exit path — a background child never writes into the post-step
        // window, and its death closes the pipes so the joins below cannot
        // hang. (Accepted micro-race: between the leader's reap and this kill
        // an emptied pgid could in principle be recycled; the window is
        // microseconds.)
        kill_group(pid);

        let consumed = stdout_h.join().expect("stdout consumer");
        let stderr = stderr_h.join().expect("stderr reader");
        let shim = shim_h.join().expect("shim reader");
        status.map(|st| (st, stderr, shim, consumed))
    })?;

    Ok((
        ExecResult {
            status,
            stdout: Vec::new(),
            stderr,
            shim,
        },
        consumed,
    ))
}

/// Resolve the effective wall-clock timeout for a workspace: `.meridian.toml`
/// `[run] timeout_secs`, else [`DEFAULT_TIMEOUT`]. A missing file or a file
/// without the key is the default; a present-but-malformed value refuses loud
/// (the same posture as the `[run.caps]` table).
///
/// # Errors
/// [`TimeoutConfigError`] — unreadable or malformed configuration.
pub fn configured_timeout(workspace_root: &Path) -> Result<Duration, TimeoutConfigError> {
    let path = workspace_root.join(".meridian.toml");
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(DEFAULT_TIMEOUT),
        Err(e) => {
            return Err(TimeoutConfigError {
                reason: format!("read {}: {e}", path.display()),
            });
        }
    };
    let value: toml::Value = text.parse().map_err(|e| TimeoutConfigError {
        reason: format!("parse: {e}"),
    })?;
    match value.get("run").and_then(|r| r.get("timeout_secs")) {
        None => Ok(DEFAULT_TIMEOUT),
        Some(toml::Value::Integer(secs)) if *secs > 0 => {
            Ok(Duration::from_secs(u64::try_from(*secs).unwrap_or(u64::MAX)))
        }
        Some(other) => Err(TimeoutConfigError {
            reason: format!("[run] timeout_secs must be a positive integer, got {other}"),
        }),
    }
}

/// `.meridian.toml` `[run] timeout_secs` is unreadable or malformed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeoutConfigError {
    /// What was wrong.
    pub reason: String,
}

impl std::fmt::Display for TimeoutConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, ".meridian.toml [run] timeout_secs: {}", self.reason)
    }
}

impl std::error::Error for TimeoutConfigError {}

/// SIGKILL every member of `pid`'s process group; an already-empty group
/// (ESRCH) is fine.
fn kill_group(pid: i32) {
    // SAFETY: plain kill(2) on a negative pgid; no memory is touched.
    unsafe {
        libc::kill(-pid, libc::SIGKILL);
    }
}

/// Map a reaped [`std::process::ExitStatus`].
fn status_of(st: std::process::ExitStatus) -> ExecStatus {
    match (st.code(), st.signal()) {
        (Some(code), _) => ExecStatus::Exited { code },
        (None, Some(signal)) => ExecStatus::Signaled { signal },
        // Unreachable on unix (a reaped status has one or the other); refuse
        // to invent an exit code.
        (None, None) => ExecStatus::Signaled { signal: 0 },
    }
}

/// Drain a pipe to a Vec until EOF (unbounded — stderr volume is time-bounded
/// by the #21 ceiling).
fn read_all<R: Read>(mut r: R) -> Vec<u8> {
    let mut out = Vec::new();
    let _ = r.read_to_end(&mut out);
    out
}

/// Drain the shim pipe until EOF: store up to [`MAX_SHIM_STREAM_BYTES`], keep
/// DRAINING past the cap (never deadlock the child on a full pipe), and flag
/// the overflow so the stream fails closed at parse.
fn read_shim<R: Read>(mut r: R) -> ShimStream {
    let mut stream = ShimStream::default();
    let mut buf = [0u8; 8192];
    loop {
        match r.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if stream.overflowed {
                    continue;
                }
                if stream.bytes.len() + n > MAX_SHIM_STREAM_BYTES {
                    stream.overflowed = true;
                    stream.bytes.clear();
                } else {
                    stream.bytes.extend_from_slice(&buf[..n]);
                }
            }
        }
    }
    stream
}
