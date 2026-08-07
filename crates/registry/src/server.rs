//! The unix-socket RPC server: socket placement, the singleton guard, the
//! accept loop, the reaper thread, and request dispatch.
//!
//! # Socket placement
//! The socket, state file, and singleton lock all live in a fixed per-user
//! directory outside any workspace: `<cache-root>/registry/` (the `cache`
//! crate's root, which the deny ceiling already refuses as a workspace, so the
//! socket can never sit inside a registerable tree). The directory is created
//! mode 0700; a **world-writable** registry directory is refused (a hostile
//! peer must not be able to swap the socket or state file).
//!
//! # Singleton
//! The daemon takes an exclusive `flock` on the registry directory (reusing
//! `cache::DrawerLock`) for its whole lifetime. A second daemon fails to
//! acquire it and refuses to start, so there is only ever one writer of the
//! state file — the single-writer invariant the atomic state write assumes.

use std::collections::BTreeMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use cache::DrawerLock;
use serde::Serialize;
use serde_json::{Map, Value};
use wire::{ErrorBody, ErrorCode, Op, ResponseBody, ResponsePayload, Root};
use wire_serve::rev::Rev;

use crate::engine::WorkspaceEngine;
use crate::protocol::{Request, Response};
use crate::registry::{PinOutcome, RegisterOutcome, Registry, ResolveOutcome};
use crate::state::StateStore;
use crate::{
    DEFAULT_IDLE_EXIT, DEFAULT_IDLE_REAP, DEFAULT_PREWARM_INTERVAL, DEFAULT_PREWARM_QUIET_MAX,
    DEFAULT_PUSH_WRITE_TIMEOUT, DEFAULT_REAP_INTERVAL, DEFAULT_SUB_IDLE_WRITE_TIMEOUT, now_secs,
};

/// How long the accept loop parks between non-blocking `accept` polls. Short
/// enough that shutdown is prompt, long enough not to spin a core.
const ACCEPT_POLL: Duration = Duration::from_millis(20);

/// The reaper's wake granularity: it sleeps in these steps so shutdown is
/// prompt even when the reap interval is an hour.
const REAP_TICK: Duration = Duration::from_millis(200);

/// The pre-warm thread's wake granularity: it sleeps in these steps so shutdown
/// stays prompt even when the pre-warm interval is configured long.
const PREWARM_TICK: Duration = Duration::from_millis(100);

/// The fixed subdirectory under the cache root that holds the socket, state
/// file, and singleton lock.
const REGISTRY_DIR: &str = "registry";
/// The RPC socket filename.
const SOCKET_NAME: &str = "daemon.sock";
/// The state file name.
const STATE_NAME: &str = "state.json";

/// Where and how a daemon runs. Construct with [`Config::resolve`] for the
/// production layout, or build the fields directly to place everything under a
/// test directory.
#[derive(Debug, Clone)]
pub struct Config {
    /// The unix socket path (its parent directory is the registry directory).
    pub socket_path: PathBuf,
    /// The state file path.
    pub state_path: PathBuf,
    /// The cache root under which drawer sentinels are written on register.
    pub cache_root: PathBuf,
    /// Idle-reap horizon (see [`DEFAULT_IDLE_REAP`]).
    pub idle_threshold: Duration,
    /// How often the reaper scans (see [`DEFAULT_REAP_INTERVAL`]).
    pub reap_interval: Duration,
    /// How often the pre-warm thread sweeps the warm workspaces (see
    /// [`DEFAULT_PREWARM_INTERVAL`]).
    pub prewarm_interval: Duration,
    /// The ceiling the pre-warm cadence backs off to while quiet (see
    /// [`DEFAULT_PREWARM_QUIET_MAX`]).
    pub prewarm_quiet_max: Duration,
    /// How long the daemon stays resident with no client request before asking
    /// its host process to exit (see [`DEFAULT_IDLE_EXIT`]). `None` disables
    /// idle exit, which is what an in-process test server wants.
    pub idle_exit: Option<Duration>,
    /// How long a push-plane write may block before the subscriber is dropped
    /// (see [`crate::DEFAULT_PUSH_WRITE_TIMEOUT`]).
    pub push_write_timeout: Duration,
    /// How long an armed sub may go with zero frames written before it is
    /// dropped (see [`crate::DEFAULT_SUB_IDLE_WRITE_TIMEOUT`], which carries the
    /// coupling constraint against D3's 30-minute client-side drain TTL).
    pub sub_idle_write_timeout: Duration,
    /// The build sha this daemon echoes as `hello.identity.build` on a v3
    /// session. `None` publishes no identity, which is distinct from
    /// `Some("unknown")`.
    ///
    /// Carried rather than read here: `MRD_BUILD_SHA` is baked into the `mrd`
    /// crate's compilation environment alone, so this crate cannot see it;
    /// `mrd daemon` supplies it (`docs/wire-contract.md`).
    pub build_sha: Option<String>,
}

impl Config {
    /// The production layout: socket, state, and lock under
    /// `<cache-root>/registry/`, drawers under `<cache-root>/`, with the
    /// default reap horizon and interval.
    #[must_use]
    pub fn for_cache_root(cache_root: PathBuf) -> Self {
        let dir = cache_root.join(REGISTRY_DIR);
        Config {
            socket_path: dir.join(SOCKET_NAME),
            state_path: dir.join(STATE_NAME),
            cache_root,
            idle_threshold: DEFAULT_IDLE_REAP,
            reap_interval: DEFAULT_REAP_INTERVAL,
            prewarm_interval: DEFAULT_PREWARM_INTERVAL,
            prewarm_quiet_max: DEFAULT_PREWARM_QUIET_MAX,
            idle_exit: Some(DEFAULT_IDLE_EXIT),
            push_write_timeout: DEFAULT_PUSH_WRITE_TIMEOUT,
            sub_idle_write_timeout: DEFAULT_SUB_IDLE_WRITE_TIMEOUT,
            // The layout cannot know the binary that will run it; the host
            // process supplies its own sha.
            build_sha: None,
        }
    }

    /// Resolve the production layout from the environment via
    /// [`cache::cache_root`].
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::NotFound`] when no cache root resolves (neither
    /// `XDG_CACHE_HOME` nor `HOME` is set) — a hard error, not a degrade: the
    /// daemon needs a stable per-user home for its socket and state.
    pub fn resolve() -> io::Result<Self> {
        Ok(Self::for_cache_root(cache::cache_root()?))
    }

    /// The registry directory (parent of the socket): where the state file,
    /// socket, and singleton lock live.
    fn registry_dir(&self) -> &Path {
        self.socket_path
            .parent()
            .unwrap_or_else(|| Path::new(REGISTRY_DIR))
    }
}

/// The default per-user RPC socket path, for a client that has no [`Config`].
///
/// # Errors
///
/// Returns [`io::ErrorKind::NotFound`] when no cache root resolves.
pub fn default_socket_path() -> io::Result<PathBuf> {
    Ok(cache::cache_root()?.join(REGISTRY_DIR).join(SOCKET_NAME))
}

/// A running daemon: the accept loop and reaper thread, the shared registry,
/// and the singleton lock. Drop or [`RunningServer::shutdown`] to stop it.
#[derive(Debug)]
pub struct RunningServer {
    shutdown: Arc<AtomicBool>,
    /// G11: raised by the reaper when the idle-exit horizon passes. A request,
    /// not the act — see [`RunningServer::idle_exit_requested`].
    exit_requested: Arc<AtomicBool>,
    accept: Option<JoinHandle<()>>,
    reaper: Option<JoinHandle<()>>,
    prewarm: Option<JoinHandle<()>>,
    registry: Arc<Registry>,
    socket_path: PathBuf,
    // The singleton flock, held for the daemon's whole lifetime; dropping it
    // releases the guard so a successor can start.
    _singleton: DrawerLock,
}

impl RunningServer {
    /// Start a daemon for `config`: prepare the registry directory, take the
    /// singleton lock, load state, bind the socket, and spawn the accept and
    /// reaper threads.
    ///
    /// # Errors
    ///
    /// Returns an error when the registry directory is world-writable, when
    /// another daemon already holds the singleton lock
    /// ([`io::ErrorKind::AlreadyExists`]), or when the socket cannot be bound.
    pub fn start(config: Config) -> io::Result<Self> {
        let dir = config.registry_dir().to_path_buf();
        prepare_dir(&dir)?;

        // Singleton: an exclusive flock on the registry directory. A held lock
        // means another daemon owns this socket — refuse rather than race the
        // state file.
        let Some(singleton) = DrawerLock::try_acquire(&dir)? else {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!(
                    "another meridian registry daemon is already running for {}",
                    dir.display()
                ),
            ));
        };

        let store = StateStore::new(config.state_path.clone());
        let entries = store.load();
        let registry = Arc::new(Registry::new(store, config.cache_root.clone(), entries));

        // We hold the singleton lock, so any existing socket is a stale leftover
        // from a crashed predecessor — remove it before binding.
        let _ = std::fs::remove_file(&config.socket_path);
        let listener = UnixListener::bind(&config.socket_path)?;
        listener.set_nonblocking(true)?;
        let _ =
            std::fs::set_permissions(&config.socket_path, std::fs::Permissions::from_mode(0o600));

        let shutdown = Arc::new(AtomicBool::new(false));
        let exit_requested = Arc::new(AtomicBool::new(false));
        let accept = spawn_accept(
            listener,
            registry.clone(),
            shutdown.clone(),
            PushDeadlines {
                write: config.push_write_timeout,
                idle_write: config.sub_idle_write_timeout,
            },
            config.build_sha.clone().map(Arc::from),
        );
        let reaper = spawn_reaper(
            registry.clone(),
            shutdown.clone(),
            config.idle_threshold,
            config.reap_interval,
            config.idle_exit,
            exit_requested.clone(),
        );
        let prewarm = spawn_prewarm(
            registry.clone(),
            shutdown.clone(),
            config.prewarm_interval,
            config.prewarm_quiet_max,
        );

        Ok(RunningServer {
            shutdown,
            exit_requested,
            accept: Some(accept),
            reaper: Some(reaper),
            prewarm: Some(prewarm),
            registry,
            socket_path: config.socket_path,
            _singleton: singleton,
        })
    }

    /// The bound socket path.
    #[must_use]
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// G11: has the daemon been idle past its exit horizon?
    ///
    /// The host process polls this and calls [`shutdown`](Self::shutdown)
    /// itself. The reaper cannot tear the server down from inside a reaper
    /// thread — `shutdown` joins that very thread — so the horizon is reported,
    /// never acted on, by the thread that observes it.
    #[must_use]
    pub fn idle_exit_requested(&self) -> bool {
        self.exit_requested.load(Ordering::SeqCst)
    }

    /// The shared registry, for in-process inspection and driving the reaper
    /// with an injected clock (tests, and the CLI daemon that owns the loop).
    #[must_use]
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// Stop the daemon: signal the threads, join them, flush the final state,
    /// and remove the socket. Idempotent via [`Drop`].
    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        if self.shutdown.swap(true, Ordering::SeqCst) {
            return; // already stopped
        }
        if let Some(handle) = self.accept.take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.reaper.take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.prewarm.take() {
            let _ = handle.join();
        }
        // Capture in-memory last_use bumps that resolve made without persisting.
        self.registry.flush();
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

impl Drop for RunningServer {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Create the registry directory mode 0700 and refuse a world-writable one.
fn prepare_dir(dir: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    let mode = std::fs::metadata(dir)?.permissions().mode();
    if mode & 0o002 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "registry directory {} is world-writable (mode {:o}); refusing to bind",
                dir.display(),
                mode & 0o777
            ),
        ));
    }
    Ok(())
}

/// The deadlines that keep a push subscription mortal — see [`push_loop`].
#[derive(Debug, Clone, Copy)]
struct PushDeadlines {
    /// [`Config::push_write_timeout`]: a blocked write on a busy workspace.
    write: Duration,
    /// [`Config::sub_idle_write_timeout`]: silence on a quiet one.
    idle_write: Duration,
}

/// Spawn the accept loop: non-blocking `accept`, one detached thread per
/// connection, polling the shutdown flag between idle polls.
fn spawn_accept(
    listener: UnixListener,
    registry: Arc<Registry>,
    shutdown: Arc<AtomicBool>,
    deadlines: PushDeadlines,
    build_sha: Option<Arc<str>>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let dispatch = |stream: UnixStream| {
            let registry = registry.clone();
            let shutdown = shutdown.clone();
            let build_sha = build_sha.clone();
            thread::Builder::new()
                .spawn(move || {
                    if let Err(e) = serve_conn(
                        &stream,
                        &registry,
                        &shutdown,
                        deadlines,
                        build_sha.as_deref(),
                    ) {
                        eprintln!("registry: connection error ({e})");
                    }
                })
                .map(drop)
        };
        accept_loop(&listener, &shutdown, dispatch);
    })
}

/// The accept loop proper, generic over how an accepted connection is
/// dispatched.
///
/// **Fault containment (R2/S2):** a dispatch failure (thread exhaustion) drops
/// that one connection and keeps accepting. `thread::spawn` panics on a failed
/// spawn, and that panic would unwind this loop while the daemon still holds
/// the listener and the singleton flock, so the spawn is fallible
/// (`thread::Builder`) and its error is a `continue`.
fn accept_loop(
    listener: &UnixListener,
    shutdown: &AtomicBool,
    dispatch: impl Fn(UnixStream) -> io::Result<()>,
) {
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                // The accepted stream inherits the listener's non-blocking
                // flag on BSD/macOS; per-connection I/O must block on reads,
                // so reset it explicitly.
                if let Err(e) = stream.set_nonblocking(false) {
                    eprintln!("registry: cannot set connection blocking ({e})");
                    continue;
                }
                if let Err(e) = dispatch(stream) {
                    eprintln!(
                        "registry: cannot spawn connection thread ({e}); dropping this connection — the daemon keeps accepting"
                    );
                }
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                if shutdown.load(Ordering::SeqCst) {
                    return;
                }
                thread::sleep(ACCEPT_POLL);
            }
            Err(e) => {
                if shutdown.load(Ordering::SeqCst) {
                    return;
                }
                eprintln!("registry: accept error ({e})");
                thread::sleep(ACCEPT_POLL);
            }
        }
    }
}

/// Spawn the reaper: wake every [`REAP_TICK`], and once `reap_interval` has
/// elapsed drop idle entries. Exits promptly on the shutdown flag.
fn spawn_reaper(
    registry: Arc<Registry>,
    shutdown: Arc<AtomicBool>,
    idle_threshold: Duration,
    reap_interval: Duration,
    idle_exit: Option<Duration>,
    exit_requested: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let threshold_secs = idle_threshold.as_secs();
        let mut elapsed = Duration::ZERO;
        while !shutdown.load(Ordering::SeqCst) {
            thread::sleep(REAP_TICK);
            elapsed += REAP_TICK;
            if elapsed < reap_interval {
                continue;
            }
            elapsed = Duration::ZERO;
            let reaped = registry.reap(now_secs(), threshold_secs);
            if !reaped.is_empty() {
                eprintln!("registry: idle-reaped {} workspace(s)", reaped.len());
            }
            // G11 idle exit. The reaper raises the flag only; the process's own
            // loop owns the teardown, because shutting the threads down from
            // inside one of them would join this thread to itself.
            if let Some(horizon) = idle_exit {
                // G11 liveness (R2/S3): an armed `sub` connection sends no
                // requests, so the request clock alone would exit out from
                // under it. Holding the clock open (rather than only skipping
                // the check) restarts the full horizon when the last subscriber
                // leaves; the push plane keeps the sub itself mortal.
                if registry.has_subscribers() {
                    registry.note_liveness();
                    continue;
                }
                let quiet_for = now_secs().saturating_sub(registry.last_request_secs());
                if quiet_for >= horizon.as_secs() {
                    eprintln!(
                        "registry: no client request in {quiet_for}s — shutting down (a client that needs the daemon auto-spawns one)"
                    );
                    exit_requested.store(true, Ordering::SeqCst);
                    return;
                }
            }
        }
    })
}

/// Pre-warm thread (P2): every `interval`, sweep warm workspaces so file
/// changes parse off the query path. Latency only; correctness is fingerprint.
/// Wakes every [`PREWARM_TICK`] for prompt shutdown.
fn spawn_prewarm(
    registry: Arc<Registry>,
    shutdown: Arc<AtomicBool>,
    interval: Duration,
    quiet_max: Duration,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut elapsed = Duration::ZERO;
        let mut delay = interval;
        let mut seen_requests = registry.request_count();
        while !shutdown.load(Ordering::SeqCst) {
            thread::sleep(PREWARM_TICK);
            elapsed += PREWARM_TICK;
            if elapsed < delay {
                continue;
            }
            elapsed = Duration::ZERO;
            let rebuilt = registry.prewarm();
            if !rebuilt.is_empty() {
                eprintln!(
                    "registry: pre-warmed {} changed workspace(s)",
                    rebuilt.len()
                );
            }
            // G11 quiet backoff: rebuild or client traffic restores base;
            // otherwise delay doubles toward the ceiling.
            let requests = registry.request_count();
            let quiet = rebuilt.is_empty() && requests == seen_requests;
            delay = next_prewarm_delay(delay, interval, quiet_max, quiet);
            seen_requests = requests;
        }
    })
}

/// G11 quiet-backoff step. Quiet ⇒ double toward `quiet_max`; else reset to
/// `base`. Clamped so a nonsense `quiet_max < base` never sweeps faster than base.
fn next_prewarm_delay(
    current: Duration,
    base: Duration,
    quiet_max: Duration,
    quiet: bool,
) -> Duration {
    if !quiet {
        return base;
    }
    current.saturating_mul(2).min(quiet_max.max(base)).max(base)
}

/// Serve one connection: the request-line loop over the accepted socket, then —
/// if a `sub` armed the connection — the push plane.
fn serve_conn(
    stream: &UnixStream,
    registry: &Registry,
    shutdown: &AtomicBool,
    deadlines: PushDeadlines,
    build_sha: Option<&str>,
) -> io::Result<()> {
    let reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream.try_clone()?;
    match serve_lines(registry, reader, &mut writer, build_sha)? {
        ServeOutcome::Eof => Ok(()),
        // U20b framing: the request loop is left for good; the connection is
        // push-only from here. Desync is unrepresentable. Dual planes ⇒ two
        // connections.
        ServeOutcome::Armed {
            workspace,
            rev,
            from_seq,
        } => push_loop(
            registry,
            &workspace,
            &mut writer,
            rev,
            from_seq,
            shutdown,
            deadlines,
        ),
    }
}

/// How the request-line loop of one connection ended.
#[derive(Debug)]
pub enum ServeOutcome {
    /// The client closed the stream with the connection still in request mode.
    Eof,
    /// An accepted `sub` armed the connection for push: the caller owns the
    /// push plane from here (`serve_conn` enters `push_loop`; a socketless
    /// caller has no push plane to enter).
    Armed {
        /// The bound workspace (canonical) the subscription reads.
        workspace: PathBuf,
        /// The contract rev the session negotiated at `hello`.
        rev: Rev,
        /// The subscription anchor the accepted `sub` declared.
        from_seq: u64,
    },
}

/// The request-line half of one connection: NDJSON in, one NDJSON response per
/// line (R1: one vocabulary on the socket). Families by `op`:
/// - **admin** (`ping`/`register`/`unregister`/`resolve_ws`/`list`) — daemon-
///   internal, absent from wire `caps`;
/// - **`hello`** — contract rev, pin, warm, bind connection (§4; subsumes deleted `attach`);
/// - **wire ops** — frozen contract from the bound workspace's warm engine.
///
/// Transport-generic (any `BufRead`/`Write`), so the frame layer is testable
/// without a socket — the daemon's socket is the one wire door (hosts ruling,
/// `docs/wire-contract.md` §3.3), and this is that door's line dialogue.
/// `serve_conn` drives it over the accepted `UnixStream`.
///
/// # Errors
/// Stream I/O failure only — never a content condition.
///
/// # Panics
/// Never in practice: an accepted `sub` proves a bound workspace (`dispatch_read`
/// refuses `sub` before `hello` binds one), so the `expect` below is an invariant.
pub fn serve_lines(
    registry: &Registry,
    input: impl BufRead,
    writer: &mut impl Write,
    build_sha: Option<&str>,
) -> io::Result<ServeOutcome> {
    // Bound workspace (canonical), set by `hello`. Per-connection.
    let mut attached: Option<PathBuf> = None;
    // Negotiated contract rev (one epoch, one rev). Default v2 = frozen path.
    let mut rev = Rev::V2;
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        // Activity clock: every frame counts, before dispatch, success or not.
        registry.note_request();
        // Set by an accepted `sub` only — see `push_loop`.
        let mut armed: Option<u64> = None;
        let out = handle_line(
            registry,
            &mut attached,
            &mut rev,
            &mut armed,
            &line,
            build_sha,
        );
        writer.write_all(out.as_bytes())?;
        writer.flush()?;
        if let Some(from_seq) = armed {
            let workspace = attached
                .take()
                .expect("an accepted sub proves a bound workspace");
            return Ok(ServeOutcome::Armed {
                workspace,
                rev,
                from_seq,
            });
        }
    }
    Ok(ServeOutcome::Eof)
}

/// An in-process registry for a socketless [`serve_lines`] — the same
/// construction [`RunningServer::start`] performs (directory prepared, state
/// loaded, drawers under the cache root) minus the singleton lock, the socket,
/// and the background threads. Wiring only: what it serves is exactly what the
/// daemon's socket serves.
///
/// # Errors
/// Returns an error when the registry directory cannot be prepared or is
/// world-writable.
pub fn in_process_registry(config: &Config) -> io::Result<Registry> {
    prepare_dir(config.registry_dir())?;
    let store = StateStore::new(config.state_path.clone());
    let entries = store.load();
    Ok(Registry::new(store, config.cache_root.clone(), entries))
}

/// How often a subscriber checks for undelivered frames.
///
/// Shorter than [`crate::ring::DETECT_CADENCE`]: noticing a frame (mutex + seq
/// compare) is cheaper than finding one (corpus fold), so N subscribers share
/// folds yet each delivers promptly.
const PUSH_TICK: Duration = Duration::from_millis(50);

/// Push channel: detect, deliver undelivered frames, repeat.
///
/// Ends on client disconnect (broken pipe is normal), daemon shutdown, or drop.
/// [`SubGuard`](crate::ring::SubGuard) keeps the reaper off this workspace for
/// the lifetime and releases on every exit path.
///
/// An armed sub holds the idle-exit clock open (R2/S3) and parks an OS thread,
/// so three signals keep it mortal:
/// - **peer closed** — the probe read ([`peer_closed`]) sees EOF within one
///   [`PUSH_TICK`], the only death signal a quiet workspace ever produces;
/// - **peer wedged** — `deadlines.write` bounds a blocked write once the
///   socket buffers fill, so a subscriber that stopped draining is dropped;
/// - **peer wedged on a quiet workspace** (R2b) — neither of the above can fire
///   there, so `deadlines.idle_write` drops a sub that has written zero frames
///   for that long.
fn push_loop(
    registry: &Registry,
    ws: &Path,
    writer: &mut UnixStream,
    rev: Rev,
    from_seq: u64,
    shutdown: &AtomicBool,
    deadlines: PushDeadlines,
) -> io::Result<()> {
    let ring = registry.ring(ws);
    // Subscribe before first detect — otherwise a reap can leave a silent hole.
    let _subscribed = ring.subscribe();
    // A timed-out write may leave a partial frame on the wire; the connection is
    // dropped right after, so the client reads a truncated line then EOF,
    // redials, and resyncs by root (§7.1).
    writer.set_write_timeout(Some(deadlines.write))?;
    let mut probe = writer.try_clone()?;
    // The probe read parks for the tick, so it replaces the sleep.
    probe.set_read_timeout(Some(PUSH_TICK))?;
    let ws_root = fs::WorkspaceRoot(ws.to_path_buf());
    let mut delivered = from_seq;
    // R2b: zero frames written for this long ⇒ drop. Any frame resets it.
    let mut last_write = Instant::now();
    while !shutdown.load(Ordering::SeqCst) {
        // Detection failure never ends the sub; log and retry next cycle.
        if let Err(e) = ring.detect(&ws_root) {
            eprintln!("registry: watch reconcile ({}): {e:?}", ws.display());
        }
        for frame in ring.frames_after(delivered) {
            wire_serve::ring::write_frame(writer, &frame, rev == Rev::V3)?;
            delivered = frame.delta.seq;
            last_write = Instant::now();
        }
        writer.flush()?;
        if peer_closed(&mut probe) {
            return Ok(());
        }
        // Drops between frames: nothing was written, so a live client redials
        // with its cursor and catches up from an empty ring.
        if last_write.elapsed() >= deadlines.idle_write {
            return Ok(());
        }
    }
    Ok(())
}

/// Has the push peer gone away? Parks up to the socket's read timeout, so this
/// is also the loop's tick.
///
/// The push plane is one-way by construction (`serve_conn` never returns to the
/// request loop), so a readable zero is EOF, not data — the only death signal a
/// quiet workspace produces. Bytes are a client speaking on a channel it does
/// not own: not a death signal, so the sub survives.
fn peer_closed(probe: &mut UnixStream) -> bool {
    let mut byte = [0u8; 1];
    match probe.read(&mut byte) {
        Ok(0) => true,
        Ok(_) => {
            thread::sleep(PUSH_TICK);
            false
        }
        Err(e) => !matches!(
            e.kind(),
            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
        ),
    }
}

/// Route one frame by `op` and render the `\n`-terminated response line.
///
/// Wire classes project per negotiated `rev`: v3 re-keys request before decode
/// and reshapes `root` → `fingerprint` on the way out. v2 serializes the typed
/// response (frozen path). Admin verbs are never projected.
fn handle_line(
    registry: &Registry,
    attached: &mut Option<PathBuf>,
    rev: &mut Rev,
    armed: &mut Option<u64>,
    line: &str,
    build_sha: Option<&str>,
) -> String {
    let value: Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(e) => {
            return to_line(&Response::Error {
                message: format!("malformed request ({e})"),
            });
        }
    };
    let Value::Object(mut obj) = value else {
        return to_line(&Response::Error {
            message: "request must be a JSON object".into(),
        });
    };
    // Owned tag so admin arm can consume `obj`. Base vocabulary for routing.
    let op = obj.get("op").and_then(Value::as_str).map(str::to_string);
    match op.as_deref() {
        Some("ping" | "register" | "unregister" | "resolve_ws" | "list") => {
            match serde_json::from_value::<Request>(Value::Object(obj)) {
                Ok(request) => to_line(&dispatch_admin(registry, request)),
                Err(e) => to_line(&Response::Error {
                    message: format!("malformed request ({e})"),
                }),
            }
        }
        // `hello` negotiates rev; response shaped for it. No U7 duration
        // (measure point is `dispatch_read` alone).
        Some("hello") => wire_line(&hello(registry, attached, rev, &obj, build_sha), *rev, None),
        _ => {
            // v3: re-key to v2 form for the strict decoder. v2 spelling / v2
            // connection pass through untouched.
            if *rev == Rev::V3 {
                wire_serve::rev::rename_request(&mut obj);
            }
            let (response, duration_us) =
                serve_wire(registry, attached.as_deref(), armed, &obj, *rev);
            wire_line(&response, *rev, duration_us)
        }
    }
}

/// Render one wire response line, shaped per negotiated rev.
/// v2: typed `wire::Response` (frozen). v3: project `root` → `fingerprint`,
/// attach `meta: {duration_us}` for dispatched ops (U7, engine work only).
fn wire_line(response: &wire::Response, rev: Rev, duration_us: Option<u64>) -> String {
    let mut out = if rev == Rev::V3 {
        let mut v = serde_json::to_value(response).expect("wire response serializes");
        wire_serve::rev::project_response(&mut v);
        if let Some(us) = duration_us {
            wire_serve::rev::attach_meta(&mut v, us);
        }
        serde_json::to_string(&v).expect("wire response serializes")
    } else {
        // Frozen v2 never grows a field: drop v3-additive extras here.
        let demoted = wire_serve::rev::demote_v2(response);
        serde_json::to_string(demoted.as_ref().unwrap_or(response))
            .expect("wire response serializes")
    };
    out.push('\n');
    out
}

/// Serialize a response value to one NDJSON line. The response types are plain
/// serde structs with no failible shapes, so serialization is an invariant.
fn to_line<T: Serialize>(value: &T) -> String {
    let mut out = serde_json::to_string(value).expect("daemon response serializes");
    out.push('\n');
    out
}

/// Map one admin request onto a registry operation and its response.
fn dispatch_admin(registry: &Registry, request: Request) -> Response {
    match request {
        Request::Ping => Response::Pong,
        Request::Resolve { cwd } => match registry.resolve(&cwd) {
            ResolveOutcome::Adopted(entry) => Response::Resolved { entry },
            ResolveOutcome::Miss => Response::Miss,
        },
        Request::Register { path } => match registry.register(&path) {
            RegisterOutcome::Registered(entry) => Response::Registered {
                entry,
                adopted: false,
            },
            RegisterOutcome::Adopted(entry) => Response::Registered {
                entry,
                adopted: true,
            },
            RegisterOutcome::Denied(reason) => Response::Denied {
                detail: reason.to_string(),
                reason,
            },
            RegisterOutcome::Error(message) => Response::Error { message },
        },
        Request::Unregister { path } => Response::Unregistered {
            removed: registry.unregister(&path),
        },
        Request::List => Response::Listed {
            entries: registry.entries(),
        },
    }
}

/// The resident daemon's `hello` server identity (v2 §3.2 server name).
const SERVER_NAME: &str = "meridian-daemon/0.1";

/// Caps the resident daemon serves (§3.2 discovery honesty). An op is in
/// `caps` or answers `unknown_op`, never both. `hello` is answered but not a
/// cap. Field-only caps name surfaces the arms honor; `splice.verdicts` is
/// §11.1, served `[]` (no pack loaded). `splice ∈ caps` ⇒ `node_rev` on every
/// `toc`/`cat`/`extract` node (shared read arms).
const CAPS: [&str; 16] = [
    "toc",
    "cat",
    "extract",
    "resolve",
    "resolve.content",
    "links",
    "links.require_root",
    "root",
    "diff",
    "splice",
    "splice.if_node_rev",
    "splice.if_root",
    "splice.dry",
    "splice.receipt",
    "splice.verdicts",
    // U20b push channel (§4.7). `sub` converts this connection to push-only.
    "sub",
];

/// Resident-engine handshake (§4): decode `hello` (unknown rev ⇒
/// `bad_request`), pin + warm + bind the connection. Subsumes deleted `attach`.
fn hello(
    registry: &Registry,
    attached: &mut Option<PathBuf>,
    rev: &mut Rev,
    obj: &Map<String, Value>,
    build_sha: Option<&str>,
) -> wire::Response {
    let id = obj.get("id").and_then(Value::as_u64);
    let body = match wire_serve::decode::decode(obj, *rev) {
        Ok(Op::Hello {
            contract,
            workspace,
            ..
        }) => {
            // Negotiate rev from decoded contract. Failed decode stays v2.
            *rev = Rev::from_contract(contract.as_deref());
            hello_body(registry, attached, workspace, *rev, build_sha)
        }
        // Only `hello` routes here; any other decoded op is a routing break.
        Ok(_) => Err(Box::new(ErrorBody::new(ErrorCode::Internal))),
        Err(error) => Err(error),
    };
    match body {
        Ok(body) => wire::Response {
            id,
            ok: true,
            payload: ResponsePayload::Body { body },
        },
        Err(error) => wire::Response {
            id,
            ok: false,
            payload: ResponsePayload::Error { error: *error },
        },
    }
}

/// Pin + warm + bind for a `hello` declared workspace.
///
/// Workspace-less hello: version handshake only (proto + caps). With a target:
/// exact pin via [`Registry::pin_declared`] (never cwd-shaped widening), warm
/// from disk, bind for subsequent ops. Response names the root that actually
/// bound (canonicalization may rewrite the declared spelling).
fn hello_body(
    registry: &Registry,
    attached: &mut Option<PathBuf>,
    workspace: Option<String>,
    rev: Rev,
    build_sha: Option<&str>,
) -> Result<ResponseBody, Box<ErrorBody>> {
    let (root, storage, bound) = match workspace {
        None => (None, None, None),
        Some(target) => match registry.pin_declared(Path::new(&target)) {
            PinOutcome::Pinned { workspace, drawer } => {
                registry
                    .warm_or_build(&workspace)
                    .map_err(|e| warm_err_to_wire(&e))?;
                let root = registry.with_engine(&workspace, |engine| engine.map(engine_root));
                let bound = workspace.to_string_lossy().into_owned();
                *attached = Some(workspace);
                (
                    root,
                    Some(drawer.to_string_lossy().into_owned()),
                    Some(bound),
                )
            }
            PinOutcome::Denied(reason) => {
                return Err(wire_serve::bad_request(format!(
                    "cannot pin `{target}` as a workspace: it is the {reason} (deny ceiling)"
                )));
            }
            PinOutcome::Error(message) => {
                let mut e = ErrorBody::new(ErrorCode::IoError);
                e.cause = Some(message);
                return Err(Box::new(e));
            }
        },
    };
    Ok(ResponseBody::Hello {
        proto: wire_serve::PROTO,
        server: SERVER_NAME.to_string(),
        caps: CAPS.iter().map(ToString::to_string).collect(),
        root,
        storage,
        workspace: bound,
        // v3-only, and only when this daemon was given a sha: v2 is the frozen
        // path and never grows the key, and no configured sha publishes no
        // identity rather than `unknown` (`docs/wire-contract.md`).
        identity: match (rev, build_sha) {
            (Rev::V3, Some(build)) => Some(wire::Identity {
                build: build.to_string(),
            }),
            _ => None,
        },
    })
}

/// Strict-decode one wire op and serve from the attached workspace. Returns
/// the response plus U7 duration: `Some(µs)` when the frame reached
/// `dispatch_read` (decode refusal carries none).
fn serve_wire(
    registry: &Registry,
    attached: Option<&Path>,
    armed: &mut Option<u64>,
    obj: &Map<String, Value>,
    rev: Rev,
) -> (wire::Response, Option<u64>) {
    let id = obj.get("id").and_then(Value::as_u64);
    let (body, duration_us) = match wire_serve::decode::decode(obj, rev) {
        Ok(op) => {
            // U7: dispatch call alone (after decode, before render).
            let started = Instant::now();
            let body = dispatch_read(registry, attached, armed, id, op, rev == Rev::V3);
            let duration_us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
            (body, Some(duration_us))
        }
        Err(error) => (Err(error), None),
    };
    let response = match body {
        Ok(body) => wire::Response {
            id,
            ok: true,
            payload: ResponsePayload::Body { body },
        },
        Err(error) => wire::Response {
            id,
            ok: false,
            payload: ResponsePayload::Error { error: *error },
        },
    };
    (response, duration_us)
}

/// Route one decoded wire op against the attached workspace.
/// Reads from the warm engine; `resolve` is cold (walk-plane corpus, §4.5);
/// `splice`/`create` write disk via the shared choke-point; `sub` arms push.
/// `hello` is intercepted upstream. `id` rides only into the splice receipt (§6.1).
#[expect(
    clippy::too_many_lines,
    reason = "exhaustive op router — one arm per wire op; splitting arms adds indirection, not insight"
)]
fn dispatch_read(
    registry: &Registry,
    attached: Option<&Path>,
    armed: &mut Option<u64>,
    id: Option<u64>,
    op: Op,
    v3: bool,
) -> Result<ResponseBody, Box<ErrorBody>> {
    // § A.5 mount-table discovery: machine-scoped, so it dispatches BEFORE
    // the binding guard — a workspace-less connection (a bare `hello`) may
    // call it; the caller discovery exists for is exactly the agent that
    // does not know a root yet. v3-only, like every post-freeze op.
    if matches!(op, Op::Mounts) {
        return if v3 {
            crate::mounts::serve(registry.mounts_cache())
        } else {
            Err(Box::new(ErrorBody::new(ErrorCode::UnknownOp)))
        };
    }
    let Some(ws) = attached else {
        return Err(wire_serve::bad_request(
            "no workspace bound — send `hello` with a `workspace` first",
        ));
    };
    match op {
        Op::Toc { path } => warm_engine_read(registry, ws, |engine| {
            let doc = engine
                .docs
                .get(&path.0)
                .ok_or_else(|| file_not_found(&path))?;
            Ok(wire_serve::read::toc(doc, &path, &engine_root(engine)))
        }),
        Op::Cat { path, sec } => warm_engine_read(registry, ws, |engine| {
            let doc = engine
                .docs
                .get(&path.0)
                .ok_or_else(|| file_not_found(&path))?;
            wire_serve::read::cat(doc, sec)
        }),
        Op::Extract { path, kinds } => warm_engine_read(registry, ws, |engine| {
            let doc = engine
                .docs
                .get(&path.0)
                .ok_or_else(|| file_not_found(&path))?;
            Ok(wire_serve::read::extract(doc, &path, kinds, v3))
        }),
        // Composed read — v3-only (§3.2); one warm-engine snapshot (D6).
        Op::Read {
            path,
            frag,
            sections,
            display_path,
            actor,
        } if v3 => composed_read_warm(
            registry,
            ws,
            &path,
            &wire_serve::read::ReadParams {
                frag,
                sections,
                display_path,
                actor,
            },
        ),
        Op::Links { path, require_root } => warm_engine_read(registry, ws, |engine| {
            let as_of = engine_root(engine);
            wire_serve::read::require_root_check(require_root.as_ref(), &as_of)?;
            let live = as_of.clone();
            wire_serve::read::links(&engine.index, &engine.docs, path.as_ref(), as_of, 0, || {
                Ok(live)
            })
        }),
        Op::Root => warm_engine_read(registry, ws, |engine| {
            Ok(ResponseBody::Root {
                root: engine_root(engine),
                seq: 0,
            })
        }),
        Op::Diff { from_root, to_root } => warm_engine_read(registry, ws, |engine| {
            // `diff` does not read the ring: same-root ⇒ empty; else
            // `root_unknown` → resync. `root.seq` stays 0 for the same reason —
            // both surfaces read the ring together or neither does.
            let current = engine_root(engine);
            if from_root == current && to_root == current {
                Ok(ResponseBody::Diff {
                    batches: Vec::new(),
                })
            } else {
                Err(Box::new(ErrorBody::new(ErrorCode::RootUnknown)))
            }
        }),
        Op::Resolve {
            from,
            r#ref,
            content,
        } => resolve_cold(ws, &from, &r#ref, content.unwrap_or(false)),
        // Write path: bare meridian-fs commit via shared choke-point.
        // No rule packs (`&[]` ⇒ `verdicts: []`). Writes disk; warm engine
        // rebuilds on next read (fingerprint moved). Numbered on the workspace ring.
        Op::Splice {
            path,
            actor,
            now,
            receipt,
            if_root,
            dry,
            force,
            edits,
            plan_edits,
            pin,
        } => {
            let ws_root = fs::WorkspaceRoot(ws.to_path_buf());
            let args = wire_serve::write::SpliceArgs {
                id,
                // U10: wire door ⇒ fingerprint-or-force (every wire door).
                origin: wire_serve::guard::Origin::Wire,
                path,
                actor,
                now,
                receipt,
                if_root,
                dry: dry.unwrap_or(false),
                force: force.unwrap_or(false),
                edits,
                plan_edits,
                pin,
            };
            // S7: session host — pin gate answers from the read-mint ledger.
            // Handle outside any engine borrow (H1).
            let mints = registry.read_mints(ws);
            // U20b: numbered producer on this workspace's ring. Sink inside
            // flock; advance after flock drops (allocator makes the gap safe).
            let ring = registry.ring(ws);
            let out = wire_serve::write::splice(&ws_root, Some(&*ring), &args, &[], Some(&mints))?;
            if let Some(frame) = out.committed {
                ring.advance(frame);
            }
            Ok(out.body)
        }
        // Birth op — v3-only; the shared guarded door (`write::create`).
        // Bare commit, numbered on the same ring as `splice`.
        Op::Create {
            path,
            body,
            actor,
            now,
            if_root,
            dry,
        } if v3 => {
            let ws_root = fs::WorkspaceRoot(ws.to_path_buf());
            let args = wire_serve::write::CreateArgs {
                id,
                path: path.clone(),
                body,
                actor,
                now,
                if_root,
                dry: dry.unwrap_or(false),
            };
            // Birth is a root advance — owes the chain a seq.
            let ring = registry.ring(ws);
            let out = wire_serve::write::create(&ws_root, Some(&*ring), &args, &[])?;
            if let Some(frame) = out.committed.clone() {
                ring.advance(frame);
            }
            Ok(wire_serve::write::create_response(path, &out))
        }
        // I4 def-conformance — v3-only, warm-engine doc, read-only.
        Op::CheckWrite {
            path,
            target,
            actor,
            now,
            edits,
        } if v3 => warm_engine_read(registry, ws, |engine| {
            let doc = engine
                .docs
                .get(&path.0)
                .ok_or_else(|| file_not_found(&path))?;
            Ok(wire_serve::check_write::check_write(
                doc, &target, &actor, &now, &edits,
            ))
        }),
        // U20b §4.7 push channel. Arm here; convert after ack (`serve_conn`).
        // S2: stands behind the same bind + deny ceiling as composed reads.
        // No actor: delta stream is not actor-scoped (identities/revs/spans only).
        // v2 `read`/`check_write`/`create` fall through to `unknown_op` below.
        Op::Sub { from_seq } => {
            let ring = registry.ring(ws);
            if !ring.can_anchor(from_seq) {
                let mut e = ErrorBody::new(ErrorCode::RootUnknown);
                e.message = Some(
                    "from_seq outside this epoch's retained history — catch up by diff-by-root (§7.1)"
                        .into(),
                );
                return Err(Box::new(e));
            }
            // Prime before ack: first reconcile adopts baseline silently;
            // ack-then-prime would swallow interim edits.
            let root = ring.prime(&fs::WorkspaceRoot(ws.to_path_buf()))?;
            // Armed only on success — refused `sub` leaves a request channel.
            *armed = Some(from_seq);
            // §4.7 ack: baseline root so first frame's `root_before` matches.
            Ok(ResponseBody::Root {
                root,
                seq: ring.seq(),
            })
        }
        // `Op::Mounts` is unreachable here (routed before the binding guard);
        // it rides this arm for exhaustiveness only.
        Op::Hello { .. }
        | Op::Read { .. }
        | Op::CheckWrite { .. }
        | Op::Create { .. }
        | Op::Mounts => Err(Box::new(ErrorBody::new(ErrorCode::UnknownOp))),
    }
}

/// The workspace's ambient root cursor = its warm engine's corpus content hash
/// (the reuse key), re-homed into the wire `Root` token.
fn engine_root(engine: &WorkspaceEngine) -> Root {
    Root(engine.at_fingerprint.0.clone())
}

/// Composed read over the warm engine: one borrow supplies doc, `file_rev`,
/// and ambient root (D6 one-snapshot). Session host hands the read-mint ledger
/// ([`Registry::read_mints`]) — taken before the engine borrow (H1).
fn composed_read_warm(
    registry: &Registry,
    ws: &Path,
    path: &wire::Path,
    params: &wire_serve::read::ReadParams,
) -> Result<ResponseBody, Box<ErrorBody>> {
    let mints = registry.read_mints(ws);
    warm_engine_read(registry, ws, |engine| {
        let doc = engine
            .docs
            .get(&path.0)
            .ok_or_else(|| file_not_found(path))?;
        // S10: claim-link colors from the pinned corpus, same warm snapshot (D6).
        let decorations =
            wire_serve::read::page_decorations(&engine.index, &engine.docs, path.0.as_str());
        wire_serve::read::composed_read(
            doc,
            path,
            &engine_root(engine),
            params,
            Some(&mints),
            &decorations,
        )
    })
}

/// Warm for `canonical` (idempotent), then serve through `f`. `None` after a
/// successful warm means idle-reap won the race — client retries.
fn warm_engine_read<R>(
    registry: &Registry,
    canonical: &Path,
    f: impl FnOnce(&WorkspaceEngine) -> Result<R, Box<ErrorBody>>,
) -> Result<R, Box<ErrorBody>> {
    registry
        .warm_or_build(canonical)
        .map_err(|e| warm_err_to_wire(&e))?;
    registry.with_engine(canonical, |engine| match engine {
        Some(engine) => f(engine),
        None => Err(Box::new(ErrorBody::new(ErrorCode::Internal))),
    })
}

/// §4.5 walk plane, served cold from a per-request walk corpus (not the
/// hash-domain warm engine).
fn resolve_cold(
    ws: &Path,
    from: &wire::Path,
    link: &str,
    want_content: bool,
) -> Result<ResponseBody, Box<ErrorBody>> {
    let root = fs::WorkspaceRoot(ws.to_path_buf());
    let rels = fs::walk(&root).map_err(|e| {
        let mut err = ErrorBody::new(ErrorCode::IoError);
        err.cause = Some(e.to_string());
        Box::new(err)
    })?;
    let mut index = model::CorpusIndex::new();
    let mut docs = BTreeMap::new();
    for rel in rels {
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        if let Ok(doc) = fs::load(&root, &rel) {
            index.insert(&rel_str, &doc);
            docs.insert(rel_str, doc);
        }
    }
    wire_serve::read::resolve(&index, &docs, from, link, want_content)
}

/// `file_not_found` for a wire read op whose `path` is not in the resident
/// corpus (the daemon's single-file reads are hash-domain-scoped).
fn file_not_found(path: &wire::Path) -> Box<ErrorBody> {
    let mut e = ErrorBody::new(ErrorCode::FileNotFound);
    e.path = Some(path.clone());
    Box::new(e)
}

/// Map a `warm_or_build` I/O failure onto its wire frame: a non-UTF-8 corpus
/// file is `invalid_utf8` (refused, never lossy-decoded); anything else (the
/// workspace is gone, an I/O error) carries its cause on `io_error`.
///
/// A warm failure is CORPUS-scoped — the caller asked for one file and the
/// whole rebuild refused — so the frame names its scope and its offending
/// member: `path` carries the poison member (the watch plane's `invalid_utf8`
/// precedent), `message` states the corpus scope and the condition. A bare
/// `invalid_utf8` here reads as "the file you asked for is corrupt", which is
/// false of that file and strands the caller (Law A-3c).
fn warm_err_to_wire(e: &io::Error) -> Box<ErrorBody> {
    if e.kind() == io::ErrorKind::InvalidData {
        let mut err = ErrorBody::new(ErrorCode::InvalidUtf8);
        if let Some(member) = fs::corpus_member_error(e) {
            err.path = Some(wire::Path(member.member.clone()));
        }
        err.message = Some(e.to_string());
        return Box::new(err);
    }
    let mut err = ErrorBody::new(ErrorCode::IoError);
    err.cause = Some(e.to_string());
    Box::new(err)
}

/// R2/S2 accept-loop fault containment, driven at the dispatch seam: a real
/// spawn failure cannot be provoked in-process without taking the test binary
/// down with it.
#[cfg(test)]
mod accept_containment_tests {
    use super::accept_loop;
    use std::io;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::thread;
    use std::time::{Duration, Instant};

    /// A dispatch that always fails must not end the loop: the daemon holds the
    /// listener and the singleton flock, so an accept loop that unwinds serves
    /// nothing and blocks every successor until idle-exit.
    #[test]
    fn a_dispatch_failure_drops_one_connection_and_keeps_accepting() {
        let tmp = tempfile::tempdir().unwrap();
        let socket = tmp.path().join("accept.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        listener.set_nonblocking(true).unwrap();

        let shutdown = Arc::new(AtomicBool::new(false));
        let seen = Arc::new(AtomicUsize::new(0));
        let loop_shutdown = shutdown.clone();
        let loop_seen = seen.clone();
        let accept = thread::spawn(move || {
            accept_loop(&listener, &loop_shutdown, |_stream: UnixStream| {
                loop_seen.fetch_add(1, Ordering::SeqCst);
                Err(io::Error::other("cannot spawn connection thread"))
            });
        });

        let deadline = Instant::now() + Duration::from_secs(5);
        for n in 1..=3 {
            UnixStream::connect(&socket)
                .unwrap_or_else(|e| panic!("connection {n} is accepted after {} failures: {e}", n - 1));
            while seen.load(Ordering::SeqCst) < n && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(
                seen.load(Ordering::SeqCst),
                n,
                "the loop reached dispatch for connection {n} — a failed dispatch \
                 contains to that one connection"
            );
        }

        shutdown.store(true, Ordering::SeqCst);
        accept.join().expect("the accept loop survived every dispatch failure");
    }
}

/// G11 quiet-backoff policy, pinned in isolation.
#[cfg(test)]
mod backoff_tests {
    use super::next_prewarm_delay;
    use std::time::Duration;

    const BASE: Duration = Duration::from_secs(1);
    // `Duration::from_days`/`from_mins` not const-stable at MSRV 1.96.
    #[allow(clippy::duration_suboptimal_units)]
    const CAP: Duration = Duration::from_secs(60);

    #[test]
    fn quiet_sweeps_double_the_delay_up_to_the_ceiling() {
        let mut delay = BASE;
        let mut seen = vec![delay];
        for _ in 0..10 {
            delay = next_prewarm_delay(delay, BASE, CAP, true);
            seen.push(delay);
        }
        assert_eq!(
            seen[..7],
            [1, 2, 4, 8, 16, 32, 60].map(Duration::from_secs),
            "the quiet cadence must double and then hold at the ceiling"
        );
        assert_eq!(delay, CAP, "and never climb past it");
    }

    #[test]
    fn any_activity_restores_the_base_cadence_in_one_step() {
        assert_eq!(
            next_prewarm_delay(CAP, BASE, CAP, false),
            BASE,
            "a sweep that found work drops straight back to base — no ramp-down"
        );
    }

    #[test]
    fn a_ceiling_below_the_base_still_never_sweeps_faster_than_base() {
        assert_eq!(
            next_prewarm_delay(BASE, BASE, Duration::from_millis(1), true),
            BASE
        );
    }
}
