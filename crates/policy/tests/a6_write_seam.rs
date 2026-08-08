//! § A.6.3 — the write half of the frontmatter scalar law, at the seam the
//! dogfood receipts measured.
//!
//! **The defect this pins (dogfood season 1, finding 2, fail-CLOSED).** A flat
//! caller string `[[b1892b5a]]` was emitted VERBATIM, which is a YAML
//! list-of-list, so the I4 substrate law refused the write and taught the
//! caller to "flatten the value" — a value that was already flat. The nesting
//! was the EMITTER's. Net: the engine could read the fleet-canonical
//! `owner: "[[b1892b5a]]"` and could not write it.
//!
//! **Why the refusal is the negative proof.** Asserting the new bytes alone
//! would pass on an engine that merely changed its quote style. These tests
//! drive the value through `rebuild` — the same candidate composer the splice
//! and `check_write` share — and then through `scan_nested`, the I4 judge that
//! did the refusing. A run that still manufactures nesting reddens at the judge,
//! not at a string compare.

use policy::defs::{PlanEdit, Seg, rebuild, scan_nested, yaml_safe_value};

const DOC: &str = "---\ntype: note\nstatus: seeded\n---\n\n# Todo\n\n- [ ] first item\n";

fn doc(raw: &str) -> model::Document {
    model::build(raw.to_string(), syntax::parse(raw))
}

/// One `set_property` through the shared candidate composer.
fn set_property(key: &str, value: &str) -> model::Document {
    let edits = [PlanEdit {
        op: "set_property".to_string(),
        target: vec![Seg {
            h: key.to_string(),
            n: None,
        }],
        body: value.to_string(),
        ..PlanEdit::default()
    }];
    rebuild(&doc(DOC), &edits, &|raw| doc(raw))
        .unwrap_or_else(|e| panic!("set_property {key}={value:?} was refused: {}", e.render()))
}

/// The frontmatter line the candidate carries for `key`, verbatim.
fn line_for(cand: &model::Document, key: &str) -> String {
    cand.raw
        .lines()
        .find(|l| l.starts_with(&format!("{key}:")))
        .unwrap_or_else(|| panic!("no `{key}` line in the candidate:\n{}", cand.raw))
        .to_string()
}

/// The value the READ law serves back off that line — the composed round trip.
fn reads_back_as(cand: &model::Document, key: &str) -> String {
    let line = line_for(cand, key);
    let (_, rest) = line.split_once(':').expect("a key line carries a colon");
    model::scalar::text(rest)
}

// ── the negative proof, write half ───────────────────────────────────────────

/// **The headline.** The fleet's own `owner` value — the spelling
/// `ccc-cli task claim` writes — passed as the flat string it is, lands as a
/// scalar, and the I4 judge finds nothing to refuse.
///
/// On the unfixed base this reddens twice over: the emitted line is
/// `reviewer: [[b1892b5a]]` and `scan_nested` returns the substrate-law error.
#[test]
fn a_wikilink_shaped_value_lands_as_a_scalar_and_i4_is_silent() {
    let cand = set_property("reviewer", "[[b1892b5a]]");
    assert_eq!(
        line_for(&cand, "reviewer"),
        r#"reviewer: "[[b1892b5a]]""#,
        "the emit is the fleet-canonical quoted form"
    );
    let findings = scan_nested(&cand, "cand.md");
    assert!(
        findings.is_empty(),
        "the emitter must never manufacture the nesting I4 then refuses: {findings:?}"
    );
}

/// The other half of the same claim: the value survives the trip. A quoted emit
/// that the read law could not undo would trade a fail-CLOSED for a fail-INERT.
#[test]
fn the_written_value_reads_back_as_the_caller_passed_it() {
    let cand = set_property("reviewer", "[[b1892b5a]]");
    assert_eq!(reads_back_as(&cand, "reviewer"), "[[b1892b5a]]");
}

// ── the § A.6.3 trigger table ────────────────────────────────────────────────

/// Every trigger, and — the part that makes it a law rather than a list — the
/// round trip through the read half for each one.
#[test]
fn every_quote_trigger_round_trips() {
    for (value, want_line) in [
        ("[[b1892b5a]]", r#"k: "[[b1892b5a]]""#), // list-of-list: the I4 forge
        ("{a: b}", r#"k: "{a: b}""#),             // a map in value position
        ("[unterminated", r#"k: "[unterminated""#),
        ("review: pending", r#"k: "review: pending""#), // a mapping
        ("trailing:", r#"k: "trailing:""#),
        ("#hashtag", r##"k: "#hashtag""##), // a comment: would read back empty
        ("value # note", r#"k: "value # note""#),
        (r#""already quoted""#, r#"k: "\"already quoted\"""#),
        ("'single'", r#"k: "'single'""#),
        ("", r#"k: """#), // the plane has no null to mean
        ("null", r#"k: "null""#),
        ("~", r#"k: "~""#),
        (" leading space", r#"k: " leading space""#),
        // The `\` escape the double form owns, reached through a trigger — a
        // bare `C:\path` is a legal plain scalar and stays verbatim (below).
        (r"[[C:\path]]", r#"k: "[[C:\\path]]""#),
    ] {
        let emitted = yaml_safe_value(value).expect("single-line values encode");
        assert_eq!(format!("k: {emitted}"), want_line, "emit for {value:?}");
        assert_eq!(
            model::scalar::text(&emitted),
            value,
            "round trip for {value:?}"
        );
    }
}

/// The two spellings that stay VERBATIM by standing contract — the only way
/// this string plane can author a non-string value. A fix that quoted these
/// would close the defect by breaking a working feature.
#[test]
fn typed_scalars_and_one_level_flow_lists_still_emit_verbatim() {
    for value in [
        "true",
        "7",
        "2026-08-07",
        "[a, b]",
        "[]",
        "plain text",
        r"C:\path", // `\` is not an escape in a plain scalar: it round-trips bare
    ] {
        assert_eq!(
            yaml_safe_value(value),
            Ok(value.to_string()),
            "verbatim emit for {value:?}"
        );
    }
    let cand = set_property("status", "[a, b]");
    assert_eq!(line_for(&cand, "status"), "status: [a, b]");
    assert!(scan_nested(&cand, "cand.md").is_empty());
}

/// A newline is still REFUSED, never sanitized, and never quoted away: the
/// fallible signature is the guard, and § A.6.3 does not relax it.
#[test]
fn a_newline_is_still_refused_rather_than_encoded() {
    assert_eq!(
        yaml_safe_value("seeded\ninjected: pwned"),
        Err(policy::defs::MultiLineValue)
    );
    assert_eq!(
        yaml_safe_value("seeded\rinjected: pwned"),
        Err(policy::defs::MultiLineValue)
    );
}

/// The season-1 corpus, written rather than read: each fleet-canonical value
/// the receipts measured on the READ side is set through the write seam and
/// must come back off the disk bytes unchanged.
#[test]
fn the_season_one_corpus_survives_a_write_then_read() {
    for value in ["", "3f9a1c07", "[[1ed98864]]", "doing"] {
        let cand = set_property("owner", value);
        assert_eq!(
            reads_back_as(&cand, "owner"),
            value,
            "write→read for {value:?}, candidate line {:?}",
            line_for(&cand, "owner")
        );
    }
}
