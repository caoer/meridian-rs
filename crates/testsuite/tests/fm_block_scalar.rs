//! **Block scalars on both published faces** — card
//! `mrd-frontmatter-block-scalar-decoder-gap`.
//!
//! A page carrying `description: >` and six indented lines published `">"` on
//! `read`'s `props[]` and `">"` in `sql`'s `frontmatter`, while `PyYAML` read
//! the whole 459-character text. 45 live pages were mis-served. The pages are
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
//! is a widening for BOTH indicators — clip chomping (the default, and what all
//! 45 pages use) leaves a trailing `\n` on a FOLDED scalar too, so there is no
//! "folded is the safe case" split.

/// The engine's answer for one key, off the `model` faces both published
/// surfaces call: Face B (`fm_value`, behind `sql`'s `frontmatter`) and Face A
/// (the flat `YamlMap` behind `read`'s `props[]`).
fn engine_faces(raw: &str, key: &str) -> (String, String) {
    let doc = model::build(raw.to_string(), syntax::parse(raw));
    let fm = doc
        .root
        .children
        .iter()
        .find(|n| matches!(n.kind, model::NodeKind::Frontmatter { .. }))
        .expect("the fixture carries frontmatter");
    let model::NodeKind::Frontmatter { map } = &fm.kind else {
        unreachable!("filtered above")
    };
    let face_a = map.0.iter().find(|(k, _)| k == key).map_or_else(
        || panic!("key {key:?} absent from the flat map"),
        |(_, v)| v.clone(),
    );
    let face_b = model::fm_value(&raw[fm.span.clone()], key)
        .unwrap_or_else(|| panic!("key {key:?} absent from fm_value"));
    (face_a, face_b)
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
    ] {
        let raw = page(fm);
        let want = pyyaml(&raw, "k");
        let (face_a, face_b) = engine_faces(&raw, "k");
        assert_eq!(face_b, want, "{name}: sql face vs PyYAML");
        assert_eq!(face_a, want, "{name}: read props face vs PyYAML");
    }
}

/// The leader's required test, part 1: a `|` scalar of three lines carries TWO
/// interior newlines on the JSON face, and `sql`'s text is identical.
#[test]
fn a_literal_three_line_scalar_carries_two_interior_newlines_on_both_faces() {
    let raw = page("notes: |\n  first\n  second\n  third\n");
    let (face_a, face_b) = engine_faces(&raw, "notes");
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
    assert_eq!(face_a, pyyaml(&raw, "notes"));
}

/// The leader's required test, part 2: the `>` clip case — a folded scalar is
/// NOT single-line either; clip leaves a trailing newline, which is why the
/// ruling had to cover both indicators.
#[test]
fn a_folded_clip_scalar_still_ends_in_a_newline() {
    let raw = page("description: >\n  line one\n  line two\n");
    let (face_a, face_b) = engine_faces(&raw, "description");
    assert_eq!(face_a, "line one line two\n");
    assert_eq!(face_b, face_a);
    assert!(
        face_a.ends_with('\n'),
        "clip chomping keeps one trailing newline — the reason (a) is (a) for both indicators"
    );
    assert_eq!(face_a, pyyaml(&raw, "description"));
}

/// The regression fixture the advisor named: the live page's own bytes, copied
/// (the live page is read-only). 459 characters, ending in a newline.
#[test]
fn the_live_regression_fixture_reads_back_whole() {
    let raw = page(concat!(
        "name: ground-truth-verifier\n",
        "description: >\n",
        "  Verifies a wiki page or compound source against the REAL system it describes\n",
        "  — the actual repo file, config, command output, or device — not against the\n",
        "  session transcript that wrote it. Use before declaring a compound/wiki batch\n",
        "  \"complete\", when a source claims facts about a repo/config/device, or whenever\n",
        "  a fidelity check needs an external anchor instead of self-report. Returns a\n",
        "  per-claim ledger: VERIFIED-AGAINST-REALITY / CONTRADICTED / UNVERIFIABLE.\n",
        "model: opus\n",
    ));
    let (face_a, face_b) = engine_faces(&raw, "description");
    let want = pyyaml(&raw, "description");
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
    assert_eq!(engine_faces(&raw, "model").0, "opus");
    assert_eq!(engine_faces(&raw, "name").0, "ground-truth-verifier");
}

/// The SUSPENDED half, pinned so nobody widens it by accident: a block-style
/// LIST still reads empty on the props face and as flow text in sql. That
/// asymmetry is the contract (`docs/wire-contract.md:2084-2111`), not a bug —
/// changing it needs ZT's amendment (advisor ruling 2026-08-23).
#[test]
fn a_block_list_is_untouched_by_this_card() {
    let raw = page("skills:\n  - obsidian-md\n  - other\n");
    let (face_a, face_b) = engine_faces(&raw, "skills");
    assert_eq!(face_a, "", "read props: empty, by contract");
    assert_eq!(face_b, "[obsidian-md, other]", "sql: reads it");
}
