//! The §6.4 event feed and its rescan ladder — the engine owns its senses
//! (merkle-spec, adjudicated engine-side, five lanes unanimous; merged plan
//! §4.3).
//!
//! One kernel file watcher per workspace (notify: `FSEvents` on macOS, inotify
//! on Linux), owned by this daemon — the engine process. Events accumulate
//! into a per-workspace DIRTY SET held by the registry, and the set applies
//! into the workspace's resident memo ([`fs::DomainCache`]) whenever the
//! cache is next borrowed ([`crate::Registry::domain_cache`]).
//!
//! # The registration-lifetime law (kimi D1, adopted)
//! The feed's lifetime is the workspace REGISTRATION, not engine warmth: it
//! starts when the workspace first grows resident state, survives every
//! engine idle-reap, and ends at `unregister` (or daemon exit). While the
//! engine is cold the set keeps accumulating, so the next warm applies
//! exactly the members that moved — O(dirty), never O(corpus). That is what
//! lets the reap RETAIN the resident memo: the feed covers the cold gap the
//! old design paid a full corpus re-read to close.
//!
//! # Hints only — never an instrument (§6.4 guard-plane law)
//! A dirty path is a HINT: applying one can only ADD a conservative re-read
//! (the applied entry lands with a spoiled [`fs::StatKey`], so the next
//! observation re-verifies it against disk). Nothing here can suppress a
//! read, and no guard or currency answer ever waits on an event arriving —
//! guards stay live folds through the memo's own stat evidence. The daemon
//! journal stays a legal ADDITIONAL feed through the same door
//! ([`WorkspaceFeed::note_dirty`]); its three vacuous windows are irrelevant
//! by construction because nothing depends on it.
//!
//! # What the set holds
//! Workspace-relative paths that could ever carry a member digest: `*.md`
//! with no dot-prefixed segment — both §12.1 STRUCTURAL floors, deliberately
//! config-independent so a domain-config change while cold can never have
//! filtered away the one event that mattered. Everything else (`.git`
//! churn, build artifacts, the `.meridian/cookie` sentinel) can never move
//! the root and never enters the set. Order is not kept: a spoil-set is
//! order-insensitive by construction. The one ORDERED consumer — the §6.4
//! currency cookie — rides the raw stream UPSTREAM of this filter
//! ([`WorkspaceFeed::cookie_barrier`]): a sighting of the sentinel proves
//! every event before it was delivered, which is what makes the dirty set
//! complete as of the write.
//!
//! # The rescan ladder — every cause NAMED, throttled (merkle-spec §6.4)
//! Doubt — a kernel overflow, a watcher error, a new directory (or any
//! non-member path that can hide members before its watch arms), the set
//! outgrowing [`DIRTY_CAP`], or an explicit suspicious-only trigger through
//! [`WorkspaceFeed::rescan`] — collapses the set to ALL-DIRTY under a named
//! [`RescanCause`], recorded and reported as event loss
//! ([`fs::stable::FeedGen::note_loss`], so guard currency is UNTRUSTED until
//! a full observation re-baselines). The next borrow then climbs exactly one
//! rung:
//!
//! - **Sweep** (overflow, missed event, vouch failure, cookie timeout): the
//!   resident memo is KEPT — the next observation is already the full stat
//!   sweep (lane B, 160 ms measured class: every member statted live, bytes
//!   read only for movers), and it absorbs the loss. The watcher never
//!   restarts across a rescan.
//! - **Re-baseline** (watcher instance change): the gap reaches back to an
//!   unknown point, so the memo is re-derived from disk into a FRESH memo
//!   that commits by swap (1.45 s cold / 160 ms warm measured class) — never
//!   a torn or empty index; a failed rebuild keeps the old memo and the
//!   unabsorbed loss.
//!
//! There is NO TIMER anywhere on this ladder: rescans execute by piggybacking
//! on the next borrow (pre-merge ruling 3 — suspicious-only; the periodic
//! idle sweep is DECLINED, its price on the record). Zero background work
//! when healthy. Self-echo (the engine's own writes re-arriving) is deduped
//! as a cost saving only — overlay idempotence stays the correctness;
//! masking is never load-bearing.

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use notify::event::{AccessKind, AccessMode, CreateKind};
use notify::{EventKind, RecursiveMode, Watcher as _};

/// The dirty set's size floor for the all-dirty collapse. Sized well above
/// any plausible cold-gap edit burst (the delta plane bounds one frame at
/// 128 rows) and well below corpus scale, so a runaway producer degrades to
/// the correct full re-read instead of holding an unbounded set.
const DIRTY_CAP: usize = 4096;

/// The §6.4 currency-cookie sentinel, workspace-relative. Dot-prefixed BY
/// LAW (the standing `wire-contract.md` §12.1 floor): outside the hash
/// domain, so writing it can never move the root or break a held token —
/// which is what makes the barrier free to run on every guard-grade
/// question.
pub(crate) const COOKIE_REL: &str = ".meridian/cookie";

/// The rescan record's bound: enough for any post-mortem window, dropped
/// oldest-first past it (the lifetime total stays in [`FeedStats::rescans`]).
const RESCAN_RECORD_CAP: usize = 64;

/// The §6.7 cookie holdoff: after a barrier times out, further barriers
/// answer `Unproven` immediately for this long — callers take the
/// extent-refresh floor (the pre-feed cost) without stalling — and one probe
/// per window re-tests the stream. Bounds the stall a dead-but-running
/// watcher can add to one timeout per window per workspace; a `Seen` clears
/// it, so a transient stall self-heals on the first probe that lands.
const COOKIE_HOLDOFF: Duration = Duration::from_secs(60);

/// Why a rescan was marked — every rescan carries its cause into the log and
/// the record; an anonymous rescan is unconstructible (merkle-spec §6.4,
/// pre-merge ruling 3's suspicious-only trigger set).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RescanCause {
    /// Event loss until proven otherwise: a watcher error, or a new
    /// directory (or other non-member path) that can hide members before
    /// its watch arms.
    MissedEvent,
    /// Kernel event-queue overflow (the rescan flag), or the registry-held
    /// set outgrowing [`DIRTY_CAP`].
    Overflow,
    /// The watcher instance changed — the stream's continuity broke, so the
    /// gap reaches back to an unknown point.
    InstanceChange,
    /// A spot check failed: a vouched answer disagreed with disk (the §6.3
    /// stamps plane's trigger).
    VouchFailure,
    /// A currency cookie did not return through the event stream in time
    /// (the §6.4 barrier's trigger).
    CookieTimeout,
    /// A FOREIGN edit of the domain config arrived as a dirty path
    /// (merkle-spec §6.7): the config governs membership and version, so it
    /// cannot be folded as an ordinary leaf — the overlay would serve the
    /// new config's leaf under the SUPERSEDED membership. Apply-born only;
    /// the intake never mints it.
    DomainConfig,
}

/// Which rung of the ladder a cause climbs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Rung {
    /// Memo kept; the next observation is the full stat sweep.
    Sweep,
    /// Memo re-derived from disk, committed by swap.
    Rebaseline,
}

impl RescanCause {
    /// The cause's name — the word the log line and the record carry.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::MissedEvent => "missed-event",
            Self::Overflow => "overflow",
            Self::InstanceChange => "instance-change",
            Self::VouchFailure => "vouch-failure",
            Self::CookieTimeout => "cookie-timeout",
            Self::DomainConfig => "domain-config",
        }
    }

    /// Only a broken stream continuity re-baselines; every other doubt is
    /// covered by the stat sweep the next observation already is.
    const fn rung(self) -> Rung {
        match self {
            Self::InstanceChange => Rung::Rebaseline,
            Self::MissedEvent
            | Self::Overflow
            | Self::VouchFailure
            | Self::CookieTimeout
            | Self::DomainConfig => Rung::Sweep,
        }
    }
}

/// One workspace's event feed: the kernel watcher and the dirty set it
/// accumulates. Dropping it releases the kernel watch — which is why the
/// registry drops it only at `unregister` (or a labeled instance
/// replacement), never at reap.
pub(crate) struct WorkspaceFeed {
    /// Keeps the kernel stream alive; the handler thread owns `sync`'s
    /// other [`Arc`]. Held, never spoken to after construction.
    _watcher: notify::RecommendedWatcher,
    sync: Arc<FeedSync>,
    /// Serials this feed's cookie writes have minted ([`Self::cookie_barrier`]).
    /// The next write carries `fetch_add + 1`; sightings max into
    /// [`FeedState::cookie_seen`].
    cookie_serial: AtomicU64,
}

/// The feed's shared state plus the §6.4 cookie condvar: barrier waiters
/// park on `cookie`; the watcher handler notifies on every sighting of the
/// sentinel.
struct FeedSync {
    state: Mutex<FeedState>,
    cookie: Condvar,
}

impl std::fmt::Debug for WorkspaceFeed {
    /// Summary form: the kernel handle is platform state with no stable
    /// `Debug` contract across notify backends; the counters are the whole
    /// public truth.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkspaceFeed")
            .field("stats", &self.stats())
            .finish_non_exhaustive()
    }
}

/// The registry-held dirty set plus its instrument counters.
///
/// No timer, no deadline, no interval: a doubt is sticky until the next
/// borrow takes it. The idle sweep is DECLINED (pre-merge ruling 3).
#[derive(Debug, Default)]
struct FeedState {
    /// Member-candidate rel paths seen dirty since the last take.
    dirty: BTreeSet<PathBuf>,
    /// Doubt collapse under its named cause. Sticky until taken; a second
    /// cause landing on an open doubt keeps the higher rung.
    doubt: Option<RescanCause>,
    /// The shared feed-generation cell: advanced per accepted event, loss
    /// noted per collapse — the watcher half of the §6.2 fence.
    feed: fs::stable::FeedGen,
    /// The rescan record: every mark's cause, oldest dropped past
    /// [`RESCAN_RECORD_CAP`].
    rescans: Vec<RescanCause>,
    /// Rescans marked over the feed's life (monotonic; survives trimming).
    rescans_total: u64,
    /// Kernel events accepted into the set (post-filter), over the feed's life.
    events: u64,
    /// Doubt collapses over the feed's life (rescan flags, errors, cap,
    /// explicit triggers). Equal to [`Self::rescans_total`] by construction —
    /// asserted apart in the chaos gate so an anonymous doubt path cannot
    /// appear unnoticed.
    overflows: u64,
    /// Dirty members applied into the resident memo, over the feed's life.
    applied: u64,
    /// Highest cookie serial sighted through the ordered stream (§6.4).
    /// Serials only grow, so a later sighting vouches for every earlier
    /// barrier too.
    cookie_seen: u64,
    /// When a cookie barrier last timed out (merkle-spec §6.7 holdoff).
    /// While within [`Self::cookie_holdoff`] of it, further barriers answer
    /// `Unproven` immediately — callers take the floor without re-paying the
    /// timeout — and one probe per window re-tests the stream. Cleared by
    /// the next `Seen`.
    cookie_timeout_at: Option<Instant>,
    /// The holdoff width. [`COOKIE_HOLDOFF`] in production ([`WorkspaceFeed::start`]);
    /// `Default` is zero (no holdoff) so unit fixtures stay deterministic
    /// unless they opt in.
    cookie_holdoff: Duration,
}

/// Published feed counters (the card's "counter published" receipt, probe
/// surface class): what arrived, what collapsed, what applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeedStats {
    /// Kernel events accepted into the dirty set (post-filter).
    pub events: u64,
    /// Doubt collapses (kernel rescan, watcher error, cap breach, explicit
    /// rescan triggers).
    pub overflows: u64,
    /// Rescans marked, each under a named cause (the record's total).
    pub rescans: u64,
    /// Dirty members applied into the resident memo.
    pub applied: u64,
    /// Paths currently pending application.
    pub pending: usize,
    /// Whether the pending state is the all-dirty collapse.
    pub all_dirty: bool,
}

/// What a take found pending.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Pending {
    /// Nothing pending — the hot no-op.
    Clean,
    /// These members were seen dirty since the last take.
    Paths(Vec<PathBuf>),
    /// Doubt collapse under its named cause: everything is suspect.
    All(RescanCause),
}

/// One apply's outcome.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Applied {
    /// This many dirty members were re-derived against disk.
    Members(u64),
    /// The resident memo was reset (an apply-time I/O failure that is not
    /// absence): the next pass rebuilds from a full read — the pre-feed
    /// baseline.
    Reset,
    /// Rescan, sweep rung: the memo is kept; the next observation is the
    /// full stat sweep and absorbs the noted loss.
    Sweep(RescanCause),
    /// Rescan, re-baseline rung: the memo was re-derived from disk and
    /// committed by swap.
    Rebaselined(RescanCause),
}

/// A [`WorkspaceFeed::cookie_barrier`] verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CookieOutcome {
    /// The sentinel came back through the ordered stream: every event before
    /// the write is delivered, so the dirty set is complete as of it — the
    /// O(1) currency proof (watchman's shape).
    Seen,
    /// Timeout or I/O failure: no proof either way. The caller falls to the
    /// extent-refresh floor — and a cookie timeout is a NAMED reason for
    /// doubt (§6.4 suspicious-only ladder), never silent trust.
    Unproven,
    /// The rel would enter the hash domain — refused by construction (merged
    /// plan §7 row): a member-candidate cookie would move the very root it
    /// is supposed to vouch for.
    Refused,
}

impl WorkspaceFeed {
    /// Start the kernel watcher for `workspace` (a canonical root), reporting
    /// into the shared feed-generation cell `feed` — the same cell the
    /// workspace's resident memo fences reads with. The feed must exist
    /// BEFORE the first observation lands in the workspace's resident memo —
    /// an observation without gap coverage behind it would let a later
    /// re-warm trust the memo blind.
    ///
    /// # Errors
    /// The kernel watch could not be created or attached. The caller records
    /// the failure loudly; the workspace then keeps the pre-feed semantics
    /// (its resident memo drops on every reap).
    pub(crate) fn start(
        workspace: &Path,
        feed: fs::stable::FeedGen,
    ) -> notify::Result<WorkspaceFeed> {
        let sync = Arc::new(FeedSync {
            state: Mutex::new(FeedState {
                feed,
                cookie_holdoff: COOKIE_HOLDOFF,
                ..FeedState::default()
            }),
            cookie: Condvar::new(),
        });
        let sink = Arc::clone(&sync);
        let root = workspace.to_path_buf();
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                let mut s = sink.state.lock().unwrap_or_else(PoisonError::into_inner);
                if let Ok(event) = event {
                    if event.need_rescan() {
                        s.collapse(RescanCause::Overflow);
                        sink.cookie.notify_all();
                        return;
                    }
                    if !relevant(event.kind) {
                        return;
                    }
                    for path in &event.paths {
                        let Ok(rel) = path.strip_prefix(&root) else {
                            continue;
                        };
                        // §6.4 cookie sighting — UPSTREAM of the member
                        // filter: the sentinel is the ordered stream's
                        // proof-of-delivery, never dirt. A torn or
                        // vanished read proves nothing and skips (the
                        // close event re-delivers; at worst a barrier
                        // times out to its floor — never a false Seen).
                        if rel == Path::new(COOKIE_REL) {
                            if let Some(serial) = read_serial(path) {
                                s.cookie_seen = s.cookie_seen.max(serial);
                                sink.cookie.notify_all();
                            }
                            continue;
                        }
                        s.admit(rel, path, event.kind);
                    }
                    if s.doubt.is_some() {
                        sink.cookie.notify_all();
                    }
                } else {
                    // A watcher error is event loss until proven otherwise.
                    s.collapse(RescanCause::MissedEvent);
                    sink.cookie.notify_all();
                }
            })?;
        // The sentinel's directory must exist BEFORE the recursive watch is
        // armed, so the initial walk covers it. notify's inotify backend
        // arms a NEW directory only after delivering its create event —
        // anything written into it before that arm is invisible forever
        // (notify-8.2.0 inotify.rs, `add_watch_by_event` collects, the loop
        // arms after the batch). A `.meridian` first created by a barrier
        // could lose that race once and leave every later cookie unseen.
        // Best-effort: on failure (read-only root) barriers answer
        // `Unproven` and the caller keeps the extent-refresh floor.
        if let Some(parent) = workspace.join(COOKIE_REL).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        watcher.watch(workspace, RecursiveMode::Recursive)?;
        Ok(WorkspaceFeed {
            _watcher: watcher,
            sync,
            cookie_serial: AtomicU64::new(0),
        })
    }

    /// The §6.4 currency barrier: write the sentinel at [`COOKIE_REL`] and
    /// wait (bounded) to see it return through the ordered event stream.
    /// `Seen` proves every captured event before the write is already in
    /// the dirty set, and that no capture-gap doubt is open — the O(1)
    /// currency proof. The caller takes and applies the set AFTER a
    /// `Seen`, never before, so the applied memo is complete as of the
    /// question.
    ///
    /// Precisely: `Seen` proves ORDERED DELIVERY of everything the kernel
    /// stream captured, AND that no capture-gap doubt is open. Capture has
    /// one known gap — files landing in a brand-new directory before its
    /// watch arms (see the arming note in [`Self::start`]). The watcher
    /// collapses that path as [`RescanCause::MissedEvent`]. `Seen` is
    /// illegal while that doubt is open: the barrier returns
    /// [`CookieOutcome::Unproven`] immediately and does not spend
    /// `timeout` (the write-path cookie budget is unchanged). The
    /// sentinel's own directory is still pre-created so the cookie itself
    /// never sits in the gap.
    pub(crate) fn cookie_barrier(&self, workspace: &Path, timeout: Duration) -> CookieOutcome {
        self.cookie_barrier_at(workspace, Path::new(COOKIE_REL), timeout)
    }

    /// The barrier at an explicit rel — the refusal seam. A rel the member
    /// filter would admit is REFUSED by construction (merged plan §7 row):
    /// a cookie inside the hash domain would move the very root it vouches
    /// for. [`COOKIE_REL`] can never trip this (dot-prefixed by the §12.1
    /// floor); the seam exists so the law is a tested fact, not a comment.
    pub(crate) fn cookie_barrier_at(
        &self,
        workspace: &Path,
        rel: &Path,
        timeout: Duration,
    ) -> CookieOutcome {
        if member_candidate(rel) {
            return CookieOutcome::Refused;
        }
        {
            let s = self
                .sync
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if s.doubt.is_some() {
                return CookieOutcome::Unproven;
            }
            // §6.7 holdoff: a recent timeout already taught the stream is
            // stalled — answer immediately so the caller floors without
            // re-paying the timeout; one probe per window re-tests.
            if s.cookie_timeout_at
                .is_some_and(|at| at.elapsed() < s.cookie_holdoff)
            {
                return CookieOutcome::Unproven;
            }
        }
        let serial = self.cookie_serial.fetch_add(1, Ordering::Relaxed) + 1;
        let abs = workspace.join(rel);
        let write = abs
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&abs, serial.to_string()));
        if write.is_err() {
            return CookieOutcome::Unproven;
        }
        let deadline = Instant::now() + timeout;
        let mut s = self
            .sync
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        loop {
            if s.doubt.is_some() {
                return CookieOutcome::Unproven;
            }
            if s.cookie_seen >= serial {
                // The stream is live — a standing holdoff was transient.
                s.cookie_timeout_at = None;
                return CookieOutcome::Seen;
            }
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                s.collapse(RescanCause::CookieTimeout);
                s.cookie_timeout_at = Some(Instant::now());
                return CookieOutcome::Unproven;
            };
            s = self
                .sync
                .cookie
                .wait_timeout(s, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// The ADDITIONAL-feed door (§6.4): dirty-path hints from any secondary
    /// source — the daemon journal where it already watches, or a test.
    /// Hints ride the same structural filter and the same conservative apply
    /// as kernel events; nothing anywhere depends on one arriving.
    pub(crate) fn note_dirty<'a>(&self, paths: impl IntoIterator<Item = &'a Path>) {
        let mut s = self
            .sync
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for path in paths {
            s.insert(path);
        }
    }

    /// The suspicious-only trigger door (pre-merge ruling 3): mark a rescan
    /// under its named cause. The rescan executes by piggybacking on the
    /// next borrow — nothing here schedules work.
    pub(crate) fn rescan(&self, cause: RescanCause) {
        let mut s = self
            .sync
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        s.collapse(cause);
    }

    /// Take whatever is pending, leaving the set clean.
    pub(crate) fn take(&self) -> Pending {
        let mut s = self
            .sync
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        s.take()
    }

    /// Record an APPLY-BORN rescan (today only [`RescanCause::DomainConfig`])
    /// in the counters and the cause record. The loss itself was already
    /// noted on the generation cell inside the apply, under the memo lock —
    /// this is the probe surface's bookkeeping, not the trust signal. No
    /// doubt is opened: the borrower already holds the `Sweep` outcome, and
    /// the noted loss keeps every vouch refused until a full observation
    /// absorbs it.
    pub(crate) fn note_rescan(&self, cause: RescanCause) {
        let mut s = self
            .sync
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        s.overflows += 1;
        s.rescans_total += 1;
        if s.rescans.len() == RESCAN_RECORD_CAP {
            s.rescans.remove(0);
        }
        s.rescans.push(cause);
    }

    /// Test seam: rewrite the §6.7 cookie holdoff width.
    #[cfg(test)]
    fn set_cookie_holdoff(&self, width: Duration) {
        self.sync
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .cookie_holdoff = width;
    }

    /// Record members applied into the resident memo (the published counter).
    pub(crate) fn note_applied(&self, members: u64) {
        let mut s = self
            .sync
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        s.applied += members;
    }

    /// The rescan record: every marked rescan's named cause, oldest first
    /// (bounded at [`RESCAN_RECORD_CAP`]; the lifetime total is
    /// [`FeedStats::rescans`]).
    pub(crate) fn rescan_record(&self) -> Vec<RescanCause> {
        let s = self
            .sync
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        s.rescans.clone()
    }

    /// The published counters.
    pub(crate) fn stats(&self) -> FeedStats {
        let s = self
            .sync
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        FeedStats {
            events: s.events,
            overflows: s.overflows,
            rescans: s.rescans_total,
            applied: s.applied,
            pending: s.dirty.len(),
            all_dirty: s.doubt.is_some(),
        }
    }
}

impl FeedState {
    /// Take whatever is pending, leaving the set clean.
    fn take(&mut self) -> Pending {
        if let Some(cause) = self.doubt.take() {
            self.dirty.clear();
            return Pending::All(cause);
        }
        if self.dirty.is_empty() {
            return Pending::Clean;
        }
        Pending::Paths(std::mem::take(&mut self.dirty).into_iter().collect())
    }

    /// Admit one kernel path: a member candidate dirties the set; a
    /// hideable non-member (a new directory, a renamed-in directory)
    /// collapses under [`RescanCause::MissedEvent`] so a later cookie
    /// cannot `Seen` a capture gap.
    fn admit(&mut self, rel: &Path, abs: &Path, kind: EventKind) {
        if member_candidate(rel) {
            self.insert(rel);
            return;
        }
        if can_hide_members(rel, abs, kind) {
            self.collapse(RescanCause::MissedEvent);
        }
    }

    /// Admit one rel path through the structural filter; collapse at the cap.
    /// Every accepted event advances the shared generation cell — the §6.2
    /// fence's watcher half (an event landing inside a read bracket spoils
    /// that read's record).
    fn insert(&mut self, rel: &Path) {
        if !member_candidate(rel) {
            return;
        }
        self.events += 1;
        self.feed.advance();
        if self.doubt.is_some() {
            return;
        }
        self.dirty.insert(rel.to_path_buf());
        if self.dirty.len() > DIRTY_CAP {
            self.collapse(RescanCause::Overflow);
        }
    }

    /// Doubt under its named cause: drop the enumeration, mark everything
    /// suspect, record the cause, and report the loss LOUDLY (guard currency
    /// is untrusted until a full observation re-baselines — merkle-spec
    /// §6.2 row 6). A cause landing on an open doubt keeps the higher rung.
    fn collapse(&mut self, cause: RescanCause) {
        self.doubt = Some(match self.doubt {
            Some(open) if open.rung() >= cause.rung() => open,
            _ => cause,
        });
        self.dirty.clear();
        self.overflows += 1;
        self.rescans_total += 1;
        if self.rescans.len() == RESCAN_RECORD_CAP {
            self.rescans.remove(0);
        }
        self.rescans.push(cause);
        self.feed.note_loss(cause.name());
    }
}

/// Could `rel` ever carry a member digest? The two §12.1 STRUCTURAL floors —
/// md-only and no dot-prefixed segment — plus plain-relative shape (all
/// `Normal` components, so a hint can never name a path outside the root).
/// Deliberately config-free: the custom ignore list can change while the
/// engine is cold, so filtering by it here could drop the one event that
/// mattered; a custom-ignored member's spoil is merely a no-op re-read.
fn member_candidate(rel: &Path) -> bool {
    let mut any_segment = false;
    for component in rel.components() {
        let Component::Normal(seg) = component else {
            return false;
        };
        let Some(seg) = seg.to_str() else {
            return false;
        };
        if fs::domain::dot_segment(seg) {
            return false;
        }
        any_segment = true;
    }
    any_segment
        && rel
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("md"))
}

/// A non-member path whose children would be member candidates — a new
/// directory, or a rename that lands one. Dot-prefixed trees cannot hide
/// members (the structural filter already excludes them).
fn hideable_prefix(rel: &Path) -> bool {
    let mut any_segment = false;
    for component in rel.components() {
        let Component::Normal(seg) = component else {
            return false;
        };
        let Some(seg) = seg.to_str() else {
            return false;
        };
        if fs::domain::dot_segment(seg) {
            return false;
        }
        any_segment = true;
    }
    any_segment
}

fn can_hide_members(rel: &Path, abs: &Path, kind: EventKind) -> bool {
    hideable_prefix(rel) && (matches!(kind, EventKind::Create(CreateKind::Folder)) || abs.is_dir())
}

/// Parse a sighted cookie's serial. `None` (torn or vanished mid-read) is
/// benign: the write's close event re-delivers the path, and an unseen
/// serial at worst times a barrier out to its floor — never a false `Seen`.
fn read_serial(abs: &Path) -> Option<u64> {
    let text = std::fs::read_to_string(abs).ok()?;
    text.trim().parse().ok()
}

/// Which event kinds can carry a content change. `Access` is read-side noise
/// — the engine's own observation reads would feed themselves back through
/// the watcher as fresh dirt — EXCEPT close-after-write, which on inotify is
/// the one signal some write patterns leave.
fn relevant(kind: EventKind) -> bool {
    match kind {
        EventKind::Access(AccessKind::Close(AccessMode::Write)) => true,
        EventKind::Access(_) => false,
        _ => true,
    }
}

/// Apply one take into the workspace's resident memo. Conservative by
/// construction: a present member is re-read NOW and lands through the
/// own-write overlay with a spoiled identity, so the next observation
/// re-verifies it; a vanished member leaves the fold through the overlay's
/// removal half; an unchanged digest applies nothing (self-echo dedup — a
/// cost saving only, overlay idempotence is the correctness). The tree is
/// never torn: no present member is ever transiently removed, so a
/// concurrent `overlay_root` fold stays truthful at every instant.
///
/// All-dirty climbs the rescan ladder by cause (module doc): the sweep rung
/// keeps the memo — the next observation is already the full stat sweep and
/// absorbs the loss the collapse noted; the re-baseline rung re-derives a
/// fresh memo from disk and commits it by swap, keeping the old memo (and
/// the unabsorbed loss) when the rebuild fails. An apply-time I/O failure
/// other than absence resets the memo instead: the next pass re-reads the
/// corpus, the pre-feed baseline. A memo with no observation baseline yet
/// applies nothing (the cold first pass reads everything anyway).
pub(crate) fn apply(
    root: &fs::WorkspaceRoot,
    cache: &mut fs::DomainCache,
    pending: Pending,
) -> Applied {
    let paths = match pending {
        Pending::Clean => return Applied::Members(0),
        Pending::All(cause) => match cause.rung() {
            Rung::Sweep => return Applied::Sweep(cause),
            Rung::Rebaseline => {
                // The fresh memo adopts the SAME generation cell, so the
                // fence and the loss ledger survive the swap.
                let mut fresh = fs::DomainCache::with_feed(cache.feed_gen());
                return match fresh.root(root) {
                    Ok(_) => {
                        *cache = fresh;
                        Applied::Rebaselined(cause)
                    }
                    Err(e) => {
                        eprintln!(
                            "feed: re-baseline for {} failed ({e}) — old memo kept, guard \
                             currency stays untrusted until a full observation lands",
                            root.0.display()
                        );
                        Applied::Sweep(cause)
                    }
                };
            }
        },
        Pending::Paths(paths) => paths,
    };
    // §6.7: the domain config governs membership and version, so a FOREIGN
    // edit of it cannot be folded as an ordinary leaf — the overlay would
    // serve the new config's leaf under the superseded membership, a root no
    // true corpus state ever had. The governed write's own echo (digest
    // already imposed through `overlay_membership`) passes; anything else
    // escalates to the Sweep rung, loss noted HERE so the very next borrower
    // reads untrusted until a full observation absorbs it. The dropped dirty
    // paths are covered by that same observation.
    let config_rel = Path::new(fs::domain::DOMAIN_CONFIG_PATH);
    if paths.iter().any(|rel| rel == config_rel) {
        let echo = std::fs::read(root.0.join(config_rel)).is_ok_and(|bytes| {
            cache.fold_at(config_rel)
                == Ok(fs::resident::ScopeFold::Value(model::leaf_digest(&bytes)))
        });
        if !echo {
            cache.feed_gen().note_loss(RescanCause::DomainConfig.name());
            return Applied::Sweep(RescanCause::DomainConfig);
        }
    }
    let mut applied = 0u64;
    for rel in paths {
        match std::fs::read(root.0.join(&rel)) {
            Ok(bytes) => {
                let digest = model::leaf_digest(&bytes);
                if cache.fold_at(&rel) == Ok(fs::resident::ScopeFold::Value(digest)) {
                    continue; // unchanged — the engine's own write echoing back
                }
                match cache.overlay_leaf(&rel, digest) {
                    Ok(_) => applied += 1,
                    // No baseline: the memo is cold and the first observation
                    // reads everything — the rest of the take is moot.
                    Err(_) => return Applied::Members(applied),
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                match cache.overlay_remove(&rel) {
                    Ok(_) => applied += 1,
                    Err(_) => return Applied::Members(applied),
                }
            }
            // Unreadable ≠ absent: no digest can be derived and absence would
            // be a lie — reset to the loud, correct baseline, keeping the
            // shared generation cell so the fence survives.
            Err(_) => {
                *cache = fs::DomainCache::with_feed(cache.feed_gen());
                return Applied::Reset;
            }
        }
    }
    Applied::Members(applied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    /// The §7 gate row, refused by construction: a cookie rel the member
    /// filter would admit can never be a barrier — it would move the very
    /// root it vouches for. The lawful sentinel sits outside the hash domain
    /// twice over: the feed's structural filter refuses it, and so does the
    /// fs domain law itself.
    #[test]
    fn a_cookie_inside_the_hash_domain_is_refused_by_construction() {
        let dir = tempfile::tempdir().unwrap();
        // Canonical root, as the registry supplies (macOS tempdirs live
        // behind a /var → /private/var symlink the kernel stream resolves).
        let root = dir.path().canonicalize().unwrap();
        let feed = WorkspaceFeed::start(&root, fs::stable::FeedGen::default()).expect("watcher");
        assert_eq!(
            feed.cookie_barrier_at(&root, Path::new("notes/plan.md"), Duration::from_millis(50)),
            CookieOutcome::Refused,
            "a member-candidate rel is refused before any byte lands"
        );
        assert!(
            !root.join("notes/plan.md").exists(),
            "the refusal wrote nothing"
        );
        assert!(!member_candidate(Path::new(COOKIE_REL)));
        assert!(
            !fs::domain::Domain::new().contains(Path::new(COOKIE_REL)),
            "the sentinel is outside the hash domain by the §12.1 floor"
        );
    }

    /// The barrier's proof arm, live kernel stream: the sentinel write comes
    /// back through the ordered stream as `Seen`, and the sighting never
    /// enters the dirty set (the cookie is proof-of-delivery, not dirt).
    /// Generous timeout: CI inotify delivery can lag.
    #[test]
    fn the_cookie_returns_through_the_ordered_stream() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let feed = WorkspaceFeed::start(&root, fs::stable::FeedGen::default()).expect("watcher");
        assert_eq!(
            feed.cookie_barrier(&root, Duration::from_secs(10)),
            CookieOutcome::Seen,
            "the sentinel returned through the kernel stream"
        );
        let stats = feed.stats();
        assert_eq!(
            (stats.pending, stats.events, stats.all_dirty),
            (0, 0, false),
            "the sighting fed the barrier, never the dirty set"
        );
    }

    /// The barrier's doubt arm: a sentinel the watched stream never carries
    /// (written under a DIFFERENT root) proves nothing — `Unproven` at the
    /// deadline, never a false `Seen`, never a hang.
    #[test]
    fn an_unwatched_cookie_write_times_out_unproven() {
        let watched = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let feed = WorkspaceFeed::start(
            &watched.path().canonicalize().unwrap(),
            fs::stable::FeedGen::default(),
        )
        .expect("watcher");
        assert_eq!(
            feed.cookie_barrier(
                &elsewhere.path().canonicalize().unwrap(),
                Duration::from_millis(50)
            ),
            CookieOutcome::Unproven,
            "no sighting by the deadline is doubt, not proof"
        );
        assert_eq!(
            feed.rescan_record(),
            vec![RescanCause::CookieTimeout],
            "a timed-out barrier is the named CookieTimeout trigger"
        );
    }

    /// The structural filter: md-only, no dot segment, plain-relative shape.
    #[test]
    fn the_dirty_set_admits_only_member_candidates() {
        assert!(member_candidate(Path::new("notes/plan.md")));
        assert!(member_candidate(Path::new("UPPER.MD")));
        assert!(!member_candidate(Path::new("notes/data.json")));
        assert!(!member_candidate(Path::new(".git/index.md")));
        assert!(!member_candidate(Path::new("a/.hidden/x.md")));
        assert!(!member_candidate(Path::new(".meridian/cookie")));
        assert!(!member_candidate(Path::new("/abs/olute.md")));
        assert!(!member_candidate(Path::new("../escape.md")));
        assert!(!member_candidate(Path::new("")));
    }

    /// A hideable directory is named doubt, not a silent drop. A non-md
    /// file and a dot-prefixed directory cannot hide members.
    #[test]
    fn a_hideable_directory_collapses_as_missed_event() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir(root.join("new")).unwrap();
        std::fs::write(root.join("data.json"), "{}\n").unwrap();
        std::fs::create_dir(root.join(".git")).unwrap();

        let mut s = FeedState::default();
        s.admit(
            Path::new("new"),
            &root.join("new"),
            EventKind::Create(CreateKind::Folder),
        );
        assert_eq!(s.doubt, Some(RescanCause::MissedEvent));
        assert_eq!(s.rescans.as_slice(), &[RescanCause::MissedEvent][..]);
        assert!(s.dirty.is_empty());

        let mut file = FeedState::default();
        file.admit(
            Path::new("data.json"),
            &root.join("data.json"),
            EventKind::Create(CreateKind::File),
        );
        assert_eq!(file.doubt, None);
        assert!(file.dirty.is_empty());

        let mut dot = FeedState::default();
        dot.admit(
            Path::new(".git"),
            &root.join(".git"),
            EventKind::Create(CreateKind::Folder),
        );
        assert_eq!(dot.doubt, None, "dot-prefixed trees cannot hide members");
    }

    /// `Seen` is illegal while a capture-gap doubt is open: the barrier
    /// returns `Unproven` without minting `CookieTimeout` (the cookie is
    /// not what failed).
    #[test]
    fn seen_is_illegal_while_new_directory_doubt_is_open() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let feed = WorkspaceFeed::start(&root, fs::stable::FeedGen::default()).expect("watcher");
        assert_eq!(
            feed.cookie_barrier(&root, Duration::from_secs(5)),
            CookieOutcome::Seen,
            "quiet already-watched tree still Sees"
        );
        feed.rescan(RescanCause::MissedEvent);
        assert_eq!(
            feed.cookie_barrier(&root, Duration::from_secs(5)),
            CookieOutcome::Unproven,
            "open doubt cannot produce Seen"
        );
        assert_eq!(
            feed.rescan_record(),
            vec![RescanCause::MissedEvent],
            "an already-open doubt does not spend the cookie timeout"
        );
    }

    /// Read-side kinds never feed the set — the engine's own observation
    /// reads must not dirty what they read — except close-after-write.
    #[test]
    fn access_noise_is_filtered_except_close_write() {
        use notify::event::{AccessKind, AccessMode};
        assert!(!relevant(EventKind::Access(AccessKind::Read)));
        assert!(!relevant(EventKind::Access(AccessKind::Open(
            AccessMode::Any
        ))));
        assert!(!relevant(EventKind::Access(AccessKind::Close(
            AccessMode::Read
        ))));
        assert!(relevant(EventKind::Access(AccessKind::Close(
            AccessMode::Write
        ))));
        assert!(relevant(EventKind::Modify(notify::event::ModifyKind::Any)));
        assert!(relevant(EventKind::Create(CreateKind::Any)));
        assert!(relevant(EventKind::Remove(notify::event::RemoveKind::Any)));
        assert!(relevant(EventKind::Any));
    }

    /// Every constructible cause is named; there is no anonymous variant.
    #[test]
    fn every_rescan_cause_is_named() {
        let all = [
            RescanCause::MissedEvent,
            RescanCause::Overflow,
            RescanCause::InstanceChange,
            RescanCause::VouchFailure,
            RescanCause::CookieTimeout,
            RescanCause::DomainConfig,
        ];
        let mut names = BTreeSet::new();
        for cause in all {
            assert!(!cause.name().is_empty(), "{cause:?} must carry a name");
            assert!(
                names.insert(cause.name()),
                "cause names are unique: {}",
                cause.name()
            );
        }
        assert_eq!(all.len(), 6, "the suspicious-only set is exactly these six");
        assert_eq!(RescanCause::InstanceChange.rung(), Rung::Rebaseline);
        assert_eq!(RescanCause::Overflow.rung(), Rung::Sweep);
        assert_eq!(RescanCause::MissedEvent.rung(), Rung::Sweep);
        assert_eq!(RescanCause::VouchFailure.rung(), Rung::Sweep);
        assert_eq!(RescanCause::CookieTimeout.rung(), Rung::Sweep);
        assert_eq!(RescanCause::DomainConfig.rung(), Rung::Sweep);
    }

    /// Cap breach collapses to all-dirty under the named overflow cause; the
    /// take drains it and the next take is clean. Every accepted event
    /// advanced the shared generation; the collapse noted exactly one loss.
    #[test]
    fn the_cap_collapses_to_all_dirty_and_take_drains() {
        let mut s = FeedState::default();
        for i in 0..=DIRTY_CAP {
            s.insert(Path::new(&format!("m{i}.md")));
        }
        assert_eq!(
            s.doubt,
            Some(RescanCause::Overflow),
            "past the cap everything is suspect, under a named cause"
        );
        assert_eq!(s.overflows, 1);
        assert_eq!(
            (s.rescans_total, s.rescans.as_slice()),
            (1, &[RescanCause::Overflow][..])
        );
        assert!(s.dirty.is_empty(), "the enumeration is dropped, not kept");
        assert_eq!(s.feed.generation(), DIRTY_CAP as u64 + 1);
        assert_eq!(s.feed.losses(), 1, "the collapse is LOUD event loss");
        // Late arrivals during a collapse change nothing.
        s.insert(Path::new("late.md"));
        assert!(s.dirty.is_empty());
        assert_eq!(s.take(), Pending::All(RescanCause::Overflow));
        assert_eq!(s.take(), Pending::Clean);
    }

    /// A doubt is sticky until taken; a lower-rung cause landing on an open
    /// doubt never downgrades it; a higher rung upgrades; every mark stays
    /// in the record.
    #[test]
    fn a_higher_rung_doubt_is_never_downgraded() {
        let mut s = FeedState::default();
        s.insert(Path::new("a.md"));
        s.collapse(RescanCause::InstanceChange);
        s.collapse(RescanCause::Overflow);
        assert_eq!(
            s.doubt,
            Some(RescanCause::InstanceChange),
            "the re-baseline rung outranks the sweep rung"
        );
        assert_eq!(
            s.rescans.as_slice(),
            &[RescanCause::InstanceChange, RescanCause::Overflow][..],
        );
        assert_eq!((s.rescans_total, s.overflows), (2, 2));
        assert!(s.doubt.is_some() && s.dirty.is_empty());

        let mut up = FeedState::default();
        up.collapse(RescanCause::Overflow);
        up.collapse(RescanCause::InstanceChange);
        assert_eq!(
            up.doubt,
            Some(RescanCause::InstanceChange),
            "a later instance-change upgrades an open sweep"
        );
    }

    /// Apply, path arm: a changed member lands through the overlay (spoiled
    /// identity — the next observation re-verifies), an unchanged member
    /// applies nothing, a vanished member leaves the fold. All against a
    /// baselined memo.
    #[test]
    fn apply_re_derives_changed_members_and_skips_echoes() {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::WorkspaceRoot(dir.path().to_path_buf());
        std::fs::write(dir.path().join("a.md"), "# A\n").unwrap();
        std::fs::write(dir.path().join("b.md"), "# B\n").unwrap();
        std::fs::write(dir.path().join("c.md"), "# C\n").unwrap();
        let mut cache = fs::DomainCache::new();
        let baseline = cache.root(&root).unwrap();

        std::fs::write(dir.path().join("a.md"), "# A moved\n").unwrap();
        std::fs::remove_file(dir.path().join("c.md")).unwrap();
        let pending = Pending::Paths(vec![
            PathBuf::from("a.md"),
            PathBuf::from("b.md"), // untouched — the echo arm
            PathBuf::from("c.md"), // vanished
        ]);
        assert_eq!(
            apply(&root, &mut cache, pending),
            Applied::Members(2),
            "a and c apply; b is an unchanged echo"
        );
        let after = cache.root(&root).unwrap();
        assert_ne!(baseline, after);
        // The applied state equals a fresh derivation of the same disk.
        let fresh = fs::DomainCache::new().root(&root).unwrap();
        assert_eq!(after, fresh);
    }

    /// §6.7: a FOREIGN domain-config dirty path escalates to the Sweep rung
    /// with the loss noted — never folded as an ordinary leaf under the
    /// superseded membership — while the governed write's own echo (digest
    /// already imposed on the overlay) applies as the ordinary no-op.
    #[test]
    fn a_foreign_domain_config_dirty_path_sweeps_instead_of_folding() {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::WorkspaceRoot(dir.path().to_path_buf());
        let config_abs = dir.path().join(fs::domain::DOMAIN_CONFIG_PATH);
        std::fs::create_dir_all(config_abs.parent().unwrap()).unwrap();
        std::fs::write(&config_abs, "---\nignore:\n  - \"drafts/**\"\n---\n").unwrap();
        std::fs::write(dir.path().join("a.md"), "# A\n").unwrap();
        std::fs::write(dir.path().join("b.md"), "# B\n").unwrap();
        let mut cache = fs::DomainCache::new();
        cache.root(&root).unwrap();

        // Echo: the config's bytes match the overlay's leaf — ordinary skip.
        let config_rel = PathBuf::from(fs::domain::DOMAIN_CONFIG_PATH);
        let losses_before = cache.feed_gen().losses();
        assert_eq!(
            apply(&root, &mut cache, Pending::Paths(vec![config_rel.clone()])),
            Applied::Members(0),
            "an echo of the governed write applies as the ordinary no-op"
        );
        assert_eq!(cache.feed_gen().losses(), losses_before);

        // Foreign edit: membership changed on disk — the overlay may not
        // serve it; the Sweep rung with a LOUD loss is the honest answer.
        std::fs::write(&config_abs, "---\nignore:\n  - \"b.md\"\n---\n").unwrap();
        assert_eq!(
            apply(&root, &mut cache, Pending::Paths(vec![config_rel])),
            Applied::Sweep(RescanCause::DomainConfig),
            "a foreign config edit sweeps — folding it would stamp a root no \
             true corpus state ever had"
        );
        assert_eq!(
            cache.feed_gen().losses(),
            losses_before + 1,
            "the escalation is loud: guard currency is untrusted until the \
             floor observation absorbs it"
        );
        // The floor under the fresh config equals a from-scratch derivation.
        let after = cache.root(&root).unwrap();
        let fresh = fs::DomainCache::new().root(&root).unwrap();
        assert_eq!(after, fresh);
    }

    /// §6.7 holdoff: after a timed-out barrier, further barriers answer
    /// `Unproven` immediately (no second timeout is paid) until the holdoff
    /// lapses; with the holdoff zeroed the stream is re-probed and a live
    /// watcher earns `Seen` again.
    #[test]
    fn a_timed_out_barrier_holds_off_the_next_probe() {
        let watched = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let root = watched.path().canonicalize().unwrap();
        let feed = WorkspaceFeed::start(&root, fs::stable::FeedGen::default()).expect("watcher");

        // Force a deterministic timeout: the sentinel lands under a root the
        // stream never carries.
        assert_eq!(
            feed.cookie_barrier(
                &elsewhere.path().canonicalize().unwrap(),
                Duration::from_millis(50)
            ),
            CookieOutcome::Unproven
        );
        // Drain the CookieTimeout doubt so the next verdict is the holdoff's.
        assert!(matches!(
            feed.take(),
            Pending::All(RescanCause::CookieTimeout)
        ));

        let asked = Instant::now();
        assert_eq!(
            feed.cookie_barrier(&root, Duration::from_secs(10)),
            CookieOutcome::Unproven,
            "inside the holdoff the barrier answers immediately"
        );
        assert!(
            asked.elapsed() < Duration::from_secs(2),
            "the holdoff never re-pays the timeout"
        );

        feed.set_cookie_holdoff(Duration::ZERO);
        assert_eq!(
            feed.cookie_barrier(&root, Duration::from_secs(10)),
            CookieOutcome::Seen,
            "past the holdoff one probe re-tests the stream, and a live \
             watcher clears the mark"
        );
    }

    /// Apply, sweep rung: the memo is KEPT — the next observation re-verifies
    /// every member by live stat and reads NOTHING on an unmoved corpus (the
    /// lane-B 160 ms class, vs the 1.45 s full re-read a reset would force) —
    /// and it re-baselines guard currency by absorbing the noted loss.
    #[test]
    fn a_sweep_rescan_keeps_the_memo_and_the_sweep_absorbs_the_loss() {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::WorkspaceRoot(dir.path().to_path_buf());
        std::fs::write(dir.path().join("a.md"), "# A\n").unwrap();
        std::fs::write(dir.path().join("b.md"), "# B\n").unwrap();
        let mut cache = fs::DomainCache::new();
        let baseline = cache.root(&root).unwrap();
        let reads = cache.leaves_read();
        let sweeps = cache.sweeps();

        // Mark the doubt the way the feed does: loss noted, then all-dirty.
        cache.feed_gen().note_loss("overflow");
        assert!(
            matches!(
                cache.guard_currency(),
                fs::stable::GuardCurrency::Untrusted { .. }
            ),
            "an unabsorbed loss refuses vouching"
        );
        assert_eq!(
            apply(&root, &mut cache, Pending::All(RescanCause::Overflow)),
            Applied::Sweep(RescanCause::Overflow)
        );
        assert_eq!(
            cache.leaves_read(),
            reads,
            "the memo survived the rescan mark"
        );

        // The piggybacked observation IS the full stat sweep: same root,
        // zero byte reads on an unmoved corpus, loss absorbed.
        assert_eq!(cache.root(&root).unwrap(), baseline);
        assert_eq!(cache.sweeps(), sweeps + 1, "the sweep ran");
        assert_eq!(
            cache.leaves_read(),
            reads,
            "an unmoved corpus sweeps by stat alone — no member re-read"
        );
        assert_eq!(
            cache.guard_currency(),
            fs::stable::GuardCurrency::Trusted,
            "the completed sweep re-baselined guard currency"
        );
    }

    /// Apply, re-baseline rung: an instance change re-derives the memo from
    /// disk into a fresh one committed by swap — the swapped memo equals a
    /// from-scratch derivation and read the whole corpus.
    #[test]
    fn an_instance_change_rebaselines_by_swap() {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::WorkspaceRoot(dir.path().to_path_buf());
        std::fs::write(dir.path().join("a.md"), "# A\n").unwrap();
        std::fs::write(dir.path().join("b.md"), "# B\n").unwrap();
        let mut cache = fs::DomainCache::new();
        cache.root(&root).unwrap();

        // A change the (dead) stream never delivered.
        std::fs::write(dir.path().join("a.md"), "# A moved unseen\n").unwrap();
        assert_eq!(
            apply(&root, &mut cache, Pending::All(RescanCause::InstanceChange)),
            Applied::Rebaselined(RescanCause::InstanceChange)
        );
        assert_eq!(
            cache.leaves_read(),
            2,
            "the swapped-in memo is fresh: it read the whole corpus"
        );
        let fresh = fs::DomainCache::new().root(&root).unwrap();
        assert_eq!(cache.root(&root).unwrap(), fresh);
    }

    /// Apply, doubt arm on a cold memo: with no baseline the sweep rung is
    /// moot by construction (the first pass reads everything), and the
    /// re-baseline rung still lands a correct fresh memo.
    #[test]
    fn a_cold_memo_survives_both_rungs() {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::WorkspaceRoot(dir.path().to_path_buf());
        std::fs::write(dir.path().join("a.md"), "# A\n").unwrap();

        let mut cold = fs::DomainCache::new();
        assert_eq!(
            apply(&root, &mut cold, Pending::All(RescanCause::VouchFailure)),
            Applied::Sweep(RescanCause::VouchFailure)
        );
        assert_eq!(cold.leaves_read(), 0, "nothing read at the mark");
        cold.root(&root).unwrap();
        assert_eq!(cold.leaves_read(), 1, "the cold first pass reads it all");

        let mut cold2 = fs::DomainCache::new();
        assert_eq!(
            apply(&root, &mut cold2, Pending::All(RescanCause::InstanceChange)),
            Applied::Rebaselined(RescanCause::InstanceChange)
        );
        assert_eq!(cold2.leaves_read(), 1);
    }

    /// An apply-time I/O failure that is not absence still resets, and the
    /// shared generation cell survives the reset so the fence stays one
    /// instrument.
    #[test]
    fn an_io_failure_resets_and_keeps_the_feed_cell() {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::WorkspaceRoot(dir.path().to_path_buf());
        std::fs::write(dir.path().join("a.md"), "# A\n").unwrap();
        let mut cache = fs::DomainCache::new();
        cache.root(&root).unwrap();
        let handle = cache.feed_gen();
        // A directory named like a member: read fails with IsADirectory, not
        // NotFound — absence would be a lie, so the memo resets.
        std::fs::create_dir(dir.path().join("ghost.md")).unwrap();
        assert_eq!(
            apply(
                &root,
                &mut cache,
                Pending::Paths(vec![PathBuf::from("ghost.md")])
            ),
            Applied::Reset
        );
        handle.advance();
        assert_eq!(
            cache.feed_gen().generation(),
            handle.generation(),
            "the reset memo still rides the same generation cell"
        );
    }

    /// Measurement, not a gate: how often does mkdir+child lose the child
    /// event on THIS backend? Run with `--ignored --nocapture` on each OS.
    /// Unfixed code: `collapsed` stays 0; `missed` vs `captured` is the gap
    /// rate. Inherited comments are not a measurement.
    #[test]
    #[ignore = "measurement probe; run on each watch backend"]
    fn measure_newdir_child_capture() {
        const TRIALS: u32 = 20;
        let mut captured = 0u32;
        let mut missed = 0u32;
        let mut collapsed = 0u32;
        for i in 0..TRIALS {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().canonicalize().unwrap();
            let feed =
                WorkspaceFeed::start(&root, fs::stable::FeedGen::default()).expect("watcher");
            assert_eq!(
                feed.cookie_barrier(&root, Duration::from_secs(5)),
                CookieOutcome::Seen,
                "trial {i}: watch must be live before the mkdir"
            );
            let new = root.join("new");
            std::fs::create_dir(&new).unwrap();
            std::fs::write(new.join("x.md"), "# X\n").unwrap();
            let start = Instant::now();
            let mut saw_member = false;
            let mut saw_collapse = false;
            while start.elapsed() < Duration::from_secs(2) {
                let s = feed.stats();
                if s.all_dirty {
                    saw_collapse = true;
                    break;
                }
                if s.events > 0 || s.pending > 0 {
                    saw_member = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            if saw_collapse {
                collapsed += 1;
            } else if saw_member {
                captured += 1;
            } else {
                missed += 1;
            }
        }
        eprintln!(
            "newdir arming measurement backend={} captured={captured} missed={missed} collapsed={collapsed} trials={TRIALS}",
            std::env::consts::OS
        );
    }

    /// Ladder rung latencies on the fixture, recorded with grades. The 160 ms
    /// / 1.45 s classes are the 29 k-member production numbers; this fixture
    /// is two files, so the grades are the CLASS (O(1) mark / stat-sweep /
    /// rebuild), not those wall times.
    #[test]
    fn ladder_rung_latencies_carry_grades() {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::WorkspaceRoot(dir.path().to_path_buf());
        std::fs::write(dir.path().join("a.md"), "# A\n").unwrap();
        std::fs::write(dir.path().join("b.md"), "# B\n").unwrap();
        let mut cache = fs::DomainCache::new();
        cache.root(&root).unwrap();

        let t0 = Instant::now();
        assert_eq!(
            apply(&root, &mut cache, Pending::All(RescanCause::Overflow)),
            Applied::Sweep(RescanCause::Overflow)
        );
        let sweep_mark = t0.elapsed();

        let t1 = Instant::now();
        cache.root(&root).unwrap();
        let sweep_observe = t1.elapsed();

        let t2 = Instant::now();
        assert_eq!(
            apply(&root, &mut cache, Pending::All(RescanCause::InstanceChange)),
            Applied::Rebaselined(RescanCause::InstanceChange)
        );
        let rebaseline = t2.elapsed();

        eprintln!(
            "ladder_rung sweep_mark={sweep_mark:?} grade=A-O(1)-no-io \
             sweep_observe={sweep_observe:?} grade=B-stat-sweep \
             rebaseline={rebaseline:?} grade=rebuild-by-swap"
        );
        assert!(
            sweep_mark.as_millis() < 50,
            "sweep mark is a return, not I/O: {sweep_mark:?}"
        );
        assert!(
            sweep_observe.as_secs() < 2,
            "two-file stat sweep stayed in the fixture envelope: {sweep_observe:?}"
        );
        assert!(
            rebaseline.as_secs() < 2,
            "two-file re-baseline stayed in the fixture envelope: {rebaseline:?}"
        );
    }
}
