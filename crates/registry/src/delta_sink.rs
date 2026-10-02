//! The run plane's frame mint (§ A.8 Delta honesty, run-delta ruling
//! 2026-08-14): run applies mint Deltas like every other daemon-side write.
//!
//! The plane's executor commits through its own seam and offers each
//! committed batch's facts to a [`run::executor::DeltaSink`]; this module is
//! the one production implementor. It assembles the frame at the §7.3 single
//! constructor and advances the workspace ring — all inside the executor's
//! write flock, which is what closes the detector window: by the time a
//! detect cycle can take the flock, the ring tip already carries the moved
//! root.
//!
//! The bracket rides the wire door's plane ([`DoorPlane`]) and its two steps:
//! `root_before` is the door-entry observation through the workspace's
//! resident memo, with the watcher baseline settled onto it; `root_after` is
//! the commit's own overlay of the page and receipt bytes onto that memo,
//! and the baseline follows the frame. No corpus is read under the flock.
//!
//! Degradation posture: the mint never fails a landed commit. A post-commit
//! overlay or receipt read failure logs and skips the frame — the detector's
//! next cycle reconciles the change as external (actor-absent), degraded but
//! never wrong — matching the push loop's own log-and-retry posture.

use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use run::executor::{CommitFacts, DeltaSink};
use wire::Root;
use wire_serve::seq::SeqSink as _;

use crate::Registry;
use crate::ring::{DoorPlane, WorkspaceRing};

/// The registry's sink: one per served run/script call, bound to the bound
/// workspace's ring. Frames from a multi-commit run chain in commit order
/// because each commit advances the ring before the next can begin.
#[derive(Debug)]
pub(crate) struct RingSink<'a> {
    registry: &'a Registry,
    ring: Arc<WorkspaceRing>,
    /// The workspace memo, borrowed at the first [`DeltaSink::opening`] —
    /// never before: a call that commits nothing never touches the memo.
    /// Later commits reuse the handle; re-borrowing would apply the feed's
    /// pending set between a commit's observation and its overlay.
    cache: OnceLock<Arc<Mutex<fs::DomainCache>>>,
}

impl<'a> RingSink<'a> {
    pub(crate) fn new(registry: &'a Registry, ws: &Path) -> Self {
        RingSink {
            registry,
            ring: registry.ring(ws),
            cache: OnceLock::new(),
        }
    }

    /// The memo the bracket observes and overlays, borrowed bounded as a
    /// write door borrows it (`server::write_door_memo`).
    fn cache(&self, root: &fs::WorkspaceRoot) -> std::io::Result<&Arc<Mutex<fs::DomainCache>>> {
        if let Some(cache) = self.cache.get() {
            return Ok(cache);
        }
        let cache = self
            .registry
            .domain_cache_within(&root.0, crate::registry::DOOR_COOKIE_TIMEOUT)?;
        Ok(self.cache.get_or_init(|| cache))
    }
}

/// `root_after` from the commit's own overlay: the page's committed bytes,
/// and the receipt's when it lies in the hash domain (the overlay ignores a
/// path outside it). No walk, no stat, no corpus read — the write door's
/// `overlaid_root`.
fn overlaid_root(
    cache: &Mutex<fs::DomainCache>,
    facts: &CommitFacts<'_>,
    receipt_after: Option<&model::Document>,
) -> std::io::Result<Root> {
    let mut memo = cache.lock().unwrap_or_else(PoisonError::into_inner);
    memo.overlay_leaf(
        Path::new(facts.page),
        model::leaf_digest(facts.after.raw.as_bytes()),
    )?;
    if let (Some(rp), Some(doc)) = (facts.receipt_path, receipt_after) {
        memo.overlay_leaf(Path::new(rp), model::leaf_digest(doc.raw.as_bytes()))?;
    }
    Ok(Root(memo.overlay_root()?.0))
}

impl DeltaSink for RingSink<'_> {
    fn opening(&self, root: &fs::WorkspaceRoot) -> std::io::Result<model::MerkleRoot> {
        let cache = self.cache(root)?;
        DoorPlane::new(self.registry, root, &self.ring, cache).observe()
    }

    fn committed(&self, root: &fs::WorkspaceRoot, facts: &CommitFacts<'_>) {
        let Some(cache) = self.cache.get() else {
            // `opening` borrowed the memo before any byte landed; a commit
            // without it is an executor ordering defect, never minted around.
            eprintln!("registry: run delta mint: commit offered without an opening");
            return;
        };
        // The receipt file as committed (the flock is still the executor's).
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
        let root_after = match overlaid_root(cache, facts, receipt_after.as_ref()) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("registry: run delta mint (commit overlay): {e}");
                return;
            }
        };
        let root_before = Root(facts.root_before.0.clone());
        if root_before == root_after {
            return; // no root advance, no Delta (§7.1)
        }
        let files = wire_serve::write::commit_delta_files(
            facts.page,
            facts.before,
            facts.after,
            facts
                .receipt_path
                .map(|rp| (rp, facts.receipt_before, receipt_after.as_ref())),
            None, // the run plane mints no pins, so no promotion row
        );
        // Allocate under the executor's flock and record in the same act —
        // no allocation gap exists on this producer. The plane records the
        // frame and moves the baseline onto its `root_after`, which the memo
        // already folds to — no second observation.
        let plane = DoorPlane::new(self.registry, root, &self.ring, cache);
        let seq = plane.allocate(&root_before, &root_after, &files);
        let frame = wire_serve::write::assemble_delta(
            seq,
            root_before,
            root_after,
            Some(facts.actor.to_owned()),
            facts.now.map(str::to_owned),
            files,
        );
        plane.committed(&frame);
    }
}
