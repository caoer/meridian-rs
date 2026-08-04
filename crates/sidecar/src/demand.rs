//! **The demand law** — the sidecar serve loop's own scheduling policy for the
//! shared reconcile (`wire_serve::watch::reconcile`).
//!
//! This is the DRIVER, deliberately not shared (U20b): it prices an input LINE
//! against the ring, which is a fact about the sidecar's request/response loop
//! and means nothing to the registry, whose subscribers send no lines at all.
//! The classification the reconcile performs IS shared; when to run it is not.

use wire::Op;

/// **The demand law.** Does this op READ the ring — the reconcile's only
/// product?
///
/// A reconcile produces exactly two things: ring frames (the Delta stream and
/// its `seq`) and the watcher's baseline. It makes NO root fresh — every arm
/// that prints a root folds its own at its own observation point
/// (`wire_serve::ambient_root`), which is why `toc`, `hello` and `read` print a
/// root here and still owe no reconcile. So the axis is the RING, not the
/// printed root: an op owes a reconcile exactly when it reads `seq` or the
/// retained batches. Ops that do not (`cat`, `extract`, `check_write`, …) cost
/// O(target) instead of two full corpus folds for a value they never read.
///
/// Freshness is unchanged because no observer's tense moves: every root a
/// client holds is still folded at the op that handed it over, and every ring
/// reader still reconciles immediately before reading the ring. The one tense
/// that does move is the epoch BASELINE — it is primed at the first ring
/// observation instead of the first line, so a `diff` anchored on a root the
/// client learned from a ring-blind op (`toc`, `hello`, `read`) before that
/// point answers `root_unknown` → resync. That is the §7.1 late law's existing
/// category and the ruled degrade direction: re-derive, never wrong data.
///
/// Exhaustive by construction — no wildcard arm. A new op does not compile
/// until someone classifies it, which is the point: a misclassification here is
/// a freshness regression no timing test would catch.
pub(crate) const fn observes_ring(op: &Op) -> bool {
    match op {
        // Ring readers: `epoch.seq()` / the retained batches ARE their answer.
        //
        // The write ops are here for a second reason — they read `epoch.seq()`
        // to number their own Delta and chain it onto the tip, so an
        // unreconciled external change must be emitted FIRST, or the two roots
        // do not meet and `diff` over the crossing range degrades to
        // `root_unknown` (the §7.3 posture, module header).
        Op::Root
        | Op::Diff { .. }
        | Op::Links { .. }
        | Op::Sub { .. }
        | Op::Splice { .. }
        | Op::Create { .. } => true,
        // Ring-blind: O(target) document reads that print no ring fact, or
        // print a root they fold themselves. `resolve` walks the corpus on its
        // own (the §4.5 walk plane — a different corpus from the §12 hash
        // domain, and not the ring). `view_path` refuses `daemon_only` before
        // touching anything.
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

/// Does this op MOVE the world? Only a write leaves the watcher's baseline
/// behind its own commit, and only the post-dispatch reconcile rebases it —
/// without that the next external delta chains from the PRE-commit root, which
/// is the contiguity break the module header records as a stated degrade.
/// Conservative by design: a `dry` splice moves nothing and still pays the
/// rebase, because dry-ness is a field, not an op.
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
