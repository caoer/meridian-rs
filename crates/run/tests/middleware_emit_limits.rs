//! The run plane's middleware V1 emission boundary: what the fire lane's
//! splice door **admits**, and what it **refuses LOUD**.
//!
//! The run plane's page splice is ONE atomic batch on ONE page, committed
//! through `fs::apply_batch`; unlike the wire splice door it compiles no
//! sealed SET, so a cross-file `set_field`, a `create`, and a `send` have
//! nowhere to land. The lane refuses them by name rather than dropping them —
//! a rule that believes it stamped a file it never touched is a worse outcome
//! than a refusal that says which emission and why.
//!
//! This mirrors the BIRTH door's own V1 limit verbatim in shape
//! (`wire-serve/src/write.rs`, `run_birth_middleware`: *"the birth door admits
//! refuse, this-file set_field, and send only (V1 limit); route cross-file
//! work through a put on an existing record"*) — one plane, one V1 story, two
//! doors that differ only in which classes each can actually carry.
//!
//! **Named gap, not a claim of completeness:** `send` intents that the CREATE
//! door returns to the run plane's birth lane are dropped today
//! (`run::executor::realize_births` keeps only `file_rev_after` off
//! `CreateOutcome`, whose `intents` field it never reads). That is a
//! pre-existing hole on the birth lane, not this lane, and it is not fixed
//! here.

use std::collections::BTreeMap;

use effects::{ArgValue, Effect, EffectKind, Provenance};
use model::MerkleRoot;
use policy::armed::Mode;
use run::caps::{Authority, CapSet};
use run::executor::{self, ApplyRequest, ExecError, ReceiptAddr};

const CARD: &str = "---\nowner: agent:alice\nstatus: Todo\n---\n\n# Task: fix-auth\n\nbody\n";
const CARD_PATH: &str = "tasks/fix-auth.md";
const RULE_PATH: &str = "rules/emitter.md";

/// A middleware page emitting exactly one thing, scoped to `tasks/**`.
fn rule_page(id: &str, body: &str) -> String {
    format!(
        "---\ntags: [type/rule, rules/middleware]\nid: {id}\npaths:\n  - tasks/**\n---\n\n\
         # {id}\n\n```starlark\ndef middleware(ctx):\n{body}```\n"
    )
}

fn workspace() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    // Under target/ — see `middleware_frame.rs` / `birth_cap.rs` for why.
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

fn arm_ws(root: &fs::WorkspaceRoot, id: &str, page: &str) {
    write_page(root, RULE_PATH, page);
    let index = policy::RuleIndex::discover([policy::PageRef {
        layer: policy::ScopeLayer::Workspace,
        page: RULE_PATH,
        bytes: page,
    }]);
    let source = BTreeMap::from([(RULE_PATH.to_string(), page.to_string())]);
    let artifact = policy::armed::arm(
        &index,
        &policy::armed::ArmRoot::workspace(),
        [policy::armed::ArmRequest {
            id: policy::RuleId::parse(id).expect("a legal id"),
            mode: Mode::Block,
            attested_rev: policy::page_rev(page),
        }],
        &source,
        policy::CheckLimits::default(),
    )
    .expect("the fixture arms")
    .render();
    write_page(root, fs::domain::ARMED_RULES_PATH, &artifact);
    write_page(root, fs::domain::ATTESTED_MARKER_PATH, "");
}

fn fire(root: &fs::WorkspaceRoot) -> Result<executor::Applied, ExecError> {
    let observed: MerkleRoot = fs::domain_snapshot(root).unwrap().1;
    let effects = [Effect {
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
            ("value".to_owned(), ArgValue::Str("In Progress".to_owned())),
        ]),
    }];
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
            fields: &BTreeMap::new(),
            birth_seq: None,
            ambient: None,
        },
    )
}

/// Assert the apply refused with [`ExecError::MiddlewareEmit`] naming `rule`,
/// and that the emission's own words are in the detail.
fn expect_emit_refusal(root: &fs::WorkspaceRoot, rule: &str, needle: &str) {
    let before = std::fs::read_to_string(root.0.join(CARD_PATH)).unwrap();
    let err = fire(root).expect_err("the unsupported emission refuses the apply");
    let ExecError::MiddlewareEmit {
        rule: named,
        detail,
    } = &err
    else {
        panic!("an unlandable emission is a MiddlewareEmit refusal, got: {err:?}");
    };
    assert_eq!(named, rule, "the refusal names the emitting rule");
    assert!(
        detail.contains(needle),
        "the refusal says WHICH emission it could not land: {detail}"
    );
    assert!(
        detail.contains("V1 limit"),
        "the refusal names the limit it is enforcing: {detail}"
    );
    assert_eq!(
        std::fs::read_to_string(root.0.join(CARD_PATH)).unwrap(),
        before,
        "the apply refused whole — no bytes landed"
    );
}

#[test]
fn a_cross_file_set_field_refuses_loud_rather_than_vanishing() {
    let (_tmp, root) = workspace();
    let id = "090-fixture-cross-file";
    let page = rule_page(
        id,
        "    set_field(path = \"agents/witness.md\", key = \"saw\", value = \"yes\")\n",
    );
    arm_ws(&root, id, &page);
    expect_emit_refusal(&root, id, "agents/witness.md");
}

#[test]
fn a_create_from_the_splice_door_refuses_loud() {
    let (_tmp, root) = workspace();
    let id = "091-fixture-create";
    let page = rule_page(
        id,
        "    create(path = \"tasks/never.md\", body = \"# never\\n\")\n",
    );
    arm_ws(&root, id, &page);
    expect_emit_refusal(&root, id, "tasks/never.md");
}

#[test]
fn a_send_refuses_loud_because_a_fire_row_carries_no_intent_channel() {
    let (_tmp, root) = workspace();
    let id = "092-fixture-send";
    let page = rule_page(id, "    send(to = [\"leader\"], body = \"card moved\")\n");
    arm_ws(&root, id, &page);
    expect_emit_refusal(&root, id, "leader");
}

/// The admitted class, stated as its own pin so the refusals above cannot be
/// read as "this lane refuses middleware": a this-file `set_field` lands, in
/// the caller's own atomic batch.
#[test]
fn a_this_file_set_field_is_the_admitted_class_and_lands() {
    let (_tmp, root) = workspace();
    let id = "093-fixture-self-stamp";
    let page = rule_page(
        id,
        "    set_field(path = ctx.after.path, key = \"stamped\", value = \"yes\")\n",
    );
    arm_ws(&root, id, &page);

    fire(&root).expect("a this-file transform lands");
    let landed = std::fs::read_to_string(root.0.join(CARD_PATH)).unwrap();
    assert!(
        landed.contains("status: In Progress") && landed.contains("stamped: yes"),
        "caller edit and middleware transform land in ONE write: {landed}"
    );
}
