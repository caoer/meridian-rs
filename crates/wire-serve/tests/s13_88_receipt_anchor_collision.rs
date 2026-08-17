//! §6.6 at the splice door: the requested receipt anchor is resolved BEFORE
//! anything is written, so a collision refuses byte-untouched.
//!
//! The law is `wire-contract.md` §6.6 — "an anchor MUST be unique within the
//! receipt file it names" — and the kit encodes it for a host that re-mints a
//! colliding anchor (`s13-88-receipt-anchor-collision`). These gates are the
//! same law with the ENGINE as the offender, on a path where the engine also
//! commits: it used to commit both files, mint the duplicate, fail to resolve
//! its own anchor, and report "receipt corrupt" — a refusal that had already
//! written, whose taught `fix` then appended the caller's content twice.
//!
//! The assertions are on BYTES, not only on the exit shape: an error frame
//! says what the engine decided, and the defect was that the decision came
//! after the write.

use std::collections::BTreeMap;
use wire::{Edit, EditShape, NodeRev, Path as WPath, PutAt, ReceiptAddr, ResponseBody, SecRef};
use wire_serve::write::{SpliceArgs, splice};

const PLAN: &str = "# Goals\n\n## Q4\n\n- item one\n";
const RECEIPTS: &str = "# Receipts\n\n- seed entry ^r-dup\n";

/// Temp workspace carrying the plan and (unless `receipts` is `None`) the
/// receipt file the anchor collides in.
fn ws(receipts: Option<&str>) -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("plan.md"), PLAN).expect("write plan");
    if let Some(body) = receipts {
        std::fs::write(dir.path().join("receipts.md"), body).expect("write receipts");
    }
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    (dir, root)
}

fn read(root: &fs::WorkspaceRoot, rel: &str) -> String {
    std::fs::read_to_string(root.0.join(rel)).expect("read")
}

/// One append under `Goals › Q4`, receipted at `receipts.md#^anchor`.
/// A stale token: well-formed, and not any node's. Same value and same reason
/// as `u11_mismatch_ladder.rs`, which pins that it reaches `cas_mismatch`.
const STALE_REV: &str = "0000000000000000";

/// The six original cases send an unguarded edit; `args_guarded` is the same
/// request with a node-grain guard attached, so one request can carry BOTH a
/// colliding anchor and a stale guard.
fn args(anchor: &str, dry: bool) -> SpliceArgs {
    args_guarded(anchor, dry, None)
}

fn args_guarded(anchor: &str, dry: bool, if_node_rev: Option<NodeRev>) -> SpliceArgs {
    SpliceArgs {
        premises: Vec::new(),
        id: Some(1),
        origin: wire_serve::guard::Origin::InProcess,
        path: WPath("plan.md".to_string()),
        actor: Some("alice".to_string()),
        now: Some("2026-08-09T12:00:00Z".to_string()),
        receipt: Some(ReceiptAddr {
            path: WPath("receipts.md".to_string()),
            anchor: anchor.to_string(),
        }),
        if_root: None,
        dry,
        force: false,
        edits: vec![Edit {
            target: SecRef::Hpath {
                hpath: vec![
                    wire::HpathSeg {
                        h: "Goals".to_string(),
                        n: None,
                    },
                    wire::HpathSeg {
                        h: "Q4".to_string(),
                        n: None,
                    },
                ],
            },
            edit: EditShape::Put {
                at: PutAt::End,
                text: "\n- new item".to_string(),
            },
            if_node_rev,
        }],
        plan_edits: Vec::new(),
        pin: None,
        fields: BTreeMap::default(),
    }
}

/// Clause 1 + 2: an anchor the receipt file already carries refuses, and NEITHER
/// file moves a byte — so there is no commit for the armed plane to go silent
/// over.
#[test]
fn a_colliding_receipt_anchor_refuses_with_both_files_byte_untouched() {
    let (_d, root) = ws(Some(RECEIPTS));

    let err = splice(&root, None, &args("r-dup", false), &[], None)
        .expect_err("a colliding receipt anchor refuses");

    assert_eq!(err.code, wire::ErrorCode::BadRequest);
    assert_eq!(err.recovery, wire::Recovery::Fix);
    let msg = err.message.as_deref().expect("the refusal teaches");
    assert!(
        msg.contains("^r-dup") && msg.contains("receipts.md") && msg.contains("§6.6"),
        "the refusal names the anchor, the file and the law it enforces: {msg}"
    );

    assert_eq!(
        read(&root, "plan.md"),
        PLAN,
        "the content file is untouched"
    );
    assert_eq!(
        read(&root, "receipts.md"),
        RECEIPTS,
        "the receipt file is untouched — no duplicate anchor was minted"
    );
    assert_eq!(
        read(&root, "receipts.md").matches("^r-dup").count(),
        1,
        "the file still carries the anchor exactly once"
    );
}

/// Clause 3: the remedy the refusal teaches is safe to follow. A caller that
/// fixes the anchor and re-sends lands its content EXACTLY ONCE — the doubled
/// append was the consequence of the refusal having already committed.
#[test]
fn the_taught_recovery_lands_the_caller_content_exactly_once() {
    let (_d, root) = ws(Some(RECEIPTS));

    splice(&root, None, &args("r-dup", false), &[], None).expect_err("refuses");
    splice(&root, None, &args("r-fresh", false), &[], None).expect("the re-send commits");

    assert_eq!(
        read(&root, "plan.md").matches("- new item").count(),
        1,
        "following recovery:\"fix\" appends the caller's content once, never twice"
    );
    assert_eq!(
        read(&root, "receipts.md").matches("^r-fresh").count(),
        1,
        "the re-send's receipt is minted once under its own anchor"
    );
}

/// Fires-check: the fixture is not refusing for some unrelated reason. The same
/// batch under a free anchor commits and writes both files — so the refusal
/// above is attributable to the anchor and to nothing else.
#[test]
fn the_same_batch_under_a_free_anchor_commits() {
    let (_d, root) = ws(Some(RECEIPTS));

    let outcome = splice(&root, None, &args("r-free", false), &[], None).expect("commits");
    let ResponseBody::Splice { receipt, .. } = &outcome.body else {
        panic!("a splice returns a Splice body");
    };
    let fact = receipt
        .as_ref()
        .expect("a receipted splice reports its fact");
    assert_eq!(fact.anchor, "r-free");
    assert!(read(&root, "plan.md").contains("- new item"));
    assert!(read(&root, "receipts.md").contains("^r-free"));
}

/// A rehearsal refuses exactly where the real write would (§4.4): the dry path
/// carries the same pre-flight, so a caller cannot rehearse green and land red.
#[test]
fn a_dry_run_refuses_the_collision_too() {
    let (_d, root) = ws(Some(RECEIPTS));

    let err =
        splice(&root, None, &args("r-dup", true), &[], None).expect_err("the rehearsal refuses");
    assert_eq!(err.code, wire::ErrorCode::BadRequest);
    assert_eq!(read(&root, "receipts.md"), RECEIPTS);
}

/// A receipt file that does not exist yet cannot collide — the append births
/// it. The pre-flight must not turn the first receipt of a workspace into a
/// refusal.
#[test]
fn an_absent_receipt_file_is_born_by_the_append() {
    let (_d, root) = ws(None);

    splice(&root, None, &args("r-first", false), &[], None).expect("the first receipt commits");
    assert!(read(&root, "receipts.md").contains("^r-first"));
}

/// An anchor outside the block-id charset (§2.4) refuses at the same pre-flight
/// rung, byte-untouched — the mint-guard used to answer after the commit too.
#[test]
fn an_anchor_outside_the_block_id_charset_refuses_byte_untouched() {
    let (_d, root) = ws(Some(RECEIPTS));

    let err = splice(&root, None, &args("r_underscore", false), &[], None)
        .expect_err("the mint-guard refuses");
    assert_eq!(err.code, wire::ErrorCode::BadRequest);
    assert_eq!(read(&root, "plan.md"), PLAN);
    assert_eq!(read(&root, "receipts.md"), RECEIPTS);
}

/// §6.6 vs §5.1 — THE PRECEDENCE, which the contract declares in prose and no
/// test held until this one.
///
/// §6.6 says the receipt anchor is resolved **FIRST**. That is a claim about
/// ordering against the OTHER guards, and the six cases above cannot see it:
/// every one of them sends `if_node_rev: None`, so nothing else is competing to
/// refuse. Until this case, the only place in the workspace where the ordering
/// was observable was `crates/registry/tests/v3_key_set_pins.rs:606` — BY
/// ACCIDENT, and it read the ordering as a failure. Giving that fixture its own
/// anchor (the sibling half of this change) removes that accidental witness, so
/// the ordering would have been left unheld by anything.
///
/// ⛔ THE CONTROL ARM IS LOAD-BEARING, NOT DECORATION. `bad_request` is also
/// what a MALFORMED rev would earn, so asserting it alone would pass whether or
/// not the anchor won — the assertion would be vacuous and would still be green
/// if the precedence inverted tomorrow. The free-anchor arm proves this exact
/// rev genuinely reaches the CAS and refuses `cas_mismatch`, so the collision
/// arm's `bad_request` can only be the anchor arriving first.
#[test]
fn a_colliding_anchor_outranks_a_stale_node_guard_in_the_same_request() {
    let stale = Some(NodeRev(STALE_REV.to_string()));

    // CONTROL — free anchor, same stale guard: the guard IS reached and refuses
    // at node grain. Without this, the assertion below proves nothing.
    let (_dc, root_c) = ws(Some(RECEIPTS));
    let err_c = splice(
        &root_c,
        None,
        &args_guarded("r-free", false, stale.clone()),
        &[],
        None,
    )
    .expect_err("a stale node guard refuses");
    assert_eq!(
        err_c.code,
        wire::ErrorCode::CasMismatch,
        "CONTROL: under a FREE anchor this rev must reach the node-grain guard, \
         or the collision arm below is vacuous: {:?}",
        err_c.message
    );

    // SUBJECT — same stale guard, but the anchor collides. §6.6 must win.
    let (_d, root) = ws(Some(RECEIPTS));
    let before_plan = read(&root, "plan.md");
    let before_receipts = read(&root, "receipts.md");

    let err = splice(&root, None, &args_guarded("r-dup", false, stale), &[], None)
        .expect_err("a colliding receipt anchor refuses");

    assert_eq!(
        err.code,
        wire::ErrorCode::BadRequest,
        "§6.6 resolves the anchor FIRST, so the collision outranks the stale \
         node guard — cas_mismatch here means the preflight moved after the CAS: {:?}",
        err.message
    );
    assert_eq!(err.recovery, wire::Recovery::Fix);
    let msg = err.message.as_deref().expect("the refusal teaches");
    assert!(
        msg.contains("^r-dup") && msg.contains("§6.6"),
        "the refusal names the anchor and the law it enforces: {msg}"
    );

    // Same discipline as the rest of this file: the decision is pinned on BYTES,
    // because the defect this suite exists for was a refusal that had written.
    assert_eq!(read(&root, "plan.md"), before_plan, "plan.md moved a byte");
    assert_eq!(
        read(&root, "receipts.md"),
        before_receipts,
        "receipts.md moved a byte"
    );
}
