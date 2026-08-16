//! The §6.5 checkpoint's storage site and replay adjudication — the disk half
//! of [`fs::checkpoint`] (merkle-spec §6.5; ruled by
//! `decisions/2026-08-15-restart-index-allowed.md`).
//!
//! One file per workspace in its cache drawer, beside `sql.duckdb` and the run
//! plane's digest memo — outside every hash domain by construction, so the
//! object can never move a root or break a held token. Written atomically
//! (temp + rename), read whole, discarded whole.
//!
//! # Lifecycle
//! - **Save** at daemon shutdown, for every workspace holding a resident memo:
//!   that is when the process dies and the in-memory tree with it. An idle-reap
//!   needs no save — the §6.4 feed's registration lifetime already retains the
//!   memo across it.
//! - **Restore** when the registry first creates a workspace's resident memo
//!   ([`crate::Registry::domain_cache`]'s cold entry) — the one door every
//!   consumer already goes through.
//! - **Delete** at `unregister` (the registration ended, and the feed with it)
//!   and on any discard, so one bad file cannot refuse twice.
//!
//! # The cursor, adjudicated here
//! [`fs::checkpoint`] settles LAWFULNESS — may these rows enter as hypotheses —
//! and hands the CURSOR back unjudged. Replay authority is this module's, and
//! trust rides the cursor in neither direction (design ruling § I; §6.3's
//! landed cursor law):
//!
//! - **The cursor anchors** (same epoch, seq inside retained history) ⇒ replay
//!   every frame after it through [`crate::feed::apply`]: exactly the movers
//!   are read and hashed, no unchanged member is touched, no cold rebuild.
//! - **The cursor cannot anchor** ⇒ the **WARM** re-baseline: replay is
//!   forfeited and NOTHING ELSE IS. The rows enter as hypotheses and the §6.2
//!   watermark trust close re-verifies each one before it can serve — the same
//!   instrument that governs a RAM-resident row whose stat evidence is stale.
//!   Re-establishing currency from zero TRUST, never from zero BYTES.
//!
//! A cursor mismatch is CERTAIN across process death — `wire_serve::ring`'s
//! §7.1 law persists no epoch fact — so the warm event fires on every ordinary
//! restart BY DESIGN. That is why it carries its own label and its own receipt
//! field, apart from the cold alarm: reading the cursor as an identity field
//! instead would condemn the object on every restart and it would never once
//! fire. What the restart saves is the cold rebuild (1.45 s measured) and every
//! unchanged member's byte read and re-hash; what it still pays is the §6.2
//! metadata sweep, one `stat` per member and zero bytes (~160 ms, lane B).

use std::path::{Path, PathBuf};

use crate::feed;
use crate::ring::WorkspaceRing;

/// The checkpoint filename inside a workspace's cache drawer. The version
/// rides the name as well as the payload's own tag: an old binary meeting a
/// future file never even reads it.
pub(crate) const CHECKPOINT_FILENAME: &str = "checkpoint.v1";

/// What a restore did, published for the card's counters and the daemon log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointReceipt {
    /// Leaf rows adopted from the file.
    pub leaves: usize,
    /// Members re-derived by journal replay (the changed-while-down set).
    pub replayed: u64,
    /// Whether the stored cursor anchored against the live journal. `false`
    /// means replay was forfeited: rows adopted as hypotheses, and the §6.2
    /// floor re-verifies every one before anything serves.
    pub anchored: bool,
    /// The label of the WARM re-baseline — replay forfeited, object retained —
    /// or `None` when the cursor anchored. A COLD re-baseline never reaches a
    /// receipt: the object is gone and the restore answers `None`, which is
    /// what keeps "exactly one loud re-baseline" auditable (the warm event
    /// fires on every process restart BY DESIGN, so it must not share the
    /// cold event's alarm channel).
    pub warm_rebaseline: Option<String>,
}

/// The checkpoint file for `workspace`'s drawer.
pub(crate) fn path(cache_root: &Path, workspace: &Path) -> PathBuf {
    cache::drawer_dir(cache_root, workspace).join(CHECKPOINT_FILENAME)
}

/// Serialize `cache` and write it atomically into the drawer. Best-effort and
/// silent on failure: a checkpoint that fails to save costs the next start a
/// cold rebuild, never correctness, and must not fail a shutdown.
///
/// The journal cursor is captured by the CALLER before the memo snapshot —
/// under-claiming only re-replays (the overlay is idempotent), over-claiming
/// would skip a change.
pub(crate) fn save(
    cache_root: &Path,
    workspace: &Path,
    cache: &mut fs::DomainCache,
    journal_instance: String,
    journal_seq: u64,
) -> bool {
    let identity = fs::checkpoint::SaveIdentity {
        workspace: workspace.to_path_buf(),
        parse_cache: cache::SCHEMA_SALT.to_owned(),
        journal_instance,
        journal_seq,
    };
    let Some(bytes) = fs::checkpoint::serialize(cache, &identity) else {
        return false; // no observation baseline yet, or an unencodable path
    };
    let dir = cache::drawer_dir(cache_root, workspace);
    if std::fs::create_dir_all(&dir).is_err() {
        return false;
    }
    let tmp = dir.join(format!("{CHECKPOINT_FILENAME}.tmp.{}", std::process::id()));
    if std::fs::write(&tmp, &bytes).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return false;
    }
    if std::fs::rename(&tmp, dir.join(CHECKPOINT_FILENAME)).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return false;
    }
    true
}

/// Read, verify, and replay a workspace's checkpoint. `None` when there is no
/// file, it cannot be read, or it was discarded — in every one of those cases
/// the caller proceeds with a cold memo, which is always correct.
///
/// A discard is LOUD and LABELED, exactly once, and takes the file with it: a
/// checkpoint that cannot be trusted must never be re-offered.
///
/// `ring` is the workspace's LIVE delta ring, or `None` when it holds none —
/// no ring is no journal, so no cursor can anchor and the evidence arm runs.
pub(crate) fn restore(
    cache_root: &Path,
    workspace: &Path,
    ring: Option<&WorkspaceRing>,
) -> Option<(fs::DomainCache, CheckpointReceipt)> {
    let file = path(cache_root, workspace);
    let bytes = std::fs::read(&file).ok()?;
    let root = fs::WorkspaceRoot(workspace.to_path_buf());
    let domain = match fs::domain::Domain::load(&root) {
        Ok(domain) => domain,
        Err(e) => {
            eprintln!(
                "checkpoint: {} — domain config unreadable ({e}); cold start",
                workspace.display()
            );
            return None;
        }
    };
    let restored = match fs::checkpoint::restore(&bytes, workspace, cache::SCHEMA_SALT, &domain) {
        Ok(restored) => restored,
        Err(discard) => {
            // COLD re-baseline: the rows are not lawful statements about this
            // world under today's law, so nothing survives. Rare and
            // meaningful — this is the alarm channel, and it stays quiet
            // precisely because the warm event below does not share it.
            eprintln!(
                "checkpoint: COLD RE-BASELINE for {} — {discard}; the checkpoint is discarded \
                     and the corpus rebuilds cold",
                workspace.display()
            );
            let _ = std::fs::remove_file(&file);
            return None;
        }
    };

    let mut cache = restored.cache;
    let leaves = restored.leaves;
    let cursor = wire_serve::ring::Cursor {
        tree_instance: restored.journal_instance,
        seq: restored.journal_seq,
    };
    let anchor = ring.map_or(wire_serve::ring::Anchor::DeadInstance, |ring| {
        ring.can_anchor(&cursor)
    });
    let receipt = match (anchor, ring) {
        (wire_serve::ring::Anchor::Anchored, Some(ring)) => {
            // The cursor anchors: replay exactly the members the journal
            // names. One read and one hash per changed member, zero unchanged
            // members statted, no cold rebuild.
            let paths = replay_paths(ring, cursor.seq);
            match feed::apply(&root, &mut cache, paths) {
                feed::Applied::Members(replayed) => CheckpointReceipt {
                    leaves,
                    replayed,
                    anchored: true,
                    warm_rebaseline: None,
                },
                feed::Applied::Reset => {
                    // A member the journal NAMED could not be derived, and
                    // absence would be a lie. The rows can no longer be
                    // reconciled with disk, so this is the COLD event.
                    eprintln!(
                        "checkpoint: COLD RE-BASELINE for {} — replay could not derive a named \
                         member; the resident memo is reset and the next pass re-reads the \
                         corpus",
                        workspace.display()
                    );
                    let _ = std::fs::remove_file(&file);
                    return None;
                }
                // The rescan ladder's rungs answer `Pending::All` only, and
                // this call site builds `Clean`/`Paths` from journal frames —
                // so these are defensive, not expected. Both leave the memo in
                // a valid state (Sweep keeps it for the next full stat sweep;
                // Rebaselined already swapped in a fresh full derivation), so
                // the safe report is the truth: the ladder re-established
                // currency and this replay claims nothing.
                feed::Applied::Sweep(cause) | feed::Applied::Rebaselined(cause) => {
                    eprintln!(
                        "checkpoint: warm re-baseline for {} — the rescan ladder ({}) took over \
                         during replay; no replay is claimed and currency comes from the \
                         ladder's own rung",
                        workspace.display(),
                        cause.name()
                    );
                    CheckpointReceipt {
                        leaves,
                        replayed: 0,
                        anchored: true,
                        warm_rebaseline: Some(format!("rescan ladder took over: {}", cause.name())),
                    }
                }
            }
        }
        (verdict, _) => {
            // WARM re-baseline: the cursor cannot anchor, so REPLAY is
            // forfeited — and nothing else is. The rows enter as hypotheses
            // and the §6.2 floor re-verifies each one before it can serve.
            // Trust never rode the cursor in either direction, so this is not
            // an alarm: it fires on every ordinary process restart by design
            // (§7.1 persists no epoch fact), which is exactly why it is
            // labeled apart from the cold event.
            let label = match verdict {
                wire_serve::ring::Anchor::DeadInstance => {
                    "cursor epoch died (restart or reap) — no frame can be replayed"
                }
                _ => "cursor is outside retained history",
            };
            eprintln!(
                "checkpoint: warm re-baseline for {} — {label}; {leaves} leaf row(s) adopted \
                 as hypotheses, each re-verified against disk by the §6.2 trust close before \
                 it serves",
                workspace.display()
            );
            CheckpointReceipt {
                leaves,
                replayed: 0,
                anchored: false,
                warm_rebaseline: Some(label.to_owned()),
            }
        }
    };
    Some((cache, receipt))
}

/// Delete a workspace's checkpoint — the registration ended, so its derived
/// index must not outlive it.
pub(crate) fn discard(cache_root: &Path, workspace: &Path) {
    let _ = std::fs::remove_file(path(cache_root, workspace));
}

/// Every member path the journal names after `seq`, as a feed take. A rename
/// carries BOTH halves (the origin path leaves the fold, the destination
/// enters it), so neither side is missed.
fn replay_paths(ring: &WorkspaceRing, seq: u64) -> feed::Pending {
    let mut paths: Vec<PathBuf> = Vec::new();
    for frame in ring.frames_after(seq) {
        for file in &frame.delta.files {
            paths.push(PathBuf::from(file.path.0.clone()));
            if let Some(from) = &file.from_path {
                paths.push(PathBuf::from(from.0.clone()));
            }
        }
    }
    if paths.is_empty() {
        return feed::Pending::Clean;
    }
    paths.sort();
    paths.dedup();
    feed::Pending::Paths(paths)
}

#[cfg(test)]
mod tests {
    //! The §6.5 checkpoint gates, counters published on every assertion.

    use super::*;
    use crate::Registry;
    use crate::state::StateStore;

    /// `home/ws` seeded with three members, and `home/cache` beside it (so
    /// the corpus walk never sees the drawer).
    fn workspace(home: &Path) -> (PathBuf, PathBuf) {
        let ws = home.join("ws");
        let cache_root = home.join("cache");
        std::fs::create_dir_all(ws.join("notes")).unwrap();
        std::fs::create_dir_all(&cache_root).unwrap();
        std::fs::write(ws.join("a.md"), "# A\n").unwrap();
        std::fs::write(ws.join("notes/b.md"), "# B\n").unwrap();
        std::fs::write(ws.join("notes/c.md"), "# C\n").unwrap();
        (ws, cache_root)
    }

    /// An observed memo over `ws` — the state a live daemon holds.
    fn observed(ws: &Path) -> fs::DomainCache {
        let mut cache = fs::DomainCache::new();
        cache.root(&fs::WorkspaceRoot(ws.to_path_buf())).unwrap();
        cache
    }

    /// A frame at `seq` naming `rel` as modified — one journaled change.
    fn frame(seq: u64, rel: &str) -> wire::DeltaFrame {
        wire::DeltaFrame {
            delta: wire::Delta {
                seq,
                root_before: wire::Root(format!("b3:{:064x}", seq - 1)),
                root_after: wire::Root(format!("b3:{seq:064x}")),
                actor: None,
                now: None,
                files: vec![wire::DeltaFile {
                    path: wire::Path(rel.to_owned()),
                    change: wire::FileChange::Modified,
                    from_path: None,
                    file_rev_before: None,
                    file_rev_after: None,
                    nodes: vec![],
                }],
            },
            effects: vec![],
            rescope: None,
            overflow: None,
        }
    }

    /// **Codex gate 11**, the anchored arm: one journaled change while down
    /// costs exactly one file read and hashed, zero unchanged members
    /// statted, and no cold rebuild. Every clause is asserted through a
    /// published counter, not an impression.
    #[test]
    fn gate_11_one_journaled_change_costs_one_read_zero_stats_no_cold_rebuild() {
        let home = tempfile::tempdir().unwrap();
        let (ws, cache_root) = workspace(home.path());
        let root = fs::WorkspaceRoot(ws.clone());

        // A live daemon: resident memo, live journal, checkpoint at shutdown.
        let mut memo = observed(&ws);
        let ring = WorkspaceRing::new(&root);
        assert!(save(
            &cache_root,
            &ws,
            &mut memo,
            ring.instance(),
            ring.seq()
        ));

        // Down: exactly one member changes, and the journal records it.
        std::thread::sleep(std::time::Duration::from_millis(10));
        std::fs::write(ws.join("notes/b.md"), "# B moved\n").unwrap();
        ring.advance(frame(1, "notes/b.md"));

        // Restart: restore and replay.
        let (mut back, receipt) =
            restore(&cache_root, &ws, Some(&ring)).expect("the checkpoint restores");

        assert!(
            receipt.anchored,
            "the cursor anchors against its own journal"
        );
        assert_eq!(receipt.leaves, 3, "every leaf row is adopted");
        assert_eq!(
            receipt.replayed, 1,
            "gate 11: exactly one file read and hashed — the one journaled change"
        );
        assert_eq!(
            back.listings(),
            0,
            "gate 11: zero unchanged members statted — no directory was ever enumerated"
        );
        assert_eq!(
            back.leaves_read(),
            0,
            "gate 11: no cold rebuild — the observation path read no member's bytes"
        );
        // And the replayed state is CORRECT: the restored root equals a fresh
        // derivation of the same disk.
        assert_eq!(
            back.overlay_root().unwrap(),
            fs::DomainCache::new().root(&root).unwrap(),
            "the replayed tree equals a full re-derivation"
        );
    }

    /// **The anti-vacuity gate** (design ruling § I): a cursor mismatch ALONE
    /// must not discard the object. Read as an identity field the cursor would
    /// condemn the checkpoint on every ordinary restart — §7.1 persists no
    /// epoch fact, so the mismatch is CERTAIN — and the object would never
    /// once fire. Restore across that restart must reach the memo with its
    /// rows intact, the file still on disk, and zero unchanged members read.
    #[test]
    fn a_cursor_mismatch_alone_never_discards_the_object() {
        let home = tempfile::tempdir().unwrap();
        let (ws, cache_root) = workspace(home.path());
        let root = fs::WorkspaceRoot(ws.clone());

        let mut memo = observed(&ws);
        let dead = WorkspaceRing::new(&root);
        assert!(save(
            &cache_root,
            &ws,
            &mut memo,
            dead.instance(),
            dead.seq()
        ));

        // The ordinary restart: a new epoch, so the stored cursor is certain
        // to mismatch.
        let restarted = WorkspaceRing::new(&root);
        assert_ne!(dead.instance(), restarted.instance());

        let (back, receipt) = restore(&cache_root, &ws, Some(&restarted))
            .expect("a cursor mismatch forfeits replay, never the object");

        assert_eq!(receipt.leaves, 3, "every row survives the warm re-baseline");
        assert!(receipt.warm_rebaseline.is_some(), "and it is labeled warm");
        assert_eq!(
            back.leaves_read(),
            0,
            "zero unchanged members read — the whole point of keeping the rows"
        );
        assert_eq!(
            path(&cache_root, &ws).exists(),
            true,
            "the file survives too: only a COLD re-baseline removes it"
        );
    }

    /// The warm arm end to end: rows adopted as hypotheses, and the §6.2 floor
    /// re-verifies every one — the changed member is re-read, the unchanged
    /// ones are not, and the answer is the disk's truth.
    #[test]
    fn a_dead_cursor_keeps_the_rows_as_evidence_and_re_baselines_once() {
        let home = tempfile::tempdir().unwrap();
        let (ws, cache_root) = workspace(home.path());
        let root = fs::WorkspaceRoot(ws.clone());

        let mut memo = observed(&ws);
        let dead = WorkspaceRing::new(&root);
        assert!(save(
            &cache_root,
            &ws,
            &mut memo,
            dead.instance(),
            dead.seq()
        ));

        // A restart mints a NEW ring: the stored numbering is gone.
        let fresh = WorkspaceRing::new(&root);
        assert_ne!(dead.instance(), fresh.instance());
        std::thread::sleep(std::time::Duration::from_millis(10));
        std::fs::write(ws.join("notes/b.md"), "# B moved while down\n").unwrap();

        let (mut back, receipt) =
            restore(&cache_root, &ws, Some(&fresh)).expect("rows survive a dead cursor");
        assert!(!receipt.anchored);
        assert_eq!(receipt.replayed, 0, "nothing can be replayed");
        assert_eq!(receipt.leaves, 3, "the rows are adopted as evidence");
        assert!(
            receipt
                .warm_rebaseline
                .as_deref()
                .is_some_and(|label| label.contains("cursor epoch died")),
            "the warm re-baseline is labeled, not silent"
        );

        // The floor that makes the evidence arm sound: a live observation
        // re-verifies every member, so the answer is the truth on disk — the
        // changed member is re-read, the unchanged ones are not.
        assert_eq!(
            back.root(&root).unwrap(),
            fs::DomainCache::new().root(&root).unwrap(),
            "the observation answers the disk's truth, never the stored rows"
        );
        assert_eq!(
            back.leaves_read(),
            1,
            "only the member that moved was re-read — cost follows change"
        );
    }

    /// A checkpoint can never outlive a `domain_version` change: the restore
    /// discards loudly and TAKES THE FILE WITH IT, so one bad file cannot
    /// refuse twice.
    #[test]
    fn a_domain_version_bump_discards_and_deletes_the_file() {
        let home = tempfile::tempdir().unwrap();
        let (ws, cache_root) = workspace(home.path());
        let root = fs::WorkspaceRoot(ws.clone());
        let mut memo = observed(&ws);
        let ring = WorkspaceRing::new(&root);
        assert!(save(
            &cache_root,
            &ws,
            &mut memo,
            ring.instance(),
            ring.seq()
        ));
        assert!(path(&cache_root, &ws).exists());

        // The hash law's other dimension moves.
        std::fs::create_dir_all(ws.join("meridian")).unwrap();
        std::fs::write(ws.join("meridian/domain.md"), "---\nversion: 3\n---\n").unwrap();

        assert!(
            restore(&cache_root, &ws, Some(&ring)).is_none(),
            "a checkpoint may not outlive the domain version it was folded under"
        );
        assert!(
            !path(&cache_root, &ws).exists(),
            "the discarded file is removed — it must never be re-offered"
        );
    }

    /// The registry's own wiring: a cold `domain_cache` adopts the file and
    /// publishes its receipt; `unregister` ends the registration and the
    /// derived index with it.
    #[test]
    fn the_registry_adopts_at_cold_entry_and_discards_at_unregister() {
        let home = tempfile::tempdir().unwrap();
        let (ws, cache_root) = workspace(home.path());
        let ws = std::fs::canonicalize(&ws).unwrap();

        let reg = Registry::new(
            StateStore::new(home.path().join("state.json")),
            cache_root.clone(),
            Vec::new(),
        );
        // Warm it: the memo observes, then the shutdown hook persists it.
        reg.domain_cache(&ws)
            .lock()
            .unwrap()
            .root(&fs::WorkspaceRoot(ws.clone()))
            .unwrap();
        reg.save_checkpoints();
        assert!(path(&cache_root, &ws).exists(), "shutdown persisted it");

        // A fresh registry over the same drawer — the restart.
        let restarted = Registry::new(
            StateStore::new(home.path().join("state.json")),
            cache_root.clone(),
            Vec::new(),
        );
        assert!(
            restarted.checkpoint_receipt(&ws).is_none(),
            "no receipt before the first borrow"
        );
        let _ = restarted.domain_cache(&ws);
        let receipt = restarted
            .checkpoint_receipt(&ws)
            .expect("the cold entry adopted the checkpoint");
        assert_eq!(receipt.leaves, 3);

        restarted.unregister(&ws);
        assert!(
            !path(&cache_root, &ws).exists(),
            "the derived index does not outlive its registration"
        );
    }
}
