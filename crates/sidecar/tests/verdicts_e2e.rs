//! P6-VERDICTS pack-verbatim gates (impl-taskpack §9): verdicts ride BOTH the dry
//! and real splice responses, and a corpus-class pack loaded sidecar-mode is refused
//! LOUD (`daemon_only`). Driven end-to-end through `sidecar::serve` + `sidecar::admit`
//! — the real composition edge (policy + the real parse pipeline).
//!
//! The §11.1 worked verdict (`blurb-required`, span `[20,150]`, `node_rev`
//! `5a8faa717fbcdb04`) is the CONTRACT law asserted against a REAL exchange: a single
//! batch splice takes the wsfix `s0` workspace to the `s2` state (the frozen Q3 match +
//! Q4 append, disjoint targets), so the touched doc's post-batch state IS `s2` and the
//! finding carries `Goals`'s wire coordinates verbatim. The production path never
//! hard-codes them — they fall out of `model::build` over the committed bytes.

use std::collections::HashMap;
use std::io::Write as _;

use policy::{PackFiles, RulesetPin};
use serde_json::{Value, json};

// ── shared harness ───────────────────────────────────────────────────────────

fn wsfix(rel: &str) -> String {
    std::fs::read_to_string(testsuite::wsfix_dir().join(rel))
        .unwrap_or_else(|e| panic!("wsfix fixture {rel}: {e}"))
}

/// The frozen `s0` workspace (plan.md + receipts + an ignored `.github` md).
fn s0() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    for (rel, bytes) in [
        ("notes/plan.md", wsfix("s0/notes/plan.md")),
        ("receipts/2026-07-18.md", wsfix("s0/receipts/2026-07-18.md")),
        (".github/README.md", "# CI notes\n".to_string()),
    ] {
        let abs = dir.path().join(rel);
        std::fs::create_dir_all(abs.parent().expect("parent")).expect("mkdir");
        std::fs::File::create(&abs)
            .expect("create")
            .write_all(bytes.as_bytes())
            .expect("write");
    }
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    (dir, root)
}

/// One serve session over `root` with `rulesets` admitted; returns the response
/// frames (one JSON `Value` per line).
fn serve(
    root: &fs::WorkspaceRoot,
    rulesets: &[policy::CompiledRuleset],
    input: &str,
) -> Vec<Value> {
    let mut out = Vec::new();
    sidecar::serve(root, input.as_bytes(), &mut out, rulesets).expect("serve");
    String::from_utf8(out)
        .expect("frames are UTF-8")
        .lines()
        .map(|l| serde_json::from_str(l).expect("frame parses"))
        .collect()
}

/// In-memory pack-file resolver — the test stand-in for the disk reads the sidecar
/// injects in production.
struct MemFiles(HashMap<String, String>);

impl MemFiles {
    fn new(files: &[(&str, &str)]) -> Self {
        MemFiles(
            files
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
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

// The `blurb-required` rule (level variant): a section with a blurb unless its first
// following node is a DEEPER heading (a subsection reached with no intervening node).
// Over `s2/notes/plan.md` exactly `Goals` fires (it opens directly onto the deeper
// `Q3`), carrying span `[20,150]` and node_rev `5a8faa717fbcdb04`.
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

// Load-gate fixtures demonstrated over the REAL fact plane (the world model is
// paragraph- and list-blind, so the blurb between a heading and its deeper subsection
// is a real dialect node — a wikilink). PASS: `Goals` → wikilink → `Sub`, no heading
// opens directly onto a deeper heading. FAIL: `Goals` opens directly onto deeper `Sub`.
const BLURB_FX_PASS: &str = "---\nexpect: pass\n---\n# Goals\n\n[[intro]]\n\n## Sub\n\n[[more]]\n";
const BLURB_FX_FAIL: &str = "---\nexpect: fail\n---\n# Goals\n## Sub\nmore\n";

fn blurb_pack() -> policy::CompiledRuleset {
    let files = MemFiles::new(&[
        ("rules/blurb-required.md", BLURB_RULE),
        ("fixtures/blurb-pass.md", BLURB_FX_PASS),
        ("fixtures/blurb-fail.md", BLURB_FX_FAIL),
    ]);
    sidecar::admit(&pin(), BLURB_MANIFEST, &files).expect("blurb pack admits (node-class)")
}

/// A single batch splice that takes `s0` → `s2`: the frozen Q3 match + Q4 append,
/// disjoint targets in ONE batch, so the touched doc's post-batch state is `s2`
/// byte-identically. `{DRY}` is replaced by `"dry":true,` or nothing.
/// U10 (decision 18): both edits mutate existing content, so the wire frame
/// carries each target's fingerprint. The revs are DERIVED from the pre-splice
/// document, never typed.
fn s0_to_s2_splice(root: &fs::WorkspaceRoot, dry: bool) -> String {
    let dry_field = if dry { r#""dry":true,"# } else { "" };
    let rev = |sec: Value| -> String {
        let doc = fs::load(root, std::path::Path::new("notes/plan.md")).expect("load");
        let sec: wire::SecRef = serde_json::from_value(sec).expect("selector decodes");
        match wire_serve::read::cat(&doc, Some(sec)).expect("cat") {
            wire::ResponseBody::Cat { node_rev, .. } => node_rev.0,
            other => panic!("cat returned {other:?}"),
        }
    };
    let q3 = rev(json!({"hpath": [{"h": "Goals"}, {"h": "Q3"}]}));
    let q4 = rev(json!({"hpath": [{"h": "Goals"}, {"h": "Q4"}]}));
    format!(
        r#"{{"id":1,"op":"splice",{dry_field}"path":"notes/plan.md","actor":"agent:b0864fb2","now":"2026-07-18T20:33:41Z","receipt":{{"path":"receipts/2026-07-18.md","anchor":"r-000099"}},"edits":[{{"target":{{"hpath":[{{"h":"Goals"}},{{"h":"Q3"}}]}},"edit":{{"match":{{"old":"ship by August","new":"ship by September"}}}},"if_node_rev":"{q3}"}},{{"target":{{"hpath":[{{"h":"Goals"}},{{"h":"Q4"}}]}},"edit":{{"put":{{"at":"end","text":"- new item\n"}}}},"if_node_rev":"{q4}"}}]}}"#
    )
}

/// The frozen §11.1 worked verdict — the exact JSON that must ride the response.
fn worked_verdict() -> Value {
    json!({
        "rule": "blurb-required",
        "severity": "warn",
        "path": "notes/plan.md",
        "hpath": [{"h": "Goals"}],
        "span": [20, 150],
        "node_rev": "5a8faa717fbcdb04",
        "message": "section has no blurb line"
    })
}

// ── Gate 1 — the verdict rides BOTH the real and dry splice responses ─────────

/// Gate 1 (real): the §11.1 worked verdict rides a REAL splice whose post-batch state
/// is `s2` — every field computed by `model::build` over the committed bytes, matched
/// against the frozen contract values.
#[test]
fn gate1_verdict_rides_real_splice() {
    let (_d, root) = s0();
    let frames = serve(
        &root,
        &[blurb_pack()],
        &format!("{}\n", s0_to_s2_splice(&root, false)),
    );
    assert_eq!(frames.len(), 1, "one response");
    let body = &frames[0]["body"];
    assert!(body["root_after"].as_str().is_some(), "real: root advanced");
    assert_eq!(
        body["verdicts"],
        json!([worked_verdict()]),
        "the frozen §11.1 verdict rides the REAL response: {}",
        body["verdicts"]
    );
}

/// Gate 1 (dry): the identical §11.1 verdict rides the DRY twin of the same batch —
/// `root_after:null`, no disk effect, the verdict computed over the same simulated
/// after-state.
#[test]
fn gate1_verdict_rides_dry_splice() {
    let (_d, root) = s0();
    let frames = serve(
        &root,
        &[blurb_pack()],
        &format!("{}\n", s0_to_s2_splice(&root, true)),
    );
    assert_eq!(frames.len(), 1, "one response");
    let body = &frames[0]["body"];
    assert_eq!(body["root_after"], Value::Null, "dry: root_after is null");
    assert_eq!(body["dry"], json!(true), "dry flagged");
    assert_eq!(
        body["verdicts"],
        json!([worked_verdict()]),
        "the frozen §11.1 verdict rides the DRY response: {}",
        body["verdicts"]
    );
}

// ── Ruling 2 — dry ≡ real verdicts, byte-identical (LEDGER caveat) ────────────

/// Advisor Ruling 2 — the dry-response verdicts and the subsequent real-response
/// verdicts are BYTE-IDENTICAL for the same batch state, because both are the ONE
/// `policy::evaluate` call over the ONE simulated after-doc the arm builds.
///
/// LEDGER CAVEAT (binding): this pins the MECHANISM as derived data — the first
/// consumer pins the shared-call-site identity. It asserts NO frozen worked value for
/// dry/real identity (the frozen text prints no such value). If a frozen worked value
/// ever contradicts this, STOP — the frozen text wins, and this fixture is the thing
/// that is wrong, not the contract.
#[test]
fn ruling2_dry_verdicts_equal_real_verdicts_byte_identical() {
    let (_d1, root_dry) = s0();
    let dry = serve(
        &root_dry,
        &[blurb_pack()],
        &format!("{}\n", s0_to_s2_splice(&root_dry, true)),
    );

    let (_d2, root_real) = s0();
    let real = serve(
        &root_real,
        &[blurb_pack()],
        &format!("{}\n", s0_to_s2_splice(&root_real, false)),
    );

    // Serialize each verdict array to canonical JSON bytes and compare — the
    // byte-identity claim, not merely structural equality.
    let dry_bytes = serde_json::to_vec(&dry[0]["body"]["verdicts"]).expect("dry json");
    let real_bytes = serde_json::to_vec(&real[0]["body"]["verdicts"]).expect("real json");
    assert_eq!(
        dry_bytes, real_bytes,
        "dry verdicts ≡ real verdicts byte-identical (Ruling 2): dry={} real={}",
        dry[0]["body"]["verdicts"], real[0]["body"]["verdicts"]
    );
    assert!(!dry_bytes.is_empty() && dry[0]["body"]["verdicts"] != json!([]));
}

// ── Gate 2 — corpus-class pack in sidecar mode → daemon_only, loud ───────────

/// Gate 2 — a corpus-class pack (its rule names the `link_resolves` assertion, whose
/// WHEN needs the resident corpus name index) loaded against the sidecar-mode engine
/// is REFUSED LOUD at admission with `daemon_only` (env class) — the §8/§11.3
/// `BudgetClass::Corpus` law.
#[test]
fn gate2_corpus_class_pack_is_daemon_only_loud() {
    let files = MemFiles::new(&[
        (
            "rules/link-check.md",
            "# link-check\n\nCorpus-class: resolves cross-document links via the `link_resolves`\nassertion (§4 #13), so the pack runs daemon-only.\n\n```starlark\ndef check(doc):\n    pass\n```\n",
        ),
        ("fixtures/blurb-pass.md", BLURB_FX_PASS),
    ]);
    let manifest = "\
id: link-hygiene
api: rulepack-api@1
budgets: { steps: 10000, mem: 4194304 }
fixtures: [fixtures/blurb-pass.md]
rules: [rules/link-check.md]
";
    let err =
        sidecar::admit(&pin(), manifest, &files).expect_err("corpus-class refused sidecar-mode");
    assert_eq!(
        err.code,
        wire::ErrorCode::DaemonOnly,
        "loud daemon_only: {err:?}"
    );
    assert_eq!(
        err.recovery,
        wire::Recovery::Env,
        "daemon_only is env class (§8)"
    );
}

// ── Gate 4 (errata) — a splice with no packs still carries verdicts:[] ────────

/// Gate 4 re-assertion (ERRATA): the `verdicts` array rides every splice from birth;
/// with no pack loaded it is `[]` (the shape never changes at rung 6). No cap is added
/// — the naked §3.2 full-list caps ≡ stays owned by the T5-SUB fixture.
#[test]
fn gate4_no_pack_still_carries_empty_verdicts() {
    let (_d, root) = s0();
    let frames = serve(&root, &[], &format!("{}\n", s0_to_s2_splice(&root, false)));
    assert_eq!(
        frames[0]["body"]["verdicts"],
        json!([]),
        "verdicts:[] with no packs"
    );
}
