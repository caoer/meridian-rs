//! R44 P0-1 (R45 shape B): a rule id cannot carry a forged `@fp` into the reserved journal.
//!
//! `ForcedSkip.rule` lands verbatim via `token_safe` (whitespace only). Intake is
//! `RuleId::parse` §2 grammar (narrower than old folder-slug charset). Hostile bytes
//! are unrepresentable at intake (three sites; this suite drives `parse_artifact`).
//! Assert is R26: `syntax::fp_removals` over journal bytes on disk, with a control leg
//! proving the force path actually journals a row under a legal id.

use policy::armed::Mode;
use wire::{Edit, EditShape, Path as WPath, PinSpec, PutAt, SecRef};
use wire_serve::write::{SpliceArgs, splice};

/// `@fp` claim token spelled as a rule id.
const TOKEN_ID: &str = "[[guide#^goal@green.b3af12cd|G]]";

/// Legal id for the control leg; also the placeholder the hostile artifact is re-keyed from.
const LEGAL_ID: &str = "harness.frozen-guide";

/// Rule page that refuses every change to `guide.md` (tag-registered, no `kind:`).
fn frozen_guide(id: &str) -> String {
    format!(
        "---\ntags: [type/rule, rules/check]\nid: {id}\npaths:\n  - guide.md\n---\n\n\
         # frozen guide (gate fixture)\n\n\
         ```starlark\ndef check_change(change):\n    refuse(\n        \
         message = \"frozen-guide: guide.md is frozen by an armed rule\",\n        \
         passing = \"frozen-guide.md#leave-it-alone\",\n    )\n```\n"
    )
}

const RULE_PATH: &str = "rules/frozen-guide.md";
const PINNER: &str = "---\ntitle: Plan\n---\n\n# Plan\n\ndraws from the guide.\n";

/// Arm `id`: page + engine-rendered artifact (re-keyed from LEGAL_ID) + once-armed marker.
fn arm(root: &fs::WorkspaceRoot, id: &str) {
    let page = frozen_guide(LEGAL_ID);
    write(root, RULE_PATH, &page);

    let index = policy::RuleIndex::discover([policy::PageRef {
        layer: policy::ScopeLayer::Workspace,
        page: RULE_PATH,
        bytes: &page,
    }]);
    let artifact = policy::armed::arm(
        &index,
        &policy::armed::ArmRoot::workspace(),
        [policy::armed::ArmRequest {
            id: policy::RuleId::parse(LEGAL_ID).expect("a legal id"),
            mode: Mode::Block,
            attested_rev: policy::page_rev(&page),
        }],
    )
    .expect("arm at the live rev")
    .render();

    if id != LEGAL_ID {
        // Hostile leg: page id matches the re-keyed row (not testing mismatch).
        write(root, RULE_PATH, &frozen_guide(id));
    }
    write(
        root,
        fs::domain::ARMED_RULES_PATH,
        &artifact.replace(LEGAL_ID, id),
    );
    write(root, fs::domain::ATTESTED_MARKER_PATH, "");
}

fn write(root: &fs::WorkspaceRoot, rel: &str, body: &str) {
    let full = root.0.join(rel);
    std::fs::create_dir_all(full.parent().expect("parent")).expect("dir");
    std::fs::write(full, body).expect("write");
}

fn workspace() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("plan.md"), PINNER).expect("pinner");
    std::fs::write(
        dir.path().join("guide.md"),
        "# Guide\n\n## Omega\n\nbody.\n",
    )
    .expect("target");
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    (dir, root)
}

fn pin_args(force: bool) -> SpliceArgs {
    SpliceArgs {
        id: None,
        path: WPath("plan.md".into()),
        actor: None,
        now: Some("2026-07-25T12:00:00Z".into()),
        receipt: None,
        if_root: None,
        dry: false,
        force,
        edits: Vec::new(),
        plan_edits: Vec::new(),
        pin: Some(PinSpec {
            target: WPath("guide.md".into()),
            selector: "Guide/Omega".into(),
            vibe: None,
        }),
    }
}

/// Journal bytes, or empty if never written (assert on bytes, not file existence).
fn journal(root: &fs::WorkspaceRoot) -> String {
    std::fs::read_to_string(root.0.join(fs::domain::RESERVED_JOURNAL_PATH)).unwrap_or_default()
}

fn read(root: &fs::WorkspaceRoot, rel: &str) -> String {
    std::fs::read_to_string(root.0.join(rel)).expect("read")
}

/// Gate: force-pin against artifact keyed by claim-token id → no `@fp` in journal; corrupt refusal.
#[test]
fn a_rule_id_lands_no_claim_token_in_the_reserved_journal() {
    let (_dir, root) = workspace();
    arm(&root, TOKEN_ID);
    let plan_before = read(&root, "plan.md");
    let guide_before = read(&root, "guide.md");

    let outcome = splice(&root, 0, &pin_args(true), &[], None);

    let journal = journal(&root);
    assert!(
        syntax::fp_removals(&journal).is_empty(),
        "an `@fp` claim token stands in a claim-link position in the RESERVED \
         JOURNAL — a claim nobody computed, in the ledger the chain-continuity \
         detector reads.\nfp_removals = {:?}\njournal:\n{journal}",
        syntax::fp_removals(&journal)
    );

    // Id grammar refused before force is considered; absence is unconditional.
    let err = outcome.expect_err("the id grammar refuses a row keyed outside the charset");
    let message = err.message.clone().unwrap_or_default();
    assert!(
        message.contains("corrupt"),
        "the hostile row is refused as a corrupt attestation, not survived: {:?} / {message}",
        err.code
    );

    // R32: refusal writes nothing.
    assert_eq!(
        read(&root, "plan.md"),
        plan_before,
        "the pinning page stands"
    );
    assert_eq!(read(&root, "guide.md"), guide_before, "the target stands");
}

/// Control: same force + legal id journals a real `forced_rule=` row (door is open).
#[test]
fn the_same_force_journals_a_row_when_the_id_is_in_charset() {
    let (_dir, root) = workspace();
    arm(&root, LEGAL_ID);

    splice(&root, 0, &pin_args(false), &[], None)
        .expect_err("the armed rule refuses the unforced pin");

    splice(&root, 0, &pin_args(true), &[], None).expect("--force escapes the armed refusal");

    let journal = journal(&root);
    assert!(
        journal.contains("op=force") && journal.contains(&format!("forced_rule={LEGAL_ID}")),
        "the forced row is written and names the bypassed rule BY ID:\n{journal}"
    );
    assert!(
        syntax::fp_removals(&journal).is_empty(),
        "an in-charset id journals no claim token:\n{journal}"
    );
}

/// Second `ForcedSkip.rule` origin: engine-derived `binding-break:file` (armed-page edit).
#[test]
fn the_engine_derived_forced_rule_journals_no_claim_token_either() {
    let (_dir, root) = workspace();
    arm(&root, LEGAL_ID);

    let break_args = |force: bool| SpliceArgs {
        id: None,
        path: WPath(RULE_PATH.into()),
        actor: None,
        now: Some("2026-07-25T12:00:00Z".into()),
        receipt: None,
        if_root: None,
        dry: false,
        force,
        edits: vec![Edit {
            target: SecRef::Hpath {
                hpath: vec![wire::HpathSeg {
                    h: "frozen guide (gate fixture)".into(),
                    n: None,
                }],
            },
            edit: EditShape::Put {
                at: PutAt::End,
                text: "an out-of-band edit to the attested law.\n".into(),
            },
            if_node_rev: None,
        }],
        plan_edits: Vec::new(),
        pin: None,
    };

    splice(&root, 0, &break_args(false), &[], None)
        .expect_err("the binding law refuses a one-sided change to an armed rule page");

    splice(&root, 0, &break_args(true), &[], None).expect("--force escapes the binding break");

    let journal = journal(&root);
    assert!(
        journal.contains("op=force") && journal.contains("forced_rule=binding-break:file"),
        "the engine-derived forced rule is journaled verbatim:\n{journal}"
    );
    assert!(
        syntax::fp_removals(&journal).is_empty(),
        "the engine-derived forced rule journals no claim token:\n{journal}"
    );
}
