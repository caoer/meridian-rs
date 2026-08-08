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
    engines: RwLock<HashMap<PathBuf, WorkspaceEngine>>,
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
    /// Same lifetime as the engine: dropped on idle-reap.
    domain_caches: Mutex<HashMap<PathBuf, fs::DomainCache>>,
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
            // Cold: no rings; pre-restart `from_seq` ⇒ `root_unknown` (§7.1).
            rings: Mutex::new(HashMap::new()),
            prewarm_signatures: Mutex::new(HashMap::new()),
            // Cold: no memo; the first currency pass reads every member once.
            domain_caches: Mutex::new(HashMap::new()),
            requests: AtomicU64::new(0),
            // Clock starts at birth so idle-exit can age an unused daemon.
            last_request: AtomicU64::new(now_secs()),
            // Cold: the first `mounts` call derives the table.
            mounts: crate::mounts::MountsCache::default(),
            state,
            cache_root,
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

        // Cheap half (no parse, and no re-read of anything that did not move):
        // content hash from disk through the leaf memo.
        let fingerprint = {
            let mut caches = self
                .domain_caches
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            caches.entry(canonical.clone()).or_default().root(&root)?
        };

        // Warm + unchanged → reuse, zero parses. Nothing is copied out of the
        // memo on this path: it is the hot one, and a 20k-entry clone per
        // currency pass would put back a slice of what the memo just removed.
        {
            let engines = self.engines.read().unwrap_or_else(PoisonError::into_inner);
            if engines
                .get(&canonical)
                .is_some_and(|engine| engine.at_fingerprint == fingerprint)
            {
                return Ok(WarmOutcome::Reused);
            }
        }

        // Cold or content changed → rebuild once (only parse site). The rebuild
        // re-reads from disk rather than from the memo: `docs` must be the bytes
        // it parsed, and `domain_snapshot`'s fold is the byte-derived one, so
        // the reuse key a served answer is stamped with never comes from a
        // digest the memo carried forward.
        let (files, fingerprint) = fs::domain_snapshot(&root)?;
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
            WarmOutcome::Built { docs: 2 },
            "the corpus pass is not memo-blind"
        );
        let after = reg.with_engine(&canonical, |e| e.unwrap().at_fingerprint.clone());
        assert_ne!(before, after, "and the ambient root advanced");
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
