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
/// **The I4 judge is asserted FIRST, on purpose.** The receipts' refusal is the
/// defect; the emitted bytes are only how it got there. Asserting the line first
/// would redden on the base for a string mismatch and never reach the finding
/// that actually refused the write, so the negative proof would show the wrong
/// failure. On the unfixed base this reddens on the substrate-law finding
/// itself, verbatim from the receipts.
#[test]
fn a_wikilink_shaped_value_lands_as_a_scalar_and_i4_is_silent() {
    let cand = set_property("reviewer", "[[b1892b5a]]");
    let findings = scan_nested(&cand, "cand.md");
    assert!(
        findings.is_empty(),
        "the emitter must never manufacture the nesting I4 then refuses. \
         Candidate line: {:?}; findings: {findings:?}",
        line_for(&cand, "reviewer")
    );
    assert_eq!(
        line_for(&cand, "reviewer"),
        r#"reviewer: "[[b1892b5a]]""#,
        "the emit is the fleet-canonical quoted form"
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

/// The ONE spelling that stays VERBATIM by standing contract — the only way
/// this string plane can author a list. A fix that quoted it would close a
/// defect by breaking a working feature.
///
/// `true` and `7` left this list on 2026-08-23 (card
/// `all-digit-short-ids-read-as-int`) — `a_value_a_yaml_parser_reads_as_a_number_or_a_bool_is_quoted`
/// is their new home. `2026-08-07` stays: `serde_yaml` reads a timestamp back
/// as a string, so the plain emit already round-trips as the caller's string.
#[test]
fn one_level_flow_lists_still_emit_verbatim() {
    for value in [
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

// ── § A.6.3c — spelling preservation on a semantic no-op ─────────────────────

/// One `set_property` through the shared candidate composer, over a chosen doc.
fn set_property_on(raw: &str, key: &str, value: &str) -> model::Document {
    let edits = [PlanEdit {
        op: "set_property".to_string(),
        target: vec![Seg {
            h: key.to_string(),
            n: None,
        }],
        body: value.to_string(),
        ..PlanEdit::default()
    }];
    rebuild(&doc(raw), &edits, &|raw| doc(raw))
        .unwrap_or_else(|e| panic!("set_property {key}={value:?} was refused: {}", e.render()))
}

/// **The oscillation, pinned (review gate d5654f18, P2).** The fleet-canonical
/// quoted spelling — the bytes `ccc-cli task claim` writes — written back with
/// the exact value the read law serves must leave the DOCUMENT byte-identical.
/// § A.6.2's planes (`prop_rev`, `span`, `props1`, pins) are computed over
/// source bytes, so byte identity is the whole claim: nothing moves on a
/// semantic no-op, and the two writers stop oscillating.
#[test]
fn a_write_back_of_the_served_value_is_byte_stable() {
    for raw in [
        "---\ntype: task\nowner: \"3f9a1c07\"\n---\n\n# Todo\n",
        "---\ntype: task\nowner: 'doing'\n---\n\n# Todo\n", // legacy single-quoted spelling
        "---\ntype: task\nowner: \"42\"\n---\n\n# Todo\n",  // quoted typed scalar stays a STRING
        "---\ntype: task\nowner: plain\n---\n\n# Todo\n", // plain spelling: encoder emits it anyway
    ] {
        let d = doc(raw);
        let served = reads_back_as(&d, "owner");
        let cand = set_property_on(raw, "owner", &served);
        assert_eq!(
            cand.raw, raw,
            "write-back of served value {served:?} must keep the stored spelling"
        );
    }
}

/// The § A.6.3c exclusions: a NULL or NESTED stored spelling is never
/// preserved — the text-equal write-back re-encodes to the quoted canonical
/// form, because those are the two classes this string plane cannot express.
#[test]
fn null_and_nested_spellings_still_re_encode_on_a_text_equal_write_back() {
    for (raw, value, want_line) in [
        // bare key: the caller's "" lands the empty STRING, not a preserved null (R4)
        ("---\nowner:\n---\n\n# T\n", "", r#"owner: """#),
        ("---\nowner: ~\n---\n\n# T\n", "~", r#"owner: "~""#),
        ("---\nowner: null\n---\n\n# T\n", "null", r#"owner: "null""#),
        // stored nesting is repaired to the quoted form, never preserved
        (
            "---\nowner: [[b1892b5a]]\n---\n\n# T\n",
            "[[b1892b5a]]",
            r#"owner: "[[b1892b5a]]""#,
        ),
    ] {
        let cand = set_property_on(raw, "owner", value);
        assert_eq!(
            line_for(&cand, "owner"),
            want_line,
            "text-equal write-back over {raw:?}"
        );
        assert_eq!(reads_back_as(&cand, "owner"), value);
    }
}

/// **The indicator class, measured not enumerated** (2026-08-23, card 17).
///
/// The § A.6.3 rule is "emit plain when the plain form decodes back to exactly
/// the caller's string". Until this landed, that question was asked only of the
/// engine's OWN classifier, which is more permissive than YAML: every value
/// below emitted plain, and the first group produces bytes NO yaml parser can
/// read — the whole frontmatter block dies, not one key. Measured with `PyYAML`
/// on `k: <val>` before the fix; pinned here so the class cannot reopen.
#[test]
fn plain_scalars_that_no_yaml_parser_can_read_are_quoted() {
    for value in [
        // ScannerError / ParserError — the block is unreadable
        "- not a list item",
        "- ",
        "-",
        "? question",
        "?",
        ", comma",
        ",",
        "* alias",
        "&anchor",
        "%directive",
        "@at",
        "`tick",
        "]close",
        "}close",
        "[[a]] and [[b]]",
        // parses, but as something the caller never wrote
        "!tag",
        ">fold",
        "|block",
        // an INTERIOR tab: `serde_yaml` reads it, `PyYAML` dies on the whole
        // block ("while scanning for the next token"), so the parser oracle
        // alone never flagged it (2026-08-23, measured over the live root)
        "a\tb",
    ] {
        let emitted = yaml_safe_value(value).expect("single-line values encode");
        assert_ne!(
            emitted, value,
            "{value:?} must not ride plain — it is not readable as itself"
        );
        assert_eq!(
            model::scalar::text(&emitted),
            value,
            "round trip for {value:?}"
        );
        let parsed: serde_yaml::Value = serde_yaml::from_str(&format!("k: {emitted}\n"))
            .unwrap_or_else(|e| panic!("the emit for {value:?} must parse as yaml: {e}"));
        assert_eq!(
            parsed.get("k").and_then(serde_yaml::Value::as_str),
            Some(value),
            "a real yaml parser reads the emit for {value:?} back as itself"
        );
    }
}

/// The other half of the same fix: values that were ALREADY safe stay plain,
/// so the fix costs no corpus churn. A `-` or `?` inside the text, the
/// timestamp spelling, and the flow-list carve-out are untouched.
#[test]
fn safe_plain_scalars_are_still_emitted_plain() {
    for value in [
        "a - b",
        "a ? b",
        "x, y",
        "-nodash",
        "--- not a fence",
        "...",
        "it's mine",
        "a \" quote",
        "2026-08-07",
        "2026-08-07T11:03:41-04:00",
        "[a, b]",
        r"C:\path",
    ] {
        assert_eq!(
            yaml_safe_value(value),
            Ok(value.to_string()),
            "verbatim emit for {value:?}"
        );
    }
}

// ── the number/bool class (2026-08-23, card `all-digit-short-ids-read-as-int`) ─

/// Every all-digit 8-hex agent short id that already sits BARE in the live
/// sessions root's frontmatter (37 distinct, under `session`, `agent`, `from`,
/// `owner`, `author`, `worker`, `leader`, `created_by`, `interrogator`), plus
/// the two spellings the review named. Ids are the fleet's join key: `owner`,
/// `spawned-by`, `handoff-from` are compared as strings, so an id that reads
/// back as a NUMBER joins to nothing.
const LIVE_ALL_DIGIT_IDS: [&str; 39] = [
    "01742328", "02016429", "02146210", "05376152", "05667469", "09639133", "09695835", "19895504",
    "23110406", "27386265", "39268943", "44090036", "44343639", "44870138", "47167772", "48316841",
    "50466467", "51407904", "54396085", "60636352", "64246177", "65871869", "71561417", "76589640",
    "77769434", "78267924", "81937540", "82525787", "83480721", "84956441", "88191010", "88877785",
    "92574281", "92829274", "94485806", "95253977", "98594329",
    // the review's two, kept even if they leave the live root
    "00000042", // leading zeros: PyYAML reads YAML 1.1 octal, serde_yaml decimal 42
    "12345678",
];

/// **The defect this pins** (PR 185 review finding F2, card
/// `all-digit-short-ids-read-as-int`). `plain_reads_back` used to override
/// `serde_yaml` for the Int/Float/Bool classes, so an all-digit short id was
/// emitted PLAIN: `19895504` read back as the integer 19 895 504, and
/// `02146210` as 576 648 (YAML 1.1 octal, `PyYAML`) — 203 of 8 125 distinct 8-hex
/// ids on the live root (2.5 %) are all-digit, and 8-hex git shas share the
/// shape. The write seam now emits them quoted, and a real YAML parser reads
/// the emit back as the caller's STRING.
#[test]
fn an_all_digit_short_id_is_written_so_a_yaml_parser_reads_it_as_a_string() {
    for id in LIVE_ALL_DIGIT_IDS {
        let emitted = yaml_safe_value(id).expect("an id is single-line");
        assert_eq!(emitted, format!("\"{id}\""), "emit for id {id:?}");
        assert_eq!(model::scalar::text(&emitted), id, "round trip for {id:?}");
        let parsed: serde_yaml::Value = serde_yaml::from_str(&format!("owner: {emitted}\n"))
            .unwrap_or_else(|e| panic!("the emit for {id:?} must parse as yaml: {e}"));
        assert_eq!(
            parsed.get("owner").and_then(serde_yaml::Value::as_str),
            Some(id),
            "a real yaml parser must read the emit for {id:?} back as the string"
        );
        // …and through the shared candidate composer, not just the encoder.
        let cand = set_property("owner", id);
        assert_eq!(line_for(&cand, "owner"), format!("owner: \"{id}\""));
        assert_eq!(reads_back_as(&cand, "owner"), id);
        assert!(scan_nested(&cand, "cand.md").is_empty());
    }
}

/// The class the id belongs to, stated as the law: a value a REAL yaml parser
/// reads back as a number, a bool or a null is quoted, whatever the engine's
/// own classifier makes of it. The one survivor is the flow list, which this
/// string plane has no other way to author.
#[test]
fn a_value_a_yaml_parser_reads_as_a_number_or_a_bool_is_quoted() {
    for value in [
        "7", "0", "-36", "+7", "0.5", "1e3", "0657e070", // int / float spellings
        "true", "false", "True", "FALSE", // bools
        "0x1f", "0o17", "1_000", // yaml 1.2 int spellings serde_yaml resolves
        ".inf", ".nan", // float specials
    ] {
        let emitted = yaml_safe_value(value).expect("single-line values encode");
        let parsed: serde_yaml::Value = serde_yaml::from_str(&format!("k: {emitted}\n"))
            .unwrap_or_else(|e| panic!("the emit for {value:?} must parse as yaml: {e}"));
        let read = parsed.get("k").and_then(serde_yaml::Value::as_str);
        assert_eq!(
            read,
            Some(value),
            "a yaml parser must read the emit {emitted:?} for {value:?} back as the string"
        );
        assert_eq!(model::scalar::text(&emitted), value, "round trip {value:?}");
    }
}

/// **The residual, pinned rather than claimed away.** § A.6.3c preservation is
/// unchanged by the fix: an id already stored PLAIN keeps its stored bytes on a
/// same-value write-back, so the 37 live bare ids are not repaired by this
/// change alone — only a write that CHANGES the value re-encodes. That is the
/// PR 185 discipline (no mass churn), and it is the reason the measured churn
/// is a population, not a rewrite.
#[test]
fn an_id_already_stored_plain_is_still_preserved_on_a_no_op_write_back() {
    let raw = "---\ntype: task\nowner: 19895504\n---\n\n# Todo\n";
    let d = doc(raw);
    let served = reads_back_as(&d, "owner");
    assert_eq!(served, "19895504", "the read law serves the id as a string");
    let cand = set_property_on(raw, "owner", &served);
    assert_eq!(cand.raw, raw, "a semantic no-op moves no byte");
    // A CHANGED write repairs the spelling.
    let changed = set_property_on(raw, "owner", "02146210");
    assert_eq!(line_for(&changed, "owner"), r#"owner: "02146210""#);
}

/// **The 1.1 blind spot, pinned.** `serde_yaml` resolves the YAML **1.2** core
/// schema; `PyYAML` and go-yaml (`gopkg.in/yaml.v3`, which `ccc-statusd` links)
/// resolve **1.1**, and both of them answer 576 648 for `02146210`. Deferring to the 1.2 parser alone left the WORSE half of
/// the id defect standing: `02146210` is the string `"02146210"` to
/// `serde_yaml` — a leading zero is not a 1.2 integer — and the integer 576 648
/// to `PyYAML`, which reads a leading-zero digit run as octal. The law is the
/// union of the schemas, so these quote even though `serde_yaml` is content.
#[test]
fn the_yaml_1_1_only_typed_spellings_are_quoted_too() {
    for value in [
        "02146210", // 1.1 octal — the review's example, and 7 live ids share the shape
        "0002",
        "000",
        "007",       // the same class, shorter
        "1_000",     // 1.1 underscore grouping
        "12:30",     // 1.1 sexagesimal: 750 (PyYAML, measured)
        "1:02:03",   // …the three-group form: 3723
        "1:2:3:4:5", // …and the five-group form: 13 403 045
        "yes",
        "no",
        "on",
        "off",
        "Yes",
        "OFF", // 1.1 booleans
        // `y`/`n` are booleans in the 1.1 SPEC and strings in `PyYAML`; a
        // one-letter value quotes rather than depend on which reader is right.
        "y",
        "N",
        // The radix ints. `0x1f`/`0o17` are 1.2 integers the parser oracle
        // already catches; the UNDERSCORE spellings and every `0b…` are
        // 1.1-only — `0x1_f` is 31 and `0b1_010` is 10 to `PyYAML`, and plain
        // strings to `serde_yaml` (measured 2026-08-23).
        "0b1010",
        "0b1_010",
        "0x1f",
        "0x1_f",
        "0o17",
        "011",
        // The two 1.1 resolver TAGS, which are not types at all: emitted plain,
        // `PyYAML` refuses the WHOLE block — "could not determine a constructor
        // for the tag …:merge / …:value" — while `serde_yaml` reads both back
        // as strings, so only this arm catches them.
        "<<",
        "=",
        // the 1.1 sexagesimal FLOAT, and the float specials
        "1:30.5",
        ".inf",
        "-.inf",
        ".nan",
        ".NaN",
    ] {
        let emitted = yaml_safe_value(value).expect("single-line values encode");
        assert_eq!(emitted, format!("\"{value}\""), "emit for {value:?}");
        assert_eq!(model::scalar::text(&emitted), value, "round trip {value:?}");
    }
}

/// The other half of that trigger: values that only LOOK like the 1.1 classes
/// keep their plain spelling, so the widening costs no churn it did not have to.
#[test]
fn near_misses_of_the_yaml_1_1_classes_stay_plain() {
    for value in [
        "12:345",     // not sexagesimal: a group of three digits
        "1:60",       // …nor this: a base-60 digit is 0–59 (PyYAML: a string)
        "yesterday",  // not the boolean `yes`
        "no-one",     //
        "on-call",    //
        "2026-08-07", // a timestamp reads back as a string in serde_yaml
        "0657e070",   // an id, but this one already quotes as a 1.2 float
        "v1_000",     // digits with a letter: a plain string in both schemas
        "22-18-hook", // a session slug
    ] {
        let emitted = yaml_safe_value(value).expect("single-line values encode");
        if value == "0657e070" {
            assert_eq!(emitted, "\"0657e070\"", "the exponent form still quotes");
            continue;
        }
        assert_eq!(emitted, value, "verbatim emit for {value:?}");
    }
}
