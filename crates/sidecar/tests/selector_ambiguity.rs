//! U2.2 — selector grammar + ambiguity through live serve: duplicate heads,
//! refuse-ambiguous-only, node-index disambiguation, dangling refuse, taxonomy
//! (`docs/wire-contract.md`).

use std::io::Write as _;
use std::path::Path;

use serde_json::Value;

/// A duplicate-headed file: two `## Objective` under `# Task`, each carrying a
/// distinct block id, plus an unambiguous `## Notes` sibling.
const DUP: &str = "# Task\n\n## Objective\n\nalpha ^a1b2c3\n\n## Objective\n\nbeta ^d4e5f6\n\n## Notes\n\ngamma\n";

fn workspace(files: &[(&str, &str)]) -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    for (rel, bytes) in files {
        let abs = dir.path().join(rel);
        std::fs::create_dir_all(abs.parent().expect("parent")).expect("mkdir");
        let mut f = std::fs::File::create(&abs).expect("create");
        f.write_all(bytes.as_bytes()).expect("write");
    }
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    (dir, root)
}

/// One request line → one response frame through the full serve loop.
fn one(root: &fs::WorkspaceRoot, line: &str) -> Value {
    let mut out = Vec::new();
    sidecar::serve(root, format!("{line}\n").as_bytes(), &mut out, &[]).expect("serve");
    let text = String::from_utf8(out).expect("frames are UTF-8");
    let mut frames: Vec<Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).expect("frame parses"))
        .collect();
    assert_eq!(frames.len(), 1, "exactly one response per request");
    frames.remove(0)
}

fn splice(path: &str, target: &Value) -> String {
    serde_json::to_string(&serde_json::json!({
        "id": 1, "op": "splice", "path": path,
        "edits": [{"target": target, "edit": {"put": {"at": "content", "text": "x\n"}}}],
    }))
    .unwrap()
}

/// U10: a wire-origin write on EXISTING content carries the target node's
/// fingerprint. Only the writes that are meant to SERVE need it — a write at an
/// unresolvable or ambiguous selector is answered by the selector's own refusal,
/// which is the rung this file exists to gate.
fn guarded_splice(root: &fs::WorkspaceRoot, path: &str, target: &Value) -> String {
    let doc = fs::load(root, Path::new(path)).expect("load");
    let sec: wire::SecRef = serde_json::from_value(target.clone()).expect("target decodes");
    let rev = match wire_serve::read::cat(&doc, Some(sec)).expect("cat") {
        wire::ResponseBody::Cat { node_rev, .. } => node_rev.0,
        other => panic!("cat returned {other:?}"),
    };
    let mut v: Value = serde_json::from_str(&splice(path, target)).expect("frame");
    v["edits"][0]["if_node_rev"] = Value::String(rev);
    v.to_string()
}

/// THE load-bearing gate: a write at the AMBIGUOUS selector refuses
/// `ambiguous_ref`, naming BOTH candidates by node index (`n=`) AND block id
/// (`^block`), while a write at the unambiguous sibling still SERVES.
#[test]
fn ambiguous_selector_refuses_naming_both_candidates_sibling_serves() {
    let (_dir, root) = workspace(&[("notes/dup.md", DUP)]);

    // (1) write at the ambiguous bare heading → refuse, naming both duplicates.
    let amb = one(
        &root,
        &splice(
            "notes/dup.md",
            &serde_json::json!({"hpath": [{"h": "Task"}, {"h": "Objective"}]}),
        ),
    );
    assert_eq!(amb["ok"], Value::Bool(false), "ambiguous write refuses");
    let err = &amb["error"];
    assert_eq!(err["code"], "ambiguous_ref");
    assert_eq!(err["recovery"], "fix");

    // candidates carry the machine-addressable `n=` forms, one per duplicate.
    let cands = err["candidates"].as_array().expect("candidates array");
    assert_eq!(cands.len(), 2, "both duplicates named");
    let last_n = |c: &Value| c["hpath"].as_array().unwrap().last().unwrap()["n"].clone();
    assert_eq!(last_n(&cands[0]), Value::from(1));
    assert_eq!(last_n(&cands[1]), Value::from(2));

    // the teaching refusal names each candidate by n= AND ^block, verbatim d1.
    let msg = err["message"].as_str().expect("teaching message");
    assert!(
        msg.contains("selector 'Task/Objective' is ambiguous — 2 matches:"),
        "{msg}"
    );
    assert!(
        msg.contains("n=1 (^a1b2c3)"),
        "candidate 1 named by n= + ^block: {msg}"
    );
    assert!(
        msg.contains("n=2 (^d4e5f6)"),
        "candidate 2 named by n= + ^block: {msg}"
    );
    assert!(
        msg.ends_with(
            "Unambiguous writes to this file remain served. Fix: address the duplicate \
             by block id or node index and rename one heading; see [[selector-grammar]]."
        ),
        "carries the verbatim d1 teaching tail: {msg}"
    );

    // (2) write at the UNAMBIGUOUS sibling still serves — the F6 fix.
    let sib = one(
        &root,
        &guarded_splice(
            &root,
            "notes/dup.md",
            &serde_json::json!({"hpath": [{"h": "Task"}, {"h": "Notes"}]}),
        ),
    );
    assert_eq!(
        sib["ok"],
        Value::Bool(true),
        "an unambiguous sibling write SERVES despite the file's duplicate heading (F6 fix): {sib}"
    );

    // (3) the duplicate is addressable by node index (`n=`) — the write serves.
    let by_index = one(
        &root,
        &guarded_splice(
            &root,
            "notes/dup.md",
            &serde_json::json!({"hpath": [{"h": "Task"}, {"h": "Objective", "n": 1}]}),
        ),
    );
    assert_eq!(
        by_index["ok"],
        Value::Bool(true),
        "node-index addressing disambiguates the duplicate and serves: {by_index}"
    );
}

/// A write whose own selector resolves to nothing (a dangling `^block-id`)
/// refuses `ref_not_found` — the write-door code the taxonomy binds to
/// `red(dangling-anchor)` (amendment row 2, refresh).
#[test]
fn dangling_anchor_selector_refuses_ref_not_found() {
    let (_dir, root) = workspace(&[("notes/dup.md", DUP)]);
    let frame = one(
        &root,
        &splice("notes/dup.md", &serde_json::json!({"anchor": "gone"})),
    );
    assert_eq!(frame["ok"], Value::Bool(false));
    assert_eq!(frame["error"]["code"], "ref_not_found");
    assert_eq!(frame["error"]["recovery"], "refresh");
}

/// Taxonomy conformance (U4.1 law): every refusal reason U2.2 touches binds to a
/// row in `docs/wire-contract.md`, and the frozen wire
/// `code → recovery` binding matches the class the amendment declares. No code
/// is invented; each is reused with its declared recovery class.
#[test]
fn refusals_conform_to_the_amendment_taxonomy() {
    use wire::{ErrorCode, Recovery};

    let doc = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/wire-contract.md"),
    )
    .expect("refusal amendment doc");

    // The row for `code` declares recovery class `class` (one table row).
    let row_binds = |code: &str, class: &str| -> bool {
        doc.lines()
            .filter(|l| l.contains(code))
            .any(|l| l.contains(&format!("| {class} |")))
    };

    // ambiguity refusal → row 1 (ambiguous_ref, fix).
    assert!(
        row_binds("ambiguous_ref{candidates}", "fix"),
        "row 1 present"
    );
    assert_eq!(ErrorCode::AmbiguousRef.recovery(), Recovery::Fix);

    // red(dangling-anchor) → row 2 (ref_not_found, refresh).
    assert!(
        row_binds("ref_not_found{stage,dest?}", "refresh"),
        "row 2 present"
    );
    assert_eq!(ErrorCode::RefNotFound.recovery(), Recovery::Refresh);

    // red(selector-unresolved) → row 3 (no_match, fix) — the status/pin-plane
    // code bound to the color reason (U2.2 lands the color; later units mint the
    // pin-door no_match). The code's recovery class matches the amendment.
    assert!(row_binds("`no_match`", "fix"), "row 3 present");
    assert_eq!(ErrorCode::NoMatch.recovery(), Recovery::Fix);
}
