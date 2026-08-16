//! In-memory registry: map keyed by canonical workspace path, plus
//! register / resolve / unregister / list / reap.
//!
//! Map write lock is the serialization point. Register critical section
//! (deny → sentinel → insert → persist) holds one guard so concurrent
//! registrars for the same path first-writer-win by serialization.

use std::collections::{BTreeSet, HashMap};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError, RwLock, Weak};
use std::time::Duration;

use crate::engine::{WarmOutcome, WorkspaceEngine};
use crate::feed;
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
/// start holds none. Idle-reap drops the engine; the registration — and with
/// it the §6.4 feed and the resident memo — survives (merkle-spec §6.4).
#[derive(Debug)]
pub struct Registry {
    inner: RwLock<HashMap<PathBuf, WorkspaceEntry>>,
    engines: RwLock<HashMap<PathBuf, Arc<WorkspaceEngine>>>,
    /// U20b delta plane, one ring per workspace — created on first use,
    /// dropped on idle-reap like [`Self::engines`]. S6: key is canonical
    /// path (not a global ring).
    rings: Mutex<HashMap<PathBuf, Arc<crate::ring::WorkspaceRing>>>,
    /// G11 pre-warm quiet map: last [`fs::domain_stat_signature`] per warm
    /// workspace. Matching signature skips the corpus fold. Advisory only —
    /// missing/stale costs one extra snapshot, never a wrong answer.
    prewarm_signatures: Mutex<HashMap<PathBuf, u64>>,
    /// Per-workspace §12.2 leaf memo + resident tree — what makes a currency
    /// pass cost one `stat` per domain member instead of a re-read of the
    /// whole corpus. Registration-lifetime under a live §6.4 feed: it
    /// survives the idle-reap (the feed's dirty set covers the cold gap, so
    /// the re-warm is O(dirty)); with no live feed it drops on reap as it
    /// always did. Each entry is its own `Arc<Mutex<…>>` so the run plane
    /// can borrow ONE workspace's memo for its bracket observations (card
    /// run-observation-unification) without holding the map — and so one
    /// workspace's pass never serializes another's.
    domain_caches: Mutex<HashMap<PathBuf, Arc<Mutex<fs::DomainCache>>>>,
    /// The §6.4 event feed per workspace (kernel watcher + registry-held
    /// dirty set). Registration-lifetime (kimi D1): created with the
    /// workspace's first resident state, kept across every idle-reap,
    /// dropped at `unregister` — which is exactly what makes retaining
    /// [`Self::domain_caches`] across a reap sound: the feed covers the cold
    /// gap. A slot that failed to start is sticky-Failed, loud once; that
    /// workspace keeps the pre-feed semantics (memo drops on reap).
    feeds: Mutex<HashMap<PathBuf, FeedSlot>>,
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
    /// The §6.5 checkpoint receipt of each workspace's last restore — what
    /// the file delivered (rows adopted, members replayed, whether the cursor
    /// anchored, the labeled re-baseline if it did not). The card's published
    /// counters; empty for a workspace that started genuinely cold.
    checkpoints: Mutex<HashMap<PathBuf, crate::checkpoint::CheckpointReceipt>>,
    /// § A.5 mount-table cache. Machine-scoped (not per-workspace): the
    /// binding file lives outside every workspace's hash domain, so no
    /// engine or ring can carry it.
    mounts: crate::mounts::MountsCache,
    /// §3.2 cold-read law: background drawer rebuilds in flight plus the
    /// cause of the last rebuild that failed, per workspace. One mutex so
    /// the kick / finish / fail transitions stay atomic.
    cold_builds: Mutex<ColdBuilds>,
    /// Signaled by every background rebuild exit (landed, failed, or
    /// panicked) — what the kicking read's bounded wait parks on.
    cold_builds_done: Condvar,
    /// Self-handle for the background rebuild thread the cold gate spawns.
    /// Dead (`Weak::new()`) on a bare [`Registry::new`] — the in-process
    /// lane (`in_process_registry`: the CLI direct lane and fixtures), where
    /// [`Registry::cold_gate`] answers `Serve` and the caller builds inline;
    /// with no daemon and no deadline on the other end, blocking is the
    /// honest answer there. [`Registry::new_shared`] (the daemon) binds it.
    myself: Weak<Registry>,
    state: StateStore,
    cache_root: PathBuf,
    /// Test-only pause gate for the rebuild race window. When armed, the next
    /// rebuild pass announces itself on the first channel, then parks on the
    /// second — between its disk snapshot and its `engines` insert, the exact
    /// window the insert guard must protect. One-shot: the pass that hits it
    /// consumes it. `cfg(test)` excludes it from every release build by
    /// construction (disclosed; RC1-precedent seam).
    #[cfg(test)]
    pub(crate) pause_before_insert:
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

/// §3.2: how long the READ THAT KICKED a cold rebuild absorbs it before
/// refusing `corpus_warming`. Small drawers land inside this and first
/// contact serves; a drawer still rebuilding past it is the long kind the
/// refusal exists for. Engine-internal on purpose — the contract publishes
/// the bound's ORDER, never its value: well under any sane host op deadline
/// (ccc-statusd's D4 floor is 10 s), well over a small corpus build.
const COLD_BUILD_WAIT: Duration = Duration::from_secs(2);

/// §3.2 cold-read state: which workspaces have a background drawer rebuild
/// running, and the cause of the last rebuild that failed.
#[derive(Debug, Default)]
struct ColdBuilds {
    /// Workspaces with a background rebuild in flight — the single-flight
    /// key: however many callers ask, one rebuild runs per workspace.
    in_flight: BTreeSet<PathBuf>,
    /// The last background rebuild failure per workspace. Served (and
    /// cleared) by the next cold read as `io_error{cause}` — warming never
    /// masks a broken corpus; the read after that kicks a fresh rebuild.
    failed: HashMap<PathBuf, String>,
}

/// What the §3.2 cold gate tells a serving door to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColdGate {
    /// Proceed to the inline pass: a resident engine exists (the currency
    /// pass is incremental), or this registry has no background substrate
    /// (in-process lane — build inline, today's behavior).
    Serve,
    /// No resident engine; the drawer rebuild is running in the background
    /// and did not land inside the kicker's bounded wait (non-kicking reads
    /// answer this in milliseconds). Refuse `corpus_warming` (retry).
    Warming,
    /// The rebuild failed — inside the kicker's wait, or recorded by an
    /// earlier one. Refuse `io_error` with this cause (env). The slot is
    /// cleared — a later cold read kicks afresh.
    Failed(String),
}

/// One workspace's feed slot: live, or start-failed (sticky — recorded and
/// logged once; the workspace then keeps the pre-feed reap semantics).
/// [`Arc`] so a §6.4 cookie barrier parks on the FEED, never on the feeds
/// map lock ([`Registry::currency_refresh`] clones the handle out first).
#[derive(Debug)]
enum FeedSlot {
    Live(Arc<feed::WorkspaceFeed>),
    Failed,
}

impl FeedSlot {
    /// Start the workspace's kernel watcher; loud on failure, once.
    fn start(workspace: &Path, feed: fs::stable::FeedGen) -> FeedSlot {
        match feed::WorkspaceFeed::start(workspace, feed) {
            Ok(feed) => FeedSlot::Live(Arc::new(feed)),
            Err(e) => {
                eprintln!(
                    "feed: kernel watcher start failed for {} ({e}) — the resident memo \
                     will not be retained across idle reaps for this workspace",
                    workspace.display()
                );
                FeedSlot::Failed
            }
        }
    }
}

/// How long a write door waits for the §6.4 cookie before it falls to the
/// §6.2 extent-refresh floor. A healthy stream answers in single-digit
/// milliseconds; the wait runs its full length only when the watcher is
/// dead or badly behind — and the timeout is itself a named reason for
/// doubt (the barrier collapses the memo), so the floor that follows
/// re-derives instead of trusting. Two seconds bounds the stall a dead
/// watcher adds to a write while staying far above delivery latency.
pub(crate) const DOOR_COOKIE_TIMEOUT: Duration = Duration::from_secs(2);

/// Take-and-apply the feed's pending set into `memo` — the atomic §6.4
/// apply every cache borrow ([`Registry::domain_cache`]) and every
/// door-entry observation ([`Registry::door_observation`]) shares. The
/// caller holds the memo lock; inside it only the feed's state mutex (the
/// take) is entered. `None`: nothing was pending.
fn apply_pending(
    workspace: &Path,
    memo: &mut fs::DomainCache,
    feed: &feed::WorkspaceFeed,
) -> Option<feed::Applied> {
    let pending = feed.take();
    if pending == feed::Pending::Clean {
        return None;
    }
    let root = fs::WorkspaceRoot(workspace.to_path_buf());
    let outcome = feed::apply(&root, memo, pending);
    match &outcome {
        feed::Applied::Members(0) => {}
        feed::Applied::Members(n) => {
            feed.note_applied(*n);
            eprintln!(
                "feed: applied {n} dirty member(s) into the resident memo for {}",
                workspace.display()
            );
        }
        feed::Applied::Reset => {
            eprintln!(
                "feed: apply-time I/O failure for {} — resident memo reset, next \
                 pass re-reads the corpus",
                workspace.display()
            );
        }
        feed::Applied::Sweep(cause) => {
            eprintln!(
                "feed: rescan {} for {} — memo kept, next observation is the full \
                 stat sweep",
                cause.name(),
                workspace.display()
            );
        }
        feed::Applied::Rebaselined(cause) => {
            eprintln!(
                "feed: rescan {} for {} — memo re-baselined by swap",
                cause.name(),
                workspace.display()
            );
        }
    }
    Some(outcome)
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
            // Cold: no rings; a pre-restart cursor dies on its instance ⇒
            // `root_unknown` (§7.1, B-01).
            rings: Mutex::new(HashMap::new()),
            prewarm_signatures: Mutex::new(HashMap::new()),
            // Cold: no memo; the first currency pass reads every member once.
            domain_caches: Mutex::new(HashMap::new()),
            // Cold: feeds start with the first resident state per workspace.
            feeds: Mutex::new(HashMap::new()),
            // Cold: no open sql handles; first `sql` op opens (or cold-builds)
            // each workspace's file.
            sql_stores: Mutex::new(HashMap::new()),
            // Cold: no restore has run; each workspace's first resident memo
            // records its own receipt.
            checkpoints: Mutex::new(HashMap::new()),
            requests: AtomicU64::new(0),
            // Clock starts at birth so idle-exit can age an unused daemon.
            last_request: AtomicU64::new(now_secs()),
            // Cold: the first `mounts` call derives the table.
            mounts: crate::mounts::MountsCache::default(),
            // Cold: no rebuild in flight, no recorded failure.
            cold_builds: Mutex::new(ColdBuilds::default()),
            cold_builds_done: Condvar::new(),
            // Dead on the in-process lane; `new_shared` (the daemon) binds it.
            myself: Weak::new(),
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

    /// The daemon's constructor: [`Registry::new`] with the self-handle
    /// bound, so the §3.2 cold gate can spawn background drawer rebuilds.
    pub(crate) fn new_shared(
        state: StateStore,
        cache_root: PathBuf,
        entries: Vec<WorkspaceEntry>,
    ) -> Arc<Self> {
        Arc::new_cyclic(|weak| {
            let mut registry = Registry::new(state, cache_root, entries);
            registry.myself = weak.clone();
            registry
        })
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

    /// The §3.2 cold gate, called by every serving door BEFORE its inline
    /// [`warm_or_build`](Self::warm_or_build): proceed, refuse
    /// `corpus_warming`, or refuse with the failed rebuild's cause. A
    /// workspace with no resident engine gets its drawer rebuild KICKED
    /// HERE, on a background thread, and the kicking read absorbs it for at
    /// most [`COLD_BUILD_WAIT`] — a small drawer lands inside the wait and
    /// first contact SERVES; a drawer still rebuilding past it refuses,
    /// and every non-kicking read during the rebuild refuses in
    /// milliseconds, instead of blocking behind minutes of parse (dogfood
    /// 2026-08-16: post-install restart, every corpus read blocked 5–8 min
    /// while `roots` answered — a rebuilding drawer must never read as a
    /// hung product). Single-flight per workspace. A failed rebuild
    /// surfaces its cause to the kicking read when the failure lands inside
    /// the wait, else to the next read (`Failed`, slot cleared); a later
    /// read kicks afresh. On a registry with no self-handle (the in-process
    /// lane) the gate always answers `Serve`.
    ///
    /// # Errors
    /// Canonicalize failure only — the gate itself does no corpus I/O.
    pub fn cold_gate(&self, workspace: &Path) -> io::Result<ColdGate> {
        let canonical = workspace::canonicalize(workspace)
            .map_err(|e| io::Error::new(io::ErrorKind::NotFound, e))?;
        {
            let engines = self.engines.read().unwrap_or_else(PoisonError::into_inner);
            if engines.contains_key(&canonical) {
                return Ok(ColdGate::Serve);
            }
        }
        let mut builds = self
            .cold_builds
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if builds.in_flight.contains(&canonical) {
            // Non-kicking read: the kicker already absorbed the bounded
            // wait for this rebuild — refuse in milliseconds.
            return Ok(ColdGate::Warming);
        }
        if let Some(cause) = builds.failed.remove(&canonical) {
            return Ok(ColdGate::Failed(cause));
        }
        // No substrate to build on: the in-process lane serves inline.
        let Some(registry) = self.myself.upgrade() else {
            return Ok(ColdGate::Serve);
        };
        builds.in_flight.insert(canonical.clone());
        drop(builds);
        let key = canonical.clone();
        let spawned = std::thread::Builder::new()
            .name("drawer-rebuild".into())
            .spawn(move || {
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    registry.warm_or_build(&canonical)
                }));
                {
                    let mut builds = registry
                        .cold_builds
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner);
                    builds.in_flight.remove(&canonical);
                    match &outcome {
                        Ok(Ok(_)) => {}
                        Ok(Err(e)) => {
                            builds.failed.insert(canonical.clone(), e.to_string());
                        }
                        // A panicking rebuild must not wedge the workspace
                        // in a warming state no thread will ever end.
                        Err(_) => {
                            builds.failed.insert(
                                canonical.clone(),
                                "the background drawer rebuild panicked".to_owned(),
                            );
                        }
                    }
                }
                registry.cold_builds_done.notify_all();
                match outcome {
                    Ok(Ok(outcome)) => eprintln!(
                        "registry: drawer warm for {} ({outcome:?})",
                        canonical.display()
                    ),
                    Ok(Err(e)) => eprintln!(
                        "registry: background drawer rebuild failed for {} ({e})",
                        canonical.display()
                    ),
                    Err(_) => eprintln!(
                        "registry: background drawer rebuild panicked for {}",
                        canonical.display()
                    ),
                }
            });
        if spawned.is_err() {
            // Could not spawn the builder: clear the flag and serve inline
            // rather than wedging every read in a warming state no thread
            // will ever end.
            let mut builds = self
                .cold_builds
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            builds.in_flight.remove(&key);
            return Ok(ColdGate::Serve);
        }
        // The kicker's bounded wait: absorb a small drawer's build so first
        // contact serves; a long one refuses at the bound.
        let builds = self
            .cold_builds
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let (mut builds, timeout) = self
            .cold_builds_done
            .wait_timeout_while(builds, COLD_BUILD_WAIT, |builds| {
                builds.in_flight.contains(&key)
            })
            .unwrap_or_else(PoisonError::into_inner);
        if timeout.timed_out() {
            return Ok(ColdGate::Warming);
        }
        if let Some(cause) = builds.failed.remove(&key) {
            return Ok(ColdGate::Failed(cause));
        }
        Ok(ColdGate::Serve)
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

    /// The resident engine's [`Arc`] for `canonical` — WITHOUT parking.
    /// `None` when no engine is resident, and also when the engines lock is
    /// not immediately free (a writer holds it, or queues behind a long
    /// read). The hello door reports the resident fold through this: hello
    /// is config-grade (wire-contract §3.2) and must never queue behind
    /// corpus-scoped work — a links serve holds the engines read lock for
    /// its whole corpus computation, so a rebuild insert queued behind it
    /// walls every ordinary reader (measured on the live sessions corpus,
    /// 2026-08-16: hello went from 20 ms to a 10 s timeout the moment the
    /// resident engine landed and the links closure started). The fold is an
    /// optional field; "not readable this instant" serves as absent, exactly
    /// like cold.
    #[must_use]
    pub fn engine_snapshot_nowait(&self, canonical: &Path) -> Option<Arc<WorkspaceEngine>> {
        let engines = self.engines.try_read().ok()?;
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
    /// Every borrow first ensures the workspace's §6.4 feed exists (the feed
    /// must predate the first observation — no observation may land without
    /// gap coverage behind it) and applies its pending dirty set, so every
    /// consumer — currency pass, warm rebuild, run-plane bracket, script
    /// door — reads through a memo the feed has already patched.
    #[must_use]
    pub fn domain_cache(&self, workspace: &Path) -> Arc<Mutex<fs::DomainCache>> {
        self.patched_cache(workspace).0
    }

    /// [`Self::domain_cache`] plus the borrow's feed outcome (`None`:
    /// nothing was pending). The currency fast path
    /// ([`Self::currency_refresh`]) needs the distinction — a doubt collapse
    /// (`Applied::Reset` / `Sweep` / `Rebaselined`) inside the borrow means
    /// the memo just lost its vouched baseline, so no cookie may vouch for
    /// it this pass.
    ///
    /// The borrow also keeps the §6.3 STAMP PLANE bound to the workspace's
    /// LIVE ring epoch — when one exists. A cache borrow never mints a ring
    /// (reap reporting stays truthful: only claims mint epochs); until the
    /// workspace's first ring exists, stamp queries answer `None` and every
    /// guard stays on the content-fold compare. When the memo's bound
    /// instance is not the live ring's (first bind, or the ring died to an
    /// idle-reap and a fresh epoch was minted), the plane is re-bound with
    /// the ring's tip as the stamp clock. Old stamp values stay — max-only
    /// leftovers from a dead epoch read "touched" against young tokens,
    /// conservative and self-healing — and every cross-epoch stamp query
    /// already degrades to the content-fold compare on the instance
    /// mismatch (the §7 restart/reap row). Lock discipline: inside the memo
    /// lock only two leaf mutexes are ever entered — the feed's state (the
    /// atomic take) and the ring's state (bind check + stamp clock;
    /// fold→state, the detector's own order) — and no path acquires a memo
    /// while holding either.
    fn patched_cache(
        &self,
        workspace: &Path,
    ) -> (Arc<Mutex<fs::DomainCache>>, Option<feed::Applied>) {
        let cache = {
            let mut caches = self
                .domain_caches
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if let Some(cache) = caches.get(workspace) {
                Arc::clone(cache)
            } else {
                // Cold entry — the one door a §6.5 checkpoint can enter
                // through. A restore that discards, or finds no file, yields
                // the same empty memo this map always defaulted to, so the
                // cold path is unchanged by construction.
                let restored = self.restore_checkpoint(workspace);
                let cache = Arc::new(Mutex::new(restored.unwrap_or_default()));
                caches.insert(workspace.to_path_buf(), Arc::clone(&cache));
                cache
            }
        };
        // The feed reports into the memo's generation cell so the §6.2 fence
        // and the rescan-loss ledger are one instrument.
        let feed_cell = cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .feed_gen();
        let feed = {
            let mut feeds = self.feeds.lock().unwrap_or_else(PoisonError::into_inner);
            match feeds
                .entry(workspace.to_path_buf())
                .or_insert_with(|| FeedSlot::start(workspace, feed_cell))
            {
                FeedSlot::Live(feed) => Some(Arc::clone(feed)),
                FeedSlot::Failed => None,
            }
        };
        let ring = {
            let rings = self.rings.lock().unwrap_or_else(PoisonError::into_inner);
            rings.get(workspace).cloned()
        };
        // Take-and-apply is ATOMIC under the memo lock: a take that could
        // sit un-applied while another borrower serves would let a §6.4
        // vouched answer miss events its cookie vouches for. Every map lock
        // above is released first; inside the memo lock only the feed's
        // state mutex (take) and the ring's (clock, at stamp time) are
        // touched — both leaf mutexes no path holds while acquiring a memo.
        let mut applied = None;
        {
            let mut memo = cache.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(feed) = &feed {
                applied = apply_pending(workspace, &mut memo, feed);
            }
            if let Some(ring) = ring {
                let instance = ring.instance();
                if memo.stamp_instance() != Some(instance.as_str()) {
                    memo.bind_stamps(&instance, Arc::new(move || ring.seq()));
                }
            }
        }
        (cache, applied)
    }

    /// The workspace's current root at the cheapest lawful grain (merged
    /// plan §4.3/§4.9; merkle-spec §6.3/§6.4): `(root, vouched)`.
    ///
    /// `vouched == true` is the O(1) fast path: the §6.4 cookie returned
    /// through the ordered event stream (every foreign event before it is
    /// in the dirty set), the set applied without a doubt collapse, the
    /// §6.2 close holds the memo trusted, and the served root folds from
    /// the resident overlay — NO walk, NO stat, NO byte read. Work done is
    /// O(dirty), zero when quiet — never O(corpus).
    ///
    /// ANY miss — no live feed, cookie `Unproven`/`Refused`, a doubt
    /// collapse, an untrusted memo, no baseline yet — falls to the
    /// §6.2-governed extent-refresh floor ([`fs::DomainCache::root`]: the
    /// full stat sweep), `vouched == false`. The floor re-derives; it never
    /// silently trusts.
    ///
    /// # Errors
    /// I/O failure on the floor pass (the vouched path does no I/O).
    pub fn currency_refresh(
        &self,
        workspace: &Path,
        timeout: Duration,
    ) -> io::Result<(model::MerkleRoot, bool)> {
        let feed = {
            let feeds = self.feeds.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(FeedSlot::Live(feed)) = feeds.get(workspace) {
                Some(Arc::clone(feed))
            } else {
                None
            }
        };
        // Barrier FIRST, take-and-apply second: `Seen` proves every event
        // before the sentinel write was delivered, so the apply that
        // follows folds in everything this question must see. The wait
        // parks on the feed handle, outside every registry lock.
        let seen = feed.is_some_and(|feed| {
            feed.cookie_barrier(workspace, timeout) == feed::CookieOutcome::Seen
        });
        let (cache, applied) = self.patched_cache(workspace);
        let mut memo = cache.lock().unwrap_or_else(PoisonError::into_inner);
        let collapse = matches!(
            applied,
            Some(feed::Applied::Reset | feed::Applied::Sweep(_) | feed::Applied::Rebaselined(_))
        );
        if seen
            && !collapse
            && matches!(memo.guard_currency(), fs::stable::GuardCurrency::Trusted)
            && let Ok(root) = memo.overlay_root()
        {
            return Ok((root, true));
        }
        let root = memo.root(&fs::WorkspaceRoot(workspace.to_path_buf()))?;
        Ok((root, false))
    }

    /// The write door's §6.1 door-entry observation, made inside the door's
    /// flock on the DOOR'S OWN memo handle (card
    /// bug-trusted-overlay-unvouched): §6.4 cookie barrier first,
    /// take-and-apply second, and the overlay serves as `root_before` only
    /// on `Seen` + no doubt collapse + `Trusted` — the same vouch
    /// [`Self::currency_refresh`] demands. A drained dirty set alone is
    /// never a completeness proof. Any miss — no live feed (the sticky
    /// `Failed` slot included), cookie `Unproven`/`Refused`, a doubt
    /// collapse, an untrusted memo — falls to the §6.2 extent-refresh floor
    /// on that same memo.
    ///
    /// The observation lands in the SUPPLIED handle, never a re-borrow: a
    /// door that borrowed before an idle-reap must observe through the memo
    /// its own-write overlay will land in, or `root_before` and
    /// `root_after` would fold from two different trees.
    ///
    /// # Errors
    /// I/O failure on the floor pass (the vouched path does no I/O).
    pub fn door_observation(
        &self,
        workspace: &Path,
        cache: &Arc<Mutex<fs::DomainCache>>,
        timeout: Duration,
    ) -> io::Result<model::MerkleRoot> {
        let feed = {
            let feeds = self.feeds.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(FeedSlot::Live(feed)) = feeds.get(workspace) {
                Some(Arc::clone(feed))
            } else {
                None
            }
        };
        // Barrier FIRST, take-and-apply second (`currency_refresh`'s order):
        // `Seen` proves every event before the sentinel is delivered, so the
        // apply that follows folds in everything this door must guard
        // against. The wait parks on the feed handle, outside every registry
        // lock — the caller holds the write flock, which serializes
        // cooperating writers only; the watcher thread it waits on never
        // takes that flock.
        let seen = feed.as_ref().is_some_and(|feed| {
            feed.cookie_barrier(workspace, timeout) == feed::CookieOutcome::Seen
        });
        let mut memo = cache.lock().unwrap_or_else(PoisonError::into_inner);
        let applied = feed
            .as_ref()
            .and_then(|feed| apply_pending(workspace, &mut memo, feed));
        let collapse = matches!(
            applied,
            Some(feed::Applied::Reset | feed::Applied::Sweep(_) | feed::Applied::Rebaselined(_))
        );
        if seen
            && !collapse
            && matches!(memo.guard_currency(), fs::stable::GuardCurrency::Trusted)
            && let Ok(root) = memo.overlay_root()
        {
            return Ok(root);
        }
        memo.root(&fs::WorkspaceRoot(workspace.to_path_buf()))
    }

    /// Restore this workspace's §6.5 checkpoint, adjudicating its journal
    /// cursor against the LIVE ring — read without creating one, because no
    /// ring is no journal and no cursor can anchor against a journal that
    /// does not exist. `None` when there is no file or it was discarded (the
    /// discard is loud and labeled at the source).
    fn restore_checkpoint(&self, workspace: &Path) -> Option<fs::DomainCache> {
        let ring = {
            let rings = self.rings.lock().unwrap_or_else(PoisonError::into_inner);
            rings.get(workspace).map(Arc::clone)
        };
        let (cache, receipt) =
            crate::checkpoint::restore(&self.cache_root, workspace, ring.as_deref())?;
        eprintln!(
            "checkpoint: {} — {} leaf row(s) adopted, {} member(s) replayed from the journal",
            workspace.display(),
            receipt.leaves,
            receipt.replayed
        );
        self.checkpoints
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(workspace.to_path_buf(), receipt);
        Some(cache)
    }

    /// What this workspace's last §6.5 checkpoint restore delivered — the
    /// card's published counters. `None` when the workspace started cold.
    #[must_use]
    pub fn checkpoint_receipt(&self, workspace: &Path) -> Option<crate::CheckpointReceipt> {
        self.checkpoints
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(workspace)
            .cloned()
    }

    /// Persist every resident memo as a §6.5 checkpoint — the shutdown hook,
    /// where the process (and the in-memory tree with it) is about to die. An
    /// idle-reap needs no save: the §6.4 feed's registration lifetime already
    /// retains the memo across it.
    ///
    /// The journal cursor is captured BEFORE the memo snapshot: under-claiming
    /// only re-replays (the overlay is idempotent), over-claiming would skip a
    /// change.
    pub(crate) fn save_checkpoints(&self) {
        let caches: Vec<(PathBuf, Arc<Mutex<fs::DomainCache>>)> = {
            let caches = self
                .domain_caches
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            caches
                .iter()
                .map(|(key, cache)| (key.clone(), Arc::clone(cache)))
                .collect()
        };
        for (workspace, cache) in caches {
            let journal = {
                let rings = self.rings.lock().unwrap_or_else(PoisonError::into_inner);
                rings
                    .get(&workspace)
                    .map(|ring| (ring.instance(), ring.seq()))
            };
            // No ring is no journal: the cursor is recorded as unanchorable
            // rather than invented, so a later restore takes the evidence arm
            // instead of replaying against a numbering that never existed.
            let (instance, seq) = journal.unwrap_or_else(|| (String::new(), 0));
            let mut memo = cache.lock().unwrap_or_else(PoisonError::into_inner);
            crate::checkpoint::save(&self.cache_root, &workspace, &mut memo, instance, seq);
        }
    }

    /// The §6.4 ADDITIONAL-feed door: dirty-path hints from a secondary
    /// source (the daemon journal where it already watches, or a test rig).
    /// A hint can only ever ADD a conservative re-read; no guard or currency
    /// answer depends on one arriving — which is the whole legal standing of
    /// the journal as a feed. `false` when the workspace has no live feed
    /// (nothing resident to patch, so the hint is moot).
    pub fn note_dirty(&self, workspace: &Path, paths: &[PathBuf]) -> bool {
        let feeds = self.feeds.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(FeedSlot::Live(feed)) = feeds.get(workspace) {
            feed.note_dirty(paths.iter().map(PathBuf::as_path));
            true
        } else {
            false
        }
    }

    /// The workspace's published feed counters (`None`: no live feed).
    #[must_use]
    pub fn feed_stats(&self, workspace: &Path) -> Option<feed::FeedStats> {
        let feeds = self.feeds.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(FeedSlot::Live(feed)) = feeds.get(workspace) {
            Some(feed.stats())
        } else {
            None
        }
    }

    /// The suspicious-only trigger door (pre-merge ruling 3): mark a named
    /// rescan on a live feed. The rescan executes on the next
    /// [`domain_cache`](Self::domain_cache) borrow — nothing here schedules
    /// work. `false` when the workspace has no live feed.
    pub fn rescan(&self, workspace: &Path, cause: crate::RescanCause) -> bool {
        let feeds = self.feeds.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(FeedSlot::Live(feed)) = feeds.get(workspace) {
            feed.rescan(cause);
            true
        } else {
            false
        }
    }

    /// The rescan record for a live feed (`None`: no live feed). Every entry
    /// carries a named cause; an unnamed rescan is unconstructible.
    #[must_use]
    pub fn rescan_record(&self, workspace: &Path) -> Option<Vec<crate::RescanCause>> {
        let feeds = self.feeds.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(FeedSlot::Live(feed)) = feeds.get(workspace) {
            Some(feed.rescan_record())
        } else {
            None
        }
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

    /// Park the G11 activity clock `extra_secs` into the future.
    ///
    /// Integration fixtures call this immediately after `RunningServer::start`
    /// so a short idle-exit horizon cannot latch during handshake (pin +
    /// `sub` arm). Production never calls this.
    /// [`note_liveness`] starts the horizon at a known instant after the
    /// subscriber is armed.
    pub fn park_activity_clock(&self, extra_secs: u64) {
        self.last_request
            .store(now_secs().saturating_add(extra_secs), Ordering::Relaxed);
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
    /// The §6.4 feed's lifetime IS the registration, so it ends here — and
    /// the resident memo, whose retention across reaps rode the feed's gap
    /// coverage, leaves with it.
    ///
    /// Matches on the canonical path when the directory still resolves, else
    /// on the path as given — so a vanished workspace can still be unregistered
    /// by the canonical path a `list` reported.
    pub fn unregister(&self, path: &Path) -> bool {
        let key = workspace::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let removed = {
            let mut map = self.inner.write().unwrap_or_else(PoisonError::into_inner);
            let removed = map.remove(&key).is_some();
            if removed {
                self.persist(&map);
            }
            removed
        };
        let feed = {
            let mut feeds = self.feeds.lock().unwrap_or_else(PoisonError::into_inner);
            feeds.remove(&key)
        };
        let cache = {
            let mut caches = self
                .domain_caches
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            caches.remove(&key)
        };
        self.checkpoints
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&key);
        // The §6.5 checkpoint is derived state OF the registration: it ends
        // here too, or a re-register would adopt an index nothing has covered
        // the gap for.
        crate::checkpoint::discard(&self.cache_root, &key);
        // Kernel-stream release (the feed's Drop) runs outside every map lock.
        drop(feed);
        drop(cache);
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

    /// Idle-reap: DEMOTE workspaces with `last_use <= now - threshold_secs`
    /// — drop the warm engine, read-mint ledger, ring, and sql handle. The
    /// REGISTRATION survives, and with it the §6.4 feed and the resident
    /// memo ([`Self::domain_caches`]): the feed's dirty set covers the cold
    /// gap, which is what makes the next warm O(dirty) instead of a full
    /// corpus re-read (merkle-spec §6.4, kimi D1 — "an idle-reaped engine
    /// keeps its watcher"). A workspace with no LIVE feed has no gap
    /// coverage, so its memo still drops exactly as it always did.
    /// Injectable clock for tests. Returns the workspaces that actually shed
    /// state — an already-cold workspace is not re-reported.
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
        let mut demoted: BTreeSet<PathBuf> = BTreeSet::new();
        let candidates: Vec<PathBuf> = {
            let map = self.inner.write().unwrap_or_else(PoisonError::into_inner);
            let mut rings = self.rings.lock().unwrap_or_else(PoisonError::into_inner);
            let candidates: Vec<PathBuf> = map
                .iter()
                .filter(|(key, entry)| {
                    entry.last_use <= cutoff
                        && !rings.get(*key).is_some_and(|ring| ring.has_subscribers())
                })
                .map(|(key, _)| key.clone())
                .collect();
            for key in &candidates {
                // Ring dies inside the same critical section its exemption was
                // decided in — a later `sub` gets a fresh epoch, never an
                // orphaned ring a concurrent claim is riding.
                if rings.remove(key).is_some() {
                    demoted.insert(key.clone());
                }
            }
            candidates
        };
        if !candidates.is_empty() {
            let mut engines = self.engines.write().unwrap_or_else(PoisonError::into_inner);
            for key in &candidates {
                if engines.remove(key).is_some() {
                    demoted.insert(key.clone());
                }
            }
            drop(engines);
            // The resident memo SURVIVES the horizon under a live feed — the
            // §6.4 point: memo + dirty set make the re-warm O(dirty). With no
            // live feed there is no gap coverage, so the memo dies here as it
            // did before the feed existed.
            {
                let feeds = self.feeds.lock().unwrap_or_else(PoisonError::into_inner);
                let mut caches = self
                    .domain_caches
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                for key in &candidates {
                    if matches!(feeds.get(key), Some(FeedSlot::Live(_))) {
                        continue;
                    }
                    if caches.remove(key).is_some() {
                        demoted.insert(key.clone());
                    }
                }
            }
            // The sql handle rides the same horizon; the FILE stays (its pin
            // is content-derived, re-warm re-compares before serving).
            let mut stores = self
                .sql_stores
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            for key in &candidates {
                if stores.remove(key).is_some() {
                    demoted.insert(key.clone());
                }
            }
        }
        demoted.into_iter().collect()
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

    /// Daemon-shaped registry (self-handle bound) under `home` — the §3.2
    /// cold gate can spawn background rebuilds.
    fn registry_shared_in(home: &Path) -> Arc<Registry> {
        let cache_root = home.join("cache");
        fs::create_dir_all(&cache_root).unwrap();
        Registry::new_shared(
            StateStore::new(home.join("state.json")),
            cache_root,
            Vec::new(),
        )
    }

    /// Bounded spin until the cold gate reports `Serve` (the drawer landed).
    /// The builder runs on its own thread, so completion needs a poll; the
    /// bound keeps a wedged builder loud instead of hung.
    fn wait_serve(reg: &Registry, ws: &Path) {
        for _ in 0..400 {
            if reg.cold_gate(ws).unwrap() == ColdGate::Serve {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("drawer rebuild did not land within 2s");
    }

    /// §3.2 cold gate: the first ask kicks ONE background rebuild and
    /// answers `Warming`; every ask while it runs answers `Warming`; the
    /// landed drawer is the real engine (the inline pass reuses it).
    #[test]
    fn cold_gate_kicks_one_background_build_and_refuses_warming() {
        let home = tempfile::tempdir().unwrap();
        let reg = registry_shared_in(home.path());
        let ws = write_ws(home.path(), &[("a.md", "# A\n"), ("b.md", "# B\n")]);

        // Park the background builder between its parse and its insert.
        let (arrived_tx, arrived) = std::sync::mpsc::channel();
        let (release, release_rx) = std::sync::mpsc::channel();
        *reg.pause_before_insert
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some((arrived_tx, release_rx));

        assert_eq!(
            reg.cold_gate(&ws).unwrap(),
            ColdGate::Warming,
            "the first ask kicks the rebuild and refuses at the bounded wait"
        );
        arrived
            .recv()
            .expect("the background builder is mid-rebuild");
        // Single-flight: were a second builder racing the parked one, it
        // would sail past the consumed one-shot gate and insert the engine,
        // and these asks would answer `Serve` instead.
        assert_eq!(reg.cold_gate(&ws).unwrap(), ColdGate::Warming);
        assert_eq!(reg.cold_gate(&ws).unwrap(), ColdGate::Warming);

        release
            .send(())
            .expect("the builder parked on the release gate");
        wait_serve(&reg, &ws);
        assert_eq!(
            reg.warm_or_build(&ws).unwrap(),
            WarmOutcome::Reused,
            "the background build landed the real engine at the current fingerprint"
        );
    }

    /// §3.2: a fast-failing rebuild surfaces its cause TO THE KICKING ASK
    /// (the failure lands inside the bounded wait) — warming never masks a
    /// broken corpus — and every later ask kicks afresh and learns the same.
    #[test]
    fn cold_gate_failed_build_surfaces_cause_to_the_kicker() {
        let home = tempfile::tempdir().unwrap();
        let reg = registry_shared_in(home.path());
        let ws = write_ws(home.path(), &[("a.md", "# A\n")]);
        // Two domain configs present — the one deterministic warm refusal.
        fs::write(ws.join("mdfs_config.yaml"), "ignore: []\n").unwrap();
        fs::create_dir_all(ws.join("meridian")).unwrap();
        fs::write(ws.join("meridian/domain.md"), "# Domain\n").unwrap();

        let ColdGate::Failed(cause) = reg.cold_gate(&ws).unwrap() else {
            panic!("the kicking ask absorbs the fast failure and serves its cause");
        };
        assert!(!cause.is_empty(), "the refusal names what broke");
        let ColdGate::Failed(again) = reg.cold_gate(&ws).unwrap() else {
            panic!("a later ask kicks afresh and learns the same cause");
        };
        assert!(!again.is_empty());
    }

    /// §3.2: a small drawer lands inside the kicker's bounded wait — first
    /// contact SERVES; the refusal is for the long rebuilds only.
    #[test]
    fn small_drawer_first_contact_serves_inside_the_bounded_wait() {
        let home = tempfile::tempdir().unwrap();
        let reg = registry_shared_in(home.path());
        let ws = write_ws(home.path(), &[("a.md", "# A\n")]);
        assert_eq!(reg.cold_gate(&ws).unwrap(), ColdGate::Serve);
        assert_eq!(
            reg.warm_or_build(&ws).unwrap(),
            WarmOutcome::Reused,
            "the background build already landed the engine"
        );
    }

    /// The in-process lane (bare `Registry::new`, no self-handle): the gate
    /// answers `Serve` and the caller builds inline — today's behavior.
    #[test]
    fn in_process_cold_gate_serves_inline() {
        let home = tempfile::tempdir().unwrap();
        let reg = registry_in(home.path());
        let ws = write_ws(home.path(), &[("a.md", "# A\n")]);
        assert_eq!(
            reg.cold_gate(&ws).unwrap(),
            ColdGate::Serve,
            "no background substrate — the CLI lane blocks inline, honestly"
        );
        assert_eq!(
            reg.warm_or_build(&ws).unwrap(),
            WarmOutcome::Built { docs: 1 }
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

        // The engine drops on the idle-reap horizon (R4, amended by the
        // §6.4 registration-lifetime law: the registration itself survives).
        let reaped = reg.reap(u64::MAX, 0);
        assert!(reaped.contains(&canonical), "the workspace was demoted");
        assert!(
            !reg.engines.read().unwrap().contains_key(&canonical),
            "reap drops the warm engine"
        );
    }

    /// The §6.4 registration-lifetime law, end to end: an idle-reap demotes
    /// (engine gone) but the registration, the feed, and the resident memo
    /// survive — and the quiet re-warm reads ZERO members (O(dirty), dirty
    /// = 0). A second sweep over the already-cold workspace reports nothing.
    #[test]
    fn an_engine_reap_keeps_registration_feed_and_memo() {
        let home = tempfile::tempdir().unwrap();
        let reg = registry_in(home.path());
        let ws = write_ws(
            home.path(),
            &[("a.md", "# A\n"), ("b.md", "# B\n"), ("c.md", "# C\n")],
        );
        let canonical = workspace::canonicalize(&ws).unwrap();
        reg.register(&canonical);
        assert_eq!(
            reg.warm_or_build(&ws).unwrap(),
            WarmOutcome::Built { docs: 3 }
        );
        assert!(
            reg.feed_stats(&canonical).is_some(),
            "the feed starts with the workspace's first resident state"
        );
        let reads_before = reg.domain_cache(&canonical).lock().unwrap().leaves_read();

        let reaped = reg.reap(u64::MAX, 0);
        assert!(reaped.contains(&canonical), "the engine was demoted");
        assert!(!reg.engines.read().unwrap().contains_key(&canonical));
        assert!(
            matches!(reg.resolve(&canonical), ResolveOutcome::Adopted(_)),
            "the registration survives the reap"
        );
        assert!(
            reg.feed_stats(&canonical).is_some(),
            "an idle-reaped engine keeps its watcher (§6.4)"
        );

        // Quiet re-warm: the retained memo serves every digest — zero reads.
        reg.currency_root(&canonical).unwrap();
        let reads_after = reg.domain_cache(&canonical).lock().unwrap().leaves_read();
        assert_eq!(
            reads_after, reads_before,
            "a quiet re-warm after a reap reads zero members (O(dirty), dirty=0)"
        );

        // Nothing warm remains, so the next sweep has nothing to report.
        assert!(
            reg.reap(u64::MAX, 0).is_empty(),
            "an already-cold workspace is not re-demoted every sweep"
        );
    }

    /// THE card receipt (quality gate 1): after an engine reap, members
    /// edited while cold are re-derived at O(dirty) — the counters prove the
    /// re-warm read exactly the dirty members, never the corpus, and the
    /// re-derived root equals a from-scratch derivation of the same disk.
    #[test]
    fn re_warm_after_a_reap_reads_only_the_dirty_members() {
        let home = tempfile::tempdir().unwrap();
        let reg = registry_in(home.path());
        let ws = write_ws(
            home.path(),
            &[
                ("a.md", "# A\n"),
                ("b.md", "# B\n"),
                ("c.md", "# C\n"),
                ("d.md", "# D\n"),
                ("sub/e.md", "# E\n"),
                ("sub/f.md", "# F\n"),
            ],
        );
        let canonical = workspace::canonicalize(&ws).unwrap();
        reg.register(&canonical);
        assert_eq!(
            reg.warm_or_build(&ws).unwrap(),
            WarmOutcome::Built { docs: 6 }
        );
        let reads_warm = reg.domain_cache(&canonical).lock().unwrap().leaves_read();

        assert!(reg.reap(u64::MAX, 0).contains(&canonical));

        // Two members move while the engine is cold; the feed (here its
        // deterministic hint door — the kernel plumbing is the integration
        // gate's) accumulates them into the dirty set.
        rewrite(&canonical, "a.md", "# A moved while cold\n");
        rewrite(&canonical, "sub/e.md", "# E moved while cold\n");
        assert!(
            reg.note_dirty(
                &canonical,
                &[PathBuf::from("a.md"), PathBuf::from("sub/e.md")]
            ),
            "the live feed accepts dirty-path hints"
        );

        // Re-warm: the application re-derives the two movers, the spoiled
        // observation re-verifies exactly those two, and nothing else is
        // read. 6 members, 2 dirty ⇒ 2 observation reads, never 6.
        let root = reg.currency_root(&canonical).unwrap();
        let reads_after = reg.domain_cache(&canonical).lock().unwrap().leaves_read();
        assert_eq!(
            reads_after - reads_warm,
            2,
            "the re-warm observation read exactly the dirty members"
        );
        let stats = reg.feed_stats(&canonical).expect("live feed");
        assert_eq!(stats.applied, 2, "the published counter names the work");

        // Correctness: the O(dirty) re-warm equals a from-scratch derivation.
        let scratch_home = tempfile::tempdir().unwrap();
        let scratch = registry_in(scratch_home.path());
        assert_eq!(
            root,
            scratch.currency_root(&canonical).unwrap(),
            "the retained-memo root equals the from-scratch root"
        );
    }

    /// Quality gate 2: guard correctness consults neither journal nor
    /// watcher. A dirty-path hint left UN-APPLIED does not change what a
    /// guard observes — the guard is a live fold through the memo's own
    /// stat evidence, and it answers identically with no feed at all. The
    /// pending set is still pending afterwards: the guard consumed nothing.
    #[test]
    fn a_pending_dirty_set_never_gates_guard_currency() {
        let home = tempfile::tempdir().unwrap();
        let reg = registry_in(home.path());
        let ws = write_ws(home.path(), &[("a.md", "# A\n"), ("b.md", "# B\n")]);
        let canonical = workspace::canonicalize(&ws).unwrap();
        reg.warm_or_build(&ws).unwrap();

        // The corpus moves; the hint sits in the feed, deliberately not
        // drained (the cache is reached through the raw map, not the
        // draining accessor).
        rewrite(&canonical, "a.md", "# A moved\n");
        assert!(reg.note_dirty(&canonical, &[PathBuf::from("a.md")]));
        // `::fs` is the engine crate — this test module aliases `fs` to
        // `std::fs` for its fixtures.
        let fs_root = ::fs::WorkspaceRoot(canonical.clone());
        let cache = {
            let caches = reg.domain_caches.lock().unwrap();
            Arc::clone(caches.get(&canonical).expect("resident memo"))
        };
        let guarded_root = {
            let mut memo = cache.lock().unwrap();
            ::fs::guard::StepGuard::open_cached(&fs_root, &mut memo)
                .expect("guard opens")
                .pre_root()
        };

        // The same observation with NO feed anywhere near it.
        let mut bare = ::fs::DomainCache::new();
        let bare_root = ::fs::guard::StepGuard::open_cached(&fs_root, &mut bare)
            .expect("guard opens")
            .pre_root();
        assert_eq!(
            guarded_root, bare_root,
            "the guard saw the edit through its own live fold — with or \
             without a feed in the process"
        );

        let stats = reg.feed_stats(&canonical).expect("live feed");
        assert_eq!(
            (stats.pending, stats.applied),
            (1, 0),
            "the guard consumed nothing from the feed: the hint is still pending"
        );
    }

    /// THE card receipt (quality gate 3): a vouched world-guard refresh is
    /// O(1) — cookie returned, dirty set applied, root served from the
    /// resident overlay with zero listings, zero member reads, zero folds —
    /// measured against the extent-refresh sweep floor on the same corpus.
    /// A foreign write costs exactly the dirty members (O(dirty)), never
    /// the corpus, and the vouched root equals a fresh-cache oracle.
    /// Live kernel stream; generous timeout for CI inotify.
    #[test]
    fn a_vouched_currency_refresh_is_o1_against_the_sweep_floor() {
        let home = tempfile::tempdir().unwrap();
        let reg = registry_in(home.path());
        let ws = write_ws(
            home.path(),
            &[
                ("a.md", "# A\n"),
                ("b.md", "# B\n"),
                ("c.md", "# C\n"),
                ("d.md", "# D\n"),
                ("sub/e.md", "# E\n"),
                ("sub/f.md", "# F\n"),
            ],
        );
        let canonical = workspace::canonicalize(&ws).unwrap();
        reg.register(&canonical);
        let fs_root = ::fs::WorkspaceRoot(canonical.clone());

        // The sweep baseline this gate measures against: a fresh memo's
        // extent refresh walks every listing and reads every member.
        let (sweep_listings, sweep_reads) = {
            let mut oracle = ::fs::DomainCache::new();
            oracle.root(&fs_root).unwrap();
            (oracle.listings(), oracle.leaves_read())
        };
        assert_eq!(sweep_reads, 6, "the floor reads the whole corpus");
        assert!(sweep_listings > 0, "the floor walks the listings");

        // First refresh: cold memo, feed not yet live — the floor answers.
        let (root_cold, vouched_cold) = reg
            .currency_refresh(&canonical, Duration::from_secs(10))
            .unwrap();
        assert!(!vouched_cold, "a cold first refresh never vouches");

        let counters = |reg: &Registry| {
            let cache = reg.domain_cache(&canonical);
            let memo = cache.lock().unwrap();
            (memo.listings(), memo.leaves_read(), memo.flat_folds())
        };

        // Quiet corpus, vouched refresh: O(1) — every instrument frozen.
        let (l0, r0, f0) = counters(&reg);
        let (root_quiet, vouched_quiet) = reg
            .currency_refresh(&canonical, Duration::from_secs(10))
            .unwrap();
        assert!(vouched_quiet, "live feed + returned cookie vouches");
        assert_eq!(root_quiet, root_cold, "the vouched root is the served root");
        let (l1, r1, f1) = counters(&reg);
        assert_eq!(
            (l1 - l0, r1 - r0, f1 - f0),
            (0, 0, 0),
            "vouched quiet refresh: 0 listings, 0 member reads, 0 folds — \
             the sweep floor on this corpus costs {sweep_listings} listings \
             and {sweep_reads} member reads"
        );

        // One foreign write: the vouched refresh pays O(dirty) — the one
        // mover rides the feed's apply; the memo still walks and reads
        // nothing on its own account.
        rewrite(&canonical, "a.md", "# A moved\n");
        let applied_before = reg.feed_stats(&canonical).expect("live feed").applied;
        let (root_dirty, vouched_dirty) = reg
            .currency_refresh(&canonical, Duration::from_secs(10))
            .unwrap();
        assert!(
            vouched_dirty,
            "the ordered stream vouches for the mover too"
        );
        let (l2, r2, f2) = counters(&reg);
        assert_eq!(
            (l2 - l1, r2 - r1),
            (0, 0),
            "no walk, no observation read — the apply's one byte-read is \
             the whole I/O cost"
        );
        assert_eq!(f2 - f1, 1, "one refold serves the moved root");
        let applied_after = reg.feed_stats(&canonical).expect("live feed").applied;
        assert_eq!(
            applied_after - applied_before,
            1,
            "O(dirty): exactly the one mover was applied"
        );
        let oracle_root = ::fs::DomainCache::new().root(&fs_root).unwrap();
        assert_eq!(
            root_dirty, oracle_root,
            "the vouched root equals a fresh derivation of the same disk"
        );
        assert_ne!(root_dirty, root_quiet, "the mover moved the root");
    }

    /// A cold workspace's first refresh is the floor: no resident state, no
    /// live feed, no proof — `vouched == false`, and the answer is the full
    /// re-derivation, never a blind trust.
    #[test]
    fn the_cold_first_refresh_is_the_floor() {
        let home = tempfile::tempdir().unwrap();
        let reg = registry_in(home.path());
        let ws = write_ws(home.path(), &[("a.md", "# A\n"), ("b.md", "# B\n")]);
        let canonical = workspace::canonicalize(&ws).unwrap();
        reg.register(&canonical);
        let (root, vouched) = reg
            .currency_refresh(&canonical, Duration::from_secs(10))
            .unwrap();
        assert!(
            !vouched,
            "nothing can vouch for a memo that does not exist yet"
        );
        let fs_root = ::fs::WorkspaceRoot(canonical.clone());
        assert_eq!(
            root,
            ::fs::DomainCache::new().root(&fs_root).unwrap(),
            "the floor answer is the full re-derivation"
        );
    }

    /// Card step 3 / quality gate (bug-cookie-newdir-false-seen): a member
    /// created inside a brand-new directory in the admitted arming window
    /// cannot produce a vouched stale overlay. `currency_refresh` must return
    /// `vouched=false` and a root equal to a fresh-cache oracle.
    ///
    /// Quiet already-watched tree still `Seen` (the vouched baseline above
    /// the mkdir) — the cookie law is unchanged for a tree the watch already
    /// covers. Linux-only: this is the inotify arming gap (19/20 miss on
    /// the unfixed tree); Darwin `FSEvents` did not exhibit it (0/20).
    #[cfg(target_os = "linux")]
    #[test]
    fn a_new_directory_child_before_arm_cannot_vouch_a_stale_overlay() {
        let home = tempfile::tempdir().unwrap();
        let reg = registry_in(home.path());
        let ws = write_ws(home.path(), &[("a.md", "# A\n"), ("b.md", "# B\n")]);
        let canonical = workspace::canonicalize(&ws).unwrap();
        reg.register(&canonical);

        // Cold floor, then a quiet vouched baseline — Trusted overlay.
        let (root_cold, vouched_cold) = reg
            .currency_refresh(&canonical, Duration::from_secs(10))
            .unwrap();
        assert!(!vouched_cold);
        let (root_quiet, vouched_quiet) = reg
            .currency_refresh(&canonical, Duration::from_secs(10))
            .unwrap();
        assert!(vouched_quiet, "quiet already-watched tree still Sees");
        assert_eq!(root_quiet, root_cold);

        // The constructed failure: create new/ and immediately new/x.md
        // before the sub-watch arms (notify 8.2.0 inotify `add_watch_by_event`
        // collects, then the loop arms after the batch).
        fs::create_dir(canonical.join("new")).unwrap();
        fs::write(canonical.join("new/x.md"), "# X\n").unwrap();

        // The parent create is delivered; the child may not be. Wait for
        // the named doubt the parent must raise, then refresh.
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_secs(10) {
            if reg.feed_stats(&canonical).is_some_and(|s| s.all_dirty) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            reg.feed_stats(&canonical).is_some_and(|s| s.all_dirty),
            "the directory create must mark doubt: {:?}",
            reg.feed_stats(&canonical)
        );

        let (root, vouched) = reg
            .currency_refresh(&canonical, Duration::from_secs(10))
            .unwrap();
        eprintln!(
            "fixture vouched={vouched} root==quiet={}",
            root == root_quiet
        );
        assert!(
            !vouched,
            "a new-directory arming gap is named doubt, never a vouched old root"
        );
        let oracle = ::fs::DomainCache::new()
            .root(&::fs::WorkspaceRoot(canonical.clone()))
            .unwrap();
        assert_eq!(root, oracle, "the unvouched floor includes the new member");
        assert_ne!(root, root_quiet, "the new member moved the root");
    }

    /// The §6.3 instance binding across a reap (kimi D3): the borrow binds
    /// the stamp plane to the live ring epoch; the reap kills the ring; the
    /// next live epoch re-binds the plane under its OWN instance — so every
    /// stamp token minted under the dead epoch degrades on the instance
    /// mismatch instead of false-passing against a reset seq.
    #[test]
    fn a_reap_minted_ring_epoch_rebinds_the_stamp_plane() {
        let home = tempfile::tempdir().unwrap();
        let reg = registry_in(home.path());
        let ws = write_ws(home.path(), &[("a.md", "# A\n")]);
        let canonical = workspace::canonicalize(&ws).unwrap();
        reg.register(&canonical);

        // No ring yet: the borrow binds nothing — stamps cannot answer.
        assert!(
            reg.domain_cache(&canonical)
                .lock()
                .unwrap()
                .stamp_instance()
                .is_none(),
            "a cache borrow never mints a ring epoch"
        );

        // A claim mints epoch A; the next borrow binds the plane to it.
        let _ = reg.ring(&canonical);
        let epoch_a = {
            let cache = reg.domain_cache(&canonical);
            let memo = cache.lock().unwrap();
            memo.stamp_instance()
                .expect("bound to the live epoch")
                .to_owned()
        };

        // The reap kills the ring; the memo survives under its live feed.
        assert!(reg.reap(u64::MAX, 0).contains(&canonical));

        // A fresh epoch, a fresh binding — never the dead epoch's name.
        let _ = reg.ring(&canonical);
        let epoch_b = {
            let cache = reg.domain_cache(&canonical);
            let memo = cache.lock().unwrap();
            memo.stamp_instance()
                .expect("re-bound to the young epoch")
                .to_owned()
        };
        assert_ne!(
            epoch_a, epoch_b,
            "a dead epoch's stamp tokens degrade on the instance mismatch"
        );
    }

    /// The feed's lifetime IS the registration: `unregister` ends both the
    /// feed and the resident memo. A hint for an unknown workspace is
    /// refused rather than buffered.
    #[test]
    fn unregister_ends_the_feed_and_the_resident_memo() {
        let home = tempfile::tempdir().unwrap();
        let reg = registry_in(home.path());
        let ws = write_ws(home.path(), &[("a.md", "# A\n")]);
        let canonical = workspace::canonicalize(&ws).unwrap();
        reg.register(&canonical);
        reg.warm_or_build(&ws).unwrap();
        assert!(reg.feed_stats(&canonical).is_some());

        assert!(reg.unregister(&canonical));
        assert!(
            reg.feed_stats(&canonical).is_none(),
            "the feed ends with the registration"
        );
        assert!(
            !reg.domain_caches.lock().unwrap().contains_key(&canonical),
            "the resident memo leaves with its gap coverage"
        );
        assert!(
            !reg.note_dirty(&canonical, &[PathBuf::from("a.md")]),
            "a hint for an unregistered workspace is refused"
        );
    }

    /// The suspicious-only door piggybacks on the next borrow: a named
    /// overflow marks all-dirty, the next `domain_cache` climb is the sweep
    /// rung (memo kept), and the record names the cause. No timer fires.
    #[test]
    fn a_named_rescan_piggybacks_on_the_next_borrow() {
        let home = tempfile::tempdir().unwrap();
        let reg = registry_in(home.path());
        let ws = write_ws(home.path(), &[("a.md", "# A\n"), ("b.md", "# B\n")]);
        let canonical = workspace::canonicalize(&ws).unwrap();
        reg.warm_or_build(&ws).unwrap();
        let (reads_before, sweeps_before) = {
            let cache = reg.domain_cache(&canonical);
            let g = cache.lock().unwrap();
            (g.leaves_read(), g.sweeps())
        };

        assert!(
            reg.rescan(&canonical, crate::RescanCause::Overflow),
            "a live feed accepts a named rescan"
        );
        let stats = reg.feed_stats(&canonical).expect("live feed");
        assert!(stats.all_dirty, "the mark is sticky until the next borrow");
        assert_eq!(stats.rescans, 1);
        assert_eq!(stats.overflows, 1);
        assert_eq!(
            reg.rescan_record(&canonical).as_deref(),
            Some(&[crate::RescanCause::Overflow][..]),
            "the record names the cause"
        );

        // The borrow climbs the sweep rung; the observation is the stat
        // sweep — unmoved corpus, zero extra reads.
        let _ = reg.currency_root(&canonical).unwrap();
        let stats = reg.feed_stats(&canonical).expect("live feed");
        assert!(!stats.all_dirty, "the take drained the mark");
        let (reads_after, sweeps_after) = {
            let cache = reg.domain_cache(&canonical);
            let g = cache.lock().unwrap();
            (g.leaves_read(), g.sweeps())
        };
        assert_eq!(
            reads_after, reads_before,
            "the sweep kept the memo and re-read nothing"
        );
        assert!(
            sweeps_after > sweeps_before,
            "the piggybacked observation was the sweep"
        );
    }

    /// An instance-change mark climbs the re-baseline rung on the next
    /// borrow: the swapped memo equals a from-scratch derivation of a change
    /// the (injected) stream never delivered.
    #[test]
    fn an_instance_change_rebaselines_on_the_next_borrow() {
        let home = tempfile::tempdir().unwrap();
        let reg = registry_in(home.path());
        let ws = write_ws(home.path(), &[("a.md", "# A\n"), ("b.md", "# B\n")]);
        let canonical = workspace::canonicalize(&ws).unwrap();
        reg.warm_or_build(&ws).unwrap();

        rewrite(&canonical, "a.md", "# A moved unseen\n");
        assert!(reg.rescan(&canonical, crate::RescanCause::InstanceChange));
        let root = reg.currency_root(&canonical).unwrap();
        assert_eq!(
            reg.rescan_record(&canonical).as_deref(),
            Some(&[crate::RescanCause::InstanceChange][..])
        );

        let scratch_home = tempfile::tempdir().unwrap();
        let scratch = registry_in(scratch_home.path());
        assert_eq!(
            root,
            scratch.currency_root(&canonical).unwrap(),
            "the swapped memo equals a from-scratch derivation"
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
        std::thread::sleep(Duration::from_millis(10));
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

        std::thread::sleep(Duration::from_millis(10));
        fs::rename(ws.join("sub/c.md"), ws.join("sub/c2.md")).unwrap();
        assert_eq!(
            reg.warm_or_build(&ws).unwrap(),
            WarmOutcome::Built { docs: 1 },
            "rename parses the one member under its new name"
        );
        assert_matches_scratch(&reg, &ws, "rename");

        std::thread::sleep(Duration::from_millis(10));
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

    fn plan_page(word: &str) -> String {
        format!("# Alpha\n\n## Beta\n\nship by {word}\n")
    }

    fn match_edit(old: &str, new: &str) -> wire::Edit {
        wire::Edit {
            target: wire::SecRef::Hpath {
                hpath: vec![
                    wire::HpathSeg {
                        h: "Alpha".into(),
                        n: None,
                    },
                    wire::HpathSeg {
                        h: "Beta".into(),
                        n: None,
                    },
                ],
            },
            edit: wire::EditShape::Match {
                old: old.into(),
                new: new.into(),
            },
            if_node_rev: None,
        }
    }

    fn splice_args(path: &str, old: &str, new: &str) -> wire_serve::write::SpliceArgs {
        wire_serve::write::SpliceArgs {
            premises: Vec::new(),
            id: None,
            origin: wire_serve::guard::Origin::InProcess,
            path: wire::Path(path.into()),
            actor: Some("alice".into()),
            now: None,
            receipt: None,
            if_root: None,
            dry: false,
            force: false,
            edits: vec![match_edit(old, new)],
            plan_edits: Vec::new(),
            pin: None,
        }
    }

    /// Quality gate: a daemon splice and a daemon currency pass lock the
    /// same `DomainCache` address — the write overlays that memo, not `WRITE_CACHES`.
    #[test]
    fn a_daemon_splice_and_currency_pass_lock_the_same_cache() {
        let home = tempfile::tempdir().unwrap();
        let reg = registry_in(home.path());
        let plan = plan_page("August");
        let other = plan_page("still");
        let ws = write_ws(
            home.path(),
            &[("notes/plan.md", &plan), ("notes/other.md", &other)],
        );
        let canonical = workspace::canonicalize(&ws).unwrap();
        reg.warm_or_build(&ws).unwrap();
        let _ = reg.currency_root(&canonical).unwrap();

        let cache = reg.domain_cache(&canonical);
        let currency = reg.domain_cache(&canonical);
        assert!(
            Arc::ptr_eq(&cache, &currency),
            "currency and splice resolve one DomainCache"
        );

        let fs_root = ::fs::WorkspaceRoot(canonical.clone());
        let observe = || reg.door_observation(&canonical, &cache, Duration::from_secs(10));
        let out = wire_serve::write::splice(
            &fs_root,
            None,
            &splice_args("notes/plan.md", "August", "w1"),
            &[],
            Some(wire_serve::write::ResidentDoor {
                cache: &cache,
                observe: &observe,
            }),
        )
        .expect("daemon-cache splice");
        let frame = out.committed.expect("real splice commits");

        let again = reg.domain_cache(&canonical);
        assert!(
            Arc::ptr_eq(&cache, &again),
            "the resident memo is still the same Arc after the splice"
        );
        let overlaid = {
            let mut memo = cache.lock().unwrap();
            memo.overlay_root().expect("overlay after splice")
        };
        assert_eq!(
            overlaid.0, frame.delta.root_after.0,
            "the currency memo carries the commit's own overlay"
        );
    }

    /// Quality gate: injected watcher overflow makes the next guarded write
    /// see `Untrusted` and full-reobserve (absorb) on that same cache.
    #[test]
    fn overflow_makes_the_next_guarded_write_reobserve_on_the_same_cache() {
        let home = tempfile::tempdir().unwrap();
        let reg = registry_in(home.path());
        let plan = plan_page("August");
        let other = plan_page("still");
        let ws = write_ws(
            home.path(),
            &[("notes/plan.md", &plan), ("notes/other.md", &other)],
        );
        let canonical = workspace::canonicalize(&ws).unwrap();
        reg.warm_or_build(&ws).unwrap();
        let _ = reg.currency_root(&canonical).unwrap();

        assert!(reg.rescan(&canonical, crate::RescanCause::Overflow));
        let cache = reg.domain_cache(&canonical);
        assert!(
            matches!(
                cache.lock().unwrap().guard_currency(),
                ::fs::stable::GuardCurrency::Untrusted { .. }
            ),
            "the sweep-rung apply leaves the loss unabsorbed"
        );
        let sweeps_before = cache.lock().unwrap().sweeps();

        let fs_root = ::fs::WorkspaceRoot(canonical.clone());
        let observe = || reg.door_observation(&canonical, &cache, Duration::from_secs(10));
        wire_serve::write::splice(
            &fs_root,
            None,
            &splice_args("notes/plan.md", "August", "w1"),
            &[],
            Some(wire_serve::write::ResidentDoor {
                cache: &cache,
                observe: &observe,
            }),
        )
        .expect("degrade splice");

        let (sweeps_after, currency) = {
            let memo = cache.lock().unwrap();
            (memo.sweeps(), memo.guard_currency())
        };
        assert!(
            sweeps_after > sweeps_before,
            "Untrusted degrades to a full observe on the daemon cache"
        );
        assert_eq!(currency, ::fs::stable::GuardCurrency::Trusted);
        assert!(
            Arc::ptr_eq(&cache, &reg.domain_cache(&canonical)),
            "the absorb landed on the same cache currency still holds"
        );
    }

    /// Quality gate: a warm splice after a feed-patched dirty set does not
    /// stat-sweep untouched members.
    #[test]
    fn a_warm_splice_after_a_feed_patch_does_not_stat_sweep() {
        let home = tempfile::tempdir().unwrap();
        let reg = registry_in(home.path());
        let plan = plan_page("August");
        let other = plan_page("still");
        let ws = write_ws(
            home.path(),
            &[
                ("notes/plan.md", &plan),
                ("notes/other.md", &other),
                ("notes/a.md", "# A\n"),
                ("notes/b.md", "# B\n"),
                ("notes/c.md", "# C\n"),
                ("notes/d.md", "# D\n"),
            ],
        );
        let canonical = workspace::canonicalize(&ws).unwrap();
        reg.warm_or_build(&ws).unwrap();
        let _ = reg.currency_root(&canonical).unwrap();

        rewrite(&canonical, "notes/a.md", "# A moved\n");
        assert!(reg.note_dirty(&canonical, &[PathBuf::from("notes/a.md")]));
        let cache = reg.domain_cache(&canonical);
        let (sweeps_before, stats_before) = {
            let memo = cache.lock().unwrap();
            assert_eq!(memo.guard_currency(), ::fs::stable::GuardCurrency::Trusted);
            (memo.sweeps(), memo.member_stats())
        };

        let fs_root = ::fs::WorkspaceRoot(canonical.clone());
        let observe = || reg.door_observation(&canonical, &cache, Duration::from_secs(10));
        wire_serve::write::splice(
            &fs_root,
            None,
            &splice_args("notes/plan.md", "August", "w1"),
            &[],
            Some(wire_serve::write::ResidentDoor {
                cache: &cache,
                observe: &observe,
            }),
        )
        .expect("warm splice");

        let (sweeps_after, stats_after) = {
            let memo = cache.lock().unwrap();
            (memo.sweeps(), memo.member_stats())
        };
        assert_eq!(
            sweeps_after, sweeps_before,
            "Trusted overlay after a feed patch does not start a door-entry sweep"
        );
        assert_eq!(
            stats_after, stats_before,
            "untouched members are not re-stat'd"
        );
    }

    /// The card's race fixture (bug-trusted-overlay-unvouched): foreign B
    /// lands on disk AFTER the borrow's take and BEFORE the door's
    /// observation. The door's own §6.4 barrier is written after B, so
    /// `Seen` proves B's event was delivered and the vouched apply folds it
    /// in — `root_before` moves off R0 and `if_root=R0` refuses.
    /// Drain-and-hope served stale R0 here and accepted a write against a
    /// world that was already B1.
    #[test]
    fn a_foreign_write_between_take_and_door_refuses_the_stale_world_guard() {
        let home = tempfile::tempdir().unwrap();
        let reg = registry_in(home.path());
        let plan = plan_page("August");
        let other = plan_page("still");
        let ws = write_ws(
            home.path(),
            &[("notes/plan.md", &plan), ("notes/other.md", &other)],
        );
        let canonical = workspace::canonicalize(&ws).unwrap();
        reg.warm_or_build(&ws).unwrap();

        // R0: the baseline the stale guard will name.
        let (r0, _) = reg
            .currency_refresh(&canonical, Duration::from_secs(10))
            .unwrap();

        // The borrow's take runs first; then B lands, its event in flight.
        let cache = reg.domain_cache(&canonical);
        rewrite(&canonical, "notes/other.md", "# Notes\n\nmoved by B\n");

        let fs_root = ::fs::WorkspaceRoot(canonical.clone());
        let mut args = splice_args("notes/plan.md", "August", "w1");
        args.if_root = Some(wire::Root(r0.0.clone()));
        let observe = || reg.door_observation(&canonical, &cache, Duration::from_secs(10));
        let err = wire_serve::write::splice(
            &fs_root,
            None,
            &args,
            &[],
            Some(wire_serve::write::ResidentDoor {
                cache: &cache,
                observe: &observe,
            }),
        )
        .expect_err("a stale world guard must refuse once B is folded in");
        assert_eq!(
            err.code,
            wire::ErrorCode::RootMismatch,
            "the refusal is the world guard's: {err:?}"
        );
        assert_eq!(
            err.expected.as_ref().map(|r| r.0.as_str()),
            Some(r0.0.as_str()),
            "the guard refused the caller's R0, not some other premise: {err:?}"
        );
        assert_ne!(
            err.actual.as_ref().map(|r| r.0.as_str()),
            Some(r0.0.as_str()),
            "the door's observation absorbed B — root_before moved off R0: {err:?}"
        );
    }

    /// The same race through the §4.4 SET door — the card's gate names
    /// `splice` AND `splice.set`, and both ride one `observed_root` seam.
    /// Driven rather than argued from the shared seam: a claim is bounded
    /// by the instrument that produced it.
    #[test]
    fn the_set_door_refuses_a_stale_world_guard_after_a_foreign_write() {
        let home = tempfile::tempdir().unwrap();
        let reg = registry_in(home.path());
        let plan = plan_page("August");
        let second = plan_page("July");
        let other = plan_page("still");
        let ws = write_ws(
            home.path(),
            &[
                ("notes/plan.md", &plan),
                ("notes/second.md", &second),
                ("notes/other.md", &other),
            ],
        );
        let canonical = workspace::canonicalize(&ws).unwrap();
        reg.warm_or_build(&ws).unwrap();

        let (r0, _) = reg
            .currency_refresh(&canonical, Duration::from_secs(10))
            .unwrap();

        let cache = reg.domain_cache(&canonical);
        rewrite(&canonical, "notes/other.md", "# Notes\n\nmoved by B\n");

        let fs_root = ::fs::WorkspaceRoot(canonical.clone());
        let args = wire_serve::write::SpliceSetArgs {
            premises: Vec::new(),
            id: None,
            files: vec![
                wire::SpliceFile {
                    path: wire::Path("notes/plan.md".into()),
                    edits: vec![match_edit("August", "w1")],
                    plan_edits: Vec::new(),
                },
                wire::SpliceFile {
                    path: wire::Path("notes/second.md".into()),
                    edits: vec![match_edit("July", "w2")],
                    plan_edits: Vec::new(),
                },
            ],
            origin: wire_serve::guard::Origin::InProcess,
            actor: Some("alice".into()),
            now: None,
            receipt: None,
            if_root: Some(wire::Root(r0.0.clone())),
            dry: false,
            force: false,
        };
        let observe = || reg.door_observation(&canonical, &cache, Duration::from_secs(10));
        let err = wire_serve::write::splice_set_with_cache(
            &fs_root,
            None,
            &args,
            &[],
            Some(wire_serve::write::ResidentDoor {
                cache: &cache,
                observe: &observe,
            }),
        )
        .expect_err("the set door's world guard must refuse the stale R0");
        assert_eq!(
            err.code,
            wire::ErrorCode::RootMismatch,
            "the set refusal is the world guard's: {err:?}"
        );
        assert_ne!(
            err.actual.as_ref().map(|r| r.0.as_str()),
            Some(r0.0.as_str()),
            "the set door's observation absorbed B too: {err:?}"
        );
        // Nothing committed: both members still carry their pre-image.
        let plan_now = fs::read_to_string(canonical.join("notes/plan.md")).unwrap();
        assert!(
            plan_now.contains("August"),
            "a refused set leaves every member byte-unchanged"
        );
    }

    /// The sticky-`Failed` arm: no live feed means no vouch, so the door
    /// observation floors to the live fold and a foreign edit is seen even
    /// though no event will ever report it. Before the fix, `Trusted` alone
    /// served the stale overlay — a dead feed was indistinguishable from a
    /// quiet corpus.
    #[test]
    fn a_failed_feed_slot_floors_the_door_observation() {
        let home = tempfile::tempdir().unwrap();
        let reg = registry_in(home.path());
        let plan = plan_page("August");
        let other = plan_page("still");
        let ws = write_ws(
            home.path(),
            &[("notes/plan.md", &plan), ("notes/other.md", &other)],
        );
        let canonical = workspace::canonicalize(&ws).unwrap();
        reg.warm_or_build(&ws).unwrap();
        let _ = reg.currency_root(&canonical).unwrap();

        let cache = reg.domain_cache(&canonical);
        let stale = cache.lock().unwrap().overlay_root().unwrap();
        assert_eq!(
            cache.lock().unwrap().guard_currency(),
            ::fs::stable::GuardCurrency::Trusted,
            "the memo itself still claims trust — that is the hole"
        );
        // The watcher dies sticky; the memo never hears about it.
        reg.feeds
            .lock()
            .unwrap()
            .insert(canonical.clone(), FeedSlot::Failed);
        rewrite(
            &canonical,
            "notes/other.md",
            "# Notes\n\nsilent foreign edit\n",
        );

        let observed = reg
            .door_observation(&canonical, &cache, Duration::from_secs(10))
            .unwrap();
        let truth = ::fs::DomainCache::new()
            .root(&::fs::WorkspaceRoot(canonical.clone()))
            .unwrap();
        assert_eq!(
            observed.0, truth.0,
            "no vouch: the floor re-derives the disk truth"
        );
        assert_ne!(
            observed.0, stale.0,
            "the stale Trusted overlay was not served"
        );
    }

    /// The unproven-cookie arm: a barrier that cannot prove delivery within
    /// its budget never authorizes the overlay. Either outcome of the
    /// zero-budget race is lawful — `Unproven` collapses to the floor, a
    /// lucky `Seen` applies everything before the sentinel — and both must
    /// serve the fresh disk truth, never the pre-edit overlay.
    #[test]
    fn an_unproven_cookie_floors_the_door_observation() {
        let home = tempfile::tempdir().unwrap();
        let reg = registry_in(home.path());
        let plan = plan_page("August");
        let other = plan_page("still");
        let ws = write_ws(
            home.path(),
            &[("notes/plan.md", &plan), ("notes/other.md", &other)],
        );
        let canonical = workspace::canonicalize(&ws).unwrap();
        reg.warm_or_build(&ws).unwrap();
        let _ = reg.currency_root(&canonical).unwrap();

        let cache = reg.domain_cache(&canonical);
        let stale = cache.lock().unwrap().overlay_root().unwrap();
        rewrite(
            &canonical,
            "notes/other.md",
            "# Notes\n\nlate-notify edit\n",
        );

        let observed = reg
            .door_observation(&canonical, &cache, Duration::ZERO)
            .unwrap();
        let truth = ::fs::DomainCache::new()
            .root(&::fs::WorkspaceRoot(canonical.clone()))
            .unwrap();
        assert_eq!(
            observed.0, truth.0,
            "no proof within budget: the door serves the disk truth"
        );
        assert_ne!(
            observed.0, stale.0,
            "the stale Trusted overlay was not served"
        );
    }
}
