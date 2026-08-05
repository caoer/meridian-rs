//! Rung-1 parse-truth gate: `syntax::parse` reproduces the frozen GT pack's node
//! boundaries and classifications, file by file.
//!
//! Syntax owns exactly `(kind, span)` — the dialect facts. The pack's `hpath`
//! (section-tree assembly) is `model`'s and `text_prefix_16b` is `wire-map`'s, so this
//! gate compares the syntax-owned projection: every GT node's kind and byte span, in the
//! §1 total order both sides emit. The pack is demoted to a bench fixture only later
//! (PF-GT-RETARGET); until then this diff is reviewable.

use serde_json::Value;
use std::fmt::Write as _;
use std::fs;
use syntax::{DialectKind, DialectNode};

/// GT kind string for a parsed node — the lane vocabulary the pack is written in.
fn gt_kind(kind: &DialectKind) -> &'static str {
    match kind {
        DialectKind::Frontmatter { .. } => "frontmatter",
        DialectKind::Heading { .. } => "heading",
        DialectKind::Fence { .. } => "fence",
        DialectKind::InlineCode => "inline-code",
        DialectKind::Anchor { .. } => "anchor",
        DialectKind::Wikilink { .. } => "wikilink",
        DialectKind::Embed { .. } => "embed",
        DialectKind::Callout { .. } => "callout",
        DialectKind::Task { .. } => "task",
        DialectKind::Table => "table",
        DialectKind::Comment => "comment",
    }
}

fn parsed_pairs(nodes: &[DialectNode]) -> Vec<(String, usize, usize)> {
    nodes
        .iter()
        .map(|n| (gt_kind(&n.kind).to_string(), n.span.start, n.span.end))
        .collect()
}

fn expected_pairs(nodes: &[Value]) -> Vec<(String, usize, usize)> {
    nodes
        .iter()
        .map(|n| {
            let kind = n["kind"]
                .as_str()
                .expect("node kind is a string")
                .to_string();
            let span = n["span"].as_array().expect("node span is an array");
            let start = usize::try_from(span[0].as_u64().expect("span start")).unwrap();
            let end = usize::try_from(span[1].as_u64().expect("span end")).unwrap();
            (kind, start, end)
        })
        .collect()
}

#[test]
fn gt_pack_parse_truth() {
    let gt = testsuite::gt_pack_dir();
    let ground_truth = gt.join("ground-truth");

    let mut checked = 0;
    let mut failures: Vec<String> = Vec::new();

    for entry in fs::read_dir(&ground_truth).expect("ground-truth dir exists") {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        checked += 1;

        let doc: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let source_rel = doc["file"].as_str().expect("`file` field");
        let source = fs::read_to_string(gt.join(source_rel))
            .unwrap_or_else(|e| panic!("read GT source {source_rel}: {e}"));

        let want = expected_pairs(doc["nodes"].as_array().expect("`nodes` array"));
        let got = parsed_pairs(&syntax::parse(&source));

        if want != got {
            // find the first divergence for a precise, reviewable message
            let mut detail = format!(
                "\n{source_rel}: {} expected nodes, {} parsed",
                want.len(),
                got.len()
            );
            let n = want.len().max(got.len());
            for i in 0..n {
                let w = want.get(i);
                let g = got.get(i);
                if w != g {
                    let _ = write!(detail, "\n  [{i}] expected {w:?}\n      got      {g:?}");
                }
            }
            failures.push(detail);
        }
    }

    assert_eq!(checked, 10, "the frozen pack carries 10 GT files");
    assert!(
        failures.is_empty(),
        "syntax::parse diverges from the frozen GT pack (kind,span):{}",
        failures.join("\n")
    );
}
