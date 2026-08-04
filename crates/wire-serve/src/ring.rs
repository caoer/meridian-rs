//! The root-history ring (v2 §7.3): host-held process state, RAM-only,
//! bound 256 — one ring per EPOCH.
//!
//! **Shared by both hosts (U20b).** The sidecar holds one ring per serve
//! lifetime; the registry holds one per WORKSPACE, born at first use and dropped
//! on the `read_mints` horizon (`Registry::reap` — "session memory does not
//! outlive the registration"). The two hosts differ in what an epoch IS and in
//! what drives detection; the retention law below is identical for both, which
//! is why it lives here and not in either host.
//!
//! **Ratified as TRANSPORT, not memory (advisor, U20a Q2, 2026-08-03):** this is
//! a bounded in-flight buffer for a live channel. It reconstructs no history, it
//! answers a gap with `root_unknown` → resync, and nothing in it survives a
//! restart — so ZT's decision 19 ("the engine does not have memory … history is
//! pin to git when we lock") is not engaged. **Binding constraint on future
//! work:** any replay-what-you-missed BEYOND this in-flight window is a memory
//! claim and goes to ZT, never to an implementer.
//!
//! The §7.1 late law, encoded structurally: `seq` is per-daemon-epoch — the
//! ring lives inside one `serve` lifetime and no counter survives on disk
//! (fs no-storage law), so a restart IS a new epoch: fresh ring, seq reset,
//! and every stale root query answers `root_unknown` → resync. Cross-epoch
//! catchup is diff-by-root; the root is the only restart-durable handle.
//! There is deliberately NO persisted epoch fact anywhere.
//!
//! Sized for reuse: D4-DELTAS-LIVE advances the ring on every batch commit;
//! T5-SUB reads the same entries for the push path.

use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::atomic::{AtomicU64, Ordering};

use wire::{DeltaFrame, Root};

use crate::rev::Position;

/// §7.3: history retention bound (i harvest) — a range outside it degrades
/// to re-derive (`root_unknown` → full resync), never to wrong data.
pub const RING_BOUND: usize = 256;

/// One epoch's delta history: contiguous [`DeltaFrame`]s in emission order,
/// oldest evicted past [`RING_BOUND`].
///
/// # THE SEQ INVARIANT (advisor-ruled 2026-08-04, U20b completion)
///
/// **A `seq` returned by an allocation can never be re-issued by [`Self::advance`].**
///
/// That sentence is the contract. A reservation high-water mark is its
/// MECHANISM and must never be mistaken for its statement — the invariant is
/// what a reader has to keep true, the counter is merely how.
///
/// ## Why it is not already true
/// The registry allocates a `seq` INSIDE the workspace write flock (inside
/// `write::splice`) but ADVANCES this ring AFTER that call returns, with the
/// flock already dropped. In that window the external-change detector can run a
/// cycle, find [`Self::tip_root`] still at the OLD root, classify the change as
/// EXTERNAL, and emit a frame carrying the very `seq` the write path just
/// allocated. Two frames, one number — which breaks the property the design's
/// own gate ruled: ONE LOCK, ONE ALLOCATOR, ONE CHAIN.
///
/// Closing it RESTORES the ruled design; it does not extend it.
///
/// ## Binding conditions on whoever implements it
/// - **The guarantee lives on this type**, stated as the invariant above.
/// - **Interior mutability on the RESERVATION ONLY, never on the ring.** An
///   allocation stays `&self` — an atomic max-bump, or a small mutex around the
///   reservation counter inside this type. [`Self::advance`] keeps its current
///   shape. Then the sidecar's single-threaded sink compiles unchanged and the
///   registry's `Arc<WorkspaceRing>` discipline (a slow read never holds the
///   map lock) is undisturbed. **Do not thread `&mut` through both hosts for a
///   twenty-line counter.** If the reservation turns out NOT to be isolable
///   that way, that is a NEW FACT and it goes back to the leader — it is not
///   licence to widen.
/// - **Land it as a TYPE change, not a convention.** The argument is measured:
///   when U20b changed the `seq` SIGNATURE, two unconverted call sites surfaced
///   as COMPILE ERRORS. A convention would have surfaced them as production
///   gaps instead. Make the mistake unrepresentable and the compiler becomes
///   the census.
///
/// ## The proof it owes
/// Mutation-proof the EXACT RACE SHAPE — allocate, then detector-advance BEFORE
/// writer-advance — and assert no duplicate `seq` and no gap. **Run it with real
/// frames on BOTH sides:** a race test over an empty ring is the
/// fixture-with-one-row defect wearing a concurrency costume, and it would pass
/// while proving nothing about the seam.
///
/// ## As landed
/// [`Self::allocate_seq`] is the ONE issuing point for both producers, and
/// `reserved` below is the high-water mark that makes re-issue impossible. The
/// proof is `allocation_survives_a_detector_advance_before_the_writers`.
#[derive(Debug, Default)]
pub struct RootRing {
    entries: VecDeque<DeltaFrame>,
    /// High-water mark of every seq ever ISSUED this epoch — the OUTSTANDING
    /// ones specifically. An allocation lands here the instant it is handed out,
    /// while its frame reaches `entries` only after the writer's flock drops,
    /// and that gap is the whole race.
    ///
    /// It does NOT track recorded frames, and deliberately: [`Self::seq`] reads
    /// the tip, so a number already in `entries` is covered by the other term of
    /// [`Self::allocate_seq`]'s max. An earlier draft also bumped this mark
    /// inside [`Self::advance`] to cover them twice; the mutation harness could
    /// not construct a case where removing that bump changed any allocation,
    /// which is the definition of a line no test can defend.
    ///
    /// Atomic, and deliberately the ONLY interior mutability in this type: an
    /// allocation must be callable through a `&self` sink from either host,
    /// while [`Self::advance`] keeps its `&mut` shape. Widening this to a lock
    /// around the whole ring would put the registry's `Arc<WorkspaceRing>` reads
    /// behind a writer's critical section for the sake of a counter.
    reserved: AtomicU64,
}

impl RootRing {
    /// A fresh epoch: empty history, seq at 0.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The last seq assigned this epoch (0 = no batch yet). Monotone within
    /// the epoch, never compared across epochs (§7.1 late law).
    #[must_use]
    pub fn seq(&self) -> u64 {
        self.entries.back().map_or(0, |f| f.delta.seq)
    }

    /// **Issue the next `seq` — the only place either producer may get one.**
    ///
    /// Returns `max(tip, reserved) + 1` and records it, so a number handed out
    /// here can never be handed out again and can never be reached by a later
    /// [`Self::advance`]. Both terms are load-bearing: `reserved` covers an
    /// allocation whose frame has not landed yet, and `tip` covers a frame that
    /// landed without one (the pre-U20b in-process paths, which pass no sink and
    /// write `seq` 0, plus any host that advances a frame it did not allocate).
    ///
    /// `&self` by ruling — the sidecar's sink borrows the live ring and the
    /// registry's holds it behind an `Arc`; neither can produce a `&mut` at the
    /// instant the frame is assembled, which is precisely the instant the number
    /// has to be decided.
    pub fn allocate_seq(&self) -> u64 {
        let tip = self.seq();
        let mut seen = self.reserved.load(Ordering::Relaxed);
        loop {
            let next = seen.max(tip) + 1;
            // Compare-exchange rather than fetch_add: the bump is to
            // `max(tip, reserved) + 1`, not `reserved + 1`, so a competing
            // allocation invalidates the arithmetic and not just the value.
            match self.reserved.compare_exchange_weak(
                seen,
                next,
                Ordering::SeqCst,
                Ordering::Relaxed,
            ) {
                Ok(_) => return next,
                Err(actual) => seen = actual,
            }
        }
    }

    /// The oldest retained frame's seq (`None`: empty ring) — the push
    /// path's catchup boundary (T5-SUB).
    #[must_use]
    pub fn oldest_seq(&self) -> Option<u64> {
        self.entries.front().map(|f| f.delta.seq)
    }

    /// The tip's `root_after` (`None`: no batch this epoch) — the watcher's
    /// internal-commit sync check (F5-WATCH reconcile disposition).
    #[must_use]
    pub fn tip_root(&self) -> Option<&Root> {
        self.entries.back().map(|f| &f.delta.root_after)
    }

    /// Frames with `delta.seq > after`, emission order — the push path's
    /// read (§7.3: the SAME stored objects `diff` replays; no second
    /// serialization site).
    #[must_use]
    pub fn frames_after(&self, after: u64) -> Vec<DeltaFrame> {
        self.entries
            .iter()
            .filter(|f| f.delta.seq > after)
            .cloned()
            .collect()
    }

    /// **The subscribe anchor law (§7.1 late law).** Can a subscription start at
    /// `from_seq` — i.e. can this ring deliver a contiguous stream from there?
    ///
    /// Two anchorable positions, and only two: `current` (live-only, nothing
    /// replayed) and any position the retained window can still replay from.
    /// Everything else — evicted, ahead of the tip, or a PREVIOUS EPOCH's
    /// counter — is unanchorable, and the caller answers `root_unknown` →
    /// resync by diff-by-root, the only restart-durable handle. Never a
    /// cross-epoch seq comparison: the old counter died with its ring.
    ///
    /// Shared by both hosts (U20b) because it is the retention law's own
    /// predicate — a host that spelled it differently would be a second
    /// retention law wearing the first one's vocabulary.
    #[must_use]
    pub fn can_anchor(&self, from_seq: u64) -> bool {
        let current = self.seq();
        // `oldest - 1` is the seq a client holds when it has consumed nothing
        // from the retained window: the position the oldest frame advances FROM.
        // Saturating because a ring whose oldest frame is seq 0 has no earlier
        // position to name (frames are numbered from 1).
        from_seq == current
            || (from_seq < current
                && self
                    .oldest_seq()
                    .is_some_and(|oldest| from_seq >= oldest.saturating_sub(1)))
    }

    /// Record one emitted batch (D4 write path / F5 external detection).
    /// Contiguity is the caller's: `frame.delta.root_before` extends the
    /// chain, `frame.delta.seq` is this epoch's next seq.
    pub fn advance(&mut self, frame: DeltaFrame) {
        if self.entries.len() == RING_BOUND {
            self.entries.pop_front(); // oldest falls outside retained history
        }
        self.entries.push_back(frame);
    }

    /// §7.3 replay: the byte-identical frames emitted between `from` and
    /// `to`, or `None` when the range is outside the retained history
    /// (evicted, unknown, previous-epoch, or backwards — every `None` is the
    /// caller's `root_unknown` → resync; degrade to re-derive, never to
    /// wrong data). `current` anchors the chain when no batch has been
    /// emitted yet (fresh epoch: history is exactly the current root).
    #[must_use]
    pub fn batches_between(
        &self,
        from: &Root,
        to: &Root,
        current: &Root,
    ) -> Option<Vec<DeltaFrame>> {
        // The root chain this epoch retains: entry 0's root_before, then
        // every root_after in order; a fresh epoch retains only `current`.
        let chain: Vec<&Root> = match self.entries.front() {
            None => vec![current],
            Some(first) => std::iter::once(&first.delta.root_before)
                .chain(self.entries.iter().map(|f| &f.delta.root_after))
                .collect(),
        };
        let i = chain.iter().position(|r| *r == from)?;
        let j = chain.iter().position(|r| *r == to)?;
        if i > j {
            return None; // backwards range: nothing was emitted from→to
        }
        Some(self.entries.iter().skip(i).take(j - i).cloned().collect())
    }
}

/// Write one Notification frame — **the ONE push serializer, both hosts.**
///
/// The frame is the STORED ring object serialized directly (`{"delta":{…}}`, no
/// `id` key — §3.1 classification), so the d4 single-constructor law extends to
/// the push path by construction: there is no second place a Delta becomes
/// bytes. Under a v3 session each frame is re-shaped `root_before/after` →
/// `fingerprint_*` before write.
///
/// # The v2 demotion, and why it is TYPED rather than a `Value` pass
/// A v2 session must never receive a field that postdates its contract, and
/// `#[serde(skip_serializing_if)]` does not provide that: it skips on an empty
/// VALUE, never on a v2 SESSION. So the v2 branch strips every key
/// [`crate::rev::is_reserved`] names at [`Position::NotificationRoot`] — today
/// exactly `effects`, the C3 reaction sibling.
///
/// It strips on the TYPED value, and that is load-bearing. `serde_json::Map` is
/// a `BTreeMap` here (`preserve_order` is not enabled), so routing a frame
/// through `Value` ALPHABETIZES its keys — the v2 wire prints struct order
/// (`seq`, `root_before`, `root_after`, `files`), and alphabetizing it would be
/// a byte-level break of the frozen contract committed while trying to protect
/// it. Worse, a key-SET pin compares sets and cannot see order, so it would ship
/// green. `u20b_v2_notification_shape.rs` pins the ORDER for exactly this
/// reason; a future `ResponseRoot` row owes the same typed treatment.
///
/// # Errors
/// I/O failure on the output stream — never a content condition.
pub fn write_frame(output: &mut impl Write, frame: &DeltaFrame, v3: bool) -> io::Result<()> {
    if v3 {
        let mut v = serde_json::to_value(frame)?;
        crate::rev::project_delta_frame(&mut v);
        serde_json::to_writer(&mut *output, &v)?;
    } else if crate::rev::is_reserved("effects", Position::NotificationRoot)
        && !frame.effects.is_empty()
    {
        // Demoted COPY: the ring keeps the full frame, because a v3 subscriber
        // on the same workspace is entitled to the reaction plane.
        let demoted = DeltaFrame {
            delta: frame.delta.clone(),
            effects: Vec::new(),
        };
        serde_json::to_writer(&mut *output, &demoted)?;
    } else {
        serde_json::to_writer(&mut *output, frame)?;
    }
    output.write_all(b"\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(n: u64) -> Root {
        Root(format!("b3:{n:064x}"))
    }

    /// A synthetic contiguous frame `root(n-1) → root(n)`, seq n.
    fn frame(n: u64) -> DeltaFrame {
        DeltaFrame {
            delta: wire::Delta {
                seq: n,
                root_before: root(n - 1),
                root_after: root(n),
                actor: None,
                now: None,
                files: vec![],
            },
            effects: vec![],
        }
    }

    /// Fresh epoch: history is exactly the current root — same-root diff is
    /// truthfully empty, anything else is outside history.
    #[test]
    fn fresh_epoch_retains_only_current() {
        let ring = RootRing::new();
        assert_eq!(ring.seq(), 0);
        assert_eq!(
            ring.batches_between(&root(0), &root(0), &root(0)),
            Some(vec![])
        );
        assert_eq!(ring.batches_between(&root(1), &root(1), &root(0)), None);
        assert_eq!(ring.batches_between(&root(0), &root(1), &root(0)), None);
    }

    /// §7.3 replay over retained history: the exact frames between two
    /// roots, in emission order; full-range and sub-range.
    #[test]
    fn replay_returns_the_frames_between_roots() {
        let mut ring = RootRing::new();
        for n in 1..=4 {
            ring.advance(frame(n));
        }
        assert_eq!(ring.seq(), 4);
        let full = ring.batches_between(&root(0), &root(4), &root(4)).unwrap();
        assert_eq!(full.len(), 4);
        assert_eq!(full[0], frame(1));
        assert_eq!(full[3], frame(4));
        let sub = ring.batches_between(&root(1), &root(3), &root(4)).unwrap();
        assert_eq!(sub, vec![frame(2), frame(3)]);
        assert_eq!(
            ring.batches_between(&root(2), &root(2), &root(4)),
            Some(vec![])
        );
    }

    /// Gate 2 (overflow): past the 256 bound the oldest roots leave the
    /// retained history — a range touching them answers `None`
    /// (`root_unknown`), while the surviving window still replays.
    #[test]
    fn ring_overflow_evicts_oldest_roots() {
        let mut ring = RootRing::new();
        for n in 1..=(RING_BOUND as u64 + 10) {
            ring.advance(frame(n));
        }
        assert_eq!(ring.seq(), RING_BOUND as u64 + 10);
        // evicted: roots 0..=9 are gone (entry 1..=10 dropped)
        assert_eq!(
            ring.batches_between(&root(0), &root(20), &root(266)),
            None,
            "evicted origin is outside retained history"
        );
        // the surviving window replays exactly
        let window = ring
            .batches_between(&root(10), &root(266), &root(266))
            .unwrap();
        assert_eq!(window.len(), RING_BOUND);
        assert_eq!(window[0], frame(11));
    }

    /// Gate 6 (per-daemon-epoch, §7.1 late law): a restart is a NEW ring —
    /// seq resets to 0 and every previous-epoch root resolves outside
    /// history; no cross-epoch seq comparison is representable because the
    /// old counter is gone with the old ring.
    #[test]
    fn restart_opens_a_new_epoch() {
        let mut epoch1 = RootRing::new();
        for n in 1..=5 {
            epoch1.advance(frame(n));
        }
        assert_eq!(epoch1.seq(), 5);
        drop(epoch1); // the restart: memory is disposable, nothing persists

        let epoch2 = RootRing::new();
        assert_eq!(epoch2.seq(), 0, "seq never survives an epoch");
        // stale roots from the previous epoch: outside retained history
        assert_eq!(
            epoch2.batches_between(&root(1), &root(5), &root(5)),
            None,
            "previous-epoch range → root_unknown → resync"
        );
        // the current root alone is retained
        assert_eq!(
            epoch2.batches_between(&root(5), &root(5), &root(5)),
            Some(vec![])
        );
    }

    /// **THE SEQ INVARIANT, at the exact race shape it exists for.**
    ///
    /// The order is the point, and it is the order that used to break: the
    /// writer ALLOCATES under its flock, the detector then allocates AND
    /// advances a full cycle inside the window, and only afterwards does the
    /// writer's own frame land. Under `seq() + 1` the detector read a tip that
    /// could not see the writer's outstanding allocation and re-issued it.
    ///
    /// Both sides carry REAL frames: the ring holds four before the race, and
    /// each producer advances a frame of its own. On an empty ring the two
    /// allocations would differ for the trivial reason that any counter
    /// increments, and the test would pass over a seam it never touched.
    #[test]
    fn allocation_survives_a_detector_advance_before_the_writers() {
        let mut ring = RootRing::new();
        for n in 1..=4 {
            ring.advance(frame(n));
        }

        // The writer, under the workspace flock: number decided, frame not yet
        // recorded. `seq()` still reads 4 for everyone else — that is the window.
        let writer_seq = ring.allocate_seq();
        assert_eq!(ring.seq(), 4, "the allocation is outstanding, not recorded");

        // The detector, inside that window: a full cycle of its own.
        let detector_seq = ring.allocate_seq();
        ring.advance(DeltaFrame {
            delta: wire::Delta {
                seq: detector_seq,
                root_before: root(4),
                root_after: root(5),
                actor: None,
                now: None,
                files: vec![],
            },
            effects: vec![],
        });

        // The writer's flock drops and its frame lands last.
        ring.advance(DeltaFrame {
            delta: wire::Delta {
                seq: writer_seq,
                root_before: root(5),
                root_after: root(6),
                actor: None,
                now: None,
                files: vec![],
            },
            effects: vec![],
        });

        assert_ne!(
            writer_seq, detector_seq,
            "a seq returned by allocate must never be re-issued"
        );

        let seqs: Vec<u64> = ring.frames_after(0).iter().map(|f| f.delta.seq).collect();
        let mut sorted = seqs.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), seqs.len(), "no duplicate seq");
        assert_eq!(
            sorted,
            (1..=6).collect::<Vec<u64>>(),
            "no gap: the two producers between them number 1..=6 exactly once"
        );
    }

    /// Backwards ranges are outside emission history (nothing was emitted
    /// from a later root to an earlier one) — `None`, resync-safe.
    #[test]
    fn backwards_range_is_unknown() {
        let mut ring = RootRing::new();
        for n in 1..=3 {
            ring.advance(frame(n));
        }
        assert_eq!(ring.batches_between(&root(3), &root(1), &root(3)), None);
    }
}
