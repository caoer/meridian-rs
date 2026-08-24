//! **A fire's CHECK leg sees the §9 actor, not the plane's self-label.**
//!
//! Reviewer `36637e1a` on PR 214, finding 4. One apply mounts two armed legs,
//! and they disagreed about who was writing: the middleware leg (PR 214)
//! resolved `req.actor` by the receipt's §9 law, while the CHECK leg (6c,
//! pre-existing) passed `run:<task>` unconditionally and dropped the supplied
//! identity. So a fire by `agent:alice` presented `agent:alice` to a
//! middleware rule and `run:hook` to a check rule **in the same write**.
//!
//! **What these assert, and why that is the load-bearing thing.** Not that the
//! write landed or refused in the abstract — on the value the rule OBSERVED,
//! echoed back through the rule's own refusal message (the idiom
//! `middleware_frame.rs` uses for `ctx.fields`). A rule that was handed the
//! wrong actor cannot echo the right one, so these cannot pass by accident of
//! the bytes moving.
//!
//! **The aperture, named.** These drive the CHECK leg only — `policy::gate`
//! via `run::gate::refuse_reason`. The middleware leg's own actor is pinned
//! elsewhere; what is new here is that both legs now read ONE resolution
//! (`ApplyRequest::actor`), so they cannot drift apart again without this
//! file going red.

use std::collections::BTreeMap;

use effects::{ArgValue, Effect, EffectKind, Provenance};
use model::MerkleRoot;
use policy::armed::Mode;
use run::caps::{Authority, CapSet};
use run::executor::{self, ApplyRequest, ExecError};

/// Empty run-birth fields for these fixtures.
static TEST_EMPTY_FIELDS: BTreeMap<String, String> = BTreeMap::new();

/// The card under fire. `owner` is what the `reviewer-not-owner`-shaped rule
/// below compares the actor against.
const CARD: &str = "---\nowner: agent:alice\nstatus: Todo\n---\n\n# Task: fix-auth\n\nbody\n";

const CARD_PATH: &str = "tasks/fix-auth.md";

const RULE_PATH: &str = "rules/actor.md";

/// The task name, so the plane's self-label is `run:hook` — deliberately
/// unlike any actor the tests supply, and unlike the card's `owner`.
const TASK: &str = "hook";

/// A CHECK that ECHOES the actor it was handed into its refusal message. It
/// always refuses, because the claim under test is *what the rule saw*, not
/// whether it chose to refuse — a rule that passes tells us nothing about the
/// value it compared.
///
/// The explicit `return` after `refuse` matters for the same reason
/// `middleware_frame.rs` documents: `refuse()` records a refusal and the
/// program KEEPS RUNNING, so without it the `None` leg falls into
/// `"..." + None` and dies as an evaluation fault instead of a refusal.
const RULE_ECHO: &str = "---\ntags: [type/rule, rules/check]\nid: actor-echo\n\
    paths:\n  - tasks/**\n---\n\n# actor-echo\n\n\
    ```starlark\ndef check_change(change):\n    \
    seen = change.actor\n    \
    if seen == None:\n        \
    refuse(message = \"check saw no actor\", passing = \"rules/actor.md#none\")\n        \
    return\n    \
    refuse(message = \"check saw actor \" + seen, passing = \"rules/actor.md#seen\")\n```\n";

/// The shipped `reviewer-not-owner` shape, verbatim in spirit: refuse when the
/// actor closing the task is its own owner. This is the rule class the defect
/// silently disarmed — on a fire it compared `run:hook` against
/// `agent:alice`, never matched, and passed for the wrong reason.
const RULE_REVIEWER: &str = "---\ntags: [type/rule, rules/check]\nid: reviewer-not-owner\n\
    paths:\n  - tasks/**\n---\n\n# reviewer-not-owner\n\n\
    ```starlark\ndef check_change(change):\n    \
    owner = change.doc.frontmatter.get(\"owner\")\n    \
    actor = change.actor\n    \
    if actor != None and owner != None and actor == owner:\n        \
    refuse(message = \"reviewer must not be the owner\", \
    passing = \"rules/reviewer.md#reviewer-close\")\n```\n";

fn workspace() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    // Under target/, not $TMPDIR: macOS's /var→/private/var symlink makes the
    // door's cache canonicalization disagree with the fixture root (the same
    // reason `birth_cap.rs` and `middleware_frame.rs` say so).
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

/// Arm the workspace on `rule` — BOTH files, which is what arming IS.
fn arm_ws(root: &fs::WorkspaceRoot, id: &str, rule: &str) {
    write_page(root, RULE_PATH, rule);
    let index = policy::RuleIndex::discover([policy::PageRef {
        layer: policy::ScopeLayer::Workspace,
        page: RULE_PATH,
        bytes: rule,
    }]);
    let source = BTreeMap::from([(RULE_PATH.to_string(), rule.to_string())]);
    let artifact = policy::armed::arm(
        &index,
        &policy::armed::ArmRoot::workspace(),
        [policy::armed::ArmRequest {
            id: policy::RuleId::parse(id).expect("a legal id"),
            mode: Mode::Block,
            attested_rev: policy::page_rev(rule),
        }],
        &source,
        policy::CheckLimits::default(),
    )
    .expect("the fixture arms")
    .render();
    write_page(root, fs::domain::ARMED_RULES_PATH, &artifact);
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

/// One hook-shaped fire carrying `actor` exactly as the § A.8 wire arm threads
/// the firing session's identity.
fn fire_as(root: &fs::WorkspaceRoot, actor: Option<&str>) -> Result<executor::Applied, ExecError> {
    let observed: MerkleRoot = fs::domain_snapshot(root).unwrap().1;
    let effects = [set_status("In Progress")];
    executor::apply(
        root,
        &ApplyRequest {
            page: CARD_PATH,
            task: TASK,
            task_rev: "b3:proc",
            invocation_id: "inv-1",
            now: Some("2026-08-24T09:00:00Z"),
            effects: &effects,
            authority: &Authority::granted(CapSet::parse("md.edit").unwrap()),
            observed_root: &observed,
            receipt: None,
            actor,
            exec: None,
            depth: 0,
            delta: None,
            fields: &TEST_EMPTY_FIELDS,
            birth_seq: None,
            ambient: None,
        },
    )
}

fn refusal_detail(r: Result<executor::Applied, ExecError>) -> String {
    match r {
        Err(ExecError::ArmedRefusal { detail }) => detail,
        Err(other) => panic!("expected the armed CHECK to refuse, got {other:?}"),
        Ok(applied) => panic!(
            "expected the armed CHECK to refuse; it LANDED: {applied:?} — an armed rule \
             reading as a pass is the failure this pins"
        ),
    }
}

// ── THE test: the CHECK rule SEES the supplied actor ─────────────────────────

/// The card's own acceptance. A CHECK armed over the page evaluates on a
/// fire's `set_field`, and its verdict carries **the identity the caller
/// supplied**, verbatim.
///
/// At the parent this reads `run:hook` and the assertion fails — which is the
/// whole point: the value is observed, not inferred.
#[test]
fn the_check_leg_observes_the_supplied_actor() {
    let (_tmp, root) = workspace();
    arm_ws(&root, "actor-echo", RULE_ECHO);

    let detail = refusal_detail(fire_as(&root, Some("agent:alice")));
    assert!(
        detail.contains("check saw actor agent:alice"),
        "the CHECK leg must be handed the supplied §9 actor; it echoed: {detail}"
    );
    assert!(
        !detail.contains(&format!("check saw actor run:{TASK}")),
        "the plane's self-label must not reach a rule when a caller supplied an \
         identity: {detail}"
    );
}

/// The other half of the §9 law, so the fix is not "always use the caller":
/// an ABSENT actor still keeps the plane's self-label, and the CLI's
/// behaviour is unchanged.
#[test]
fn an_absent_actor_still_reads_the_plane_self_label() {
    let (_tmp, root) = workspace();
    arm_ws(&root, "actor-echo", RULE_ECHO);

    let detail = refusal_detail(fire_as(&root, None));
    assert!(
        detail.contains(&format!("check saw actor run:{TASK}")),
        "an absent actor keeps `run:<task>`; the rule echoed: {detail}"
    );
}

// ── The harm, in the shipped rule's own shape ────────────────────────────────

/// `reviewer-not-owner` is a real armed CHECK in `crate::gate`'s own scenario
/// fixture. Before the fix it could not fire on a fire at all: it compared
/// `run:hook` against the card's `owner`, never matched, and the write landed
/// — an armed rule reading as a pass for the wrong reason.
///
/// A fire BY the owner must now refuse.
#[test]
fn a_reviewer_not_owner_check_fires_on_a_fire_by_the_owner() {
    let (_tmp, root) = workspace();
    arm_ws(&root, "reviewer-not-owner", RULE_REVIEWER);

    let detail = refusal_detail(fire_as(&root, Some("agent:alice")));
    assert!(
        detail.contains("reviewer must not be the owner"),
        "the owner closing their own card must be refused on the fire lane: {detail}"
    );
    assert_eq!(
        std::fs::read_to_string(root.0.join(CARD_PATH)).unwrap(),
        CARD,
        "refused whole — no byte landed"
    );
}

/// The negative that stops the test above from passing by "always refuse":
/// a fire by someone who is NOT the owner still lands. The rule fires, sees a
/// real identity, and correctly declines to refuse.
#[test]
fn a_reviewer_not_owner_check_lets_a_non_owner_through() {
    let (_tmp, root) = workspace();
    arm_ws(&root, "reviewer-not-owner", RULE_REVIEWER);

    fire_as(&root, Some("agent:bob")).expect("a non-owner actor must pass the reviewer check");
    assert!(
        std::fs::read_to_string(root.0.join(CARD_PATH))
            .unwrap()
            .contains("status: In Progress"),
        "the write lands when the rule does not refuse"
    );
}
