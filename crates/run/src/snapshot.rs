//! U6b — the detection bracket around the bash exec window (plan §4 U6b;
//! decisions #14/#19/#20/#25; review S3/S4).
//!
//! [`ExecBracket::open`] pins the pre-exec baseline: it takes the guarded
//! snapshot (`fs::guard::StepGuard` — symlink-refusing walk, config capture)
//! and cross-checks the observed root against the flock-computed
//! `root_after_phase1` (#19 addendum: the COMPUTED root is the authority — a
//! mismatch means the tree moved between the phase-1 commit and the bracket
//! opening, and the exec must not start).
//!
//! [`ExecBracket::close`] renders the window's [`Detection`] verdict after
//! the process group is dead (S3). The window has ZERO governed writes by
//! construction — the two-phase design puts phase 1 before the bracket and
//! phase 2 after it — so the residual-compare runs with an empty governed
//! set: ANY domain delta is out-of-band. `close` never errors: every failure
//! is a typed verdict, and phase 2 gates on [`Detection::Clean`] (fail
//! closed — no verdict is not a pass).
//!
//! # Wording (S4)
//! A delta renders as an "out-of-band change during exec window" — the
//! window is named, never the block. The wording's single source is
//! `fs::guard::ResidualDelta`'s `Display`; this module only delegates.
//!
//! # What detection can and cannot see (the named gaps)
//! Detection covers the §12 hash domain: md-only, dot-excluded, custom
//! ignores honored. Non-md / `.meridian/` / dot-path writes are UNDETECTED
//! (#20, explicit accepted gap, distinct from the out-of-tree honor system);
//! symlinks on non-dot paths REFUSE (#25); writes landing after the close
//! snapshot are outside the bracket (S3 residual escape window).

use fs::guard::{GuardError, ResidualDelta, StepGuard};
use model::MerkleRoot;

use crate::fence::GuaranteeClass;

/// The detection bracket around ONE exec window: opened after the phase-1
/// commit (against its flock-computed root), closed after the process group
/// is dead, consumed by the close (one window, one verdict).
#[derive(Debug)]
#[must_use = "an unclosed bracket detects nothing — close() renders the verdict"]
pub struct ExecBracket {
    guard: StepGuard,
}

/// Why the bracket refused to open — the exec never starts. Nothing ran; a
/// committed phase-1 pre-exec receipt stands and the orphan lint finds it.
#[derive(Debug)]
pub enum OpenRefusal {
    /// The observed pre-exec tree does not fold to the flock-computed
    /// `root_after_phase1`: an out-of-band change landed BETWEEN the phase-1
    /// commit and the bracket opening. Distinct from the in-window delta by
    /// construction — and no block ran, so there is nothing to accuse.
    PreExecMismatch {
        /// The computed authority (#19).
        expected: MerkleRoot,
        /// What the guarded snapshot observed on disk.
        observed: MerkleRoot,
    },
    /// The guarded snapshot refused (symlink, #25) or failed (I/O).
    Guard(GuardError),
}

impl std::fmt::Display for OpenRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenRefusal::PreExecMismatch { expected, observed } => write!(
                f,
                "out-of-band change before exec window: computed root_after_phase1 {} != observed pre-exec root {}",
                expected.0, observed.0,
            ),
            OpenRefusal::Guard(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for OpenRefusal {}

/// The exec-window detection verdict (#14) — ALWAYS rendered, orthogonal to
/// how the exec itself ended (a delta is named even when the step also timed
/// out or exited nonzero). Phase 2 gates on [`Detection::Clean`].
#[derive(Debug)]
pub enum Detection {
    /// The post-window tree is byte-identical to the pre-exec baseline; the
    /// verified root equals `root_after_phase1`.
    Clean {
        /// The verified post-window root.
        root: MerkleRoot,
    },
    /// An out-of-band change during the exec window, the exact delta named
    /// (S4 wording via the delta's `Display`). NEVER rolled back (ruling 2 —
    /// rollback would be a second write path with invented authority): the
    /// change persists as actor-absent external change; the run refuses.
    OutOfBand(ResidualDelta),
    /// `mdfs_config.yaml` changed during the window (#20, the
    /// config-widening attack) — the detection domain itself moved, so no
    /// residual verdict is possible.
    ConfigChanged,
    /// A symlinked non-dot path appeared (#25 laundering), named.
    Symlink {
        /// Workspace-relative forward-slash path of the refused link.
        path: String,
    },
    /// The close snapshot itself failed — no verdict can be rendered. Fail
    /// closed: not clean, phase 2 refuses.
    Failed {
        /// The underlying failure.
        reason: String,
    },
}

impl Detection {
    /// Is the window verified clean? Everything else — delta, moved config,
    /// symlink, or no verdict at all — gates phase 2 shut.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        matches!(self, Detection::Clean { .. })
    }
}

impl std::fmt::Display for Detection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Detection::Clean { root } => {
                write!(f, "exec window clean (root verified {})", root.0)
            }
            Detection::OutOfBand(delta) => delta.fmt(f),
            Detection::ConfigChanged => write!(
                f,
                "mdfs_config.yaml changed during exec window — config-widening refused",
            ),
            Detection::Symlink { path } => {
                write!(f, "symlinked path refused in exec-window snapshot: {path}")
            }
            Detection::Failed { reason } => {
                write!(
                    f,
                    "exec-window detection could not render a verdict: {reason}"
                )
            }
        }
    }
}

impl ExecBracket {
    /// The guarantee class this bracket earns a bash block (#23: the U7
    /// labeler must source `detected` from the bracket actually being wired,
    /// never from the language alone).
    pub const GUARANTEE_CLASS: GuaranteeClass = GuaranteeClass::Detected;

    /// Open the bracket against the flock-computed `root_after_phase1`.
    ///
    /// # Errors
    /// [`OpenRefusal::PreExecMismatch`] when the observed tree does not fold
    /// to the computed root; [`OpenRefusal::Guard`] when the guarded
    /// snapshot refuses (symlink) or fails (I/O). Either way the exec must
    /// not start.
    pub fn open(
        root: &fs::WorkspaceRoot,
        root_after_phase1: &MerkleRoot,
    ) -> Result<ExecBracket, OpenRefusal> {
        let guard = StepGuard::open(root).map_err(OpenRefusal::Guard)?;
        let observed = guard.pre_root();
        if observed != *root_after_phase1 {
            return Err(OpenRefusal::PreExecMismatch {
                expected: root_after_phase1.clone(),
                observed,
            });
        }
        Ok(ExecBracket { guard })
    }

    /// Close the bracket after the process group is dead: re-snapshot,
    /// config bracket first (#20), symlink refusal (#25), residual-compare
    /// with an empty governed set (#19 — any delta is out-of-band).
    #[must_use]
    pub fn close(self) -> Detection {
        match self.guard.close(&[]) {
            Ok(root) => Detection::Clean { root },
            Err(GuardError::OutOfBand(delta)) => Detection::OutOfBand(delta),
            Err(GuardError::ConfigChanged) => Detection::ConfigChanged,
            Err(GuardError::Symlink { path }) => Detection::Symlink { path },
            Err(GuardError::Io(e)) => Detection::Failed {
                reason: e.to_string(),
            },
        }
    }
}
