//! Promotion correctness: rev-neutral, gated, genuinely last.
//!
//! Production `write::splice` against real workspaces. Refusals assert byte-identity of
//! both files; rev-neutrality asserts fingerprint equality (with controls that prove the
//! hash *can* move when bytes differ).

use wire::{Path as WPath, PinSpec, ResponseBody};
use wire_serve::write::{SpliceArgs, splice};

/// Pinning page with no lock block — first pin births one as file preamble.
const PINNER: &str = "---\ntitle: Plan\n---\n\n# Plan\n\ndraws from the guide.\n";

fn workspace(pinner: &str, target: &str) -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("plan.md"), pinner).expect("pinner");
    std::fs::write(dir.path().join("guide.md"), target).expect("target");
    // Git: R4 pin row needs a `hash`.
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "s2fix@example.invalid"],
        vec!["config", "user.name", "s2fix"],
    ] {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(&args)
            .status()
            .expect("git runs in the test environment");
        assert!(status.success(), "git {args:?}");
    }
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    (dir, root)
}

fn pin_args(pinning_page: &str, selector: &str) -> SpliceArgs {
    SpliceArgs {
        premises: Vec::new(),
        id: None,
        origin: wire_serve::guard::Origin::InProcess,
        path: WPath(pinning_page.into()),
        actor: None,
        now: None,
        receipt: None,
        if_root: None,
        dry: false,
        force: false,
        edits: Vec::new(),
        plan_edits: Vec::new(),
        pin: Some(PinSpec {
            target: WPath("guide.md".into()),
            selector: wire::ReadSel::parse(selector),
            vibe: None,
            fingerprint: None,
            sec_rev: None,
        }),
        fields: Default::default(),
    }
}

fn pin_fact(body: &ResponseBody) -> wire::PinFact {
    let ResponseBody::Splice { pin, .. } = body else {
        panic!("splice body");
    };
    pin.as_deref().cloned().expect("a pin answers with a fact")
}

fn read(root: &fs::WorkspaceRoot, rel: &str) -> String {
    std::fs::read_to_string(root.0.join(rel)).expect("read")
}

/// Engine content id for page bytes (names which bytes moved in identity asserts).
fn rev(bytes: &str) -> String {
    model::build(bytes.to_string(), syntax::parse(bytes))
        .root
        .node_rev
        .0
}

/// Live fingerprint of a lock `ref` (VERIFY plane: selector → resolve → hash span).
fn live_fingerprint(root: &fs::WorkspaceRoot, declared_ref: &str) -> String {
    let (rel, _) = declared_ref.split_once('#').expect("a ref names a section");
    let doc = fs::load(root, std::path::Path::new(rel)).expect("load");
    let r#ref = match model::selector::Selector::parse(declared_ref) {
        model::selector::Selector::Heading(segs) => model::Ref::Hpath(segs),
        model::selector::Selector::Block(id) => model::Ref::anchor(id).expect("block id"),
        other => panic!("unpinnable selector class: {other:?}"),
    };
    let target = model::resolve(&doc, &r#ref).expect("the lock ref resolves");
    let removals = syntax::anchor_removals(&doc.raw);
    model::fingerprint::fingerprint_span(&doc, &target.span, &removals)
        .expect("the fixture target has content")
        .into_string()
}

/// Fingerprint of a heading chain over arbitrary bytes (control: would two docs hash the same?).
fn fingerprint_of(bytes: &str, chain: &[&str]) -> String {
    let doc = model::build(bytes.to_string(), syntax::parse(bytes));
    let r#ref = model::Ref::Hpath(
        chain
            .iter()
            .map(|h| model::HpathSeg {
                h: (*h).to_string(),
                n: None,
            })
            .collect(),
    );
    let target = model::resolve(&doc, &r#ref).expect("the chain resolves");
    let removals = syntax::anchor_removals(&doc.raw);
    model::fingerprint::fingerprint_span(&doc, &target.span, &removals)
        .expect("the fixture target has content")
        .into_string()
}

/// A `/`-bearing heading — the case whose REFUSAL this file used to pin.
const SLASH_TARGET: &str = "# Guide\n\n## A/B\n\nreview before you close.\n";

/// U14 tripwire: `/` in heading no longer refuses — pin COMMITS (machine surface).
/// Scope: `/` only; `#`-in-heading refusal still lives.
#[test]
fn a_slash_bearing_heading_is_no_longer_refused_and_the_pin_commits() {
    let (_dir, root) = workspace(PINNER, SLASH_TARGET);
    let mut args = pin_args("plan.md", "unused");
    args.pin = Some(PinSpec {
        target: WPath("guide.md".into()),
        selector: wire::ReadSel::Hpath {
            hpath: vec![
                wire::HpathSeg {
                    h: "Guide".into(),
                    n: None,
                },
                wire::HpathSeg {
                    h: "A/B".into(),
                    n: None,
                },
            ],
        },
        vibe: None,
        fingerprint: None,
        sec_rev: None,
    });

    splice(&root, None, &args, &[], None).unwrap_or_else(|e| {
        panic!(
            "the `/`-round-trip refusal was RULED DEAD in U14 (2026-08-03) — an R4 \
             path array carries [\"Guide\", \"A/B\"] unambiguously and no joined echo \
             reads it back. A refusal here re-opens that ruling: {e:?}"
        )
    });
}

/// Control: withheld promotion is a real byte/rev change (identity assert is load-bearing).
#[test]
fn the_promotion_the_refusal_withholds_is_a_real_byte_change() {
    let promoted = SLASH_TARGET.replace("## A/B\n", "## A/B\n^a-b\n");
    assert_ne!(
        promoted, SLASH_TARGET,
        "the marker is a byte change, so byte-identity can fail"
    );
    assert_ne!(
        rev(&promoted),
        rev(SLASH_TARGET),
        "and the two states carry different content ids"
    );
}

/// Target last line is an unterminated heading — promotion at EOF must stay rev-neutral.
const EOF_TARGET: &str = "# Guide\n\n## Alpha\n\nalpha body.\n\n## Omega";

#[test]
fn promoting_at_eof_leaves_another_pages_pinned_fingerprint_identical() {
    let (_dir, root) = workspace(PINNER, EOF_TARGET);
    // Whole-page heading pin (span to EOF) — would move if bare terminator were appended.
    std::fs::write(root.0.join("other.md"), PINNER).expect("second pinning page");
    let first = pin_fact(
        &splice(&root, None, &pin_args("other.md", "Guide"), &[], None)
            .expect("the first pin commits")
            .body,
    );
    assert_eq!(
        first.selector,
        wire::ReadSel::Hpath {
            hpath: vec![wire::HpathSeg {
                h: "Guide".into(),
                n: None
            }]
        },
        "U14: the pin fact carries a structured selector, not a joined `page#sel` echo"
    );
    assert_eq!(
        first.fingerprint,
        live_fingerprint(&root, "guide.md#Guide"),
        "the first claim is GREEN before the second pin runs"
    );

    let second = pin_fact(
        &splice(&root, None, &pin_args("plan.md", "Guide/Omega"), &[], None)
            .expect("the second pin commits")
            .body,
    );
    assert!(second.promoted, "the marker was written");

    assert_eq!(
        live_fingerprint(&root, "guide.md#Guide"),
        first.fingerprint,
        "the OTHER page's pinned span hashes identically after the promotion — \
         rev-neutral means rev-neutral (target now: {:?})",
        read(&root, "guide.md")
    );
    assert_eq!(
        second.fingerprint,
        live_fingerprint(&root, "guide.md#Guide/Omega"),
        "and the newly minted claim is green too"
    );

    // Marker at unterminated EOF must still parse as anchor (D15 idempotency).
    let promoted_target = read(&root, "guide.md");
    let again = pin_fact(
        &splice(&root, None, &pin_args("plan.md", "Guide/Omega"), &[], None)
            .expect("the re-pin commits")
            .body,
    );
    assert_eq!(again.anchor, second.anchor, "the same slug, recomputed");
    assert!(!again.promoted, "and nothing is promoted a second time");
    assert_eq!(
        read(&root, "guide.md"),
        promoted_target,
        "the target is byte-unchanged by the re-pin"
    );
}

/// Control: bare EOF `\n` moves fingerprint; own-line marker at EOF is masked (norm-v2 R2b).
#[test]
fn a_bare_eof_terminator_moves_the_fingerprint_while_a_marker_line_does_not() {
    let terminated = format!("{EOF_TARGET}\n");
    assert_ne!(
        fingerprint_of(&terminated, &["Guide"]),
        fingerprint_of(EOF_TARGET, &["Guide"]),
        "a bare EOF terminator is NOT masked — this is the byte the promotion \
         must not add"
    );
    let marked = format!("{EOF_TARGET}\n^omega");
    assert_eq!(
        fingerprint_of(&marked, &["Guide"]),
        fingerprint_of(EOF_TARGET, &["Guide"]),
        "an own-line marker at an unterminated EOF is masked with its leading \
         terminator (norm-v2 R2b)"
    );
}

/// Rule page path (tag registration; location is free).
const RULE_PATH: &str = "rules/frozen-guide.md";

/// Rule that refuses every change to `guide.md` (tag-registered, no `kind:`).
const FROZEN_GUIDE_RULE: &str = "---\ntags: [type/rule, rules/check]\nid: frozen.guide\n\
    paths:\n  - guide.md\n---\n\n# frozen guide (gate fixture)\n\n\
    ```starlark\ndef check_change(change):\n    refuse(\n        \
    message = \"frozen-guide: guide.md is frozen by an armed rule\",\n        \
    passing = \"frozen-guide.md#leave-it-alone\",\n    )\n```\n";

/// Arm frozen.guide: rule page + artifact + marker (both files required).
fn arm_frozen_guide(root: &fs::WorkspaceRoot) {
    let page = root.0.join(RULE_PATH);
    std::fs::create_dir_all(page.parent().expect("parent")).expect("rules dir");
    std::fs::write(&page, FROZEN_GUIDE_RULE).expect("rule page");

    let index = policy::RuleIndex::discover([policy::PageRef {
        layer: policy::ScopeLayer::Workspace,
        page: RULE_PATH,
        bytes: FROZEN_GUIDE_RULE,
    }]);
    let artifact = policy::armed::arm(
        &index,
        &policy::armed::ArmRoot::workspace(),
        [policy::armed::ArmRequest {
            id: policy::RuleId::parse("frozen.guide").expect("a legal id"),
            mode: policy::armed::Mode::Block,
            attested_rev: policy::page_rev(FROZEN_GUIDE_RULE),
        }],
    )
    .expect("arm at the live rev");

    let artifact_path = root.0.join(fs::domain::ARMED_RULES_PATH);
    std::fs::create_dir_all(artifact_path.parent().expect("artifact parent")).expect("dir");
    std::fs::write(artifact_path, artifact.render()).expect("artifact");

    let marker = root.0.join(fs::domain::ATTESTED_MARKER_PATH);
    std::fs::create_dir_all(marker.parent().expect("marker parent")).expect("marker dir");
    std::fs::write(marker, "").expect("once-armed marker");
}

#[test]
fn the_promotion_is_refused_by_an_armed_rule_on_the_target() {
    let (_dir, root) = workspace(PINNER, "# Guide\n\n## Omega\n\nbody.\n");
    arm_frozen_guide(&root);
    let guide_before = read(&root, "guide.md");
    let plan_before = read(&root, "plan.md");

    let err = splice(&root, None, &pin_args("plan.md", "Guide/Omega"), &[], None)
        .expect_err("the armed law refuses the change to the target");
    assert!(
        err.message
            .as_deref()
            .is_some_and(|m| m.contains("frozen-guide")),
        "the refusal is the armed rule's, naming the rule: {:?} / {:?}",
        err.code,
        err.message
    );

    assert_eq!(
        read(&root, "guide.md"),
        guide_before,
        "no marker landed on the frozen target"
    );
    assert_eq!(
        read(&root, "plan.md"),
        plan_before,
        "and no lock landed on the pinning page"
    );
}

/// Control: same pin commits when unarmed — refusal above is armed law, not fixture breakage.
#[test]
fn the_same_pin_commits_when_the_target_is_not_armed() {
    let (_dir, root) = workspace(PINNER, "# Guide\n\n## Omega\n\nbody.\n");
    let fact = pin_fact(
        &splice(&root, None, &pin_args("plan.md", "Guide/Omega"), &[], None)
            .expect("an unarmed workspace is a no-op gate")
            .body,
    );
    assert!(fact.promoted, "the marker landed");
    assert!(read(&root, "guide.md").contains("^omega"));
    assert!(read(&root, "plan.md").contains("```meridian-lock"));
}

/// Dry pin rehearses promotion's armed gate (§4.4) and writes nothing.
#[test]
fn a_dry_pin_rehearses_the_promotions_gate() {
    let (_dir, root) = workspace(PINNER, "# Guide\n\n## Omega\n\nbody.\n");
    arm_frozen_guide(&root);
    let mut args = pin_args("plan.md", "Guide/Omega");
    args.dry = true;

    let err = splice(&root, None, &args, &[], None).expect_err("the rehearsal refuses too");
    assert!(
        err.message
            .as_deref()
            .is_some_and(|m| m.contains("frozen-guide")),
        "{:?}",
        err.message
    );
    assert!(!read(&root, "guide.md").contains("^omega"));
}
