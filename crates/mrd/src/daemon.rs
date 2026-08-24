//! `mrd daemon` — run the registry+engine server, and the client-side auto-spawn that starts it
//! detached on first use (decision 0002 §3, the watchman model). Two entry points share this
//! module: - [`run`] is the daemon body: start the server (which binds the socket and writes
//! the pidfile, in that contract's order — `registry::RunningServer`), and block on a signal
//! loop until SIGINT/SIGTERM.

use std::fs::OpenOptions;
use std::io::{self, Read as _, Seek as _};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use registry::{Config, RunningServer};

use crate::Fail;

/// Set by the signal handler; polled by the foreground loop.
static SIGNALLED: AtomicBool = AtomicBool::new(false);

/// Environment override for the binary [`spawn_detached`] launches as the daemon (default: the
/// running executable). A packager points it at a dedicated daemon binary; a test points it at
/// a nonexistent path to force the spawn-impossible degrade.
const DAEMON_BIN_ENV: &str = "MERIDIAN_DAEMON_BIN";

/// The extension of the detached daemon's voice file, beside the socket and the
/// pidfile that already key off the same stem ([`voice_path`]).
const VOICE_EXTENSION: &str = "log";

/// How much of the lane's new region [`voice_since`] may quote. The region can
/// be a whole timing firehose; a daemon's dying words are at its END.
const VOICE_QUOTE_BYTES: u64 = 4096;

/// How many lines of that region reach the quote — the last few, which is where
/// a startup refusal lands.
const VOICE_QUOTE_LINES: usize = 3;

/// A hard ceiling on the quoted text, because it rides ONE diagnostic line.
const VOICE_QUOTE_CHARS: usize = 600;

extern "C" fn on_signal(_sig: libc::c_int) {
    SIGNALLED.store(true, Ordering::SeqCst);
}

/// Install the SIGINT/SIGTERM handler that flips [`SIGNALLED`] so the foreground
/// loop tears the daemon down cleanly.
fn install_signal_handlers() {
    // SAFETY: `on_signal` only stores to a static AtomicBool — an
    // async-signal-safe operation (no allocation, no lock, no reentrancy).
    let handler = on_signal as *const () as libc::sighandler_t;
    unsafe {
        let _ = libc::signal(libc::SIGINT, handler);
        let _ = libc::signal(libc::SIGTERM, handler);
    }
}

/// Run `mrd daemon`: start the server (socket bound, pidfile written before
/// the first request is served — the server owns that order and its mirror on
/// shutdown), and block until a signal.
pub(crate) fn run() -> Result<(), Fail> {
    let mut config = Config::resolve()
        .map_err(|e| Fail::tool(format!("cannot resolve the daemon layout: {e}")))?;
    // Only this crate can read the baked sha (`build.rs` bakes it into this crate's
    // compilation environment), so the host hands it to the registry. The `unknown`
    // fallback rides through verbatim: publishing an identity, even an unnamed one,
    // is distinguishable from publishing none (`docs/wire-contract.md`).
    config.build_sha = Some(env!("MRD_BUILD_SHA").to_owned());
    install_signal_handlers();
    let server = RunningServer::start(config)
        .map_err(|e| Fail::tool(format!("cannot start the registry daemon: {e}")))?;
    eprintln!(
        "meridian registry daemon listening on {}",
        server.socket_path().display()
    );
    eprintln!("press Ctrl-C to stop");

    // Two ways out — a signal, or the idle-exit horizon the reaper watches. A detached daemon is
    // reparented to init, so without the second condition every isolated run leaks one forever.
    while !SIGNALLED.load(Ordering::SeqCst) && !server.idle_exit_requested() {
        thread::sleep(Duration::from_millis(200));
    }
    eprintln!("shutting down");
    server.shutdown();
    Ok(())
}

/// Auto-spawn the resident daemon DETACHED (decision 0002 §3): launch `mrd daemon` in a new
/// session with null stdin/stdout, so it survives this clients exit. Its stderr is a FILE it
/// can speak into ([`voice`]) — a detached daemon has no terminal, and a daemon nobody can
/// hear dies unheard. Returns as soon as the child is launched — the caller polls the socket
/// for readiness (a launched daemon that never binds is a caller-side timeout, then a degrade
/// that QUOTES that file: `engine::degrade_reason`).
pub(crate) fn spawn_detached() -> io::Result<()> {
    let bin = match std::env::var_os(DAEMON_BIN_ENV) {
        Some(path) => PathBuf::from(path),
        None => std::env::current_exe()?,
    };
    let mut command = Command::new(bin);
    command
        .arg("daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(voice());
    // SAFETY: `setsid(2)` is async-signal-safe (a single syscall, no allocation,
    // no lock) — the only work done in the forked child before exec.
    unsafe {
        command.pre_exec(|| {
            // A new session detaches the daemon from the client's controlling
            // terminal and process group; the leader has no controlling tty.
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    // Launch and detach: the child is reparented to init once this client exits,
    // so dropping the handle leaks no zombie in practice.
    command.spawn().map(drop)
}

/// Where a detached daemon speaks: beside the socket, `<socket-stem>.log` — the
/// same per-cache-root keying the pidfile already uses (`<socket-stem>.pid`), so
/// client and daemon derive it from one function and always agree.
///
/// # Errors
///
/// Returns [`io::ErrorKind::NotFound`] when no cache root resolves — the same
/// condition that leaves the client with no socket to dial.
pub(crate) fn voice_path() -> io::Result<PathBuf> {
    Ok(registry::default_socket_path()?.with_extension(VOICE_EXTENSION))
}

/// How many bytes the lane already holds — the mark a caller takes BEFORE
/// spawning, so [`voice_since`] reads back exactly what the child IT spawned
/// said, never a line an earlier daemon left on the same append-only file.
///
/// An absent lane (the cold case, and the common one) is `0`, which is also
/// what an unreadable one reports: the mark exists to bound a later read, and
/// bounding it at the start of the file is the honest answer when the length
/// cannot be established.
pub(crate) fn voice_mark() -> u64 {
    voice_path()
        .and_then(std::fs::metadata)
        .map(|meta| meta.len())
        .unwrap_or_default()
}

/// What the lane gained since `mark`, folded to ONE line fit to ride a
/// diagnostic — the dying words of the daemon this caller spawned.
///
/// `None` when the lane never opened, when it gained nothing, or when it cannot
/// be read back. That is a real distinction for the caller: **an empty region
/// is itself a finding** ("it died before it could speak"), and reporting it as
/// absent-text rather than as empty-text is what lets the caller say so.
///
/// Bounded twice, because the region can be a whole timing session: the last
/// [`VOICE_QUOTE_BYTES`] of it, then its last [`VOICE_QUOTE_LINES`] non-empty
/// lines, then [`VOICE_QUOTE_CHARS`]. A byte-bounded read can start mid-line,
/// so that first partial line is dropped rather than quoted as if it were a
/// whole one.
pub(crate) fn voice_since(mark: u64) -> Option<String> {
    let path = voice_path().ok()?;
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    if len <= mark {
        return None;
    }
    let start = mark.max(len.saturating_sub(VOICE_QUOTE_BYTES));
    file.seek(io::SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    let mut lines = text.lines();
    if start > mark {
        // The read began mid-file, so line one may be a fragment of a line the
        // mark already covered.
        lines.next();
    }
    let kept: Vec<&str> = lines
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    if kept.is_empty() {
        return None;
    }
    let quote = kept[kept.len().saturating_sub(VOICE_QUOTE_LINES)..].join(" / ");
    Some(one_line(&quote))
}

/// Text as it may appear on a diagnostic LINE. A raw newline would split the
/// line and leave the tail carrying no prefix at all; every other control
/// character is squeezed for the same reason. Also length-bounded — a daemon
/// that dies mid-dump must not own the terminal.
fn one_line(text: &str) -> String {
    let mut folded: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if folded.chars().count() > VOICE_QUOTE_CHARS {
        folded = folded.chars().take(VOICE_QUOTE_CHARS).collect();
        folded.push('…');
    }
    folded
}

/// Say — on the SPAWNING CLIENT's stderr, a lane somebody hears — that the
/// daemon is about to start with no lane of its own, and start it anyway.
///
/// Muting is always the degrade, never the failure: a daemon that answers
/// inaudibly is worth strictly more than no daemon, and the point of this whole
/// file is that silence must be ANNOUNCED rather than discovered.
fn mute(reason: &str) -> Stdio {
    eprintln!("mrd: {}", one_line(reason));
    Stdio::null()
}

/// The socket directory, created if this client got there before any daemon.
///
/// It is the daemon's own directory and the daemon wants it 0700
/// (`registry::server` § `prepare_dir`), so a client that creates it first must
/// not leave it looser than the daemon would. An existing directory is left
/// exactly as its owner set it.
fn prepare_voice_dir(dir: &Path) -> io::Result<()> {
    if dir.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(dir)?;
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    Ok(())
}

/// The child's stderr: [`voice_path`], opened append — ALWAYS.
///
/// A detached daemon has no terminal, so this was `Stdio::null()`
/// unconditionally, and then `Stdio::null()` unless the timing mode was on.
/// Both spellings shared one hole, and the timing mode was only its loudest
/// instance: **everything the daemon says goes to `/dev/null`**. It dies in
/// `RunningServer::start` — a panic, a layout that will not resolve, an
/// unbindable socket, a poisoned state file — and says its refusal into
/// nothing. The client's only symptom is the `SPAWN_READY_TIMEOUT` degrade, so
/// every startup failure alike presents as "5 seconds slower, then the
/// ephemeral answer" (card `auto-spawned-daemon-dies-silently`). The registry's
/// own operational diagnostics — three dozen `eprintln!` sites: accept errors,
/// connection errors, a failed state save, idle reaps — were inaudible on this
/// path for the same reason.
///
/// The degrade is right as a POLICY and is untouched. What changes is that it
/// now has something to quote (`engine::degrade_reason` reads this file back
/// through [`voice_since`]), so a run can say WHY it degraded.
///
/// What that costs a run that did not ask for the timing mode: one file beside
/// the socket, gaining a handful of lines per daemon LIFETIME (start, shutdown,
/// errors) — not per operation. The per-operation firehose is still gated on
/// `MRD_TIMING`, which is what that gate was actually protecting; this trades
/// the startup lines for the end of the silence, deliberately. The file is
/// nobody's to rotate: it is the operator's to read and to remove.
///
/// It is opened BEFORE the spawn, so a spawn that then fails (a bad
/// [`DAEMON_BIN_ENV`]) leaves an empty one with no daemon behind it. Named
/// rather than fixed: the alternative is handing the child a descriptor opened
/// after it exists, which is not a thing, and an empty file next to a daemon
/// that did not start is not a lie about anything.
///
/// A voice that cannot be opened is said out loud HERE, on the spawning
/// client's stderr, which is a lane somebody hears ([`mute`]): the daemon then
/// starts mute, and the diagnostic says so rather than letting the operator
/// read silence as "nothing went wrong".
fn voice() -> Stdio {
    let path = match voice_path() {
        Ok(path) => path,
        Err(error) => {
            return mute(&format!(
                "the daemon has nowhere to speak (no cache root resolves: {error}) — it starts \
                 MUTE, so a startup failure of its own will be silent."
            ));
        }
    };
    let opened = match path.parent() {
        Some(dir) => prepare_voice_dir(dir),
        None => Ok(()),
    }
    .and_then(|()| OpenOptions::new().create(true).append(true).open(&path));
    match opened {
        Ok(file) => Stdio::from(file),
        Err(error) => mute(&format!(
            "cannot open the daemon's lane `{}` ({error}) — it starts MUTE, so a startup failure \
             of its own will be silent.",
            path.display()
        )),
    }
}
