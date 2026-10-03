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
    /// Test-only pause gate for priming's unlocked window: when armed, the
    /// next priming cycle announces itself on the first channel after its
    /// unlocked byte snapshot, then parks on the second — before it takes
    /// the flock, the window in which a batch can land under the snapshot.
    /// One-shot. `cfg(test)` excludes it from every release build by
    /// construction (the `Registry::pause_before_insert` seam class).
    #[cfg(test)]
    pause_after_prime_snapshot:
        Mutex<Option<(std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>)>>,
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
            #[cfg(test)]
            pause_after_prime_snapshot: Mutex::new(None),
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
    /// Every caller holds the workspace flock: [`DoorPlane`]'s `committed` —
    /// a wire door under its own, the run plane's sink (`delta_sink`) under
    /// the executor's — and this ring's own `SeqSink::committed` (the run
    /// plane's births, under the create door's). [`RootRing::allocate_seq`]'s
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
    ///    An ordinary cycle observes vouched-only under the flock and skips on
    ///    any miss; the [`DETECT_FLOOR_CADENCE`] backstop cycle floors there,
    ///    so a miss delays a frame by at most that cadence.
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
    /// Unlocked first: the pre-observation through the SHARED memo
    /// (`Registry::door_observation`), compared with the baseline, and — when
    /// priming — the byte snapshot ([`wire_serve::watch::PrimeSnapshot`]).
    /// Then [`Self::cycle_locked`] under the workspace write flock: reconcile
    /// must never observe a mid-batch state. `force_floor` is the §6.6
    /// fallback clock: once per [`DETECT_FLOOR_CADENCE`] the backstop
    /// ([`crate::Registry::backstop`]) compares disk to the memo unlocked and
    /// hints what the stream lost, and the flock-held observation may floor,
    /// so neither a silent capture loss nor a feed that cannot vouch delays a
    /// frame past that cadence.
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
            if !was_unprimed && state.watch.root() == Some(&Root(pre.0.clone())) {
                let now = Instant::now();
                state.last_detect = Some(now);
                if force_floor {
                    // A true floor pass found nothing — anchor the §6.6
                    // fallback clock.
                    state.last_floor = Some(now);
                }
                return Ok(false);
            }
        }
        // Priming reads the corpus HERE, before the flock: its bytes seed the
        // baseline, and its root is written nowhere (merkle-spec §6.7).
        let snapshot = if was_unprimed {
            Some(wire_serve::watch::PrimeSnapshot::take(ws_root)?)
        } else {
            None
        };
        #[cfg(test)]
        self.park_after_prime_snapshot(snapshot.is_some());
        self.cycle_locked(ws_root, registry, &cache, force_floor, snapshot)
    }

    /// The flock-held half of [`Self::cycle`]: the root moved (or the watch
    /// is priming), so take the write flock and RE-observe under it — the
    /// flock-held observation is the one the frame folds.
    ///
    /// An ordinary cycle observes vouched-only
    /// (`Registry::door_observation_vouched`, in one [`DETECT_CADENCE`]): on
    /// any miss it releases the flock and tells nothing, a skip that costs
    /// latency only. The backstop and priming cycles make the full §6.1
    /// door-grade observation, floor included, and anchor the fallback clock
    /// only once it ran — a backstop that lost the flock to a writer re-runs
    /// on the next cadence instead of waiting out another
    /// [`DETECT_FLOOR_CADENCE`].
    ///
    /// Priming builds the baseline from the memo's leaf view, reusing each
    /// `snapshot` member whose digest matches its leaf
    /// (`wire_serve::watch::prime`); a re-read that races its leaf falls back
    /// to the full snapshot under the flock (`reconcile`, through
    /// [`Self::reconcile_observed`] on the unprimed watch).
    fn cycle_locked(
        &self,
        ws_root: &fs::WorkspaceRoot,
        registry: &crate::Registry,
        cache: &Arc<Mutex<fs::DomainCache>>,
        force_floor: bool,
        snapshot: Option<wire_serve::watch::PrimeSnapshot>,
    ) -> Result<bool, Box<ErrorBody>> {
        let full = force_floor || snapshot.is_some();
        let holder = if snapshot.is_some() {
            "flock.hold.prime"
        } else if force_floor {
            "flock.hold.backstop"
        } else {
            "flock.hold.detect"
        };
        let Ok(_write_lock) = fs::WriteLock::acquire(ws_root).map(|lock| lock.timed(holder)) else {
            // A write is in flight; its change keeps. A full cycle that lost
            // the flock leaves the fallback clock unanchored and retries one
            // cadence later, not on every tick: its unlocked passes are the
            // expensive ones.
            if full {
                self.state().last_detect = Some(Instant::now());
            }
            return Ok(false);
        };
        let observed = if full {
            registry
                .door_observation(&ws_root.0, cache, crate::registry::DOOR_COOKIE_TIMEOUT)
                .map_err(|e| io_error_body(&e))?
        } else if let Some(root) =
            registry.door_observation_vouched(&ws_root.0, cache, DETECT_CADENCE)
        {
            root
        } else {
            self.state().last_detect = Some(Instant::now());
            return Ok(false);
        };
        let observed = Root(observed.0);
        let emitted = match snapshot {
            Some(snapshot) => {
                let leaves = leaf_view(cache);
                let primed = {
                    let mut state = self.state();
                    wire_serve::watch::prime(
                        ws_root,
                        &mut state.watch,
                        snapshot,
                        &leaves,
                        &observed,
                    )?
                };
                !primed && self.reconcile_observed(ws_root, cache, &observed)?
            }
            None => self.reconcile_observed(ws_root, cache, &observed)?,
        };
        let now = Instant::now();
        let mut state = self.state();
        state.last_detect = Some(now);
        if full {
            state.last_floor = Some(now);
        }
        Ok(emitted)
    }

    /// Test seam: park between priming's unlocked snapshot and the flock
    /// (the `pause_after_prime_snapshot` gate).
    #[cfg(test)]
    fn park_after_prime_snapshot(&self, priming: bool) {
        if !priming {
            return;
        }
        let gate = self
            .pause_after_prime_snapshot
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some((announce, resume)) = gate {
            let _ = announce.send(());
            let _ = resume.recv();
        }
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
        let leaves = leaf_view(cache);
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
}

/// One write door's hold on its workspace's delta plane: the ring it
/// numbers on and the memo it observes through, both used under the door's
/// flock. It is the door's [`SeqSink`](wire_serve::seq::SeqSink) and the
/// body of its observation closure, so the baseline law holds at the door's
/// two points ([`WorkspaceRing::settle`], [`WorkspaceRing::follow`]). The
/// wire doors and the run plane's delta-mint bracket (`delta_sink`) both
/// hold one.
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

/// The memo's leaf view, String-spelled: non-UTF-8 NAMES stay
/// baseline-invisible exactly as the snapshot kept them (their leaves still
/// fold into the root; a `wire::Path` cannot spell them).
fn leaf_view(cache: &Arc<Mutex<fs::DomainCache>>) -> std::collections::BTreeMap<String, [u8; 32]> {
    let memo = cache.lock().unwrap_or_else(PoisonError::into_inner);
    memo.leaf_digests()
        .into_iter()
        .filter_map(|(rel, digest)| rel.to_str().map(|s| (s.to_owned(), digest)))
        .collect()
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
    /// plane's births) leaves the baseline one frame behind the tip until the
    /// next detect cycle. That cycle syncs silently when the frames chain
    /// from the baseline; an external edit inside it is told from the stale
    /// baseline, with the born file re-named. A door with the workspace memo
    /// takes a [`DoorPlane`].
    fn committed(&self, frame: &DeltaFrame) {
        self.advance(frame.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::mpsc;

    /// An in-process registry rooted under `tmp` (no socket, no threads).
    fn registry_in(tmp: &Path) -> crate::Registry {
        let dir = tmp.join("registry");
        std::fs::create_dir_all(&dir).unwrap();
        let mut config = crate::Config::for_cache_root(tmp.join("cache"));
        config.socket_path = dir.join("daemon.sock");
        config.state_path = dir.join("state.json");
        crate::in_process_registry(&config).expect("in-process registry")
    }

    /// A workspace `tmp/ws` seeded with `files`, spelled canonically — the
    /// registry keys its feeds and memos by the canonical path.
    fn workspace(tmp: &Path, files: &[(&str, &str)]) -> fs::WorkspaceRoot {
        let ws = tmp.join("ws");
        for (rel, body) in files {
            write(&ws, rel, body);
        }
        fs::WorkspaceRoot(std::fs::canonicalize(&ws).unwrap())
    }

    fn write(ws: &Path, rel: &str, body: &str) {
        let path = ws.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    /// The disk truth: a full fold of the workspace as it stands.
    fn disk_root(ws: &fs::WorkspaceRoot) -> Root {
        Root(fs::domain_snapshot(ws).unwrap().1.0)
    }

    /// The `(path, change)` rows a frame tells, path-sorted.
    fn told(frame: &DeltaFrame) -> Vec<(String, wire::FileChange)> {
        let mut rows: Vec<_> = frame
            .delta
            .files
            .iter()
            .map(|f| (f.path.0.clone(), f.change))
            .collect();
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        rows
    }

    /// P1(b): no ordinary cycle runs the floor under the flock. With the vouch
    /// impossible — every cookie barrier unproven (a directory at the
    /// sentinel's path fails each sentinel write), or the feed slot sticky
    /// `Failed` — the ordinary cycle releases the flock and tells nothing; the
    /// `DETECT_FLOOR_CADENCE` backstop cycle floors and tells the edit, so the
    /// miss costs latency, never the frame.
    ///
    /// *Mutation:* observe through `door_observation` on the ordinary arm —
    /// the ordinary cycle floors under the flock and tells the edit itself.
    #[test]
    fn an_unvouched_ordinary_cycle_skips_and_the_backstop_tells_the_edit() {
        for miss in ["cookie_unproven", "no_feed"] {
            let tmp = tempfile::tempdir().unwrap();
            let ws = workspace(
                tmp.path(),
                &[
                    ("plan.md", "# Plan\n\nship by August\n"),
                    ("notes.md", "# Notes\n"),
                ],
            );
            let reg = registry_in(tmp.path());
            if miss == "cookie_unproven" {
                std::fs::create_dir_all(ws.0.join(crate::feed::COOKIE_REL)).unwrap();
            } else {
                reg.fail_feed(&ws.0);
            }
            let ring = WorkspaceRing::new(&ws);
            let (root, seq) = ring.prime(&ws, &reg).expect("priming floors");
            assert_eq!(root, disk_root(&ws), "{miss}: priming observes the disk");
            write(&ws.0, "plan.md", "# Plan\n\nship by September\n");

            assert!(
                !ring.cycle(&ws, &reg, false).expect("ordinary cycle"),
                "{miss}: an ordinary cycle that cannot vouch tells nothing"
            );
            assert_eq!(ring.seq(), seq, "{miss}: the skip numbered nothing");
            assert_eq!(
                ring.state().watch.root(),
                Some(&root),
                "{miss}: the skip left the baseline"
            );
            drop(fs::WriteLock::acquire(&ws).expect("the skip released the flock"));

            assert!(
                ring.cycle(&ws, &reg, true).expect("backstop cycle"),
                "{miss}: the backstop tells the edit"
            );
            let frames = ring.frames_after(seq);
            assert_eq!(frames.len(), 1, "{miss}: one frame");
            assert_eq!(frames[0].delta.root_before, root, "{miss}");
            assert_eq!(frames[0].delta.root_after, disk_root(&ws), "{miss}");
            assert_eq!(
                told(&frames[0]),
                vec![("plan.md".to_owned(), wire::FileChange::Modified)],
                "{miss}"
            );
        }
    }

    /// P1(c): priming reads the corpus before the flock, and that unlocked
    /// snapshot's root is written nowhere. The snapshot here sees a batch
    /// mid-landing — `a.md` rewritten, `b.md` not yet — and inside priming's
    /// unlocked window the batch finishes, `d.md` is deleted and `e.md` added.
    /// The baseline is the flock-held observation: the ack root is the disk
    /// after all of it, never the torn root, and the ring numbers nothing.
    /// Every baseline member carries the observed bytes, so restoring `b.md`,
    /// re-creating `d.md` and removing `e.md` are all told: a baseline that
    /// kept the snapshot's stale `b.md` or `d.md` would miss them, and one
    /// that never read `e.md` would not hold it.
    ///
    /// *Mutation:* adopt a snapshot member without checking its digest
    /// against the memo leaf — the restored `b.md` equals the stale baseline
    /// and is never told.
    #[test]
    fn the_unlocked_prime_snapshot_reuses_verified_bytes_and_writes_its_root_nowhere() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = workspace(
            tmp.path(),
            &[
                ("a.md", "# A\n\none\n"),
                ("b.md", "# B\n\none\n"),
                ("d.md", "# D\n"),
            ],
        );
        let reg = registry_in(tmp.path());
        let ring = WorkspaceRing::new(&ws);
        // The batch's first rename has landed; its second has not.
        write(&ws.0, "a.md", "# A\n\ntwo\n");
        let (announce, announced) = mpsc::channel();
        let (resume, resumed) = mpsc::channel();
        *ring.pause_after_prime_snapshot.lock().unwrap() = Some((announce, resumed));

        let ((root, seq), torn) = std::thread::scope(|scope| {
            let priming = scope.spawn(|| ring.prime(&ws, &reg));
            announced
                .recv_timeout(Duration::from_secs(10))
                .expect("priming never paused between an unlocked snapshot and the flock");
            let torn = disk_root(&ws);
            write(&ws.0, "b.md", "# B\n\ntwo\n");
            std::fs::remove_file(ws.0.join("d.md")).unwrap();
            write(&ws.0, "e.md", "# E\n");
            resume.send(()).unwrap();
            (priming.join().unwrap().expect("prime"), torn)
        });
        assert_eq!(root, disk_root(&ws), "the ack root is the observed disk");
        assert_ne!(
            root, torn,
            "the torn snapshot's root never became the baseline"
        );
        assert_eq!(seq, 0, "priming numbers nothing");
        assert!(
            ring.frames_after(0).is_empty(),
            "the ring carries no root no door committed"
        );

        write(&ws.0, "b.md", "# B\n\none\n");
        write(&ws.0, "d.md", "# D\n");
        std::fs::remove_file(ws.0.join("e.md")).unwrap();
        assert!(ring.cycle(&ws, &reg, true).expect("backstop cycle"));
        let frames = ring.frames_after(0);
        assert_eq!(frames.len(), 1, "one frame tells all three");
        assert_eq!(frames[0].delta.root_before, root);
        assert_eq!(frames[0].delta.root_after, disk_root(&ws));
        assert_eq!(
            told(&frames[0]),
            vec![
                ("b.md".to_owned(), wire::FileChange::Modified),
                ("d.md".to_owned(), wire::FileChange::Created),
                ("e.md".to_owned(), wire::FileChange::Deleted),
            ]
        );
    }
}
