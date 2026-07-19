//! P6-COMPILE pack-verbatim gates (impl-taskpack §9 gates 1–2) plus admission
//! and budget-class surfacing, driven entirely through the public `compile`
//! API with an in-memory `PackFiles`.

use std::collections::HashMap;

use policy::{BudgetClass, CompileError, CompiledRuleset, PackFiles, RulesetPin};

/// The real parse→facts builder the composition layer (sidecar) injects into the
/// load gate — fixtures demonstrate over the SAME fact plane production `evaluate`
/// runs (P6-VERDICTS load-gate unification), NOT the retired synthetic per-line
/// stand-in. Every `compile` call below routes through the thin wrapper so the
/// gates read unchanged while running on real facts.
fn real_facts(path: &str, body: &str) -> policy::FactDoc {
    let mut doc = model::build(body.to_string(), syntax::parse(body));
    if let model::NodeKind::Document { path: p, .. } = &mut doc.root.kind {
        *p = path.to_string();
    }
    policy::facts_from_document(&doc)
}

/// `policy::compile` with `real_facts` injected — the signature adaptation the
/// unification forces on every call site (behaviour unchanged; the fact plane is
/// now the production one).
fn compile(
    pin: &RulesetPin,
    source: &str,
    files: &dyn PackFiles,
) -> Result<CompiledRuleset, CompileError> {
    policy::compile(pin, source, files, &real_facts)
}

/// In-memory pack file resolver — the test-side stand-in for the disk reads
/// `fs`/`sidecar` inject in production.
struct MemFiles(HashMap<String, String>);

impl MemFiles {
    fn new(files: &[(&str, &str)]) -> Self {
        MemFiles(
            files
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        )
    }
}

impl PackFiles for MemFiles {
    fn read(&self, rel_path: &str) -> std::io::Result<String> {
        self.0
            .get(rel_path)
            .cloned()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, rel_path.to_string()))
    }
}

fn pin() -> RulesetPin {
    RulesetPin {
        id: "wiki-hygiene".into(),
        path: "policy/wiki-hygiene.yaml".into(),
        sha256: None,
    }
}

const MANIFEST: &str = "\
id: wiki-hygiene
api: rulepack-api@1
budgets: { steps: 10000, mem: 4194304 }
fixtures: [fixtures/blurb-pass.md, fixtures/blurb-fail.md]
rules: [rules/blurb-required.md]
";

// A literate rule page: prose + a fenced Starlark predicate (rulepack-api@1). The
// predicate reads the injected `doc.nodes` and emits a §11.1 finding per blurbless
// heading — a heading immediately followed by another heading (or EOF).
const RULE: &str = r#"# blurb-required

Every section must open with a blurb line.

```starlark
def check(doc):
    nodes = doc.nodes
    count = len(nodes)
    for i in range(count):
        node = nodes[i]
        if node.kind != "heading":
            continue
        has_blurb = i + 1 < count and nodes[i + 1].kind != "heading"
        if not has_blurb:
            violation(
                rule = "blurb-required",
                severity = "warn",
                span = node.span,
                node_rev = node.node_rev,
                hpath = node.hpath,
                message = "section has no blurb line",
            )
```
"#;
// PASS demonstrated over the REAL fact plane (P6-VERDICTS load-gate unification):
// the world model is paragraph-blind (and list-blind — `DialectKind` mints no
// bare prose or list node), so the heading's blurb must be a real dialect node. A
// wikilink is: `Goals` is followed by a `wikilink` (non-heading) node, so
// `has_blurb` holds and the rule is satisfied. (The retired synthetic per-line
// builder faked a paragraph node here; that drift is exactly what this unit kills
// — see the fail-first artifact.)
const FX_PASS: &str = "---\nexpect: pass\n---\n# Goals\n\n[[intro]]\n";
// FAIL under real facts: a heading immediately followed by another heading (no
// intervening node) demonstrates the violation.
const FX_FAIL: &str = "---\nexpect: fail\n---\n# Goals\n## Sub\nbody\n";

fn conforming_pack() -> MemFiles {
    MemFiles::new(&[
        ("rules/blurb-required.md", RULE),
        ("fixtures/blurb-pass.md", FX_PASS),
        ("fixtures/blurb-fail.md", FX_FAIL),
    ])
}

/// Gate 1 — Load gate: a pack with one failing fixture is refused at `compile`,
/// never admitted. Here `blurb-pass.md` is mislabeled — its body actually
/// violates, so its demonstrated verdict (fail) disagrees with `expect: pass`.
#[test]
fn load_gate_refuses_pack_with_failing_fixture() {
    let files = MemFiles::new(&[
        ("rules/blurb-required.md", RULE),
        // declares pass, but the heading has no blurb -> demonstrates fail
        (
            "fixtures/blurb-pass.md",
            "---\nexpect: pass\n---\n# Goals\n## Sub\nbody\n",
        ),
        ("fixtures/blurb-fail.md", FX_FAIL),
    ]);
    let err = compile(&pin(), MANIFEST, &files).unwrap_err();
    match err {
        CompileError::FixtureFailed { fixture, .. } => {
            assert_eq!(fixture, "fixtures/blurb-pass.md");
        }
        other => panic!("expected FixtureFailed, got {other:?}"),
    }
}

/// STANDING TRIPWIRE (P6-VERDICTS load-gate unification, fail-first pin): the
/// synthetic-only blurb-pass fixture — `# Goals` + a bare prose line, which the
/// RETIRED per-line builder faked as a paragraph node so the fixture "passed" — is
/// REFUSED by the real load gate. The real world model is paragraph-blind: no
/// paragraph node exists, so `Goals` opens directly onto EOF and `blurb-required`
/// fires (demonstrated:fail ≠ declared:pass). This IS the synthetic-pass/real-fail
/// drift the unification kills; a frozen worked value can never show paragraph-grain
/// facts. If this ever passes, the synthetic plane leaked back into admission — STOP.
#[test]
fn synthetic_only_pass_fixture_is_refused_by_real_load_gate() {
    let files = MemFiles::new(&[
        ("rules/blurb-required.md", RULE),
        // the OLD synthetic-only pass fixture bytes, verbatim (the retired shape)
        (
            "fixtures/blurb-pass.md",
            "---\nexpect: pass\n---\n# Goals\nA blurb line.\n",
        ),
        ("fixtures/blurb-fail.md", FX_FAIL),
    ]);
    match compile(&pin(), MANIFEST, &files).unwrap_err() {
        CompileError::FixtureFailed { fixture, detail } => {
            assert_eq!(fixture, "fixtures/blurb-pass.md");
            assert!(
                detail.contains("demonstrated:fail"),
                "paragraph-blind: the bare prose line is not a node, so Goals fires; detail: {detail}"
            );
        }
        other => panic!("expected FixtureFailed (paragraph-blind drift), got {other:?}"),
    }
}

/// Gate 1 corollary — a pack whose fixture exhausts the per-eval budget is
/// refused (the budget is enforced, not decorative).
#[test]
fn load_gate_refuses_pack_over_budget() {
    let manifest = "\
id: wiki-hygiene
api: rulepack-api@1
budgets: { steps: 1, mem: 4194304 }
fixtures: [fixtures/blurb-pass.md]
rules: [rules/blurb-required.md]
";
    let files = MemFiles::new(&[
        ("rules/blurb-required.md", RULE),
        ("fixtures/blurb-pass.md", FX_PASS),
    ]);
    assert!(matches!(
        compile(&pin(), manifest, &files),
        Err(CompileError::FixtureFailed { .. })
    ));
}

/// Gate 2a — an `api` pin mismatch is a loud `PinMismatch`, never evaluates.
#[test]
fn api_pin_mismatch_returns_pinmismatch() {
    let manifest = MANIFEST.replace("rulepack-api@1", "rulepack-api@2");
    let err = compile(&pin(), &manifest, &conforming_pack()).unwrap_err();
    match err {
        CompileError::PinMismatch { expected, actual } => {
            assert_eq!(expected, "rulepack-api@1");
            assert_eq!(actual, "rulepack-api@2");
        }
        other => panic!("expected PinMismatch, got {other:?}"),
    }
}

/// Gate 2b — a `sha256` content-pin mismatch is also `PinMismatch`, checked
/// before any fixture runs.
#[test]
fn sha256_content_pin_mismatch_returns_pinmismatch() {
    let mut p = pin();
    p.sha256 =
        Some("sha256:0000000000000000000000000000000000000000000000000000000000000000".into());
    let err = compile(&p, MANIFEST, &conforming_pack()).unwrap_err();
    assert!(
        matches!(err, CompileError::PinMismatch { .. }),
        "got {err:?}"
    );
}

/// A correct content pin passes and the pack is admitted.
#[test]
fn matching_sha256_pin_is_accepted() {
    // Compile once to learn the content hash, then pin it.
    let admitted = compile(&pin(), MANIFEST, &conforming_pack()).unwrap();
    let mut p = pin();
    p.sha256 = Some(format!("sha256:{}", admitted.content_hash()));
    assert!(compile(&p, MANIFEST, &conforming_pack()).is_ok());
}

/// Happy path — a conforming pack (all fixtures demonstrate as declared) is
/// admitted, and the compiled ruleset surfaces its id, budget, and (node-class)
/// ruleset budget class.
#[test]
fn conforming_pack_is_admitted() {
    let compiled = compile(&pin(), MANIFEST, &conforming_pack()).unwrap();
    assert_eq!(compiled.id(), "wiki-hygiene");
    assert_eq!(compiled.rule_count(), 1);
    assert_eq!(compiled.budget().steps, 10_000);
    assert_eq!(compiled.budget().mem, 4_194_304);
    // blurb-required uses only node-class assertions.
    assert_eq!(compiled.budget_class(), BudgetClass::Node);
}

/// The ruleset budget class is the max over used assertions (schema L188): a
/// pack whose rule page uses the corpus-class `link_resolves` surfaces `Corpus`
/// — the seam P6-VERDICTS reads to refuse it sidecar-mode (`daemon_only`).
#[test]
fn corpus_class_rule_surfaces_corpus_budget_class() {
    // A corpus-class rule page: its prose names the `link_resolves` assertion
    // (§4 #13), which the stand-in classifier reads to mark the pack corpus-class.
    // The fenced predicate is valid Starlark whose fixture demonstrates cleanly
    // (the link fact surface itself is P6-EVAL's, not this unit's).
    let files = MemFiles::new(&[
        (
            "rules/link-check.md",
            "# link-check\n\nCorpus-class: resolves cross-document links via the `link_resolves`\nassertion (§4 #13), so the pack runs daemon-only.\n\n```starlark\ndef check(doc):\n    pass\n```\n",
        ),
        ("fixtures/blurb-pass.md", FX_PASS),
    ]);
    let manifest = "\
id: link-hygiene
api: rulepack-api@1
budgets: { steps: 10000, mem: 4194304 }
fixtures: [fixtures/blurb-pass.md]
rules: [rules/link-check.md]
";
    let compiled = compile(&pin(), manifest, &files).unwrap();
    assert_eq!(compiled.budget_class(), BudgetClass::Corpus);
}

/// A malformed manifest (missing a required key) is the loud `Malformed` path.
#[test]
fn malformed_manifest_is_loud() {
    let manifest = "id: x\napi: rulepack-api@1\n"; // missing budgets/fixtures/rules
    assert!(matches!(
        compile(&pin(), manifest, &conforming_pack()),
        Err(CompileError::Malformed { .. })
    ));
}

/// A referenced rule page that will not resolve is `Malformed` (the pack points
/// at a rule that is not there).
#[test]
fn unreadable_rule_is_malformed() {
    let files = MemFiles::new(&[
        ("fixtures/blurb-pass.md", FX_PASS),
        ("fixtures/blurb-fail.md", FX_FAIL),
    ]); // no rules/blurb-required.md
    assert!(matches!(
        compile(&pin(), MANIFEST, &files),
        Err(CompileError::Malformed { .. })
    ));
}

// ── P6-STARLARK pack-verbatim gates (impl-taskpack §9 gates 1 & 3) ───────────
// Gate 2 (`grep -rn 'starlark' crates/wire crates/transport-proto` = 0) is a
// wire-invariance grep gate, evidenced in the task card — not a Rust test.

/// Gate 1 — a real fenced Starlark block from a rule page, evaluated over the
/// injected `doc` facts, produces the expected verdict. The conforming pack's
/// `blurb-fail` fixture (heading→heading) must DEMONSTRATE as `fail`: flipping its
/// declared `expect` to `pass` makes the load gate catch the disagreement — proof
/// the predicate actually ran and produced a violation from the injected facts.
#[test]
fn p6_starlark_gate1_fenced_predicate_evaluates_over_injected_facts() {
    assert!(
        compile(&pin(), MANIFEST, &conforming_pack()).is_ok(),
        "conforming pack (predicate demonstrates pass + fail) is admitted"
    );

    let mislabeled = MemFiles::new(&[
        ("rules/blurb-required.md", RULE),
        ("fixtures/blurb-pass.md", FX_PASS),
        // heading→heading body — the predicate yields a violation ⇒ verdict `fail`
        // — but the fixture now (wrongly) declares `expect: pass`.
        (
            "fixtures/blurb-fail.md",
            "---\nexpect: pass\n---\n# Goals\n## Sub\nbody\n",
        ),
    ]);
    match compile(&pin(), MANIFEST, &mislabeled).unwrap_err() {
        CompileError::FixtureFailed { fixture, detail } => {
            assert_eq!(fixture, "fixtures/blurb-fail.md");
            assert!(
                detail.contains("demonstrated:fail"),
                "predicate must have produced the `fail` verdict; detail: {detail}"
            );
        }
        other => panic!("expected FixtureFailed, got {other:?}"),
    }
}

/// Gate 3 — the `rulepack-api@1` pin string is verified in the manifest fixture:
/// the public pin const is exactly `rulepack-api@1`, the manifest fixture declares
/// it, and that pin is honored end-to-end (the pack compiles).
#[test]
fn p6_starlark_gate3_pin_string_rulepack_api_1_in_manifest() {
    assert_eq!(policy::RULEPACK_API, "rulepack-api@1");
    assert!(
        MANIFEST.contains("api: rulepack-api@1"),
        "manifest fixture pins rulepack-api@1"
    );
    assert!(compile(&pin(), MANIFEST, &conforming_pack()).is_ok());
}
