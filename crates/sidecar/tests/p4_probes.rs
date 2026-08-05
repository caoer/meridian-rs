//! Class-4 wire probes (`p4-regression-probes.json`) against live dispatch.
//!
//! Pins: MP-1..4 + MP-8 at decode/read; MP-5/6 alien fields → `bad_request`;
//! MP-7 guardless splice → `guard_required` (frame legal; pack's
//! `expect.ok:true` is superseded, deviation loud); MP-8 `block_anchor` →
//! `bad_request{unknown_kinds}` (D-C5; pack expect superseded). MP-9 skipped
//! (GT-provenance, not a wire frame).

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

/// The wsfix S0 workspace MP-7 assumes ("state s0" in its expect note).
fn s0_workspace() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    for rel in ["s0/notes/plan.md", "s0/receipts/2026-07-18.md"] {
        let to = dir.path().join(rel.strip_prefix("s0/").expect("s0 prefix"));
        std::fs::create_dir_all(to.parent().expect("parent")).expect("mkdir");
        std::fs::copy(testsuite::wsfix_dir().join(rel), &to).expect("copy fixture");
    }
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    (dir, root)
}

fn probes() -> Vec<Value> {
    let raw = std::fs::read_to_string(testsuite::harness_dir().join("p4-regression-probes.json"))
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
    sidecar::serve(root, line.as_bytes(), &mut out, &[]).expect("serve");
    let text = String::from_utf8(out).expect("frames are UTF-8");
    serde_json::from_str(text.lines().next().expect("one frame")).expect("frame parses")
}

/// MP-5/6: alien write fields (`ref`/`span`) refuse at strict decode.
fn assert_splice_decode_kill(id: &str, frame: &Value) {
    assert_eq!(frame["ok"], false, "{id}: {frame}");
    assert_eq!(frame["error"]["code"], "bad_request", "{id}: {frame}");
    let named = match id {
        "MP-5" => "`ref`",
        _ => "`span`",
    };
    assert!(
        frame["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains(named)),
        "{id}: the refusal names the alien field {named}: {frame}"
    );
}

/// MP-7: guardless splice — write refused (`guard_required`); frame legal
/// (not `bad_frame`/`bad_request`). Schema-optional guards must stay optional.
fn assert_mp7_007_plus_write(id: &str, request: &Value) {
    // Probe assumes "state s0" (wsfix S0), not walkvault.
    let (_d, s0_root) = s0_workspace();
    let frame = answer(&s0_root, request);
    assert_eq!(frame["ok"], false, "{id}: the WRITE is refused: {frame}");
    assert_eq!(
        frame["error"]["code"], "guard_required",
        "{id}: a SEMANTIC write refusal: {frame}"
    );
    // Frame must never be called malformed.
    for illegal in ["bad_frame", "bad_request", "unknown_op"] {
        assert_ne!(
            frame["error"]["code"], illegal,
            "{id}: a guardless frame stays a LEGAL frame — 007's \
                     schema half survives: {frame}"
        );
    }
    assert_eq!(
        frame["error"]["recovery"], "fix",
        "{id}: fix class — change the request, not the channel: {frame}"
    );
    // Reached write path: refusal names the target file.
    assert_eq!(
        frame["error"]["path"], request["path"],
        "{id}: the refusal comes from the write path, so the frame \
                 decoded: {frame}"
    );
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
            // Duplicate block id: loud refuse; `candidates` = [] (no §2.1
            // spelling distinguishes identical ids); message carries count.
            "MP-2" => {
                assert_eq!(frame["ok"], false, "{id}: {frame}");
                assert_eq!(frame["error"]["code"], "ambiguous_ref", "{id}: {frame}");
                assert_eq!(
                    frame["error"]["candidates"],
                    serde_json::json!([]),
                    "{id}: {frame}"
                );
                assert!(
                    frame["error"]["message"].as_str().is_some(),
                    "{id}: {frame}"
                );
            }
            // occurrence index picks the SECOND '## Beta'; node_rev rides
            "MP-3" => {
                assert_eq!(frame["ok"], true, "{id}: {frame}");
                assert!(
                    frame["body"]["node_rev"].as_str().is_some(),
                    "{id}: response must carry node_rev: {frame}"
                );
                let raw =
                    std::fs::read_to_string(testsuite::harness_dir().join("walkvault/walk.md"))
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
            "MP-5" | "MP-6" => assert_splice_decode_kill(id, &frame),
            // Frame legal + write refused (not schema-required guards).
            "MP-7" => assert_mp7_007_plus_write(id, &probe["request"]),
            // D-C5 text-lawful answer; pack expect is superseded.
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
        [
            "MP-1", "MP-2", "MP-3", "MP-4", "MP-5", "MP-6", "MP-7", "MP-8"
        ],
        "every wire probe ran"
    );
}
