//! Resident daemon delta plane: one ring per workspace, and the detector that
//! feeds it.
//!
//! Ring + change classifier live in `wire-serve` (one law, two hosts). Here:
//! shared-state wrapping (N connection threads) and a detector — a subscriber
//! sends no lines, so nothing else would drive reconcile on its behalf.
//!
//! # Sole producer (until write-path `SeqSink` attribution)
//! Until the write choke-point allocates `seq` under the flock, the registry's
//! own splices look external to this detector: frames carry `actor`/`now`
//! absent (§7.1 — engine never invents identity). Missing attribution, not
//! wrong data; chain stays contiguous because there is exactly one producer.
//!
//! # Flock is the serialization point
//! Detect reconcile holds the workspace write flock so it never classifies a
//! torn mid-batch state. Pre-check (unlocked fold vs baseline) keeps quiet
//! cycles off the flock so writers are not spuriously `workspace_busy`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use wire::{DeltaFrame, ErrorBody, Root};
use wire_serve::ring::RootRing;
use wire_serve::watch::WatchState;

/// How often a subscribed workspace folds looking for external change.
///
/// Sets the push-latency floor for an edit made outside the engine. Folds run
/// only while someone is subscribed, and are coalesced across subscribers
/// ([`WorkspaceRing::detect`]) so N watchers fold once per cadence, not N times.
pub const DETECT_CADENCE: Duration = Duration::from_millis(250);

/// One workspace's delta plane: retained ring, watcher baseline, multi-thread bookkeeping.
#[derive(Debug)]
pub struct WorkspaceRing {
    state: Mutex<RingState>,
    /// Live subscriptions — drives detection and reaper exemption.
    subscribers: AtomicUsize,
}

#[derive(Debug)]
struct RingState {
    ring: RootRing,
    watch: WatchState,
    /// When a detection cycle last completed (coalescing window). `None` until
    /// first cycle so the first subscriber detects immediately.
    last_detect: Option<Instant>,
}

/// Live subscription claim. Decrements on drop so EOF, broken pipe, or panic
/// always releases; a leaked count would keep the workspace un-reapable forever.
///
/// Owns its ring (`Arc`), so the claim is created where the `sub` is accepted
/// and carried into the push plane — there is no arm-to-convert gap in which
/// an acked subscription is still reapable.
#[derive(Debug)]
pub struct SubGuard {
    ring: Arc<WorkspaceRing>,
}

impl SubGuard {
    /// The subscribed ring — the same epoch the `sub` was acked on.
    #[must_use]
    pub fn ring(&self) -> &Arc<WorkspaceRing> {
        &self.ring
    }
}

impl Drop for SubGuard {
    fn drop(&mut self) {
        self.ring.subscribers.fetch_sub(1, Ordering::SeqCst);
    }
}

impl WorkspaceRing {
    /// Fresh epoch for `root`: empty ring, unprimed watcher, no subscribers.
    #[must_use]
    pub fn new(root: &fs::WorkspaceRoot) -> Self {
        WorkspaceRing {
            state: Mutex::new(RingState {
                ring: RootRing::new(),
                watch: WatchState::new(root),
                last_detect: None,
            }),
            subscribers: AtomicUsize::new(0),
        }
    }

    fn state(&self) -> MutexGuard<'_, RingState> {
        // Poisoned = detector panicked mid-cycle. `RootRing` invariants hold
        // (append + bounded evict); recover rather than leave the workspace dead.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Tip seq — position a `sub` with no catchup anchors at.
    #[must_use]
    pub fn seq(&self) -> u64 {
        self.state().ring.seq()
    }

    /// §7.1 anchor law, delegated to the shared ring (both hosts refuse the same positions).
    #[must_use]
    pub fn can_anchor(&self, from_seq: u64) -> bool {
        self.state().ring.can_anchor(from_seq)
    }

    /// Register a subscription. Guard lifetime is the subscription's.
    pub fn subscribe(self: &Arc<Self>) -> SubGuard {
        self.subscribers.fetch_add(1, Ordering::SeqCst);
        SubGuard {
            ring: Arc::clone(self),
        }
    }

    /// Anyone watching? Reaper and detector both ask this.
    #[must_use]
    pub fn has_subscribers(&self) -> bool {
        self.subscribers.load(Ordering::SeqCst) > 0
    }

    /// Frames the caller has not seen, in emission order.
    #[must_use]
    pub fn frames_after(&self, delivered: u64) -> Vec<DeltaFrame> {
        self.state().ring.frames_after(delivered)
    }

    /// Record a frame the write path emitted (detector records its own).
    ///
    /// Lands after `splice` drops the flock — why [`RootRing::allocate_seq`]
    /// exists: a detection cycle may allocate and advance between allocation
    /// inside the flock and this call; its number cannot be ours.
    pub fn advance(&self, frame: DeltaFrame) {
        self.state().ring.advance(frame);
    }

    /// One detection cycle unless another subscriber ran within [`DETECT_CADENCE`].
    ///
    /// Gates in order:
    /// 1. **Coalesce** — recent cycle ⇒ do nothing (N subscribers, one fold).
    /// 2. **Pre-check** — unlocked fold vs baseline; unchanged root updates
    ///    cadence and returns without the write flock (quiet cycles dominate;
    ///    flock is `LOCK_EX|LOCK_NB`, so holding it would refuse concurrent splices).
    /// 3. **Reconcile under flock** — only when root moved. `WouldBlock` ⇒ write
    ///    in flight; skip (never block a subscriber thread on an unrelated writer).
    ///
    /// # Errors
    /// Snapshot or classification failure. Callers log and continue.
    pub fn detect(&self, ws_root: &fs::WorkspaceRoot) -> Result<bool, Box<ErrorBody>> {
        {
            let state = self.state();
            if state
                .last_detect
                .is_some_and(|at| at.elapsed() < DETECT_CADENCE)
            {
                return Ok(false);
            }
        }
        self.cycle(ws_root)
    }

    /// Establish baseline and return the settled root — at subscribe time,
    /// before the ack is written.
    ///
    /// First reconcile of an unprimed `WatchState` emits nothing (adopts world
    /// as baseline). Ack-then-prime would swallow edits between; the §4.7 ack
    /// root must be that baseline so the first frame's `root_before` matches.
    ///
    /// # Errors
    /// Snapshot or classification failure — refuse rather than ack an unanchorable stream.
    pub fn prime(&self, ws_root: &fs::WorkspaceRoot) -> Result<Root, Box<ErrorBody>> {
        self.cycle(ws_root)?;
        let state = self.state();
        match state.watch.root() {
            Some(root) => Ok(root.clone()),
            // Cycle skipped (write held flock). Disk root is still a truthful
            // anchor; in-flight write's change is detected later.
            None => wire_serve::ambient_root(ws_root),
        }
    }

    /// One detection cycle, cadence ignored. Gates described on [`Self::detect`].
    fn cycle(&self, ws_root: &fs::WorkspaceRoot) -> Result<bool, Box<ErrorBody>> {
        // Unlocked fold — gate 2 on `detect`.
        let disk_root: Root = wire_serve::ambient_root(ws_root)?;
        {
            let mut state = self.state();
            if state.watch.root() == Some(&disk_root) {
                state.last_detect = Some(Instant::now());
                return Ok(false);
            }
        }
        // Root moved — take write flock so reconcile cannot observe mid-batch.
        let Ok(_write_lock) = fs::WriteLock::acquire(ws_root) else {
            return Ok(false); // a write is in flight; its change keeps
        };
        let mut state = self.state();
        let RingState { ring, watch, .. } = &mut *state;
        wire_serve::watch::reconcile(ws_root, ring, watch)?;
        state.last_detect = Some(Instant::now());
        Ok(true)
    }
}

/// Write-path allocator: registry `seq` from the same ring the detector
/// numbers from — two producers, one chain.
///
/// State lock only for the bump. Lock order is flock → ring state on both
/// producers; no path takes them the other way.
impl wire_serve::seq::SeqSink for WorkspaceRing {
    fn allocate(&self, _before: &Root, _after: &Root, _files: &[wire::DeltaFile]) -> u64 {
        self.state().ring.allocate_seq()
    }
}
