//! U6b — detection bracket around the bash exec window (plan §4 U6b;
//! decisions #19/#20/#25). Opens against the computed post-phase1 root;
//! closes after group kill. Residual compare, config bracket, and symlink
//! refusal produce a [`Detection`] verdict. Observation only — never rolls
//! back; phase 2 gates on [`Detection::is_clean`].

use fs::guard::{ConfigState, GuardError, ResidualDelta, StepGuard};
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

    /// Open the bracket AND pin it to the run-initial config state. #20 is
    /// "mid-RUN", not just mid-step: a config change landing BETWEEN two
    /// clean windows must refuse step N even though each window's own
    /// bracket was clean. Multi-step activation (the U7 #23 checklist) MUST
    /// open every bracket after the first through this form, with step 1's
    /// [`ExecBracket::config_state`].
    ///
    /// # Errors
    /// Everything [`ExecBracket::open`] refuses, plus
    /// [`OpenRefusal::Guard`] carrying [`GuardError::ConfigChanged`] when
    /// the captured config differs from `run_config`.
    pub fn open_pinned(
        root: &fs::WorkspaceRoot,
        root_after_phase1: &MerkleRoot,
        run_config: &ConfigState,
    ) -> Result<ExecBracket, OpenRefusal> {
        let bracket = Self::open(root, root_after_phase1)?;
        bracket
            .guard
            .verify_config(run_config)
            .map_err(OpenRefusal::Guard)?;
        Ok(bracket)
    }

    /// The captured config state — pin step 1's state and open every later
    /// bracket with [`ExecBracket::open_pinned`] (#20 mid-RUN continuity).
    #[must_use]
    pub fn config_state(&self) -> &ConfigState {
        self.guard.config_state()
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
