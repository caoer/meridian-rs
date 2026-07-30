//! The root-history ring (v2 §7.3): sidecar-held process state, RAM-only,
//! bound 256 — one ring per daemon EPOCH.
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

use wire::{DeltaFrame, Root};

/// §7.3: history retention bound (i harvest) — a range outside it degrades
/// to re-derive (`root_unknown` → full resync), never to wrong data.
pub const RING_BOUND: usize = 256;

/// One epoch's delta history: contiguous [`DeltaFrame`]s in emission order,
/// oldest evicted past [`RING_BOUND`].
#[derive(Debug, Default)]
pub struct RootRing {
    entries: VecDeque<DeltaFrame>,
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
