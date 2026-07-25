//! S1 gate (stage-2), as amended by s1c: the composed read carries the AUTHZ
//! FACTS — canonical hpaths, byte spans, and the `^id` ANCHORS — so
//! ccc-statusd's put authz derives governing sections from the engine's facts
//! instead of its own markdown mirror (`sanitizeHeadingHost`, M1 residual #4).
//!
//! Shaped after the Go-side drift guard the mirror-removal retires
//! (`ccc-statusd/internal/mcpserver/sanitize_drift_guard_test.go`): over the
//! SAME parity corpus, one live serve session per corpus root, comparing the
//! HOST mirror's own input — the v2 `toc` op's RAW hpath segments, sanitized
//! through the engine's single owner `model::gotext::sanitize_heading` (S0) —
//! against the v3 composed-read row's canonical `hpath`.
//!
//! It then gates what the guard could not, because the row did not carry it:
//!
//! - every heading row and every anchor carries a `span`, byte-equal to the
//!   v2 toc node's span for the same node — so the substitution the host
//!   makes is fact-for-fact, not merely address-for-address;
//! - the `^id` anchors are SURFACED (toc mode served headings only), with the
//!   Go switch's drops intact (task/paragraph-hosted anchors stay
//!   unaddressable);
//! - `containingSectionTitles` (`puttoc.go:86`) computed from the v3 facts
//!   ALONE equals the same walk over the v2 nodes the host reads today;
//! - `rendered_text` never grows an anchor row — the captured Go toc bytes
//!   are frozen.
//!
//! **s1c amends S1's ONE assertion that was wrong.** S1 put the anchor rows
//! in the `toc` array beside the headings, discriminated by `depth == 0`;
//! ccc-statusd's `readText` indents by `strings.Repeat("  ", depth-1)` and
//! panicked "negative Repeat count" on the first file with a block anchor.
//! The anchors now ride their own always-emitted `anchors[]`, so this file
//! gates the SHAPE, not a discriminator: `toc` is heading-only over the whole
//! corpus, `anchors[]` is present on every read, and the containment join
//! still agrees with the v2 nodes across the two planes.

use model::gotext::sanitize_heading;
use serde_json::{Value, json};

/// Drive one v3 serve session over a corpus root; one frame per request.
fn serve(root_dir: &std::path::Path, requests: &[Value]) -> Vec<Value> {
    let root = fs::WorkspaceRoot(root_dir.to_path_buf());
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

/// The corpus roots (one workspace each), sorted.
fn corpus_roots() -> Vec<String> {
    let dir = testsuite::parity_dir().join("corpus");
    let mut out: Vec<String> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("corpus dir {}: {e}", dir.display()))
        .map(|e| e.expect("dir entry"))
        .filter(|e| e.file_type().expect("file type").is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    out.sort();
    assert!(!out.is_empty(), "parity corpus present");
    out
}

/// Every `.md` under a root, workspace-relative and sorted — the drift
/// guard's own discovery.
fn md_files(root: &std::path::Path) -> Vec<String> {
    fn walk(dir: &std::path::Path, base: &std::path::Path, out: &mut Vec<String>) {
        let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
        for entry in entries {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                walk(&path, base, out);
            } else if path.extension().is_some_and(|e| e == "md") {
                let rel = path.strip_prefix(base).expect("under base");
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

/// `sanitizedHPath` (`puttoc.go:143`), rebuilt from the v2 toc node's RAW
/// hpath segments through the engine's ONE owner — the host mirror's exact
/// computation, minus the host's copy of the predicate.
fn sanitized_hpath(node: &Value) -> String {
    node["hpath"]
        .as_array()
        .map(|segs| {
            segs.iter()
                .map(|s| sanitize_heading(s["h"].as_str().expect("raw heading segment")))
                .collect::<Vec<_>>()
                .join("/")
        })
        .unwrap_or_default()
}

/// `containingSectionTitles` (`puttoc.go:104`): every heading whose span
/// contains the block's start byte, in document order. `headings` is
/// `(span_start, span_end, title)`.
fn containing_titles(headings: &[(u64, u64, String)], block_start: u64) -> Vec<String> {
    headings
        .iter()
        .filter(|(start, end, _)| *start <= block_start && block_start < *end)
        .map(|(_, _, title)| title.clone())
        .collect()
}

fn span(v: &Value) -> (u64, u64) {
    let a = v.as_array().unwrap_or_else(|| panic!("span array: {v}"));
    (
        a[0].as_u64().expect("span start"),
        a[1].as_u64().expect("span end"),
    )
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one linear corpus replay, split adds nothing"
)]
fn composed_read_rows_carry_authz_facts_over_the_drift_guard_corpus() {
    let mut files_checked = 0_u32;
    let mut headings_checked = 0_u32;
    let mut anchor_rows_checked = 0_u32;
    let mut dropped_anchors = 0_u32;

    for root_name in corpus_roots() {
        let root_dir = testsuite::parity_dir().join("corpus").join(&root_name);
        let files = md_files(&root_dir);
        if files.is_empty() {
            continue;
        }
        // Both faces in ONE session: the v2 `toc` op (the host mirror's
        // production input) and the v3 composed `read` (the row set that
        // replaces it).
        let mut requests: Vec<Value> = Vec::new();
        for rel in &files {
            requests.push(json!({"id": requests.len() + 1, "op": "toc", "path": rel}));
            requests
                .push(json!({"id": requests.len() + 1, "op": "read", "path": rel, "mode": "toc"}));
        }
        let frames = serve(&root_dir, &requests);
        assert_eq!(
            frames.len(),
            requests.len() + 1,
            "{root_name}: one frame per request (hello + toc/read per file)"
        );

        for (i, rel) in files.iter().enumerate() {
            let ctx = format!("{root_name}/{rel}");
            let toc = &frames[i * 2 + 1];
            let read = &frames[i * 2 + 2];
            assert_eq!(toc["ok"], json!(true), "{ctx}: toc frame: {toc}");
            assert_eq!(read["ok"], json!(true), "{ctx}: read frame: {read}");
            let raw_len = std::fs::metadata(root_dir.join(rel))
                .unwrap_or_else(|e| panic!("{ctx}: {e}"))
                .len();
            let nodes = toc["body"]["nodes"].as_array().expect("toc nodes");
            let rows = read["body"]["toc"].as_array().expect("read rows");
            // s1c gate: `anchors[]` is ALWAYS emitted — a document with no
            // addressable block anchor answers `[]`, never an absent field a
            // caller has to negotiate for.
            let anchors = read["body"]["anchors"]
                .as_array()
                .unwrap_or_else(|| panic!("{ctx}: `anchors` rides every composed read: {read}"));
            files_checked += 1;

            // --- headings: the drift-guard equality, engine-side ------------
            // s1c gate: `toc` is the HEADING plane, whole. Nothing filters
            // here — an anchor row reaching this array is the panic that
            // shipped, so the assertion is that the array cannot hold one.
            let v2_headings: Vec<&Value> =
                nodes.iter().filter(|n| n["kind"] == "heading").collect();
            for r in rows {
                assert!(
                    r["depth"].as_u64().expect("depth") > 0,
                    "{ctx}: a depth-0 row reached `toc` — ccc-statusd's readText \
                     indents by depth-1 and panics on it: {r}"
                );
                assert!(
                    r.get("anchor").is_none(),
                    "{ctx}: an anchor fact reached a `toc` row: {r}"
                );
                assert!(
                    !r["hpath"].as_str().expect("row hpath").starts_with('^'),
                    "{ctx}: a `^id` address reached `toc`: {r}"
                );
            }
            let v3_headings: Vec<&Value> = rows.iter().collect();
            assert_eq!(
                v2_headings.len(),
                v3_headings.len(),
                "{ctx}: heading count — host mirror sees {}, engine rows {}",
                v2_headings.len(),
                v3_headings.len()
            );
            for (n, r) in v2_headings.iter().zip(&v3_headings) {
                assert_eq!(
                    sanitized_hpath(n),
                    r["hpath"].as_str().expect("row hpath"),
                    "{ctx}: host-mirror sanitized hpath != engine canonical row hpath"
                );
                assert_eq!(n["span"], r["span"], "{ctx}: heading row span != node span");
                assert_eq!(
                    n.get("content_span"),
                    r.get("content_span"),
                    "{ctx}: heading row content_span != node content_span"
                );
                let (start, end) = span(&r["span"]);
                assert!(
                    start < end && end <= raw_len,
                    "{ctx}: heading span [{start},{end}) in bounds of {raw_len}"
                );
                headings_checked += 1;
            }

            // --- the anchor plane: surfaced, with the Go switch's drops intact
            let v2_anchors: Vec<&Value> = nodes
                .iter()
                .filter(|n| n["kind"] == "list_item" && n["anchor"].is_string())
                .collect();
            assert_eq!(
                v2_anchors.len(),
                anchors.len(),
                "{ctx}: addressable anchor count — v2 nodes {}, v3 anchors {}",
                v2_anchors.len(),
                anchors.len()
            );
            for (n, a) in v2_anchors.iter().zip(anchors) {
                let id = n["anchor"].as_str().expect("anchor id");
                assert_eq!(a["anchor"].as_str(), Some(id), "{ctx}: anchor id");
                assert_eq!(n["span"], a["span"], "{ctx}: anchor span != node span");
                let (start, end) = span(&a["span"]);
                assert!(
                    start < end && end <= raw_len,
                    "{ctx}: anchor span [{start},{end}) in bounds of {raw_len}"
                );
                anchor_rows_checked += 1;
            }
            // A task/paragraph-hosted anchor is NOT addressable on this face
            // (the Go switch default) — surfacing the anchor plane must not
            // smuggle one in, on EITHER array.
            for n in nodes
                .iter()
                .filter(|n| n["anchor"].is_string() && n["kind"] != "list_item")
            {
                let id = n["anchor"].as_str().expect("anchor id");
                assert!(
                    !anchors.iter().any(|a| a["anchor"].as_str() == Some(id)),
                    "{ctx}: `{id}` is hosted by kind {} — it stays unaddressable",
                    n["kind"]
                );
                dropped_anchors += 1;
            }

            // --- governing sections: the v3 two-plane join == the v2 nodes --
            // The join is absolute-byte arithmetic across the arrays, which is
            // exactly why s1c could split them: containment never read
            // document order, only spans.
            let from_rows: Vec<(u64, u64, String)> = v3_headings
                .iter()
                .map(|r| {
                    let (s, e) = span(&r["span"]);
                    (s, e, r["title"].as_str().expect("row title").to_owned())
                })
                .collect();
            let from_nodes: Vec<(u64, u64, String)> = v2_headings
                .iter()
                .map(|n| {
                    let (s, e) = span(&n["span"]);
                    let title = n["hpath"]
                        .as_array()
                        .and_then(|segs| segs.last())
                        .map(|s| s["h"].as_str().expect("raw heading").to_owned())
                        .unwrap_or_default();
                    (s, e, title)
                })
                .collect();
            for a in anchors {
                let (block_start, _) = span(&a["span"]);
                assert_eq!(
                    containing_titles(&from_rows, block_start),
                    containing_titles(&from_nodes, block_start),
                    "{ctx}: governing sections for ^{} diverge between the v3 two-plane \
                     join and the v2 nodes the host reads today",
                    a["anchor"]
                );
            }

            // --- render stays frozen: the anchor plane never renders --------
            let text = read["body"]["rendered_text"]
                .as_str()
                .expect("rendered_text");
            for a in anchors {
                let addr = format!("^{}", a["anchor"].as_str().expect("anchor id"));
                assert!(
                    !text.contains(&addr),
                    "{ctx}: `{addr}` must not reach rendered_text: {text}"
                );
            }
        }
    }

    // Vacuity guards: a discovery break must not read as a pass.
    assert!(files_checked >= 10, "files checked: {files_checked}");
    assert!(
        headings_checked >= 30,
        "headings checked: {headings_checked}"
    );
    assert!(
        anchor_rows_checked >= 2,
        "anchor rows checked: {anchor_rows_checked}"
    );
    assert!(
        dropped_anchors >= 2,
        "non-list_item anchors verified dropped: {dropped_anchors}"
    );
    println!(
        "S1 authz facts: {files_checked} files, {headings_checked} heading rows, \
         {anchor_rows_checked} anchor rows, {dropped_anchors} anchors verified unaddressable"
    );
}

/// The heading rows of the EXTENDED wire row set, against the U0 goldens'
/// captured Go `structured.toc` — the host's own bytes, so "host hpath ==
/// engine row hpath" is pinned to what the Go read face actually emitted, not
/// to a second engine-side derivation. The M1 fact set must be untouched by
/// the added authz facts, and every row must carry a span.
#[test]
fn extended_rows_match_the_captured_go_toc_rows() {
    let goldens = testsuite::parity_dir().join("goldens");
    let mut docs: Vec<String> = std::fs::read_dir(&goldens)
        .unwrap_or_else(|e| panic!("goldens dir {}: {e}", goldens.display()))
        .filter_map(|e| {
            e.expect("dir entry")
                .file_name()
                .to_string_lossy()
                .strip_suffix(".json")
                .map(str::to_owned)
        })
        .collect();
    docs.sort();
    let mut steps_replayed = 0_u32;
    let mut rows_compared = 0_u32;

    for doc in docs {
        let golden: Value = serde_json::from_str(
            &std::fs::read_to_string(goldens.join(format!("{doc}.json"))).expect("golden reads"),
        )
        .expect("golden parses");
        let mut requests: Vec<Value> = Vec::new();
        let mut expects: Vec<(String, Value)> = Vec::new();
        for step in golden["steps"].as_array().expect("steps") {
            if step["tool"] == "put" {
                break; // state moves — the rest is the write plane's
            }
            if step["tool"] != "read"
                || step["is_error"] == Value::Bool(true)
                || step["args"]["mode"].as_str().unwrap_or("toc") != "toc"
            {
                continue;
            }
            let Some(want) = step["structured"]["toc"].as_array() else {
                continue;
            };
            let full_ref = step["args"]["ref"].as_str().expect("ref");
            let (rel, frag) = full_ref.split_once('#').unwrap_or((full_ref, ""));
            let mut req =
                json!({"id": requests.len() + 1, "op": "read", "path": rel, "mode": "toc"});
            if !frag.is_empty() {
                req["frag"] = json!(frag);
            }
            requests.push(req);
            expects.push((
                step["id"].as_str().unwrap_or("?").to_owned(),
                json!(want.clone()),
            ));
        }
        if requests.is_empty() {
            continue;
        }
        let frames = serve(
            &testsuite::parity_dir().join("corpus").join(&doc),
            &requests,
        );
        for (i, (step_id, want)) in expects.iter().enumerate() {
            let ctx = format!("{doc}/{step_id}");
            let frame = &frames[i + 1];
            assert_eq!(frame["ok"], json!(true), "{ctx}: ok frame: {frame}");
            // No filter: s1c makes `toc` heading-only, so the array itself
            // must line up row-for-row with the captured Go table.
            let got: Vec<&Value> = frame["body"]["toc"]
                .as_array()
                .unwrap_or_else(|| panic!("{ctx}: rows: {frame}"))
                .iter()
                .collect();
            let want = want.as_array().expect("golden rows");
            assert_eq!(got.len(), want.len(), "{ctx}: heading row count");
            for (g, w) in got.iter().zip(want) {
                for key in ["n", "depth", "title", "hpath", "words", "sec_rev"] {
                    assert_eq!(g[key], w[key], "{ctx}: `{key}` on row {}", w["hpath"]);
                }
                assert!(
                    g["span"].is_array(),
                    "{ctx}: the authz span rides row {}: {g}",
                    w["hpath"]
                );
                rows_compared += 1;
            }
            steps_replayed += 1;
        }
    }
    assert!(steps_replayed >= 8, "toc steps replayed: {steps_replayed}");
    assert!(rows_compared >= 20, "rows compared: {rows_compared}");
    println!("S1 vs captured Go rows: {steps_replayed} toc steps, {rows_compared} heading rows");
}

/// A `frag`-scoped read carries the anchors of that subtree ONLY — the
/// scoping predicate is the host's own byte containment, so a scoped read is
/// still a complete authz fact set for what it returns. Each plane answers
/// its own question: `toc` the scoped headings, `anchors` the scoped blocks.
#[test]
fn frag_scoped_read_carries_only_the_subtree_anchors() {
    let root_dir = testsuite::parity_dir().join("corpus").join("trailing-ws");
    let rel = "corpus/trailing-ws.md";
    let frames = serve(
        &root_dir,
        &[
            json!({"id": 1, "op": "read", "path": rel, "mode": "toc", "frag": "Anchor-Zone"}),
            json!({"id": 2, "op": "read", "path": rel, "mode": "toc", "frag": "Padded-Title"}),
        ],
    );
    let headings = |frame: &Value| -> Vec<String> {
        frame["body"]["toc"]
            .as_array()
            .unwrap_or_else(|| panic!("rows: {frame}"))
            .iter()
            .map(|r| r["hpath"].as_str().expect("hpath").to_owned())
            .collect()
    };
    let anchors = |frame: &Value| -> Vec<String> {
        frame["body"]["anchors"]
            .as_array()
            .unwrap_or_else(|| panic!("anchors: {frame}"))
            .iter()
            .map(|a| a["anchor"].as_str().expect("anchor id").to_owned())
            .collect()
    };
    assert_eq!(
        headings(&frames[1]),
        vec!["Anchor-Zone"],
        "the scoped heading, and no anchor row beside it"
    );
    assert_eq!(
        anchors(&frames[1]),
        vec!["anc1", "anc2"],
        "the anchors byte-contained in the scoped subtree"
    );
    assert_eq!(
        headings(&frames[2]),
        vec!["Padded-Title"],
        "a sibling subtree carries none of them"
    );
    assert!(
        anchors(&frames[2]).is_empty(),
        "and its anchor plane is EMPTY, not absent — always-emitted"
    );
}

/// s1c: `anchors[]` rides a SECTIONS-mode read too, scoped by the same frag —
/// the field is a property of the response, never of the mode, so no caller
/// has to know which mode serves it.
#[test]
fn sections_mode_carries_the_anchor_plane_too() {
    let root_dir = testsuite::parity_dir().join("corpus").join("trailing-ws");
    let rel = "corpus/trailing-ws.md";
    let frames = serve(
        &root_dir,
        &[
            json!({"id": 1, "op": "read", "path": rel, "mode": "sections",
                   "sections": ["Anchor-Zone"]}),
            json!({"id": 2, "op": "read", "path": rel, "mode": "sections",
                   "frag": "Padded-Title"}),
        ],
    );
    let anchors = |frame: &Value| -> Vec<String> {
        assert_eq!(frame["ok"], json!(true), "ok frame: {frame}");
        assert!(
            frame["body"]["toc"].is_null(),
            "sections mode serves no heading table: {frame}"
        );
        frame["body"]["anchors"]
            .as_array()
            .unwrap_or_else(|| panic!("anchors: {frame}"))
            .iter()
            .map(|a| a["anchor"].as_str().expect("anchor id").to_owned())
            .collect()
    };
    assert_eq!(
        anchors(&frames[1]),
        vec!["anc1", "anc2"],
        "unscoped (no frag): the whole document's anchor plane"
    );
    assert!(
        anchors(&frames[2]).is_empty(),
        "a frag scopes the whole call, anchor plane included"
    );
}
