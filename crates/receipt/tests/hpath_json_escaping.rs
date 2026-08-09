//! §6.7 rule 2 gates: the hpath JSON a receipt template writes carries its
//! punctuation from the TEMPLATE, and no heading text can become part of it.
//!
//! The forging case is the reason this file exists. `is_receipt_ident_char`
//! permits `"`, so a heading carrying one — legal markdown any user may write
//! — would close the template's JSON string early and mint a receipt naming a
//! structurally different target than the write took (a §6.1 misreport). The
//! detector is standing rather than remembered.

fn seg(h: &str) -> wire::HpathSeg {
    wire::HpathSeg {
        h: h.into(),
        n: None,
    }
}

fn seg_n(h: &str, n: u32) -> wire::HpathSeg {
    wire::HpathSeg {
        h: h.into(),
        n: Some(n),
    }
}

/// The class this card exists for: a heading carrying a double quote cannot
/// extend or re-key the target object.
#[test]
fn a_quoted_heading_cannot_forge_the_target_object() {
    // A heading written to break out: close the string, close the object,
    // and open a second segment of the attacker's choosing.
    let forge = r#"Q3","n":9},{"h":"elsewhere"#;
    let rendered = receipt::render_hpath_json(&[seg("Goals"), seg(forge)]);

    assert!(
        !rendered.contains(r#"","n":9"#),
        "the forged occurrence key survived into the JSON: {rendered}"
    );
    // Exactly four `"` per segment — key and value, both the template's own,
    // never the data's. Two segments, so eight.
    assert_eq!(
        rendered.matches('"').count(),
        4 * 2,
        "quote count is the template's alone: {rendered}"
    );

    let parsed: Vec<wire::HpathSeg> = serde_json::from_str(&rendered).expect("strict parse");
    assert_eq!(parsed.len(), 2, "the array is still two segments");
    assert_eq!(parsed[1].h, forge, "the heading round-trips byte-for-byte");
    assert_eq!(parsed[1].n, None, "no occurrence index was forged");
}

/// The escape is JSON's own, so every hostile shape round-trips exactly —
/// escaping preserves the fact, it never edits it (§5.2 reversibility).
#[test]
fn every_out_of_charset_shape_round_trips_through_a_strict_parser() {
    for h in [
        "Goals",           // conforming
        "Q3 plan",         // space: a token boundary in a receipt line
        "say \"hi\"",      // the forging character
        "back\\slash",     // JSON's own escape introducer
        "[[wikilink]]",    // markdown structure
        "with `backtick`", // would close a code span early
        "line\nbreak",     // a row boundary
        "tab\there",       // control whitespace
        "\u{0}\u{1f}",     // C0 controls
        "café — 中文",     // non-ASCII, multi-byte
        "emoji 🧭 astral", // above U+FFFF: surrogate pair
        "",                // degenerate, not forged
    ] {
        let rendered = receipt::render_hpath_json(&[seg(h)]);
        let parsed: Vec<wire::HpathSeg> =
            serde_json::from_str(&rendered).unwrap_or_else(|e| panic!("{rendered:?}: {e}"));
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].h, h, "round-trip changed the fact: {rendered}");
    }
}

/// The rendered array is inert markdown and inert JSON: only template
/// punctuation appears outside the receipt-identifier charset, no `[[` can
/// form, no code span can open, no token or row boundary exists.
#[test]
fn the_rendered_array_carries_no_structure_the_template_did_not_write() {
    let hostile = "a\"b\\c[[d]]`e f\n\tg\u{7}🧭";
    let rendered = receipt::render_hpath_json(&[seg(hostile), seg_n(hostile, 2)]);

    assert!(
        !rendered.contains("[["),
        "a wikilink could form: {rendered}"
    );
    assert!(
        !rendered.contains('`'),
        "a code span could open: {rendered}"
    );
    assert!(
        !rendered.contains(' ') && !rendered.contains('\n') && !rendered.contains('\t'),
        "a token or row boundary survived: {rendered}"
    );
    assert!(
        !rendered.chars().any(char::is_control),
        "a control character survived: {rendered}"
    );
    assert!(
        rendered.is_ascii(),
        "non-ASCII survived unescaped: {rendered}"
    );
    // Every backslash opens a complete six-byte `\uXXXX` escape — so none can
    // consume the template's closing quote.
    let bytes = rendered.as_bytes();
    for (i, _) in rendered.match_indices('\\') {
        let esc = &bytes[i..(i + 6).min(bytes.len())];
        assert_eq!(esc.len(), 6, "a dangling backslash at {i}: {rendered}");
        assert_eq!(esc[1], b'u', "not a \\u escape at {i}: {rendered}");
        assert!(
            esc[2..].iter().all(u8::is_ascii_hexdigit),
            "short escape at {i}: {rendered}"
        );
    }
    // and the whole thing is still the address it claims to be
    let parsed: Vec<wire::HpathSeg> = serde_json::from_str(&rendered).expect("strict parse");
    assert_eq!(
        parsed[1].n,
        Some(2),
        "the occurrence index is data, escaped"
    );
}

/// Byte-neutrality on conforming text, and the §2.1 spelling exactly: the
/// contract's own worked segments render as the contract prints them, and
/// an absent `n` writes no key (§2.1, §9).
#[test]
fn conforming_segments_render_the_contract_form_byte_for_byte() {
    assert_eq!(
        receipt::render_hpath_json(&[seg("Goals"), seg("Q3")]),
        r#"[{"h":"Goals"},{"h":"Q3"}]"#
    );
    assert_eq!(
        receipt::render_hpath_json(&[seg_n("Beta", 2)]),
        r#"[{"h":"Beta","n":2}]"#
    );
    assert_eq!(receipt::render_hpath_json(&[]), "[]");
    // The escaping renderer is the identity on receipt-identifier text, so
    // adopting §6.7 moves no published byte.
    for h in ["Goals", "Q3", "2026-07-18", "a/b#c(1)"] {
        assert_eq!(receipt::render_hpath_segment_text(h), h);
    }
}

/// The one-character difference from [`receipt::render_field`], asserted so a
/// future refactor cannot quietly route segments through the field renderer.
#[test]
fn the_field_renderer_would_not_have_caught_the_quote() {
    // No space, so nothing else pushes this value out of the field charset:
    // the quote alone reaches the line untouched. That is the hazard, and it
    // is why a heading needs a renderer of its own rather than this one.
    let quoted = "say\"hi\"";
    assert_eq!(
        receipt::render_field(quoted),
        quoted,
        "field law still passes a quote through verbatim — this is the hazard"
    );
    assert!(
        receipt::render_hpath_segment_text(quoted).contains("\\u0022"),
        "the segment renderer must escape it"
    );
}
