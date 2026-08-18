//! The run plane's `@fp` door (R32 (3)).
//! Free-text doors on the run plane strip before procedure-hash / receipt
//! identity. Companion to the document-grain strip in [`run::fp`].

use std::collections::BTreeMap;

use effects::{ArgValue, Domain, Effect, EffectKind, Provenance};
use model::MerkleRoot;
use run::caps::{Authority, CapSet};

/// Empty run-birth fields for these fixtures.
static TEST_EMPTY_FIELDS: std::collections::BTreeMap<String, String> =
    std::collections::BTreeMap::new();
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
            authority: &Authority::granted(
                CapSet::parse("md.set_field md.append_section").unwrap(),
            ),
            observed_root: &now,
            receipt: None,
            exec: None,
            actor: None,
            depth: 0,
            delta: None,
            fields: &TEST_EMPTY_FIELDS,
            birth_seq: None,
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

/// A token already on disk is NOT this write's to remove (R32 (1)): deleting
/// bytes the batch never addressed would move the fingerprint of a node this
/// write does not own, reddening unrelated pins. Asserted by revs: the
/// untouched section's rev is byte-identical across the write; the edited
/// one's moves.
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

/// **The md write surface, exhaustive by construction.**
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

/// The masked page: the token sits in a frontmatter VALUE, where the one grammar
/// mints no link node — so the document carries **no claim** while it stands.
/// A write that closes the fence early turns those same bytes into a body claim
/// link that nobody computed and no payload supplies.
const MASKED: &str = "\
---
status: todo
source: [[guide#^goal@green.b3af12cd|G]]
---

# Tasks

## Log

- existing line
";

/// The `md.set_field` value that closes the frontmatter after the FIRST key. It
/// carries no `@`, so nothing here is the strip's to remove.
const CLOSING_VALUE: &str = "todo\n---\n";

/// The bytes this batch lands, spliced INDEPENDENTLY of the executor — the
/// oracle for "what would be on disk". Deriving it from `fp::candidate` would
/// ask the strip's own machinery whether the strip has anything to do.
fn would_land(seed: &str) -> String {
    seed.replacen("status: todo", "status: todo\n---\n", 1)
}

/// The frontmatter key `source`, resolved off the page as it stands. `Ok` means
/// the token is still masked inside the fence; `Err` means the fence closed
/// above it and those bytes are body now.
fn source_is_frontmatter(root: &fs::WorkspaceRoot) -> bool {
    let doc = fs::load(root, std::path::Path::new("page.md")).expect("load");
    model::resolve(&doc, &model::Ref::FmKey("source".to_owned())).is_ok()
}

/// **The COMPOSED arm — the one origin with no payload to strip.**
/// The token is not introduced — no payload carries it — and it is not
/// retained — the pre-image has no claim at all, because frontmatter is not a
/// claim-link position. This batch MAKES it a claim by un-masking bytes it does
/// not supply, so there is nothing to remove and nobody to attribute it to: the
/// only honest outcome is a loud refusal with nothing applied.
///
/// The three preconditions are asserted, not assumed, so this test cannot pass
/// on a refusal that came from somewhere else: the pre-image carries no claim,
/// the payload carries no token, and the bytes this batch would land DO carry
/// one — the last measured by an independent splice, never by the strip's own
/// candidate.
#[test]
fn a_composed_claim_token_is_refused_with_nothing_applied() {
    assert!(
        syntax::fp_removals(MASKED).is_empty(),
        "precondition: the pre-image carries NO claim, so `retained` is not the honest origin"
    );
    assert!(
        !CLOSING_VALUE.contains('@'),
        "precondition: the payload carries no token, so `introduced` is not the honest origin"
    );
    let landed = would_land(MASKED);
    assert!(
        !syntax::fp_removals(&landed).is_empty(),
        "precondition: the bytes this batch would land DO carry a claim — that is the forgery:\n\
         {landed}"
    );

    let (_tmp, root) = workspace(MASKED);
    let refused = apply(&root, &[set_field("status", CLOSING_VALUE)]);
    let Err(ExecError::FpClaim { cause, .. }) = &refused else {
        panic!("a composed claim token must refuse: {refused:?}");
    };
    assert!(
        cause.contains("COMPOSE"),
        "the refusal must be the COMPOSED arm, not attribution or re-validation: {cause}"
    );
    assert_eq!(
        page_text(&root),
        MASKED,
        "a refusal applies nothing — the page is byte-identical"
    );
}

/// **The anti-vacuity control.**
/// The refusal above is only evidence if the write it refuses is one this batch
/// could really have made. Same page, same edit, same closing value — only the
/// masked value is token-free — and the write COMMITS, proving the window is
/// genuinely open: `source` stops being a frontmatter key and its link lands in
/// a position where a token WOULD be a claim.
///
/// If `md.set_field` ever escapes newlines, or the fm-key span moves, or the
/// dialect stops calling that position a claim link, the window closes — and
/// then the test above would refuse for some other reason (or stop refusing)
/// while still looking green. This fails LOUD instead.
#[test]
fn the_frontmatter_close_window_is_genuinely_open() {
    let clean = MASKED.replace(TOKEN, "");
    assert!(
        !clean.contains('@'),
        "the control's page differs from the guard's by the token alone"
    );
    let (_tmp, root) = workspace(&clean);
    assert!(
        source_is_frontmatter(&root),
        "before the write, the masked key IS frontmatter — that is what masks it"
    );

    apply(&root, &[set_field("status", CLOSING_VALUE)])
        .expect("the same edit, token-free, commits — the window is open");

    let text = page_text(&root);
    assert!(
        !source_is_frontmatter(&root),
        "the fence closed above it: the key is body now, not frontmatter:\n{text}"
    );
    assert_eq!(
        text,
        would_land(&clean),
        "the independent splice is what actually landed — the oracle is honest:\n{text}"
    );
    let decorated = text.replace("^goal|G", &format!("^goal{TOKEN}|G"));
    assert!(
        !syntax::fp_removals(&decorated).is_empty(),
        "the un-masked line sits where a token IS a claim — the position the guard defends:\n\
         {decorated}"
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
