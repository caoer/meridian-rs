//! The TOON goldens that re-pin the rendered CLI face.
//!
//! These are not captures of another implementation but reviewed fixtures of
//! our own face, hand-checked once and frozen. Nothing outside this repo is
//! owed byte parity.
//!
//! No regeneration path exists here, deliberately: no `--bless`, no
//! `UPDATE_GOLDENS` escape, no write-back on mismatch. A golden the code under
//! test can rewrite is not a pin. Changing the face means editing the
//! committed bytes by hand and defending the diff in review.
//!
//! Each case drives the sidecar serve loop — the real v3 wire door over a real
//! on-disk workspace — and pins `body.rendered_text`, so what is frozen is
//! what a caller actually receives.

use serde_json::{Value, json};

/// Where the reviewed fixtures live.
fn goldens_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data/toon-goldens")
}

/// The one comparison primitive every case in this file shares: exact byte
/// equality against a committed file, failing loudly when the file is absent
/// rather than creating it.
fn assert_golden(case: &str, actual: &str) {
    let path = goldens_dir().join(format!("{case}.toon"));
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "golden `{case}` is missing ({e}) at {}.\n\
             It is NOT created for you: read the bytes below, check them by \
             eye, and commit the file as a reviewed fixture.\n\
             ----- actual -----\n{actual}\n----- end -----",
            path.display()
        )
    });
    assert_eq!(
        actual, expected,
        "\nthe rendered face moved off its golden `{case}`.\n\
         ----- actual -----\n{actual}\n----- expected -----\n{expected}\n----- end -----"
    );
}

/// One fixture workspace on disk, plus one v3 serve session over it.
///
/// The workspace is a tempdir, but nothing path-shaped reaches the golden: the
/// face's `path` field is the REQUEST's relative path, so the fixtures stay
/// byte-stable across machines and runs.
fn render_face(doc_name: &str, body: &str, request: &Value) -> String {
    let dir = tempfile::tempdir().expect("tempdir");
    let docs = dir.path().join("corpus");
    std::fs::create_dir_all(&docs).expect("corpus dir");
    std::fs::write(docs.join(doc_name), body).expect("write fixture");

    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    let mut input = String::from("{\"id\":0,\"op\":\"hello\",\"proto\":1,\"contract\":\"v3\"}\n");
    input.push_str(&serde_json::to_string(request).expect("request serializes"));
    input.push('\n');

    let mut out = Vec::new();
    sidecar::serve(&root, input.as_bytes(), &mut out, &[]).expect("serve");
    let frames: Vec<Value> = String::from_utf8(out)
        .expect("frames are UTF-8")
        .lines()
        .map(|l| serde_json::from_str(l).expect("frame parses"))
        .collect();
    let read = &frames[1];
    assert_eq!(read["ok"], json!(true), "the read was served: {read}");
    read["body"]["rendered_text"]
        .as_str()
        .expect("rendered_text is a string")
        .to_string()
}

/// A representative document: nested headings, a slash in a title (the
/// sanitizer's case), and non-ASCII text that must ride the face literally.
const NESTED: &str = "---\ntype: note\n---\n\n# Todo\n\n- [ ] first item\n\n# Notes\n\nseed note line one\n\n## Slash/Title Here\n\ndeep content\n\n## 世界\n\nunicode body\n";

/// The toc face over a representative document: header scalars, then the
/// section map as one tabular array. The dewey ordinals arrive QUOTED — `2.1`
/// bare would decode as a float, and the ordinal is a string.
#[test]
fn toc_face_over_a_nested_document() {
    let text = render_face(
        "nested.md",
        NESTED,
        &json!({"id": 1, "op": "read", "path": "corpus/nested.md"}),
    );
    assert_golden("toc-nested", &text);
}

/// A document with no headings: `toc: []`, never the retired `toc[0]:` header
/// and never a missing field. An empty result is a VALUE here, which is what
/// lets a consumer tell "no sections" from "no toc was rendered".
#[test]
fn toc_face_over_a_document_with_no_headings() {
    let text = render_face(
        "flat.md",
        "just prose, no headings at all\n",
        &json!({"id": 1, "op": "read", "path": "corpus/flat.md"}),
    );
    assert_golden("toc-empty", &text);
}

/// The sections face: a TOON head declaring each body's `n`, raw `title`,
/// `rev`, `words` and `bytes`, then the bodies verbatim behind their markers.
/// The marker is the row's `n`, not its full heading path.
#[test]
fn sections_face_over_one_section() {
    let text = render_face(
        "nested.md",
        NESTED,
        &json!({
            "id": 1, "op": "read", "path": "corpus/nested.md",
            "sections": [{"hpath": [{"h": "Notes"}]}]
        }),
    );
    assert_golden("sections-one", &text);
}

/// A PARTIAL read: one selector resolves, one does not. The notice is a head
/// FIELD, so a consumer looks it up rather than noticing a sentence.
#[test]
fn sections_face_carries_the_partial_read_notice_as_a_field() {
    let text = render_face(
        "nested.md",
        NESTED,
        &json!({
            "id": 1, "op": "read", "path": "corpus/nested.md",
            "sections": [{"hpath": [{"h": "Notes"}]}, {"hpath": [{"h": "Ghost"}]}]
        }),
    );
    assert_golden("sections-partial", &text);
}

/// The positive control for the boundary claim: a section whose prose
/// contains a line spelled exactly like a body marker.
///
/// The marker is not the boundary — the head's `bytes` is — so the lookalike
/// is served verbatim and the face stays unambiguous. The golden is the proof:
/// it contains two `== 1 ==` lines, one structural and one authored, and the
/// head declares exactly one section.
#[test]
fn prose_may_spell_a_marker_without_forging_a_boundary() {
    let text = render_face(
        "lookalike.md",
        "# Notes\n\nthe face writes a line like\n\n== 1 ==\n\nand this section is ABOUT that line\n",
        &json!({
            "id": 1, "op": "read", "path": "corpus/lookalike.md",
            "sections": [{"hpath": [{"h": "Notes"}]}]
        }),
    );
    assert!(
        text.matches("== 1 ==").count() == 2,
        "the fixture really exercises the collision: {text}"
    );
    assert_golden("sections-marker-lookalike", &text);
}

/// The U4b elision law on the production face: an engine-emitted block is gone
/// from the rendered bodies, and the section still appears as a declared row.
/// Fully-elided is not empty.
#[test]
fn sections_face_elides_engine_emitted_blocks() {
    let text = render_face(
        "locked.md",
        "# Pins\n\nbefore the lock\n\n```meridian-lock\nv: 1\n```\n\nafter the lock\n",
        &json!({
            "id": 1, "op": "read", "path": "corpus/locked.md",
            "sections": [{"hpath": [{"h": "Pins"}]}]
        }),
    );
    assert!(
        !text.contains("meridian-lock"),
        "the engine block is elided from the RENDER face: {text}"
    );
    assert_golden("sections-elided-lock", &text);
}
