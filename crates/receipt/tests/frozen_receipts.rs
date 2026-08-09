//! RC4-RECEIPT gates against the frozen text (§6.3 worked receipts): E3/E4
//! byte-exact, block-span widths independently confirmed, and the anchor
//! grammar fixture (in-charset per decision 011).

use receipt::{ArmedFacts, EditFact};

/// §6.3 E3's frozen bytes, single-sourced: the byte-exact gate and the
/// claim-link gate below must judge the SAME string, or one can go stale
/// while the other stays green.
const E3_FROZEN: &str = "- splice notes/plan.md id=42 actor=agent:b0864fb2 now=2026-07-18T20:31:04Z fingerprint_before=b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9 edits=1 target.hpath=[{\"h\":\"Goals\"},{\"h\":\"Q3\"}] match 33d5b0e1b27cb48b->41f643f034e5681f ^r-000042";

fn seg(h: &str) -> wire::HpathSeg {
    wire::HpathSeg {
        h: h.into(),
        n: None,
    }
}

/// §6.3 E3, exact bytes appended to `receipts/2026-07-18.md`: block span
/// `[26,286]` (terminator excluded, v1 leaf law), `node_rev 60bbee70d4a63a48`
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
    assert_eq!(line, E3_FROZEN);
    // independent width check: the frozen block span [26,286) is these bytes
    assert_eq!(line.len(), 286 - 26, "E3 block-span width");
}

/// §6.3 E4, exact bytes appended at S2: block span `[287,549]`, rev
/// `5c6ca7ec00ae279e` — the put:end (append-verb) rendering.
#[test]
fn e4_receipt_line_byte_exact() {
    let path = wire::Path("notes/plan.md".into());
    let root_before =
        wire::Root("b3:7f3b44376c719be236279e168c22fa2f4d346cd6e5da5bcf0784adb72e7c1f12".into());
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
        "- splice notes/plan.md id=57 actor=agent:b0864fb2 now=2026-07-18T20:33:41Z fingerprint_before=b3:7f3b44376c719be236279e168c22fa2f4d346cd6e5da5bcf0784adb72e7c1f12 edits=1 target.hpath=[{\"h\":\"Goals\"},{\"h\":\"Q4\"}] put:end 4b8bc385a58da0e0->f43203a1f0b4c9a3 ^r-000043"
    );
    assert_eq!(line.len(), 549 - 287, "E4 block-span width");
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
    assert!(line.starts_with("- splice notes/plan.md fingerprint_before="));
    assert!(line.contains(" ^blk-1 put:content "));
    assert!(line.ends_with(" ^r-000044"));
}

/// The rendered line carries NO claim-link token, and the gate is a measured
/// 1 → 0 rather than a terminal 0 whose provenance cannot be seen.
///
/// This became load-bearing in a new way when the template started writing
/// §2.1 JSON: `target.hpath=[…]` puts the FIRST `[` the template ever emitted
/// into a receipt line, so "no wikilink can form" stopped being true by the
/// field charset alone and started depending on §6.7 rule 2's segment escape.
/// The hostile heading below carries a well-formed token in a claim-link
/// position; it reads 1 raw and MUST read 0 once rendered.
#[test]
fn rendered_line_carries_no_claim_link_token() {
    // POSITIVE CONTROL FIRST. The token body is `tone.8hex` (`syntax::split_fp`);
    // a bare `@fp` is NOT well-formed and matches nothing, so a control written
    // that way reads 0 and certifies the assertion below without testing it.
    let hostile_raw = "Q[[x#^blk-1@green.b3af12cd]]3";
    assert_eq!(
        syntax::fp_removals(hostile_raw).len(),
        1,
        "positive control: the hostile heading really does forge a token in raw text"
    );

    let path = wire::Path("notes/plan.md".into());
    let root_before = wire::Root("b3:aa".into());
    let target = wire::SecRef::Hpath {
        hpath: vec![seg(hostile_raw), seg("Q3")],
    };
    let shape = wire::EditShape::Match {
        old: "a".into(),
        new: "b".into(),
    };
    let before = wire::NodeRev("aaaaaaaaaaaaaaaa".into());
    let after = wire::NodeRev("bbbbbbbbbbbbbbbb".into());
    let line = receipt::render_line(&ArmedFacts {
        id: Some(1),
        path: &path,
        actor: Some("agent:x"),
        now: Some("2026-07-18T20:31:04Z"),
        root_before: &root_before,
        anchor: "r-000001",
        edits: vec![EditFact {
            target: &target,
            shape: &shape,
            before: &before,
            after: &after,
        }],
    });
    assert_eq!(
        syntax::fp_removals(&line).len(),
        0,
        "the rendered line must carry no claim-link token: {line}"
    );
    // and the forging bytes are escaped rather than merely absent
    assert!(!line.contains("[["), "no doubled bracket in {line}");
    assert!(
        line.contains("\\u005b"),
        "the heading's `[` is escaped in {line}"
    );

    // The frozen E3 line is the same gate over the SHIPPED bytes.
    assert_eq!(syntax::fp_removals(E3_FROZEN).len(), 0, "E3 frozen line");
}

/// END TO END through `render_line`: a double-quote-bearing heading survives
/// the emission as recoverable JSON. Asked for by the integrator (8fcdcf2a)
/// when this card armed the renderer.
///
/// `hpath_json_escaping.rs` gates the renderer in ISOLATION. This gates the
/// SEAM — that `render_line` reaches the segment renderer at all, and that the
/// `target.hpath=` token it emits is parseable §2.1 form rather than merely
/// harmless-looking text. A template that quietly routed the array through
/// `render_field` would still pass a charset check and fail here, because the
/// address would arrive wrapped in a code span and parse as nothing.
#[test]
fn quoted_heading_survives_the_emission_as_recoverable_json() {
    // The heading a user is entitled to write, and the one that forges the
    // target object if the template interpolates it raw between its own quotes.
    let hostile = r#"Q3 "the good one" <&> ünïcode"#;
    let path = wire::Path("notes/plan.md".into());
    let root_before = wire::Root("b3:aa".into());
    let target = wire::SecRef::Hpath {
        hpath: vec![seg("Goals"), seg(hostile)],
    };
    let shape = wire::EditShape::Match {
        old: "a".into(),
        new: "b".into(),
    };
    let before = wire::NodeRev("aaaaaaaaaaaaaaaa".into());
    let after = wire::NodeRev("bbbbbbbbbbbbbbbb".into());
    let line = receipt::render_line(&ArmedFacts {
        id: Some(9),
        path: &path,
        actor: None,
        now: None,
        root_before: &root_before,
        anchor: "r-000009",
        edits: vec![EditFact {
            target: &target,
            shape: &shape,
            before: &before,
            after: &after,
        }],
    });

    // POSITIVE CONTROL: the raw heading really would close the string early —
    // so a green below is the escape working, not the input being harmless.
    assert!(
        hostile.contains('"'),
        "control: the heading must carry the character that forges the object"
    );

    // Cut the token out of the line the way a reader would, then hand it to a
    // STRICT parser rather than reading the escape by eye.
    let token = line
        .split(' ')
        .find(|t| t.starts_with("target.hpath="))
        .expect("the line carries a target.hpath= token");
    let json = token.strip_prefix("target.hpath=").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(json)
        .unwrap_or_else(|e| panic!("target.hpath is not valid JSON: {e}\n  token: {token}"));

    // The address round-trips: two segments, the heading recovered BYTE-EXACT.
    assert_eq!(parsed.as_array().map(Vec::len), Some(2), "two segments");
    assert_eq!(parsed[0]["h"], "Goals");
    assert_eq!(
        parsed[1]["h"], hostile,
        "the heading must survive byte-for-byte through the escape"
    );

    // The forging bytes never stood verbatim, and the line stays one row.
    assert!(!json.contains(r#""the good one""#), "quotes were escaped");
    assert!(!line.contains('\n'), "no row boundary can be forged");
    assert_eq!(syntax::fp_removals(&line).len(), 0, "no claim-link token");
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
