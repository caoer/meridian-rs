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
use crate::registry::{DOOR_COOKIE_TIMEOUT, PinOutcome, RegisterOutcome, Registry, ResolveOutcome};
use crate::ring::SubGuard;
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
/// The pidfile name, beside the socket. The singleton-winning daemon writes it
/// so an operator, the 0025 install pipeline (`pkill -TERM -F`), or a client's
/// kill attestation can signal the resident daemon without hunting the process
/// table; removed on graceful shutdown.
const PID_NAME: &str = "daemon.pid";

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

        // Pidfile ORDER is the contract (tests/daemon_pidfile.rs): after the
        // flock (only the winner claims the file — this also overwrites a
        // SIGKILLed predecessor's stale pid), before the accept loop spawns
        // (no pong can precede the write, so a client holding a pong always
        // reads the serving daemon's pid). The write itself stays advisory —
        // a daemon that cannot write its pidfile still serves (the socket is
        // the real liveness handle); log and carry on.
        let pid_file = config.socket_path.with_file_name(PID_NAME);
        if let Err(e) = write_pidfile(&pid_file) {
            eprintln!(
                "registry: cannot write pidfile {} ({e})",
                pid_file.display()
            );
        }

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
        // Persist every resident memo as a §6.5 checkpoint: this is the moment
        // the process — and the in-memory tree with it — dies, so it is the
        // one hook that makes the next start O(changes while down) instead of
        // a cold rebuild (merkle-spec §6.5).
        self.registry.save_checkpoints();
        // Socket first, pidfile second — the kill handle outlives the last
        // pong (the boot order's mirror), so no reader holding a fresh pong
        // finds the file already gone.
        let _ = std::fs::remove_file(&self.socket_path);
        let _ = std::fs::remove_file(self.socket_path.with_file_name(PID_NAME));
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

/// Write this process's pid to `path` atomically (same-directory temp +
/// rename), so no reader ever sees a half-written or empty file — `fs::write`
/// creates-then-fills, and the empty instant is exactly what a concurrent
/// pidfile reader would catch. The flock serializes writers, so the fixed
/// temp name cannot race itself.
fn write_pidfile(path: &Path) -> io::Result<()> {
    let tmp = path.with_extension("pid.tmp");
    std::fs::write(&tmp, format!("{}\n", std::process::id()))?;
    std::fs::rename(&tmp, path)
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
            guard,
        } => push_loop(
            &workspace,
            &mut writer,
            rev,
            from_seq,
            guard,
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
    /// caller has no push plane to enter and releases the claim by dropping
    /// the outcome).
    Armed {
        /// The bound workspace (canonical) the subscription reads.
        workspace: PathBuf,
        /// The contract rev the session negotiated at `hello`.
        rev: Rev,
        /// The resolved subscription anchor: the resumption cursor's seq, or
        /// the acked tip for a live subscribe (B-01).
        from_seq: u64,
        /// The live subscription claim, taken at arm time inside the dispatch
        /// — the reaper exemption is engaged before the ack renders, and every
        /// exit path (push loop end, error, drop) releases it.
        guard: SubGuard,
    },
}

/// An accepted `sub` in flight between its dispatch and the push plane: the
/// resolved anchor plus the subscription claim taken at arm time.
#[derive(Debug)]
struct ArmedSub {
    from_seq: u64,
    guard: SubGuard,
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
        // Set by an accepted `sub` only — carries the claim into `push_loop`.
        let mut armed: Option<ArmedSub> = None;
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
        if let Some(ArmedSub { from_seq, guard }) = armed {
            let workspace = attached
                .take()
                .expect("an accepted sub proves a bound workspace");
            return Ok(ServeOutcome::Armed {
                workspace,
                rev,
                from_seq,
                guard,
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
/// The [`SubGuard`](crate::ring::SubGuard) arrives with the accepted `sub`
/// (claimed at arm time, inside the dispatch — before the ack rendered), keeps
/// the reaper off this workspace for the lifetime, and releases on every exit
/// path.
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
#[expect(
    clippy::needless_pass_by_value,
    reason = "ownership is the mechanism: the push plane HOLDS the subscription \
              claim, and returning from this function is what releases it"
)]
fn push_loop(
    ws: &Path,
    writer: &mut UnixStream,
    rev: Rev,
    from_seq: u64,
    guard: SubGuard,
    shutdown: &AtomicBool,
    deadlines: PushDeadlines,
) -> io::Result<()> {
    // The guard's ring is the epoch the `sub` was acked on — and the claim has
    // protected it from the reaper since arm time, so it cannot have been
    // replaced by a fresh epoch in between.
    let ring = Arc::clone(guard.ring());
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
    armed: &mut Option<ArmedSub>,
    line: &str,
    build_sha: Option<&str>,
) -> String {
    // §3.1 raw-lexeme id law: classification and id validation happen on the
    // RAW `id` lexeme, BEFORE typed decode. A non-conforming lexeme is refused
    // `bad_request` with `id:null` plus the verbatim lexeme in `id_raw` —
    // never served, never reclassified as a notification. An unparseable line
    // is framing, not id law: it falls through to the malformed-request answer.
    if let Ok(transport::IdScan::BadId(lexeme)) = transport::scan_id(line) {
        let mut error = ErrorBody::new(ErrorCode::BadRequest);
        error.id_raw = Some(lexeme);
        return wire_line(
            &wire::Response {
                id: None,
                ok: false,
                payload: ResponsePayload::Error { error },
            },
            *rev,
            None,
        );
    }
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
        // § A.7 in-process script submission: routed here, not through
        // `serve_wire`, because its SUCCESS body is the run-plane ScriptTrace
        // (which embeds the §4.4 splice response verbatim) — not a
        // `ResponseBody` variant. Error frames leave through the ordinary
        // renderer inside. The v3 re-key runs first, as on every other op.
        Some("script") => {
            if *rev == Rev::V3 {
                wire_serve::rev::rename_request(&mut obj);
            }
            crate::script_op::serve_line(registry, attached.as_deref(), &obj, *rev)
        }
        // § A.8 page-task execution: routed here for the same reason as
        // `script` — its SUCCESS body embeds the run plane's own report
        // objects verbatim, not a `ResponseBody` variant.
        Some("run") => {
            if *rev == Rev::V3 {
                wire_serve::rev::rename_request(&mut obj);
            }
            crate::run_op::serve_line(registry, attached.as_deref(), &obj, *rev)
        }
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
pub(crate) fn wire_line(response: &wire::Response, rev: Rev, duration_us: Option<u64>) -> String {
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
///
/// **Derived, never hardcoded** (v1 stamp, `docs/release.md` §5.1). It used to
/// be a literal `meridian-daemon/0.1` independent of the workspace version, so
/// a release could announce a number the build did not carry. §3.2 makes the
/// string informational — there is no version sniffing, ever, and a caller
/// reads capability from `caps` — so deriving it breaks no promise; it kills
/// the drift class instead.
const SERVER_NAME: &str = concat!("meridian-daemon/", env!("CARGO_PKG_VERSION"));

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
/// `bad_request`), pin + bind the connection at config cost (§3.2 — hello
/// never walks the corpus). Subsumes deleted `attach`.
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

/// Pin + bind for a `hello` declared workspace — config-grade (§3.2, ruled
/// 2026-08-16, roots-hello-starved).
///
/// Workspace-less hello: version handshake only (proto + caps). With a target:
/// exact pin via [`Registry::pin_declared`] (never cwd-shaped widening),
/// domain CONFIG validated (an ambiguous or unreadable config refuses
/// `io_error{cause}` at the handshake, as it always has — the walk-free half
/// of the old inline warm), bind for subsequent ops. Response names the root
/// that actually bound (canonicalization may rewrite the declared spelling).
///
/// `hello` NEVER walks the corpus or builds the engine: the inline warm here
/// let one client's cold whole-corpus build hold every other client's `hello`
/// — and `mounts` behind it — past the face deadline (dogfood 2026-08-16).
/// `root` is the RESIDENT engine's fold when one exists and absent on a cold
/// workspace ("the engine may not have walked yet"); the first corpus read
/// pays the warm ([`warm_engine_read`]).
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
                fs::domain::Domain::load(&fs::WorkspaceRoot(workspace.clone()))
                    .map_err(|e| warm_err_to_wire(&e))?;
                // try_read, never a parking read: a rebuild insert queued
                // behind a long links closure walls ordinary readers, and
                // hello must answer through that too
                // (Registry::engine_snapshot_nowait). Absent-under-contention
                // is the same honest answer as cold.
                let root = registry
                    .engine_snapshot_nowait(&workspace)
                    .map(|engine| engine_root(&engine));
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
    armed: &mut Option<ArmedSub>,
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
    armed: &mut Option<ArmedSub>,
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
            let doc = doc_or_refusal(engine, ws, &path)?;
            Ok(wire_serve::read::toc(&doc, &path, &engine_root(engine)))
        }),
        Op::Cat { path, sec } => warm_engine_read(registry, ws, |engine| {
            let doc = doc_or_refusal(engine, ws, &path)?;
            wire_serve::read::cat(&doc, sec)
        }),
        Op::Extract { path, kinds } => warm_engine_read(registry, ws, |engine| {
            let doc = doc_or_refusal(engine, ws, &path)?;
            Ok(wire_serve::read::extract(&doc, &path, kinds, v3))
        }),
        // Composed read — v3-only (§3.2); one warm-engine snapshot (D6).
        Op::Read {
            path,
            toc,
            sections,
            display_path,
            actor,
        } if v3 => composed_read_warm(
            registry,
            ws,
            &path,
            &wire_serve::read::ReadParams {
                toc,
                sections,
                display_path,
                actor,
            },
        ),
        Op::Links { path, require_root } => warm_engine_read(registry, ws, |engine| {
            let as_of = engine_root(engine);
            wire_serve::read::require_root_check(require_root.as_ref(), &as_of)?;
            let live = as_of.clone();
            wire_serve::read::links(
                &fs::WorkspaceRoot(ws.to_path_buf()),
                &engine.index,
                &engine.docs,
                &engine.unserved,
                path.as_ref(),
                as_of,
                0,
                || Ok(live),
            )
        }),
        Op::Root => warm_engine_read(registry, ws, |engine| {
            // `tree_instance` absent for the same reason `seq` stays 0: this
            // op does not read the ring, and a ring surface it did not read
            // is not its fact to teach (B-01 identity rides the `sub` ack).
            Ok(ResponseBody::Root {
                root: engine_root(engine),
                seq: 0,
                tree_instance: None,
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
        // § A.10 pin-graph context assembly — v3-only, read-only, served from
        // the warm engine's projection through the shared walk computer.
        Op::Walk { path, down, depth } if v3 => warm_engine_read(registry, ws, |engine| {
            crate::walk_op::serve(engine, ws, &path, down.unwrap_or(false), depth)
        }),
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
                premises: Vec::new(),
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
            // Handle outside any engine borrow (H1). `foreign` hands the
            // cross-root pin gate the TARGET workspace's ledger (D-C): the
            // per-workspace map behind a closure, keyed by the same canonical
            // path the serving session's hello bound.
            let mints = registry.read_mints(ws);
            let foreign = |workspace: &Path| registry.read_mints(workspace);
            // U20b: numbered producer on this workspace's ring. The sink
            // allocates AND records inside the choke-point's flock
            // (`SeqSink::committed`), so no detect cycle can re-tell this
            // write as external between commit and record.
            let ring = registry.ring(ws);
            // Same memo the feed patches; the door observes it through the
            // §6.4 vouch at entry (`door_observation`) — a drained dirty set
            // alone never authorizes the overlay.
            let cache = registry.domain_cache(ws);
            let observe = || registry.door_observation(ws, &cache, DOOR_COOKIE_TIMEOUT);
            let out = wire_serve::write::splice_with_mints(
                &ws_root,
                Some(&*ring),
                &args,
                &[],
                wire_serve::write::Mints {
                    ambient: Some(&mints),
                    foreign: Some(&foreign),
                },
                Some(wire_serve::write::ResidentDoor {
                    cache: &cache,
                    observe: &observe,
                }),
            )?;
            Ok(out.body)
        }
        // The §4.4 SET form — v3-only (cap `splice.set`); the set choke-point.
        // One sealed commit, one frame on this workspace's ring.
        Op::SpliceSet {
            files,
            actor,
            now,
            receipt,
            if_root,
            dry,
            force,
        } if v3 => {
            let ws_root = fs::WorkspaceRoot(ws.to_path_buf());
            let args = wire_serve::write::SpliceSetArgs {
                premises: Vec::new(),
                id,
                files,
                origin: wire_serve::guard::Origin::Wire,
                actor,
                now,
                receipt,
                if_root,
                dry: dry.unwrap_or(false),
                force: force.unwrap_or(false),
            };
            // Sink records inside the flock (`SeqSink::committed`) — see `Op::Splice`.
            let ring = registry.ring(ws);
            let cache = registry.domain_cache(ws);
            let observe = || registry.door_observation(ws, &cache, DOOR_COOKIE_TIMEOUT);
            let out = wire_serve::write::splice_set_with_cache(
                &ws_root,
                Some(&*ring),
                &args,
                &[],
                Some(wire_serve::write::ResidentDoor {
                    cache: &cache,
                    observe: &observe,
                }),
            )?;
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
            // Birth is a root advance — owes the chain a seq. Sink records
            // inside the flock (`SeqSink::committed`) — see `Op::Splice`.
            let ring = registry.ring(ws);
            let cache = registry.domain_cache(ws);
            let observe = || registry.door_observation(ws, &cache, DOOR_COOKIE_TIMEOUT);
            let out = wire_serve::write::create_with_cache(
                &ws_root,
                Some(&*ring),
                &args,
                &[],
                Some(wire_serve::write::ResidentDoor {
                    cache: &cache,
                    observe: &observe,
                }),
            )?;
            Ok(wire_serve::write::create_response(path, &out))
        }
        // Death op — v3-only; the shared guarded door (`write::remove`, § A.3):
        // remove-what-you-read + the in-flock referential check. Bare commit,
        // numbered on the same ring as `splice`.
        Op::Remove {
            path,
            if_file_rev,
            actor,
            now,
            if_root,
            dry,
        } if v3 => {
            let ws_root = fs::WorkspaceRoot(ws.to_path_buf());
            let args = wire_serve::write::RemoveArgs {
                id,
                path: path.clone(),
                if_file_rev,
                actor,
                now,
                if_root,
                dry: dry.unwrap_or(false),
            };
            // Death is a root advance — owes the chain a seq. Sink records
            // inside the flock (`SeqSink::committed`) — see `Op::Splice`.
            let ring = registry.ring(ws);
            let cache = registry.domain_cache(ws);
            let observe = || registry.door_observation(ws, &cache, DOOR_COOKIE_TIMEOUT);
            let out = wire_serve::write::remove_with_cache(
                &ws_root,
                Some(&*ring),
                &args,
                &[],
                Some(wire_serve::write::ResidentDoor {
                    cache: &cache,
                    observe: &observe,
                }),
            )?;
            Ok(wire_serve::write::remove_response(path, &out))
        }
        // I4 def-conformance — v3-only, warm-engine doc, read-only.
        Op::CheckWrite {
            path,
            target,
            actor,
            now,
            edits,
        } if v3 => warm_engine_read(registry, ws, |engine| {
            let doc = doc_or_refusal(engine, ws, &path)?;
            Ok(wire_serve::check_write::check_write(
                &doc, &target, &actor, &now, &edits,
            ))
        }),
        // § A.11 corpus SQL — v3-only; the resident engine owns the cache
        // file and this daemon is its one append actor.
        Op::Sql { query } if v3 => {
            registry.warm_or_build(ws).map_err(|e| warm_err_to_wire(&e))?;
            crate::sql_op::serve(registry, ws, &query)
        }
        // U20b §4.7 push channel. Arm here; convert after ack (`serve_conn`).
        // S2: stands behind the same bind + deny ceiling as composed reads.
        // No actor: delta stream is not actor-scoped (identities/revs/spans only).
        // v2 `read`/`check_write`/`create` fall through to `unknown_op` below.
        Op::Sub {
            tree_instance,
            from_seq,
        } => {
            // Cursor-pair law first (B-01, §4.7): `from_seq` without
            // `tree_instance` anchors by number alone — the upgrade-required
            // refusal; instance without seq is half a cursor. Request-shape
            // facts, refused before any ring is touched.
            let cursor = match (tree_instance, from_seq) {
                (Some(tree_instance), Some(seq)) => Some(wire_serve::ring::Cursor {
                    tree_instance,
                    seq,
                }),
                (None, None) => None,
                (None, Some(_)) => {
                    let mut e = ErrorBody::new(ErrorCode::BadRequest);
                    e.message = Some(wire::sub_upgrade_teaching().into());
                    return Err(Box::new(e));
                }
                (Some(_), None) => {
                    let mut e = ErrorBody::new(ErrorCode::BadRequest);
                    e.message = Some(wire::sub_half_cursor_teaching().into());
                    return Err(Box::new(e));
                }
            };
            // The claim is taken HERE, at fetch, under the rings map lock
            // (`Registry::subscribe`) — linearized against the reaper's
            // decide-and-remove, so from this line the workspace is
            // reaper-exempt (U20b) and the ring cannot be orphaned under the
            // prime below or before the ack renders. Subscribing in
            // `push_loop` instead left a window between the acked `sub` and
            // the loop's own subscribe that the reaper could win (CI run
            // 31276217830, deterministic on the 2-core runner); a bare
            // fetch-then-claim left the same window one level down. A refused
            // `sub` drops the guard on return — the transient claim releases.
            let guard = registry.subscribe(ws);
            let ring = Arc::clone(guard.ring());
            if let Some(cursor) = &cursor {
                // Instance before sequence (B-01): the two refusals are
                // distinct facts, and a dead instance never consults seq.
                match ring.can_anchor(cursor) {
                    wire_serve::ring::Anchor::Anchored => {}
                    wire_serve::ring::Anchor::DeadInstance => {
                        let mut e = ErrorBody::new(ErrorCode::RootUnknown);
                        e.message = Some(wire::sub_dead_instance_teaching(
                            &cursor.tree_instance,
                            &ring.instance(),
                        ));
                        return Err(Box::new(e));
                    }
                    wire_serve::ring::Anchor::OutsideHistory => {
                        let mut e = ErrorBody::new(ErrorCode::RootUnknown);
                        e.message = Some(
                            "from_seq outside this tree instance's retained history — catch up by diff-by-root (§7.1)"
                                .into(),
                        );
                        return Err(Box::new(e));
                    }
                }
            }
            // Prime before ack: first reconcile adopts baseline silently;
            // ack-then-prime would swallow interim edits. The (root, seq)
            // pair is one instant's — a live sub anchors at exactly the tip
            // the acked baseline carries.
            let (root, seq) = ring.prime(&fs::WorkspaceRoot(ws.to_path_buf()))?;
            // A live subscribe anchors at the acked tip; a resumption at its
            // cursor — frames landed during prime are past it and deliver.
            let from_seq = cursor.map_or(seq, |c| c.seq);
            // Armed only on success — refused `sub` leaves a request channel.
            *armed = Some(ArmedSub { from_seq, guard });
            // §4.7 ack: baseline root so first frame's `root_before` matches,
            // plus the cursor identity the client resumes with (B-01).
            Ok(ResponseBody::Root {
                root,
                seq,
                tree_instance: Some(ring.instance()),
            })
        }
        // `Op::Mounts` is unreachable here (routed before the binding guard);
        // it rides this arm for exhaustiveness only.
        Op::Hello { .. }
        // § A.7 `script` and § A.8 `run` are served by their own
        // `serve_line`s, routed at `handle_line`; an op that reaches THIS
        // dispatch is a v2 session's — or a future mis-route's — and answers
        // the discovery-honesty word.
        | Op::Script { .. }
        | Op::Run { .. }
        | Op::Read { .. }
        | Op::CheckWrite { .. }
        | Op::Create { .. }
        | Op::Remove { .. }
        | Op::SpliceSet { .. }
        | Op::Walk { .. }
        | Op::Sql { .. }
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
        let doc = doc_or_refusal(engine, ws, path)?;
        // S10: claim-link colors from the pinned corpus, same warm snapshot (D6).
        let decorations =
            wire_serve::read::page_decorations(&engine.index, &engine.docs, path.0.as_str());
        wire_serve::read::composed_read(
            &doc,
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
///
/// **Every** read op goes through this, and the corpus pass it takes is
/// whole-corpus on purpose: a read is corpus-scoped, not file-scoped. A poison
/// member anywhere in the domain refuses a read of a healthy member, naming the
/// poison (Law A-3c, `registry/tests/corpus_refusal.rs`). Narrowing this to
/// "verify only the file being asked for" would serve a healthy answer out of a
/// corpus that cannot be parsed.
///
/// What the leaf memo changed is the COST of the pass, never its scope
/// (`docs/run-plane.md` § What an entry costs).
fn warm_engine_read<R>(
    registry: &Registry,
    canonical: &Path,
    f: impl FnOnce(&WorkspaceEngine) -> Result<R, Box<ErrorBody>>,
) -> Result<R, Box<ErrorBody>> {
    registry
        .warm_or_build(canonical)
        .map_err(|e| warm_err_to_wire(&e))?;
    // Test-only: park here when the gate is armed (see the field docs) —
    // the warm→borrow window the idle reaper can win.
    #[cfg(test)]
    {
        let gate = registry
            .pause_before_borrow
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some((arrived, release)) = gate {
            let _ = arrived.send(());
            let _ = release.recv();
        }
    }
    registry.with_engine(canonical, |engine| {
        if let Some(engine) = engine {
            f(engine)
        } else {
            // Idle-reap won the warm→borrow race: the engine this request
            // just warmed was reclaimed, not broken. `respawn` would teach
            // the client to tear down a healthy channel; the truthful §8
            // class is `retry` — the same request re-warms.
            let mut e = ErrorBody::new(ErrorCode::CorpusRace);
            e.message = Some(
                "the idle reaper reclaimed this workspace's warm engine between warm and \
                 serve — transient; the same request re-warms it"
                    .to_owned(),
            );
            Err(Box::new(e))
        }
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

/// `file_not_found` for a wire read op whose `path` names no file under the
/// workspace root. It means exactly that one thing (§12.1 addressability):
/// corpus residency is not the read admission test, so domain exclusion is
/// never a second reading of this miss — offering it taught the caller that an
/// out-of-domain file is unservable, the opposite of the law (dogfood
/// 2026-08-09, s10).
pub(crate) fn file_not_found(path: &wire::Path) -> Box<ErrorBody> {
    let mut e = ErrorBody::new(ErrorCode::FileNotFound);
    e.path = Some(path.clone());
    e.message = Some(format!(
        "file_not_found: no file at {} under the workspace root. Nothing was read and no rev \
         was minted. Fix: check the workspace-relative spelling (`results/f.md`, never \
         absolute). Being outside the hash domain is NOT this refusal: an ignored path is \
         served by explicit path like any other (wire-contract §12.1); its bytes simply do \
         not move the fingerprint.",
        path.0
    ));
    Box::new(e)
}

/// The document for `path` on this snapshot, or the refusal the miss means.
///
/// Resident corpus first (the warm parse); an UNSERVED member (in the hash
/// domain, not UTF-8 — node-rev-merkle-spec §3 per-file degradation) is the
/// per-file `invalid_utf8` naming itself. A path the corpus does not hold is
/// then loaded from disk: §12.1's hash domain ⊂ addressable domain holds at
/// every door, so an ignored `.md` reads exactly as a member reads — same
/// spans, same `file_rev` — and only a path with no file behind it refuses
/// `file_not_found`. Before this fallback the warm read door refused what the
/// write door committed on the same path (dogfood 2026-08-09, s10).
fn doc_or_refusal<'e>(
    engine: &'e WorkspaceEngine,
    ws: &Path,
    path: &wire::Path,
) -> Result<std::borrow::Cow<'e, model::Document>, Box<ErrorBody>> {
    if let Some(doc) = engine.docs.get(&path.0) {
        return Ok(std::borrow::Cow::Borrowed(doc));
    }
    if let Some(condition) = engine.unserved.get(&path.0) {
        // One mint for the §52 per-file refusal, shared with the links doors
        // (`wire_serve::read::unserved_refusal`).
        return Err(wire_serve::read::unserved_refusal(path, condition));
    }
    // Out-of-domain read: the same single-file load the write door and the
    // daemonless degrade already run, so all three agree on what a path serves.
    let root = fs::WorkspaceRoot(ws.to_path_buf());
    match wire_serve::load_doc(&root, path) {
        Ok(doc) => Ok(std::borrow::Cow::Owned(doc)),
        Err(e) if e.code == ErrorCode::FileNotFound => Err(file_not_found(path)),
        Err(e) => Err(e),
    }
}

/// Map a `warm_or_build` I/O failure onto its wire frame: `InvalidData` is
/// `invalid_utf8` (refused, never lossy-decoded); a member the walk listed
/// but the stat/read found gone (`NotFound` carrying the member) is the
/// transient delete window — `corpus_race`, retry class; anything else (the
/// workspace is gone, an I/O error) carries its cause on `io_error`.
///
/// A warm failure is CORPUS-scoped — the caller asked for one file and the
/// whole rebuild refused — so the frame names its scope and its offending
/// member when the error carries one ([`fs::corpus_member_error`]): a bare
/// refusal reads as "the file you asked for is corrupt", which is false of
/// that file and strands the caller (Law A-3c). A non-UTF-8 corpus MEMBER no
/// longer reaches here at all — it degrades per-file (`fs::build_corpus`
/// skips and reports it; [`doc_or_refusal`] mints its per-file refusal) — so
/// the `InvalidData` arm survives for the decode failures that really are
/// corpus-scoped, e.g. a domain config file that is itself not UTF-8.
fn warm_err_to_wire(e: &io::Error) -> Box<ErrorBody> {
    if e.kind() == io::ErrorKind::InvalidData {
        let mut err = ErrorBody::new(ErrorCode::InvalidUtf8);
        if let Some(member) = fs::corpus_member_error(e) {
            err.path = Some(wire::Path(member.member.clone()));
        }
        err.message = Some(e.to_string());
        return Box::new(err);
    }
    if e.kind() == io::ErrorKind::NotFound
        && let Some(member) = fs::corpus_member_error(e)
    {
        // The walk listed the member; the stat/read found it gone — a
        // concurrent delete won a benign per-round-trip race. The next
        // snapshot serves the post-delete corpus, so the truthful §8 class
        // is `retry`, not `env`; the frame keeps naming the member
        // (Law A-3c).
        let mut err = ErrorBody::new(ErrorCode::CorpusRace);
        err.path = Some(wire::Path(member.member.clone()));
        // The REASON only: this frame carries `recovery: retry` structurally,
        // so repeating the text face's recovery line here would teach the
        // same move twice, in two registers.
        err.message = Some(format!(
            "{} — transient; the same request snapshots the current corpus",
            member.reason()
        ));
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
            UnixStream::connect(&socket).unwrap_or_else(|e| {
                panic!("connection {n} is accepted after {} failures: {e}", n - 1)
            });
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
        accept
            .join()
            .expect("the accept loop survived every dispatch failure");
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

/// p2-recovery-class-truth red gates (contract §8): two read-path refusals
/// teach the wrong recovery class. The truthful class for both is `retry` —
/// a benign race with a cooperating actor (the idle reaper; a concurrent
/// delete), where the same request re-derives from the current world.
///
/// Deterministic via the `pause_before_borrow` seam (the PR #9
/// `pause_before_insert` precedent, disclosed): a real reap cannot be
/// scheduled into the warm→borrow gap from outside the process.
#[cfg(test)]
mod recovery_class_truth_tests {
    use super::warm_engine_read;
    use crate::registry::Registry;
    use crate::state::StateStore;
    use std::fs::{create_dir_all, write};
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, PoisonError};

    fn registry_in(home: &Path) -> Registry {
        let cache_root = home.join("cache");
        create_dir_all(&cache_root).unwrap();
        Registry::new(
            StateStore::new(home.join("state.json")),
            cache_root,
            Vec::new(),
        )
    }

    fn write_ws(home: &Path, files: &[(&str, &str)]) -> PathBuf {
        let ws = home.join("ws");
        create_dir_all(&ws).unwrap();
        for (rel, content) in files {
            let path = ws.join(rel);
            if let Some(parent) = path.parent() {
                create_dir_all(parent).unwrap();
            }
            write(path, content).unwrap();
        }
        ws
    }

    /// Defect 1 (sonnet P2, verdict § C5): the idle reaper dropping the
    /// engine between `warm_or_build` and `with_engine` must teach `retry` —
    /// the function's own doc says "idle-reap won the race — client
    /// retries" — never `internal`/`respawn`, which teaches the client to
    /// tear down a healthy channel.
    #[test]
    fn idle_reap_between_warm_and_borrow_teaches_retry_not_respawn() {
        let home = tempfile::tempdir().unwrap();
        let reg = Arc::new(registry_in(home.path()));
        let ws = write_ws(home.path(), &[("a.md", "# A\n")]);
        let canonical = workspace::canonicalize(&ws).unwrap();
        reg.register(&canonical);

        // Arm the one-shot warm→borrow gate for the read pass (thread A).
        let (arrived_tx, arrived) = std::sync::mpsc::channel();
        let (release, release_rx) = std::sync::mpsc::channel();
        *reg.pause_before_borrow
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some((arrived_tx, release_rx));

        // A: warms, then parks before its borrow.
        let a = {
            let reg = Arc::clone(&reg);
            let canonical = canonical.clone();
            std::thread::spawn(move || {
                warm_engine_read(&reg, &canonical, |engine| Ok(engine.docs.len()))
            })
        };
        arrived.recv().expect("thread A parked after its warm");

        // The reaper wins the window: entry + engine drop
        // (`reap_drops_the_warm_engine` proves the drop).
        let reaped = reg.reap(u64::MAX, 0);
        assert!(reaped.contains(&canonical), "the reaper took the engine");

        release
            .send(())
            .expect("thread A parked on the release gate");
        let err = a
            .join()
            .expect("thread A panicked")
            .expect_err("the borrow found no engine — the read refuses");

        assert_eq!(
            err.recovery,
            wire::Recovery::Retry,
            "a lost idle-reap race is transient by construction — the same \
             request re-warms; got {:?}/{:?}",
            err.code,
            err.recovery
        );
        assert_eq!(
            err.code,
            wire::ErrorCode::CorpusRace,
            "the code names the race, statically bound to retry"
        );
        let message = err.message.as_deref().unwrap_or_default();
        assert!(
            message.contains("reap"),
            "the message names the actor that won the race (the idle \
             reaper), so the client learns why retry succeeds: {message:?}"
        );
    }

    /// Defect 2 (opus P2-3, verdict § C5): a member deleted between the
    /// domain walk and its stat refuses the whole read as `io_error`/`env` —
    /// "the world outside is wrong; fix it" — where the truthful teaching is
    /// `retry`: the window is per-round-trip, and the next snapshot serves
    /// the post-delete corpus. The frame keeps naming the member (Law A-3c).
    ///
    /// Exercised at the one mapping seam (`warm_err_to_wire`) with the error
    /// exactly as `fs::member_identities` mints it (`CorpusMemberError`
    /// inside a `NotFound`): pausing the fs stat sweep itself would need a
    /// second seam in another card's named file.
    #[test]
    fn member_vanished_mid_snapshot_teaches_retry_not_env() {
        let e = std::io::Error::new(
            std::io::ErrorKind::NotFound,
            fs::CorpusMemberError {
                kind: std::io::ErrorKind::NotFound,
                member: "notes/x.md".to_owned(),
                condition: "vanished between the domain walk and its stat".to_owned(),
            },
        );
        let err = super::warm_err_to_wire(&e);
        assert_eq!(
            err.recovery,
            wire::Recovery::Retry,
            "a member the walk listed and the stat missed is a concurrent \
             delete — transient, not an environment fault; got {:?}/{:?}",
            err.code,
            err.recovery
        );
        assert_eq!(
            err.code,
            wire::ErrorCode::CorpusRace,
            "the code names the race, statically bound to retry"
        );
        assert_eq!(
            err.path.as_ref().map(|p| p.0.as_str()),
            Some("notes/x.md"),
            "the refusal still names the member (Law A-3c)"
        );
    }

    /// Control (green on base, guards the fix against over-widening): a
    /// member that exists but cannot be read is a real environment fault —
    /// retry never fixes permissions.
    #[test]
    fn member_unreadable_stays_env() {
        let e = std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            fs::CorpusMemberError {
                kind: std::io::ErrorKind::PermissionDenied,
                member: "notes/x.md".to_owned(),
                condition: "cannot be read (permission denied)".to_owned(),
            },
        );
        let err = super::warm_err_to_wire(&e);
        assert_eq!(err.code, wire::ErrorCode::IoError);
        assert_eq!(err.recovery, wire::Recovery::Env);
    }

    /// Control (green on base): a plain `NotFound` with no corpus member —
    /// the workspace itself is gone — stays `env`; nothing was racing.
    #[test]
    fn workspace_gone_stays_env() {
        let e = std::io::Error::new(std::io::ErrorKind::NotFound, "no such workspace");
        let err = super::warm_err_to_wire(&e);
        assert_eq!(err.code, wire::ErrorCode::IoError);
        assert_eq!(err.recovery, wire::Recovery::Env);
    }
}

/// U20b arm-time exemption gates: the reaper exemption engages when the `sub`
/// is accepted — inside the dispatch, before the ack renders, before the push
/// plane starts — and releases when the claim drops.
///
/// Closes the arm-to-convert window CI run 31276217830 lost deterministically
/// (2-core runner): `serve_lines` returned `Armed`, the client acted on its
/// ack, and `reap` won against `push_loop`'s own late subscribe. Holding the
/// `Armed` outcome without entering the push plane IS that window, held open
/// indefinitely — a stronger schedule than any yield storm, and deterministic.
#[cfg(test)]
mod arm_time_exemption_tests {
    use super::{ServeOutcome, serve_lines};
    use crate::registry::Registry;
    use crate::state::StateStore;
    use std::fs::{create_dir_all, write};
    use std::path::Path;
    use std::sync::Arc;

    fn registry_in(home: &Path) -> Registry {
        let cache_root = home.join("cache");
        create_dir_all(&cache_root).unwrap();
        Registry::new(
            StateStore::new(home.join("state.json")),
            cache_root,
            Vec::new(),
        )
    }

    /// `hello` (binds + registers) then a live `sub` (no cursor, B-01) — the
    /// accepted-sub line dialogue, socketless (`serve_lines` is the socket's
    /// own path).
    fn hello_then_sub(ws: &Path) -> String {
        format!(
            "{{\"op\":\"hello\",\"proto\":1,\"workspace\":{}}}\n{{\"op\":\"sub\"}}\n",
            serde_json::to_string(ws.to_str().unwrap()).unwrap()
        )
    }

    /// The exemption is live from the moment the `sub` is accepted: with the
    /// connection parked between its ack and the push plane (the old window),
    /// the widest-horizon reap takes nothing, and the ring keeps its epoch.
    /// Dropping the claim restores mortality — which also proves the survival
    /// above was the claim's doing, not a workspace the reaper never saw.
    #[test]
    fn an_accepted_sub_is_reaper_exempt_before_the_push_plane_starts() {
        let home = tempfile::tempdir().unwrap();
        let reg = registry_in(home.path());
        let ws = home.path().join("ws");
        create_dir_all(&ws).unwrap();
        write(ws.join("plan.md"), "# Goals\n\nship\n").unwrap();

        let mut out = Vec::new();
        let outcome =
            serve_lines(&reg, hello_then_sub(&ws).as_bytes(), &mut out, None).expect("serve_lines");
        let ServeOutcome::Armed {
            workspace, guard, ..
        } = outcome
        else {
            let dialogue = String::from_utf8_lossy(&out);
            panic!("sub was not accepted; dialogue: {dialogue}");
        };

        // The old window, held open: armed and acked, push plane not entered.
        let before = reg.ring(&workspace);
        let reaped = reg.reap(u64::MAX, 0);
        assert!(
            !reaped.contains(&workspace),
            "a subscribed workspace is not reaped — the claim is taken at arm \
             time, before the ack renders: {reaped:?}"
        );
        assert!(
            Arc::ptr_eq(&before, &reg.ring(&workspace)),
            "one workspace keeps ONE ring across a reap — a second ring would \
             fork the per-workspace seq counter §4.7 defines"
        );

        drop(guard);
        let reaped = reg.reap(u64::MAX, 0);
        assert!(
            reaped.contains(&workspace),
            "dropping the claim restores mortality (and proves the survival \
             above was the claim, not an unregistered workspace): {reaped:?}"
        );
    }
}

/// The §3.2 config-grade hello law (ruled 2026-08-16, roots-hello-starved):
/// one client's cold whole-corpus build must never hold another client's
/// `hello`. Deterministic via the disclosed `pause_before_insert` seam — the
/// build parks mid-flight (snapshot taken, insert pending), exactly the
/// window the old inline warm serialized every other hello behind.
#[cfg(test)]
mod hello_config_grade_tests {
    use super::{Config, RunningServer};
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    use std::path::Path;
    use std::sync::mpsc;
    use std::time::Duration;

    use serde_json::{Value, json};
    use tempfile::TempDir;

    /// Daemon config under `tmp` with horizons too large to interfere (the
    /// `engine_rpc` precedent).
    #[allow(clippy::duration_suboptimal_units)]
    fn test_config(tmp: &TempDir) -> Config {
        let forever = Duration::from_secs(365 * 24 * 60 * 60);
        let dir = tmp.path().join("registry");
        let mut config = Config::for_cache_root(tmp.path().join("cache"));
        config.socket_path = dir.join("daemon.sock");
        config.state_path = dir.join("state.json");
        config.idle_threshold = forever;
        config.reap_interval = forever;
        config.prewarm_interval = forever;
        config.prewarm_quiet_max = forever;
        config.idle_exit = None;
        config
    }

    struct Conn {
        writer: UnixStream,
        reader: BufReader<UnixStream>,
    }

    impl Conn {
        fn open(socket: &Path) -> Self {
            let stream = UnixStream::connect(socket).unwrap();
            Conn {
                writer: stream.try_clone().unwrap(),
                reader: BufReader::new(stream),
            }
        }

        /// A starved read fails loud at `timeout` instead of hanging the test.
        fn set_read_timeout(&self, timeout: Duration) {
            self.writer.set_read_timeout(Some(timeout)).unwrap();
        }

        fn call(&mut self, request: &Value) -> Value {
            let mut line = serde_json::to_string(request).unwrap();
            line.push('\n');
            self.writer.write_all(line.as_bytes()).unwrap();
            self.writer.flush().unwrap();
            let mut response = String::new();
            self.reader.read_line(&mut response).unwrap();
            serde_json::from_str(&response).unwrap()
        }

        fn hello(&mut self, ws: &Path) -> Value {
            self.call(&json!({
                "op": "hello",
                "proto": 1,
                "contract": "v3",
                "workspace": ws.to_str().unwrap(),
            }))
        }
    }

    #[test]
    fn a_parked_cold_build_never_holds_another_connections_hello() {
        let tmp = TempDir::new().unwrap();
        let ws = tmp.path().join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::write(ws.join("a.md"), "# A\n").unwrap();
        let server = RunningServer::start(test_config(&tmp)).unwrap();

        // Arm the one-shot pre-insert gate: the NEXT rebuild pass parks
        // between its disk snapshot and its `engines` insert.
        let (arrived_tx, arrived_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        *server.registry().pause_before_insert.lock().unwrap() = Some((arrived_tx, release_rx));

        // Connection A: hello binds at config cost; its `toc` pays the cold
        // build and parks at the gate.
        let socket = server.socket_path().to_path_buf();
        let ws_a = ws.clone();
        let a = std::thread::spawn(move || {
            let mut conn = Conn::open(&socket);
            let hi = conn.hello(&ws_a);
            assert_eq!(hi["ok"], json!(true), "A's hello binds: {hi}");
            conn.call(&json!({"op": "toc", "path": "a.md"}))
        });
        arrived_rx.recv().expect("the cold build reached the gate");

        // Connection B, same workspace, while A's build is parked: hello
        // answers — and cold (nothing inserted yet), so no `fingerprint`.
        // Before the config-grade law this assertion failed two ways: a
        // second inline warm produced a fingerprint, and a hello serialized
        // behind a real multi-minute build tripped the read timeout.
        let mut b = Conn::open(server.socket_path());
        b.set_read_timeout(Duration::from_secs(10));
        let hi = b.hello(&ws);
        assert_eq!(hi["ok"], json!(true), "B's hello answers mid-build: {hi}");
        assert!(
            !hi["body"].as_object().unwrap().contains_key("fingerprint"),
            "B's hello answered at config cost — it neither ran nor waited on \
             a corpus build: {hi}"
        );

        release_tx.send(()).expect("release the parked build");
        let toc = a.join().expect("connection A completes");
        assert_eq!(
            toc["ok"],
            json!(true),
            "A's read lands after release: {toc}"
        );

        server.shutdown();
    }

    /// The second seam the live probe caught (2026-08-16, sessions corpus):
    /// a `links` serve computes its whole map INSIDE the engines read lock,
    /// a rebuild insert queues behind it, and from that moment every
    /// ordinary reader parks behind the queued writer. hello reads the
    /// resident fold with `try_read` and answers through that wall.
    ///
    /// The long read is driven through the public borrow (`with_engine`
    /// parked in its closure — exactly the links shape); the writer is a
    /// second workspace's cold build parked on its `engines` insert.
    #[test]
    fn hello_answers_while_a_long_read_holds_the_engines_lock_and_a_writer_queues() {
        let tmp = TempDir::new().unwrap();
        let ws = tmp.path().join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::write(ws.join("a.md"), "# A\n").unwrap();
        let other = tmp.path().join("other");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("b.md"), "# B\n").unwrap();
        let server = RunningServer::start(test_config(&tmp)).unwrap();

        // Warm ws so a resident engine exists to read.
        let mut warmer = Conn::open(server.socket_path());
        assert_eq!(warmer.hello(&ws)["ok"], json!(true));
        assert_eq!(
            warmer.call(&json!({"op": "toc", "path": "a.md"}))["ok"],
            json!(true)
        );

        let registry = server.registry();
        let canonical = workspace::canonicalize(&ws).unwrap();
        let (r_in_tx, r_in_rx) = mpsc::channel::<()>();
        let (r_out_tx, r_out_rx) = mpsc::channel::<()>();
        std::thread::scope(|scope| {
            // Thread R: the long read — parks inside the engines read borrow.
            let r_canonical = canonical.clone();
            let r_handle = scope.spawn(move || {
                registry.with_engine(&r_canonical, |engine| {
                    assert!(engine.is_some(), "ws is warm");
                    r_in_tx.send(()).unwrap();
                    r_out_rx.recv().unwrap();
                });
            });
            r_in_rx.recv().expect("the long read holds the borrow");

            // Thread W: a cold build of ANOTHER workspace — its insert
            // queues on the engines write lock behind R.
            let w_other = other.clone();
            let w_handle = scope.spawn(move || {
                registry.warm_or_build(&w_other).unwrap();
            });
            // Let W finish its tiny build and reach the queued insert.
            std::thread::sleep(Duration::from_millis(300));

            // Connection B: hello answers through the wall. (Whether the
            // fold rides along is platform lock policy — under a queued
            // writer a try_read may refuse — so only the ANSWERING is
            // pinned, with the read timeout as the loud failure.)
            let mut b = Conn::open(server.socket_path());
            b.set_read_timeout(Duration::from_secs(10));
            let hi = b.hello(&ws);
            assert_eq!(
                hi["ok"],
                json!(true),
                "hello answers while a long read holds the engines lock and \
                 a writer queues: {hi}"
            );

            r_out_tx.send(()).expect("release the long read");
            r_handle.join().expect("long read completes");
            w_handle.join().expect("queued build completes");
        });

        server.shutdown();
    }
}
