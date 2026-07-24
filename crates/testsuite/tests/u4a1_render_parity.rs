//! U4a1 golden gate: the render plane's TEXT projection byte-for-byte
//! against the U0 captured goldens' `text` fields (the parity SURFACE — D's
//! capture note: parity surface = `text` + `is_error` + `file_after`).
//!
//! Same replay discipline as `u0_read_parity`: pre-put read steps only,
//! success frames only (refusal texts are the composed op's, gated at
//! U4a2/U8a). The goldens' `$SESSION` placeholder is passed through as the
//! display path verbatim — the engine renders the path string the caller
//! hands it.

use render::{Header, RenderJob, Renderer, SectionRow, TextRenderer};
use serde_json::Value;
use wire_map::facts::{read_facts, resolve_selector, toc_rows};

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

#[test]
fn rendered_text_matches_u0_goldens() {
    let mut replayed = 0;
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
        for step in golden["steps"].as_array().expect("steps") {
            if step["tool"] == "put" {
                break;
            }
            if step["tool"] != "read"
                || step["is_error"] == Value::Bool(true)
                || !step["structured"].is_object()
            {
                continue;
            }
            let id = step["id"].as_str().unwrap_or("?");
            let ctx = format!("{doc_name}/{id}");
            let args = &step["args"];
            let full_ref = args["ref"].as_str().expect("ref");
            let (rel, frag) = full_ref.split_once('#').unwrap_or((full_ref, ""));

            let path = testsuite::parity_dir()
                .join("corpus")
                .join(&doc_name)
                .join(rel);
            let raw = String::from_utf8(std::fs::read(&path).expect("corpus reads"))
                .expect("corpus is UTF-8");
            let doc = model::build(raw.clone(), syntax::parse(&raw));
            let facts = read_facts(&wire_map::project_toc(&doc), raw.as_bytes());

            let display_path = format!("$SESSION/{rel}");
            let words_total: u64 = facts.iter().map(|f| f.words).sum();
            let header = Header {
                display_path: &display_path,
                file_rev: &doc.root.node_rev.0,
                words_total,
            };

            let mode = args["mode"].as_str().unwrap_or("toc");
            let text = if mode == "toc" {
                let rows = toc_rows(&facts, frag);
                render::toc_text(&header, &rows)
            } else {
                let sels: Vec<String> = if frag.is_empty() {
                    args["sections"]
                        .as_array()
                        .expect("sections")
                        .iter()
                        .map(|s| s.as_str().expect("sel").to_owned())
                        .collect()
                } else {
                    vec![frag.to_owned()]
                };
                let mut rows: Vec<SectionRow<'_>> = Vec::new();
                let mut missing: Vec<&str> = Vec::new();
                for sel in &sels {
                    match resolve_selector(&facts, sel) {
                        Some(fact) => rows.push(SectionRow { sel, fact }),
                        None => missing.push(sel),
                    }
                }
                let notice = (!missing.is_empty()).then(|| {
                    format!(
                        "unresolved selectors (no rev minted): {}",
                        missing.join(", ")
                    )
                });
                let job = RenderJob::Sections {
                    header,
                    rows: &rows,
                    notice: notice.as_deref(),
                };
                TextRenderer::default()
                    .render(&doc, &job)
                    .unwrap_or_else(|e| panic!("{ctx}: {e}"))
                    .text
            };

            assert_eq!(
                text,
                step["text"].as_str().expect("text"),
                "{ctx}: rendered text is byte-identical to the captured Go readText"
            );
            replayed += 1;
        }
    }
    assert!(replayed >= 11, "render steps replayed: {replayed}");
}
