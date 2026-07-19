//! Adversarial-harness Class-4 wire probes (`p4-regression-probes.json`,
//! vendored: `testsuite/data/harness/PROVENANCE.md`) against the LIVE
//! dispatch — the D2 harness-conformance law from the unit card.
//!
//! Rung disposition (recorded in the provenance note): MP-1..4 + MP-8 bind
//! this rung's armed ops; MP-5..7 are `splice` probes — unarmed at rung 2,
//! they answer `unknown_op` here and bind D4-SPLICE; MP-9 is a GT-provenance
//! check, not a wire frame. MP-8's recorded `expect.ok:true` predates the
//! frozen D-C5 law — the runner asserts the TEXT-lawful answer
//! (`bad_request{unknown_kinds:["block_anchor"]}`; the closed §4.3 enum
//! spells the kind `anchor`) and the deviation stays loud in the provenance.

use serde_json::Value;

/// Materialize the vendored walkvault into a workspace.
fn walkvault() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let src = testsuite::harness_dir().join("walkvault");
    let dir = tempfile::tempdir().expect("tempdir");
    for rel in ["walk.md", "blocks.md", "alias-target.md", "sub/walk.md"] {
        let to = dir.path().join(rel);
        std::fs::create_dir_all(to.parent().expect("parent")).expect("mkdir");
        std::fs::copy(src.join(rel), &to).expect("copy fixture");
    }
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    (dir, root)
}

fn probes() -> Vec<Value> {
    let raw = std::fs::read_to_string(
        testsuite::harness_dir().join("p4-regression-probes.json"),
    )
    .expect("vendored probe pack");
    serde_json::from_str::<Value>(&raw).expect("probe pack parses")["probes"]
        .as_array()
        .expect("probes array")
        .clone()
}

/// Serve one probe request (id-less single-shot; legal per §3.1).
fn answer(root: &fs::WorkspaceRoot, request: &Value) -> Value {
    let line = format!("{request}\n");
    let mut out = Vec::new();
    sidecar::serve(root, line.as_bytes(), &mut out).expect("serve");
    let text = String::from_utf8(out).expect("frames are UTF-8");
    serde_json::from_str(text.lines().next().expect("one frame")).expect("frame parses")
}

#[test]
fn p4_probes_applicable_at_rung_2() {
    let (_d, root) = walkvault();
    let mut seen = Vec::new();
    for probe in probes() {
        let id = probe["id"].as_str().expect("probe id");
        if id == "MP-9" {
            continue; // GT-provenance check, not a wire frame
        }
        let frame = answer(&root, &probe["request"]);
        match id {
            // mint plane is per-segment BYTE-EQUALITY: lowercase 'b' misses '# B'
            "MP-1" => {
                assert_eq!(frame["ok"], false, "{id}: {frame}");
                assert_eq!(frame["error"]["code"], "ref_not_found", "{id}: {frame}");
            }
            // duplicate block id: the mint plane refuses loud, never last-wins.
            // Advisor-ruled shape: `candidates` stays type-level SecRef and
            // EMPTY (`[]` — no §2.1 spelling distinguishes two identical ids;
            // prose in the grammar field would violate the one-grammar law),
            // the human message carries the count.
            "MP-2" => {
                assert_eq!(frame["ok"], false, "{id}: {frame}");
                assert_eq!(frame["error"]["code"], "ambiguous_ref", "{id}: {frame}");
                assert_eq!(
                    frame["error"]["candidates"],
                    serde_json::json!([]),
                    "{id}: {frame}"
                );
                assert!(frame["error"]["message"].as_str().is_some(), "{id}: {frame}");
            }
            // occurrence index picks the SECOND '## Beta'; node_rev rides
            "MP-3" => {
                assert_eq!(frame["ok"], true, "{id}: {frame}");
                assert!(
                    frame["body"]["node_rev"].as_str().is_some(),
                    "{id}: response must carry node_rev: {frame}"
                );
                let raw = std::fs::read_to_string(
                    testsuite::harness_dir().join("walkvault/walk.md"),
                )
                .expect("walk.md");
                let second_beta = {
                    let first = raw.find("## Beta").expect("first Beta");
                    raw[first + 1..].find("## Beta").expect("second Beta") + first + 1
                };
                assert_eq!(
                    frame["body"]["span"][0].as_u64(),
                    Some(second_beta as u64),
                    "{id}: span must start at the SECOND '## Beta': {frame}"
                );
            }
            // walk plane: location facts only — never a rev key of any spelling
            "MP-4" => {
                assert_eq!(frame["ok"], true, "{id}: {frame}");
                let body = frame["body"].as_object().expect("body");
                for key in probe["expect"]["must_not_contain_keys"]
                    .as_array()
                    .expect("keys")
                {
                    let key = key.as_str().expect("key");
                    assert!(!body.contains_key(key), "{id}: no `{key}`: {frame}");
                }
            }
            // splice is unarmed at rung 2: in caps or unknown_op (§3.2);
            // these probes bind D4-SPLICE
            "MP-5" | "MP-6" | "MP-7" => {
                assert_eq!(frame["ok"], false, "{id}: {frame}");
                assert_eq!(frame["error"]["code"], "unknown_op", "{id}: {frame}");
            }
            // TEXT-LAWFUL answer (D-C5) — deviation from the probe file's
            // recorded expect, documented in the provenance note
            "MP-8" => {
                assert_eq!(frame["ok"], false, "{id}: {frame}");
                assert_eq!(frame["error"]["code"], "bad_request", "{id}: {frame}");
                assert_eq!(
                    frame["error"]["unknown_kinds"],
                    serde_json::json!(["block_anchor"]),
                    "{id}: {frame}"
                );
            }
            other => panic!("unmapped probe {other} — extend the runner"),
        }
        seen.push(id.to_string());
    }
    assert_eq!(
        seen,
        ["MP-1", "MP-2", "MP-3", "MP-4", "MP-5", "MP-6", "MP-7", "MP-8"],
        "every wire probe ran"
    );
}
