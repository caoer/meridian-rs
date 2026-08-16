//! U11 gates: the **mismatch-recovery ladder** on `cas_mismatch` (R1.2).
//!
//! The law: a fingerprint mismatch NEVER instructs a whole-file read. The
//! refusal carries the richest rung it can compute — diff, else new content
//! plus the new fingerprint, else the bare floor — so the caller re-plans and
//! resends WITHOUT a read call. The extras ride the existing `cas_mismatch`
//! code with its bound `Recovery::Refresh`; no new error code exists.

use std::fmt::Write as _;
use std::path::PathBuf;

use wire::{Edit, EditShape, ErrorCode, HpathSeg, NodeRev, Path as WPath, PutAt, SecRef};
use wire_serve::guard::Origin;
use wire_serve::ladder::DIFF_CAP_BYTES;
use wire_serve::write::{SpliceArgs, splice};

/// Two sibling sections at the same level. `Secret` exists so a scope leak has
/// something to leak — see the S4 gate.
const DOC: &str = "---\nstatus: draft\nowner: d\n---\n# Memo\n\n## Public\n\nalpha\nbravo\ncharlie\n\n## Secret\n\nCLASSIFIED-XYZZY\n";

/// A stale token: well-formed, and not any node's.
const STALE: &str = "0000000000000000";

fn ws(body: &str) -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("memo.md"), body).expect("seed");
    let root = fs::WorkspaceRoot(PathBuf::from(dir.path()));
    (dir, root)
}

fn args(edits: Vec<Edit>) -> SpliceArgs {
    SpliceArgs {
        premises: Vec::new(),
        id: None,
        origin: Origin::Wire,
        path: WPath("memo.md".into()),
        actor: Some("agent:alice".into()),
        now: Some("2026-08-03T12:00:00Z".into()),
        receipt: None,
        if_root: None,
        dry: false,
        force: false,
        edits,
        plan_edits: Vec::new(),
        pin: None,
    }
}

fn seg(h: &str) -> HpathSeg {
    HpathSeg {
        h: h.into(),
        n: None,
    }
}

fn public() -> SecRef {
    SecRef::Hpath {
        hpath: vec![seg("Memo"), seg("Public")],
    }
}

fn stale_match(target: SecRef, old: &str) -> Edit {
    Edit {
        target,
        edit: EditShape::Match {
            old: old.into(),
            new: "rewritten".into(),
        },
        if_node_rev: Some(NodeRev(STALE.into())),
    }
}

/// The ladder's own refusal contract — never a loosening of the shared
/// `assert_refusal_contract`. Four properties plus one negative: the subject,
/// the cause, the partial state, a fix the caller can act on — and NEVER an
/// instruction to read the whole file (the standing law).
fn assert_ladder_contract(ctx: &str, message: &str) {
    assert!(
        message.contains("memo.md"),
        "{ctx}: names the file it is talking about: {message}"
    );
    assert!(
        message.contains("No edit was applied; the batch is refused whole."),
        "{ctx}: discloses the partial state: {message}"
    );
    assert!(
        message.contains("Fix:"),
        "{ctx}: carries a fix clause: {message}"
    );
    assert!(
        message.contains("`force`"),
        "{ctx}: names the ONE sanctioned bypass — refuse→rewrite is one path: {message}"
    );
    assert!(
        !message.contains("mrd read memo.md") && !message.contains("whole file"),
        "{ctx}: NEVER instructs a whole-file read (R1.2): {message}"
    );
    assert!(
        !message.contains("the fingerprint your write pinned"),
        "{ctx}: a NODE-grain refusal never calls the pinned token a `fingerprint` — \
         that noun is the workspace guard's (`if_fingerprint`), and borrowing it \
         steers the caller to the wrong guard slot: {message}"
    );
}

// ── Rung 1 — the change diff ────────────────────────────────────────────────

#[test]
fn rung_one_returns_a_unified_line_diff_for_a_section_body_mismatch() {
    let (_d, root) = ws(DOC);
    // The caller's own picture of the section: close to current, one line off.
    let pinned = "## Public\n\nalpha\nbravo\ndelta\n";
    let a = args(vec![stale_match(public(), pinned)]);
    let err = splice(&root, None, &a, &[], None).expect_err("refuses");

    assert_eq!(err.code, ErrorCode::CasMismatch);
    assert_eq!(err.recovery, wire::Recovery::Refresh);
    assert_eq!(err.rung, Some(1));
    assert_eq!(err.new_content, None, "rung 1 sends the diff, not the body");

    let diff = err.diff.as_deref().expect("rung 1 carries a diff");
    assert!(
        diff.starts_with("--- pinned\n+++ current\n@@ "),
        "unified line diff, sides named for what they are: {diff}"
    );
    assert!(
        diff.contains(" alpha\n") && diff.contains(" bravo\n"),
        "unchanged lines ride as context: {diff}"
    );
    assert!(
        diff.contains("-delta\n") && diff.contains("+charlie\n"),
        "the change is the caller's pinned line out, the current line in: {diff}"
    );

    // The token to resend with rides the rung the caller resends FROM.
    assert_eq!(err.new_fingerprint, err.actual);
    assert!(err.new_fingerprint.is_some());

    let message = err.message.as_deref().expect("a teaching message");
    assert_ladder_contract("rung 1", message);
    assert!(
        message.contains(
            "section \"Memo/Public\" in memo.md changed under the `if_node_rev` your \
                          write pinned."
        ),
        "names the subject and the cause, at the pin's own grain: {message}"
    );
    assert!(
        message.contains("apply the `diff` extra to your copy")
            && message.contains("no re-read is needed"),
        "the fix is the ladder's whole point — proceed without a read: {message}"
    );
}

#[test]
fn rung_one_returns_ops_form_for_a_frontmatter_mismatch() {
    let (_d, root) = ws(DOC);
    // A frontmatter key is a PROPERTY, so the caller's picture is its value.
    let a = args(vec![stale_match(
        SecRef::FmKey {
            fm_key: "status".into(),
        },
        "status: published",
    )]);
    let err = splice(&root, None, &a, &[], None).expect_err("refuses");

    assert_eq!(err.code, ErrorCode::CasMismatch);
    assert_eq!(err.rung, Some(1));
    let diff = err.diff.as_deref().expect("rung 1 carries a diff");
    assert_eq!(
        diff, "[{\"op\":\"replace\",\"path\":\"/status\",\"value\":\"draft\"}]",
        "the FRONTMATTER scope tripped, so the form is ops, not a line diff"
    );
    assert!(
        !diff.contains("--- pinned"),
        "a property map never diffs as text: {diff}"
    );
    let message = err.message.as_deref().expect("a teaching message");
    assert_ladder_contract("rung 1 frontmatter", message);
    assert!(
        message.contains("frontmatter key \"status\" in memo.md"),
        "names the subject at ITS grain: {message}"
    );
}

// ── The cap — what keeps rung 1 richer than rung 2, not merely different ────

#[test]
fn rung_one_falls_to_rung_two_when_the_diff_exceeds_the_size_cap() {
    let mut body = String::from("---\nstatus: draft\n---\n# Memo\n\n## Public\n\n");
    for i in 0..200 {
        writeln!(body, "current line {i}").expect("String write is infallible");
    }
    let (_d, root) = ws(&body);

    // A picture far enough from current that the diff is no cheaper than the
    // content — exactly the case the cap exists to catch.
    let mut pinned = String::from("## Public\n\n");
    for i in 0..200 {
        writeln!(pinned, "stale line {i}").expect("String write is infallible");
    }
    let a = args(vec![stale_match(public(), &pinned)]);
    let err = splice(&root, None, &a, &[], None).expect_err("refuses");

    assert_eq!(err.code, ErrorCode::CasMismatch, "the named refusal fired");
    assert_eq!(err.rung, Some(2), "over the cap, the ladder falls a rung");
    assert_eq!(err.diff, None, "no diff ships past the cap");
    let content = err.new_content.as_deref().expect("rung 2 carries content");
    assert!(content.contains("current line 0") && content.contains("current line 199"));
    assert_ladder_contract("cap fallback", err.message.as_deref().expect("message"));

    // Discriminator: same document, target, and edit shape; only the picture's
    // DISTANCE changes. A near picture must come back rung 1, leaving the cap
    // as the sole cause of the fallback above.
    let mut near = String::from("## Public\n\n");
    for i in 0..200 {
        let line = if i == 100 {
            "current line ONE HUNDRED".to_owned()
        } else {
            format!("current line {i}")
        };
        writeln!(near, "{line}").expect("String write is infallible");
    }
    let close = splice(
        &root,
        None,
        &args(vec![stale_match(public(), &near)]),
        &[],
        None,
    )
    .expect_err("refuses");
    assert_eq!(
        close.rung,
        Some(1),
        "a NEAR picture of the same node diffs — so the cap, not the shape, \
         caused the fallback above"
    );
    let diff = close.diff.as_deref().expect("the near picture diffs");
    assert!(
        diff.len() <= DIFF_CAP_BYTES && diff.len() < content.len(),
        "and bounded context makes rung 1 genuinely CHEAPER than rung 2 \
         ({} diff bytes vs {} content bytes)",
        diff.len(),
        content.len()
    );
    assert!(
        diff.contains("-current line ONE HUNDRED") && diff.contains("+current line 100"),
        "the hunk names the one line that moved: {diff}"
    );
    assert!(
        !diff.contains("current line 0") && !diff.contains("current line 199"),
        "and carries only its context window, not the whole node: {diff}"
    );
}

// ── S4 — scope is a security rule ──────────────────────────────────────────

#[test]
fn rung_one_discloses_nothing_outside_the_callers_targeted_section() {
    let (_d, root) = ws(DOC);
    let pinned = "## Public\n\nalpha\nbravo\ndelta\n";
    let a = args(vec![stale_match(public(), pinned)]);
    let err = splice(&root, None, &a, &[], None).expect_err("refuses");

    assert_eq!(err.code, ErrorCode::CasMismatch, "the named refusal fired");
    assert_eq!(err.rung, Some(1), "the rung under test is the rich one");

    // The WHOLE envelope, as it goes on the wire — not just the diff field, so
    // a leak cannot hide in a slot this test forgot to name.
    let wire = serde_json::to_string(&*err).expect("serialize");
    assert!(
        !wire.contains("CLASSIFIED-XYZZY"),
        "S4: a rung discloses the caller's targeted section and nothing beyond it: {wire}"
    );
    assert!(
        !wire.contains("Secret"),
        "S4: not even the neighbouring section's HEADING is the caller's to learn: {wire}"
    );
    // Positive control: the fixture really does hold the secret, so the two
    // assertions above cannot pass by an empty document.
    assert!(DOC.contains("CLASSIFIED-XYZZY"));
    assert!(
        err.diff.as_deref().expect("diff").contains("alpha"),
        "and the targeted section IS disclosed — the test is not passing by silence"
    );
}

// ── Rung 2 — the ETag-412-with-body shape ──────────────────────────────────

#[test]
fn rung_two_carries_new_content_and_the_new_fingerprint() {
    let (_d, root) = ws(DOC);
    // A `put` states what to WRITE, never what the caller believed was there —
    // so no baseline exists and rung 1 is not computable.
    let a = args(vec![Edit {
        target: public(),
        edit: EditShape::Put {
            at: PutAt::End,
            text: "delta\n".into(),
        },
        if_node_rev: Some(NodeRev(STALE.into())),
    }]);
    let err = splice(&root, None, &a, &[], None).expect_err("refuses");

    assert_eq!(err.code, ErrorCode::CasMismatch);
    assert_eq!(err.rung, Some(2));
    assert_eq!(err.diff, None);
    let content = err.new_content.as_deref().expect("rung 2 carries content");
    // The node's span, byte-exact — heading line through the section's end,
    // trailing blank included (the newline-INCLUSIVE section grain).
    assert_eq!(content, "## Public\n\nalpha\nbravo\ncharlie\n\n");
    let fingerprint = err.new_fingerprint.as_ref().expect("the token to resend");
    assert_eq!(Some(fingerprint), err.actual.as_ref());
    assert_ne!(fingerprint.0, STALE);

    let message = err.message.as_deref().expect("a teaching message");
    assert_ladder_contract("rung 2", message);
    assert!(
        message.contains("take `new_content` as that node's current bytes")
            && message.contains("resend it with `new_fingerprint` as its `if_node_rev` guard")
            && message.contains("no re-read is needed"),
        "the fix spends the rung it was given, and names the guard field the CAS \
         actually checks: {message}"
    );
}

// ── Rung 3 — the floor ─────────────────────────────────────────────────────

#[test]
fn rung_three_is_the_bare_mismatch_floor() {
    let (_d, root) = ws(DOC);
    // An upsert BIRTH: the key does not exist, so its CAS compares against a
    // synthesized insertion-point rev and there is no node whose bytes the
    // ladder could return. Nothing is computable — the floor is honest here.
    let a = args(vec![Edit {
        target: SecRef::FmKey {
            fm_key: "reviewer".into(),
        },
        edit: EditShape::Put {
            at: PutAt::Upsert,
            text: "alice".into(),
        },
        if_node_rev: Some(NodeRev(STALE.into())),
    }]);
    let err = splice(&root, None, &a, &[], None).expect_err("refuses");

    assert_eq!(err.code, ErrorCode::CasMismatch);
    assert_eq!(err.recovery, wire::Recovery::Refresh);
    assert_eq!(err.rung, Some(3));
    assert_eq!(err.diff, None);
    assert_eq!(err.new_content, None);
    assert_eq!(err.new_fingerprint, None);
    // The floor still carries the frozen comparison tokens.
    assert_eq!(err.expected.as_ref().map(|r| r.0.as_str()), Some(STALE));
    assert!(err.actual.is_some());

    let message = err.message.as_deref().expect("a teaching message");
    assert_ladder_contract("rung 3", message);
    assert!(
        message.contains("its current bytes are not computable here"),
        "the floor says WHY it is the floor: {message}"
    );
    assert!(
        message.contains("re-read the ONE node you targeted"),
        "even the floor never sends the caller to the whole file: {message}"
    );
    assert!(
        message.contains("resend with that token as `if_node_rev`"),
        "the floor names the guard field the resend must carry — a re-read that \
         stops one noun short of the recovery is not a fix clause: {message}"
    );
}

// ── The builder's choice ───────────────────────────────────────────────────

#[test]
fn the_builder_picks_the_richest_computable_rung_not_the_first_that_works() {
    let (_d, root) = ws(DOC);
    // Rung 2 is ALWAYS computable for a resolvable node — so a builder that
    // stopped at the first rung that works would answer 2 here. It answers 1.
    let pinned = "## Public\n\nalpha\nbravo\ndelta\n";
    let rich = splice(
        &root,
        None,
        &args(vec![stale_match(public(), pinned)]),
        &[],
        None,
    )
    .expect_err("refuses");
    assert_eq!(rich.code, ErrorCode::CasMismatch, "the named refusal fired");
    assert_eq!(rich.rung, Some(1));
    assert!(rich.diff.is_some() && rich.new_content.is_none());

    // And when rung 1 is genuinely not computable it stops at 2, never falling
    // through to the floor it did not need.
    let poorer = splice(
        &root,
        None,
        &args(vec![Edit {
            target: public(),
            edit: EditShape::Put {
                at: PutAt::End,
                text: "delta\n".into(),
            },
            if_node_rev: Some(NodeRev(STALE.into())),
        }]),
        &[],
        None,
    )
    .expect_err("refuses");
    assert_eq!(
        poorer.code,
        ErrorCode::CasMismatch,
        "the named refusal fired"
    );
    assert_eq!(poorer.rung, Some(2));
    assert!(poorer.new_content.is_some());
}

// ── Recovery by the book — the teaching IS the contract ────────────────────
//
// A literal client owns no knowledge beyond the refusal it was handed: it
// takes the guard noun from the fix clause and resends with that field
// carrying the served token, through the same strict door a wire frame walks.
// The taught noun must therefore be a field the door decodes and the CAS
// honors — a message that names any other noun teaches an impossible
// recovery (G-P1-2).

/// The guard noun a literal reading takes from the fix clause: the last
/// backticked token before the word "guard" — "resend … with `X` as its
/// guard" reads X; "resend … as its `Y` guard" reads Y.
fn taught_guard_field(message: &str) -> String {
    let before = &message[..message
        .find(" guard")
        .expect("the fix clause names a guard")];
    let close = before.rfind('`').expect("the guard noun is backticked");
    let open = before[..close].rfind('`').expect("a backtick pair");
    before[open + 1..close].to_owned()
}

/// The literal resend: the re-decided edit, guarded by the taught noun
/// carrying the served token, decoded at the strict wire door and executed
/// through the same choke-point the daemon uses.
fn resend_by_the_book(root: &fs::WorkspaceRoot, err: &wire::ErrorBody, edit: serde_json::Value) {
    let token = err
        .new_fingerprint
        .as_ref()
        .expect("the token to resend")
        .0
        .clone();
    let noun = taught_guard_field(err.message.as_deref().expect("a teaching message"));

    let mut edit_obj = serde_json::Map::new();
    edit_obj.insert(
        "target".into(),
        serde_json::json!({"hpath": [{"h": "Memo"}, {"h": "Public"}]}),
    );
    edit_obj.insert("edit".into(), edit);
    edit_obj.insert(noun.clone(), serde_json::Value::String(token));
    let frame = serde_json::json!({
        "id": 7, "op": "splice", "path": "memo.md",
        "actor": "agent:alice", "now": "2026-08-03T12:05:00Z",
        "edits": [edit_obj],
    });

    let op = wire_serve::decode::decode(
        frame.as_object().expect("an object frame"),
        wire_serve::rev::Rev::V3,
    )
    .unwrap_or_else(|e| {
        panic!(
            "the taught guard noun `{noun}` must decode at the strict wire door — this \
             refusal is the teaching sending its reader into a wall: {:?} {:?}",
            e.code, e.message
        )
    });
    let wire::Op::Splice {
        path,
        actor,
        now,
        receipt,
        if_root,
        dry,
        force,
        edits,
        plan_edits,
        pin,
        ..
    } = op
    else {
        panic!("a splice frame decodes as a splice")
    };
    let resend = SpliceArgs {
        premises: Vec::new(),
        id: Some(7),
        origin: Origin::Wire,
        path,
        actor,
        now,
        receipt,
        if_root,
        dry: dry.unwrap_or(false),
        force: force.unwrap_or(false),
        edits,
        plan_edits,
        pin,
    };
    if let Err(e) = splice(root, None, &resend, &[], None) {
        panic!(
            "the taught recovery must SUCCEED — the resend carried the served token \
             under the taught guard noun `{noun}` and was still refused: {:?} {:?}",
            e.code, e.message
        );
    }
}

#[test]
fn rung_one_recovery_by_the_book_succeeds_when_followed_literally() {
    let (_d, root) = ws(DOC);
    let pinned = "## Public\n\nalpha\nbravo\ndelta\n";
    let a = args(vec![stale_match(public(), pinned)]);
    let err = splice(&root, None, &a, &[], None).expect_err("refuses");
    assert_eq!(err.code, ErrorCode::CasMismatch, "the named refusal fired");
    assert_eq!(err.rung, Some(1), "the rung under test carries a diff");

    // The book, step by step: apply the `diff` extra (the caller's copy now
    // reads `charlie` where it believed `delta`), re-decide the edit against
    // that copy, resend with the taught guard.
    resend_by_the_book(
        &root,
        &err,
        serde_json::json!({"match": {"old": "charlie", "new": "charlie, revised"}}),
    );
    let after = std::fs::read_to_string(root.0.join("memo.md")).expect("read");
    assert!(
        after.contains("charlie, revised"),
        "the taught recovery landed the edit: {after}"
    );
}

#[test]
fn rung_two_recovery_by_the_book_succeeds_when_followed_literally() {
    let (_d, root) = ws(DOC);
    let a = args(vec![Edit {
        target: public(),
        edit: EditShape::Put {
            at: PutAt::End,
            text: "delta\n".into(),
        },
        if_node_rev: Some(NodeRev(STALE.into())),
    }]);
    let err = splice(&root, None, &a, &[], None).expect_err("refuses");
    assert_eq!(err.code, ErrorCode::CasMismatch, "the named refusal fired");
    assert_eq!(err.rung, Some(2), "the rung under test carries new content");

    // The book, step by step: take `new_content` as the node's current bytes,
    // re-decide (the append still stands), resend with the taught guard.
    resend_by_the_book(
        &root,
        &err,
        serde_json::json!({"put": {"at": "end", "text": "delta\n"}}),
    );
    let after = std::fs::read_to_string(root.0.join("memo.md")).expect("read");
    assert!(
        after.contains("\ndelta\n"),
        "the taught recovery landed the append: {after}"
    );
}

// ── Rung 0 — U10's door is unchanged ───────────────────────────────────────

#[test]
fn guard_required_rung_zero_still_behaves_as_u10_shipped_it() {
    let (_d, root) = ws(DOC);
    let a = args(vec![Edit {
        target: public(),
        edit: EditShape::Match {
            old: "alpha".into(),
            new: "ALPHA".into(),
        },
        if_node_rev: None,
    }]);
    let err = splice(&root, None, &a, &[], None).expect_err("refuses");

    assert_eq!(err.code, ErrorCode::GuardRequired);
    assert_eq!(err.recovery, wire::Recovery::Fix);
    let message = err.message.as_deref().expect("a teaching message");
    assert!(
        message.contains(
            "section \"Memo/Public\" in memo.md changes existing content with no \
                          fingerprint"
        ),
        "U10's own sentence, untouched: {message}"
    );
    // Rung 0 is a plain envelope: the ladder's extras belong to the rungs that
    // compute them, and enriching the missing-token door would answer a question
    // the caller has not asked yet.
    assert_eq!(err.rung, None);
    assert_eq!(err.diff, None);
    assert_eq!(err.new_content, None);
    assert_eq!(err.new_fingerprint, None);
    assert_eq!(
        std::fs::read_to_string(root.0.join("memo.md")).expect("read"),
        DOC
    );
}

// ── The extras are v3-ADDITIVE ─────────────────────────────────────────────

fn as_response(error: wire::ErrorBody) -> wire::Response {
    wire::Response {
        id: Some(88),
        ok: false,
        payload: wire::ResponsePayload::Error { error },
    }
}

#[test]
fn a_frozen_v2_session_never_grows_a_field_from_the_ladder() {
    let (_d, root) = ws(DOC);
    let pinned = "## Public\n\nalpha\nbravo\ndelta\n";
    let a = args(vec![stale_match(public(), pinned)]);
    let err = splice(&root, None, &a, &[], None).expect_err("refuses");
    assert_eq!(err.rung, Some(1), "the richest rung was minted");

    let v3 = as_response(*err);
    let demoted = wire_serve::rev::demote_v2(&v3).expect("a rung-bearing frame demotes");
    let wire::ResponsePayload::Error { error } = &demoted.payload else {
        panic!("still an error frame")
    };

    // Every slot the ladder authored is gone …
    assert_eq!(error.rung, None);
    assert_eq!(error.diff, None);
    assert_eq!(error.new_content, None);
    assert_eq!(error.new_fingerprint, None);
    assert_eq!(error.message, None);
    assert_eq!(error.path, None);
    // … and the frozen §5.2 frame is exactly what it always was.
    assert_eq!(error.code, ErrorCode::CasMismatch);
    assert_eq!(error.recovery, wire::Recovery::Refresh);
    assert_eq!(error.expected.as_ref().map(|r| r.0.as_str()), Some(STALE));
    assert!(error.actual.is_some());
    assert_eq!(
        serde_json::to_value(&demoted).expect("serialize"),
        serde_json::json!({
            "id": 88, "ok": false,
            "error": {
                "code": "cas_mismatch", "recovery": "refresh",
                "expected": STALE,
                "actual": error.actual.as_ref().expect("actual").0,
            }
        }),
        "the v2 frame carries FOUR keys — the ones it carried before U11"
    );
}

#[test]
fn demotion_touches_only_ladder_authored_envelopes() {
    let (_d, root) = ws(DOC);
    // U10's rung-0 refusal: no rung, so its teaching message survives v2 whole.
    let a = args(vec![Edit {
        target: public(),
        edit: EditShape::Match {
            old: "alpha".into(),
            new: "ALPHA".into(),
        },
        if_node_rev: None,
    }]);
    let err = splice(&root, None, &a, &[], None).expect_err("refuses");
    assert_eq!(err.code, ErrorCode::GuardRequired);
    assert!(err.message.is_some() && err.rung.is_none());

    assert!(
        wire_serve::rev::demote_v2(&as_response(*err)).is_none(),
        "a rung-less refusal is not the ladder's to strip"
    );
}
