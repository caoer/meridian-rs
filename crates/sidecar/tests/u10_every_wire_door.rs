//! U10: every wire door enforces fingerprint-or-force — at the non-MCP door.
//!
//! Sidecar is plain stdio NDJSON (`Origin::Wire`); registry covers the MCP
//! socket. Law binds the door, not the client. Pins: guardless content change
//! → `guard_required` (semantic, frame stays legal); `force` lands here too.

use serde_json::{Value, json};
use std::io::Write as _;

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

fn serve(root: &fs::WorkspaceRoot, input: &str) -> Vec<Value> {
    let mut out = Vec::new();
    sidecar::serve(root, input.as_bytes(), &mut out, &[]).expect("serve");
    String::from_utf8(out)
        .expect("frames are UTF-8")
        .lines()
        .map(|l| serde_json::from_str(l).expect("frame parses"))
        .collect()
}

const DOC: &str = "# Memo\n\n## Tasks\n\n- item one\n";

fn guardless_splice() -> String {
    json!({
        "id": 1,
        "op": "splice",
        "path": "memo.md",
        "edits": [{
            "target": {"hpath": [{"h": "Memo"}, {"h": "Tasks"}]},
            "edit": {"match": {"old": "item one", "new": "item ONE"}},
        }]
    })
    .to_string()
}

/// Guardless content change refused at this door (`guard_required`).
#[test]
fn the_non_mcp_wire_door_enforces_too() {
    let (_d, root) = workspace(&[("memo.md", DOC)]);
    let frames = serve(&root, &format!("{}\n", guardless_splice()));

    assert_eq!(frames.len(), 1, "one response: {frames:?}");
    assert_eq!(
        frames[0]["ok"],
        json!(false),
        "the sidecar is a wire door and enforces: {:?}",
        frames[0]
    );
    assert_eq!(
        frames[0]["error"]["code"],
        json!("guard_required"),
        "the same refusal the MCP door raises: {:?}",
        frames[0]
    );

    // Nothing landed.
    assert_eq!(
        std::fs::read_to_string(root.0.join("memo.md")).expect("read"),
        DOC,
        "a refused write leaves the file byte-unchanged"
    );
}

/// Refusal is semantic (not `bad_frame`/`bad_request`): guard fields stay optional.
#[test]
fn the_refusal_is_semantic_the_frame_stays_legal() {
    let (_d, root) = workspace(&[("memo.md", DOC)]);
    let frames = serve(&root, &format!("{}\n", guardless_splice()));

    let code = frames[0]["error"]["code"].as_str().expect("an error code");
    assert_ne!(
        code, "bad_frame",
        "the frame was well-formed; saying otherwise violates the ruling"
    );
    assert_ne!(
        code, "bad_request",
        "the request was well-formed; the WRITE is what is refused"
    );
    assert_eq!(
        frames[0]["error"]["recovery"],
        json!("fix"),
        "fix class — change the request, not the channel: {:?}",
        frames[0]
    );
    assert_eq!(
        frames[0]["id"],
        json!(1),
        "the response correlates to the frame, which is only possible because \
         the frame decoded: {:?}",
        frames[0]
    );
}

/// `force` works at every door (not MCP-only).
#[test]
fn force_is_any_clients_rewrite_path() {
    let (_d, root) = workspace(&[("memo.md", DOC)]);
    let forced = json!({
        "id": 2,
        "op": "splice",
        "path": "memo.md",
        "force": true,
        "edits": [{
            "target": {"hpath": [{"h": "Memo"}, {"h": "Tasks"}]},
            "edit": {"match": {"old": "item one", "new": "item ONE"}},
        }]
    })
    .to_string();
    let frames = serve(&root, &format!("{forced}\n"));

    assert_eq!(
        frames[0]["ok"],
        json!(true),
        "force lands at every door: {:?}",
        frames[0]
    );
    assert!(
        std::fs::read_to_string(root.0.join("memo.md"))
            .expect("read")
            .contains("item ONE")
    );
}
