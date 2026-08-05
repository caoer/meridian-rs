//! The in-memory registry: a mutex-guarded map keyed by canonical workspace
//! path, plus the register / resolve / unregister / list / reap operations.
//!
//! The map lock is the serialization point. Every mutation takes the write
//! lock; the whole register critical section (deny check → sentinel write →
//! insert → persist) runs under one held guard, so two concurrent registrars
//! for the same path serialize: the first creates the entry and the drawer
//! sentinel, the second sees the key present and adopts it. One registry
//! entry, one drawer sentinel — first-writer-wins **by serialization**
//! (decision 0001 round 5, point 1).

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

/// The daemon's workspace registry: the guarded map plus the state store and
/// the drawer cache root.
///
/// `engines` is the resident query-engine state (decision 0002 spine root, U1):
/// a warm `WorkspaceEngine` per workspace, parallel to `inner` and keyed by the
/// same canonical `PathBuf`. It is a disposable projection of disk — never
/// persisted, never loaded at start (a cold daemon holds no engines) — so it is
/// NOT part of the state file. Idle-reap drops a warm engine alongside the
/// registration it belongs to.
#[derive(Debug)]
pub struct Registry {
    inner: RwLock<HashMap<PathBuf, WorkspaceEntry>>,
    engines: RwLock<HashMap<PathBuf, WorkspaceEngine>>,
    /// V2 §Q2 the per-workspace **publish mutex** (OD6/B1). The daemon is the
    /// sole persistent writer; `view::publish` adds zero flock and assumes the
    /// caller holds this. One `Mutex` per workspace, created on first publish,
    /// held across build + post-build live sample so a concurrent `view_path`
    /// for the same workspace serializes (never two `rename`s racing one
    /// drawer). Distinct workspaces never contend.
    publish_locks: Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>,
    /// OD7 refresh telemetry per workspace (daemon memory, lost on restart;
    /// advisory, never correctness). Also the daemon-memory `as_of` proxy: the
    /// last successful publish fingerprint, sole-writer-faithful within one
    /// epoch.
    refresh_state: RwLock<HashMap<PathBuf, RefreshState>>,
    /// Stage-2 S6 the read-is-the-mint ledger, one per workspace (D6/H1).
    ///
    /// **Deliberately NOT a field on [`WorkspaceEngine`]**: `warm_or_build`
    /// replaces a workspace's engine on every corpus content-hash change, so a
    /// receipt held inside the engine would be evaporated by the very write the
    /// receipt authorized (a pin writes). This map sits beside `engines` and no
    /// rebuild touches it. Daemon memory only — never persisted, never in the
    /// state file — and dropped alongside the registration on the ONE idle-reap
    /// horizon.
    read_mints: Mutex<HashMap<PathBuf, Arc<receipt::read_mint::ReadMintStore>>>,
    /// U20b the delta plane, one ring per workspace — keyed, created and reaped
    /// EXACTLY like [`Self::read_mints`], and for the same reason: it is session
    /// memory that must survive a corpus rebuild but must not outlive the
    /// registration.
    ///
    /// **S6 — one ring per workspace, never one global ring.** The key is the
    /// canonical workspace path, so two spellings of one workspace share a ring
    /// and two workspaces can never see each other's frames. A global ring here
    /// would leak every path, hpath and rev of every workspace to every
    /// subscriber.
    rings: Mutex<HashMap<PathBuf, Arc<crate::ring::WorkspaceRing>>>,
    /// This daemon instance's epoch token — the `built_epoch` half of a
    /// published stamp (pairs with `seq`). A per-instance value (a restart mints
    /// a new one).
    epoch: String,
    /// G11 the pre-warm quiet map: the last [`fs::domain_stat_signature`] each
    /// warm workspace was swept at. A sweep whose signature matches its entry
    /// skips the corpus snapshot entirely — the difference between one `stat`
    /// per domain file and reading plus folding every byte of a 20 GB vault,
    /// once a second, forever.
    ///
    /// Written only by [`Self::prewarm`], so it is never load-bearing: an entry
    /// that is missing or stale costs one extra snapshot, never a wrong answer.
    /// Correctness stays the content root inside `engines`.
    prewarm_signatures: Mutex<HashMap<PathBuf, u64>>,
    /// G11 the activity clock: how many client requests this daemon has served,
    /// and when the last one landed (unix seconds). Bumped by the socket's
    /// dispatch, read by the pre-warm backoff (traffic means work is happening,
    /// so sweep eagerly again) and by the idle-exit check.
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
            // Cold start holds no warm engines: residency is a disposable
            // projection, rebuilt from disk on the first `warm_or_build`.
            engines: RwLock::new(HashMap::new()),
            publish_locks: Mutex::new(HashMap::new()),
            refresh_state: RwLock::new(HashMap::new()),
            // Session memory starts empty: a cold daemon has minted nothing.
            read_mints: Mutex::new(HashMap::new()),
            // Nor is anyone subscribed: a cold daemon's every ring is unborn,
            // so every pre-restart `from_seq` is outside retained history and
            // answers `root_unknown` → resync (§7.1 late law).
            rings: Mutex::new(HashMap::new()),
            // A per-instance epoch stamp: restart mints a new one, so a stale
            // cross-restart `built_epoch` never reads as current.
            epoch: now_secs().to_string(),
            // No workspace has been swept yet, so the first sweep of each is a
            // full snapshot — the signature is a skip-gate, not a cold-start
            // assumption.
            prewarm_signatures: Mutex::new(HashMap::new()),
            requests: AtomicU64::new(0),
            // A daemon nobody ever calls must still age out, so the clock starts
            // at birth rather than at zero.
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

    /// Pin storage for a **declared** root — the `hello` frame's `workspace`
    /// field (decision 0002 §4, U3; marker-retirement ruling, 2026-07-26).
    ///
    /// **Exact, or refuse.** The declared path is pinned at exactly that path:
    /// no ancestor walk, so a declaration can never widen to an enclosing
    /// registered workspace. This is the `26-13` jail premise — the workspace
    /// root IS the read/put jail root, one explicit already-enforced path — and
    /// a declaration that could silently bind an ancestor would not be that.
    ///
    /// Registration reuses [`register`](Self::register) whole: the SAME
    /// canonicalize → deny-ceiling → drawer-sentinel path (risk R2: reuse the
    /// one registration path, never a second copy). An already-registered
    /// declared path is adopted **at that same path**, which is reuse, not
    /// widening.
    ///
    /// # Why exact, rather than keeping the ancestor walk here
    /// The walk looked like a nesting guard and was not one:
    /// [`register`](Self::register) has no equal-or-nested refusal (unlike the
    /// mount table's INV-4), so registering `/a/b/c` before `/a` leaves both
    /// registered anyway. The walk therefore only made the outcome depend on
    /// registration *order* — not a guarantee, and the cost was a silent bind.
    ///
    /// The canonical root still may not equal the declared string (symlinks and
    /// on-disk case both rewrite it), so the caller learns what actually bound
    /// from the `workspace` field of the hello response, never by assuming.
    ///
    /// Returns the canonical workspace root and its drawer directory
    /// ([`cache::drawer_dir`]). Does NOT warm the engine; the caller warms and
    /// binds the connection ([`warm_or_build`](Self::warm_or_build)).
    pub fn pin_declared(&self, root: &Path) -> PinOutcome {
        let workspace = match self.register(root) {
            RegisterOutcome::Registered(entry) | RegisterOutcome::Adopted(entry) => entry.workspace,
            RegisterOutcome::Denied(reason) => return PinOutcome::Denied(reason),
            RegisterOutcome::Error(message) => return PinOutcome::Error(message),
        };
        let drawer = cache::drawer_dir(&self.cache_root, &workspace);
        PinOutcome::Pinned { workspace, drawer }
    }

    /// Resolve + pin storage for a **cwd** — a hint, not a declaration.
    ///
    /// Here the ancestor walk is correct: a cwd is a position inside a tree, so
    /// the enclosing registered workspace is the right answer.
    /// [`resolve`](Self::resolve) walks `cwd` and its ancestors; a hit is
    /// already pinned, so its canonical path is used directly (no second
    /// registration under an already-registered root); a miss pins `cwd` fresh
    /// via [`register`](Self::register).
    ///
    /// Split from [`pin_declared`](Self::pin_declared) deliberately: one
    /// function serving both a declaration and a hint was the defect — it made
    /// widening the silent default for callers who had stated their root.
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

    /// Warm the resident query engine for `workspace`, rebuilding it ONLY when
    /// the corpus content hash has changed (decision 0002 spine root, U1).
    ///
    /// Canonicalizes `workspace` (the `engines` key is the same canonical path
    /// `inner` uses), reads + folds the corpus content hash fresh from disk
    /// (`fs::domain_snapshot` — the cheap half, no parse), and:
    /// - reuses the warm engine and parses NOTHING when the hash is unchanged
    ///   ([`WarmOutcome::Reused`]);
    /// - rebuilds the index + document map exactly once when the hash changed or
    ///   the workspace was cold ([`WarmOutcome::Built`]).
    ///
    /// The reuse key is the corpus CONTENT hash the commit guards already
    /// compute (risk R5), not the unimplemented workspace-identity Merkle. The
    /// parse-heavy `fs::build_corpus` runs on the rebuild branch alone, so a
    /// `Reused` result provably ran zero parses.
    ///
    /// The fingerprint read holds no lock, and the rebuild parses OUTSIDE the
    /// `engines` write lock — the lock is taken only for the final insert — so
    /// warming one workspace never blocks another. A rare concurrent rebuild of
    /// the same workspace is last-write-wins and still correct (generous
    /// residency, decision 0002 §2).
    ///
    /// # Errors
    /// `workspace` cannot be canonicalized (does not exist), the corpus cannot
    /// be read, or a corpus file is non-UTF-8 (refused,
    /// [`io::ErrorKind::InvalidData`]).
    pub fn warm_or_build(&self, workspace: &Path) -> io::Result<WarmOutcome> {
        let canonical = workspace::canonicalize(workspace)
            .map_err(|e| io::Error::new(io::ErrorKind::NotFound, e))?;
        let root = fs::WorkspaceRoot(canonical.clone());

        // Cheap half (no parse): read + fold the corpus content hash from disk.
        let (files, fingerprint) = fs::domain_snapshot(&root)?;

        // Warm and unchanged → reuse, ZERO parses.
        {
            let engines = self.engines.read().unwrap_or_else(PoisonError::into_inner);
            if engines
                .get(&canonical)
                .is_some_and(|engine| engine.at_fingerprint == fingerprint)
            {
                return Ok(WarmOutcome::Reused);
            }
        }

        // Cold or content changed → rebuild exactly once (the only parse site).
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

    /// Borrow the warm engine for `canonical` (an already-canonical workspace
    /// path) under the read lock, running `f` on it — `None` when no engine is
    /// resident. The daemon's read path calls [`warm_or_build`](Self::warm_or_build)
    /// first (ensuring the engine reflects current disk and is warm), then serves
    /// the borrowed state through this accessor — so the read never parses when
    /// the corpus is unchanged (U2, served from U1's resident state).
    ///
    /// The closure runs UNDER the read lock; keep it to a borrow-and-project (no
    /// blocking, no re-entrancy into the engines lock), so concurrent reads of
    /// other workspaces never wait on it.
    pub fn with_engine<R>(
        &self,
        canonical: &Path,
        f: impl FnOnce(Option<&WorkspaceEngine>) -> R,
    ) -> R {
        let engines = self.engines.read().unwrap_or_else(PoisonError::into_inner);
        f(engines.get(canonical))
    }

    /// V2 §Q2 the `view_path` op: resolve `cwd` → workspace, publish (or serve)
    /// `view.duckdb`, and return the stamped PATH plus a pre-open freshness
    /// hint — never rows. The daemon is the sole persistent builder (OD6).
    ///
    /// The whole build + post-build sample runs under the workspace's publish
    /// mutex ([`publish_lock`](Self::publish_lock)), so `view::publish`'s
    /// flock-free atomic `temp + rename` never races a concurrent `view_path`
    /// for the same workspace (B1). Branches:
    /// - `fresh` ⇒ the bounded `--fresh` rebuild (§Q3): build at `F0`, sample
    ///   `live = F_now` after, `FRESH_AT_SAMPLE` on equality, one retry, else
    ///   `RACED`;
    /// - absent file, or an unknown `as_of` (a fresh daemon that did not build
    ///   this file) ⇒ a first build + post-build sample;
    /// - a published file with a known `as_of` ⇒ **serve it, no rebuild** (a
    ///   default stale query serves last-good immediately, §Huge-corpus): the
    ///   daemon-memory `as_of` vs a fresh `live` fold decides `FRESH_AT_SAMPLE`
    ///   / `STALE`.
    ///
    /// On a publish failure with a last-good file present, the last-good path is
    /// served with the OD7 `last_error` set (telemetry, never a freshness gate);
    /// with no last-good, the wire error propagates.
    ///
    /// # Errors
    /// `bad_request` when `cwd` is refused by the deny ceiling; `io_error` when
    /// the workspace cannot be resolved, the corpus cannot be folded, or a build
    /// fails with no last-good file to serve; `invalid_utf8` for a non-UTF-8
    /// corpus.
    pub fn view_path(&self, cwd: &str, fresh: bool) -> Result<ResponseBody, Box<ErrorBody>> {
        let (workspace, drawer, dir) = self.resolve_drawer(cwd)?;
        let dest = dir.join("view.duckdb");

        // The per-workspace publish mutex, held across build + post-build sample.
        let lock = self.publish_lock(&workspace);
        let _publish = lock.lock().unwrap_or_else(PoisonError::into_inner);

        // A published file whose `as_of` this daemon knows is served WITHOUT a
        // rebuild (a default stale query serves last-good, §Huge-corpus);
        // `fresh`, an absent file, or an unknown `as_of` must build.
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
            // A failed build serves the last-good file if one exists (OD7:
            // telemetry, never a freshness gate); otherwise the error stands.
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
            // The resident daemon holds no delta ring (subscriptions are
            // round-2), so the per-epoch counter is `0` — mirroring the `Root`
            // and `Links` arms, which already report `0`.
            changes_seq: 0,
            state,
            // The daemon sampled `live` from its warm `at_fingerprint` fold: a
            // hint, never a post-result verdict (§Q3 C3 — never `fold` here).
            live_source: wire::ViewLiveSource::Watch,
            // A PRE-OPEN hint is never a verdict (B5+C3): always null.
            stale: None,
            refresh_in_progress,
            last_error,
        })
    }

    /// Resolve `cwd` to its canonical workspace, disk drawer, and drawer
    /// directory (V2 §Q2). Reuses the one canonicalize → deny-ceiling → sentinel
    /// path via [`pin_for_cwd`](Self::pin_for_cwd) — the cwd-shaped pin, since
    /// the input here really is a position inside a tree — then addresses the
    /// drawer under THIS registry's `cache_root` (never the ambient
    /// `CacheDrawer::open`, so a test cache root is honored).
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

    /// The workspace's delta ring (U20b), created on first use — the in-flight
    /// buffer a subscription reads and the detector feeds.
    ///
    /// `workspace` must already be the CANONICAL path, exactly as
    /// [`Self::read_mints`] requires and for a sharper reason: this key IS the
    /// S6 isolation boundary. The `hello` bind supplies it
    /// ([`Self::pin_declared`] → `workspace::canonicalize`), so two spellings of
    /// one workspace — a symlink, a `.`-relative path, a case variant — resolve
    /// to ONE ring, and no spelling can reach a ring that is not its own.
    ///
    /// The returned handle is an [`Arc`] so a parked subscriber never holds this
    /// map's lock; the ring has its own interior lock.
    #[must_use]
    pub fn ring(&self, workspace: &Path) -> Arc<crate::ring::WorkspaceRing> {
        let mut rings = self.rings.lock().unwrap_or_else(PoisonError::into_inner);
        Arc::clone(rings.entry(workspace.to_path_buf()).or_insert_with(|| {
            Arc::new(crate::ring::WorkspaceRing::new(&fs::WorkspaceRoot(
                workspace.to_path_buf(),
            )))
        }))
    }

    /// The workspace's read-is-the-mint ledger (stage-2 S6), created on first
    /// use — the daemon-session layer a composed read mints into and a pin gate
    /// reads back.
    ///
    /// `workspace` must already be the CANONICAL path (the same key `engines`
    /// and `inner` use, which the `hello` bind supplies): one mount, one ledger,
    /// so two workspaces holding the same relative path never answer each
    /// other's lookups. That is also the D12 seam — mount identity is THIS key,
    /// never a field inside the ledger, so a later per-root world hands out one
    /// ledger per root and the ledger itself is unchanged.
    ///
    /// The returned handle is an [`Arc`] so a slow read never holds this map's
    /// lock (the `publish_lock` pattern); the ledger has its own interior lock.
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

    /// Warm the engine (fold + parse-on-change), snapshot its fingerprint + docs
    /// OUT of the engines lock, then `view::publish` (holding the publish mutex,
    /// never the engines lock — a slow `DuckDB` build must not block another
    /// workspace's warm insert). Records OD7 success/failure telemetry. Returns
    /// the built fingerprint `F0`.
    fn publish_now(
        &self,
        workspace: &Path,
        drawer: &cache::CacheDrawer,
    ) -> Result<model::MerkleRoot, Box<ErrorBody>> {
        // Ensure the resident engine reflects current disk (the docs source).
        self.warm_or_build(workspace)
            .map_err(|e| warm_err_to_wire(&e))?;
        // Clone the fingerprint + docs out of the read lock: `view::publish`
        // does slow DuckDB I/O, and holding the engines read lock across it
        // would block a concurrent `warm_or_build` write insert (any workspace).
        let Some((f0, docs)) = self.with_engine(workspace, |engine| {
            engine.map(|e| (e.at_fingerprint.clone(), e.docs.clone()))
        }) else {
            // An idle-reap evicted the engine between warm and borrow — transient.
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

    /// Pre-warm every resident (warm) workspace, rebuilding only those whose
    /// corpus content hash changed since it was built (decision 0002, P2).
    ///
    /// This is the daemon's watch driver: the background pre-warm thread
    /// ([`spawn_prewarm`](crate::server)) calls it on an interval so a file
    /// change pays its parse HERE (the watch event), not on the next query —
    /// latency only, never correctness. Correctness stays fingerprint:
    /// [`warm_or_build`](Self::warm_or_build) reuses the warm engine when the
    /// content hash is unchanged, so a quiet sweep parses nothing, and a query
    /// arriving after a pre-warm finds the engine already warm
    /// ([`WarmOutcome::Reused`] — zero parse on the query path).
    ///
    /// It sweeps only the ALREADY-warm set (the `engines` keys): pre-warm keeps
    /// warm what is warm; a workspace is first warmed on demand by a query
    /// ([`warm_or_build`]). A cold daemon holds nothing warm, so its sweep is a
    /// no-op — crash recovery needs no machinery (start empty, the first query
    /// rebuilds from disk via the fingerprint).
    ///
    /// Best-effort: a workspace that vanished or turned non-UTF-8 between warms
    /// is skipped — pre-warm is a latency optimization, not a correctness path,
    /// so the next query re-derives from disk and reports the real error.
    /// Returns the workspaces that were REBUILT (their files changed); an
    /// unchanged sweep returns empty.
    ///
    /// The warm-key snapshot is taken under the read lock and released BEFORE
    /// any `warm_or_build`, so the sweep never holds the `engines` lock across a
    /// rebuild — no self-deadlock, and a concurrent query is never blocked.
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
            // Only a `Built` rebuilt: `Reused` reused the warm engine (nothing
            // parsed), and an `Err` (vanished or non-UTF-8 workspace) pre-warms
            // nothing — the next query re-derives from disk and surfaces the
            // real error. Both are silently best-effort.
            if let Ok(WarmOutcome::Built { .. }) = self.warm_or_build(&workspace) {
                rebuilt.push(workspace);
            }
        }
        rebuilt
    }

    /// G11: has `workspace` looked untouched since the last sweep? Records the
    /// signature it observed, so the answer is `false` exactly once per change.
    ///
    /// An unreadable workspace answers `false` — never skip on an error, or a
    /// transient `stat` failure would silently freeze a corpus's pre-warm until
    /// something else changed. The snapshot behind it is the error-reporting
    /// path, and it is best-effort there too.
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

    /// Drop entries whose `last_use` is at or before `now - threshold_secs`,
    /// persisting the survivors. Returns the reaped workspace paths.
    ///
    /// `now` and `threshold_secs` are parameters (an injectable clock), so the
    /// reap horizon is unit-testable without waiting days: a far-future `now`
    /// ages every entry past the horizon, and `threshold_secs == 0` reaps all
    /// present entries. The reaper only deregisters; it never touches the
    /// drawer (that is `cache::gc`'s separate horizon).
    ///
    /// A reaped workspace's warm engine is dropped too (decision 0002 risk R4):
    /// warm-engine eviction hangs off this ONE idle-reap horizon — no separate
    /// memory budget or eviction policy. The `engines` lock is taken AFTER the
    /// `inner` lock is released, so the two maps are never held at once.
    ///
    /// Its S6 read-mint ledger is dropped on the same horizon (session memory
    /// does not outlive the registration). This is the ONLY place a ledger is
    /// dropped — never on a corpus change, which is the whole point of holding
    /// it outside the warm engine.
    pub fn reap(&self, now: u64, threshold_secs: u64) -> Vec<PathBuf> {
        let cutoff = now.saturating_sub(threshold_secs);
        // U20b — **a live subscription is USE.** A subscribed connection is
        // push-only by design: it sends no requests, so it makes no `last_use`
        // touch and idles straight past the cutoff.
        //
        // What reaping it costs, measured rather than assumed: NOT a dead
        // stream. The subscriber holds an `Arc` to its ring, so it keeps
        // detecting and delivering perfectly well. The cost is a FORK — the map
        // entry is gone, so the next `sub` on this workspace mints a SECOND ring
        // with its own epoch and counter, and `seq` stops being the monotone
        // per-workspace counter §4.7 defines. The orphan also folds the corpus
        // for as long as its holder lives, invisible to this reaper because it
        // is no longer in the map to be found.
        //
        // Subscribers are read before taking the `inner` write lock and the
        // removal happens under it, so the exemption cannot be computed from a
        // set that a concurrent `sub` has already invalidated.
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
            // The ring dies on the SAME horizon (U20b, advisor-ratified): the
            // ring is transport, not memory, and session transport does not
            // outlive the registration any more than session receipts do. A
            // later `sub` on this workspace gets a fresh epoch whose every
            // pre-reap `from_seq` answers `root_unknown` → resync.
            let mut rings = self.rings.lock().unwrap_or_else(PoisonError::into_inner);
            for key in &reaped {
                rings.remove(key);
            }
        }
        reaped
    }

    /// Workspaces with at least one live subscription — the reaper's exemption
    /// set. Rings with no subscribers are ordinary reap candidates: an unwatched
    /// ring is holding bytes nobody will ever read.
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
    //! Resident-engine gates (decision 0002 spine root, U1): warm reuse parses
    //! nothing, a corpus change forces exactly one rebuild, the warm engine
    //! answers a real query, and idle-reap drops the warm engine.

    use super::*;
    use crate::state::StateStore;
    use std::fs;

    /// A registry rooted under `home` (state file + cache drawer root). No
    /// socket — `warm_or_build` is an in-process method (the socket is U2/U3).
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

        // Cold → build: two files, two parses (one syntax::parse per doc).
        assert_eq!(
            reg.warm_or_build(&ws).unwrap(),
            WarmOutcome::Built { docs: 2 },
            "first warm builds the corpus"
        );
        // Unchanged corpus content hash → reuse. `Reused` is the parse-count
        // proof: `fs::build_corpus` (the only parse site) is reached on the
        // rebuild branch alone, so a `Reused` result ran ZERO parses.
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

        // Mutate one file → the corpus content hash changes.
        fs::write(ws.join("a.md"), "# A changed\n\nnew body\n").unwrap();

        // Exactly ONE rebuild on the next warm...
        assert_eq!(
            reg.warm_or_build(&ws).unwrap(),
            WarmOutcome::Built { docs: 2 },
            "a corpus change rebuilds once"
        );
        // ...then warm reuse resumes at the new fingerprint.
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

        // The resident index + docs answer a real `query::links` — the warm
        // state is genuinely usable, not just present (U2's read surface).
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

        // A registration gives the reaper an `inner` entry to age out; warm the
        // engine alongside it.
        reg.register(&canonical);
        reg.warm_or_build(&canonical).unwrap();
        assert!(
            reg.engines.read().unwrap().contains_key(&canonical),
            "engine warm before reap"
        );

        // Reap the whole warm set (far-future now, zero horizon) — the entry AND
        // its warm engine are dropped on the ONE idle-reap horizon (risk R4).
        let reaped = reg.reap(u64::MAX, 0);
        assert!(reaped.contains(&canonical), "the entry was reaped");
        assert!(
            !reg.engines.read().unwrap().contains_key(&canonical),
            "reap drops the warm engine with the registration"
        );
    }

    /// P2 gate 1 (latency): a file change pre-warms the resident engine on the
    /// watch event, so the next query pays ZERO parse. `prewarm` is the driver
    /// the daemon's background thread calls; testing it directly makes the
    /// warm-vs-cold trace deterministic (the reaper is unit-tested the same way).
    #[test]
    fn prewarm_absorbs_the_change_so_the_next_query_parses_nothing() {
        let home = tempfile::tempdir().unwrap();
        let reg = registry_in(home.path());
        let ws = write_ws(
            home.path(),
            &[("a.md", "# A\n\nsee [[b]]\n"), ("b.md", "# B\n")],
        );
        let canonical = workspace::canonicalize(&ws).unwrap();

        // A query warms the workspace — it is now resident (two parses).
        assert_eq!(
            reg.warm_or_build(&ws).unwrap(),
            WarmOutcome::Built { docs: 2 }
        );
        // A quiet pre-warm sweep parses nothing: an unchanged corpus rebuilds
        // none (correctness stays fingerprint).
        assert!(reg.prewarm().is_empty(), "a quiet sweep rebuilds nothing");

        // An external edit lands on disk.
        fs::write(ws.join("a.md"), "# A changed\n\nnew body\n").unwrap();

        // The pre-warm sweep IS the watch event: the changed workspace rebuilds
        // HERE, so the parse is paid off the query path.
        assert_eq!(
            reg.prewarm(),
            vec![canonical],
            "the edit rebuilds on the watch event, not lazily on the query"
        );

        // The next query finds the engine already warm — ZERO parse. `Reused` is
        // the parse-count proof: the only parse site (`fs::build_corpus`) is the
        // rebuild branch, not reached here.
        assert_eq!(
            reg.warm_or_build(&ws).unwrap(),
            WarmOutcome::Reused,
            "the query after a pre-warm parses nothing — latency moved to the watch event"
        );
    }

    /// P2 gate 2 (crash recovery, no new machinery): a daemon crash drops the
    /// disposable resident state; a fresh daemon starts cold (its sweep is a
    /// no-op) and the first query recomputes correctly from disk via the
    /// fingerprint — the doctrine's "start empty, recompute from disk, diff."
    #[test]
    fn crash_recovery_rebuilds_from_disk_with_no_added_machinery() {
        let home = tempfile::tempdir().unwrap();
        let ws = write_ws(
            home.path(),
            &[("a.md", "# A\n\nsee [[b]]\n"), ("b.md", "# B\n")],
        );

        // A daemon warms the workspace, sweeps quiet, then CRASHES — the resident
        // state is a disposable projection, so dropping it persists nothing.
        {
            let reg = registry_in(home.path());
            reg.warm_or_build(&ws).unwrap();
            assert!(reg.prewarm().is_empty(), "warm + quiet before the crash");
        } // reg dropped == crash

        // Files changed while the daemon was down (no watcher ran): one edit plus
        // one brand-new file.
        fs::write(ws.join("a.md"), "# A\n\nsee [[b]] and [[c]]\n").unwrap();
        fs::write(ws.join("c.md"), "# C\n").unwrap();

        // A fresh daemon starts COLD: it holds no warm engines, so its pre-warm
        // sweep is a no-op — recovery adds nothing.
        let reg = registry_in(home.path());
        assert!(
            reg.prewarm().is_empty(),
            "a cold daemon holds no warm engines to sweep"
        );

        // The first query recomputes from disk via the fingerprint: three docs
        // now, and the resident index reflects the on-disk truth.
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
