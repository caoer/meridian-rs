//! **A fire's splice write carries the put frame to armed middleware as
//! `ctx.fields`** (design § 6 step 6: *splice door for `set_field` /
//! `append_section` — carrying `actor`/`now`/`fields`; armed middleware
//! evaluates on those writes as on any put, with the frame the put face would
//! have given it*).
//!
//! **What these assert, and why that is the load-bearing thing.** Not that the
//! write landed — a write lands with no middleware in the tree at all. They
//! assert on the **content the rule OBSERVED in `ctx.fields`**, echoed back
//! through the rule's own refusal message and its own stamp. A rule that never
//! evaluated cannot echo a value it was never handed, so these cannot pass by
//! accident of the bytes moving.
//!
//! **The aperture, named.** `ctx.fields` is a MIDDLEWARE-ctx surface
//! (`policy::MwCtxInput`) and lives nowhere else: a CHECK rule (`policy::gate`,
//! the parity mount the run plane already had) has no `fields` surface at all,
//! and `run::executor::CommitFacts` is a NOTIFICATION lane the delta sink
//! reads and no rule does. So the middleware leg is the only lane that can
//! carry this, and these tests drive it through the real
//! `run::executor::apply`.
//!
//! Before the mount these tests pin, every case below LANDED UNREFUSED: the
//! fire lane evaluated the armed law's CHECK leg only, so an armed middleware
//! governing a page governed a put on it and ignored a fire at it — the
//! two-lanes-disagree class the parity mount exists to kill.

use std::collections::BTreeMap;

use effects::{ArgValue, Effect, EffectKind, Provenance};
use model::MerkleRoot;
use policy::armed::Mode;
use run::caps::{Authority, CapSet};
use run::executor::{self, ApplyRequest, ExecError, ReceiptAddr};

/// The card under fire, in the session dir the rule considers legal.
const CARD: &str = "---\nowner: agent:alice\nstatus: Todo\n---\n\n# Task: fix-auth\n\nbody\n";

const CARD_PATH: &str = "tasks/fix-auth.md";

/// The `session_dir` value the rule admits — the one the fixture's frame
/// carries on the legal leg.
const HOME_DIR: &str = "sessions/19-20-real";

/// The `session_dir` value a foreign session would carry.
const FOREIGN_DIR: &str = "sessions/19-20-other";

const RULE_ID: &str = "025-fixture-session-dir";
const RULE_PATH: &str = "rules/session-dir.md";

/// A `session_dir`-keyed door rule in the shape `rules/025-middleware-task-card`
/// uses: it reads the put frame's `session_dir`, and its verdict is a function
/// of that value.
///
/// It **echoes what it saw** on every leg — into the refusal message when it
/// refuses, into a stamped field when it passes — because "the rule saw the
/// frame" is the claim under test, and a verdict alone cannot carry it: a rule
/// that never ran and a rule that ran on an empty frame would both simply
/// let the write through.
const RULE: &str = "---\ntags: [type/rule, rules/middleware]\nid: 025-fixture-session-dir\n\
    paths:\n  - tasks/**\n---\n\n# 025-fixture-session-dir\n\n\
    ```starlark\ndef middleware(ctx):\n    \
    seen = ctx.fields.get(\"session_dir\")\n    \
    if seen == None:\n        \
    refuse(message = \"the put frame carries no session_dir\", \
    passing = \"rules/session-dir.md#frame\")\n    \
    if seen != \"sessions/19-20-real\":\n        \
    refuse(message = \"session_dir is \" + seen, \
    passing = \"rules/session-dir.md#same-dir\")\n    \
    set_field(path = ctx.after.path, key = \"session-dir-seen\", value = seen)\n```\n";

fn workspace() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    // Under target/, not $TMPDIR: macOS's /var→/private/var symlink makes the
    // door's cache canonicalization disagree with the fixture root (the same
    // reason `birth_cap.rs` says so). A real path sidesteps the interplay.
    let tmp = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let root = fs::WorkspaceRoot(tmp.path().to_owned());
    write_page(&root, CARD_PATH, CARD);
    (tmp, root)
}

fn write_page(root: &fs::WorkspaceRoot, rel: &str, bytes: &str) {
    let full = root.0.join(rel);
    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
    std::fs::write(full, bytes).unwrap();
}

fn read_page(root: &fs::WorkspaceRoot, rel: &str) -> String {
    std::fs::read_to_string(root.0.join(rel)).unwrap()
}

/// Write the rule page and the attested artifact that arms it in `block`.
/// Deliberately does NOT stamp the marker — `arm_ws` does, because arming is
/// both files and the never-armed control's whole subject is having one
/// without the other.
fn write_rule_and_artifact(root: &fs::WorkspaceRoot) {
    write_page(root, RULE_PATH, RULE);
    let index = policy::RuleIndex::discover([policy::PageRef {
        layer: policy::ScopeLayer::Workspace,
        page: RULE_PATH,
        bytes: RULE,
    }]);
    let source = BTreeMap::from([(RULE_PATH.to_string(), RULE.to_string())]);
    let artifact = policy::armed::arm(
        &index,
        &policy::armed::ArmRoot::workspace(),
        [policy::armed::ArmRequest {
            id: policy::RuleId::parse(RULE_ID).expect("a legal id"),
            mode: Mode::Block,
            attested_rev: policy::page_rev(RULE),
        }],
        &source,
        policy::CheckLimits::default(),
    )
    .expect("the fixture arms")
    .render();
    write_page(root, fs::domain::ARMED_RULES_PATH, &artifact);
}

/// Arm the workspace — BOTH files, which is what arming IS.
fn arm_ws(root: &fs::WorkspaceRoot) {
    write_rule_and_artifact(root);
    write_page(root, fs::domain::ATTESTED_MARKER_PATH, "");
}

fn set_status(value: &str) -> Effect {
    Effect {
        kind: EffectKind::SetField,
        rule_id: "t".to_owned(),
        seq: 0,
        depth: 0,
        provenance: Provenance::Run {
            invocation_id: "inv-1".to_owned(),
            root_at_eval: "b3:x".to_owned(),
        },
        args: BTreeMap::from([
            ("field".to_owned(), ArgValue::Str("status".to_owned())),
            ("value".to_owned(), ArgValue::Str(value.to_owned())),
        ]),
    }
}

fn frame(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

/// One hook-shaped fire: a `set_field` on the card, carrying `fields` exactly
/// as the wire arm threads the firing session's put frame (cap `run.fields`).
fn fire_set_field(
    root: &fs::WorkspaceRoot,
    fields: &BTreeMap<String, String>,
) -> Result<executor::Applied, ExecError> {
    let observed: MerkleRoot = fs::domain_snapshot(root).unwrap().1;
    let effects = [set_status("In Progress")];
    executor::apply(
        root,
        &ApplyRequest {
            page: CARD_PATH,
            task: "hook",
            task_rev: "b3:proc",
            invocation_id: "inv-1",
            now: Some("2026-08-23T16:00:00Z"),
            effects: &effects,
            authority: &Authority::granted(CapSet::parse("md.edit").unwrap()),
            observed_root: &observed,
            receipt: Some(ReceiptAddr {
                path: "receipts/run.md".to_owned(),
                anchor: "r-000001".to_owned(),
            }),
            actor: Some("agent:f014bbc4"),
            exec: None,
            depth: 0,
            delta: None,
            fields,
            birth_seq: None,
            ambient: None,
        },
    )
}

// ── THE test: the rule SEES the put frame ───────────────────────────────────

/// The card's own acceptance: a `session_dir`-keyed rule armed over the page
/// evaluates on a fire's `set_field` and its verdict carries **the value the
/// frame delivered**, verbatim.
///
/// The assertion is on the OBSERVED CONTENT, not on the write landing: the
/// refusal detail must contain the exact `session_dir` string this fire's
/// frame carried. A rule that never evaluated cannot produce that string, and
/// a rule handed an empty frame would take the other refusal leg (pinned
/// below), so this passes only when the frame reached the program intact.
#[test]
fn a_fires_splice_write_hands_the_put_frame_to_middleware_as_ctx_fields() {
    let (_tmp, root) = workspace();
    arm_ws(&root);
    let before = read_page(&root, CARD_PATH);

    let err = fire_set_field(
        &root,
        &frame(&[("session_dir", FOREIGN_DIR), ("agent", "f014bbc4")]),
    )
    .expect_err("the armed middleware refuses this frame");

    let ExecError::ArmedRefusal { detail } = &err else {
        panic!("a middleware refusal is an armed refusal, got: {err:?}");
    };
    assert!(
        detail.contains(&format!("session_dir is {FOREIGN_DIR}")),
        "the refusal must echo the session_dir the frame carried — that echo IS the \
         proof the rule read ctx.fields: {detail}"
    );
    assert!(
        detail.contains(RULE_ID),
        "the refusal names the rule that refused: {detail}"
    );
    assert_eq!(
        read_page(&root, CARD_PATH),
        before,
        "a middleware refusal lands no bytes"
    );
    assert!(
        !root.0.join("receipts/run.md").exists(),
        "no run receipt on a refused apply"
    );
}

/// The passing leg of the same rule, on a second independent instrument: the
/// value the rule read out of `ctx.fields` is stamped into the page, in the
/// same atomic write as the caller's own edit.
///
/// This is what makes the refusal above a statement about `ctx.fields` rather
/// than about refusals: the SAME program, the SAME key, a different frame
/// value — and the value that lands is the one the frame carried.
#[test]
fn the_value_middleware_read_from_ctx_fields_lands_in_the_same_write() {
    let (_tmp, root) = workspace();
    arm_ws(&root);

    fire_set_field(
        &root,
        &frame(&[("session_dir", HOME_DIR), ("agent", "f014bbc4")]),
    )
    .expect("the legal frame lands");

    let landed = read_page(&root, CARD_PATH);
    assert!(
        landed.contains("status: In Progress"),
        "the fire's own edit landed: {landed}"
    );
    assert!(
        landed.contains(&format!("session-dir-seen: {HOME_DIR}")),
        "the middleware transform landed IN the same write, carrying what it read \
         out of ctx.fields: {landed}"
    );
}

/// An ABSENT key is distinguishable from a key that never arrived: the CLI
/// entry threads an empty `fields` map, the rule sees an empty `ctx.fields`,
/// and takes its own no-frame leg.
///
/// Without this leg the test above could pass on a mount that hands middleware
/// some other map — this pins that what arrives is the request's own frame,
/// empty when the request's frame is empty.
#[test]
fn an_empty_frame_reaches_middleware_as_an_empty_ctx_fields() {
    let (_tmp, root) = workspace();
    arm_ws(&root);

    let err = fire_set_field(&root, &BTreeMap::new())
        .expect_err("the rule refuses a frame with no session_dir");

    let ExecError::ArmedRefusal { detail } = &err else {
        panic!("a middleware refusal is an armed refusal, got: {err:?}");
    };
    assert!(
        detail.contains("the put frame carries no session_dir"),
        "the rule took its OWN absent-key leg — it ran, and the frame it ran on was \
         empty: {detail}"
    );
}

/// The no-op posture, unchanged: a workspace that was never armed runs the
/// same fire to completion. The artifact is present and the marker is not —
/// that state is this control's whole subject, and it is what proves the
/// mount costs a never-armed workspace nothing.
#[test]
fn a_never_armed_workspace_runs_the_same_fire_untouched() {
    let (_tmp, root) = workspace();
    write_rule_and_artifact(&root); // the artifact is present…
    // …but NO marker → never armed → no middleware, no check gate.

    fire_set_field(
        &root,
        &frame(&[("session_dir", FOREIGN_DIR), ("agent", "f014bbc4")]),
    )
    .expect("never-armed: the fire lands");

    let landed = read_page(&root, CARD_PATH);
    assert!(
        landed.contains("status: In Progress"),
        "the fire's edit landed: {landed}"
    );
    assert!(
        !landed.contains("session-dir-seen"),
        "no middleware ran, so nothing stamped: {landed}"
    );
}
