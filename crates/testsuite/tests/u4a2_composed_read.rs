//! U4a2 wire gate: the composed `read` op through the LIVE sidecar serve
//! loop, replayed against the U0 goldens — the U8a-grade parity surface:
//!
//! - success steps: `body.rendered_text` byte-equals the captured Go `text`
//!   (with the goldens' `$SESSION` placeholder passed as `display_path`).
//! - refusal steps: `error.message` carries the Go host face's VERBATIM
//!   string (`ok:false`), so the thin proxy forwards without re-minting.
//!   MCP-schema refusals (golden text starting "validating ") stay
//!   host-side and are skipped.
//!
//! Pre-put steps only (state never moves); one v3 session per doc.
//!
//! # Render-divergent-by-design steps (U4b, G5 class)
//! The render face's production configuration elides `meridian-*` blocks
//! (predicate: `lock::is_meridian_lang`), an INTENDED divergence from the
//! captured Go face on steps whose sections carry engine blocks. The raw
//! captured truth stays untouched in `goldens/`; the elided expectation is
//! pinned alongside in `render-elided/<doc>.<step>.txt` and used INSTEAD
//! for exactly those steps. Raw-face gates (`u0_read_parity`, cat) never
//! elide — byte pin #4.

use serde_json::{Value, json};

fn docs() -> Vec<String> {
    let dir = testsuite::parity_dir().join("goldens");
    let mut out: Vec<String> = std::fs::read_dir(&dir)
        .expect("goldens dir")
        .map(|e| {
            e.expect("dir entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter_map(|n| n.strip_suffix(".json").map(str::to_owned))
        .collect();
    out.sort();
    out
}

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

/// The expected `rendered_text` for a step: the pinned elided expectation
/// (`render-elided/<doc>.<step>.txt`, U4b render-divergent-by-design) when
/// present, else the captured Go golden text.
fn expected_text(doc_name: &str, step_id: &str, golden: &str) -> String {
    std::fs::read_to_string(
        testsuite::parity_dir()
            .join("render-elided")
            .join(format!("{doc_name}.{step_id}.txt")),
    )
    .unwrap_or_else(|_| golden.to_owned())
}

#[test]
fn composed_read_matches_u0_goldens_over_the_wire() {
    let mut ok_steps = 0;
    let mut refusal_steps = 0;
    for doc_name in docs() {
        let golden: Value = serde_json::from_str(
            &std::fs::read_to_string(
                testsuite::parity_dir()
                    .join("goldens")
                    .join(format!("{doc_name}.json")),
            )
            .expect("golden reads"),
        )
        .expect("golden parses");

        // collect the replayable pre-put read steps
        let mut requests: Vec<Value> = Vec::new();
        let mut expects: Vec<(String, String, bool)> = Vec::new(); // (id, text, is_error)
        for step in golden["steps"].as_array().expect("steps") {
            if step["tool"] == "put" {
                break;
            }
            if step["tool"] != "read" {
                continue;
            }
            let text = step["text"].as_str().expect("text");
            if text.starts_with("validating ") {
                continue; // MCP schema refusal — stays host-side
            }
            let args = &step["args"];
            let full_ref = args["ref"].as_str().expect("ref");
            let (rel, frag) = full_ref.split_once('#').unwrap_or((full_ref, ""));
            let mut req = json!({
                "id": requests.len() + 1,
                "op": "read",
                "path": rel,
                "display_path": format!("$SESSION/{rel}"),
            });
            if let Some(mode) = args["mode"].as_str() {
                req["mode"] = json!(mode);
            }
            if !frag.is_empty() {
                req["frag"] = json!(frag);
            }
            if let Some(sections) = args["sections"].as_array() {
                req["sections"] = json!(sections);
            }
            requests.push(req);
            let step_id = step["id"].as_str().unwrap_or("?");
            expects.push((
                step_id.to_owned(),
                expected_text(&doc_name, step_id, text),
                step["is_error"] == Value::Bool(true),
            ));
        }
        if requests.is_empty() {
            continue;
        }

        let doc_dir = testsuite::parity_dir().join("corpus").join(&doc_name);
        let frames = serve(&doc_dir, &requests);
        assert_eq!(
            frames.len(),
            requests.len() + 1,
            "{doc_name}: one frame per request (hello + reads)"
        );
        for (i, (step_id, want_text, want_err)) in expects.iter().enumerate() {
            let frame = &frames[i + 1];
            let ctx = format!("{doc_name}/{step_id}");
            if *want_err {
                assert_eq!(frame["ok"], json!(false), "{ctx}: refusal frame: {frame}");
                assert_eq!(
                    frame["error"]["message"].as_str(),
                    Some(want_text.as_str()),
                    "{ctx}: verbatim refusal message"
                );
                refusal_steps += 1;
            } else {
                assert_eq!(frame["ok"], json!(true), "{ctx}: ok frame: {frame}");
                assert_eq!(
                    frame["body"]["rendered_text"].as_str(),
                    Some(want_text.as_str()),
                    "{ctx}: rendered_text byte-equals the pinned expectation \
                     (captured Go text, or the elided render-divergent pin)"
                );
                // D6 atomicity witness: file_rev + fingerprint at one snapshot
                assert!(
                    frame["body"]["file_rev"].is_string()
                        && frame["body"]["fingerprint"].is_string(),
                    "{ctx}: file_rev + fingerprint ride the body: {frame}"
                );
                // U7: the composed read is a dispatched op — timing rides
                assert!(
                    frame["meta"]["duration_us"].is_u64(),
                    "{ctx}: meta.duration_us rides: {frame}"
                );
                ok_steps += 1;
            }
        }
    }
    assert!(ok_steps >= 11, "ok steps replayed: {ok_steps}");
    assert!(
        refusal_steps >= 3,
        "refusal steps replayed: {refusal_steps}"
    );
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
