//! The run receipt's free-text doors.
//! Pins which receipt fields admit free text and which are closed/typed, so a
//! later widen cannot sneak narrative into attested identity fields.

use std::collections::BTreeMap;

use effects::{ArgValue, Effect, EffectKind, Provenance};
use run::caps::{Authority, CapSet};
use run::executor::{self, ApplyRequest, ReceiptAddr};

/// Empty run-birth fields for these fixtures.
static TEST_EMPTY_FIELDS: std::collections::BTreeMap<String, String> =
    std::collections::BTreeMap::new();

/// The hostile probe shape, driven at each door.
const HOSTILE: &str = "[[guide#^goal@green.b3af12cd|G]]";
const RECEIPT_PATH: &str = "receipts/r.md";

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

/// Drive one real apply with a receipt address and hand back the receipt file's
/// bytes as they landed on disk.
fn receipt_after_apply(page_name: &str, seed: &str, effects: &[Effect]) -> String {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join(page_name), seed).unwrap();
    let root = fs::WorkspaceRoot(tmp.path().to_owned());
    let live = fs::domain_snapshot(&root).unwrap().1;
    executor::apply(
        &root,
        &ApplyRequest {
            page: page_name,
            task: "fix-x",
            task_rev: "b3:proc-test",
            invocation_id: "inv-1",
            now: Some("2026-07-25T01:00:00Z"),
            effects,
            authority: &Authority::granted(
                CapSet::parse("md.set_field md.append_section").unwrap(),
            ),
            observed_root: &live,
            receipt: Some(ReceiptAddr {
                path: RECEIPT_PATH.to_owned(),
                anchor: "r-000001".to_owned(),
            }),
            exec: None,
            actor: None,
            depth: 0,
            delta: None,
            fields: &TEST_EMPTY_FIELDS,
            birth_seq: None,
        },
    )
    .expect("the apply commits — the claim is what the receipt may carry, not whether it runs");
    std::fs::read_to_string(root.0.join(RECEIPT_PATH)).expect("the receipt file was written")
}

/// The claim, on the bytes that landed.
///
/// The structural half is `[[`, not `[`: this line's body is JSON, whose array
/// syntax puts a legitimate single `[` before every `sec` chain. A wikilink
/// needs the DOUBLED opener, and a JSON array of strings can never produce one
/// (`[` is always followed by `"`), so "no `[[`" is the honest structural
/// statement here — the wire template's stronger "no `[` at all" is a property
/// of that template, not of this one.
fn assert_receipt_carries_no_claim(door: &str, receipt: &str) {
    assert!(
        syntax::fp_removals(receipt).is_empty(),
        "the `{door}` door landed a claim token in the receipt on disk: {receipt}"
    );
    assert!(
        !receipt.contains("[["),
        "no wikilink opener reaches the receipt, so no claim-link position can exist: {receipt}"
    );
}

const CLEAN_PAGE: &str = "---\nstatus: todo\n---\n\n# Tasks\n\n## Log\n\n- existing line\n";

/// **Door 1 — the `md.set_field` FIELD NAME** (`EditTarget::FmKey`). The task
/// block's author supplies it and `model` applies no charset guard, so it
/// reaches `ReceiptFacts.edits[].target` verbatim.
#[test]
fn a_hostile_fm_key_lands_no_claim_in_the_receipt() {
    let effects = vec![effect(
        EffectKind::SetField,
        &[("field", HOSTILE), ("value", "done")],
    )];
    let receipt = receipt_after_apply("page.md", CLEAN_PAGE, &effects);
    assert_receipt_carries_no_claim("fm", &receipt);
}

/// **Door 2 — the PAGE PATH.** The §1 path law admits `[`, `]`, `#`, `^` and
/// `@`, so a page whose own filename is a decorated claim link puts one in every
/// receipt line that records a write to it.
#[test]
fn a_hostile_page_path_lands_no_claim_in_the_receipt() {
    let effects = vec![effect(
        EffectKind::SetField,
        &[("field", "status"), ("value", "done")],
    )];
    let receipt = receipt_after_apply(&format!("{HOSTILE}.md"), CLEAN_PAGE, &effects);
    assert_receipt_carries_no_claim("page", &receipt);
}

/// **Door 3 — the SECTION HEADING** (`EditTarget::Section`), driven rather than
/// reasoned about. The heading text is read off a page that already carries the
/// token, which a write must leave exactly as found (R32 (1)) — so the page
/// keeps its token and the RECEIPT must not gain one. Retention on one plane is
/// not a licence to introduce on another.
#[test]
fn a_hostile_section_heading_lands_no_claim_in_the_receipt() {
    let seed = format!("---\nstatus: todo\n---\n\n# Tasks\n\n## {HOSTILE}\n\n- alpha line\n");
    let effects = vec![effect(
        EffectKind::AppendSection,
        &[("section", HOSTILE), ("content", "- appended")],
    )];
    let receipt = receipt_after_apply("page.md", &seed, &effects);
    assert_receipt_carries_no_claim("sec", &receipt);
}

/// The mirror that keeps the guard honest: an ordinary run's receipt is
/// UNCHANGED by the field law. Every identifier renders byte-identically, so the
/// guard is reachable only by bytes that could forge — the same shape as
/// R32 (1)'s "identical behaviour on clean pre-images".
#[test]
fn an_ordinary_run_receipt_is_byte_for_byte_what_it_always_was() {
    let effects = vec![
        effect(
            EffectKind::SetField,
            &[("field", "status"), ("value", "done")],
        ),
        effect(
            EffectKind::AppendSection,
            &[("section", "Log"), ("content", "- appended")],
        ),
    ];
    let receipt = receipt_after_apply("page.md", CLEAN_PAGE, &effects);
    assert!(
        receipt.contains(r#""page":"page.md""#),
        "the path renders as itself: {receipt}"
    );
    assert!(
        receipt.contains(r#""fm":"status""#),
        "an identifier fm key renders as itself: {receipt}"
    );
    assert!(
        receipt.contains(r#""sec":["Tasks","Log"]"#),
        "an identifier heading chain renders as itself: {receipt}"
    );
    assert!(
        !receipt.contains('`'),
        "nothing was escaped, because nothing needed to be: {receipt}"
    );
    assert_receipt_carries_no_claim("clean", &receipt);
}
