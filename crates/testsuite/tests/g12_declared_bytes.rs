//! G12 — the bytes-boundary property, asserted rather than frozen.
//!
//! The registered rule (`docs/wire-contract.md` § A.3, the counting law:
//! "`bytes` alone declares the served length") is that a sections body is
//! bounded by the head's declared `bytes` rather than by its markers: a reader
//! takes each body's declared byte count after its marker line and never scans
//! for a boundary, because content may spell one.
//!
//! Not a golden: a regenerated golden freezes the face as it is, defect
//! included, so a golden can never gate this class. The gate is the property —
//! for every body in a sections read, head-declared `bytes` equals bytes
//! served. [`reader`] is the strict reader the law describes: it consumes each
//! body by its declared length and requires the face to end exactly there. The
//! stored-cross-root-link case runs in its own binary (`g12_stored_link.rs`):
//! the link reads back only under a bound mount table, and `MERIDIAN_CONFIG`
//! is process-global.

pub(crate) mod reader;

use reader::{bodies_by_declared_length, render_face};
use serde_json::{Value, json};

const NESTED: &str = "---\ntype: note\n---\n\n# Todo\n\n- [ ] first item\n\n# Notes\n\nseed note line one\n\n## Slash/Title Here\n\ndeep content\n\n## 世界\n\nunicode body\n";

/// **The named case from the ruling.** On the lookalike fixture, scanning is
/// forbidden — the prose spells `== 1 ==` itself — so the declaration is the
/// only parse there is, and it must be exact.
#[test]
fn the_marker_lookalike_body_serves_exactly_its_declared_bytes() {
    let text = render_face(
        "lookalike.md",
        "# Notes\n\nthe face writes a line like\n\n== 1 ==\n\nand this section is ABOUT that line\n",
        &json!({
            "id": 1, "op": "read", "path": "corpus/lookalike.md",
            "sections": [{"hpath": [{"h": "Notes"}]}]
        }),
    );
    assert_eq!(
        text.matches("== 1 ==").count(),
        2,
        "the fixture really exercises the collision: {text}"
    );
    let bodies = bodies_by_declared_length("marker-lookalike", &text);
    assert_eq!(bodies.len(), 1);
    assert!(
        bodies[0].ends_with("and this section is ABOUT that line\n"),
        "the declared-length parse recovered the body's LAST byte — the \
         trailing newline the head counted: {:?}",
        bodies[0]
    );
}

/// The property over every shape the sections face takes: a single body, a
/// partial read carrying a notice, several bodies in one read (so a NON-final
/// body is covered too), a body whose engine block was elided, and a
/// unicode body (where a byte length is not a char count).
#[test]
fn every_body_in_a_sections_read_serves_its_declared_bytes() {
    let cases: [(&str, &str, Value); 5] = [
        (
            "one",
            "nested.md",
            json!({"id": 1, "op": "read", "path": "corpus/nested.md",
                   "sections": [{"hpath": [{"h": "Notes"}]}]}),
        ),
        (
            "partial",
            "nested.md",
            json!({"id": 1, "op": "read", "path": "corpus/nested.md",
                   "sections": [{"hpath": [{"h": "Notes"}]}, {"hpath": [{"h": "Ghost"}]}]}),
        ),
        (
            "many",
            "nested.md",
            json!({"id": 1, "op": "read", "path": "corpus/nested.md",
                   "sections": [{"hpath": [{"h": "Todo"}]},
                                {"hpath": [{"h": "Notes"}, {"h": "Slash/Title Here"}]},
                                {"hpath": [{"h": "Notes"}, {"h": "世界"}]}]}),
        ),
        (
            "unicode",
            "nested.md",
            json!({"id": 1, "op": "read", "path": "corpus/nested.md",
                   "sections": [{"hpath": [{"h": "Notes"}, {"h": "世界"}]}]}),
        ),
        (
            "elided-lock",
            "locked.md",
            json!({"id": 1, "op": "read", "path": "corpus/locked.md",
                   "sections": [{"hpath": [{"h": "Pins"}]}]}),
        ),
    ];

    for (case, doc, request) in cases {
        let source = if doc == "locked.md" {
            "# Pins\n\nbefore the lock\n\n```meridian-lock\nv: 1\n```\n\nafter the lock\n"
        } else {
            NESTED
        };
        let text = render_face(doc, source, &request);
        let bodies = bodies_by_declared_length(case, &text);
        assert!(!bodies.is_empty(), "[{case}] the read served bodies");
    }
}
