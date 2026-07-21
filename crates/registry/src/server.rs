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
use std::time::Duration;

use cache::DrawerLock;
use serde::Serialize;
use serde_json::{Map, Value};
use wire::{ErrorBody, ErrorCode, Op, ResponseBody, ResponsePayload, Root};

use crate::engine::WorkspaceEngine;
use crate::protocol::{Request, Response};
use crate::registry::{PinOutcome, RegisterOutcome, Registry, ResolveOutcome};
use crate::state::StateStore;
use crate::{DEFAULT_IDLE_REAP, DEFAULT_REAP_INTERVAL, now_secs};

/// How long the accept loop parks between non-blocking `accept` polls. Short
/// enough that shutdown is prompt, long enough not to spin a core.
const ACCEPT_POLL: Duration = Duration::from_millis(20);

/// The reaper's wake granularity: it sleeps in these steps so shutdown is
/// prompt even when the reap interval is an hour.
const REAP_TICK: Duration = Duration::from_millis(200);

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

        Ok(RunningServer {
            shutdown,
            accept: Some(accept),
            reaper: Some(reaper),
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
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let out = handle_line(registry, &mut attached, &line);
        writer.write_all(out.as_bytes())?;
        writer.flush()?;
    }
    Ok(())
}

/// Route one frame by its `op` tag and render its response line (`\n`-terminated).
fn handle_line(registry: &Registry, attached: &mut Option<PathBuf>, line: &str) -> String {
    let value: Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(e) => {
            return to_line(&Response::Error {
                message: format!("malformed request ({e})"),
            });
        }
    };
    let Value::Object(obj) = value else {
        return to_line(&Response::Error {
            message: "request must be a JSON object".into(),
        });
    };
    // Read the tag as owned so the borrow of `obj` is dropped before the admin
    // arm consumes it.
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
        Some("hello") => to_line(&hello(registry, attached, &obj)),
        _ => to_line(&serve_wire(registry, attached.as_deref(), &obj)),
    }
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
/// exactly the wire read ops answered from the resident engine. NOT `splice`
/// (W1), NOT `sub` (P2), and `hello` itself is answered but is not a cap — an op
/// is in `caps` or answers `unknown_op`, never both. Field-only caps
/// (`resolve.content`, `links.require_root`) name the amendments the arms honor.
const CAPS: [&str; 9] = [
    "toc",
    "cat",
    "extract",
    "resolve",
    "resolve.content",
    "links",
    "links.require_root",
    "root",
    "diff",
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
    obj: &Map<String, Value>,
) -> wire::Response {
    let id = obj.get("id").and_then(Value::as_u64);
    let body = match wire_serve::decode::decode(obj) {
        Ok(Op::Hello { workspace, .. }) => hello_body(registry, attached, workspace),
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

/// Resolve + pin + warm + bind the connection for a `hello`'s workspace-target.
///
/// A workspace-less hello is a pure version handshake: negotiate `proto` + list
/// `caps`, bind and pin nothing. With a target, the storage pin reuses the
/// registry's one canonicalize → deny-ceiling → sentinel path (risk R2, via
/// [`Registry::pin`]); the warm reflects current disk (U1 residency); the bind
/// swaps the read ops' corpus source from the deleted `attach` to `hello`.
fn hello_body(
    registry: &Registry,
    attached: &mut Option<PathBuf>,
    workspace: Option<String>,
) -> Result<ResponseBody, Box<ErrorBody>> {
    let (root, storage) = match workspace {
        None => (None, None),
        Some(target) => match registry.pin(Path::new(&target)) {
            PinOutcome::Pinned { workspace, drawer } => {
                registry
                    .warm_or_build(&workspace)
                    .map_err(|e| warm_err_to_wire(&e))?;
                let root = registry.with_engine(&workspace, |engine| engine.map(engine_root));
                *attached = Some(workspace);
                (root, Some(drawer.to_string_lossy().into_owned()))
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
    })
}

/// Strict-decode one wire read op and serve it from the attached workspace's
/// resident engine, rendering the frozen `wire::Response` frame (echoing `id`).
fn serve_wire(
    registry: &Registry,
    attached: Option<&Path>,
    obj: &Map<String, Value>,
) -> wire::Response {
    let id = obj.get("id").and_then(Value::as_u64);
    let body = wire_serve::decode::decode(obj).and_then(|op| dispatch_read(registry, attached, op));
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

/// Route one decoded wire read op to its arm, served from the attached
/// workspace's resident engine. `resolve` (§4.5 walk plane) is served COLD from
/// a per-request walk corpus — a different corpus from the hash-domain warm
/// engine. The write path (`splice`), `hello` (U3), and `sub` (P2) are not
/// served here yet and answer `unknown_op` (§3.2 discovery honesty).
fn dispatch_read(
    registry: &Registry,
    attached: Option<&Path>,
    op: Op,
) -> Result<ResponseBody, Box<ErrorBody>> {
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
            Ok(wire_serve::read::extract(doc, &path, kinds))
        }),
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
            // The resident daemon holds no delta ring yet (the watcher is P2):
            // a same-root diff is truthfully empty; any other range is
            // `root_unknown` → full resync (degrade to re-derive, never to
            // wrong data).
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
        // `hello` is intercepted upstream in `handle_line` (the handshake binds
        // the connection), so it never reaches here. `splice` = W1, `sub` = P2
        // are not served yet — §3.2 discovery honesty: an op is served or
        // answers `unknown_op`.
        Op::Hello { .. } | Op::Splice { .. } | Op::Sub { .. } => {
            Err(Box::new(ErrorBody::new(ErrorCode::UnknownOp)))
        }
    }
}

/// The workspace's ambient root cursor = its warm engine's corpus content hash
/// (the reuse key), re-homed into the wire `Root` token.
fn engine_root(engine: &WorkspaceEngine) -> Root {
    Root(engine.at_fingerprint.0.clone())
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
