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

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use run::executor::{CommitFacts, DeltaSink};
use wire::Root;
use wire_serve::seq::SeqSink as _;

use crate::registry::{DOOR_COOKIE_TIMEOUT, Registry};
use crate::ring::WorkspaceRing;

/// The registry's sink: one per served run/script call, bound to the bound
/// workspace's ring. Frames from a multi-commit run chain in commit order
/// because each commit advances the ring before the next can begin.
///
/// Both frame roots are the sink's observations. On a submission that
/// borrowed the workspace's resident domain memo (one carrying a live task
/// target — `run_op::serve`), they are made through that memo at the door
/// grade ([`Registry::door_observation`]: §6.4 cookie barrier, take-and-apply,
/// the vouched overlay fold, the §6.2 stat floor on a named miss —
/// `node-rev-merkle-spec.md` §6.7). `root_before` is asked with the
/// executor's flock held, before the commit; `root_after` after it, on the
/// same handle. Neither reads a byte of the corpus that did not move: the
/// two used to be `domain_snapshot` folds — every member read and hashed,
/// twice per committed batch.
///
/// A mode-only submission borrows no memo (a fire drives no currency pass
/// and never parks behind one — run-plane.md § The world a mode-bearing row
/// runs against), so the sink of a committing fire folds both tenses from
/// bytes ([`fs::domain_fold`]): the flat build's grade and price, paid only
/// by the rare fire that commits.
#[derive(Debug)]
pub(crate) struct RingSink<'r> {
    ring: Arc<WorkspaceRing>,
    registry: &'r Registry,
    ws: PathBuf,
    /// The submission's memo handle, taken once per served call when a live
    /// task target is present: both observations land in the same tree, so
    /// `root_before` and `root_after` cannot fold from two different
    /// generations. `None` on a mode-only submission.
    cache: Option<Arc<Mutex<fs::DomainCache>>>,
}

impl<'r> RingSink<'r> {
    pub(crate) fn new(
        registry: &'r Registry,
        ws: &Path,
        cache: Option<Arc<Mutex<fs::DomainCache>>>,
    ) -> Self {
        RingSink {
            ring: registry.ring(ws),
            registry,
            ws: ws.to_path_buf(),
            cache,
        }
    }

    /// The door-grade observation on this sink's memo handle, or the flat
    /// fold when the submission borrowed none.
    fn observe(&self, root: &fs::WorkspaceRoot) -> io::Result<model::MerkleRoot> {
        match &self.cache {
            Some(cache) => self
                .registry
                .door_observation(&self.ws, cache, DOOR_COOKIE_TIMEOUT),
            None => fs::domain_fold(root),
        }
    }
}

impl DeltaSink for RingSink<'_> {
    fn root_before(&self, root: &fs::WorkspaceRoot) -> io::Result<model::MerkleRoot> {
        self.observe(root)
    }

    fn committed(&self, root: &fs::WorkspaceRoot, facts: &CommitFacts<'_>) {
        // The after tenses: the workspace root observed post-commit (the
        // flock is still the executor's, so disk is settled; the cookie
        // barrier proves this commit's own events are folded in), and the
        // receipt file as committed.
        let root_after = match self.observe(root) {
            Ok(r) => Root(r.0),
            Err(e) => {
                eprintln!("registry: run delta mint (post-commit observation): {e}");
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
            None, // the run plane mints no pins, so no promotion row
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
