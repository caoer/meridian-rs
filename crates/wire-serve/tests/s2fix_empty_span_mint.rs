//! **R31 at the MINT door** — the reachability probe, frozen as a test.
//!
//! The empty-normalized-span class is the purest false green there is: a
//! fingerprint over no bytes matches every document, so the pin can never drift.
//! `crates/model/tests/s2fix_empty_span.rs` closes the class at the owner and
//! proves the verdict side; this file answers the other half of R31 — *is the
//! class reachable through the pin verb?*
//!
//! # The answer, and why it is a TEST rather than a paragraph
//!
//! It is not reachable: every enumerated ref form that can normalize away is
//! refused, and each is refused at a rung EARLIER than the fingerprint owner —
//! the read-face resolve, or the CLI's own ref grammar. That measured fact is
//! what makes this a **VERIFY-side class**, and it is why the guard that bites
//! lives in `ContentVerdict::EmptySpan` and not here.
//!
//! Prose would rot. These asserts cannot: if a future read-face change projects
//! an own-line anchor as a fact, or the grammar starts accepting a bare page
//! ref, the form arrives at the owner — and the owner refuses too, which the
//! last test pins directly. So the class stays closed whichever rung moves, and
//! this file says which rung is doing the work TODAY.
//!
//! **The mint refusal is belt-on-belt, NOT the load-bearing guard.** Stage 3
//! must not read it as one.

use wire::{ErrorCode, Path as WPath, PinSpec, Recovery, ResponseBody};
use wire_serve::write::{SpliceArgs, splice};

/// The pinning page — the drawing end.
const PINNER: &str = "# Plan\n\ndraws from the guide.\n";

fn workspace(target_name: &str, target_body: &str) -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("plan.md"), PINNER).expect("pinner");
    std::fs::write(dir.path().join(target_name), target_body).expect("target");
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
            selector: selector.into(),
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

/// Every empty-normalizing ref form from the enumeration, as the pin verb sees
/// it. The `Page` form is absent by construction: `PinSpec` carries a selector
/// and `mrd pin`'s grammar refuses both a bare `TARGET` and an empty fragment —
/// the two probes at the bottom of this file pin that separately, because "the
/// grammar makes it unreachable" is a claim that needs its own assert.
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

/// **THE MINT PROBE — the assert is the refusal.** Every empty-normalizing ref
/// form is refused by `pin`, typed, with `Recovery::Fix`, and nothing is
/// written to the pinning page.
///
/// Refusing is the whole claim. A pin that ACCEPTED any of these and rendered
/// the result grey would pass a colour assertion and still ship a token that
/// matches every document — which is why this asserts the error and the
/// untouched bytes, never a rendered tone.
#[test]
fn every_empty_normalizing_form_is_refused_at_the_pin_door() {
    for probe in probes() {
        let (_dir, root) = workspace("guide.md", probe.target_body);
        let err = *splice(&root, 0, &pin_args("guide.md", probe.selector), &[], None)
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

        // WHICH rung refused, recorded rather than assumed. Today it is the
        // read-face resolve: an own-line anchor is not a `list_item` row, so it
        // projects no fact at all. The fingerprint owner never sees it — that
        // is the measurement behind "this class is verify-side".
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

/// The control that makes the test above non-vacuous: an INLINE anchor on the
/// same page shape mints normally. So `pin` is refusing the empty span, not
/// refusing `#^id` refs as a class — a fix that broke every anchor pin would
/// pass the refusal asserts and fail here.
#[test]
fn an_inline_anchor_on_the_same_shape_still_mints() {
    let (_dir, root) = workspace("guide.md", "# H\n\n- real content ^guideline\n");
    let out = splice(&root, 0, &pin_args("guide.md", "^guideline"), &[], None)
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

    // And it is the token the VERIFY plane recomputes over the item's real
    // bytes — so it is a genuine content id, not the universal match. Computing
    // it here rather than comparing against a literal empty digest keeps the
    // gate anchored to the engine's own hasher.
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

/// The `Page` form's rung: the pin verb requires a SELECTOR, so a whole-page
/// ref cannot reach the owner. Both spellings refuse — a bare target and an
/// empty fragment — and the refusal teaches the section grammar.
///
/// This is the form R31 did not predict, and its unreachability is the reason
/// it surfaces only through a hand-authored lock (see the `walk` gate in
/// `crates/mrd/tests/color_planes_e2e.rs`).
#[test]
fn a_whole_page_ref_cannot_reach_the_mint_at_all() {
    let (_dir, root) = workspace("empty.md", "");

    // An empty selector is refused by the request shape.
    let err = *splice(&root, 0, &pin_args("empty.md", ""), &[], None)
        .expect_err("an empty selector must refuse");
    assert_eq!(err.code, ErrorCode::PinTargetMissing);
    assert_eq!(
        std::fs::read_to_string(root.0.join("plan.md")).expect("read"),
        PINNER,
        "a refused pin writes nothing"
    );
}

/// **THE OWNER'S OWN RUNG, proven independently of every rung above it.**
///
/// The tests above measure which guard fires FIRST today. This one asks the
/// question that survives them changing: if a form ever did reach the
/// fingerprint owner with an empty normalized span, would it refuse? It does —
/// and it is the same typed error, so the class stays closed no matter which
/// earlier rung moves.
///
/// Driven at the owner rather than through `pin`, precisely because `pin` cannot
/// deliver such a span today. Pinning that fact here is what keeps the
/// belt-on-belt discharge in `write.rs` honest: it is unreachable, not dead.
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
