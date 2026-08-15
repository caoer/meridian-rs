//! Resident daemon delta plane: one ring per workspace, and the detector that
//! feeds it.
//!
//! Ring + change classifier live in `wire-serve` (one law, two hosts). Here:
//! shared-state wrapping (N connection threads) and a detector — a subscriber
//! sends no lines, so nothing else would drive reconcile on its behalf.
//!
//! # Two producers, one critical section
//! The write choke-point allocates its `seq` AND records its frame under the
//! workspace write flock (`SeqSink::committed`); the detector allocates and
//! records under the same flock. So a detect cycle taking the flock after a
//! write finds the tip already at the moved root and syncs silently — the
//! just-committed change is never re-told as an actor-absent external frame
//! (the seq:655 double-emission window, closed).
//!
//! # Flock is the serialization point
//! Detect reconcile holds the workspace write flock so it never classifies a
//! torn mid-batch state. Pre-check (unlocked fold vs baseline) keeps quiet
//! cycles off the flock so writers are not spuriously `workspace_busy`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use wire::{DeltaFrame, ErrorBody, ErrorCode, Root};
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
    /// The detector's currency instrument AND its single-flight token: the
    /// leaf memo ([`fs::DomainCache`]) a cycle stats the corpus through, held
    /// for the whole cycle. `detect` try-locks it — a fold in flight means
    /// this cadence's work is already being done, so the caller skips instead
    /// of starting a duplicate (the incident shape: N subscriber threads ×
    /// one full-tree re-digest each, continuously, because the cadence gate
    /// only sees COMPLETED cycles). `prime` blocks on it — a subscribe must
    /// observe a baseline, not skip.
    fold: Mutex<fs::DomainCache>,
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
            fold: Mutex::new(fs::DomainCache::new()),
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

    /// Record an emitted frame (detector records its own inside reconcile).
    ///
    /// Both remaining callers hold the workspace flock: the run plane's sink
    /// (`delta_sink`) under the executor's, and the write choke-point through
    /// `SeqSink::committed` under its own. [`RootRing::allocate_seq`]'s
    /// reserve remains the floor for any producer that unwinds between
    /// allocation and this call: its number stays burned, never re-issued.
    pub fn advance(&self, frame: DeltaFrame) {
        self.state().ring.advance(frame);
    }

    /// One detection cycle unless another subscriber ran within [`DETECT_CADENCE`].
    ///
    /// Gates in order:
    /// 1. **Coalesce** — recent cycle ⇒ do nothing (N subscribers, one fold).
    /// 2. **Single-flight** — a cycle in flight ⇒ do nothing. The cadence
    ///    stamp lands at cycle COMPLETION, so without this gate every
    ///    subscriber thread that ticks while a slow cycle runs starts its own
    ///    (the deploy-7 incident: 23 connections × one continuous full-tree
    ///    re-digest each, multi-core pegged, face ops starved).
    /// 3. **Pre-check** — leaf-memo currency pass vs baseline; unchanged root
    ///    updates cadence and returns without the write flock (quiet cycles
    ///    dominate; flock is `LOCK_EX|LOCK_NB`, so holding it would refuse
    ///    concurrent splices).
    /// 4. **Reconcile under flock** — only when root moved. `WouldBlock` ⇒ write
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
        // Gate 2: the winner's cycle IS this cadence's detection; frames it
        // emits land on the shared ring, which every push loop drains.
        let Ok(mut fold) = self.fold.try_lock() else {
            return Ok(false);
        };
        self.cycle(ws_root, &mut fold)
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
        // Blocking, not try: a subscribe must observe a baseline. Bounded by
        // one in-flight cycle; before single-flight it would have run its own
        // concurrent fold instead.
        let mut fold = self.fold.lock().unwrap_or_else(PoisonError::into_inner);
        self.cycle(ws_root, &mut fold)?;
        let state = self.state();
        match state.watch.root() {
            Some(root) => Ok(root.clone()),
            // Cycle skipped (write held flock). Disk root is still a truthful
            // anchor; in-flight write's change is detected later.
            None => wire_serve::ambient_root(ws_root),
        }
    }

    /// One detection cycle, cadence ignored. Gates described on [`Self::detect`].
    fn cycle(
        &self,
        ws_root: &fs::WorkspaceRoot,
        fold: &mut fs::DomainCache,
    ) -> Result<bool, Box<ErrorBody>> {
        // Unlocked currency pass — gate 3 on `detect`. One `stat` per member,
        // bytes only for members whose identity moved: the same leaf memo (and
        // the same evidence standing) the registry's `warm_or_build` currency
        // pass serves reads by, byte-identical to `domain_snapshot`'s fold.
        // The old pre-check re-read and re-digested every member on every
        // cycle — G11's defect on the watch plane.
        let disk_root = Root(fold.root(ws_root).map_err(|e| io_error_body(&e))?.0);
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

/// The memo pass's refusal in the envelope [`wire_serve::ambient_root`] always
/// answered with: wire `io_error`, cause carried (member refusals arrive
/// pre-named by `fs::corpus_member_refusal`).
fn io_error_body(e: &std::io::Error) -> Box<ErrorBody> {
    let mut err = ErrorBody::new(ErrorCode::IoError);
    err.cause = Some(e.to_string());
    Box::new(err)
}

/// Write-path allocator and recorder: registry `seq` from the same ring the
/// detector numbers from — two producers, one chain.
///
/// State lock only for the bump / the record. Lock order is flock → ring
/// state on both producers; no path takes them the other way.
impl wire_serve::seq::SeqSink for WorkspaceRing {
    fn allocate(&self, _before: &Root, _after: &Root, _files: &[wire::DeltaFile]) -> u64 {
        self.state().ring.allocate_seq()
    }

    /// The write path's frame, recorded while the choke-point still holds the
    /// workspace flock — the `delta_sink` pattern, closing the detector
    /// window on the write paths: a detect cycle taking the flock next finds
    /// the tip already at the moved root and syncs silently.
    fn committed(&self, frame: &DeltaFrame) {
        self.advance(frame.clone());
    }
}
