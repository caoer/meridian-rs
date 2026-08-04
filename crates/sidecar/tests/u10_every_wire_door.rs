//! U10: **every wire door enforces** — asserted at the door that is NOT MCP.
//!
//! The ruling (ZT, 2026-08-03), verbatim:
//!
//! > Content-mutating writes on every wire door require fingerprint match or
//! > force; guard fields stay schema-optional; force is any client's
//! > refuse→rewrite path; MCP is the main agent client that implements that
//! > path, not a separate trust plane.
//!
//! The resident daemon's socket is the MCP door and its enforcement is covered
//! by the registry suite. THIS suite covers the other one: the per-workspace
//! sidecar is a plain stdio NDJSON host — `sidecar <workspace-root>` reading any
//! stdin, historically driven by the meridian-go bridge, with no MCP coupling
//! whatsoever. It enforces identically, and that is the whole point of the
//! ruling: the law binds the DOOR, never the client behind it.
//!
//! Without this file the sidecar's enforcement is only an implementation detail
//! of one `Origin::Wire` literal in `arms.rs`, which a later reader could
//! "simplify" back to exempt on the reasoning that it is not the MCP path.

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

/// The non-MCP wire door refuses a guardless content change, exactly as the MCP
/// door does. No door is exempted for who is behind it.
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

/// …and the refusal is SEMANTIC, not frame-illegality. The frame decoded — it
/// reached the write path and was answered there. Decision 007's schema half is
/// intact: guard fields stay optional and a guardless frame is legal.
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

/// `force` is any client's refuse→rewrite path — it is not an MCP affordance,
/// so it works at this door too.
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
