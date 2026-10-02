//! Resident daemon delta plane: one ring per workspace, and the detector that
//! feeds it.
//!
//! Ring + change classifier live in `wire-serve` (one law, one host). Here:
//! shared-state wrapping (N connection threads) and a detector — a subscriber
//! sends no lines, so nothing else would drive reconcile on its behalf.
//!
//! # Two producers, one critical section, one baseline
//! The write choke-point allocates its `seq` AND records its frame under the
//! workspace write flock (`SeqSink::committed`); the detector allocates and
//! records under the same flock. The law both keep: when a commit releases
//! the flock, the watcher baseline equals the ring tip. A write door holds it
//! at two points under its flock ([`DoorPlane`]): before bytes land, a
//! baseline behind the door's observation tells the pending external change
//! in its own actor-absent frame, numbered before the commit's; when the
//! frame is recorded, a baseline at its `root_before` moves onto its
//! `root_after`. So a just-committed change is never re-told as external
//! (the seq:655 double-emission window stays closed), an external edit beside
//! it is never swallowed by it, and a resumed cursor replays one chain — with
//! or without a subscriber.
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

/// How often a subscribed workspace looks for external change.
///
/// Sets the push-latency floor for an edit made outside the engine. Checks
/// run only while someone is subscribed, and are coalesced across subscribers
/// ([`WorkspaceRing::detect`]) so N watchers check once per cadence, not N
/// times. Since §6.7 the per-cadence check is O(1) while the workspace's
/// event feed vouches quiet; the fold pass runs on change, on any named
/// miss, and on the [`DETECT_FLOOR_CADENCE`] backstop.
pub const DETECT_CADENCE: Duration = Duration::from_millis(250);

/// §6.6's surviving poll — the fallback clock. Even under a continuously
/// quiet vouch the detector still runs its floor pass this often: the push
/// plane has no guard to touch a silently-lost event's scope (merkle-spec
/// §6.4 accepts that loss class on the memo plane BECAUSE guards catch it;
/// no guard reads a subscriber's frames), so the poll is its bounded
/// backstop — the staleness bound a silent capture loss can put on frames.
pub const DETECT_FLOOR_CADENCE: Duration = Duration::from_secs(30);

/// One workspace's delta plane: retained ring, watcher baseline, multi-thread bookkeeping.
#[derive(Debug)]
pub struct WorkspaceRing {
    state: Mutex<RingState>,
    /// The cycle's single-flight token. `detect` try-locks it — a cycle in
    /// flight means this cadence's work is already being done, so the caller
    /// skips instead of starting a duplicate (the incident shape: N
    /// subscriber threads × one full-tree re-digest each, continuously).
    /// `prime` blocks on it — a subscribe must observe a baseline, not skip.
    /// The private fold memo that used to double as this token is DELETED
    /// (§6.7): the SHARED registry memo is the one currency instrument on
    /// the watch plane too (card run-observation-unification).
    cycle_gate: Mutex<()>,
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
    /// When a FLOOR pass (the fold through the detector's memo) last ran —
    /// the [`DETECT_FLOOR_CADENCE`] fallback clock's anchor. `None` until
    /// the first cycle, so the backstop never pre-empts priming.
    last_floor: Option<Instant>,
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
                last_floor: None,
            }),
            cycle_gate: Mutex::new(()),
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

    /// This epoch's tree instance id (B-01) — taught on the `sub` ack, and
    /// the identity every resumption cursor must echo.
    #[must_use]
    pub fn instance(&self) -> String {
        self.state().ring.instance().to_string()
    }

    /// §7.1 + B-01 anchor law, delegated to the shared ring: instance before
    /// sequence, both refusals typed.
    #[must_use]
    pub fn can_anchor(&self, cursor: &wire_serve::ring::Cursor) -> wire_serve::ring::Anchor {
        self.state().ring.can_anchor(cursor)
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
    pub fn detect(
        &self,
        ws_root: &fs::WorkspaceRoot,
        registry: &crate::Registry,
    ) -> Result<bool, Box<ErrorBody>> {
        // §6.7 pre-check inputs, read and RELEASED before any registry
        // borrow: no path may hold this state lock while acquiring a memo
        // (the sanctioned order is memo → ring state, never the reverse).
        let (floor_due, baseline) = {
            let state = self.state();
            if state
                .last_detect
                .is_some_and(|at| at.elapsed() < DETECT_CADENCE)
            {
                return Ok(false);
            }
            let due = state
                .last_floor
                .is_none_or(|at| at.elapsed() >= DETECT_FLOOR_CADENCE);
            // Backstop due (or never primed): the floor cycle answers.
            (
                due,
                if due {
                    None
                } else {
                    state.watch.root().cloned()
                },
            )
        };
        // O(1) quiet check through the SHARED memo (§6.7): the feed vouches
        // nothing moved past this epoch's baseline — no walk, no stat, no
        // fold. A stale skip is latency-only: the next cadence re-asks, and
        // the fallback clock floors regardless.
        if let Some(baseline) = baseline
            && registry.vouched_quiet(&ws_root.0, &model::MerkleRoot(baseline.0))
        {
            self.state().last_detect = Some(Instant::now());
            return Ok(false);
        }
        // Gate 2: the winner's cycle IS this cadence's detection; frames it
        // emits land on the shared ring, which every push loop drains.
        let Ok(_flight) = self.cycle_gate.try_lock() else {
            return Ok(false);
        };
        self.cycle(ws_root, registry, floor_due)
    }

    /// Establish baseline and return the settled `(root, tip seq)` — at
    /// subscribe time, before the ack is written.
    ///
    /// First reconcile of an unprimed `WatchState` emits nothing (adopts world
    /// as baseline). Ack-then-prime would swallow edits between; the §4.7 ack
    /// root must be that baseline so the first frame's `root_before` matches.
    ///
    /// The pair is read under ONE state lock: the tip seq belongs to the same
    /// instant as the baseline root, so a live `sub` anchored at the acked
    /// seq can never skip a frame the acked root does not carry (a frame
    /// landing after this read is > the pair's seq and delivers).
    ///
    /// # Errors
    /// Snapshot or classification failure — refuse rather than ack an unanchorable stream.
    pub fn prime(
        &self,
        ws_root: &fs::WorkspaceRoot,
        registry: &crate::Registry,
    ) -> Result<(Root, u64), Box<ErrorBody>> {
        // Blocking, not try: a subscribe must observe a baseline. Bounded by
        // one in-flight cycle; before single-flight it would have run its own
        // concurrent fold instead.
        let _flight = self
            .cycle_gate
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        self.cycle(ws_root, registry, false)?;
        let state = self.state();
        let seq = state.ring.seq();
        match state.watch.root() {
            Some(root) => Ok((root.clone(), seq)),
            // Cycle skipped (write held flock). Disk root is still a truthful
            // anchor; in-flight write's change is detected later.
            None => Ok((wire_serve::ambient_root(ws_root)?, seq)),
        }
    }

    /// One detection cycle, cadence ignored. Gates described on [`Self::detect`].
    ///
    /// The cycle takes the workspace write flock FIRST — reconcile must never
    /// observe a mid-batch state — then makes the §6.1 door-grade observation
    /// through the SHARED memo (`Registry::door_observation`: cookie barrier →
    /// take-and-apply → overlay; the extent-refresh floor on any named miss),
    /// and hands the classifier that memo's leaf view and root
    /// (`wire_serve::watch::reconcile_delta` — bytes read for MOVERS only).
    /// `force_floor` is the §6.6 fallback clock: once per
    /// [`DETECT_FLOOR_CADENCE`] the backstop ([`crate::Registry::backstop`])
    /// compares disk to the memo unlocked and hints what the stream lost, so a
    /// silent capture loss is bounded, not forever.
    fn cycle(
        &self,
        ws_root: &fs::WorkspaceRoot,
        registry: &crate::Registry,
        force_floor: bool,
    ) -> Result<bool, Box<ErrorBody>> {
        let was_unprimed = self.state().watch.root().is_none();
        let cache = registry.domain_cache(&ws_root.0);
        // Unlocked pre-observation — quiet cycles stay OFF the flock so
        // writers are never spuriously `workspace_busy` (the module's
        // standing law). The floor-forced form runs the §6.6 backstop first:
        // an unlocked stat pass whose findings reach the memo as dirty hints,
        // so the observation below folds them — the memo is never held
        // across the corpus walk (merkle-spec §6.7, detect row).
        if force_floor {
            registry
                .backstop(&ws_root.0, &cache)
                .map_err(|e| io_error_body(&e))?;
        }
        let pre = registry
            .door_observation(&ws_root.0, &cache, crate::registry::DOOR_COOKIE_TIMEOUT)
            .map_err(|e| io_error_body(&e))?;
        {
            let mut state = self.state();
            if force_floor {
                // A true floor pass ran — anchor the §6.6 fallback clock.
                state.last_floor = Some(Instant::now());
            }
            if !was_unprimed && state.watch.root() == Some(&Root(pre.0.clone())) {
                state.last_detect = Some(Instant::now());
                return Ok(false);
            }
        }
        // Root moved (or priming) — take the write flock so reconcile cannot
        // observe mid-batch, and RE-observe under it: the flock-held
        // observation is the one the frame folds.
        let Ok(_write_lock) = fs::WriteLock::acquire(ws_root) else {
            return Ok(false); // a write is in flight; its change keeps
        };
        let observed = registry
            .door_observation(&ws_root.0, &cache, crate::registry::DOOR_COOKIE_TIMEOUT)
            .map_err(|e| io_error_body(&e))?;
        let emitted = self.reconcile_observed(ws_root, &cache, &Root(observed.0))?;
        let mut state = self.state();
        state.last_detect = Some(Instant::now());
        if was_unprimed {
            // The priming snapshot re-read everything — strictly stronger
            // than the stat floor; anchor the clock here too.
            state.last_floor = Some(Instant::now());
        }
        Ok(emitted)
    }

    /// The flock-held step, on an observation the caller just made through
    /// `cache`: classify the memo's leaf view against the baseline, then
    /// tell the change or sync silently (`wire_serve::watch::reconcile_delta`
    /// — bytes read for MOVERS only, each verified against its observed
    /// leaf). The detector and both write-door points run this one step; the
    /// caller holds the workspace write flock, so no batch is observed
    /// mid-landing.
    fn reconcile_observed(
        &self,
        ws_root: &fs::WorkspaceRoot,
        cache: &Arc<Mutex<fs::DomainCache>>,
        disk_root: &Root,
    ) -> Result<bool, Box<ErrorBody>> {
        // The memo's leaf view, String-spelled: non-UTF-8 NAMES stay
        // baseline-invisible exactly as the snapshot kept them (their leaves
        // still fold into the root; a `wire::Path` cannot spell them).
        let leaves: std::collections::BTreeMap<String, [u8; 32]> = {
            let memo = cache.lock().unwrap_or_else(PoisonError::into_inner);
            memo.leaf_digests()
                .into_iter()
                .filter_map(|(rel, digest)| rel.to_str().map(|s| (s.to_owned(), digest)))
                .collect()
        };
        let mut state = self.state();
        let RingState { ring, watch, .. } = &mut *state;
        Ok(wire_serve::watch::reconcile_delta(ws_root, ring, watch, &leaves, disk_root)?.is_some())
    }

    /// Baseline law, before bytes land: a write door holding the flock has
    /// just observed `observed` through `cache`. A primed baseline behind it
    /// is an external change no frame has told — told now, in its own
    /// actor-absent frame, so the commit's frame chains onto it. An unprimed
    /// watcher is left alone: priming is a full snapshot, a subscriber's
    /// cost, never a writer's.
    ///
    /// Never fails the write. A refused reconcile is logged, and a writer
    /// still mid-write holds the baseline (the race arm); either way
    /// [`Self::follow`] then leaves the baseline too, and the detector tells
    /// the change as external.
    fn settle(
        &self,
        ws_root: &fs::WorkspaceRoot,
        cache: &Arc<Mutex<fs::DomainCache>>,
        observed: &Root,
    ) {
        let behind = self
            .state()
            .watch
            .root()
            .is_some_and(|baseline| baseline != observed);
        if behind && let Err(e) = self.reconcile_observed(ws_root, cache, observed) {
            eprintln!("registry: write-door reconcile: {e:?}");
        }
    }

    /// Baseline law, at `committed`: `frame` is recorded and `cache` now
    /// folds to `observed`. A baseline at the frame's `root_before`, with the
    /// memo at its `root_after`, follows the commit: the movers are the
    /// commit's own files, re-read and verified against the memo's leaves,
    /// and the internal-commit arm syncs silently. Any other state stays for
    /// the detector — a baseline the door could not settle, a disk already
    /// past the commit, or a commit's file rewritten before the re-read.
    fn follow(
        &self,
        ws_root: &fs::WorkspaceRoot,
        cache: &Arc<Mutex<fs::DomainCache>>,
        frame: &DeltaFrame,
        observed: &Root,
    ) {
        let at_before = self.state().watch.root() == Some(&frame.delta.root_before);
        if at_before
            && *observed == frame.delta.root_after
            && let Err(e) = self.reconcile_observed(ws_root, cache, observed)
        {
            eprintln!("registry: commit baseline follow: {e:?}");
        }
    }

    /// The run plane's door point ([`Self::settle`]): its executor folds
    /// `root_before` itself, so the memo is observed only when a primed
    /// baseline is behind that root — a run on a workspace nobody watches
    /// pays nothing.
    pub(crate) fn settle_run(
        &self,
        registry: &crate::Registry,
        ws_root: &fs::WorkspaceRoot,
        root_before: &Root,
    ) {
        let behind = self
            .state()
            .watch
            .root()
            .is_some_and(|baseline| baseline != root_before);
        if behind {
            let cache = registry.domain_cache(&ws_root.0);
            if let Err(e) = DoorPlane::new(registry, ws_root, self, &cache).observe() {
                eprintln!("registry: run door observation: {e}");
            }
        }
    }

    /// The run plane's commit point ([`Self::follow`]): the executor commits
    /// through its own seam, not the memo's overlay, so the memo is observed
    /// after the commit — only when the baseline sits at the frame's
    /// `root_before`.
    pub(crate) fn follow_run(
        &self,
        registry: &crate::Registry,
        ws_root: &fs::WorkspaceRoot,
        frame: &DeltaFrame,
    ) {
        if self.state().watch.root() != Some(&frame.delta.root_before) {
            return;
        }
        let cache = registry.domain_cache(&ws_root.0);
        match registry.door_observation(&ws_root.0, &cache, crate::registry::DOOR_COOKIE_TIMEOUT) {
            Ok(observed) => self.follow(ws_root, &cache, frame, &Root(observed.0)),
            Err(e) => eprintln!("registry: run commit observation: {e}"),
        }
    }
}

/// One write door's hold on its workspace's delta plane: the ring it
/// numbers on and the memo it observes through, both used under the door's
/// flock. It is the door's [`SeqSink`](wire_serve::seq::SeqSink) and the
/// body of its observation closure, so the baseline law holds at the door's
/// two points ([`WorkspaceRing::settle`], [`WorkspaceRing::follow`]).
#[derive(Debug)]
pub(crate) struct DoorPlane<'a> {
    registry: &'a crate::Registry,
    ws_root: &'a fs::WorkspaceRoot,
    ring: &'a WorkspaceRing,
    cache: &'a Arc<Mutex<fs::DomainCache>>,
}

impl<'a> DoorPlane<'a> {
    pub(crate) fn new(
        registry: &'a crate::Registry,
        ws_root: &'a fs::WorkspaceRoot,
        ring: &'a WorkspaceRing,
        cache: &'a Arc<Mutex<fs::DomainCache>>,
    ) -> Self {
        DoorPlane {
            registry,
            ws_root,
            ring,
            cache,
        }
    }

    /// The door-entry observation (`Registry::door_observation`), with the
    /// baseline settled onto it before any byte lands.
    ///
    /// # Errors
    /// The observation's own refusal; settling never adds one.
    pub(crate) fn observe(&self) -> std::io::Result<model::MerkleRoot> {
        let observed = self.registry.door_observation(
            &self.ws_root.0,
            self.cache,
            crate::registry::DOOR_COOKIE_TIMEOUT,
        )?;
        self.ring
            .settle(self.ws_root, self.cache, &Root(observed.0.clone()));
        Ok(observed)
    }
}

impl wire_serve::seq::SeqSink for DoorPlane<'_> {
    fn allocate(&self, before: &Root, after: &Root, files: &[wire::DeltaFile]) -> u64 {
        wire_serve::seq::SeqSink::allocate(self.ring, before, after, files)
    }

    /// Recorded, then followed. The door's own-write overlay landed in this
    /// memo, so it already folds to the frame's `root_after` — no second
    /// observation.
    fn committed(&self, frame: &DeltaFrame) {
        self.ring.advance(frame.clone());
        self.ring
            .follow(self.ws_root, self.cache, frame, &frame.delta.root_after);
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
    /// workspace flock. Records only: the ring holds no memo to verify a
    /// baseline move against, so a door handed this sink directly (the run
    /// plane's births) leaves the baseline to the detector, which syncs
    /// silently once the frames chain from it. A door with the workspace memo
    /// takes a [`DoorPlane`].
    fn committed(&self, frame: &DeltaFrame) {
        self.advance(frame.clone());
    }
}
