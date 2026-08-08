//! The frontmatter-properties plane on the composed `read` (wire-contract
//! § A.3): key facts served at the same snapshot as the rest of the body.
//!
//! Five laws, each pinned here: one row per top-level key in document order
//! with the value as a canonical scalar (a stored quoted scalar serves
//! decoded — quotes and escapes resolved; any other stored form verbatim);
//! `prop_rev`/`span` agree with the `cat` `fm_key` grain (one rev per node —
//! no second derivation); always emitted, empty without frontmatter;
//! document-grain — neither `frag` nor sections mode scopes it away; and the
//! block-value grain.

use serde_json::{Value, json};

/// Drive one v3 serve session over `doc_dir`; one frame per line.
fn serve(doc_dir: &std::path::Path, requests: &[Value]) -> Vec<Value> {
    let mut input = crate::daemon_door::hello_line(Some("v3"), doc_dir);
    for r in requests {
        input.push_str(&serde_json::to_string(r).expect("request serializes"));
        input.push('\n');
    }
    crate::daemon_door::serve_frames(&input)
}

/// Keys, values, and the cross-op rev agreement: each `props` row carries the
/// document-order key, the key line's stored value, and exactly the span +
/// CAS token the `cat` `fm_key` door serves for the same key.
#[test]
fn props_serve_key_facts_that_agree_with_the_cat_fm_key_grain() {
    let dir = testsuite::parity_dir().join("corpus").join("basic");
    let frames = serve(
        &dir,
        &[
            json!({"id":1,"op":"read","path":"corpus/basic.md"}),
            json!({"id":2,"op":"cat","path":"corpus/basic.md","sec":{"fm_key":"type"}}),
            json!({"id":3,"op":"cat","path":"corpus/basic.md","sec":{"fm_key":"status"}}),
        ],
    );
    let props = frames[1]["body"]["props"].as_array().expect("props plane");
    let kv: Vec<(&str, &str)> = props
        .iter()
        .map(|p| {
            (
                p["key"].as_str().expect("key"),
                p["value"].as_str().expect("value"),
            )
        })
        .collect();
    assert_eq!(
        kv,
        vec![("type", "note"), ("status", "seeded")],
        "one row per top-level key, document order, value as stored"
    );
    for (row, cat) in props.iter().zip(&frames[2..=3]) {
        assert_eq!(cat["ok"], json!(true), "cat fm_key serves: {cat}");
        assert_eq!(
            row["prop_rev"], cat["body"]["node_rev"],
            "one rev per node: the props row and the cat fm_key grain must \
             mint the same token ({})",
            row["key"]
        );
        assert_eq!(
            row["span"], cat["body"]["span"],
            "the props row publishes the cat fm_key grain span ({})",
            row["key"]
        );
    }
}

/// Emission law (the `anchors` precedent): the plane is always emitted —
/// a document with no frontmatter serves `props: []`, never an absent key.
#[test]
fn props_are_always_emitted_empty_without_frontmatter() {
    let dir = testsuite::parity_dir().join("corpus").join("no-frontmatter");
    let frames = serve(
        &dir,
        &[json!({"id":1,"op":"read","path":"corpus/no-frontmatter.md"})],
    );
    let body = frames[1]["body"].as_object().expect("read body");
    assert_eq!(
        body.get("props"),
        Some(&json!([])),
        "props must serialize unconditionally: {}",
        frames[1]
    );
}

/// Document-grain: a `frag`-scoped toc read and a sections-mode read both
/// serve the full plane — frontmatter belongs to the document, not to any
/// subtree, so no scope removes it.
#[test]
fn props_are_document_grain_in_both_modes() {
    let dir = testsuite::parity_dir().join("corpus").join("basic");
    let frames = serve(
        &dir,
        &[
            json!({"id":1,"op":"read","path":"corpus/basic.md","frag":[{"h":"Notes"}]}),
            json!({"id":2,"op":"read","path":"corpus/basic.md",
                   "sections":[{"hpath":[{"h":"Todo"}]}]}),
        ],
    );
    for frame in &frames[1..=2] {
        assert_eq!(frame["ok"], json!(true), "read serves: {frame}");
        let keys: Vec<&str> = frame["body"]["props"]
            .as_array()
            .expect("props plane")
            .iter()
            .map(|p| p["key"].as_str().expect("key"))
            .collect();
        assert_eq!(
            keys,
            vec!["type", "status"],
            "document-grain: no mode and no frag scopes the plane ({frame})"
        );
    }
}

/// The value law (wire-contract § A.3, amended 2026-08-07): a stored QUOTED
/// scalar serves decoded — quote bytes are storage syntax, not value — and
/// every other stored form serves verbatim. The fixture rows are the dogfood
/// season-1 finding-1 table translated 1:1 (the fleet-canonical shapes), plus
/// the escape grammar and the no-coercion guards. `prop_rev`/`span` still
/// agree with the `cat` `fm_key` grain for a quoted row: the decoding never
/// moves a rev.
#[test]
fn props_serve_canonical_scalars_quoted_values_decode() {
    let home = tempfile::tempdir().expect("workspace");
    let dir = home.path().join("corpus");
    std::fs::create_dir_all(&dir).expect("mkdir");
    // The season-1 table: `owner: ""` read back as length 2, `owner:
    // "3f9a1c07"` as 10, `owner: "[[1ed98864]]"` as 14 — every quoted row
    // wrong, every unquoted row right.
    let raw = "---\n\
               owner: \"\"\n\
               claimer: \"3f9a1c07\"\n\
               author: \"[[1ed98864]]\"\n\
               single: 'zt'\n\
               doubled: 'it''s'\n\
               escaped: \"a \\\"b\\\" \\\\ c\"\n\
               status: doing\n\
               count: 007\n\
               tags: [a, b]\n\
               ---\n\n# Body\n";
    std::fs::write(dir.join("quoted.md"), raw).expect("fixture");
    let frames = serve(
        home.path(),
        &[
            json!({"id":1,"op":"read","path":"corpus/quoted.md"}),
            json!({"id":2,"op":"cat","path":"corpus/quoted.md","sec":{"fm_key":"author"}}),
        ],
    );
    let props = frames[1]["body"]["props"].as_array().expect("props plane");
    let kv: Vec<(&str, &str)> = props
        .iter()
        .map(|p| {
            (
                p["key"].as_str().expect("key"),
                p["value"].as_str().expect("value"),
            )
        })
        .collect();
    assert_eq!(
        kv,
        vec![
            ("owner", ""),               // `""` — quoted-empty serves empty
            ("claimer", "3f9a1c07"),     // double quotes dropped
            ("author", "[[1ed98864]]"),  // the fleet-canonical wikilink value
            ("single", "zt"),            // single quotes dropped
            ("doubled", "it's"),         // `''` unescapes to `'`
            ("escaped", "a \"b\" \\ c"), // `\"` and `\\` unescape
            ("status", "doing"),         // unquoted scalar: verbatim
            ("count", "007"),            // no type coercion at the read seam
            ("tags", "[a, b]"),          // a flow collection: verbatim
        ],
        "quoted scalars serve decoded; everything else verbatim"
    );
    let author = &props[2];
    assert_eq!(
        author["prop_rev"], frames[2]["body"]["node_rev"],
        "the rev is minted over the STORED grain bytes — decoding never moves it"
    );
    assert_eq!(author["span"], frames[2]["body"]["span"]);
}

/// A block value (indented continuation lines) serves the key line's own
/// remainder — empty here — while `span` covers the whole grain and
/// `prop_rev` hashes it, so a guarded upsert can never orphan the tail.
#[test]
fn a_block_value_serves_the_key_line_remainder_and_the_full_grain() {
    let home = tempfile::tempdir().expect("workspace");
    let dir = home.path().join("corpus");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let raw = "---\nowner:\n  - alice\n  - bob\ntitle: Plan\n---\n\n# Body\n";
    std::fs::write(dir.join("block.md"), raw).expect("fixture");
    let frames = serve(
        home.path(),
        &[
            json!({"id":1,"op":"read","path":"corpus/block.md"}),
            json!({"id":2,"op":"cat","path":"corpus/block.md","sec":{"fm_key":"owner"}}),
        ],
    );
    let props = frames[1]["body"]["props"].as_array().expect("props plane");
    assert_eq!(props[0]["key"], json!("owner"));
    assert_eq!(
        props[0]["value"],
        json!(""),
        "a block value's key line carries no remainder; nothing is re-serialized"
    );
    assert_eq!(props[1]["key"], json!("title"));
    assert_eq!(props[1]["value"], json!("Plan"));
    assert_eq!(
        props[0]["span"], frames[2]["body"]["span"],
        "the grain covers the key line plus its continuation lines"
    );
    assert_eq!(
        props[0]["prop_rev"], frames[2]["body"]["node_rev"],
        "the CAS token is minted over the full grain bytes"
    );
}
