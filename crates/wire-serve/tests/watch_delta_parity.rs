//! §6.7 frame-parity gates: `reconcile_delta` — classification off a
//! resident-memo leaf view, bytes read for MOVERS only — mints frames
//! byte-identical to `reconcile`, the full-snapshot classifier, over every
//! mutation kind the watch plane tells. Two twin (ring, watch) states are
//! driven over ONE disk history; every emission is compared as serialized
//! JSON, so a divergence in any field — change kind, revs, node grain,
//! rename pairing, rescope summary, overflow, effects — fails loud.
//!
//! The race arm is gated too: a mover whose re-read digest disagrees with
//! the observed leaf view emits NOTHING and holds its baseline; the next
//! cycle with a fresh view tells the whole truth once.

use std::collections::BTreeMap;
use std::path::Path;

use wire::DeltaFrame;
use wire_serve::ring::RootRing;
use wire_serve::watch::{WatchState, reconcile, reconcile_delta};

/// The observed leaf view + fold a resident memo would serve: a fresh
/// extent-refresh floor pass over the same tree — value-identical to the
/// memo's, String-spelled exactly as the snapshot spells its files.
fn leaf_view(root: &fs::WorkspaceRoot) -> (BTreeMap<String, [u8; 32]>, wire::Root) {
    let mut memo = fs::DomainCache::new();
    let folded = memo.root(root).expect("floor pass");
    let leaves = memo
        .leaf_digests()
        .into_iter()
        .filter_map(|(rel, d)| rel.to_str().map(|s| (s.to_owned(), d)))
        .collect();
    (leaves, wire::Root(folded.0))
}

/// Twin classifiers over one disk history: the snapshot form and the
/// memo-view form, each with its own ring so seq chains stay independent
/// and must still agree.
struct Twins {
    snap_ring: RootRing,
    snap_watch: WatchState,
    delta_ring: RootRing,
    delta_watch: WatchState,
}

impl Twins {
    fn primed(ws_root: &fs::WorkspaceRoot) -> Twins {
        let mut twins = Twins {
            snap_ring: RootRing::new(),
            snap_watch: WatchState::new(ws_root),
            delta_ring: RootRing::new(),
            delta_watch: WatchState::new(ws_root),
        };
        assert!(
            reconcile(ws_root, &mut twins.snap_ring, &mut twins.snap_watch)
                .expect("snapshot prime")
                .is_none(),
            "priming emits nothing"
        );
        let (leaves, disk_root) = leaf_view(ws_root);
        assert!(
            reconcile_delta(
                ws_root,
                &mut twins.delta_ring,
                &mut twins.delta_watch,
                &leaves,
                &disk_root,
            )
            .expect("delta prime (falls to the snapshot arm)")
            .is_none(),
            "priming emits nothing"
        );
        twins
    }

    /// One reconcile step on both twins; frames must match byte-for-byte.
    fn step(&mut self, ws_root: &fs::WorkspaceRoot, label: &str) -> Option<DeltaFrame> {
        let snap = reconcile(ws_root, &mut self.snap_ring, &mut self.snap_watch)
            .expect("snapshot reconcile");
        let (leaves, disk_root) = leaf_view(ws_root);
        let delta = reconcile_delta(
            ws_root,
            &mut self.delta_ring,
            &mut self.delta_watch,
            &leaves,
            &disk_root,
        )
        .expect("delta reconcile");
        assert_eq!(
            snap.as_ref().map(|f| serde_json::to_value(f).unwrap()),
            delta.as_ref().map(|f| serde_json::to_value(f).unwrap()),
            "frame parity broke at step: {label}"
        );
        delta
    }
}

fn workspace(tmp: &Path, files: &[(&str, &[u8])]) -> fs::WorkspaceRoot {
    let ws = tmp.join("ws");
    for (rel, body) in files {
        let path = ws.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }
    fs::WorkspaceRoot(std::fs::canonicalize(&ws).unwrap())
}

#[test]
fn every_mutation_kind_mints_the_same_frame_off_the_leaf_view() {
    let tmp = tempfile::tempdir().unwrap();
    let root = workspace(
        tmp.path(),
        &[
            ("plan.md", b"# Goals\n\nship by August\n"),
            ("notes/a.md", b"# A\n\nbody\n"),
            ("notes/b.md", b"# B\n"),
            (
                fs::domain::DOMAIN_CONFIG_PATH,
                b"---\nignore:\n  - \"drafts/**\"\n---\n# Domain\n",
            ),
        ],
    );
    let mut twins = Twins::primed(&root);

    // Quiet: both silent.
    assert!(twins.step(&root, "quiet").is_none());

    // Edit: node-grain modified entry.
    std::fs::write(root.0.join("plan.md"), "# Goals\n\nship by September\n").unwrap();
    let frame = twins.step(&root, "edit").expect("an edit emits");
    assert_eq!(frame.delta.files.len(), 1);

    // Add (into a brand-new directory).
    std::fs::create_dir_all(root.0.join("inbox")).unwrap();
    std::fs::write(root.0.join("inbox/new.md"), "# New\n").unwrap();
    twins.step(&root, "add").expect("an add emits");

    // Remove.
    std::fs::remove_file(root.0.join("notes/b.md")).unwrap();
    twins.step(&root, "remove").expect("a remove emits");

    // Rename: disk-true removal + byte-equal addition in one batch.
    let bytes = std::fs::read(root.0.join("notes/a.md")).unwrap();
    std::fs::remove_file(root.0.join("notes/a.md")).unwrap();
    std::fs::write(root.0.join("notes/renamed.md"), &bytes).unwrap();
    let frame = twins.step(&root, "rename").expect("a rename emits");
    assert_eq!(
        frame.delta.files[0]
            .from_path
            .as_ref()
            .map(|p| p.0.as_str()),
        Some("notes/a.md"),
        "the ruled rename pairs"
    );

    // Non-UTF-8 CONTENT: §52 degraded entry, parity preserved.
    std::fs::write(root.0.join("inbox/new.md"), [0xff, 0xfe, 0x01]).unwrap();
    twins
        .step(&root, "non-utf8 content")
        .expect("degrades, still told");

    // Re-scope: the config edit collapses membership changes into the
    // summary and rides first.
    std::fs::write(
        root.0.join(fs::domain::DOMAIN_CONFIG_PATH),
        "---\nignore:\n  - \"drafts/**\"\n  - \"notes/**\"\n---\n# Domain\n",
    )
    .unwrap();
    let frame = twins.step(&root, "rescope").expect("a re-scope emits");
    assert!(
        frame.rescope.is_some(),
        "membership changes collapse into rescope"
    );

    // Mixed batch: edit + add + remove in one telling.
    std::fs::write(root.0.join("plan.md"), "# Goals\n\nship by October\n").unwrap();
    std::fs::write(root.0.join("inbox/second.md"), "# Second\n").unwrap();
    std::fs::remove_file(root.0.join("inbox/new.md")).unwrap();
    let frame = twins.step(&root, "mixed").expect("a mixed batch emits");
    assert_eq!(frame.delta.files.len(), 3);

    // Settled: silent again.
    assert!(twins.step(&root, "settled").is_none());
}

/// The race arm: a leaf view observed BEFORE a second write disagrees with
/// the re-read bytes — the cycle emits nothing and holds its baseline; the
/// next cycle with a fresh view tells the final truth once, and the
/// snapshot twin (stepped once over the settled state) agrees.
#[test]
fn a_mid_cycle_race_emits_nothing_and_the_next_cycle_tells_the_truth_once() {
    let tmp = tempfile::tempdir().unwrap();
    let root = workspace(tmp.path(), &[("plan.md", b"# Goals\n\nv1\n")]);
    let mut twins = Twins::primed(&root);

    std::fs::write(root.0.join("plan.md"), "# Goals\n\nv2\n").unwrap();
    let (stale_leaves, stale_root) = leaf_view(&root);
    // The racing second write lands after the observation.
    std::fs::write(root.0.join("plan.md"), "# Goals\n\nv3\n").unwrap();

    let raced = reconcile_delta(
        &root,
        &mut twins.delta_ring,
        &mut twins.delta_watch,
        &stale_leaves,
        &stale_root,
    )
    .expect("the race is not an error");
    assert!(raced.is_none(), "a raced cycle emits nothing");
    assert!(
        twins.delta_ring.frames_after(0).is_empty(),
        "and advances no seq"
    );

    // The settled cycle tells v1 → v3 once, and matches the snapshot twin.
    let frame = twins
        .step(&root, "settled after race")
        .expect("one telling");
    assert_eq!(frame.delta.files.len(), 1);
}
