//! U20b — the resident daemon's delta plane: **one ring per workspace**, and the
//! detector that feeds it.
//!
//! The ring itself and the change classifier are `wire-serve`'s (one law, two
//! hosts). What lives here is everything the registry needs and the sidecar does
//! not: shared-state wrapping (the sidecar's ring is a serve-loop local; this one
//! is reached from N connection threads), and a DETECTOR to replace the line
//! boundary the sidecar reconciles at — a subscriber sends no lines, so nothing
//! would ever drive a reconcile on its behalf.
//!
//! # Why the detector is the sole producer, for now
//! Until the `SeqSink` change lands at the `wire-serve` write choke-point (its
//! own serialized car on the `write.rs` line), the registry's OWN splices are
//! seen by this detector as EXTERNAL changes. Their frames therefore carry
//! `actor`/`now` ABSENT — §7.1, the engine never invents identity it was not
//! given. That is missing attribution, never wrong data, and the chain stays
//! contiguous precisely BECAUSE there is exactly one producer. When `SeqSink`
//! arrives, the write path allocates its own `seq` under the flock this detector
//! already respects, the classifier's internal-commit arm starts firing, and
//! those frames become attributed.
//!
//! # The flock is the serialization point
//! A detection that folded the corpus while a batch was landing would classify a
//! TORN state — the sidecar documents exactly that as a stated degrade. This
//! detector instead reconciles while holding the workspace write flock, so it
//! only ever observes a quiescent workspace and the degrade does not exist here.
//! The pre-check below is what keeps that cheap for *writers*.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use wire::{DeltaFrame, ErrorBody, Root};
use wire_serve::ring::RootRing;
use wire_serve::watch::WatchState;

/// How often a subscribed workspace folds its corpus looking for external
/// change.
///
/// **250ms, and the number is a trade, not a taste.** It is the push-latency
/// floor for an edit made outside the engine: a human saving in Obsidian sees
/// the notification within a quarter second. The pre-warm sweep's 1s (the
/// obvious value to copy) is perceptible in an editor round-trip, which is the
/// whole experience this op exists to serve. Against that, 4 folds/second on a
/// subscribed workspace is the same order as the pre-warm sweep this daemon
/// already runs, so the cost is one the process was already paying — and it is
/// paid ONLY while someone is subscribed.
///
/// Coalesced across subscribers ([`WorkspaceRing::detect`]): N subscribers on
/// one workspace fold once per cadence between them, not N times. Without that,
/// this constant would silently mean "per subscriber" and a fleet of watchers
/// would melt a large corpus.
pub const DETECT_CADENCE: Duration = Duration::from_millis(250);

/// One workspace's delta plane: the retained ring, the watcher baseline, and the
/// bookkeeping that lets many connection threads share them.
#[derive(Debug)]
pub struct WorkspaceRing {
    state: Mutex<RingState>,
    /// Live subscriptions. Drives two things: whether anyone should be
    /// detecting at all, and whether the reaper may drop this workspace.
    subscribers: AtomicUsize,
}

#[derive(Debug)]
struct RingState {
    ring: RootRing,
    watch: WatchState,
    /// When a detection cycle last COMPLETED — the coalescing window. `None`
    /// until the first cycle, so the first subscriber detects immediately
    /// instead of waiting out a cadence it did not cause.
    last_detect: Option<Instant>,
}

/// A live subscription's claim on a ring. Decrements on drop, so a dropped
/// connection — clean EOF, broken pipe, or a panicking thread — always
/// releases its claim; a leaked count would keep a workspace un-reapable
/// forever, which is the failure mode that made this a guard rather than a
/// pair of calls.
#[derive(Debug)]
pub struct SubGuard<'a> {
    ring: &'a WorkspaceRing,
}

impl Drop for SubGuard<'_> {
    fn drop(&mut self) {
        self.ring.subscribers.fetch_sub(1, Ordering::SeqCst);
    }
}

impl WorkspaceRing {
    /// A fresh epoch for `root`: empty ring, unprimed watcher, no subscribers.
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
        // A poisoned ring means a detector panicked mid-cycle. The ring's own
        // invariants are upheld by `RootRing` (append + bounded evict), so the
        // recovered state is consistent; refusing to serve subscribers because a
        // previous cycle panicked would turn a transient fault into a dead
        // workspace.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The tip seq — the position a `sub` with no catchup anchors at.
    #[must_use]
    pub fn seq(&self) -> u64 {
        self.state().ring.seq()
    }

    /// The §7.1 anchor law, delegated to the shared ring so both hosts refuse
    /// the same positions.
    #[must_use]
    pub fn can_anchor(&self, from_seq: u64) -> bool {
        self.state().ring.can_anchor(from_seq)
    }

    /// Register a subscription. The guard's lifetime IS the subscription's.
    pub fn subscribe(&self) -> SubGuard<'_> {
        self.subscribers.fetch_add(1, Ordering::SeqCst);
        SubGuard { ring: self }
    }

    /// Is anyone watching? The reaper asks this before dropping a workspace, and
    /// the detector asks it before doing any work.
    #[must_use]
    pub fn has_subscribers(&self) -> bool {
        self.subscribers.load(Ordering::SeqCst) > 0
    }

    /// Frames the caller has not seen, in emission order.
    #[must_use]
    pub fn frames_after(&self, delivered: u64) -> Vec<DeltaFrame> {
        self.state().ring.frames_after(delivered)
    }

    /// Record a frame the WRITE path emitted (the detector records its own).
    ///
    /// This lands AFTER `splice` has returned and dropped the flock, which is
    /// the whole reason [`RootRing::allocate_seq`] exists: between the
    /// allocation inside the flock and this call, a detection cycle can take the
    /// flock, allocate, and advance ahead of us. Its number cannot be ours.
    pub fn advance(&self, frame: DeltaFrame) {
        self.state().ring.advance(frame);
    }

    /// Run one detection cycle, unless another subscriber already ran one within
    /// [`DETECT_CADENCE`]. Returns whether this call actually reconciled.
    ///
    /// Three gates, in this order, and the order is the whole design:
    ///
    /// 1. **Coalesce** — a recent cycle means someone else already looked; do
    ///    nothing. This is what makes N subscribers cost one fold.
    /// 2. **Pre-check** — fold the disk root and compare it to the watcher's
    ///    baseline. An unchanged root is the disposition's own first arm ("a
    ///    cycle finding nothing emits nothing"), reached WITHOUT taking the
    ///    write flock. This does not save the fold — `ambient_root` IS
    ///    `domain_snapshot().1` — it saves the LOCK, and that is the point: the
    ///    flock is `LOCK_EX|LOCK_NB`, so a detector holding it turns a client's
    ///    concurrent splice into a `workspace_busy` refusal. Quiet cycles
    ///    dominate, so writers almost never meet this detector at all.
    /// 3. **Reconcile under the flock** — only on a cycle that has already seen
    ///    the root move. `WouldBlock` means a write is landing right now: skip,
    ///    because that write's own change will still be there next cycle. Never
    ///    block: a detector that waited on the flock would hold a subscriber's
    ///    thread hostage to an unrelated writer.
    ///
    /// # Errors
    /// A snapshot or classification failure. Callers log and continue: an
    /// unreadable workspace is transient and never a reason to end a
    /// subscription.
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

    /// Establish this ring's baseline and return the root it settled on — **run
    /// at subscribe time, before the ack is written.**
    ///
    /// A `WatchState` is unprimed until its first reconcile, and that first
    /// reconcile emits NOTHING: it adopts the world as the baseline. So a
    /// subscription that acked first and primed later would silently swallow
    /// every change that landed in between — the client would hold an ack, a
    /// healthy socket, and no frame for an edit that really happened. Measured,
    /// not theorised: the end-to-end gate failed exactly this way before the ack
    /// primed.
    ///
    /// Priming here also makes the ack HONEST. The §4.7 ack body is the
    /// subscription's anchor tense; returning the baseline root means the first
    /// frame's `root_before` is exactly the root the ack named, so the client's
    /// chain starts where it was told it starts.
    ///
    /// # Errors
    /// A snapshot or classification failure — a `sub` that cannot establish a
    /// baseline is refused rather than acked into a stream it cannot anchor.
    pub fn prime(&self, ws_root: &fs::WorkspaceRoot) -> Result<Root, Box<ErrorBody>> {
        self.cycle(ws_root)?;
        let state = self.state();
        match state.watch.root() {
            Some(root) => Ok(root.clone()),
            // `cycle` skipped: a write held the flock. The disk root is still a
            // truthful anchor — the in-flight write's own change is detected on
            // a later cycle and chains from wherever the baseline lands.
            None => wire_serve::ambient_root(ws_root),
        }
    }

    /// One detection cycle, cadence ignored. The gates below are described on
    /// [`Self::detect`].
    fn cycle(&self, ws_root: &fs::WorkspaceRoot) -> Result<bool, Box<ErrorBody>> {
        // Unlocked fold — see gate 2 on `detect`. Held across no lock of ours.
        let disk_root: Root = wire_serve::ambient_root(ws_root)?;
        {
            let mut state = self.state();
            if state.watch.root() == Some(&disk_root) {
                state.last_detect = Some(Instant::now());
                return Ok(false);
            }
        }
        // The root moved. Take the write flock so the reconcile below cannot
        // observe a batch mid-landing.
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

/// The write path's allocator: the registry's `seq` comes from the SAME ring the
/// detector numbers from, which is what makes the two producers one chain.
///
/// `&self` through the `Arc`, and the state lock is taken only for the bump — an
/// allocation never holds the ring across a caller's critical section. The lock
/// ORDER is flock → ring state on BOTH producers: the write path holds the flock
/// and allocates through here, and [`WorkspaceRing::cycle`] takes the flock
/// before it touches state. No path takes them the other way round.
impl wire_serve::seq::SeqSink for WorkspaceRing {
    fn allocate(&self, _before: &Root, _after: &Root, _files: &[wire::DeltaFile]) -> u64 {
        self.state().ring.allocate_seq()
    }
}
