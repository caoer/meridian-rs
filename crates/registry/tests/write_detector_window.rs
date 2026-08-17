//! The write-path detector window (seq:655): the write choke-point records
//! its frame on the ring BEFORE the workspace flock drops, so a detect cycle
//! racing the write can never re-tell the just-committed change as an
//! actor-absent external frame.
//!
//! The old shape — sink allocates inside the flock, the caller advances the
//! ring after the choke-point returns — left a window in which the detector
//! takes the flock, finds `ring.tip_root() != disk_root`, and emits an
//! EXTERNAL frame (`actor`/`now` absent, §7.1) for the same disk movement:
//! one change delivered twice, the second telling actor-absent. The run
//! plane never had the window (`delta_sink.rs` advances inside the
//! executor's flock); this test pins the write paths to the same law.

use registry::ring::WorkspaceRing;
use std::collections::BTreeMap;
use wire::{Edit, EditShape, SecRef};
use wire_serve::guard::Origin;
use wire_serve::write::{SpliceArgs, splice};

fn workspace(tmp: &std::path::Path, files: &[(&str, &str)]) -> std::path::PathBuf {
    let ws = tmp.join("ws");
    for (rel, body) in files {
        let path = ws.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }
    std::fs::canonicalize(&ws).unwrap()
}

/// The worst-case interleaving of the old race, simulated at its exact seam:
/// the detector runs a full cycle at the first instant the flock is free
/// after a splice. Under the fixed shape the frame already landed inside the
/// flock, so the cycle finds `tip_root == disk_root` and syncs silently (the
/// internal-commit arm). Under the old shape the writer's frame was still in
/// the caller's hands at that instant, and the cycle re-told the change as
/// external.
#[test]
fn a_detect_cycle_racing_a_splice_re_tells_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = workspace(tmp.path(), &[("plan.md", "# Plan\n\n- [ ] item one ^t1\n")]);
    let ws_root = fs::WorkspaceRoot(ws);
    let ring = WorkspaceRing::new(&ws_root);
    ring.prime(&ws_root).expect("baseline prime");

    // The registry's own splice shape (server arm / script `put_live`): wire
    // door, forced guard, the workspace ring as the seq sink.
    let args = SpliceArgs {
        premises: Vec::new(),
        id: None,
        path: wire::Path("plan.md".to_string()),
        origin: Origin::Wire,
        actor: Some("alice".to_string()),
        now: Some("2026-08-15T00:00:00Z".to_string()),
        receipt: None,
        if_root: None,
        dry: false,
        force: true,
        edits: vec![Edit {
            target: SecRef::Anchor {
                anchor: "t1".to_string(),
            },
            edit: EditShape::Match {
                old: "[ ]".to_string(),
                new: "[x]".to_string(),
            },
            if_node_rev: None,
        }],
        plan_edits: Vec::new(),
        pin: None,
        fields: BTreeMap::default(),
    };
    let out = splice(&ws_root, Some(&ring), &args, &[], None).expect("the splice commits");
    let frame = out
        .committed
        .as_ref()
        .expect("a real commit emits one delta");

    // The closed window, stated positively: the frame is on the ring the
    // instant the choke-point returns — recorded before the flock dropped.
    // Under the old shape the ring is empty here; that emptiness IS the
    // detector window.
    let on_ring = ring.frames_after(0);
    assert_eq!(
        on_ring.len(),
        1,
        "the write's own frame must land under the flock, not with the caller"
    );
    assert_eq!(on_ring[0].delta.seq, frame.delta.seq);

    // The racing detector: a full cycle now (`prime` runs the same cycle as
    // `detect`, cadence ignored). tip_root == disk_root ⇒ silent sync.
    ring.prime(&ws_root).expect("racing cycle");
    let drained = ring.frames_after(0);
    assert_eq!(
        drained.len(),
        1,
        "one change, one frame — the racing cycle re-told nothing"
    );
    let told = &drained[0];
    assert_eq!(
        told.delta.actor.as_deref(),
        Some("alice"),
        "the one telling is the attributed one — an actor-absent duplicate is \
         the §7.1 external re-tell"
    );
    assert!(
        told.delta.now.is_some(),
        "the attributed frame keeps its now"
    );

    // And the cycle after the race stays quiet too.
    ring.prime(&ws_root).expect("settled cycle");
    assert_eq!(
        ring.frames_after(0).len(),
        1,
        "settled: no further telling of a change already told"
    );
}
