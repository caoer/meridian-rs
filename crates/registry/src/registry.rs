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

use crate::engine::{WarmOutcome, WorkspaceEngine};
use crate::now_secs;
use crate::protocol::{DenyKind, WorkspaceEntry};
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
    engines: RwLock<HashMap<PathBuf, Arc<WorkspaceEngine>>>,
    /// S6 read-is-the-mint ledger per workspace (D6/H1). Not on
    /// [`WorkspaceEngine`]: rebuilds replace the engine and would evaporate
    /// receipts. Session memory; dropped on idle-reap only.
    read_mints: Mutex<HashMap<PathBuf, Arc<receipt::read_mint::ReadMintStore>>>,
    /// U20b delta plane, one ring per workspace — same create/reap as
    /// [`Self::read_mints`]. S6: key is canonical path (not a global ring).
    rings: Mutex<HashMap<PathBuf, Arc<crate::ring::WorkspaceRing>>>,
    /// G11 pre-warm quiet map: last [`fs::domain_stat_signature`] per warm
    /// workspace. Matching signature skips the corpus fold. Advisory only —
    /// missing/stale costs one extra snapshot, never a wrong answer.
    prewarm_signatures: Mutex<HashMap<PathBuf, u64>>,
    /// Per-workspace §12.2 leaf memo — what makes a currency pass cost one
    /// `stat` per domain member instead of a re-read of the whole corpus.
    /// Same lifetime as the engine: dropped on idle-reap. Each entry is its
    /// own `Arc<Mutex<…>>` so the run plane can borrow ONE workspace's memo
    /// for its bracket observations (card run-observation-unification)
    /// without holding the map — and so one workspace's pass never serializes
    /// another's.
    domain_caches: Mutex<HashMap<PathBuf, Arc<Mutex<fs::DomainCache>>>>,
    /// § A.11 resident sql caches: the open `sql.duckdb` handle per
    /// workspace, this daemon being each file's single owner. The CONNECTION
    /// dies on idle-reap with the engine; the FILE deliberately survives —
    /// its pin is content-derived, so trusting it across the gap is sound
    /// (re-warm compares fingerprints before serving).
    sql_stores: Mutex<HashMap<PathBuf, Arc<Mutex<view::store::SqlStore>>>>,
    /// G11 activity clock: request count + last request unix secs. Pre-warm
    /// backoff and idle-exit both read this.
    requests: AtomicU64,
    last_request: AtomicU64,
    /// § A.5 mount-table cache. Machine-scoped (not per-workspace): the
    /// binding file lives outside every workspace's hash domain, so no
    /// engine or ring can carry it.
    mounts: crate::mounts::MountsCache,
    state: StateStore,
    cache_root: PathBuf,
    /// Test-only pause gate for the rebuild race window. When armed, the next
    /// rebuild pass announces itself on the first channel, then parks on the
    /// second — between its disk snapshot and its `engines` insert, the exact
    /// window the insert guard must protect. One-shot: the pass that hits it
    /// consumes it. `cfg(test)` excludes it from every release build by
    /// construction (disclosed; RC1-precedent seam).
    #[cfg(test)]
    pause_before_insert:
        Mutex<Option<(std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>)>>,
    /// Test-only pause gate for the warm→borrow window: when armed, the read
    /// pass in `server::warm_engine_read` announces itself on the first
    /// channel after its successful warm, then parks on the second — between
    /// `warm_or_build` and `with_engine`, the exact window the idle reaper
    /// can win. One-shot: the pass that hits it consumes it. `cfg(test)`
    /// excludes it from every release build by construction (disclosed;
    /// same seam class as `pause_before_insert`, the PR #9 precedent).
    #[cfg(test)]
    pub(crate) pause_before_borrow:
        Mutex<Option<(std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>)>>,
    /// Test-only pause gate for the reap's exemption window: when armed, the
    /// reap pass announces itself on the first channel, then parks on the
    /// second — before its decide-and-remove critical section, where the
    /// pre-linearization sweep had already read its exemption snapshot. A
    /// subscription claim landing in that park must still be honored.
    /// One-shot: the pass that hits it consumes it. `cfg(test)` excludes it
    /// from every release build by construction (disclosed; same seam class
    /// as `pause_before_insert`, the PR #9 precedent).
    #[cfg(test)]
    pub(crate) pause_in_reap_window:
        Mutex<Option<(std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>)>>,
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
            read_mints: Mutex::new(HashMap::new()),
            // Cold: no rings; a pre-restart cursor dies on its instance ⇒
            // `root_unknown` (§7.1, B-01).
            rings: Mutex::new(HashMap::new()),
            prewarm_signatures: Mutex::new(HashMap::new()),
            // Cold: no memo; the first currency pass reads every member once.
            domain_caches: Mutex::new(HashMap::new()),
            // Cold: no open sql handles; first `sql` op opens (or cold-builds)
            // each workspace's file.
            sql_stores: Mutex::new(HashMap::new()),
            requests: AtomicU64::new(0),
            // Clock starts at birth so idle-exit can age an unused daemon.
            last_request: AtomicU64::new(now_secs()),
            // Cold: the first `mounts` call derives the table.
            mounts: crate::mounts::MountsCache::default(),
            state,
            cache_root,
            #[cfg(test)]
            pause_before_insert: Mutex::new(None),
            #[cfg(test)]
            pause_before_borrow: Mutex::new(None),
            #[cfg(test)]
            pause_in_reap_window: Mutex::new(None),
        }
    }

    /// The machine-scoped mount-table cache the `mounts` op serves through
    /// (§ A.5 config-hash freshness).
    pub(crate) fn mounts_cache(&self) -> &crate::mounts::MountsCache {
        &self.mounts
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
        // Deny ceiling enforced in the daemon, not merely client-side.
        if let Some(reason) = workspace::deny_reason(&canonical) {
            return RegisterOutcome::Denied(reason.into());
        }

        let mut map = self.inner.write().unwrap_or_else(PoisonError::into_inner);
        if let Some(existing) = map.get_mut(&canonical) {
            existing.last_use = now_secs();
            return RegisterOutcome::Adopted(existing.clone());
        }

        // First writer for this path. Write the drawer sentinel before the map
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
    /// an enclosing registered workspace (jail root is the declared path).
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

    /// Warm the resident engine for `workspace`; rebuild only when the corpus
    /// content hash changed (U1). Reuse key is the content hash (R5), not
    /// workspace-identity Merkle. `Reused` ⇒ zero parses. Fingerprint read
    /// and parse are outside the `engines` write lock (the locked section
    /// compares fingerprints and inserts — no I/O, no parse) so workspaces
    /// do not block each other.
    ///
    /// A rebuild against a RESIDENT engine is INCREMENTAL
    /// ([`fs::update_corpus`]): it re-reads and re-parses exactly the members
    /// whose §12.2 leaf digest moved against the engine's recorded leaf set —
    /// O(corpus) in `stat`s, O(delta) in bytes and parses. Cold (no resident
    /// engine) builds from scratch ([`fs::domain_snapshot_with_leaves`] +
    /// [`fs::build_corpus`]) — the only whole-corpus parse site.
    ///
    /// **Stamp law (amended with the incremental arm).** A built engine's
    /// `at_fingerprint` folds its own leaf set. A mover's leaf derives from
    /// the very bytes this pass read and parsed; an unmoved member's leaf —
    /// and its parsed document — carry forward from the resident build on
    /// `StatKey` evidence (dev, ino, size, mtime, ctime — the §12.2 memo's
    /// standing). That is the SAME evidence grade every warm hit has always
    /// served on: a hit serves the resident corpus because the memo's fold
    /// matches the stamp. The incremental arm spends that one trust in the
    /// same place, once per carry; no digest ever carries across a leaf that
    /// moved, and nothing served is stamped with a root its own build did
    /// not fold.
    ///
    /// Concurrent rebuilds of one workspace are WITNESS-GUARDED: a rebuild
    /// records what was resident when it judged the rebuild necessary, and
    /// its insert lands only while the resident engine is still exactly that
    /// witness. A build that lost the race never replaces the winner blind —
    /// the pass goes around: it re-derives freshness from disk and either
    /// adopts the resident (fingerprints equal) or rebuilds again. The
    /// resident engine therefore never regresses to an older corpus state.
    /// No clock ordering decides freshness — every comparison is between
    /// fingerprints, each the fold of a build's own leaf set.
    ///
    /// # Errors
    /// Canonicalize failure or corpus unreadable. A non-UTF-8 MEMBER is not an
    /// error: it degrades per-file (`fs::build_corpus` skips and reports it) —
    /// only a domain config that cannot be decoded still refuses the warm.
    pub fn warm_or_build(&self, workspace: &Path) -> io::Result<WarmOutcome> {
        let canonical = workspace::canonicalize(workspace)
            .map_err(|e| io::Error::new(io::ErrorKind::NotFound, e))?;
        let root = fs::WorkspaceRoot(canonical.clone());

        // Documents parsed by this call's most recent rebuild pass. `None`
        // until the first rebuild: only a call that parsed nothing may report
        // `Reused` — the outcome's zero-parse proof stays per-call.
        let mut parsed: Option<usize> = None;

        let cache = self.domain_cache(&canonical);
        loop {
            // Cheap half (no parse, and no re-read of anything that did not
            // move): content hash from disk through the leaf memo.
            let fingerprint = {
                cache
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .root(&root)?
            };

            // Warm + unchanged → done. Nothing is copied out of the memo on
            // this path: it is the hot one, and a 20k-entry clone per currency
            // pass would put back a slice of what the memo just removed. A
            // miss records the resident fingerprint as the WITNESS the guarded
            // insert below checks against, and pins the resident engine as
            // the incremental pass's prior build.
            let (witness, prior) = {
                let engines = self.engines.read().unwrap_or_else(PoisonError::into_inner);
                match engines.get(&canonical) {
                    Some(engine) if engine.at_fingerprint == fingerprint => {
                        return Ok(match parsed {
                            None => WarmOutcome::Reused,
                            Some(docs) => WarmOutcome::Built { docs },
                        });
                    }
                    Some(engine) => (
                        Some(engine.at_fingerprint.clone()),
                        Some(Arc::clone(engine)),
                    ),
                    None => (None, None),
                }
            };

            // Content changed → incremental pass against the resident build
            // (movers only — see the stamp law above); cold → the one
            // whole-corpus parse site. Leaf-set clones happen on this rebuild
            // path only, never per currency pass.
            let engine = if let Some(prior) = prior {
                let fresh = {
                    cache
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .leaf_digests()
                };
                let update =
                    fs::update_corpus(&root, &prior.docs, &prior.unserved, &prior.leaves, &fresh)?;
                parsed = Some(update.parsed);
                WorkspaceEngine {
                    index: update.index,
                    docs: update.docs,
                    unserved: update.unserved,
                    at_fingerprint: update.root,
                    leaves: update.leaves,
                }
            } else {
                let (files, leaves, fingerprint) = fs::domain_snapshot_with_leaves(&root)?;
                let (index, docs, unserved) = fs::build_corpus(files);
                parsed = Some(docs.len());
                WorkspaceEngine {
                    index,
                    docs,
                    unserved,
                    at_fingerprint: fingerprint,
                    leaves,
                }
            };
            let docs_parsed = parsed.unwrap_or(0);

            // Test-only: park here when the gate is armed (see the field docs).
            #[cfg(test)]
            {
                let gate = self
                    .pause_before_insert
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take();
                if let Some((arrived, release)) = gate {
                    let _ = arrived.send(());
                    let _ = release.recv();
                }
            }

            {
                let mut engines = self.engines.write().unwrap_or_else(PoisonError::into_inner);
                let resident = engines.get(&canonical).map(|e| e.at_fingerprint.clone());
                if resident.as_ref() == Some(&engine.at_fingerprint) {
                    // A concurrent rebuild already installed this exact corpus
                    // state — keeping it IS this build, delivered.
                    return Ok(WarmOutcome::Built { docs: docs_parsed });
                }
                if resident == witness {
                    engines.insert(canonical.clone(), Arc::new(engine));
                    return Ok(WarmOutcome::Built { docs: docs_parsed });
                }
            }
            // The resident engine moved while this pass was off the lock: a
            // concurrent rebuild landed, and this build may be the older disk
            // state. Never regress on a guess — go around and re-derive.
        }
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
        f(engines.get(canonical).map(|arc| &**arc))
    }

    /// Clone the warm engine's [`Arc`] for `canonical` — the § A.7 entry
    /// world's pin. Callers warm first via [`warm_or_build`](Self::warm_or_build).
    ///
    /// The clone is O(1) and the read lock drops at return, so a holder never
    /// blocks a rebuild: a foreign write swaps the MAP entry to a NEW `Arc`
    /// while the held one keeps the entry generation alive for exactly the
    /// attempt that pinned it. Attempt-scoped by construction — nothing here
    /// retains versions (the engines map still holds ONE generation), so this
    /// is not MVCC and must not grow into one.
    #[must_use]
    pub fn engine_snapshot(&self, canonical: &Path) -> Option<Arc<WorkspaceEngine>> {
        let engines = self.engines.read().unwrap_or_else(PoisonError::into_inner);
        engines.get(canonical).cloned()
    }

    /// The workspace's resident sql cache handle (§ A.11), opened in its
    /// storage drawer on first use — this daemon is the file's single owner.
    /// `workspace` must be canonical (the hello bind supplies it).
    ///
    /// # Errors
    /// The file cannot be opened or initialised ([`view::store::SqlStore::open`]).
    pub(crate) fn sql_store(
        &self,
        workspace: &Path,
    ) -> Result<Arc<Mutex<view::store::SqlStore>>, view::ViewError> {
        let mut stores = self
            .sql_stores
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(store) = stores.get(workspace) {
            return Ok(Arc::clone(store));
        }
        let drawer = cache::drawer_dir(&self.cache_root, workspace);
        let store = view::store::SqlStore::open(&drawer.join(view::store::SQL_CACHE_FILENAME))?;
        let store = Arc::new(Mutex::new(store));
        stores.insert(workspace.to_path_buf(), Arc::clone(&store));
        Ok(store)
    }

    /// The workspace's current corpus root through the §12.2 leaf memo — one
    /// `stat` per member, byte-reads only movers. The § A.11 post-result
    /// currency pass.
    ///
    /// # Errors
    /// I/O failure walking or reading the domain.
    pub(crate) fn currency_root(&self, workspace: &Path) -> io::Result<model::MerkleRoot> {
        self.domain_cache(workspace)
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .root(&fs::WorkspaceRoot(workspace.to_path_buf()))
    }

    /// The workspace's resident domain memo (dir listings + §12.2 leaves),
    /// created on first use — the ONE instrument every currency pass, warm
    /// rebuild, and daemon-door run observation share (card
    /// run-observation-unification). `workspace` must be canonical (the hello
    /// bind supplies it — the same key discipline as [`ring`](Self::ring)).
    /// [`Arc`] so a holder locks one workspace's memo, never the map; an
    /// in-flight holder across an idle-reap keeps a private memo that dies
    /// with it, which is only ever an extra read.
    #[must_use]
    pub fn domain_cache(&self, workspace: &Path) -> Arc<Mutex<fs::DomainCache>> {
        let mut caches = self
            .domain_caches
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        Arc::clone(caches.entry(workspace.to_path_buf()).or_default())
    }

    /// Workspace delta ring, created on first use. `workspace` must be
    /// canonical — S6 isolation key (hello bind supplies it). [`Arc`] so a
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

    /// Take a live subscription claim on `workspace`'s ring (created on first
    /// use — same key discipline as [`ring`](Self::ring)). The one claim door
    /// for the `sub` dispatch: the reaper exemption (U20b) is engaged from the
    /// moment this returns.
    ///
    /// The claim is taken while the `rings` map lock is held — the same lock
    /// [`reap`](Self::reap) holds for its decide-and-remove — so claim and
    /// removal are linearized: a claim that lands first is seen and exempts
    /// the workspace; a reap that lands first has already removed the ring,
    /// and the claim mints the fresh epoch that IS the workspace's live ring.
    /// A fetch-then-claim through [`ring`](Self::ring) has no such guarantee
    /// (the fetched ring can be orphaned before the claim lands), which is
    /// why the dispatch claims here and not there.
    #[must_use]
    pub fn subscribe(&self, workspace: &Path) -> crate::ring::SubGuard {
        let mut rings = self.rings.lock().unwrap_or_else(PoisonError::into_inner);
        let ring = rings.entry(workspace.to_path_buf()).or_insert_with(|| {
            Arc::new(crate::ring::WorkspaceRing::new(&fs::WorkspaceRoot(
                workspace.to_path_buf(),
            )))
        });
        ring.subscribe()
    }

    /// Read-is-the-mint ledger (S6), created on first use. `workspace` must be
    /// canonical (same key as `engines`/`inner`). [`Arc`] so a slow read never
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
    /// An armed `sub` connection is activity for idle-exit, but not traffic for
    /// the pre-warm backoff — nothing is asked of the engine, so that cadence
    /// must still be allowed to decay.
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
    /// that moved means traffic arrived between two sweeps, which a
    /// one-second-granular clock can miss.
    #[must_use]
    pub fn request_count(&self) -> u64 {
        self.requests.load(Ordering::Relaxed)
    }

    /// Unix seconds of the last client request — or of daemon start, when there
    /// has been none, so an idle-exit check cannot fire immediately.
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
    /// The claim behind the exemption is taken at arm time, inside the `sub`
    /// dispatch and before the ack renders (`server::arm_time_exemption_tests`)
    /// — and it is taken under the `rings` map lock ([`Self::subscribe`]), the
    /// same lock this sweep holds while it decides and removes. Claim and reap
    /// are linearized: there is no exemption snapshot a landing claim can
    /// trail. Lock order is `inner` → `rings`; no path takes them the other
    /// way.
    pub fn reap(&self, now: u64, threshold_secs: u64) -> Vec<PathBuf> {
        let cutoff = now.saturating_sub(threshold_secs);
        // Test-only: park here when the gate is armed (see the field docs).
        #[cfg(test)]
        {
            let gate = self
                .pause_in_reap_window
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            if let Some((arrived, release)) = gate {
                let _ = arrived.send(());
                let _ = release.recv();
            }
        }
        let reaped: Vec<PathBuf> = {
            let mut map = self.inner.write().unwrap_or_else(PoisonError::into_inner);
            let mut rings = self.rings.lock().unwrap_or_else(PoisonError::into_inner);
            let reaped: Vec<PathBuf> = map
                .iter()
                .filter(|(key, entry)| {
                    entry.last_use <= cutoff
                        && !rings.get(*key).is_some_and(|ring| ring.has_subscribers())
                })
                .map(|(key, _)| key.clone())
                .collect();
            for key in &reaped {
                map.remove(key);
                // Ring dies inside the same critical section its exemption was
                // decided in — a later `sub` gets a fresh epoch, never an
                // orphaned ring a concurrent claim is riding.
                rings.remove(key);
            }
            drop(rings);
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
            // The leaf memo is a projection of the engine it serves: it dies on
            // the same horizon, so a re-warmed workspace re-reads its members
            // rather than trusting digests from before the gap.
            let mut caches = self
                .domain_caches
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            for key in &reaped {
                caches.remove(key);
            }
            drop(caches);
            // The sql handle rides the same horizon; the FILE stays (its pin
            // is content-derived, re-warm re-compares before serving).
            let mut stores = self
                .sql_stores
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            for key in &reaped {
                stores.remove(key);
            }
        }
        reaped
    }

    /// Persist the current map to the state file, logging (never failing) on a
    /// write error — a lost persist only costs a warm registration across
    /// restart.
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

        assert_eq!(
            reg.warm_or_build(&ws).unwrap(),
            WarmOutcome::Built { docs: 2 },
            "first warm builds the corpus"
        );
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
            WarmOutcome::Built { docs: 1 },
            "a corpus change rebuilds once, parsing only the mover"
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

    /// The reap's exemption window (pre-existing; disclosed by PR #28's
    /// review): the exemption set was read before the decide-and-remove, so a
    /// claim landing between the two could still see its workspace reaped and
    /// its ring dropped from the map. The next writer would mint a second
    /// ring — the per-workspace `seq` forks (§4.7) and delivery silently
    /// dies. Deterministic via the `pause_in_reap_window` seam (the PR #9
    /// `pause_before_insert` precedent, disclosed): the reap parks in its
    /// window, the claim lands, the reap resumes.
    #[test]
    fn a_claim_landing_in_the_reap_window_keeps_workspace_and_ring() {
        let home = tempfile::tempdir().unwrap();
        let reg = Arc::new(registry_in(home.path()));
        let ws = write_ws(home.path(), &[("a.md", "# A\n")]);
        let canonical = workspace::canonicalize(&ws).unwrap();
        reg.register(&canonical);

        // Arm the one-shot reap-window gate for the reap pass (thread A).
        let (arrived_tx, arrived) = std::sync::mpsc::channel();
        let (release, release_rx) = std::sync::mpsc::channel();
        *reg.pause_in_reap_window
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some((arrived_tx, release_rx));

        // A: the widest-horizon reap, parked in its exemption window.
        let a = {
            let reg = Arc::clone(&reg);
            std::thread::spawn(move || reg.reap(u64::MAX, 0))
        };
        arrived.recv().expect("the reap reached its window gate");

        // The claim lands while the reap is parked — from here the client
        // holds an honored subscription (the `sub` ack renders on it).
        let guard = reg.subscribe(&canonical);

        release
            .send(())
            .expect("the reap parked on the release gate");
        let reaped = a.join().expect("the reap panicked");

        assert!(
            !reaped.contains(&canonical),
            "a claimed workspace is not reaped, even by a reap already past \
             its exemption reading when the claim landed: {reaped:?}"
        );
        assert!(
            Arc::ptr_eq(guard.ring(), &reg.ring(&canonical)),
            "the claimed ring IS the workspace's live ring — a second ring \
             would fork the per-workspace seq counter §4.7 defines"
        );

        drop(guard);
        let reaped = reg.reap(u64::MAX, 0);
        assert!(
            reaped.contains(&canonical),
            "dropping the claim restores mortality (the survival above was \
             the claim's doing): {reaped:?}"
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

    /// Write `bytes` to `rel`, past the filesystem's timestamp granularity —
    /// a same-tick rewrite would be testing the stat memo's blind spot.
    fn rewrite(ws: &Path, rel: &str, bytes: &str) {
        std::thread::sleep(std::time::Duration::from_millis(10));
        fs::write(ws.join(rel), bytes).unwrap();
    }

    /// The whole-corpus pass stays exact: `warm_or_build` is what `fingerprint`
    /// and the ambient root go through, and it still moves on any member's
    /// change — including one no read has asked about.
    #[test]
    fn the_corpus_pass_still_sees_every_change() {
        let home = tempfile::tempdir().unwrap();
        let reg = registry_in(home.path());
        let ws = write_ws(home.path(), &[("a.md", "# A\n"), ("b.md", "# B\n")]);
        let canonical = workspace::canonicalize(&ws).unwrap();

        reg.warm_or_build(&ws).unwrap();
        let before = reg.with_engine(&canonical, |e| e.unwrap().at_fingerprint.clone());

        rewrite(&canonical, "b.md", "# B moved\n");
        assert_eq!(
            reg.warm_or_build(&ws).unwrap(),
            WarmOutcome::Built { docs: 1 },
            "the corpus pass is not memo-blind — and re-parses only the mover"
        );
        let after = reg.with_engine(&canonical, |e| e.unwrap().at_fingerprint.clone());
        assert_ne!(before, after, "and the ambient root advanced");
    }

    /// The p1-warm-or-build-race negative proof. Interleaving A-snapshot ·
    /// B-snapshot · B-insert · A-insert, forced deterministically: thread A
    /// parks on the armed `pause_before_insert` gate with its stale engine
    /// built but not yet inserted; the corpus moves and B warms to completion
    /// while A is parked; then A is released. A's insert must not regress the
    /// resident engine to the older corpus state — answers served from it in
    /// the warm-to-serve gap would be wrong-results class.
    #[test]
    fn a_parked_stale_rebuild_cannot_regress_the_resident_engine() {
        let home = tempfile::tempdir().unwrap();
        let reg = Arc::new(registry_in(home.path()));
        let ws = write_ws(home.path(), &[("a.md", "# A v1\n")]);
        let canonical = workspace::canonicalize(&ws).unwrap();

        // Arm the one-shot gate for the first rebuild pass (thread A).
        let (arrived_tx, arrived) = std::sync::mpsc::channel();
        let (release, release_rx) = std::sync::mpsc::channel();
        *reg.pause_before_insert
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some((arrived_tx, release_rx));

        // A: snapshots the corpus at v1, parks before its insert.
        let a = {
            let reg = Arc::clone(&reg);
            let ws = ws.clone();
            std::thread::spawn(move || reg.warm_or_build(&ws))
        };
        arrived.recv().expect("thread A reached the pause gate");

        // The corpus moves to v2 and B warms to completion: the resident
        // engine is now the v2 build (the gate is consumed; B passes through).
        rewrite(&ws, "a.md", "# A v2\n");
        assert_eq!(
            reg.warm_or_build(&ws).unwrap(),
            WarmOutcome::Built { docs: 1 },
            "B rebuilds at v2 while A is parked"
        );
        let v2 = reg.with_engine(&canonical, |e| e.unwrap().at_fingerprint.clone());

        // Release A: its build is from the older disk state.
        release
            .send(())
            .expect("thread A parked on the release gate");
        a.join().expect("thread A panicked").unwrap();

        let resident = reg.with_engine(&canonical, |e| e.unwrap().at_fingerprint.clone());
        assert_eq!(
            resident, v2,
            "a stale concurrent rebuild must never regress the resident engine"
        );
        assert_eq!(
            reg.warm_or_build(&ws).unwrap(),
            WarmOutcome::Reused,
            "disk is unchanged since the v2 build, so the next warm reuses — \
             a Built here is the self-heal of a regressed engine"
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

    /// The incremental review bar: after `label`, the registry's resident
    /// engine (incrementally maintained) must be INDISTINGUISHABLE from a
    /// cold registry's from-scratch build of the same tree — root, docs,
    /// index, unserved, and the recorded leaf set all equal.
    fn assert_matches_scratch(reg: &Registry, ws: &Path, label: &str) {
        let canonical = workspace::canonicalize(ws).unwrap();
        let scratch_home = tempfile::tempdir().unwrap();
        let scratch = registry_in(scratch_home.path());
        scratch.warm_or_build(ws).unwrap();

        let incremental = reg
            .engine_snapshot(&canonical)
            .expect("incremental engine resident");
        let fresh = scratch
            .engine_snapshot(&canonical)
            .expect("scratch engine resident");

        assert_eq!(
            incremental.at_fingerprint, fresh.at_fingerprint,
            "{label}: incremental stamp equals the from-scratch fold"
        );
        assert_eq!(
            incremental.leaves, fresh.leaves,
            "{label}: recorded leaf sets equal"
        );
        assert_eq!(
            incremental.docs.keys().collect::<Vec<_>>(),
            fresh.docs.keys().collect::<Vec<_>>(),
            "{label}: same document set"
        );
        for (rel, doc) in &incremental.docs {
            assert_eq!(
                doc.raw, fresh.docs[rel].raw,
                "{label}: {rel} carries the same bytes"
            );
        }
        assert_eq!(
            incremental.unserved, fresh.unserved,
            "{label}: same unserved map"
        );
        assert_eq!(incremental.index, fresh.index, "{label}: same name index");
    }

    /// The (a)-shape equivalence gate: one corpus evolves through every
    /// mutation kind — modify, add, remove, rename, degrade to non-UTF-8,
    /// recover — and after every step the incrementally-maintained engine
    /// equals a from-scratch build. The outcome assertions pin the O(delta)
    /// parse claim: each pass parses exactly the movers.
    #[test]
    fn incremental_rebuild_equals_from_scratch_across_mutation_kinds() {
        let home = tempfile::tempdir().unwrap();
        let reg = registry_in(home.path());
        let ws = write_ws(
            home.path(),
            &[
                ("a.md", "# A\n\nsee [[b]]\n"),
                ("b.md", "# B\n"),
                ("sub/c.md", "# C\n\nsee [[a]]\n"),
            ],
        );

        assert_eq!(
            reg.warm_or_build(&ws).unwrap(),
            WarmOutcome::Built { docs: 3 },
            "cold build parses the whole corpus"
        );
        assert_matches_scratch(&reg, &ws, "cold build");

        rewrite(&ws, "a.md", "# A changed\n\nsee [[b]] and [[c]]\n");
        assert_eq!(
            reg.warm_or_build(&ws).unwrap(),
            WarmOutcome::Built { docs: 1 },
            "modify parses the one mover"
        );
        assert_matches_scratch(&reg, &ws, "modify");

        rewrite(&ws, "d.md", "# D\n\naliased\n");
        assert_eq!(
            reg.warm_or_build(&ws).unwrap(),
            WarmOutcome::Built { docs: 1 },
            "add parses the one new member"
        );
        assert_matches_scratch(&reg, &ws, "add");

        fs::remove_file(ws.join("b.md")).unwrap();
        assert_eq!(
            reg.warm_or_build(&ws).unwrap(),
            WarmOutcome::Built { docs: 0 },
            "remove parses nothing"
        );
        assert_matches_scratch(&reg, &ws, "remove");

        std::thread::sleep(std::time::Duration::from_millis(10));
        fs::rename(ws.join("sub/c.md"), ws.join("sub/c2.md")).unwrap();
        assert_eq!(
            reg.warm_or_build(&ws).unwrap(),
            WarmOutcome::Built { docs: 1 },
            "rename parses the one member under its new name"
        );
        assert_matches_scratch(&reg, &ws, "rename");

        std::thread::sleep(std::time::Duration::from_millis(10));
        fs::write(ws.join("d.md"), [0xff, 0xfe, 0x00, 0x01]).unwrap();
        assert_eq!(
            reg.warm_or_build(&ws).unwrap(),
            WarmOutcome::Built { docs: 0 },
            "a member degrading to non-UTF-8 parses nothing (it is unserved)"
        );
        assert_matches_scratch(&reg, &ws, "degrade to non-UTF-8");

        rewrite(&ws, "d.md", "# D again\n");
        assert_eq!(
            reg.warm_or_build(&ws).unwrap(),
            WarmOutcome::Built { docs: 1 },
            "a member recovering from non-UTF-8 is parsed again"
        );
        assert_matches_scratch(&reg, &ws, "recover from non-UTF-8");
    }

    /// A rewrite that restores the exact prior bytes moves every stat clock,
    /// yet the fold is unchanged — the warm must reuse, not rebuild: the
    /// boundary's freshness is content, never time.
    #[test]
    fn a_touch_that_restores_bytes_reuses() {
        let home = tempfile::tempdir().unwrap();
        let reg = registry_in(home.path());
        let ws = write_ws(home.path(), &[("a.md", "# A\n"), ("b.md", "# B\n")]);

        reg.warm_or_build(&ws).unwrap();
        rewrite(&ws, "a.md", "# A\n");
        assert_eq!(
            reg.warm_or_build(&ws).unwrap(),
            WarmOutcome::Reused,
            "same bytes ⇒ same fold ⇒ reuse, whatever the clocks say"
        );
    }
}
