//! S1 gate: the composed read carries the authz facts — canonical hpaths,
//! byte spans, and the `^id` anchors — so ccc-statusd's put authz derives
//! governing sections from the engine's facts instead of its own markdown
//! mirror.
//!
//! Over the same parity corpus, one live serve session per corpus root,
//! comparing the host mirror's own input — the v2 `toc` op's raw hpath
//! segments, sanitized through `model::gotext::sanitize_heading` — against
//! the v3 composed-read row's canonical `hpath`. It then gates: every heading
//! row and anchor carries a `span` byte-equal to the v2 toc node's; the `^id`
//! anchors are surfaced with the Go switch's drops intact; the containment
//! join over the v3 facts equals the same walk over the v2 nodes; and
//! `rendered_text` never grows an anchor row.
//!
//! The anchors ride their own always-emitted `anchors[]`, never the `toc`
//! array: ccc-statusd's `readText` indents by `depth-1` and panics on a
//! depth-0 row.

use model::gotext::sanitize_heading;
use serde_json::{Value, json};

/// Drive one v3 serve session over a corpus root (the registry host's line
/// loop, `crate::daemon_door`); one frame per request.
fn serve(root_dir: &std::path::Path, requests: &[Value]) -> Vec<Value> {
    let mut input = crate::daemon_door::hello_line(Some("v3"), root_dir);
    for r in requests {
        input.push_str(&serde_json::to_string(r).expect("request serializes"));
        input.push('\n');
    }
    crate::daemon_door::serve_frames(&input)
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
            requests.push(json!({"id": requests.len() + 1, "op": "read", "path": rel}));
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
                // U14: the address is a segment array; the check is that no
                // segment's raw text opens with `^`.
                assert!(
                    r["hpath"]
                        .as_array()
                        .expect("row hpath is the segment array")
                        .iter()
                        .all(|seg| !seg["h"].as_str().unwrap_or_default().starts_with('^')),
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
                // The engine row publishes raw segments; sanitize them here so
                // the property stays: host mirror == engine address.
                let row_sanitized = r["hpath"]
                    .as_array()
                    .expect("row hpath is the segment array")
                    .iter()
                    .map(|seg| sanitize_heading(seg["h"].as_str().unwrap_or_default()))
                    .collect::<Vec<_>>()
                    .join("/");
                assert_eq!(
                    sanitized_hpath(n),
                    row_sanitized,
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

            // --- the anchor plane: every body host projects (F-R4); the one
            // exclusion is a frontmatter caret
            let v2_anchors: Vec<&Value> = nodes
                .iter()
                .filter(|n| n["kind"] != "frontmatter" && n["anchor"].is_string())
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
            // A frontmatter caret is NOT addressable on this face (literal
            // YAML, no block — F-R4's one exclusion): surfacing the anchor
            // plane must not smuggle one in, on EITHER array.
            for n in nodes
                .iter()
                .filter(|n| n["anchor"].is_string() && n["kind"] == "frontmatter")
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
    // The corpus carries no frontmatter caret (its captured files predate
    // the F-R4 widening, under which every BODY host projects), so the one
    // remaining exclusion is proven against a dedicated workspace instead.
    let fm_dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        fm_dir.path().join("fm.md"),
        "---\ntitle: x ^fm-caret\n---\n# T\n\n- item ^body-1\n",
    )
    .expect("seed fm fixture");
    let frames = serve(
        fm_dir.path(),
        &[json!({"id": 1, "op": "read", "path": "fm.md"})],
    );
    let fm_anchors: Vec<&str> = frames[1]["body"]["anchors"]
        .as_array()
        .expect("anchors ride the composed read")
        .iter()
        .map(|a| a["anchor"].as_str().expect("anchor id"))
        .collect();
    assert_eq!(
        fm_anchors,
        vec!["body-1"],
        "the body-hosted id projects; the frontmatter caret stays unaddressable"
    );
    dropped_anchors += 1;
    println!(
        "S1 authz facts: {files_checked} files, {headings_checked} heading rows, \
         {anchor_rows_checked} anchor rows, {dropped_anchors} anchors verified unaddressable"
    );
}

/// The extended row set carries the M1 fact set plus the authz span on every
/// row, and the address is the segment array, never a joined string. The
/// round-trip property lives in
/// `u14_read_face_contract::every_published_address_resolves_back_to_its_own_row`.
#[test]
fn extended_rows_carry_the_authz_facts_on_every_row() {
    let mut rows = 0_u32;
    for doc in ["basic", "duplicate-headings", "level-jumps", "trailing-ws"] {
        let root_dir = testsuite::parity_dir().join("corpus").join(doc);
        let rel = format!("corpus/{doc}.md");
        let frames = serve(&root_dir, &[json!({"id": 1, "op": "read", "path": rel})]);
        let frame = &frames[1];
        assert_eq!(frame["ok"], json!(true), "{doc}: ok frame: {frame}");
        for row in frame["body"]["toc"]
            .as_array()
            .unwrap_or_else(|| panic!("{doc}: rows: {frame}"))
        {
            // The M1 fact set, untouched by the authz additions.
            for key in ["n", "depth", "title", "hpath", "words", "sec_rev"] {
                assert!(!row[key].is_null(), "{doc}: `{key}` rides row {row}");
            }
            // U14: the address is the SEGMENT array, never a joined string.
            assert!(
                row["hpath"].is_array(),
                "{doc}: `hpath` is the segment array (decision 14): {row}"
            );
            // S1's own addition: the containment fact an anchor is tested
            // against. This is the assert whose absence the retired gate would
            // have caught, so it is restated here rather than dropped.
            assert!(
                row["span"].is_array(),
                "{doc}: the authz span rides row {row}"
            );
            rows += 1;
        }
    }
    assert!(rows >= 15, "rows compared: {rows}");
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
            json!({"id": 1, "op": "read", "path": rel, "frag": [{"h": "Anchor Zone"}]}),
            json!({"id": 2, "op": "read", "path": rel, "frag": [{"h": "Padded Title"}]}),
        ],
    );
    let headings = |frame: &Value| -> Vec<String> {
        frame["body"]["toc"]
            .as_array()
            .unwrap_or_else(|| panic!("rows: {frame}"))
            .iter()
            // The joined spelling is a TEST convenience for readable
            // assertions; nothing in the engine joins one (U14).
            .map(|r| {
                r["hpath"]
                    .as_array()
                    .expect("hpath is the segment array")
                    .iter()
                    .map(|seg| seg["h"].as_str().unwrap_or_default().to_owned())
                    .collect::<Vec<_>>()
                    .join("/")
            })
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
        vec!["Anchor Zone"],
        "the scoped heading (RAW text — U14), and no anchor row beside it"
    );
    assert_eq!(
        anchors(&frames[1]),
        vec!["anc1", "anc2", "anc3"],
        "the anchors byte-contained in the scoped subtree (paragraph-hosted \
         ^anc3 included — F-R4)"
    );
    assert_eq!(
        headings(&frames[2]),
        vec!["Padded Title"],
        "a sibling subtree carries none of them"
    );
    assert!(
        anchors(&frames[2]).is_empty(),
        "and its anchor plane is EMPTY, not absent — always-emitted"
    );
}

/// `anchors[]` rides a section read too — the field is a property of the
/// response, never of the face. `frag` and `sections[]` cannot ride one call,
/// so a frag-scoped section read is unreachable; the frag-scoping rule is
/// gated on the toc face above.
#[test]
fn section_read_carries_the_anchor_plane_too() {
    let root_dir = testsuite::parity_dir().join("corpus").join("trailing-ws");
    let rel = "corpus/trailing-ws.md";
    let frames = serve(
        &root_dir,
        &[json!({"id": 1, "op": "read", "path": rel,
                   "sections": [{"hpath": [{"h": "Anchor Zone"}]}]})],
    );
    let anchors = |frame: &Value| -> Vec<String> {
        assert_eq!(frame["ok"], json!(true), "ok frame: {frame}");
        assert!(
            frame["body"]["toc"].is_null(),
            "a section read serves no heading table: {frame}"
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
        vec!["anc1", "anc2", "anc3"],
        "unscoped (no frag): the whole document's anchor plane, every body \
         host included (F-R4)"
    );
}
