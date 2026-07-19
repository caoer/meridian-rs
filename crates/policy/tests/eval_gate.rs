//! P6-EVAL pack-verbatim gates (impl-taskpack §9 P6-EVAL gates 1–3), driven
//! through the public `compile` + `evaluate` API.
//!
//! Gate 1 is the frozen §11.1 worked verdict: `blurb-required` over the wsfix `s2`
//! fixture yields the single verdict with span `[20,150]` and `node_rev
//! 5a8faa717fbcdb04` — COMPUTED from the real fixture bytes through the real
//! pipeline (`model::build` → facts → Starlark predicate → `Violation`), matched
//! against the frozen values. The expected values are the CONTRACT law asserted in
//! the test; the production path never hard-codes them.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use model::NodeKind;
use policy::{CompileError, CompiledRuleset, PackFiles, RulesetPin, Severity, evaluate};

/// `policy::compile` with the real parse→facts builder injected — the load gate
/// demonstrates its fixtures over the SAME plane `evaluate` runs (P6-VERDICTS
/// unification). `build_doc` is the real AST path the sidecar composes; the
/// retired synthetic per-line builder is gone from production admission.
fn compile(
    pin: &RulesetPin,
    source: &str,
    files: &dyn PackFiles,
) -> Result<CompiledRuleset, CompileError> {
    policy::compile(pin, source, files, &|path, body| {
        policy::facts_from_document(&build_doc(body.to_string(), path))
    })
}

// ── shared harness ───────────────────────────────────────────────────────────

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

/// The frozen wsfix fixture on disk (M2-BUILD oracle bytes; `testsuite/data/wsfix`).
/// Read from the real file — never a hand-copied string (task binding constraint).
fn wsfix(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../testsuite/data/wsfix")
        .join(rel)
}

/// Build a real `model::Document` from fixture bytes and stamp its path — exactly
/// what `fs`/`sidecar` do at the disk edge (the path is a caller-supplied fact;
/// `model::build` leaves it empty, being I/O-free).
fn build_doc(raw: String, path: &str) -> model::Document {
    let nodes = syntax::parse(&raw);
    let mut doc = model::build(raw, nodes);
    if let NodeKind::Document { path: p, .. } = &mut doc.root.kind {
        *p = path.to_string();
    }
    doc
}

// ── Gate 1 — the §11.1 worked verdict ────────────────────────────────────────

// The `blurb-required` rule page: a section opens with a blurb unless its first
// following node is a DEEPER heading (a subsection reached with no intervening
// content). The world model is paragraph-blind, so over `s2` the `Goals` section
// opens directly onto `Q3` (deeper) — no blurb — while `Q3`'s next heading `Q4` is a
// same-level sibling, so only `Goals` fires. The finding carries `Goals`'s injected
// section coordinates verbatim: span `[20,150]`, `node_rev 5a8faa717fbcdb04`.
const BLURB_RULE: &str = r#"# blurb-required

Every section must open with a blurb line.

```starlark
def check(doc):
    nodes = doc.nodes
    count = len(nodes)
    for i in range(count):
        node = nodes[i]
        if node.kind != "heading":
            continue
        if i + 1 < count:
            nxt = nodes[i + 1]
            if nxt.kind == "heading" and nxt.level > node.level:
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

const BLURB_MANIFEST: &str = "\
id: wiki-hygiene
api: rulepack-api@1
budgets: { steps: 10000, mem: 4194304 }
fixtures: [fixtures/blurb-pass.md, fixtures/blurb-fail.md]
rules: [rules/blurb-required.md]
";

// Load-gate fixtures demonstrated over the REAL fact plane (P6-VERDICTS
// unification): the world model is paragraph-blind (and list-blind), so the
// "blurb" between a heading and its deeper subsection must be a real dialect node
// — a wikilink is. PASS: `Goals` is followed by a `wikilink` before `Sub`, so no
// heading opens directly onto a deeper heading (rule satisfied). FAIL: `Goals`
// opens directly onto the deeper `Sub` with no intervening node (the violation).
// The retired synthetic per-line builder faked a paragraph node for the pass case
// — see fail-first artifact.
const BLURB_FX_PASS: &str = "---\nexpect: pass\n---\n# Goals\n\n[[intro]]\n\n## Sub\n\n[[more]]\n";
const BLURB_FX_FAIL: &str = "---\nexpect: fail\n---\n# Goals\n## Sub\nmore\n";

fn blurb_pack() -> MemFiles {
    MemFiles::new(&[
        ("rules/blurb-required.md", BLURB_RULE),
        ("fixtures/blurb-pass.md", BLURB_FX_PASS),
        ("fixtures/blurb-fail.md", BLURB_FX_FAIL),
    ])
}

/// Gate 1 — the frozen §11.1 worked verdict, byte-law: `blurb-required` over the
/// real wsfix `s2/notes/plan.md` yields exactly the frozen finding, every field
/// computed through `model::build` (never hard-coded on the production path).
#[test]
fn p6_eval_gate1_worked_verdict_over_wsfix_s2() {
    let raw = fs::read_to_string(wsfix("s2/notes/plan.md")).expect("read s2/notes/plan.md");
    assert_eq!(raw.len(), 150, "the frozen s2 fixture is 150 bytes");

    let doc = build_doc(raw, "notes/plan.md");
    let compiled = compile(&pin(), BLURB_MANIFEST, &blurb_pack()).expect("blurb pack is admitted");

    let verdicts = evaluate(&compiled, &[&doc], None);

    assert_eq!(
        verdicts.len(),
        1,
        "exactly the `Goals` section fires: {verdicts:?}"
    );
    let v = &verdicts[0];
    // The frozen §11.1 verdict (wire-contract-v2.md L617–619), field for field.
    assert_eq!(v.rule, "blurb-required");
    assert_eq!(v.severity, Severity::Warn);
    assert_eq!(v.path, "notes/plan.md");
    assert_eq!(v.span, 20..150, "Goals section span = heading through EOF");
    assert_eq!(
        v.node_rev.0, "5a8faa717fbcdb04",
        "Goals section node_rev, computed by model::build from the fixture bytes"
    );
    assert_eq!(v.hpath.as_deref(), Some(["Goals".to_string()].as_slice()));
    assert_eq!(v.message, "section has no blurb line");
}

// ── Gate 2 — out-of-vocab WHEN → UnsupportedVocab at compile ──────────────────

/// Gate 2 — a WHEN that references a name outside the closed fact vocabulary (here
/// the free global `now`, which `rulepack-api@1` does not inject) is refused at
/// COMPILE with `UnsupportedVocab` — before any fixture runs, never at eval time.
#[test]
fn p6_eval_gate2_out_of_vocab_when_is_unsupported_vocab_at_compile() {
    // `now` is not a local, not a stdlib global, not the injected `violation()` — the
    // predicate reaches outside the world-model fact surface.
    let rule = "# temporal\n\n```starlark\ndef check(doc):\n    if now:\n        violation(rule=\"t\", severity=\"warn\", span=(0, 0), node_rev=\"\", hpath=[], message=\"m\")\n```\n";
    let files = MemFiles::new(&[
        ("rules/temporal.md", rule),
        // present but never read — the vocab check bails before the fixture stage
        ("fixtures/x.md", "---\nexpect: pass\n---\n# H\n\nb\n"),
    ]);
    let manifest = "\
id: temporal
api: rulepack-api@1
budgets: { steps: 10000, mem: 4194304 }
fixtures: [fixtures/x.md]
rules: [rules/temporal.md]
";
    match compile(&pin(), manifest, &files).unwrap_err() {
        CompileError::UnsupportedVocab { requires, engine } => {
            assert_eq!(engine, 1, "the engine implements fact-vocabulary version 1");
            assert!(
                requires > engine,
                "an out-of-vocab WHEN requires a newer vocabulary than the engine ({requires} > {engine})"
            );
        }
        other => panic!("expected UnsupportedVocab, got {other:?}"),
    }
}

/// An in-vocab WHEN — using only the injected surface plus stdlib — is NOT refused
/// (the vocabulary check does not over-reject valid predicates).
#[test]
fn p6_eval_gate2_in_vocab_when_compiles() {
    assert!(
        compile(&pin(), BLURB_MANIFEST, &blurb_pack()).is_ok(),
        "the blurb predicate touches only doc/node facts + stdlib + violation()"
    );
}

// ── Gate 3 — budget exhaustion → budget_exceeded FINDING, not an error ─────────

// A rule that does O(n²) work over the nodes and emits nothing. Its load-gate
// fixture is tiny (passes under budget); a larger real document overruns the same
// budget at eval time — where exhaustion must surface as a FINDING, not an error.
const HEAVY_RULE: &str = "# heavy\n\n```starlark\ndef check(doc):\n    nodes = doc.nodes\n    n = len(nodes)\n    total = 0\n    for i in range(n):\n        for j in range(n):\n            total = total + 1\n```\n";

const HEAVY_MANIFEST: &str = "\
id: heavy
api: rulepack-api@1
budgets: { steps: 200, mem: 4194304 }
fixtures: [fixtures/tiny.md]
rules: [rules/heavy.md]
";

/// Gate 3 — budget exhaustion during `evaluate` lands as the `budget_exceeded`
/// FINDING in the returned verdicts (never an error or panic, frozen §8). The pack
/// is admitted (its tiny fixture runs under budget); a bigger real document busts
/// the same per-eval budget, and the overrun is REPORTED, not refused.
#[test]
fn p6_eval_gate3_budget_exhaustion_is_a_finding_not_an_error() {
    let files = MemFiles::new(&[
        ("rules/heavy.md", HEAVY_RULE),
        ("fixtures/tiny.md", "---\nexpect: pass\n---\n# A\n\nblurb\n"),
    ]);
    let compiled = compile(&pin(), HEAVY_MANIFEST, &files)
        .expect("heavy pack is admitted — its tiny fixture runs under budget");

    // A real document with enough sections that the O(n²) predicate overruns the
    // 200-step budget it demonstrated the tiny fixture under.
    let mut md = String::new();
    for k in 0..60 {
        md.push_str("# H");
        md.push_str(&k.to_string());
        md.push_str("\n\nblurb\n\n");
    }
    let doc = build_doc(md, "notes/big.md");

    // Returns normally (no panic, no Result) — that IS the "not an error frame" law.
    let verdicts = evaluate(&compiled, &[&doc], None);

    assert_eq!(
        verdicts.len(),
        1,
        "one budget finding, evaluation truncated: {verdicts:?}"
    );
    let v = &verdicts[0];
    assert!(
        v.message.starts_with("budget_exceeded:"),
        "the finding is typed budget_exceeded; got {:?}",
        v.message
    );
    assert_eq!(
        v.rule, "rules/heavy.md",
        "the finding names the busted rule page"
    );
    assert_eq!(
        v.severity,
        Severity::Error,
        "a truncated evaluation is a correctness gap"
    );
    assert_eq!(v.path, "notes/big.md");
}
