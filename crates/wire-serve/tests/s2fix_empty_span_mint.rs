//! R31 at the mint door: is the empty-normalized-span class reachable through
//! the pin verb? A fingerprint over no bytes matches every document, so such
//! a pin could never drift. `crates/model/tests/s2fix_empty_span.rs` closes
//! the class at the owner; this file pins that every empty-normalizing ref
//! form is refused at an earlier rung, and (last test) that the owner refuses
//! too if a rung moves. The mint refusal is belt-on-belt, not the load-bearing
//! guard — that lives in `ContentVerdict::EmptySpan`.

use wire::{ErrorCode, Path as WPath, PinSpec, Recovery, ResponseBody};
use wire_serve::write::{SpliceArgs, splice};

/// The pinning page — the drawing end.
const PINNER: &str = "# Plan\n\ndraws from the guide.\n";

fn workspace(target_name: &str, target_body: &str) -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("plan.md"), PINNER).expect("pinner");
    std::fs::write(dir.path().join(target_name), target_body).expect("target");
    // Git-initialised: the positive control mints and an R4 pin row needs a
    // git `hash`; refusal probes must fail on the rung they name, never on a
    // missing repo.
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

fn pin_args(target: &str, selector: &str) -> SpliceArgs {
    SpliceArgs {
        id: None,
        origin: wire_serve::guard::Origin::InProcess,
        path: WPath("plan.md".into()),
        actor: None,
        now: None,
        receipt: None,
        if_root: None,
        dry: false,
        force: false,
        edits: Vec::new(),
        plan_edits: Vec::new(),
        pin: Some(PinSpec {
            target: WPath(target.into()),
            selector: wire::ReadSel::parse(selector),
            vibe: None,
        }),
    }
}

/// One probed form: the ref shape, the target bytes that make it normalize to
/// nothing, and the selector the pin verb is handed.
struct Probe {
    name: &'static str,
    target_body: &'static str,
    selector: &'static str,
}

/// Every empty-normalizing ref form as the pin verb sees it. The `Page` form
/// is absent by construction — `PinSpec` carries a selector and the grammar
/// refuses a bare target or empty fragment — pinned separately below.
fn probes() -> Vec<Probe> {
    vec![
        Probe {
            name: "bare #^anchor, own line mid-file (R2)",
            target_body: "# H\n\n^guideline\n\nbody\n",
            selector: "^guideline",
        },
        Probe {
            name: "bare #^anchor, own line at EOF (R2b)",
            target_body: "# H\n\n^guideline",
            selector: "^guideline",
        },
        Probe {
            name: "bare #^anchor, own line indented (R2)",
            target_body: "# H\n\n  ^guideline\n",
            selector: "^guideline",
        },
    ]
}

/// Every empty-normalizing form is refused by `pin`: typed, `Recovery::Fix`,
/// nothing written. Asserts the error and untouched bytes, never a rendered
/// tone — an accepted pin rendered grey would still ship a universal-match
/// token.
#[test]
fn every_empty_normalizing_form_is_refused_at_the_pin_door() {
    for probe in probes() {
        let (_dir, root) = workspace("guide.md", probe.target_body);
        let err = *splice(
            &root,
            None,
            &pin_args("guide.md", probe.selector),
            &[],
            None,
        )
        .err()
        .unwrap_or_else(|| panic!("{}: pin must refuse, it minted instead", probe.name));
        assert_eq!(
            err.code,
            ErrorCode::PinTargetMissing,
            "{}: typed refusal",
            probe.name
        );
        assert_eq!(err.recovery, Recovery::Fix, "{}", probe.name);
        assert_eq!(
            std::fs::read_to_string(root.0.join("plan.md")).expect("read"),
            PINNER,
            "{}: a refused pin writes nothing",
            probe.name
        );

        // Which rung refused, recorded: today the read-face resolve — an
        // own-line anchor projects no fact, so the owner never sees it.
        assert!(
            err.message
                .as_deref()
                .is_some_and(|m| m.contains("no section addressed")),
            "{}: expected the read-face resolve rung, got {:?}",
            probe.name,
            err.message
        );
    }
}

/// Control: an inline anchor on the same shape mints normally — `pin` refuses
/// the empty span, not `#^id` refs as a class.
#[test]
fn an_inline_anchor_on_the_same_shape_still_mints() {
    let (_dir, root) = workspace("guide.md", "# H\n\n- real content ^guideline\n");
    let out = splice(&root, None, &pin_args("guide.md", "^guideline"), &[], None)
        .expect("an inline anchor has content and mints");
    let ResponseBody::Splice { pin, .. } = &out.body else {
        panic!("splice body");
    };
    let fact = pin
        .as_deref()
        .expect("a pin request answers with a pin fact");
    assert!(
        fact.fingerprint.starts_with("fp1.span2.b3."),
        "a real token: {}",
        fact.fingerprint
    );

    // The token verify recomputes over the item's real bytes — computed via
    // the engine's own hasher, not compared against a literal digest.
    let doc = fs::load(&root, std::path::Path::new("guide.md")).expect("load");
    let target = model::resolve(&doc, &model::Ref::anchor("guideline").expect("block id"))
        .expect("the anchor resolves");
    let removals = syntax::anchor_removals(&doc.raw);
    assert_eq!(
        fact.fingerprint,
        model::fingerprint::fingerprint_span(&doc, &target.span, &removals)
            .expect("an inline anchor has content")
            .into_string(),
        "the minted token is the one verify recomputes over the item's real bytes"
    );
}

/// The `Page` form's rung: `pin` requires a selector, so a whole-page ref
/// cannot reach the owner. Reachable only through a hand-authored lock
/// (`crates/mrd/tests/color_planes_e2e.rs` walk gate).
#[test]
fn a_whole_page_ref_cannot_reach_the_mint_at_all() {
    let (_dir, root) = workspace("empty.md", "");

    // An empty selector is refused by the request shape.
    let err = *splice(&root, None, &pin_args("empty.md", ""), &[], None)
        .expect_err("an empty selector must refuse");
    assert_eq!(err.code, ErrorCode::PinTargetMissing);
    assert_eq!(
        std::fs::read_to_string(root.0.join("plan.md")).expect("read"),
        PINNER,
        "a refused pin writes nothing"
    );
}

/// The owner's own rung, independent of every rung above: an empty normalized
/// span that did reach the fingerprint owner would still refuse, same typed
/// error. Driven at the owner because `pin` cannot deliver such a span today —
/// which keeps the belt-on-belt discharge in `write.rs` honest.
#[test]
fn the_owner_refuses_an_empty_span_even_when_no_earlier_rung_would() {
    let raw = "# H\n\n^guideline\n\nbody\n";
    let doc = model::build(raw.to_string(), syntax::parse(raw));
    let r#ref = model::Ref::anchor("guideline").expect("block id");
    let target = model::resolve(&doc, &r#ref).expect("the anchor resolves as a node");
    let removals = syntax::anchor_removals(&doc.raw);

    assert_eq!(
        model::fingerprint::fingerprint_span(&doc, &target.span, &removals),
        Err(model::fingerprint::EmptySpan),
        "the owner refuses the empty span on its own, with no earlier rung involved"
    );
}
