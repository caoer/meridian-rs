//! **Block scalars on both published faces** — card
//! `mrd-frontmatter-block-scalar-decoder-gap`.
//!
//! A page carrying `description: >` and six indented lines published `">"` on
//! `read`'s `props[]` and `">"` in `sql`'s `frontmatter`, while `PyYAML` read
//! the whole 459-character text. 45 key ROWS on 37 pages were mis-served. The pages are
//! valid YAML: this was a decoder gap on our side, never corpus damage.
//!
//! The engine's decoder is hand-rolled (the `model` crate is serde-free by the
//! law `yaml_confinement.rs` enforces), so the only honest gate is a FOREIGN
//! parser: every shape below is decoded by the engine AND by `PyYAML`, and the
//! two must agree byte for byte. That is what makes a hand-rolled YAML reader
//! trustworthy — not the unit tests beside it in `model`.
//!
//! ZT-ruled (a) 2026-08-23, relayed by leader `73c3fab5`: `props[].value`
//! carries the real newlines. This is the FIRST widening of that plane, and it
//! is a widening for BOTH indicators — clip chomping (the default, and what
//! every one of those pages uses) leaves a trailing `\n` on a FOLDED scalar
//! too, so there is no "folded is the safe case" split.
//!
//! **These assertions drive the WIRE FACE, not the model.** The first version
//! of this file called `model`'s flat map directly and passed while the
//! published `read` face still trimmed every value at
//! `wire-serve/src/read.rs:474` — one layer BELOW the assertion. Review seat
//! `79c5905c` caught it (B1, PR 189 @ `563bff59b`): `sql` served the card
//! page's 459 characters, `read` served 458. A gate one layer off the face it
//! claims to cover is not a gate, so every row below now goes through
//! `serve()` (the `props_plane.rs` shape) for `read` and through `fm_value`
//! for `sql`, with `PyYAML` as the third opinion.

use serde_json::{Value, json};

/// The two PUBLISHED faces for one key, each driven at its own door:
/// `read`'s `props[]` over the v3 serve (what an agent actually receives) and
/// `sql`'s `frontmatter` value through `model::fm_value` (what the projector
/// stores). Never the model flat map — that is the layer the first gate
/// mistook for the face.
fn wire_faces(dir: &std::path::Path, rel: &str, key: &str) -> (String, String) {
    let frames = serve(dir, &[json!({"id":1,"op":"read","path":rel})]);
    assert_eq!(frames[1]["ok"], json!(true), "read serves: {}", frames[1]);
    let props = frames[1]["body"]["props"]
        .as_array()
        .expect("props plane")
        .clone();
    let read_face = props
        .iter()
        .find(|p| p["key"] == json!(key))
        .unwrap_or_else(|| panic!("key {key:?} absent from props[]"))["value"]
        .as_str()
        .expect("value is a string")
        .to_owned();

    let raw = std::fs::read_to_string(dir.join(rel)).expect("fixture readable");
    let doc = model::build(raw.clone(), syntax::parse(&raw));
    let fm = doc
        .root
        .children
        .iter()
        .find(|n| matches!(n.kind, model::NodeKind::Frontmatter { .. }))
        .expect("the fixture carries frontmatter");
    let sql_face = model::fm_value(&raw[fm.span.clone()], key)
        .unwrap_or_else(|| panic!("key {key:?} absent from fm_value"));
    (read_face, sql_face)
}

/// Drive one v3 serve session over `doc_dir` — the `props_plane.rs` harness.
fn serve(doc_dir: &std::path::Path, requests: &[Value]) -> Vec<Value> {
    let mut input = crate::daemon_door::hello_line(Some("v3"), doc_dir);
    for r in requests {
        input.push_str(&serde_json::to_string(r).expect("request serializes"));
        input.push('\n');
    }
    crate::daemon_door::serve_frames(&input)
}

/// Write one fixture page into a temp workspace and answer (dir, rel).
fn fixture(frontmatter: &str) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    let rel = "probe.md".to_owned();
    std::fs::write(dir.path().join(&rel), page(frontmatter)).expect("write fixture");
    (dir, rel)
}

/// One key's value as `PyYAML` reads it — the foreign oracle.
fn pyyaml(raw: &str, key: &str) -> String {
    // The split consumes the newline that ENDS the last frontmatter line, and
    // for a block scalar that byte is the value's own trailing break — drop it
    // and clip chomping has nothing to clip, so the oracle would disagree with
    // every correct decoder. Hand python the block as the file carries it.
    let fm = raw
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---").map(|(fm, _)| format!("{fm}\n")))
        .expect("the fixture carries frontmatter");
    let out = std::process::Command::new("python3")
        .args(["-c", PY_ONE_KEY, &fm, key])
        .output()
        .expect("python3 runs");
    assert!(
        out.status.success(),
        "PyYAML rejects the fixture: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("utf-8")
}

const PY_ONE_KEY: &str = r"
import sys, yaml
d = yaml.safe_load(sys.argv[1])
sys.stdout.write(d[sys.argv[2]])
";

fn page(frontmatter: &str) -> String {
    format!("---\n{frontmatter}---\n\n# Body\n")
}

/// THE MATRIX. Every block-scalar shape, decoded by the engine's two faces and
/// by `PyYAML`, all three byte-equal.
#[test]
fn every_block_scalar_shape_agrees_with_pyyaml_on_both_faces() {
    for (name, fm) in [
        ("folded clip", "k: >\n  line one\n  line two\n"),
        ("folded strip", "k: >-\n  line one\n  line two\n"),
        ("folded keep", "k: >+\n  line one\n\n"),
        ("literal clip", "k: |\n  first\n  second\n  third\n"),
        ("literal strip", "k: |-\n  first\n  second\n"),
        ("folded blank line", "k: >\n  para one\n\n  para two\n"),
        (
            "folded more-indented",
            "k: >\n  normal\n    kept as is\n  normal\n",
        ),
        ("literal blank line", "k: |\n  a\n\n  b\n"),
        ("explicit indent", "k: |2\n    a\n    b\n"),
        ("header comment", "k: > # why\n  text\n"),
        ("one line folded", "k: >\n  alone\n"),
        ("value carrying a colon", "k: |\n  owner: not a key\n"),
        ("value carrying a dash", "k: |\n  - not a list\n"),
        ("next key after the block", "k: |\n  body\nother: plain\n"),
        // The four shapes review seat 79c5905c hand-crafted to break the first
        // implementation (B2). Both faces agreed with each other and disagreed
        // with YAML; live reachability is zero, which is exactly why only an
        // oracle could find them.
        ("literal, one leading blank", "k: |\n\n  a\n"),
        ("literal, two leading blanks", "k: |\n\n\n  a\n"),
        ("folded, one leading blank", "k: >\n\n  a\n"),
        (
            "folded, blank AFTER a more-indented line",
            "k: >\n  a\n    b\n\n  c\n",
        ),
        (
            "literal, whitespace-only line indented PAST the block",
            "k: |\n  a\n   \n  b\n",
        ),
        (
            "folded, whitespace-only line indented PAST the block",
            "k: >\n  a\n   \n  b\n",
        ),
        ("keep, body is only empty lines", "k: |+\n\n\n"),
        ("keep, body is one empty line", "k: |+\n\n"),
        ("clip, body is only empty lines", "k: |\n\n\n"),
        ("keep, content then trailing blanks", "k: |+\n  a\n\n\n"),
    ] {
        let want = pyyaml(&page(fm), "k");
        let (dir, rel) = fixture(fm);
        let (read_face, sql_face) = wire_faces(dir.path(), &rel, "k");
        assert_eq!(sql_face, want, "{name}: sql frontmatter vs PyYAML");
        assert_eq!(
            read_face, want,
            "{name}: read props[].value vs PyYAML — THE face the card is about"
        );
    }
}

/// The leader's required test, part 1: a `|` scalar of three lines carries TWO
/// interior newlines on the JSON face, and `sql`'s text is identical.
#[test]
fn a_literal_three_line_scalar_carries_two_interior_newlines_on_both_faces() {
    let fm = "notes: |\n  first\n  second\n  third\n";
    let (dir, rel) = fixture(fm);
    let (face_a, face_b) = wire_faces(dir.path(), &rel, "notes");
    assert_eq!(face_a, "first\nsecond\nthird\n");
    assert_eq!(face_b, face_a, "sql text identical to the props value");
    assert_eq!(
        face_a.matches('\n').count(),
        3,
        "two interior breaks plus the clip newline"
    );
    assert_eq!(
        face_a[..face_a.len() - 1].matches('\n').count(),
        2,
        "exactly two U+000A INSIDE the value, as ruled"
    );
    assert_eq!(face_a, pyyaml(&page(fm), "notes"));
}

/// The leader's required test, part 2: the `>` clip case — a folded scalar is
/// NOT single-line either; clip leaves a trailing newline, which is why the
/// ruling had to cover both indicators.
#[test]
fn a_folded_clip_scalar_still_ends_in_a_newline() {
    let fm = "description: >\n  line one\n  line two\n";
    let (dir, rel) = fixture(fm);
    let (face_a, face_b) = wire_faces(dir.path(), &rel, "description");
    assert_eq!(face_a, "line one line two\n");
    assert_eq!(face_b, face_a);
    assert!(
        face_a.ends_with('\n'),
        "clip chomping keeps one trailing newline — the reason (a) is (a) for both indicators"
    );
    assert_eq!(face_a, pyyaml(&page(fm), "description"));
}

/// The regression fixture the advisor named: the live page's own bytes, copied
/// (the live page is read-only). 459 characters, ending in a newline.
#[test]
fn the_live_regression_fixture_reads_back_whole() {
    let fm = concat!(
        "name: ground-truth-verifier\n",
        "description: >\n",
        "  Verifies a wiki page or compound source against the REAL system it describes\n",
        "  — the actual repo file, config, command output, or device — not against the\n",
        "  session transcript that wrote it. Use before declaring a compound/wiki batch\n",
        "  \"complete\", when a source claims facts about a repo/config/device, or whenever\n",
        "  a fidelity check needs an external anchor instead of self-report. Returns a\n",
        "  per-claim ledger: VERIFIED-AGAINST-REALITY / CONTRADICTED / UNVERIFIABLE.\n",
        "model: opus\n",
    );
    let (dir, rel) = fixture(fm);
    let (face_a, face_b) = wire_faces(dir.path(), &rel, "description");
    let want = pyyaml(&page(fm), "description");
    assert_eq!(face_b, want, "sql face");
    assert_eq!(face_a, want, "read props face");
    // 459 CHARACTERS, 463 bytes — the two em-dashes cost 2 bytes each.
    // PyYAML counts characters; `str::len` counts bytes. Both are asserted
    // so the fixture cannot drift from the live page in either unit.
    assert_eq!(
        face_a.chars().count(),
        459,
        "the live page's own length in chars"
    );
    assert_eq!(face_a.len(), 463, "the same value in bytes");
    assert!(face_a.starts_with("Verifies a wiki page"));
    assert!(face_a.ends_with("UNVERIFIABLE.\n"));
    assert_ne!(face_a, ">", "the indicator byte is gone from both faces");
    // The keys AROUND a block scalar keep reading exactly as before.
    assert_eq!(wire_faces(dir.path(), &rel, "model").0, "opus");
    assert_eq!(wire_faces(dir.path(), &rel, "name").0, "ground-truth-verifier");
}

/// The SUSPENDED half, pinned so nobody widens it by accident: a block-style
/// LIST still reads empty on the props face and as flow text in sql. That
/// asymmetry is the contract (`docs/wire-contract.md:2084-2111`), not a bug —
/// changing it needs ZT's amendment (advisor ruling 2026-08-23).
#[test]
fn a_block_list_is_untouched_by_this_card() {
    let (dir, rel) = fixture("skills:\n  - obsidian-md\n  - other\n");
    let (face_a, face_b) = wire_faces(dir.path(), &rel, "skills");
    assert_eq!(face_a, "", "read props: empty, by contract");
    assert_eq!(face_b, "[obsidian-md, other]", "sql: reads it");
}

/// **The SCRIPT plane rides the same repair** — the second consumer of the
/// same function, named because a fix that covered only the JSON
/// serialization would have missed it.
///
/// `crates/registry/src/script_op.rs:1196` builds the script face's `fm` map
/// from `wire_serve::read::props_of(doc)`, which is `read_props` — where the
/// `fm_publish` branch lives. There is no layer between `props_of` and either
/// consumer that could re-decode: the composed read serializes its rows, and
/// `script_op` maps them into `(key, value)` pairs. So asserting here asserts
/// for both faces at once.
#[test]
fn the_shared_props_entry_serves_block_scalars_untrimmed() {
    let raw = page("folded: >\n  a\n  b\nliteral: |\n  x\n  y\nplain: ordinary\n");
    let doc = model::build(raw.clone(), syntax::parse(&raw));
    let rows = wire_serve::read::props_of(&doc);
    let of = |k: &str| {
        rows.iter()
            .find(|p| p.key == k)
            .unwrap_or_else(|| panic!("key {k:?} absent"))
            .value
            .clone()
    };
    assert_eq!(of("folded"), "a b\n", "the trailing newline survives props_of");
    assert_eq!(of("literal"), "x\ny\n", "interior breaks survive props_of");
    assert_eq!(of("plain"), "ordinary", "an ordinary scalar still decodes");
}
