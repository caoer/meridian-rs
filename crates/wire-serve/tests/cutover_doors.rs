//! The cutover's LIVE-FIXTURE door proof — three errors, three facts,
//! through the REAL doors (price paper §11 gates 3/4/5/10/13; card quality
//! gate 2): a real workspace, a real B-04 record, and the splice door
//! classifying a held token's family BEFORE its digest compare.
//!
//! Model-level exhaustive proof lives in
//! `crates/testsuite/tests/version_identity.rs`; this file is the door half
//! the advisor bound in writing: model coverage is to door coverage what
//! CI-green is to the sync class — both real, neither substituting.

use std::sync::{Arc, Mutex};

use wire::{Edit, EditShape, ErrorCode, HpathSeg, Root, SecRef};
use wire_serve::write::{Mints, SpliceArgs, splice, splice_with_mints};

fn ws() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    (dir, root)
}

fn page(dir: &tempfile::TempDir, rel: &str, word: &str) {
    let abs = dir.path().join(rel);
    std::fs::create_dir_all(abs.parent().expect("parent")).expect("mkdir");
    std::fs::write(abs, format!("# Alpha\n\n## Beta\n\nship by {word}\n")).expect("write");
}

fn edit(old: &str, new: &str) -> Edit {
    Edit {
        target: SecRef::Hpath {
            hpath: vec![
                HpathSeg {
                    h: "Alpha".into(),
                    n: None,
                },
                HpathSeg {
                    h: "Beta".into(),
                    n: None,
                },
            ],
        },
        edit: EditShape::Match {
            old: old.into(),
            new: new.into(),
        },
        if_node_rev: None,
    }
}

fn args(if_root: Option<Root>, old: &str, new: &str) -> SpliceArgs {
    SpliceArgs {
        id: None,
        path: wire::Path("notes/plan.md".into()),
        origin: wire_serve::guard::Origin::Wire,
        actor: None,
        now: None,
        receipt: None,
        if_root,
        dry: false,
        force: false,
        edits: vec![edit(old, new)],
        plan_edits: Vec::new(),
        pin: None,
    }
}

/// The same wire request a real client sends AFTER its read: the world guard
/// plus the node-grain token the § A.1 wire-origin demand requires for any
/// edit that changes existing content (`guard.rs::guard_batch` — landed law,
/// unrelated to the cutover). A fixture that omitted it would be driving the
/// door in a shape production never uses.
fn guarded(root: &fs::WorkspaceRoot, if_root: Option<Root>, old: &str, new: &str) -> SpliceArgs {
    let doc = fs::load(root, std::path::Path::new("notes/plan.md")).expect("read the page");
    let target = model::resolve(
        &doc,
        &model::Ref::Hpath(vec![
            model::HpathSeg {
                h: "Alpha".into(),
                n: None,
            },
            model::HpathSeg {
                h: "Beta".into(),
                n: None,
            },
        ]),
    )
    .expect("the section resolves");
    let mut args = args(if_root, old, new);
    args.edits[0].if_node_rev = Some(wire::NodeRev(target.node_rev.0));
    args
}

/// Cross the boundary the way the machine's durable acts do: the record is
/// the authority (merkle-spec §4.2.6); the registry machine is orchestration
/// around exactly these two writes.
fn cross_boundary(root: &fs::WorkspaceRoot) -> String {
    let record = fs::cutover::CutoverRecord {
        state: fs::cutover::CutoverState::NewBuilding,
        cutover_id: fs::cutover::mint_cutover_id(),
        from: model::RootVersion::law1(0),
        to: model::RootVersion::law2(0),
        old_instance: None,
        started_unix_ms: 1,
        committed_unix_ms: None,
    };
    fs::cutover::begin_new_building(root, &record).expect("begin");
    let mut done = record.clone();
    done.state = fs::cutover::CutoverState::NewCommitted;
    done.committed_unix_ms = Some(2);
    fs::cutover::commit(root, &done).expect("commit");
    record.cutover_id
}

#[test]
fn three_errors_three_facts_through_the_real_splice_door() {
    let (dir, root) = ws();
    page(&dir, "notes/plan.md", "friday");

    // Pre-cutover: the served token is law 1 and the door commits under it.
    let root0 = wire_serve::ambient_root(&root).expect("root0");
    assert!(
        root0.0.starts_with("b3:"),
        "pre-cutover serves law 1: {}",
        root0.0
    );
    splice(
        &root,
        None,
        &guarded(&root, Some(root0.clone()), "friday", "monday"),
        &[],
        None,
    )
    .expect("pre-cutover guarded commit");
    let root1 = wire_serve::ambient_root(&root).expect("root1");
    assert!(root1.0.starts_with("b3:"));
    assert_ne!(root0.0, root1.0, "the commit moved the root");

    // The boundary: durable record, no fence, no shadow.
    let cutover_id = cross_boundary(&root);

    // Past it the door serves law 2 — the radix fingerprint under the next
    // ladder prefix, minted by the same door path (gate 10's one boundary).
    let served = wire_serve::ambient_root(&root).expect("served");
    assert!(
        served.0.starts_with("b3a:"),
        "post-cutover serves law 2: {}",
        served.0
    );

    // FACT 1 — retired: BOTH held old-law tokens refuse retirement, the one
    // whose digest was current at the boundary (root1) and the stale one
    // (root0): family classification precedes the digest compare, so
    // "retired AND would-also-mismatch" still answers retired.
    for held in [&root1, &root0] {
        let refusal = splice(
            &root,
            None,
            &args(Some(held.clone()), "monday", "never"),
            &[],
            None,
        )
        .expect_err("a held old-law token refuses");
        assert_eq!(refusal.code, ErrorCode::FingerprintVersionRetired);
        assert_eq!(refusal.recovery, wire::Recovery::Resync);
        let received = refusal
            .received_fingerprint_version
            .expect("gate 4: received");
        assert_eq!((received.hash_law, received.domain), (1, 0));
        let current = refusal
            .current_fingerprint_version
            .expect("gate 4: current");
        assert_eq!((current.hash_law, current.domain), (2, 0));
        assert_eq!(refusal.cutover_id.as_deref(), Some(cutover_id.as_str()));
    }

    // FACT 2 — unknown FUTURE family: its own refusal and teaching, never a
    // fallthrough to retired. Position 2 (`b3b:`) is past the serving
    // position 1.
    let future = Root(format!("b3b:{}", "0".repeat(64)));
    let refusal = splice(
        &root,
        None,
        &args(Some(future), "monday", "never"),
        &[],
        None,
    )
    .expect_err("an unknown future family refuses distinctly");
    assert_eq!(refusal.code, ErrorCode::FingerprintVersionUnsupported);
    assert_eq!(refusal.recovery, wire::Recovery::Resync);
    assert!(
        refusal.received_fingerprint_version.is_none(),
        "a future family has no decodable identity here"
    );

    // FACT 3 — current family, unequal digest: the ordinary mismatch, with
    // the version refusals nowhere in sight.
    let mut wrong = served.0.clone();
    let flip = if wrong.ends_with('0') { "1" } else { "0" };
    wrong.replace_range(wrong.len() - 1.., flip);
    let refusal = splice(
        &root,
        None,
        &args(Some(Root(wrong)), "monday", "never"),
        &[],
        None,
    )
    .expect_err("a current-family unequal digest is the ordinary mismatch");
    assert_eq!(refusal.code, ErrorCode::RootMismatch);

    // And the CURRENT token commits: the door serves law 2 end to end.
    splice(
        &root,
        None,
        &guarded(&root, Some(served.clone()), "monday", "tuesday"),
        &[],
        None,
    )
    .expect("post-cutover guarded commit under the law-2 token");
    let after = wire_serve::ambient_root(&root).expect("after");
    assert!(after.0.starts_with("b3a:"));
    assert_ne!(after.0, served.0);
}

/// Gate 5: dry and real calls return byte-equivalent retirement verdicts at
/// the same generation.
#[test]
fn dry_and_real_retirement_verdicts_are_byte_equal() {
    let (dir, root) = ws();
    page(&dir, "notes/plan.md", "friday");
    let old = wire_serve::ambient_root(&root).expect("old");
    cross_boundary(&root);

    let mut dry_args = args(Some(old.clone()), "friday", "never");
    dry_args.dry = true;
    let dry_err = splice(&root, None, &dry_args, &[], None).expect_err("dry refuses");
    let real_err = splice(&root, None, &args(Some(old), "friday", "never"), &[], None)
        .expect_err("real refuses");
    assert_eq!(
        serde_json::to_string(&dry_err).expect("json"),
        serde_json::to_string(&real_err).expect("json"),
        "gate 5: the dry rehearsal and the real call teach identical bytes"
    );
}

/// Gate 13's door half: past the boundary the served mint is the resident
/// radix fingerprint — zero law-1 flat folds on the serving path.
#[test]
fn post_cutover_serving_pays_zero_flat_folds() {
    let (dir, root) = ws();
    page(&dir, "notes/plan.md", "friday");
    cross_boundary(&root);

    let cache = Arc::new(Mutex::new(fs::DomainCache::new()));
    // The in-process origin needs no guard token; the door still observes,
    // serves `root_before`, and overlays `root_after` through this cache.
    let mut in_process = args(None, "friday", "monday");
    in_process.origin = wire_serve::guard::Origin::InProcess;
    splice_with_mints(
        &root,
        None,
        &in_process,
        &[],
        Mints {
            ambient: None,
            foreign: None,
        },
        Some(&cache),
    )
    .expect("in-process commit");
    let folds = cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .flat_folds();
    assert_eq!(
        folds, 0,
        "gate 13: the law-2 serving path computes no law-1 flat fold"
    );
}
