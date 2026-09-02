//! The one wire door, in-process.
//!
//! The stdio sidecar host was ruled DROP (`docs/wire-contract.md` §3.3): the
//! registry daemon's unix socket is the only wire door,
//! speaking the same NDJSON one-frame-per-line dialogue. The frame-layer tests
//! that drove the sidecar's serve loop byte-for-byte now drive the registry
//! host's connection-serve path (`registry::serve_lines` — the socket's own
//! line loop, transport-generic), so byte-level coverage lives on the door
//! that ships.

use std::path::Path;

/// One `hello` line binding `workspace` on the resident host (§3.3: the
/// registry binds per-connection at `hello`). `contract: Some("v3")` opens a
/// v3 session; `None` stays on the frozen v2 base vocabulary.
pub(crate) fn hello_line(contract: Option<&str>, workspace: &Path) -> String {
    let mut frame = serde_json::json!({
        "id": 0, "op": "hello", "proto": 1,
        "workspace": workspace.to_str().expect("workspace path is UTF-8"),
    });
    if let Some(rev) = contract {
        frame["contract"] = rev.into();
    }
    let mut line = frame.to_string();
    line.push('\n');
    line
}

/// Drive one NDJSON dialogue (`input`: one frame per line, `hello` included)
/// through the registry host's connection-serve path, over a fresh in-process
/// registry homed in its own tempdir. Returns the response bytes — one frame
/// per line, exactly what the daemon's socket would carry.
pub(crate) fn serve(input: &str) -> Vec<u8> {
    let home = tempfile::tempdir().expect("registry home");
    let config = registry::Config::for_cache_root(home.path().join("cache"));
    let reg = registry::in_process_registry(&config).expect("in-process registry");
    let mut out = Vec::new();
    registry::serve_lines(&reg, input.as_bytes(), &mut out, None).expect("serve");
    out
}

/// [`serve`], with each response line parsed as one JSON frame.
pub(crate) fn serve_frames(input: &str) -> Vec<serde_json::Value> {
    String::from_utf8(serve(input))
        .expect("frames are UTF-8")
        .lines()
        .map(|l| serde_json::from_str(l).expect("frame parses"))
        .collect()
}
