//! What survives of the composed `read` op's golden replay after the
//! Go-parity captures retired: the one gate that never touched a golden —
//! v3-only discovery honesty. `u14_read_face_contract` owns the read face's
//! laws over the same corpus; the TOON goldens re-pin the rendered face.

use serde_json::{Value, json};

/// Drive one v3 serve session over the doc's corpus dir; one frame per line.
fn serve(doc_dir: &std::path::Path, requests: &[Value]) -> Vec<Value> {
    let root = fs::WorkspaceRoot(doc_dir.to_path_buf());
    let mut input = String::from("{\"id\":0,\"op\":\"hello\",\"proto\":1,\"contract\":\"v3\"}\n");
    for r in requests {
        input.push_str(&serde_json::to_string(r).expect("request serializes"));
        input.push('\n');
    }
    let mut out = Vec::new();
    sidecar::serve(&root, input.as_bytes(), &mut out, &[]).expect("serve");
    String::from_utf8(out)
        .expect("frames are UTF-8")
        .lines()
        .map(|l| serde_json::from_str(l).expect("frame parses"))
        .collect()
}

/// §3.2 discovery honesty: a v2 session's `read` answers `unknown_op` (the
/// frozen v2 caps never list it), and a v2 hello's caps stay byte-frozen
/// while the v3 hello advertises `read`.
#[test]
fn composed_read_is_v3_only() {
    let dir = testsuite::parity_dir().join("corpus").join("basic");
    let root = fs::WorkspaceRoot(dir.clone());

    let mut out = Vec::new();
    sidecar::serve(
        &root,
        "{\"id\":1,\"op\":\"hello\",\"proto\":1}\n\
         {\"id\":2,\"op\":\"read\",\"path\":\"corpus/basic.md\"}\n"
            .as_bytes(),
        &mut out,
        &[],
    )
    .expect("serve");
    let frames: Vec<Value> = String::from_utf8(out)
        .expect("utf8")
        .lines()
        .map(|l| serde_json::from_str(l).expect("parses"))
        .collect();
    let caps_v2: Vec<&str> = frames[0]["body"]["caps"]
        .as_array()
        .expect("caps")
        .iter()
        .map(|c| c.as_str().expect("cap"))
        .collect();
    assert!(!caps_v2.contains(&"read"), "v2 caps never list read");
    assert_eq!(frames[1]["ok"], json!(false));
    assert_eq!(frames[1]["error"]["code"], json!("unknown_op"));

    let frames = serve(
        &dir,
        &[json!({"id":1,"op":"read","path":"corpus/basic.md"})],
    );
    let caps_v3: Vec<&str> = frames[0]["body"]["caps"]
        .as_array()
        .expect("caps")
        .iter()
        .map(|c| c.as_str().expect("cap"))
        .collect();
    assert!(caps_v3.contains(&"read"), "v3 caps advertise read");
    assert_eq!(
        frames[1]["ok"],
        json!(true),
        "v3 read serves: {}",
        frames[1]
    );
}
