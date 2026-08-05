//! Demand law: when the serve loop runs shared reconcile
//! (`wire_serve::watch::reconcile`). Sidecar-only driver — prices an input
//! LINE against the ring; registry has no lines. Reconcile classification is
//! shared; schedule is not.

use wire::Op;

/// Does this op READ the ring (reconcile's product: frames/`seq` + baseline)?
///
/// Reconcile does not freshen printed roots — arms fold via
/// `wire_serve::ambient_root`. Axis is the RING: reconcile when the op reads
/// `seq` or retained batches. Ring-blind ops stay O(target).
///
/// Freshness holds: ring readers reconcile before read. Epoch baseline primes
/// at first ring observation (not first line), so a `diff` on a root learned
/// from a ring-blind op before that point may answer `root_unknown` → resync
/// (§7.1: re-derive, never wrong data).
///
/// Exhaustive — no wildcard. New ops must classify or fail to compile.
pub(crate) const fn observes_ring(op: &Op) -> bool {
    match op {
        // Ring readers (`seq`/batches). Writes also read `epoch.seq()` to
        // number their Delta — unreconciled external change must emit first
        // or `diff` over the crossing degrades to `root_unknown` (§7.3).
        Op::Root
        | Op::Diff { .. }
        | Op::Links { .. }
        | Op::Sub { .. }
        | Op::Splice { .. }
        | Op::Create { .. } => true,
        // Ring-blind: O(target) or self-folded root. `resolve` walks the §4.5
        // plane (not the ring). `view_path` refuses `daemon_only` first.
        Op::Hello { .. }
        | Op::Toc { .. }
        | Op::Cat { .. }
        | Op::Extract { .. }
        | Op::Read { .. }
        | Op::CheckWrite { .. }
        | Op::Resolve { .. }
        | Op::ViewPath { .. } => false,
    }
}

/// Does this op MOVE the world? Writes leave the baseline behind the commit;
/// post-dispatch reconcile rebases it, else the next external delta chains
/// from the pre-commit root (contiguity break). Conservative: `dry` still pays
/// (dry is a field, not an op).
pub(crate) const fn advances_ring(op: &Op) -> bool {
    match op {
        Op::Splice { .. } | Op::Create { .. } => true,
        Op::Hello { .. }
        | Op::Toc { .. }
        | Op::Cat { .. }
        | Op::Extract { .. }
        | Op::Read { .. }
        | Op::CheckWrite { .. }
        | Op::Resolve { .. }
        | Op::ViewPath { .. }
        | Op::Root
        | Op::Diff { .. }
        | Op::Links { .. }
        | Op::Sub { .. } => false,
    }
}
