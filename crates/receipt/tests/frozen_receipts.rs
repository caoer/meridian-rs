//! RC4-RECEIPT gates against the frozen text (§6.3 worked receipts): E3/E4
//! byte-exact, block-span widths independently confirmed, and the anchor
//! grammar fixture (in-charset per decision 011).

use receipt::{ArmedFacts, EditFact};

fn seg(h: &str) -> wire::HpathSeg {
    wire::HpathSeg {
        h: h.into(),
        n: None,
    }
}

/// §6.3 E3, exact bytes appended to `receipts/2026-07-18.md`: block span
/// `[26,248]` (terminator excluded, v1 leaf law), `node_rev 639a2dca46f6fcc8`
/// over exactly these bytes.
#[test]
fn e3_receipt_line_byte_exact() {
    let path = wire::Path("notes/plan.md".into());
    let root_before =
        wire::Root("b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9".into());
    let target = wire::SecRef::Hpath {
        hpath: vec![seg("Goals"), seg("Q3")],
    };
    let shape = wire::EditShape::Match {
        old: "ship by August".into(),
        new: "ship by September".into(),
    };
    let before = wire::NodeRev("33d5b0e1b27cb48b".into());
    let after = wire::NodeRev("41f643f034e5681f".into());
    let line = receipt::render_line(&ArmedFacts {
        id: Some(42),
        path: &path,
        actor: Some("agent:b0864fb2"),
        now: Some("2026-07-18T20:31:04Z"),
        root_before: &root_before,
        anchor: "r-000042",
        edits: vec![EditFact {
            target: &target,
            shape: &shape,
            before: &before,
            after: &after,
        }],
    });
    assert_eq!(
        line,
        "- splice notes/plan.md id=42 actor=agent:b0864fb2 now=2026-07-18T20:31:04Z root_before=b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9 edits=1 Goals>Q3 match 33d5b0e1b27cb48b->41f643f034e5681f ^r-000042"
    );
    // independent width check: the frozen block span [26,248) is these bytes
    assert_eq!(line.len(), 248 - 26, "E3 block-span width");
}

/// §6.3 E4, exact bytes appended at S2: block span `[249,473]`, rev
/// `c912d4578883f288` — the put:end (append-verb) rendering.
#[test]
fn e4_receipt_line_byte_exact() {
    let path = wire::Path("notes/plan.md".into());
    let root_before =
        wire::Root("b3:10769ae1c77f5646750f3f52df2d055156b411145a02b8361ecd32af1357a1b7".into());
    let target = wire::SecRef::Hpath {
        hpath: vec![seg("Goals"), seg("Q4")],
    };
    let shape = wire::EditShape::Put {
        at: wire::PutAt::End,
        text: "- new item\n".into(),
    };
    let before = wire::NodeRev("4b8bc385a58da0e0".into());
    let after = wire::NodeRev("f43203a1f0b4c9a3".into());
    let line = receipt::render_line(&ArmedFacts {
        id: Some(57),
        path: &path,
        actor: Some("agent:b0864fb2"),
        now: Some("2026-07-18T20:33:41Z"),
        root_before: &root_before,
        anchor: "r-000043",
        edits: vec![EditFact {
            target: &target,
            shape: &shape,
            before: &before,
            after: &after,
        }],
    });
    assert_eq!(
        line,
        "- splice notes/plan.md id=57 actor=agent:b0864fb2 now=2026-07-18T20:33:41Z root_before=b3:10769ae1c77f5646750f3f52df2d055156b411145a02b8361ecd32af1357a1b7 edits=1 Goals>Q4 put:end 4b8bc385a58da0e0->f43203a1f0b4c9a3 ^r-000043"
    );
    assert_eq!(line.len(), 473 - 249, "E4 block-span width");
}

/// §9 absent-inputs law rendered: no actor, no now, no id → no tokens
/// (absent inputs produce absent facts; the engine records nothing it
/// wasn't told).
#[test]
fn absent_inputs_render_absent_facts() {
    let path = wire::Path("notes/plan.md".into());
    let root_before =
        wire::Root("b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9".into());
    let target = wire::SecRef::Anchor {
        anchor: "blk-1".into(),
    };
    let shape = wire::EditShape::Put {
        at: wire::PutAt::Content,
        text: "x".into(),
    };
    let before = wire::NodeRev("aaaaaaaaaaaaaaaa".into());
    let after = wire::NodeRev("bbbbbbbbbbbbbbbb".into());
    let line = receipt::render_line(&ArmedFacts {
        id: None,
        path: &path,
        actor: None,
        now: None,
        root_before: &root_before,
        anchor: "r-000044",
        edits: vec![EditFact {
            target: &target,
            shape: &shape,
            before: &before,
            after: &after,
        }],
    });
    assert!(!line.contains("id="), "absent id renders no token");
    assert!(!line.contains("actor="), "absent actor renders no token");
    assert!(!line.contains("now="), "absent now renders no token");
    assert!(line.starts_with("- splice notes/plan.md root_before="));
    assert!(line.contains(" ^blk-1 put:content "));
    assert!(line.ends_with(" ^r-000044"));
}

/// Anchor grammar fixture (gate 3): `r-NNNNNN` zero-padded, widening past
/// six digits, always inside the block-id charset `[A-Za-z0-9-]+`
/// (§2.4, decision 011).
#[test]
fn anchor_grammar_in_charset() {
    let in_charset = |s: &str| s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    assert_eq!(receipt::anchor(42), "r-000042");
    assert_eq!(receipt::anchor(0), "r-000000");
    assert_eq!(receipt::anchor(999_999), "r-999999");
    assert_eq!(receipt::anchor(1_234_567), "r-1234567"); // widens, still legal
    for n in [0, 1, 42, 999_999, 1_234_567, u64::MAX] {
        let a = receipt::anchor(n);
        assert!(in_charset(&a), "anchor {a} outside [A-Za-z0-9-]+");
        assert!(!a.contains('_'), "no underscore ever (011)");
    }
}
