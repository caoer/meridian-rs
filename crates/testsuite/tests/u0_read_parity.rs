//! U2 golden gate: the engine-side read facts against the U0 captured parity
//! goldens (`data/parity/`, copied byte-exact from ccc-statusd — see its
//! README). For every corpus doc, replay the goldens' PRE-PUT read steps:
//!
//! - toc mode: row-for-row `n`/`depth`/`title`/`hpath`/`words`/`sec_rev`
//!   against `structured.toc`, plus `rev` (`file_rev`) and `words_total`.
//! - sections mode: selector resolution order + content BYTES + recomputed
//!   words against `structured.sections`, and the unresolved-selector set
//!   against the captured `notice` line.
//!
//! Steps at or after the first `put` are the write plane's (state has
//! moved); refusal steps carry no structured payload — both are skipped
//! here and gated by the Go-side U0 harness at cutover.

use serde_json::Value;
use wire_map::facts::{ReadFact, read_facts, resolve_selector, section_content, toc_rows};

/// Every corpus doc with a captured golden.
fn docs() -> Vec<String> {
    let dir = testsuite::parity_dir().join("goldens");
    let mut out: Vec<String> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("goldens dir {}: {e}", dir.display()))
        .map(|e| {
            e.expect("dir entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter_map(|n| n.strip_suffix(".json").map(str::to_owned))
        .collect();
    out.sort();
    assert!(!out.is_empty(), "parity pack present");
    out
}

struct DocState {
    raw: String,
    facts: Vec<ReadFact>,
    file_rev: String,
}

fn build(doc: &str, rel: &str) -> DocState {
    let path = testsuite::parity_dir().join("corpus").join(doc).join(rel);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("corpus {}: {e}", path.display()));
    let raw = String::from_utf8(bytes).expect("corpus is UTF-8 (exact bytes)");
    let document = model::build(raw.clone(), syntax::parse(&raw));
    let facts = read_facts(&wire_map::project_toc(&document), raw.as_bytes());
    DocState {
        raw,
        facts,
        file_rev: document.root.node_rev.0,
    }
}

/// `ref` → (relative path, frag) — the host face's `splitRef`.
fn split_ref(r: &str) -> (&str, &str) {
    r.split_once('#').unwrap_or((r, ""))
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one linear golden replay, split adds nothing"
)]
fn read_facts_match_u0_goldens() {
    let mut toc_steps = 0;
    let mut section_steps = 0;
    for doc in docs() {
        let golden_path = testsuite::parity_dir()
            .join("goldens")
            .join(format!("{doc}.json"));
        let golden: Value =
            serde_json::from_str(&std::fs::read_to_string(&golden_path).expect("golden reads"))
                .expect("golden parses");
        let steps = golden["steps"].as_array().expect("steps");
        for step in steps {
            if step["tool"] == "put" {
                break; // state moves — the rest is the Go harness's to replay
            }
            if step["tool"] != "read" || step["is_error"] == Value::Bool(true) {
                continue;
            }
            let Some(structured) = step["structured"].as_object() else {
                continue;
            };
            let id = step["id"].as_str().unwrap_or("?");
            let args = &step["args"];
            let (rel, frag) = split_ref(args["ref"].as_str().expect("ref"));
            let state = build(&doc, rel);
            let ctx = format!("{doc}/{id}");

            assert_eq!(
                structured["rev"].as_str(),
                Some(state.file_rev.as_str()),
                "{ctx}: file_rev"
            );
            let words_total: u64 = state.facts.iter().map(|f| f.words).sum();
            assert_eq!(
                structured["words_total"].as_u64().unwrap_or(0),
                words_total,
                "{ctx}: words_total"
            );

            let mode = args["mode"].as_str().unwrap_or("toc");
            if mode == "toc" {
                toc_steps += 1;
                let want = structured
                    .get("toc")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let got = toc_rows(&state.facts, frag);
                assert_eq!(got.len(), want.len(), "{ctx}: toc row count");
                for (g, w) in got.iter().zip(&want) {
                    let gv = (
                        g.n.as_str(),
                        u64::from(g.depth),
                        g.title.as_str(),
                        g.hpath.as_str(),
                        g.words,
                    );
                    let wv = (
                        w["n"].as_str().expect("n"),
                        w["depth"].as_u64().expect("depth"),
                        w["title"].as_str().expect("title"),
                        w["hpath"].as_str().expect("hpath"),
                        w["words"].as_u64().expect("words"),
                    );
                    assert_eq!(gv, wv, "{ctx}: toc row");
                    assert_eq!(
                        g.sec_rev.as_str(),
                        w["sec_rev"].as_str().expect("sec_rev"),
                        "{ctx}: sec_rev for {}",
                        g.hpath
                    );
                }
            } else {
                section_steps += 1;
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
                let mut resolved: Vec<(&str, &ReadFact)> = Vec::new();
                let mut missing: Vec<&str> = Vec::new();
                for sel in &sels {
                    match resolve_selector(&state.facts, sel) {
                        Some(f) => resolved.push((sel, f)),
                        None => missing.push(sel),
                    }
                }
                let want = structured
                    .get("sections")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                assert_eq!(resolved.len(), want.len(), "{ctx}: resolved count");
                for ((sel, fact), w) in resolved.iter().zip(&want) {
                    assert_eq!(Some(*sel), w["sel"].as_str(), "{ctx}: sel");
                    assert_eq!(
                        Some(fact.hpath.as_str()),
                        w["hpath"].as_str(),
                        "{ctx}: hpath"
                    );
                    assert_eq!(
                        Some(fact.sec_rev.as_str()),
                        w["sec_rev"].as_str(),
                        "{ctx}: sec_rev"
                    );
                    let content = section_content(fact, state.raw.as_bytes());
                    assert_eq!(
                        String::from_utf8_lossy(&content),
                        w["content"].as_str().expect("content"),
                        "{ctx}: content bytes for {sel}"
                    );
                    // sections-mode words are recomputed over the returned
                    // content (readsidecar renderSectionsSidecar), so anchor
                    // rows count their block words, not the fact's 0.
                    assert_eq!(
                        wire_map::gotext::fields_count(&String::from_utf8_lossy(&content)) as u64,
                        w["words"].as_u64().expect("words"),
                        "{ctx}: section words for {sel}"
                    );
                }
                match structured.get("notice").and_then(Value::as_str) {
                    Some(notice) => {
                        let expect = format!(
                            "unresolved selectors (no rev minted): {}",
                            missing.join(", ")
                        );
                        assert_eq!(notice, expect, "{ctx}: unresolved set");
                    }
                    None => assert!(missing.is_empty(), "{ctx}: unexpected misses {missing:?}"),
                }
            }
        }
    }
    // the pack must actually exercise both modes — a silently-empty replay
    // would pass vacuously
    assert!(toc_steps >= 8, "toc steps replayed: {toc_steps}");
    assert!(
        section_steps >= 3,
        "section steps replayed: {section_steps}"
    );
}
