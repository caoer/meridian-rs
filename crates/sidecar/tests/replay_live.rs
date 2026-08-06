//! D4-DELTAS-LIVE: replay ≡ live (§7.3). S0→S1→S2 via real commit path;
//! `(R0, R2)` batches byte-identical to live Deltas. Values engine-derived.

use serde_json::{Value, json};
use std::io::Write as _;

/// Epoch ring as allocator (production seam; real commit path).
struct EpochSink<'a>(&'a sidecar::ring::RootRing);

impl wire_serve::seq::SeqSink for EpochSink<'_> {
    fn allocate(
        &self,
        _before: &wire::Root,
        _after: &wire::Root,
        _files: &[wire::DeltaFile],
    ) -> u64 {
        self.0.allocate_seq()
    }
}

/// Write lock for `commit_batch` allocation witness (must outlive the call).
fn flock(root: &fs::WorkspaceRoot) -> fs::WriteLock {
    fs::WriteLock::acquire(root).expect("the test workspace's write lock is free")
}

const R0: &str = "b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9";
const R2: &str = "b3:83b4ba591c0291d9f2a05428cac38e5820858fbb9c47720ab352344ddccc8f68";

/// §6.3 frozen receipt lines (block-leaf bytes, no terminator — the appender
/// carries its own `\n`, §4.4 `at:end` raw-concatenation law; also pinned in
/// `crates/receipt` and `testsuite/tests/delta_e3e4.rs`).
const E3_LINE: &str = "- splice notes/plan.md id=42 actor=agent:b0864fb2 now=2026-07-18T20:31:04Z root_before=b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9 edits=1 Goals>Q3 match 33d5b0e1b27cb48b->41f643f034e5681f ^r-000042";
const E4_LINE: &str = "- splice notes/plan.md id=57 actor=agent:b0864fb2 now=2026-07-18T20:33:41Z root_before=b3:10769ae1c77f5646750f3f52df2d055156b411145a02b8361ecd32af1357a1b7 edits=1 Goals>Q4 put:end 4b8bc385a58da0e0->f43203a1f0b4c9a3 ^r-000043";

fn wsfix(rel: &str) -> String {
    std::fs::read_to_string(testsuite::wsfix_dir().join(rel))
        .unwrap_or_else(|e| panic!("wsfix fixture {rel}: {e}"))
}

/// The committed §0.3 S0 workspace — the state the frozen E3 request applies
/// to. The dot-ignored md proves the domain filter feeds the folds.
fn s0() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    for (rel, bytes) in [
        ("notes/plan.md", wsfix("s0/notes/plan.md")),
        ("receipts/2026-07-18.md", wsfix("s0/receipts/2026-07-18.md")),
        (".github/README.md", "# CI notes\n".to_string()),
    ] {
        let abs = dir.path().join(rel);
        std::fs::create_dir_all(abs.parent().expect("parent")).expect("mkdir");
        let mut f = std::fs::File::create(&abs).expect("create");
        f.write_all(bytes.as_bytes()).expect("write");
    }
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    (dir, root)
}

fn hpath(segs: &[&str]) -> model::Ref {
    model::Ref::Hpath(
        segs.iter()
            .map(|h| model::HpathSeg {
                h: (*h).to_string(),
                n: None,
            })
            .collect(),
    )
}

/// The frozen §4.4 E3 worked request: guarded match edit inside Goals>Q3 +
/// the §6.3 receipt append riding the same sealed batch (receipt file at S0
/// is 26 bytes; the append lands at its EOF).
fn e3_request() -> wire_serve::write::CommitRequest {
    wire_serve::write::CommitRequest {
        content_path: "notes/plan.md".into(),
        batch: model::SpliceRequest {
            if_root: Some(model::MerkleRoot(R0.into())),
            edits: vec![model::Edit {
                target: hpath(&["Goals", "Q3"]),
                edit: model::EditKind::Match {
                    old: "ship by August".into(),
                    new: "ship by September".into(),
                },
                if_node_rev: Some(model::NodeRev("33d5b0e1b27cb48b".into())),
            }],
            engine: None,
        },
        receipt: Some((
            "receipts/2026-07-18.md".into(),
            model::ReceiptAppend {
                span: 26..26,
                text: format!("{E3_LINE}\n"),
            },
        )),
        actor: Some("agent:b0864fb2".into()),
        now: Some("2026-07-18T20:31:04Z".into()),
    }
}

/// The frozen §4.4 E4 worked request: guardless-node `put at:end` on Goals>Q4
/// (S1 plan ends in a terminator, so the appended item is exactly
/// `- new item\n`) + the E4 receipt append at the S1 receipt file's EOF
/// (249 bytes).
fn e4_request() -> wire_serve::write::CommitRequest {
    wire_serve::write::CommitRequest {
        content_path: "notes/plan.md".into(),
        batch: model::SpliceRequest {
            if_root: None,
            edits: vec![model::Edit {
                target: hpath(&["Goals", "Q4"]),
                edit: model::EditKind::Put {
                    at: model::PutAt::End,
                    text: "- new item\n".into(),
                },
                if_node_rev: Some(model::NodeRev("4b8bc385a58da0e0".into())),
            }],
            engine: None,
        },
        receipt: Some((
            "receipts/2026-07-18.md".into(),
            model::ReceiptAppend {
                span: 249..249,
                text: format!("{E4_LINE}\n"),
            },
        )),
        actor: Some("agent:b0864fb2".into()),
        now: Some("2026-07-18T20:33:41Z".into()),
    }
}

/// The §7.1 printed E3 frame, verbatim (the Flag-A comparison target).
fn e3_printed() -> Value {
    json!({"delta":{
     "seq":1,
     "root_before":R0,
     "root_after":"b3:10769ae1c77f5646750f3f52df2d055156b411145a02b8361ecd32af1357a1b7",
     "actor":"agent:b0864fb2","now":"2026-07-18T20:31:04Z",
     "files":[
      {"path":"notes/plan.md","change":"modified",
       "file_rev_before":"e3c4acaceb75b907","file_rev_after":"a9794a262e67ed02",
       "nodes":[{"hpath":[{"h":"Goals"},{"h":"Q3"}],"change":"edited",
                 "node_rev_before":"33d5b0e1b27cb48b","node_rev_after":"41f643f034e5681f",
                 "span_after":[49,75]}]},
      {"path":"receipts/2026-07-18.md","change":"modified",
       "file_rev_before":"920a40c4ee23d37c","file_rev_after":"2731acfa39bbb92c",
       "nodes":[{"anchor":"r-000042","change":"added",
                 "node_rev_after":"639a2dca46f6fcc8","span_after":[26,248]}]}]}})
}

/// The §7.1 printed E4 frame, verbatim.
fn e4_printed() -> Value {
    json!({"delta":{
     "seq":2,
     "root_before":"b3:10769ae1c77f5646750f3f52df2d055156b411145a02b8361ecd32af1357a1b7",
     "root_after":R2,
     "actor":"agent:b0864fb2","now":"2026-07-18T20:33:41Z",
     "files":[
      {"path":"notes/plan.md","change":"modified",
       "file_rev_before":"a9794a262e67ed02","file_rev_after":"5f27a2814b517680",
       "nodes":[{"hpath":[{"h":"Goals"},{"h":"Q4"}],"change":"edited",
                 "node_rev_before":"4b8bc385a58da0e0","node_rev_after":"f43203a1f0b4c9a3",
                 "span_after":[75,150]}]},
      {"path":"receipts/2026-07-18.md","change":"modified",
       "file_rev_before":"2731acfa39bbb92c","file_rev_after":"9167b12b0eb13be6",
       "nodes":[{"anchor":"r-000043","change":"added",
                 "node_rev_after":"c912d4578883f288","span_after":[249,473]}]}]}})
}

/// THE NAMED TEST (gates 1–4): live emission through the real commit path,
/// printed-frame equality (Flag-A STOP on any byte), replay ≡ live over the
/// ring, and the single-constructor byte-identity check.
#[test]
fn replay_equals_live_e3_e4_through_the_commit_path() {
    let (_d, root) = s0();
    let mut ring = sidecar::ring::RootRing::new();

    // LIVE: two batches through validate → apply_batch → emit. `commit_batch`
    // returns the frame (seq = the passed ring seq + 1); the CALLER advances its
    // epoch ring — the seam holds no ring, so the daemon commits without one.
    let lock = flock(&root);
    let live_e3 = wire_serve::write::commit_batch(Some(&EpochSink(&ring)), &lock, &e3_request())
        .expect("E3 commits: frozen worked request against committed S0 bytes");
    ring.advance(live_e3.clone());
    let live_e4 = wire_serve::write::commit_batch(Some(&EpochSink(&ring)), &lock, &e4_request())
        .expect("E4 commits: frozen worked request against derived S1");
    ring.advance(live_e4.clone());

    // Gate 1a (Flag-A): the DERIVED frames equal the printed §7.1 frames
    // value-for-value — roots via the real fold, revs/spans via the real
    // parse, seq from the epoch ring. Any disagreement = the STOP condition.
    assert_eq!(serde_json::to_value(&live_e3).unwrap(), e3_printed(), "E3");
    assert_eq!(serde_json::to_value(&live_e4).unwrap(), e4_printed(), "E4");

    // Gate 1b (replay ≡ live): the ring's replay range (R0, R2) — exactly
    // what the diff arm serves (D3 pinned diff_op ≡ batches_between) — is
    // the two live frames.
    let current = wire::Root(R2.into());
    let replay = ring
        .batches_between(&wire::Root(R0.into()), &current, &current)
        .expect("R0→R2 is inside this epoch's retained history");
    assert_eq!(replay, vec![live_e3.clone(), live_e4.clone()]);

    // Gate 4 (single constructor, byte-level): the replayed objects
    // serialize to the SAME BYTES as the live emissions — §7.3
    // "byte-identical", literal.
    for (replayed, live) in replay.iter().zip([&live_e3, &live_e4]) {
        assert_eq!(
            serde_json::to_string(replayed).unwrap(),
            serde_json::to_string(live).unwrap(),
            "replay serves the stored emission object, byte-identical"
        );
    }

    // Post-state cross-check: the workspace landed on the committed s2 plan
    // fixture byte-identically (derivation ≡ committed bytes, §0.3).
    let plan_after = std::fs::read_to_string(root.0.join("notes/plan.md")).unwrap();
    assert_eq!(
        plan_after,
        wsfix("s2/notes/plan.md"),
        "derived S2 ≡ committed"
    );
    let receipts_after = std::fs::read_to_string(root.0.join("receipts/2026-07-18.md")).unwrap();
    assert_eq!(receipts_after.len(), 474, "S2 receipts bytes (§0.3)");
}

/// Gate 2, named: the receipt node appears in its file's Delta entry with the
/// anchor-grain worked values computed — `[26,248]`/`639a2dca46f6fcc8` is the
/// host-block-leaf grain, derived by the real parse of the just-committed
/// bytes.
#[test]
fn receipt_node_rides_its_files_delta_entry_computed() {
    let (_d, root) = s0();
    // A fresh epoch's seq is 0; this test inspects the returned frame only.
    let live = wire_serve::write::commit_batch(None, &flock(&root), &e3_request()).expect("E3");
    let receipts_entry = live
        .delta
        .files
        .iter()
        .find(|f| f.path.0 == "receipts/2026-07-18.md")
        .expect("receipt file has its own Delta entry");
    assert_eq!(receipts_entry.nodes.len(), 1);
    let node = &receipts_entry.nodes[0];
    assert_eq!(
        node.target,
        wire::SecRef::Anchor {
            anchor: "r-000042".into()
        }
    );
    assert_eq!(node.change, wire::NodeChange::Added);
    assert_eq!(node.node_rev_before, None, "added: no before rev (§7.1)");
    assert_eq!(
        node.node_rev_after,
        Some(wire::NodeRev("639a2dca46f6fcc8".into()))
    );
    assert_eq!(node.span_after, Some(wire::Span(26, 248)));
}

/// Gate 3, named (D-C7): node entries name the DEEPEST section containing
/// each changed range — ancestors are never duplicated into the delta. E3
/// edits inside Goals>Q3: exactly one node entry on the plan file, and no
/// entry names the ancestor `Goals` alone.
#[test]
fn deepest_changed_node_never_duplicates_ancestors() {
    let (_d, root) = s0();
    let live = wire_serve::write::commit_batch(None, &flock(&root), &e3_request()).expect("E3");
    let plan_entry = live
        .delta
        .files
        .iter()
        .find(|f| f.path.0 == "notes/plan.md")
        .expect("plan entry");
    assert_eq!(plan_entry.nodes.len(), 1, "one deepest entry, no ancestors");
    let goals_alone = wire::SecRef::Hpath {
        hpath: vec![wire::HpathSeg {
            h: "Goals".into(),
            n: None,
        }],
    };
    assert!(
        plan_entry.nodes.iter().all(|n| n.target != goals_alone),
        "the ancestor section never appears as its own entry"
    );
}

/// A refused batch emits NOTHING (§7.1: one Delta = one COMMITTED batch):
/// a failed world guard leaves the ring unchanged and the seq unconsumed.
#[test]
fn refused_batch_emits_no_delta() {
    let (_d, root) = s0();
    let ring = sidecar::ring::RootRing::new();
    let mut req = e3_request();
    req.batch.if_root = Some(model::MerkleRoot(R2.into())); // wrong world
    let err = wire_serve::write::commit_batch(Some(&EpochSink(&ring)), &flock(&root), &req);
    assert!(
        matches!(
            err,
            Err(wire_serve::write::CommitError::Refused(
                model::SpliceVerdict::RootMismatch { .. }
            ))
        ),
        "world guard refuses before any byte"
    );
    // A refused commit returns Err — no frame to advance the ring with, so the
    // caller's epoch seq stays unconsumed (§7.1: one Delta = one COMMITTED batch).
    assert_eq!(ring.seq(), 0, "no emission, no seq consumption");
    let plan = std::fs::read_to_string(root.0.join("notes/plan.md")).unwrap();
    assert_eq!(plan, wsfix("s0/notes/plan.md"), "no byte reached disk");
}

// ═══════════════════════════════════════════════════════════════════════════
// The flock witness proves THIS workspace's lock — the guard, exercised in the
// condition that made it necessary
// ═══════════════════════════════════════════════════════════════════════════

/// Two live workspaces in one process, each committing a real frame through
/// its own lock, and neither touching the other's bytes.
///
/// `commit_batch` takes the workspace FROM the lock, so a mismatched
/// root/lock pair is unrepresentable — there is no "mismatch refuses" arm
/// because the mismatch cannot be spelled. Two roots and two locks are live
/// simultaneously, reproducing the registry daemon's production condition
/// (fresh `WorkspaceRoot` per call), and each commit lands in the workspace
/// its own lock names.
#[test]
fn two_live_workspaces_each_commit_through_their_own_lock() {
    let (_da, root_a) = s0();
    let (_db, root_b) = s0();
    assert_ne!(root_a.0, root_b.0, "two DISTINCT workspaces, one process");

    let was_a = std::fs::read_to_string(root_a.0.join("notes/plan.md")).expect("A pre");
    let was_b = std::fs::read_to_string(root_b.0.join("notes/plan.md")).expect("B pre");
    assert_eq!(
        was_a, was_b,
        "the two workspaces start byte-identical, so any later difference is \
         attributable to which one was written"
    );

    // Both locks held at once.
    let lock_a = flock(&root_a);
    let lock_b = flock(&root_b);

    let frame_a = wire_serve::write::commit_batch(None, &lock_a, &e3_request()).expect("A commits");
    let frame_b = wire_serve::write::commit_batch(None, &lock_b, &e3_request()).expect("B commits");

    // The decisive assertion runs first: a precondition failing first would
    // shadow the claim the test is named for.
    let now_a = std::fs::read_to_string(root_a.0.join("notes/plan.md")).expect("A post");
    let now_b = std::fs::read_to_string(root_b.0.join("notes/plan.md")).expect("B post");
    assert_ne!(
        now_a, was_a,
        "A's commit changed A — the write reached the workspace A's lock names"
    );
    assert_ne!(
        now_b, was_b,
        "B's commit changed B — the write reached the workspace B's lock names"
    );

    // Real frames, not empty ones: each carries the edited file with node-grain
    // detail computed by the engine's own parse of the just-written bytes.
    for (label, frame) in [("A", &frame_a), ("B", &frame_b)] {
        let plan = frame
            .delta
            .files
            .iter()
            .find(|f| f.path.0 == "notes/plan.md")
            .unwrap_or_else(|| panic!("{label}: the edited file has a Delta entry"));
        assert!(
            !plan.nodes.is_empty(),
            "{label}: the frame carries real node-grain detail, not an empty shell"
        );
        assert_ne!(
            frame.delta.root_before, frame.delta.root_after,
            "{label}: a real commit advances the root"
        );
    }
}
