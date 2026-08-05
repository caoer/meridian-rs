//! v2 consumer e2e via real sidecar binary: no `contract` in hello → v2.
//! Frozen vocabulary byte-for-byte; `fingerprint` nowhere in the session.

use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::Value;

/// The frozen ambient root of the §0.3 S0 workspace (contract v2 goldens).
const R0: &str = "b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9";
/// The root after the E3 splice (S0 → S1), frozen.
const R1: &str = "b3:10769ae1c77f5646750f3f52df2d055156b411145a02b8361ecd32af1357a1b7";

fn sidecar_bin() -> &'static str {
    env!("CARGO_BIN_EXE_sidecar")
}

fn wsfix(rel: &str) -> String {
    std::fs::read_to_string(testsuite::wsfix_dir().join(rel))
        .unwrap_or_else(|e| panic!("wsfix fixture {rel}: {e}"))
}

/// Materialize the §0.3 S0 workspace in a tempdir.
fn s0_workspace() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let files = [
        ("notes/plan.md", wsfix("s0/notes/plan.md")),
        ("receipts/2026-07-18.md", wsfix("s0/receipts/2026-07-18.md")),
        (".github/README.md", "# CI notes\n".to_owned()),
    ];
    for (rel, bytes) in files {
        let abs = dir.path().join(rel);
        std::fs::create_dir_all(abs.parent().expect("parent")).expect("mkdir");
        std::fs::write(abs, bytes).expect("write fixture");
    }
    dir
}

/// The v2 session: hello (NO `contract`), a read (`toc`), a write (`splice`).
/// The splice is the frozen §4.4 E3 exchange — a real CAS-guarded edit.
const V2_SESSION: &str = concat!(
    r#"{"id":1,"op":"hello","proto":1}"#,
    "\n",
    r#"{"id":2,"op":"toc","path":"notes/plan.md"}"#,
    "\n",
    r#"{"id":42,"op":"splice","path":"notes/plan.md","actor":"agent:b0864fb2","now":"2026-07-18T20:31:04Z","receipt":{"path":"receipts/2026-07-18.md","anchor":"r-000042"},"if_root":"b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9","edits":[{"target":{"hpath":[{"h":"Goals"},{"h":"Q3"}]},"edit":{"match":{"old":"ship by August","new":"ship by September"}},"if_node_rev":"33d5b0e1b27cb48b"}]}"#,
    "\n",
);

fn frame(frames: &[Value], id: u64) -> &Value {
    frames
        .iter()
        .find(|f| f["id"] == id)
        .unwrap_or_else(|| panic!("no response frame for id {id}"))
}

#[test]
fn sidecarv2_consumer_path_stays_root_never_fingerprint() {
    let ws = s0_workspace();

    let mut child = Command::new(sidecar_bin())
        .arg(ws.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn sidecar");

    {
        let mut stdin = child.stdin.take().expect("stdin");
        stdin
            .write_all(V2_SESSION.as_bytes())
            .expect("write session");
    } // drop stdin → EOF, sidecar finishes in-flight work and exits 0

    let out = child.wait_with_output().expect("sidecar output");
    assert!(
        out.status.success(),
        "sidecar exits 0 on stdin EOF: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).expect("frames are UTF-8");

    // The no-break constraint, stated at the raw-byte level: not one occurrence
    // of the v3 token anywhere in the v2 session's output.
    assert!(
        !stdout.contains("fingerprint"),
        "a contract-v2 session must never emit `fingerprint`:\n{stdout}"
    );

    let frames: Vec<Value> = stdout
        .lines()
        .map(|l| serde_json::from_str(l).expect("frame parses"))
        .collect();

    // hello: `root` present (frozen R0), no `fingerprint`, no `contract` echo.
    let hello = frame(&frames, 1);
    let hello_body = hello["body"].as_object().expect("hello body");
    assert_eq!(hello_body["root"], R0, "v2 hello carries `root`");
    assert!(!hello_body.contains_key("fingerprint"), "no v3 field");
    assert!(
        !hello_body.contains_key("contract"),
        "no v3 rev echo under v2"
    );

    // read: `toc` body carries the frozen `root`.
    let toc = frame(&frames, 2);
    assert_eq!(toc["body"]["root"], R0, "v2 read carries `root`");

    // write: `splice` body carries the frozen `root_before`/`root_after`.
    let splice = frame(&frames, 42);
    assert_eq!(splice["ok"], true, "the E3 splice applies: {splice}");
    assert_eq!(
        splice["body"]["root_before"], R0,
        "v2 write carries `root_before`"
    );
    assert_eq!(
        splice["body"]["root_after"], R1,
        "v2 write carries `root_after`"
    );
    let splice_body = splice["body"].as_object().expect("splice body");
    assert!(
        !splice_body.contains_key("fingerprint_before")
            && !splice_body.contains_key("fingerprint_after"),
        "no v3 write fields under v2"
    );
}
