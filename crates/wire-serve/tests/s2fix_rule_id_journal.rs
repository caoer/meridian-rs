//! R44 P0-1 (R45 shape B), re-keyed by the registration cutover — a rule
//! IDENTIFIER cannot carry a forged `@fp` claim into the RESERVED JOURNAL,
//! because it cannot enter the engine at all.
//!
//! # The door, and what the cutover changed about it
//! `force_journal_write` strips the trailing anchor off a line
//! `receipt::journal::render_row` already returned, interpolates
//! `forced_rule={token_safe(&skip.rule)}`, and re-attaches the anchor.
//! `token_safe` is `split_whitespace().join("_")`: it removes no `[`, `]`, `@`,
//! `#`, `^`. So any identifier reaching `ForcedSkip.rule` lands verbatim in the
//! ledger the chain-continuity detector reads — **a forged claim in the journal
//! forges the chain detector's own input.**
//!
//! What changed is only WHICH identifier that is. It was an armed convention's
//! FOLDER NAME, guarded by `policy::validate_slug`; the folder loader is gone, and
//! `policy::gate` now builds `GateFinding.rule` from the armed row's `RuleId`. So
//! the intake that must hold is `RuleId::parse`'s § 2 grammar
//! (`[a-z0-9][a-z0-9-]*(\.[a-z0-9][a-z0-9-]*)*`), which is strictly NARROWER than
//! the slug charset it replaces.
//!
//! R45's ruling survives the re-key unchanged, and it is the reason this is one
//! test rather than one per renderer: the hostile bytes are made
//! **unrepresentable rather than removable**, at INTAKE, so every renderer added
//! later inherits the guard. The id has three intakes and the grammar sits at all
//! three — a page's `id:` frontmatter (`register_page`), the ARM act
//! (`ArmRequest.id`), and reading an attested page back (`parse_artifact`'s
//! `parse_row`). The hostile leg below drives the THIRD, because a hand-edited
//! artifact row is the only way a string that never passed the other two can
//! present itself as armed.
//!
//! THE ASSERT IS THE ARTIFACT (R26): `syntax::fp_removals` over the journal bytes
//! ON DISK, never a string that looks right. The absence is carried by a CONTROL
//! leg that drives the SAME force through the SAME fixture with a legal id and
//! proves the journal row is really written — so "no token" can never be "no row".

use policy::armed::Mode;
use wire::{Edit, EditShape, Path as WPath, PinSpec, PutAt, SecRef};
use wire_serve::write::{SpliceArgs, splice};

/// An `@fp` claim token in a claim-link position, spelled as a rule id.
const TOKEN_ID: &str = "[[guide#^goal@green.b3af12cd|G]]";

/// The legal id the control leg arms — and the placeholder the hostile artifact is
/// rendered through, so both legs read bytes the ENGINE produced.
const LEGAL_ID: &str = "harness.frozen-guide";

/// A rule page that refuses every change to `guide.md` — the live refusal the
/// `--force` leg must escape. Registers by TAG, with no `kind:` key.
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

/// Arm `id` in a fresh workspace: the rule page on disk, an attested artifact
/// pinned to its live rev, and the once-armed marker.
///
/// The artifact is rendered by the ENGINE through [`LEGAL_ID`] and then re-keyed to
/// `id`, so the hostile leg reads the exact row grammar `ArmedArtifact::render`
/// emits rather than a hand-typed imitation — and the same fixture builds
/// identically before and after the intake guard.
///
/// BOTH files are written. The artifact alone leaves a workspace the marker says
/// was never armed; the marker alone is `ArmedFault::Missing`.
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
        // The hostile leg presents the page under the hostile id too, so the row
        // and the page agree — the fixture is not refused for a mismatch it did
        // not mean to test.
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

/// The journal page's bytes, or empty when no row was ever appended — an absent
/// page is the strongest possible "no forged row", and reading it this way keeps
/// the assertion about BYTES rather than about a file's existence.
fn journal(root: &fs::WorkspaceRoot) -> String {
    std::fs::read_to_string(root.0.join(fs::domain::RESERVED_JOURNAL_PATH)).unwrap_or_default()
}

fn read(root: &fs::WorkspaceRoot, rel: &str) -> String {
    std::fs::read_to_string(root.0.join(rel)).expect("read")
}

/// **THE GATE.** The production splice choke point, `force: true`, against a
/// workspace whose attested artifact carries a row keyed by a claim token. The
/// journal bytes on disk carry no `@fp` claim.
#[test]
fn a_rule_id_lands_no_claim_token_in_the_reserved_journal() {
    let (_dir, root) = workspace();
    arm(&root, TOKEN_ID);
    let plan_before = read(&root, "plan.md");
    let guide_before = read(&root, "guide.md");

    let outcome = splice(&root, 0, &pin_args(true), &[], None);

    // THE ASSERT — the artifact, first, so a regression quotes the ranges.
    let journal = journal(&root);
    assert!(
        syntax::fp_removals(&journal).is_empty(),
        "an `@fp` claim token stands in a claim-link position in the RESERVED \
         JOURNAL — a claim nobody computed, in the ledger the chain-continuity \
         detector reads.\nfp_removals = {:?}\njournal:\n{journal}",
        syntax::fp_removals(&journal)
    );

    // And it is absent for the RIGHT reason: the id grammar refused the row, so the
    // artifact never parsed and nothing was armed under that name. Note `--force`
    // does NOT escape this — an armed-law fault refuses before the gate reads
    // `change.force`, which is what makes the absence above unconditional.
    let err = outcome.expect_err("the id grammar refuses a row keyed outside the charset");
    let message = err.message.clone().unwrap_or_default();
    assert!(
        message.contains("corrupt"),
        "the hostile row is refused as a corrupt attestation, not survived: {:?} / {message}",
        err.code
    );

    // A refusal writes nothing (R32 (1)): both files byte-unchanged.
    assert_eq!(
        read(&root, "plan.md"),
        plan_before,
        "the pinning page stands"
    );
    assert_eq!(read(&root, "guide.md"), guide_before, "the target stands");
}

/// **THE CONTROL for the absence above** — an assertion of absence passes on an
/// empty world, so this drives the SAME force through the SAME fixture with a legal
/// id: the armed law refuses unforced, `--force` escapes it, and the journal really
/// does gain an `op=force` row naming `forced_rule=`. The gate above is therefore
/// measuring a door that is open, not machinery that never ran.
#[test]
fn the_same_force_journals_a_row_when_the_id_is_in_charset() {
    let (_dir, root) = workspace();
    arm(&root, LEGAL_ID);

    // The armed rule genuinely refuses — the force escapes something real.
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

/// **THE SECOND SOURCE of `forced_rule=`, enumerated and driven.**
/// `ForcedSkip.rule` has exactly two origins in `policy::gate`: an armed row's
/// `RuleId` (closed by intake, above) and the ENGINE-DERIVED
/// `binding-break:{index|file}` of a forced one-sided change. The second is a
/// constant the engine writes, not caller bytes — but "engine-derived" is a claim,
/// and an undriven door of a named shape is the enumeration not closing. So it is
/// driven: the row lands, and it carries no claim token either.
///
/// The break is now a direct edit of the ARMED PAGE — the page-side binding break,
/// re-keyed from the folder generation's `conventions/<slug>/CHECK.md` shape to
/// membership in what the workspace attested.
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

    // CONTROL — unforced, the door law refuses, so the force escapes something real.
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
