//! Bash process supervision (U6a, plan decision #21, review S3) — exec one
//! task block under the run plane's resource bracket:
//!
//! - **Invocation cwd (U16, requirements row E1):** the block runs where `mrd`
//!   runs — the process cwd is INHERITED, never relocated. The supervisor once
//!   chdir'd the child into the scratch directory; the ruling overturned that
//!   ("DO NOT CHANGE THE RUNNING PATH"), and `run/tests/exec.rs` asserts the
//!   new truth. Scratch stays minted by the caller as the artifact location —
//!   it is simply no longer the cwd.
//! - **Tree access is not confinement:** the cwd inheritance means a step CAN
//!   write into the workspace. Such a write is not tolerated and not merely
//!   reported — the U6b exec bracket detects it as [`crate::snapshot::Detection::OutOfBand`]
//!   and phase 2 REFUSES to converge: nothing the step emitted applies, and the
//!   ungoverned write is never rolled back (ruling 2). Governed change reaches
//!   the tree ONLY through the shim fd. Gates:
//!   `run/tests/dispatch_bash.rs::an_ungoverned_tree_write_refuses_phase2_with_the_delta_named`
//!   and `::a_project_root_relative_stray_write_refuses_convergence`.
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
//!   bytes), the contract-validated declared env, `MD_EFFECT_FD`, and
//!   `MERIDIAN_PROJECT_ROOT` (U16, plan decision P6 — the project root as a
//!   convenience for a step that no longer starts there; it grants nothing,
//!   and a write through it refuses exactly as above).
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
use std::path::{Path, PathBuf};
use std::process::{ChildStdout, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::shim::ShimStream;

/// The shim descriptor fd number in the child (also exported as
/// `$MD_EFFECT_FD`).
pub const SHIM_FD: i32 = 3;

/// The default wall-clock timeout when the root's declaration configures none
/// (`run.timeout_secs`). The safe value: a root raises it, never lowers into it.
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
    /// The caller-created out-of-tree scratch directory — the artifact
    /// location. NOT the cwd (U16): the step inherits the invocation cwd.
    pub scratch: &'a Path,
    /// The project root, exported to the step as `$MERIDIAN_PROJECT_ROOT`
    /// (P6). Convenience only — it confers no write authority, and a stray
    /// write under it refuses convergence (module docs).
    pub project_root: &'a Path,
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
        // No `current_dir`: U16 — the step runs where `mrd` runs.
        .env_clear()
        .env(
            "PATH",
            std::env::var_os("PATH").unwrap_or_else(|| "/usr/bin:/bin".into()),
        )
        .env("LC_ALL", "C")
        .envs(spec.env)
        .env("MD_EFFECT_FD", SHIM_FD.to_string())
        // After `envs`, so a declared key cannot shadow the plane's own.
        .env("MERIDIAN_PROJECT_ROOT", spec.project_root)
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

/// The frontmatter key carrying the wall-clock ceiling in the root's
/// declaration.
pub const TIMEOUT_KEY: &str = "run.timeout_secs";

/// Resolve the effective wall-clock timeout for a root: `run.timeout_secs` in
/// the root's own `MERIDIAN.md` declaration, else [`DEFAULT_TIMEOUT`].
///
/// This rode along with the caps rehoming because it was this crate's SECOND
/// reader of the retired marker. It is a RESOURCE ceiling, not a capability — it
/// gates no effect, the compiled default is the safe value, and the same writer
/// could already raise it under the retired plane. A move, not a grant.
///
/// `root` is `None` on the ladder's `CwdDefault`: no declaring root, so the
/// compiled default stands. An absent declaration or a declaration without the
/// key is likewise the default; a present-but-malformed value refuses loud (the
/// same posture as the convention table).
///
/// # Errors
/// [`TimeoutConfigError`] — unreadable declaration or malformed value.
pub fn configured_timeout(root: Option<&Path>) -> Result<Duration, TimeoutConfigError> {
    let Some(root) = root else {
        return Ok(DEFAULT_TIMEOUT);
    };
    let declaration = match config::mount::read_root_declaration(root) {
        Ok(d) => d,
        Err(config::mount::DeclarationFault::Absent) => return Ok(DEFAULT_TIMEOUT),
        Err(config::mount::DeclarationFault::Unreadable(reason)) => {
            return Err(TimeoutConfigError {
                path: root.join(config::mount::DECLARATION_FILENAME),
                reason,
            });
        }
    };
    let Some(map) = crate::address::frontmatter(&declaration.document) else {
        return Ok(DEFAULT_TIMEOUT);
    };
    let Some((_, raw)) = map.0.iter().find(|(k, _)| k == TIMEOUT_KEY) else {
        return Ok(DEFAULT_TIMEOUT);
    };
    match raw.trim().trim_matches(['"', '\'']).parse::<u64>() {
        Ok(secs) if secs > 0 => Ok(Duration::from_secs(secs)),
        _ => Err(TimeoutConfigError {
            path: root.join(config::mount::DECLARATION_FILENAME),
            reason: format!("`{TIMEOUT_KEY}` must be a positive integer, got `{raw}`"),
        }),
    }
}

/// The root declaration's `run.timeout_secs` is unreadable or malformed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeoutConfigError {
    /// The declaration the fault was read from.
    pub path: PathBuf,
    /// What was wrong.
    pub reason: String,
}

impl std::fmt::Display for TimeoutConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "refused: {}: {}", self.path.display(), self.reason)
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
