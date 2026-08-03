//! The unix-socket RPC server: socket placement, the singleton guard, the
//! accept loop, the reaper thread, and request dispatch.
//!
//! # Socket placement
//! The socket, state file, and singleton lock all live in a fixed per-user
//! directory OUTSIDE any workspace: `<cache-root>/registry/` (the `cache`
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
use std::io::{self, BufRead, BufReader, Write};
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
use crate::{DEFAULT_IDLE_REAP, DEFAULT_PREWARM_INTERVAL, DEFAULT_REAP_INTERVAL, now_secs};

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
        }
    }

    /// Resolve the production layout from the environment via
    /// [`cache::cache_root`].
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::NotFound`] when no cache root resolves (neither
    /// `XDG_CACHE_HOME` nor `HOME` is set); a daemon needs a stable per-user
    /// home for its socket and state, so this is a hard error, not a degrade.
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
        let accept = spawn_accept(listener, registry.clone(), shutdown.clone());
        let reaper = spawn_reaper(
            registry.clone(),
            shutdown.clone(),
            config.idle_threshold,
            config.reap_interval,
        );
        let prewarm = spawn_prewarm(registry.clone(), shutdown.clone(), config.prewarm_interval);

        Ok(RunningServer {
            shutdown,
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

/// Spawn the accept loop: non-blocking `accept`, one detached thread per
/// connection, polling the shutdown flag between idle polls.
fn spawn_accept(
    listener: UnixListener,
    registry: Arc<Registry>,
    shutdown: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::spawn(move || {
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
                    let registry = registry.clone();
                    thread::spawn(move || {
                        if let Err(e) = serve_conn(&stream, &registry) {
                            eprintln!("registry: connection error ({e})");
                        }
                    });
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
    })
}

/// Spawn the reaper: wake every [`REAP_TICK`], and once `reap_interval` has
/// elapsed drop idle entries. Exits promptly on the shutdown flag.
fn spawn_reaper(
    registry: Arc<Registry>,
    shutdown: Arc<AtomicBool>,
    idle_threshold: Duration,
    reap_interval: Duration,
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
        }
    })
}

/// Spawn the pre-warm thread (decision 0002, P2): every `interval`, sweep the
/// warm workspaces so a file change pays its parse HERE — the watch event — not
/// on the next query. Latency only; correctness stays fingerprint
/// ([`Registry::prewarm`] reuses the warm engine when the corpus content hash is
/// unchanged, so a quiet sweep parses nothing). Wakes every [`PREWARM_TICK`] so
/// shutdown is prompt even when the interval is long; exits on the shutdown flag.
fn spawn_prewarm(
    registry: Arc<Registry>,
    shutdown: Arc<AtomicBool>,
    interval: Duration,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut elapsed = Duration::ZERO;
        while !shutdown.load(Ordering::SeqCst) {
            thread::sleep(PREWARM_TICK);
            elapsed += PREWARM_TICK;
            if elapsed < interval {
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
        }
    })
}

/// Serve one connection: read NDJSON frames until EOF, routing each to the
/// unified daemon surface and writing one NDJSON response per line (R1: ONE
/// vocabulary on the socket).
///
/// Three verb families share the socket, disambiguated by the `op` tag:
/// - **admin** (`ping`/`register`/`unregister`/`resolve_ws`/`list`) — the
///   workspace-registry verbs the `Client` drives, daemon-internal and absent
///   from any wire `caps` (so the 108da20a v3 proxy sees no change);
/// - **`hello`** — the resident-engine handshake (§4): assert the contract rev,
///   resolve the workspace-target, pin its storage, warm its resident engine,
///   and BIND this connection to it, so the wire read ops know which corpus to
///   serve. This is the U3 fold — `hello` subsumes the old daemon-internal
///   `attach` op, which is deleted (§5, no parallel paths);
/// - **wire read ops** (`toc`/`cat`/`extract`/`links`/`root`/`diff`/`resolve`)
///   — the frozen contract, strict-decoded and served from the bound
///   workspace's warm engine.
fn serve_conn(stream: &UnixStream, registry: &Registry) -> io::Result<()> {
    let reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream.try_clone()?;
    // The connection's bound workspace (canonical path), set by `hello`.
    // Per-connection, so concurrent clients target different workspaces.
    let mut attached: Option<PathBuf> = None;
    // The connection's negotiated contract rev (one epoch, one rev), set by
    // `hello` (docs/wire-contract-v3-amendment.md). Defaults to v2 so an
    // un-negotiated connection is byte-for-byte the frozen contract.
    let mut rev = Rev::V2;
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let out = handle_line(registry, &mut attached, &mut rev, &line);
        writer.write_all(out.as_bytes())?;
        writer.flush()?;
    }
    Ok(())
}

/// Route one frame by its `op` tag and render its response line (`\n`-terminated).
///
/// The wire classes (`hello` + the read/write ops) carry the v3 vocabulary
/// projection per the connection's negotiated `rev`: a v3 request is re-keyed to
/// its v2 form BEFORE decode, and the answer is re-shaped `root` → `fingerprint`
/// on the way out (`hello` caps + every frame class). A v2 connection serializes
/// the typed `wire::Response` directly — the frozen path, byte-identical. The
/// admin verbs are daemon-internal, absent from any wire `caps`, and NEVER
/// projected (the 108da20a v3 proxy sees no change on them).
fn handle_line(
    registry: &Registry,
    attached: &mut Option<PathBuf>,
    rev: &mut Rev,
    line: &str,
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
    // Read the tag as owned so the borrow of `obj` is dropped before the admin
    // arm consumes it. The tag is read in the base vocabulary: `hello` and the
    // admin verbs never carry a v3 spelling, and the v3 read/write ops route
    // through the `_` arm regardless (the `fingerprint` op tag included).
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
        // `hello` negotiates the rev; its OWN response is then shaped for it (the
        // caps + binding follow the negotiated vocabulary immediately). The
        // handshake is intercepted BEFORE the dispatch shell, so it carries no
        // U7 duration (the daemon measure point is `dispatch_read` alone).
        Some("hello") => wire_line(&hello(registry, attached, rev, &obj), *rev, None),
        _ => {
            // v3 connection: re-key the request into its v2 form so the strict
            // decoder + arms stay v2-only. A v2 spelling passes through untouched
            // (input leniency); a v2 connection is never re-keyed at all.
            if *rev == Rev::V3 {
                wire_serve::rev::rename_request(&mut obj);
            }
            let (response, duration_us) = serve_wire(registry, attached.as_deref(), &obj, *rev);
            wire_line(&response, *rev, duration_us)
        }
    }
}

/// Render one wire response line (`\n`-terminated), shaped per the negotiated
/// rev. v2 serializes the typed `wire::Response` directly — the frozen path,
/// byte-identical. v3 projects the serialized frame `root` → `fingerprint` at the
/// envelope layer (the typed layer never changes), then attaches the in-band
/// timing block `meta: {duration_us}` when this frame answered a dispatched op
/// (U7: the daemon measure point is the `dispatch_read` call — engine work only).
fn wire_line(response: &wire::Response, rev: Rev, duration_us: Option<u64>) -> String {
    let mut out = if rev == Rev::V3 {
        let mut v = serde_json::to_value(response).expect("wire response serializes");
        wire_serve::rev::project_response(&mut v);
        if let Some(us) = duration_us {
            wire_serve::rev::attach_meta(&mut v, us);
        }
        serde_json::to_string(&v).expect("wire response serializes")
    } else {
        serde_json::to_string(response).expect("wire response serializes")
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

/// The capability set the resident daemon serves (v2 §3.2 discovery honesty):
/// the wire read ops answered from the resident engine PLUS the write op
/// (`splice`, W1 — a BARE meridian-fs commit). NOT `sub` (P2), and `hello` itself
/// is answered but is not a cap — an op is in `caps` or answers `unknown_op`,
/// never both. Field-only caps (`resolve.content`, `links.require_root`, the
/// `splice.*` amendments) name the surfaces the arms honor; `splice.verdicts` is
/// the §11.1 surface, served `[]` (the daemon loads no pack yet). The S2/L22 law
/// holds: `splice ∈ caps` ⇒ `node_rev` rides every `toc`/`cat`/`extract` node,
/// which the shared read arms already emit.
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
    // V2 §Q2 the view-organ path forwarder (daemon-exclusive: the daemon is the
    // sole persistent builder, OD6). §3.2 discovery honesty — the daemon serves
    // it, so it advertises it; a sidecar neither advertises it nor serves it
    // (answers `daemon_only`).
    "view_path",
];

/// The resident-engine handshake (§4, U3): strict-decode the `hello` (asserting
/// the contract rev — an unknown declared rev is `bad_request` at decode), then
/// resolve the workspace-target, pin its storage, warm its resident engine, and
/// BIND this connection to it — one round trip. This is the fold that subsumes
/// the deleted `attach` op (bind + warm) and adds resolve + pin. Renders the
/// frozen wire `hello` body (echoing `id`).
fn hello(
    registry: &Registry,
    attached: &mut Option<PathBuf>,
    rev: &mut Rev,
    obj: &Map<String, Value>,
) -> wire::Response {
    let id = obj.get("id").and_then(Value::as_u64);
    let body = match wire_serve::decode::decode(obj, *rev) {
        Ok(Op::Hello {
            contract,
            workspace,
            ..
        }) => {
            // Negotiate the connection rev from the DECODED contract (decode
            // already refused an unknown rev loudly), so this hello response and
            // every frame after ride the negotiated vocabulary. A failed decode
            // never negotiates — the connection stays v2, its error serializes
            // on the frozen path.
            *rev = Rev::from_contract(contract.as_deref());
            hello_body(registry, attached, workspace)
        }
        // Only `op == "hello"` frames route here, and their decode is a
        // `Hello`; any other decoded op is a routing invariant break.
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

/// Pin + warm + bind the connection for a `hello`'s **declared** workspace.
///
/// A workspace-less hello is a pure version handshake: negotiate `proto` + list
/// `caps`, bind and pin nothing. With a target, the storage pin reuses the
/// registry's one canonicalize → deny-ceiling → sentinel path (risk R2, via
/// [`Registry::pin_declared`]); the warm reflects current disk (U1 residency);
/// the bind swaps the read ops' corpus source from the deleted `attach` to
/// `hello`.
///
/// `hello.workspace` is a DECLARATION, so it pins exactly
/// ([`Registry::pin_declared`], never the cwd-shaped
/// [`Registry::pin_for_cwd`]): the bound root can never widen to an enclosing
/// registered workspace. The response then NAMES the root that actually bound,
/// because canonicalization can still rewrite the declared spelling (symlinks,
/// on-disk case) — the ruling's "never silently" applied to the binding itself.
fn hello_body(
    registry: &Registry,
    attached: &mut Option<PathBuf>,
    workspace: Option<String>,
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
    })
}

/// Strict-decode one wire read op and serve it from the attached workspace's
/// resident engine, rendering the frozen `wire::Response` frame (echoing `id`).
///
/// Returns the response plus the U7 in-band duration: `Some(µs)` exactly when
/// the frame reached `dispatch_read` (the daemon measure point — engine work
/// only, success or refusal alike; a decode refusal carries none).
fn serve_wire(
    registry: &Registry,
    attached: Option<&Path>,
    obj: &Map<String, Value>,
    rev: Rev,
) -> (wire::Response, Option<u64>) {
    let id = obj.get("id").and_then(Value::as_u64);
    let (body, duration_us) = match wire_serve::decode::decode(obj, rev) {
        Ok(op) => {
            // U7 measure point: the dispatch call alone (after decode, before
            // the response render) — checked µs, never a lossy `as`.
            let started = Instant::now();
            let body = dispatch_read(registry, attached, id, op, rev == Rev::V3);
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

/// Route one decoded wire op to its arm against the attached workspace. Reads
/// (`toc`/`cat`/`extract`/`links`/`root`/`diff`) serve from the resident engine;
/// `resolve` (§4.5 walk plane) is served COLD from a per-request walk corpus — a
/// different corpus from the hash-domain warm engine. `splice` (W1) is the
/// resident WRITE path — a BARE meridian-fs commit through the shared choke-point,
/// reading + writing disk directly (independent of the warm engine; the next read
/// rebuilds). `hello` is intercepted upstream (the handshake binds the
/// connection); `sub` (P2) is not served yet and answers `unknown_op` (§3.2
/// discovery honesty). `id` rides only into the splice receipt line (§6.1).
#[expect(
    clippy::too_many_lines,
    reason = "exhaustive op router — one arm per wire op; splitting arms adds indirection, not insight"
)]
fn dispatch_read(
    registry: &Registry,
    attached: Option<&Path>,
    id: Option<u64>,
    op: Op,
    v3: bool,
) -> Result<ResponseBody, Box<ErrorBody>> {
    // V2 §Q2: `view_path` carries its own `cwd`, so it self-resolves the
    // workspace + drawer and needs NO `hello`-bound workspace — it is routed
    // before the bound-workspace guard. The daemon is the sole persistent
    // builder (OD6); `Registry::view_path` publishes under the per-workspace
    // publish mutex.
    if let Op::ViewPath { cwd, fresh } = &op {
        return registry.view_path(cwd, (*fresh).unwrap_or(false));
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
        // M1 U4a2 the composed read op — v3-ONLY (absent from the frozen v2
        // caps; §3.2 discovery honesty), served from the warm engine at ONE
        // snapshot (D6 atomicity: file_rev + root from the same borrow).
        Op::Read {
            path,
            mode,
            frag,
            sections,
            display_path,
            actor,
        } if v3 => composed_read_warm(
            registry,
            ws,
            &path,
            &wire_serve::read::ReadParams {
                mode,
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
            // The resident daemon holds no delta ring: the P2 pre-warm watcher is
            // latency-only (it re-warms the engine, emits no deltas), and the ring
            // lands only when subscriptions (`sub`) do. So a same-root diff is
            // truthfully empty; any other range is `root_unknown` → full resync
            // (degrade to re-derive, never to wrong data).
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
        // Routed before the bound-workspace guard above (it self-resolves `cwd`).
        Op::ViewPath { .. } => {
            unreachable!("view_path is handled before the bound-workspace guard")
        }
        // W1: the resident WRITE path — a BARE meridian-fs commit through the ONE
        // shared `splice → commit` choke-point (`wire_serve::write::splice`). The
        // daemon holds no rule packs (`&[]` ⇒ `verdicts: []`; pack admission is a
        // reserved, later unit) and no delta ring (`seq` 0; the emitted frame is
        // discarded — the ring lands with subscriptions, not the latency-only P2
        // pre-warm watcher). The commit reads + writes disk
        // directly, so the warm engine is untouched here; the change is durable in
        // the disk bytes, and the next read's `warm_or_build` rebuilds from them
        // (fingerprint moved), reflecting the write.
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
                // U10: a wire door — so it enforces fingerprint-or-force, as
                // EVERY wire door does. Not because of who is behind it: MCP is
                // the main agent client, never a trust plane of its own.
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
            // S7: this is the ONE host that holds a session, so it is the one
            // host whose pin gate can answer — the workspace's read-mint ledger
            // rides in beside the write. The handle is taken outside any engine
            // borrow (H1: the ledger is not the engine's, and the pin's own
            // write must not evaporate it).
            let mints = registry.read_mints(ws);
            wire_serve::write::splice(&ws_root, 0, &args, &[], Some(&mints)).map(|out| out.body)
        }
        // The BIRTH op — v3-ONLY, the resident twin of the sidecar's arm: the
        // SAME guarded door (`wire_serve::write::create`), the same forwarding
        // discipline, no second birth path. Like `splice` here it is a BARE
        // commit (`&[]` — the daemon loads no pack) reading + writing disk
        // directly; the daemon holds no delta ring, so `seq` is 0 and the
        // emitted birth frame is discarded. The newborn is durable in the disk
        // bytes, and the next read's `warm_or_build` rebuilds from them.
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
            wire_serve::write::create(&ws_root, 0, &args, &[])
                .map(|out| wire_serve::write::create_response(path, &out))
        }
        // M1 U8c the I4 def-conformance verdict — v3-ONLY, served from the
        // warm engine's doc (read-only: never a write path).
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
        // `hello` is intercepted upstream in `handle_line` (the handshake binds
        // the connection), so it never reaches here. `sub` = P2 is not served yet
        // — §3.2 discovery honesty: an op is served or answers `unknown_op`.
        // `read`/`check_write`/`create` land here only on a NON-v3 connection
        // (the guarded arms above take v3): absent from the frozen v2 caps →
        // `unknown_op`.
        Op::Hello { .. }
        | Op::Sub { .. }
        | Op::Read { .. }
        | Op::CheckWrite { .. }
        | Op::Create { .. } => Err(Box::new(ErrorBody::new(ErrorCode::UnknownOp))),
    }
}

/// The workspace's ambient root cursor = its warm engine's corpus content hash
/// (the reuse key), re-homed into the wire `Root` token.
fn engine_root(engine: &WorkspaceEngine) -> Root {
    Root(engine.at_fingerprint.0.clone())
}

/// The composed read (M1 U4a2) over the warm engine: one borrow supplies the
/// doc, its `file_rev`, and the ambient root — the D6 one-snapshot guarantee.
///
/// Stage-2 S6: this is the one host that holds a session, so it hands the arm
/// the workspace's read-mint ledger ([`Registry::read_mints`]) — a
/// daemon-derived actor's read mints a receipt there. The handle is taken
/// BEFORE the engine borrow and lives outside it: the ledger is not the
/// engine's, and no `warm_or_build` rebuild touches it (H1).
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
        // Stage-2 S10: this is also the one host that holds a CORPUS, and a
        // claim-link's color is a fact about the PINNED page, not the page
        // being read — so the decorations are built here, from the same warm
        // snapshot the read is served from (D6: one snapshot, one answer).
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

/// Warm the resident engine for `canonical` (idempotent — reflects current disk,
/// reuses the warm corpus when unchanged), then serve the borrowed engine
/// through `f`. `None` engine after a successful warm means an idle-reap evicted
/// it in the gap between warm and borrow — a transient the client retries.
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

/// wire §4.5 the walk plane, served COLD: build the walk-plane corpus (the
/// addressable SUPERSET, skip-broken — the app indexes nothing it cannot read)
/// from the attached workspace, then resolve. A different corpus from the
/// hash-domain warm engine, so it is never served from resident state.
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
fn warm_err_to_wire(e: &io::Error) -> Box<ErrorBody> {
    if e.kind() == io::ErrorKind::InvalidData {
        return Box::new(ErrorBody::new(ErrorCode::InvalidUtf8));
    }
    let mut err = ErrorBody::new(ErrorCode::IoError);
    err.cause = Some(e.to_string());
    Box::new(err)
}
