//! V2 §Q2: `view_path` is DAEMON-ONLY. The per-process sidecar is not the
//! persistent builder (OD6) and cannot publish — C2 forbids `sidecar`→`view`,
//! and an in-memory build has no filesystem path to forward. So the sidecar
//! refuses `view_path` LOUD with `daemon_only` (the `env` recovery class — start
//! the daemon), and never advertises it in its caps (§3.2 discovery honesty:
//! the sidecar's cap set is unchanged).

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

fn one(root: &fs::WorkspaceRoot, line: &str) -> Value {
    let mut out = Vec::new();
    sidecar::serve(root, format!("{line}\n").as_bytes(), &mut out, &[]).expect("serve");
    let frames: Vec<Value> = String::from_utf8(out)
        .expect("frames are UTF-8")
        .lines()
        .map(|l| serde_json::from_str(l).expect("frame parses"))
        .collect();
    assert_eq!(frames.len(), 1, "exactly one response per request");
    frames.into_iter().next().unwrap()
}

#[test]
fn sidecar_refuses_view_path_with_daemon_only() {
    let (_dir, root) = workspace(&[("a.md", "# A\n")]);
    let reply = one(
        &root,
        r#"{"id":1,"op":"view_path","cwd":"/some/workspace"}"#,
    );
    assert_eq!(
        reply["ok"],
        json!(false),
        "sidecar refuses view_path: {reply}"
    );
    assert_eq!(reply["id"], json!(1), "the refusal echoes the id: {reply}");
    assert_eq!(
        reply["error"]["code"],
        json!("daemon_only"),
        "view_path needs the daemon builder: {reply}"
    );
    assert_eq!(
        reply["error"]["recovery"],
        json!("env"),
        "daemon_only is the env recovery class (start the daemon): {reply}"
    );
}

#[test]
fn sidecar_caps_do_not_advertise_view_path() {
    let (_dir, root) = workspace(&[("a.md", "# A\n")]);
    let hello = one(&root, r#"{"id":1,"op":"hello","proto":1}"#);
    let caps = hello["body"]["caps"].as_array().expect("caps array");
    assert!(
        !caps.contains(&json!("view_path")),
        "the sidecar never advertises the daemon-only view_path: {hello}"
    );
}
