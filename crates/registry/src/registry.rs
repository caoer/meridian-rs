//! In-memory registry: map keyed by canonical workspace path, plus
//! register / resolve / unregister / list / reap.
//!
//! Map write lock is the serialization point. Register critical section
//! (deny → sentinel → insert → persist) holds one guard so concurrent
//! registrars for the same path first-writer-win by serialization.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use wire::{ErrorBody, ErrorCode, ResponseBody, Root};

use crate::engine::{WarmOutcome, WorkspaceEngine};
use crate::now_secs;
use crate::protocol::{DenyKind, WorkspaceEntry};
use crate::refresh::{RefreshState, view_err_to_refresh};
use crate::state::StateStore;

/// The outcome of a [`Registry::register`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegisterOutcome {
    /// This caller created the entry (first writer) and wrote the sentinel.
    Registered(WorkspaceEntry),
    /// The path was already registered; this caller adopted the existing
    /// entry (no second sentinel written).
    Adopted(WorkspaceEntry),
    /// The deny ceiling refused the path.
    Denied(DenyKind),
    /// The path could not be canonicalized, or the sentinel write failed.
    Error(String),
}

/// The outcome of a [`Registry::resolve`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveOutcome {
    /// The cwd is inside a registered workspace; the adopted entry is returned.
    Adopted(WorkspaceEntry),
    /// No registered ancestor — the caller degrades to ephemeral.
    Miss,
}

/// The outcome of a [`Registry::pin`] call (decision 0002 §4, U3 hello).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinOutcome {
    /// The workspace resolved (ancestor walk) or was pinned fresh: the canonical
    /// workspace root plus its storage drawer directory (the storage pin the
    /// hello response reports).
    Pinned {
        /// The canonical workspace root — the same key `inner` and `engines` use.
        workspace: PathBuf,
        /// The pinned drawer directory ([`cache::drawer_dir`]).
        drawer: PathBuf,
    },
    /// The deny ceiling refused the workspace-target.
    Denied(DenyKind),
    /// The target could not be canonicalized, or the sentinel write failed.
    Error(String),
}

/// Daemon workspace registry: guarded map, state store, drawer cache root.
///
/// `engines` is resident query state (U1): warm `WorkspaceEngine` per workspace,
/// keyed like `inner`. Disposable projection of disk — never persisted; cold
/// start holds none. Idle-reap drops engine with registration.
#[derive(Debug)]
pub struct Registry {
    inner: RwLock<HashMap<PathBuf, WorkspaceEntry>>,
    engines: RwLock<HashMap<PathBuf, WorkspaceEngine>>,
    /// V2 §Q2 per-workspace publish mutex (OD6/B1). Sole persistent writer;
    /// held across build + post-build sample so concurrent `view_path` on one
    /// workspace serializes. Distinct workspaces never contend.
    publish_locks: Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>,
    /// OD7 refresh telemetry (daemon memory, advisory). Also `as_of` proxy:
    /// last successful publish fingerprint within this epoch.
    refresh_state: RwLock<HashMap<PathBuf, RefreshState>>,
    /// S6 read-is-the-mint ledger per workspace (D6/H1). Not on
    /// [`WorkspaceEngine`]: rebuilds replace the engine and would evaporate
    /// receipts. Session memory; dropped on idle-reap only.
    read_mints: Mutex<HashMap<PathBuf, Arc<receipt::read_mint::ReadMintStore>>>,
    /// U20b delta plane, one ring per workspace — same create/reap as
    /// [`Self::read_mints`]. S6: key is canonical path (not a global ring).
    rings: Mutex<HashMap<PathBuf, Arc<crate::ring::WorkspaceRing>>>,
    /// Instance epoch token (`built_epoch`); restart mints a new one.
    epoch: String,
    /// G11 pre-warm quiet map: last [`fs::domain_stat_signature`] per warm
    /// workspace. Matching signature skips the corpus fold. Advisory only —
    /// missing/stale costs one extra snapshot, never a wrong answer.
    prewarm_signatures: Mutex<HashMap<PathBuf, u64>>,
    /// G11 activity clock: request count + last request unix secs. Pre-warm
    /// backoff and idle-exit both read this.
    requests: AtomicU64,
    last_request: AtomicU64,
    state: StateStore,
    cache_root: PathBuf,
}

impl Registry {
    /// Build a registry seeded with `entries` (loaded from the state file),
    /// persisting to `state` and writing drawer sentinels under `cache_root`.
    pub(crate) fn new(
        state: StateStore,
        cache_root: PathBuf,
        entries: Vec<WorkspaceEntry>,
    ) -> Self {
        let inner = entries
            .into_iter()
            .map(|entry| (entry.workspace.clone(), entry))
            .collect();
        Registry {
            inner: RwLock::new(inner),
            // Cold: no engines; first `warm_or_build` rebuilds from disk.
            engines: RwLock::new(HashMap::new()),
            publish_locks: Mutex::new(HashMap::new()),
            refresh_state: RwLock::new(HashMap::new()),
            read_mints: Mutex::new(HashMap::new()),
            // Cold: no rings; pre-restart `from_seq` ⇒ `root_unknown` (§7.1).
            rings: Mutex::new(HashMap::new()),
            epoch: now_secs().to_string(),
            prewarm_signatures: Mutex::new(HashMap::new()),
            requests: AtomicU64::new(0),
            // Clock starts at birth so idle-exit can age an unused daemon.
            last_request: AtomicU64::new(now_secs()),
            state,
            cache_root,
        }
    }

    /// Register `path` as a warm workspace.
    ///
    /// Canonicalizes, enforces the deny ceiling, then — under the write lock —
    /// adopts an existing entry or writes the drawer sentinel, inserts, and
    /// persists the state file. See the module docs for the serialization
    /// guarantee.
    pub fn register(&self, path: &Path) -> RegisterOutcome {
        let canonical = match workspace::canonicalize(path) {
            Ok(canonical) => canonical,
            Err(e) => {
                return RegisterOutcome::Error(format!(
                    "cannot canonicalize {} ({e})",
                    path.display()
                ));
            }
        };
        // Deny ceiling enforced IN THE DAEMON, not merely client-side.
        if let Some(reason) = workspace::deny_reason(&canonical) {
            return RegisterOutcome::Denied(reason.into());
        }

        let mut map = self.inner.write().unwrap_or_else(PoisonError::into_inner);
        if let Some(existing) = map.get_mut(&canonical) {
            existing.last_use = now_secs();
            return RegisterOutcome::Adopted(existing.clone());
        }

        // First writer for this path. Write the drawer sentinel BEFORE the map
        // insert, still under the lock, so a sentinel failure leaves no
        // dangling registry entry — one entry iff one sentinel.
        let drawer = cache::drawer_dir(&self.cache_root, &canonical);
        if let Err(e) = cache::register(&drawer, &canonical) {
            return RegisterOutcome::Error(format!(
                "drawer sentinel write failed for {} ({e})",
                canonical.display()
            ));
        }

        let now = now_secs();
        let entry = WorkspaceEntry {
            workspace: canonical.clone(),
            registered_at: now,
            last_use: now,
        };
        map.insert(canonical, entry.clone());
        self.persist(&map);
        RegisterOutcome::Registered(entry)
    }

    /// Resolve `cwd` against the registry: canonicalize, then walk it and its
    /// ancestors for the nearest registered workspace. A hit is adopted (its
    /// `last_use` bumped in memory — an LRU touch, not persisted); no hit is a
    /// [`ResolveOutcome::Miss`]. Never registers.
    pub fn resolve(&self, cwd: &Path) -> ResolveOutcome {
        let Ok(canonical) = workspace::canonicalize(cwd) else {
            return ResolveOutcome::Miss;
        };
        let mut map = self.inner.write().unwrap_or_else(PoisonError::into_inner);
        for ancestor in canonical.ancestors() {
            if let Some(entry) = map.get_mut(ancestor) {
                entry.last_use = now_secs();
                return ResolveOutcome::Adopted(entry.clone());
            }
        }
        ResolveOutcome::Miss
    }

    /// Pin storage for a **declared** root (`hello.workspace`).
    ///
    /// **Exact, or refuse** — no ancestor walk; a declaration never widens to
    /// an enclosing registered workspace (jail root IS the declared path).
    /// Reuses [`register`](Self::register) whole (R2). Response `workspace`
    /// names what actually bound (canonicalization may rewrite spelling).
    /// Does not warm; caller warms and binds.
    pub fn pin_declared(&self, root: &Path) -> PinOutcome {
        let workspace = match self.register(root) {
            RegisterOutcome::Registered(entry) | RegisterOutcome::Adopted(entry) => entry.workspace,
            RegisterOutcome::Denied(reason) => return PinOutcome::Denied(reason),
            RegisterOutcome::Error(message) => return PinOutcome::Error(message),
        };
        let drawer = cache::drawer_dir(&self.cache_root, &workspace);
        PinOutcome::Pinned { workspace, drawer }
    }

    /// Resolve + pin for a **cwd** (hint, not declaration). Ancestor walk is
    /// correct here; hit reuses the registered root, miss registers `cwd`.
    /// Split from [`pin_declared`](Self::pin_declared) so declarations never
    /// silently widen.
    pub fn pin_for_cwd(&self, cwd: &Path) -> PinOutcome {
        let workspace = match self.resolve(cwd) {
            ResolveOutcome::Adopted(entry) => entry.workspace,
            ResolveOutcome::Miss => match self.register(cwd) {
                RegisterOutcome::Registered(entry) | RegisterOutcome::Adopted(entry) => {
                    entry.workspace
                }
                RegisterOutcome::Denied(reason) => return PinOutcome::Denied(reason),
                RegisterOutcome::Error(message) => return PinOutcome::Error(message),
            },
        };
        let drawer = cache::drawer_dir(&self.cache_root, &workspace);
        PinOutcome::Pinned { workspace, drawer }
    }

    /// Warm the resident engine for `workspace`; rebuild only when the corpus
    /// content hash changed (U1). Reuse key is the content hash (R5), not
    /// workspace-identity Merkle. `Reused` ⇒ zero parses (`build_corpus` is
    /// rebuild-only). Fingerprint read and parse are outside the `engines`
    /// write lock (insert only) so workspaces do not block each other.
    ///
    /// # Errors
    /// Canonicalize failure, corpus unreadable, or non-UTF-8
    /// ([`io::ErrorKind::InvalidData`]).
    pub fn warm_or_build(&self, workspace: &Path) -> io::Result<WarmOutcome> {
        let canonical = workspace::canonicalize(workspace)
            .map_err(|e| io::Error::new(io::ErrorKind::NotFound, e))?;
        let root = fs::WorkspaceRoot(canonical.clone());

        // Cheap half (no parse): content hash from disk.
        let (files, fingerprint) = fs::domain_snapshot(&root)?;

        // Warm + unchanged → reuse, zero parses.
        {
            let engines = self.engines.read().unwrap_or_else(PoisonError::into_inner);
            if engines
                .get(&canonical)
                .is_some_and(|engine| engine.at_fingerprint == fingerprint)
            {
                return Ok(WarmOutcome::Reused);
            }
        }

        // Cold or content changed → rebuild once (only parse site).
        let (index, docs) = fs::build_corpus(files)?;
        let parsed = docs.len();
        let engine = WorkspaceEngine {
            index,
            docs,
            at_fingerprint: fingerprint,
        };
        self.engines
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(canonical, engine);
        Ok(WarmOutcome::Built { docs: parsed })
    }

    /// Borrow the warm engine for `canonical` under the read lock. Callers
    /// warm first via [`warm_or_build`](Self::warm_or_build). Closure must be
    /// borrow-and-project only (no re-entry into engines).
    pub fn with_engine<R>(
        &self,
        canonical: &Path,
        f: impl FnOnce(Option<&WorkspaceEngine>) -> R,
    ) -> R {
        let engines = self.engines.read().unwrap_or_else(PoisonError::into_inner);
        f(engines.get(canonical))
    }

    /// V2 §Q2 `view_path`: resolve `cwd`, publish/serve `view.duckdb`, return
    /// stamped PATH + pre-open freshness hint — never rows. Sole builder (OD6).
    /// Under the per-workspace publish mutex (B1). Branches: `fresh` ⇒ bounded
    /// rebuild (§Q3); known `as_of` ⇒ serve last-good (no rebuild); else first
    /// build. Publish failure with last-good serves it + OD7 `last_error`.
    ///
    /// # Errors
    /// `bad_request` (deny), `io_error` (resolve/fold/build with no last-good),
    /// `invalid_utf8`.
    pub fn view_path(&self, cwd: &str, fresh: bool) -> Result<ResponseBody, Box<ErrorBody>> {
        let (workspace, drawer, dir) = self.resolve_drawer(cwd)?;
        let dest = dir.join("view.duckdb");

        // Publish mutex across build + post-build sample.
        let lock = self.publish_lock(&workspace);
        let _publish = lock.lock().unwrap_or_else(PoisonError::into_inner);

        // Known as_of + file present ⇒ serve without rebuild; else build.
        let served = self
            .last_ok_fingerprint(&workspace)
            .filter(|_| dest.exists());
        let built = if fresh {
            self.bounded_fresh(&workspace, &drawer)
        } else if let Some(as_of) = served {
            let live = sample_fingerprint(&workspace)?;
            let state = view_state(&as_of, &live);
            Ok((as_of, live, state))
        } else {
            self.build_once(&workspace, &drawer)
        };

        let (as_of, live, state) = match built {
            Ok(triple) => triple,
            // Failed build: last-good if present (OD7 telemetry); else error.
            Err(op_err) => {
                if dest.exists()
                    && let Some(as_of) = self.last_ok_fingerprint(&workspace)
                {
                    let live = sample_fingerprint(&workspace)?;
                    let state = view_state(&as_of, &live);
                    (as_of, live, state)
                } else {
                    return Err(op_err);
                }
            }
        };

        let (refresh_in_progress, last_error) = self.refresh_telemetry(&workspace);
        Ok(ResponseBody::ViewPath {
            path: dest.to_string_lossy().into_owned(),
            as_of_root: Root(as_of.0),
            live_root: Root(live.0),
            // Not the ring tip — same `0` as `Root`/`Links` arms.
            changes_seq: 0,
            state,
            // Live sampled from warm fold; hint only (§Q3 C3).
            live_source: wire::ViewLiveSource::Watch,
            // Pre-open hint is never a verdict (B5+C3).
            stale: None,
            refresh_in_progress,
            last_error,
        })
    }

    /// Resolve `cwd` → workspace + drawer under this registry's `cache_root`
    /// (via [`pin_for_cwd`](Self::pin_for_cwd); honors test cache roots).
    fn resolve_drawer(
        &self,
        cwd: &str,
    ) -> Result<(PathBuf, cache::CacheDrawer, PathBuf), Box<ErrorBody>> {
        let workspace = match self.pin_for_cwd(Path::new(cwd)) {
            PinOutcome::Pinned { workspace, .. } => workspace,
            PinOutcome::Denied(reason) => {
                return Err(wire_serve::bad_request(format!(
                    "cannot resolve `{cwd}` to a workspace: it is the {reason} (deny ceiling)"
                )));
            }
            PinOutcome::Error(message) => {
                let mut e = ErrorBody::new(ErrorCode::IoError);
                e.cause = Some(message);
                return Err(Box::new(e));
            }
        };
        let dir = cache::drawer_dir(&self.cache_root, &workspace);
        let drawer = cache::CacheDrawer::Disk {
            dir: dir.clone(),
            workspace: workspace.clone(),
        };
        Ok((workspace, drawer, dir))
    }

    /// The per-workspace publish mutex, created on first use (V2 §Q2 / OD6).
    fn publish_lock(&self, workspace: &Path) -> Arc<Mutex<()>> {
        let mut locks = self
            .publish_locks
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        Arc::clone(
            locks
                .entry(workspace.to_path_buf())
                .or_insert_with(|| Arc::new(Mutex::new(()))),
        )
    }

    /// Workspace delta ring (U20b), created on first use. `workspace` must be
    /// CANONICAL — S6 isolation key (hello bind supplies it). [`Arc`] so a
    /// parked subscriber never holds this map's lock.
    #[must_use]
    pub fn ring(&self, workspace: &Path) -> Arc<crate::ring::WorkspaceRing> {
        let mut rings = self.rings.lock().unwrap_or_else(PoisonError::into_inner);
        Arc::clone(rings.entry(workspace.to_path_buf()).or_insert_with(|| {
            Arc::new(crate::ring::WorkspaceRing::new(&fs::WorkspaceRoot(
                workspace.to_path_buf(),
            )))
        }))
    }

    /// Read-is-the-mint ledger (S6), created on first use. `workspace` must be
    /// CANONICAL (same key as `engines`/`inner`). [`Arc`] so a slow read never
    /// holds this map's lock.
    #[must_use]
    pub fn read_mints(&self, workspace: &Path) -> Arc<receipt::read_mint::ReadMintStore> {
        let mut mints = self
            .read_mints
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        Arc::clone(
            mints
                .entry(workspace.to_path_buf())
                .or_insert_with(|| Arc::new(receipt::read_mint::ReadMintStore::new())),
        )
    }

    /// The last successful publish fingerprint (daemon-memory `as_of` proxy), or
    /// `None` when this daemon has not published this workspace.
    fn last_ok_fingerprint(&self, workspace: &Path) -> Option<model::MerkleRoot> {
        self.refresh_state
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(workspace)
            .and_then(|s| s.last_ok_fingerprint.clone())
    }

    /// The OD7 reply telemetry: `refresh_in_progress` (always `false` in round-1
    /// — rebuilds are synchronous, done before the reply) + the last failure.
    fn refresh_telemetry(&self, workspace: &Path) -> (bool, Option<wire::RefreshError>) {
        let states = self
            .refresh_state
            .read()
            .unwrap_or_else(PoisonError::into_inner);
        let last_error = states.get(workspace).and_then(|s| s.last_error.clone());
        (false, last_error)
    }

    /// Build + publish `view.duckdb` at the current disk fold, then sample
    /// `live` AFTER — the absent/first-build path (§Q3 post-result fold shape,
    /// no retry). `FRESH_AT_SAMPLE` iff `F0 == F_now`, else `STALE`.
    fn build_once(
        &self,
        workspace: &Path,
        drawer: &cache::CacheDrawer,
    ) -> Result<(model::MerkleRoot, model::MerkleRoot, wire::ViewState), Box<ErrorBody>> {
        let f0 = self.publish_now(workspace, drawer)?;
        let f_now = sample_fingerprint(workspace)?;
        let state = view_state(&f0, &f_now);
        Ok((f0, f_now, state))
    }

    /// The bounded `--fresh` rebuild (§Q3): build at `F0`, sample `live = F_now`
    /// after; equal ⇒ `FRESH_AT_SAMPLE`; else retry ONCE; still differing ⇒
    /// `RACED` with both fingerprints. Never loops, never labels fresh.
    fn bounded_fresh(
        &self,
        workspace: &Path,
        drawer: &cache::CacheDrawer,
    ) -> Result<(model::MerkleRoot, model::MerkleRoot, wire::ViewState), Box<ErrorBody>> {
        let f0 = self.publish_now(workspace, drawer)?;
        let f_now = sample_fingerprint(workspace)?;
        if f0 == f_now {
            return Ok((f0, f_now, wire::ViewState::FreshAtSample));
        }
        // The workspace raced the build — one bounded retry.
        let f0b = self.publish_now(workspace, drawer)?;
        let f_nowb = sample_fingerprint(workspace)?;
        let state = if f0b == f_nowb {
            wire::ViewState::FreshAtSample
        } else {
            wire::ViewState::Raced
        };
        Ok((f0b, f_nowb, state))
    }

    /// Warm, snapshot fingerprint+docs out of the engines lock, then
    /// `view::publish` (publish mutex only — slow `DuckDB` must not block warm
    /// inserts). Records OD7 telemetry. Returns built fingerprint `F0`.
    fn publish_now(
        &self,
        workspace: &Path,
        drawer: &cache::CacheDrawer,
    ) -> Result<model::MerkleRoot, Box<ErrorBody>> {
        self.warm_or_build(workspace)
            .map_err(|e| warm_err_to_wire(&e))?;
        // Clone out of the read lock before slow publish I/O.
        let Some((f0, docs)) = self.with_engine(workspace, |engine| {
            engine.map(|e| (e.at_fingerprint.clone(), e.docs.clone()))
        }) else {
            // Idle-reap race between warm and borrow — transient.
            return Err(Box::new(ErrorBody::new(ErrorCode::Internal)));
        };

        let stamp = view::PublishStamp {
            workspace: workspace.to_string_lossy().into_owned(),
            as_of_fingerprint: f0.0.clone(),
            epoch: self.epoch.clone(),
            seq: 0,
        };
        match view::publish(&docs, drawer, &stamp) {
            Ok(_path) => {
                self.record_publish_ok(workspace, f0.clone());
                Ok(f0)
            }
            Err(e) => {
                self.record_publish_err(workspace, &f0, &e);
                let mut err = ErrorBody::new(ErrorCode::IoError);
                err.cause = Some(e.to_string());
                Err(Box::new(err))
            }
        }
    }

    /// Adopt `fingerprint` as the workspace's last-good and clear its error
    /// (OD7 recovery).
    fn record_publish_ok(&self, workspace: &Path, fingerprint: model::MerkleRoot) {
        self.refresh_state
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(workspace.to_path_buf())
            .or_default()
            .record_ok(fingerprint, now_secs());
    }

    /// Record a publish failure against the workspace (OD7); the last-good
    /// fingerprint is left untouched (the old file is still published).
    fn record_publish_err(
        &self,
        workspace: &Path,
        attempted: &model::MerkleRoot,
        e: &view::ViewError,
    ) {
        let error = view_err_to_refresh(e, attempted, now_secs());
        self.refresh_state
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(workspace.to_path_buf())
            .or_default()
            .record_err(error);
    }

    /// Pre-warm every already-warm workspace (P2 watch driver). Rebuilds only
    /// when content hash changed — latency only; correctness is fingerprint.
    /// Cold daemon: no-op. Snapshot warm keys under read lock, then release
    /// before any rebuild. Returns workspaces that rebuilt; best-effort on errors.
    pub fn prewarm(&self) -> Vec<PathBuf> {
        let warm: Vec<PathBuf> = {
            let engines = self.engines.read().unwrap_or_else(PoisonError::into_inner);
            engines.keys().cloned().collect()
        };
        let mut rebuilt = Vec::new();
        for workspace in warm {
            if self.stat_signature_unchanged(&workspace) {
                continue;
            }
            // Only `Built` counts; `Reused`/`Err` are best-effort no-ops.
            if let Ok(WarmOutcome::Built { .. }) = self.warm_or_build(&workspace) {
                rebuilt.push(workspace);
            }
        }
        rebuilt
    }

    /// G11: has `workspace` looked untouched since the last sweep? Records the
    /// observed signature (`false` once per change). Unreadable ⇒ `false`
    /// (never skip on error).
    fn stat_signature_unchanged(&self, workspace: &Path) -> bool {
        let Ok(signature) = fs::domain_stat_signature(&fs::WorkspaceRoot(workspace.to_path_buf()))
        else {
            return false;
        };
        let mut seen = self
            .prewarm_signatures
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if seen.get(workspace) == Some(&signature) {
            return true;
        }
        seen.insert(workspace.to_path_buf(), signature);
        false
    }

    /// G11: record that a client request was served — the daemon's activity
    /// clock. Bumped once per dispatched request by the socket loop.
    pub fn note_request(&self) {
        self.requests.fetch_add(1, Ordering::Relaxed);
        self.last_request.store(now_secs(), Ordering::Relaxed);
    }

    /// G11 liveness: hold the quiet clock open without counting a request.
    ///
    /// An armed `sub` connection is activity for idle-exit — the daemon has a
    /// live consumer — but it is not traffic for the pre-warm backoff: nothing
    /// is being asked of the engine, so the cadence must still be allowed to
    /// decay. Two clocks, one bump.
    pub fn note_liveness(&self) {
        self.last_request.store(now_secs(), Ordering::Relaxed);
    }

    /// Is any workspace subscribed? (G11 idle-exit: a subscribed daemon does
    /// not exit under its subscriber.)
    #[must_use]
    pub fn has_subscribers(&self) -> bool {
        let rings = self.rings.lock().unwrap_or_else(PoisonError::into_inner);
        rings.values().any(|ring| ring.has_subscribers())
    }

    /// How many client requests this daemon has served since it started.
    ///
    /// The pre-warm backoff watches this rather than a timestamp: a counter
    /// that moved means traffic arrived *between two sweeps*, which a
    /// one-second-granular clock can miss entirely.
    #[must_use]
    pub fn request_count(&self) -> u64 {
        self.requests.load(Ordering::Relaxed)
    }

    /// Unix seconds of the last client request — or of daemon start, when there
    /// has been none. Never `0` in practice, so an idle-exit check on it cannot
    /// be tricked into firing immediately.
    #[must_use]
    pub fn last_request_secs(&self) -> u64 {
        self.last_request.load(Ordering::Relaxed)
    }

    /// Unregister `path`, dropping it from memory and the state file. The
    /// drawer is left for `cache::gc`. Returns `true` when an entry was
    /// removed.
    ///
    /// Matches on the canonical path when the directory still resolves, else
    /// on the path as given — so a vanished workspace can still be unregistered
    /// by the canonical path a `list` reported.
    pub fn unregister(&self, path: &Path) -> bool {
        let key = workspace::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let mut map = self.inner.write().unwrap_or_else(PoisonError::into_inner);
        let removed = map.remove(&key).is_some();
        if removed {
            self.persist(&map);
        }
        removed
    }

    /// Every registered workspace, unordered.
    #[must_use]
    pub fn entries(&self) -> Vec<WorkspaceEntry> {
        self.inner
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .cloned()
            .collect()
    }

    /// Drop entries with `last_use <= now - threshold_secs`. Injectable clock
    /// for tests. Deregisters only (drawer is `cache::gc`). Also drops warm
    /// engine, read-mint ledger, and ring on the same horizon — never on corpus
    /// change. `engines` lock taken after `inner` is released.
    ///
    /// Live subscriptions are exempt (U20b): push-only connections never touch
    /// `last_use`. Reaping them would fork the per-workspace `seq` (§4.7) —
    /// next `sub` would mint a second ring — not merely stop delivery.
    pub fn reap(&self, now: u64, threshold_secs: u64) -> Vec<PathBuf> {
        let cutoff = now.saturating_sub(threshold_secs);
        // Exemption set before `inner` write lock — concurrent `sub` safe.
        let subscribed = self.subscribed_workspaces();
        let reaped: Vec<PathBuf> = {
            let mut map = self.inner.write().unwrap_or_else(PoisonError::into_inner);
            let reaped: Vec<PathBuf> = map
                .iter()
                .filter(|(key, entry)| entry.last_use <= cutoff && !subscribed.contains(*key))
                .map(|(key, _)| key.clone())
                .collect();
            for key in &reaped {
                map.remove(key);
            }
            if !reaped.is_empty() {
                self.persist(&map);
            }
            reaped
        };
        if !reaped.is_empty() {
            let mut engines = self.engines.write().unwrap_or_else(PoisonError::into_inner);
            for key in &reaped {
                engines.remove(key);
            }
            drop(engines);
            let mut mints = self
                .read_mints
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            for key in &reaped {
                mints.remove(key);
            }
            drop(mints);
            // Ring dies on the same horizon; later `sub` gets a fresh epoch.
            let mut rings = self.rings.lock().unwrap_or_else(PoisonError::into_inner);
            for key in &reaped {
                rings.remove(key);
            }
        }
        reaped
    }

    /// Workspaces with ≥1 live subscription — reaper exemption set.
    fn subscribed_workspaces(&self) -> std::collections::HashSet<PathBuf> {
        let rings = self.rings.lock().unwrap_or_else(PoisonError::into_inner);
        rings
            .iter()
            .filter(|(_, ring)| ring.has_subscribers())
            .map(|(key, _)| key.clone())
            .collect()
    }

    /// Persist the current map to the state file, logging (never failing) on a
    /// write error — a lost persist costs a warm registration across restart,
    /// which is recoverable; it must not crash the daemon.
    fn persist(&self, map: &HashMap<PathBuf, WorkspaceEntry>) {
        let entries: Vec<WorkspaceEntry> = map.values().cloned().collect();
        if let Err(e) = self.state.save(&entries) {
            eprintln!("registry: state save failed ({e}); warm set may not survive restart");
        }
    }

    /// Persist the current map to the state file (used at graceful shutdown to
    /// capture in-memory `last_use` bumps from `resolve`).
    pub(crate) fn flush(&self) {
        let map = self.inner.read().unwrap_or_else(PoisonError::into_inner);
        self.persist(&map);
    }
}

/// Sample a workspace's CURRENT disk fingerprint — a full-corpus fold
/// (`fs::domain_snapshot`, the cheap half, no parse), the §Q3 live sample.
fn sample_fingerprint(workspace: &Path) -> Result<model::MerkleRoot, Box<ErrorBody>> {
    let root = fs::WorkspaceRoot(workspace.to_path_buf());
    Ok(fs::domain_snapshot(&root)
        .map_err(|e| warm_err_to_wire(&e))?
        .1)
}

/// The §Q3 pre-open state from an `as_of`/`live` fingerprint pair:
/// `FRESH_AT_SAMPLE` on equality, else `STALE` (a legal frame, never an error).
/// `RACED` is a `--fresh`-only outcome, decided in [`Registry::bounded_fresh`],
/// so it never arises here.
fn view_state(as_of: &model::MerkleRoot, live: &model::MerkleRoot) -> wire::ViewState {
    if as_of == live {
        wire::ViewState::FreshAtSample
    } else {
        wire::ViewState::Stale
    }
}

/// Map a `warm_or_build` / `domain_snapshot` I/O failure onto its wire frame: a
/// non-UTF-8 corpus file is `invalid_utf8` (refused, never lossy-decoded);
/// anything else carries its cause on `io_error`. Mirrors the daemon read
/// path's `warm_err_to_wire` (`server.rs`) so a fold failure reads identically
/// whichever op raised it.
fn warm_err_to_wire(e: &io::Error) -> Box<ErrorBody> {
    if e.kind() == io::ErrorKind::InvalidData {
        return Box::new(ErrorBody::new(ErrorCode::InvalidUtf8));
    }
    let mut err = ErrorBody::new(ErrorCode::IoError);
    err.cause = Some(e.to_string());
    Box::new(err)
}

#[cfg(test)]
mod engine_tests {
    //! U1 resident-engine gates: warm reuse, one rebuild on change, query serve, reap.

    use super::*;
    use crate::state::StateStore;
    use std::fs;

    /// Registry under `home` (no socket — in-process `warm_or_build`).
    fn registry_in(home: &Path) -> Registry {
        let cache_root = home.join("cache");
        fs::create_dir_all(&cache_root).unwrap();
        Registry::new(
            StateStore::new(home.join("state.json")),
            cache_root,
            Vec::new(),
        )
    }

    /// A workspace `home/ws` seeded with `files` (a sibling of the cache root,
    /// so the corpus walk never sees the drawer).
    fn write_ws(home: &Path, files: &[(&str, &str)]) -> PathBuf {
        let ws = home.join("ws");
        fs::create_dir_all(&ws).unwrap();
        for (rel, content) in files {
            let path = ws.join(rel);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(path, content).unwrap();
        }
        ws
    }

    #[test]
    fn second_warm_at_same_fingerprint_parses_nothing() {
        let home = tempfile::tempdir().unwrap();
        let reg = registry_in(home.path());
        let ws = write_ws(
            home.path(),
            &[("a.md", "# A\n\nsee [[b]]\n"), ("b.md", "# B\n")],
        );

        // Cold → build.
        assert_eq!(
            reg.warm_or_build(&ws).unwrap(),
            WarmOutcome::Built { docs: 2 },
            "first warm builds the corpus"
        );
        // Unchanged hash → reuse (zero parses).
        assert_eq!(
            reg.warm_or_build(&ws).unwrap(),
            WarmOutcome::Reused,
            "second warm at the same fingerprint parses nothing"
        );
        assert_eq!(
            reg.warm_or_build(&ws).unwrap(),
            WarmOutcome::Reused,
            "and stays warm — no rebuild storm"
        );
    }

    #[test]
    fn corpus_mutation_triggers_exactly_one_rebuild() {
        let home = tempfile::tempdir().unwrap();
        let reg = registry_in(home.path());
        let ws = write_ws(home.path(), &[("a.md", "# A\n"), ("b.md", "# B\n")]);

        assert_eq!(
            reg.warm_or_build(&ws).unwrap(),
            WarmOutcome::Built { docs: 2 }
        );
        assert_eq!(reg.warm_or_build(&ws).unwrap(), WarmOutcome::Reused);

        fs::write(ws.join("a.md"), "# A changed\n\nnew body\n").unwrap();

        assert_eq!(
            reg.warm_or_build(&ws).unwrap(),
            WarmOutcome::Built { docs: 2 },
            "a corpus change rebuilds once"
        );
        assert_eq!(
            reg.warm_or_build(&ws).unwrap(),
            WarmOutcome::Reused,
            "the rebuild is once, not a storm"
        );
    }

    #[test]
    fn warm_engine_serves_a_real_query() {
        let home = tempfile::tempdir().unwrap();
        let reg = registry_in(home.path());
        let ws = write_ws(
            home.path(),
            &[("a.md", "# A\n\nsee [[b]]\n"), ("b.md", "# B\n")],
        );

        reg.warm_or_build(&ws).unwrap();

        // Warm state answers a real query::links.
        let canonical = workspace::canonicalize(&ws).unwrap();
        let engines = reg.engines.read().unwrap();
        let engine = engines.get(&canonical).expect("warm engine resident");
        let links = query::links(&engine.index, &engine.docs, Some("a.md"));
        let a = links.get("a.md").expect("a.md edge entry");
        assert_eq!(
            a.resolved.get("b.md"),
            Some(&1),
            "the resident index resolves [[b]] → b.md"
        );
    }

    #[test]
    fn reap_drops_the_warm_engine() {
        let home = tempfile::tempdir().unwrap();
        let reg = registry_in(home.path());
        let ws = write_ws(home.path(), &[("a.md", "# A\n")]);
        let canonical = workspace::canonicalize(&ws).unwrap();

        reg.register(&canonical);
        reg.warm_or_build(&canonical).unwrap();
        assert!(
            reg.engines.read().unwrap().contains_key(&canonical),
            "engine warm before reap"
        );

        // Entry + engine drop on the one idle-reap horizon (R4).
        let reaped = reg.reap(u64::MAX, 0);
        assert!(reaped.contains(&canonical), "the entry was reaped");
        assert!(
            !reg.engines.read().unwrap().contains_key(&canonical),
            "reap drops the warm engine with the registration"
        );
    }

    /// P2 latency: change pre-warms on the watch event; next query pays zero parse.
    #[test]
    fn prewarm_absorbs_the_change_so_the_next_query_parses_nothing() {
        let home = tempfile::tempdir().unwrap();
        let reg = registry_in(home.path());
        let ws = write_ws(
            home.path(),
            &[("a.md", "# A\n\nsee [[b]]\n"), ("b.md", "# B\n")],
        );
        let canonical = workspace::canonicalize(&ws).unwrap();

        assert_eq!(
            reg.warm_or_build(&ws).unwrap(),
            WarmOutcome::Built { docs: 2 }
        );
        assert!(reg.prewarm().is_empty(), "a quiet sweep rebuilds nothing");

        fs::write(ws.join("a.md"), "# A changed\n\nnew body\n").unwrap();

        assert_eq!(
            reg.prewarm(),
            vec![canonical],
            "the edit rebuilds on the watch event, not lazily on the query"
        );

        assert_eq!(
            reg.warm_or_build(&ws).unwrap(),
            WarmOutcome::Reused,
            "the query after a pre-warm parses nothing — latency moved to the watch event"
        );
    }

    /// P2 crash recovery: cold start, first query rebuilds from disk (no new machinery).
    #[test]
    fn crash_recovery_rebuilds_from_disk_with_no_added_machinery() {
        let home = tempfile::tempdir().unwrap();
        let ws = write_ws(
            home.path(),
            &[("a.md", "# A\n\nsee [[b]]\n"), ("b.md", "# B\n")],
        );

        {
            let reg = registry_in(home.path());
            reg.warm_or_build(&ws).unwrap();
            assert!(reg.prewarm().is_empty(), "warm + quiet before the crash");
        } // reg dropped == crash

        fs::write(ws.join("a.md"), "# A\n\nsee [[b]] and [[c]]\n").unwrap();
        fs::write(ws.join("c.md"), "# C\n").unwrap();

        let reg = registry_in(home.path());
        assert!(
            reg.prewarm().is_empty(),
            "a cold daemon holds no warm engines to sweep"
        );

        assert_eq!(
            reg.warm_or_build(&ws).unwrap(),
            WarmOutcome::Built { docs: 3 },
            "the first query after a crash rebuilds from disk"
        );
        let canonical = workspace::canonicalize(&ws).unwrap();
        let engines = reg.engines.read().unwrap();
        let engine = engines.get(&canonical).expect("warm engine resident");
        let links = query::links(&engine.index, &engine.docs, Some("a.md"));
        let a = links.get("a.md").expect("a.md edge entry");
        assert_eq!(
            a.resolved.get("c.md"),
            Some(&1),
            "the rebuilt index reflects the on-disk edit — correct via fingerprint"
        );
    }
}
