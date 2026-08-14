//! The run plane's frame mint (§ A.8 Delta honesty, run-delta ruling
//! 2026-08-14): run applies mint Deltas like every other daemon-side write.
//!
//! The plane's executor commits through its own seam and offers each
//! committed batch's facts to a [`run::executor::DeltaSink`]; this module is
//! the one production implementor. It assembles the frame at the §7.3 single
//! constructor and advances the workspace ring — all inside the executor's
//! write flock, which is what closes the detector window: by the time a
//! detect cycle can take the flock, the ring tip already carries the moved
//! root and `reconcile` syncs silently (the internal-commit arm).
//!
//! Degradation posture: the mint never fails a landed commit. A post-commit
//! fold or receipt read failure logs and skips the frame — the detector's
//! next cycle reconciles the change as external (actor-absent), degraded but
//! never wrong — matching the push loop's own log-and-retry posture.

use std::path::Path;
use std::sync::Arc;

use run::executor::{CommitFacts, DeltaSink};
use wire::Root;
use wire_serve::seq::SeqSink as _;

use crate::ring::WorkspaceRing;

/// The registry's sink: one per served run/script call, bound to the bound
/// workspace's ring. Frames from a multi-commit run chain in commit order
/// because each commit advances the ring before the next can begin.
#[derive(Debug)]
pub(crate) struct RingSink {
    ring: Arc<WorkspaceRing>,
}

impl RingSink {
    pub(crate) fn new(ring: Arc<WorkspaceRing>) -> Self {
        RingSink { ring }
    }
}

impl DeltaSink for RingSink {
    fn committed(&self, root: &fs::WorkspaceRoot, facts: &CommitFacts<'_>) {
        // The after tenses: the workspace root folded post-commit (the flock
        // is still the executor's, so disk is settled), and the receipt file
        // as committed.
        let root_after = match wire_serve::ambient_root(root) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("registry: run delta mint (post-commit fold): {e:?}");
                return;
            }
        };
        let root_before = Root(facts.root_before.0.clone());
        if root_before == root_after {
            return; // no root advance, no Delta (§7.1)
        }
        let receipt_after = match facts.receipt_path {
            Some(rp) => match fs::load(root, Path::new(rp)) {
                Ok(doc) => Some(doc),
                Err(e) => {
                    eprintln!("registry: run delta mint (receipt after tense): {e}");
                    return;
                }
            },
            None => None,
        };
        let files = wire_serve::write::commit_delta_files(
            facts.page,
            facts.before,
            facts.after,
            facts
                .receipt_path
                .map(|rp| (rp, facts.receipt_before, receipt_after.as_ref())),
        );
        // Allocate under the executor's flock and advance in the same act —
        // no allocation gap exists on this producer.
        let seq = self.ring.allocate(&root_before, &root_after, &files);
        self.ring.advance(wire_serve::write::assemble_delta(
            seq,
            root_before,
            root_after,
            Some(facts.actor.to_owned()),
            facts.now.map(str::to_owned),
            files,
        ));
    }
}
