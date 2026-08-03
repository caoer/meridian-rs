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
//! The render face's production configuration elides the blocks THE ENGINE
//! EMITS (predicate: `lock::is_engine_emitted`), an INTENDED divergence from
//! the captured Go face on steps whose sections carry those blocks. The raw
//! captured truth stays untouched in `goldens/`; the elided expectation is
//! pinned alongside in `render-elided/<doc>.<step>.txt` and used INSTEAD
//! for exactly those steps. Raw-face gates (`u0_read_parity`, cat) never
//! elide — byte pin #4.
//!
//! **U36 SHRANK this divergence, and that is the measurement.** U4b keyed
//! elision on the `meridian-*` NAMESPACE, so `meridian-block.r-sections`
//! diverged from Go in TWO places: the `meridian-lock` block (ratified — the
//! engine writes it) and a `meridian-journal` block (over-applied — Go rendered
//! it, and nothing in the engine emits it). With elision derived per-language,
//! the second divergence is gone: the `Code` section is once again BYTE-EQUAL to
//! the captured Go text, `words:10` included, and the pin now differs from the
//! golden in exactly one place — the engine-written block. The re-based pin was
//! derived FROM the golden by eliding only that block, never from the engine's
//! own output.

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
/// The two steps whose refusal MESSAGE is no longer meridian-go's to pin.
///
/// **Ruling `d9419c03`, 2026-08-02 — an APPLICATION of ratified law, not new
/// authority.** Two independent sources agree: the meridian-rs end-state ruling
/// (`CLAUDE.md`, 2026-07-22 **plus § Amendment 2026-07-26**; llm-wiki
/// `decisions/2026-07-22-meridian-go-end-state.md`) says the remaining bridge
/// legs owe NO byte parity with meridian-go — and read/put is not among them: it
/// already migrated as a REDESIGNED CONTRACT (M1 `9a21d6d3`, stage-2
/// `ae0568c6`). So these two byte-pins were a bridge-era gate binding a
/// redesigned-contract leg. They were correct for a world that had already been
/// superseded.
///
/// **CONVERTED, not re-captured, and that distinction is the point.** Re-pinning
/// today's bytes as tomorrow's golden is a snapshot: it would block the NEXT
/// refusal improvement exactly as it blocked this one. Asserting the refusal's
/// PROPERTIES survives any honest rewording and still fails any regression below
/// the bar.
///
/// The goldens themselves are untouched — they remain the faithful record of
/// what meridian-go said. Only this crate's claim over the engine's wording is
/// narrowed, and only for these two steps.
///
/// `r-frag-plus-sections` joined them when site 8 of the same sweep was fixed:
/// its refusal is the same redesigned read contract, and its byte pin bound it
/// for the same superseded reason. Its properties differ from the section-miss
/// family's, so it gets its own contract rather than a loosened shared one.
fn refusal_is_redesigned(doc_name: &str, step_id: &str) -> bool {
    matches!(
        (doc_name, step_id),
        ("basic", "r-sections-all-missing")
            | ("empty", "r-sections")
            | ("basic", "r-frag-plus-sections")
    )
}

/// The both-selector-planes refusal's contract — same four properties, its own
/// subject. The caller passed a `#fragment` AND `sections[]`, so there is no ref
/// that "did not resolve": what the message owes instead is the CONTRADICTION it
/// found, in the terms the original taught.
///
/// The negative is the load-bearing half here too. This refusal used to name no
/// file at all, so asserting the file's presence is what fails if a later edit
/// drops the subject back out of the sentence.
fn assert_both_planes_contract(ctx: &str, message: &str) {
    assert!(
        message.contains("basic.md"),
        "{ctx}: names the file it is talking about: {message}"
    );
    assert!(
        message.contains("scopes the whole call") && message.contains("document-absolute"),
        "{ctx}: keeps the distinction the original taught: {message}"
    );
    assert!(
        message.contains("Nothing was read") && message.contains("no rev was minted"),
        "{ctx}: discloses the partial state: {message}"
    );
    assert!(
        message.contains("Fix:"),
        "{ctx}: carries a fix clause: {message}"
    );
    assert!(
        !message.contains("mode toc") && !message.contains("mode sections"),
        "{ctx}: the fix must NOT name an internal mode name: {message}"
    );
}

/// The engine's OWN refusal contract: the four properties the `mrd config`
/// exemplar sets — the ref that failed, the cause, the partial state, the fix —
/// plus the one negative the redesign exists to enforce.
fn assert_refusal_contract(ctx: &str, message: &str) {
    assert!(
        message.contains("Ghost"),
        "{ctx}: names the ref that did not resolve: {message}"
    );
    assert!(
        message.contains("no section addressed by"),
        "{ctx}: names the cause: {message}"
    );
    assert!(
        message.contains("Nothing was read") && message.contains("no rev was minted"),
        "{ctx}: discloses the partial state — a reader must not have to wonder \
         whether anything was served: {message}"
    );
    assert!(
        message.contains("Fix:"),
        "{ctx}: carries a fix clause: {message}"
    );
    assert!(
        message.contains("mrd read"),
        "{ctx}: the fix names a RUNNABLE COMMAND: {message}"
    );
    // The defect this whole redesign exists to kill: naming an internal mode the
    // caller never selected instead of a command they can type (issue-05).
    assert!(
        !message.contains("mode toc"),
        "{ctx}: the fix must NOT name an internal mode name: {message}"
    );
}

fn expected_text(doc_name: &str, step_id: &str, golden: &str) -> String {
    std::fs::read_to_string(
        testsuite::parity_dir()
            .join("render-elided")
            .join(format!("{doc_name}.{step_id}.txt")),
    )
    .unwrap_or_else(|_| golden.to_owned())
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one linear golden replay, split adds nothing"
)]
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

        // collect the replayable pre-put read steps:
        // (id, text, is_error, golden structured.sections — the RAW body pin)
        let mut requests: Vec<Value> = Vec::new();
        let mut expects: Vec<(String, String, bool, Option<Value>)> = Vec::new();
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
                (!step["structured"]["sections"].is_null())
                    .then(|| step["structured"]["sections"].clone()),
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
        for (i, (step_id, want_text, want_err, want_sections)) in expects.iter().enumerate() {
            let frame = &frames[i + 1];
            let ctx = format!("{doc_name}/{step_id}");
            if *want_err {
                assert_eq!(frame["ok"], json!(false), "{ctx}: refusal frame: {frame}");
                let got = frame["error"]["message"]
                    .as_str()
                    .unwrap_or_else(|| panic!("{ctx}: refusal carries a message: {frame}"));
                if refusal_is_redesigned(&doc_name, step_id) {
                    // Design test of the engine's own contract (ruling
                    // d9419c03) — properties, not bytes.
                    if *step_id == "r-frag-plus-sections" {
                        assert_both_planes_contract(&ctx, got);
                    } else {
                        assert_refusal_contract(&ctx, got);
                    }
                } else {
                    assert_eq!(
                        Some(got),
                        Some(want_text.as_str()),
                        "{ctx}: verbatim refusal message"
                    );
                }
                refusal_steps += 1;
            } else {
                assert_eq!(frame["ok"], json!(true), "{ctx}: ok frame: {frame}");
                assert_eq!(
                    frame["body"]["rendered_text"].as_str(),
                    Some(want_text.as_str()),
                    "{ctx}: rendered_text byte-equals the pinned expectation \
                     (captured Go text, or the elided render-divergent pin)"
                );
                // Op-owner ruling pin: `sections[].content` is the RAW face —
                // byte-equal to the CAPTURED golden structured sections even
                // where rendered_text carries the elided pin (meridian-block:
                // the fence bytes stay verbatim in the body; sec_rev/words
                // pair with that raw content).
                if let Some(want_sections) = want_sections {
                    let got = frame["body"]["sections"]
                        .as_array()
                        .unwrap_or_else(|| panic!("{ctx}: body sections: {frame}"));
                    let want = want_sections.as_array().expect("golden sections");
                    assert_eq!(got.len(), want.len(), "{ctx}: section row count");
                    for (g, w) in got.iter().zip(want) {
                        for key in ["sel", "hpath", "sec_rev", "words", "content"] {
                            assert_eq!(
                                g[key], w[key],
                                "{ctx}: RAW body `{key}` equals the captured golden"
                            );
                        }
                    }
                }
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
