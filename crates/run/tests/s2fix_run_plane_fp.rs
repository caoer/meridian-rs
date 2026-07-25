//! **Advisor R32 (3) — the run plane's `@fp` door.**
//!
//! The run plane lands bytes through `fs::apply_batch`, bypassing the wire
//! choke-point that strips `@fp` decoration tokens, and it had no strip of its
//! own: an `md.append_section` body carrying `[[guide#^goal@green.b3af12cd]]`
//! landed the claim on disk. A guarded invariant with an unguarded door is not an
//! invariant — without this file, criterion 4's law ("no write introduces an fp
//! token in a claim-link position") is FALSE on the run surface.
//!
//! Every assertion here is on the WRITE — the bytes on disk, or the typed
//! refusal — never on the colour a forged claim would render (R26).
//!
//! The claim is R22's, unchanged in width: **no `@fp` token in a claim-link
//! POSITION**. The positions the one grammar does NOT call claim links are named
//! and tested here as explicit exclusions, because a claim that does not state
//! its own edges is wider than its proof.

use std::collections::BTreeMap;

use effects::{ArgValue, Domain, Effect, EffectKind, Provenance};
use model::MerkleRoot;
use run::caps::CapSet;
use run::executor::{self, ApplyRequest, ExecError};

const TOKEN: &str = "@green.b3af12cd";

const PAGE: &str = "\
---
status: todo
---

# Tasks

## Alpha

- alpha line

## Log

- existing line
";

fn workspace(seed: &str) -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("page.md"), seed).unwrap();
    let root = fs::WorkspaceRoot(tmp.path().to_owned());
    (tmp, root)
}

fn current_root(root: &fs::WorkspaceRoot) -> MerkleRoot {
    fs::domain_snapshot(root).unwrap().1
}

fn effect(kind: EffectKind, args: &[(&str, &str)]) -> Effect {
    Effect {
        kind,
        rule_id: "t".to_owned(),
        seq: 0,
        depth: 0,
        provenance: Provenance::Run {
            invocation_id: "inv-1".to_owned(),
            root_at_eval: "b3:x".to_owned(),
        },
        args: args
            .iter()
            .map(|(k, v)| ((*k).to_owned(), ArgValue::Str((*v).to_owned())))
            .collect::<BTreeMap<_, _>>(),
    }
}

fn append(section: &str, content: &str) -> Effect {
    effect(
        EffectKind::AppendSection,
        &[("section", section), ("content", content)],
    )
}

fn set_field(field: &str, value: &str) -> Effect {
    effect(EffectKind::SetField, &[("field", field), ("value", value)])
}

fn apply(root: &fs::WorkspaceRoot, effects: &[Effect]) -> Result<executor::Applied, ExecError> {
    let now = current_root(root);
    executor::apply(
        root,
        &ApplyRequest {
            page: "page.md",
            task: "fix-x",
            task_rev: "b3:proc-test",
            invocation_id: "inv-1",
            now: Some("2026-07-25T01:00:00Z"),
            effects,
            caps: &CapSet::parse("md.set_field md.append_section").unwrap(),
            pin_root: &now,
            live_root: &now,
            receipt: None,
            takeover: false,
            exec: None,
            depth: 0,
        },
    )
}

fn page_text(root: &fs::WorkspaceRoot) -> String {
    std::fs::read_to_string(root.0.join("page.md")).unwrap()
}

/// The live rev of a section, read off the page as it stands — the fingerprint a
/// pin over that section would cover.
fn section_rev(root: &fs::WorkspaceRoot, heading: &str) -> String {
    let doc = fs::load(root, std::path::Path::new("page.md")).expect("load");
    model::resolve(
        &doc,
        &model::Ref::Hpath(vec![
            model::HpathSeg {
                h: "Tasks".into(),
                n: None,
            },
            model::HpathSeg {
                h: heading.into(),
                n: None,
            },
        ]),
    )
    .expect("the section resolves")
    .node_rev
    .0
}

/// **The defect R32 (3) names.** `md.append_section` with a token in its body
/// landed the claim on disk: the run plane mints no fingerprint, so every token
/// it writes is a claim nobody computed. The token is stripped; the address it
/// decorated survives intact, because the address is the content and the token
/// is a render face.
#[test]
fn an_append_section_body_token_never_reaches_disk() {
    let (_tmp, root) = workspace(PAGE);
    apply(
        &root,
        &[append("Log", &format!("- see [[guide#^goal{TOKEN}|G]]"))],
    )
    .expect("the append lands, stripped");
    let text = page_text(&root);
    assert!(
        !text.contains(TOKEN),
        "no @fp token may reach disk from a run-plane body:\n{text}"
    );
    assert!(
        text.contains("- see [[guide#^goal|G]]"),
        "the line landed with its address intact:\n{text}"
    );
}

/// The token run is peeled WHOLE, doubled shapes included: peeling only the last
/// token would emit the very shape the strip exists to remove (`syntax::fp_base`),
/// and a run-plane body is exactly where a doubled token would be pasted in.
#[test]
fn a_doubled_token_run_is_peeled_whole() {
    let (_tmp, root) = workspace(PAGE);
    apply(
        &root,
        &[append(
            "Log",
            &format!("- see [[guide#^goal{TOKEN}{TOKEN}]]"),
        )],
    )
    .expect("the append lands, stripped");
    let text = page_text(&root);
    assert!(!text.contains('@'), "the whole run is gone:\n{text}");
    assert!(text.contains("- see [[guide#^goal]]"), "{text}");
}

/// **The attribution law, and the bug an index would have.** The sealed batch is
/// sorted by span, so a two-effect batch whose effects are emitted in the reverse
/// of document order lands sealed edit 0 for effect 1. Attribution is by the
/// TARGET SPAN the model resolved, never by position: both tokens are stripped
/// and each body stays in its own section.
#[test]
fn two_bodies_in_one_batch_are_attributed_by_target_not_by_index() {
    let (_tmp, root) = workspace(PAGE);
    apply(
        &root,
        &[
            append("Log", &format!("- log [[guide#^log{TOKEN}|L]]")),
            append("Alpha", &format!("- alpha [[guide#^alpha{TOKEN}|A]]")),
        ],
    )
    .expect("both appends land, stripped");
    let text = page_text(&root);
    assert!(!text.contains(TOKEN), "{text}");
    let (alpha, log) = text.split_once("## Log").expect("both sections stand");
    assert!(
        alpha.contains("- alpha [[guide#^alpha|A]]"),
        "the Alpha body stayed in Alpha:\n{text}"
    );
    assert!(
        log.contains("- log [[guide#^log|L]]"),
        "the Log body stayed in Log:\n{text}"
    );
}

/// **The ratified deviation (R32 (1)), tested by name.** A token already on disk
/// is NOT this write's to remove: deleting bytes the batch never addressed would
/// move the fingerprint of a node this write does not own, reddening pins that
/// have nothing to do with it. The assertion is the deviation's own reasoning —
/// the untouched section's rev is byte-identical across the write, while the
/// edited one's moves. A whole-document `is_empty` rule would fail this write
/// outright, which is the false-red generator the ruling rejected.
#[test]
fn a_pre_existing_token_is_left_exactly_as_found() {
    let seed = format!(
        "---\nstatus: todo\n---\n\n# Tasks\n\n## Alpha\n\nsee [[guide#^goal{TOKEN}|G]]\n\n## Log\n\n- existing line\n"
    );
    let (_tmp, root) = workspace(&seed);
    let alpha_before = section_rev(&root, "Alpha");
    let log_before = section_rev(&root, "Log");

    apply(&root, &[append("Log", "- a run-plane line")])
        .expect("an unrelated append on a damaged page still commits");

    let text = page_text(&root);
    assert!(
        text.contains(&format!("see [[guide#^goal{TOKEN}|G]]")),
        "the pre-existing token is untouched — this write does not own those bytes:\n{text}"
    );
    assert!(text.contains("- a run-plane line"), "{text}");
    assert_eq!(
        section_rev(&root, "Alpha"),
        alpha_before,
        "the untouched section's fingerprint did not move — a pin over it stays green"
    );
    assert_ne!(
        section_rev(&root, "Log"),
        log_before,
        "the control: the section this write DID own fingerprints differently"
    );
}

/// **R22's deliberate exclusion, on the run plane.** A token-shaped string inside
/// a code fence is a code SAMPLE; stripping it would be corruption. The run
/// plane's append lands at the section's end — inside an unterminated fence, when
/// that is where the section ends — and the one grammar mints no link node there,
/// so the bytes land verbatim.
#[test]
fn a_body_landing_inside_a_fence_survives_r22() {
    let seed = "---\nstatus: todo\n---\n\n# Tasks\n\n## Log\n\n```text\nsample\n";
    let (_tmp, root) = workspace(seed);
    apply(
        &root,
        &[append("Log", &format!("[[guide#^goal{TOKEN}|G]]"))],
    )
    .expect("an append into a fence commits");
    let text = page_text(&root);
    assert!(
        text.contains(&format!("[[guide#^goal{TOKEN}|G]]")),
        "a token inside a code fence is a code SAMPLE (R22) — stripping it would corrupt it:\n{text}"
    );
}

/// **`md.set_field`'s edge, stated at exactly the width of its proof.**
/// Frontmatter is not a claim-link position: the dialect parse mints no link node
/// there, so nothing the field carries is a claim — and the DECORATE side reads
/// the same tree, so the engine can never mint a token in a position this strip
/// cannot reach. The value lands verbatim, and the document still carries no
/// claim token by the one grammar's own reckoning.
#[test]
fn a_set_field_value_is_not_a_claim_link_position() {
    let (_tmp, root) = workspace(PAGE);
    apply(
        &root,
        &[set_field(
            "source",
            &format!("\"[[guide#^goal{TOKEN}|G]]\""),
        )],
    )
    .expect("the field lands");
    let text = page_text(&root);
    assert!(
        text.contains(&format!("source: \"[[guide#^goal{TOKEN}|G]]\"")),
        "the frontmatter value landed verbatim:\n{text}"
    );
    assert!(
        syntax::fp_removals(&text).is_empty(),
        "the one grammar sees no claim-link token in this document:\n{text}"
    );
}

/// **The md write surface, exhaustive by construction (R32's count finding).**
/// `EffectKind::ALL` is the source of truth for the closed descriptor surface and
/// `executor::apply` is the run plane's ONE markdown door (`fs::apply_batch` is
/// called nowhere else in the crate). So the verbs this file must cover are
/// exactly the `Domain::Md` kinds — and a third one cannot ship without failing
/// here first.
#[test]
fn the_md_write_surface_is_exactly_the_verbs_this_file_covers() {
    let md: Vec<&str> = EffectKind::ALL
        .iter()
        .filter(|k| k.domain() == Domain::Md)
        .map(|k| k.as_str())
        .collect();
    assert_eq!(
        md,
        vec!["md.set_field", "md.append_section"],
        "a new md.* verb writes page bytes through `executor::apply` — give it its \
         claim-link test in this file before it ships"
    );
}

/// A non-md descriptor never reaches the page at all — the choke point refuses it
/// before any I/O, which is why the strip's surface is the md domain and not the
/// whole effect set.
#[test]
fn a_non_md_descriptor_cannot_carry_a_body_to_disk() {
    let (_tmp, root) = workspace(PAGE);
    let refused = apply(
        &root,
        &[effect(
            EffectKind::Notice,
            &[("text", &format!("[[guide#^goal{TOKEN}|G]]"))],
        )],
    );
    assert!(
        matches!(refused, Err(ExecError::NonMdEffect { .. })),
        "the choke point refuses a non-md descriptor: {refused:?}"
    );
    assert_eq!(page_text(&root), PAGE, "nothing was applied");
}
