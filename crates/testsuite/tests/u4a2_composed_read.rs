//! U4a2 wire gate, POST-RETIREMENT (U14, docket row P9): what survives of the composed
//! `read` op's golden replay after the 11 Go-parity captures retired.
//!
//! # What retired, and why it could not be updated
//! This file replayed `data/parity/goldens/` through the live sidecar serve loop:
//! `body.rendered_text` byte-equal to meridian-go's captured `text`, the raw `sections[]`
//! body byte-equal to its captured `structured.sections`, and refusal messages verbatim.
//! Every replayed step fed a CAPTURED REQUEST — `args["ref"]`, `args["sections"]`, joined
//! strings — into the `read` op. U14 made that op take structured selectors (meridian-rs
//! end-state ruling, 2026-07-22 + § Amendment 2026-07-26).
//!
//! The `render-elided/` pins retired with them — they were derivations OF the captured
//! text, so they could outlive neither their source nor the requests that produced them.
//!
//! # What replaced it
//! `u14_read_face_contract` — the read face's own laws over the SAME corpus (the corpus
//! is hand-built input, not a capture, and stays). This file keeps the one gate that
//! never touched a golden: v3-only discovery honesty. U15 mints the TOON goldens that
//! re-pin the rendered face.
//!
//! Precedent followed exactly: `refusal_is_redesigned` (ruling `d9419c03`) and the
//! U0/U5/U5b refusal-pin retirements before it — per-case contracts added, no shared
//! assertion loosened.
//!
//!
//!
//!
//!

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
