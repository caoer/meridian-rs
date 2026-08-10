//! Def rebuild unit gates — rev neutralization and plan-edit application.

use policy::defs::{PlanEdit, Seg, rebuild, rev8};

// The rebuild corpus doc IS the basic doc (same bytes, per the golden).
const DOC: &str = "---\ntype: note\nstatus: seeded\n---\n\n# Todo\n\n- [ ] first item\n- [ ] second item ^task1\n\n# Notes\n\nseed note line one\nseed note line two\n\n## Slash/Title Here\n\ndeep content\n\n# Memo\n\nseed memo\n";

fn doc(raw: &str) -> model::Document {
    model::build(raw.to_string(), syntax::parse(raw))
}

fn run(prev_raw: &str, edits: &[PlanEdit]) -> Result<model::Document, policy::defs::BodyError> {
    let prev = doc(prev_raw);
    rebuild(&prev, edits, &|raw| doc(raw))
}

fn seg(h: &str) -> Seg {
    Seg {
        h: h.to_string(),
        n: None,
    }
}

fn segn(h: &str, n: u32) -> Seg {
    Seg {
        h: h.to_string(),
        n: Some(n),
    }
}

fn segs(hs: &[&str]) -> Vec<Seg> {
    hs.iter().map(|h| seg(h)).collect()
}

fn edit(op: &str, target: &str) -> PlanEdit {
    PlanEdit {
        op: op.to_string(),
        // C-08: the address is an ARRAY. The helper keeps taking the human
        // spelling so the cases read as before, and splits it at the one door.
        target: target.split('/').map(seg).collect(),
        ..PlanEdit::default()
    }
}

fn assert_err(res: Result<model::Document, policy::defs::BodyError>, want: &str, case: &str) {
    match res {
        Err(e) => assert_eq!(e.render(), want, "{case}: refusal bytes diverged"),
        Ok(_) => panic!("{case}: expected a refusal, got a candidate"),
    }
}

#[test]
fn xxh64_rev8_matches_the_go_domain() {
    // Golden vector: the ECAS remedies embed rev8 of the Notes CONTENT span
    // (heading-excluded, subtree-inclusive) = "7a90261e" (rebuild.json).
    let notes_content =
        "\nseed note line one\nseed note line two\n\n## Slash/Title Here\n\ndeep content\n\n";
    assert_eq!(rev8(notes_content.as_bytes()), "7a90261e");
    // Published XXH64 sanity vectors (seed 0).
    assert_eq!(rev8(b""), "ef46db37");
}

#[test]
fn rebuild_refusals_match_the_u0_goldens() {
    // 1. rev-less replace_section (ECAS; the remedy names NO rev — the only
    //    value this layer holds is rev8, which no door accepts as a CAS).
    assert_err(
        run(
            DOC,
            &[PlanEdit {
                body: "no rev given".into(),
                ..edit("replace_section", "Notes")
            }],
        ),
        "ECAS: replace_section on \"Notes\" requires a fresh rev (whole-section rewrite is destructive) — read the section and pass its rev",
        "replace-section-no-rev",
    );

    // 2. rev-less all-occurrence replace (ECAS, no rev in remedy — as above).
    assert_err(
        run(
            DOC,
            &[PlanEdit {
                find: "seed".into(),
                body: "x".into(),
                all: true,
                ..edit("replace", "Notes")
            }],
        ),
        "ECAS: an all-occurrence replace requires a rev — read the section and pass its rev to confirm the current content",
        "replace-all-no-rev",
    );

    // 3. find-less replace.
    assert_err(
        run(
            DOC,
            &[PlanEdit {
                body: "x".into(),
                ..edit("replace", "Notes")
            }],
        ),
        "E_NO_MATCH: replace requires a Find anchor — set Find to the exact bytes to replace",
        "replace-no-find",
    );

    // 4. invalid frontmatter key.
    assert_err(
        run(
            DOC,
            &[PlanEdit {
                body: "v".into(),
                ..edit("set_property", "bad key")
            }],
        ),
        "E_FAIL_LOUD: invalid frontmatter key \"bad key\" — a property key is [A-Za-z0-9_-]+ (single line, no spaces or ':')",
        "prop-bad-key",
    );

    // 5. newline in a property value.
    assert_err(
        run(
            DOC,
            &[PlanEdit {
                body: "line1\nline2".into(),
                ..edit("set_property", "status")
            }],
        ),
        "E_FAIL_LOUD: property value for \"status\" contains a newline — frontmatter values are single-line in v1; put multi-line content in a body section",
        "prop-newline-value",
    );

    // 6. create_section on an existing section.
    assert_err(
        run(
            DOC,
            &[PlanEdit {
                body: "shadow attempt".into(),
                ..edit("create_section", "Notes")
            }],
        ),
        "E_EXISTS: section \"Notes\" already exists — target the existing section with append/replace_section, or create under a distinct heading",
        "create-existing",
    );

    // 7. block-target replace PASSES the rebuild (the lib resolves blocks;
    //    the golden's refusal is the HOST's, post-CheckWrite).
    let ok = run(
        DOC,
        &[PlanEdit {
            find: "second item".into(),
            body: "second thing".into(),
            ..edit("replace", "^task1")
        }],
    );
    let cand = ok.expect("block-target replace rebuilds");
    assert!(
        cand.raw.contains("- [ ] second thing ^task1"),
        "block replace lands in the candidate: {}",
        cand.raw
    );

    // 8. anchor miss inside a block (E_NO_MATCH names the HOME section title).
    assert_err(
        run(
            DOC,
            &[PlanEdit {
                find: "absent".into(),
                body: "x".into(),
                ..edit("replace", "^task1")
            }],
        ),
        // The section NAME for a block target is its innermost CONTAINING
        // section title (Go resolveEditTarget → innermostSection), not the
        // block's own id/title.
        "E_NO_MATCH: anchor \"absent\" not found in section \"Todo\" — read the section (md read) and copy the exact bytes; Find is byte-exact (no normalization)",
        "block-anchor-miss",
    );
}

/// S4b/D11: a `set_property` value carrying a newline forges frontmatter keys
/// (`{key}: {value}` is composed literally, and a single-line YAML scalar —
/// quoted or not — cannot carry a raw newline) — so the shared predicate
/// REFUSES it, and the rebuild yields NO candidate for any injection spelling.
#[test]
fn set_property_refuses_multiline_values_that_forge_frontmatter_keys() {
    // The forged-key spelling: no ": " anywhere, so the conditional quote would
    // never have fired — the value would have landed raw as a second FM line.
    let res = run(
        DOC,
        &[PlanEdit {
            body: "seeded\ninjected:pwned".into(),
            ..edit("set_property", "status")
        }],
    );
    assert_err(
        res,
        "E_FAIL_LOUD: property value for \"status\" contains a newline — frontmatter values are single-line in v1; put multi-line content in a body section",
        "prop-injection-forged-key",
    );

    // The quoted spelling: a mid-value ": " DOES trigger the quote (the
    // § A.6.3 double-quoted emit — the former single-quote emit is retired),
    // and the quote leaks across the newline — refused for the same reason.
    assert_err(
        run(
            DOC,
            &[PlanEdit {
                body: "seeded\nowner: mallory".into(),
                ..edit("set_property", "status")
            }],
        ),
        "E_FAIL_LOUD: property value for \"status\" contains a newline — frontmatter values are single-line in v1; put multi-line content in a body section",
        "prop-injection-quoted-scalar",
    );

    // A bare \r counts (CRLF frontmatter is line-split on \r too).
    assert_err(
        run(
            DOC,
            &[PlanEdit {
                body: "seeded\rinjected:pwned".into(),
                ..edit("set_property", "status")
            }],
        ),
        "E_FAIL_LOUD: property value for \"status\" contains a newline — frontmatter values are single-line in v1; put multi-line content in a body section",
        "prop-injection-carriage-return",
    );

    // The predicate itself is the owner: no caller can mint a quoted value for
    // a multi-line input, and single-line values still quote unchanged.
    assert_eq!(
        policy::defs::yaml_safe_value("seeded\ninjected:pwned"),
        Err(policy::defs::MultiLineValue)
    );
    // The quoted SPELLING is double, not single (§ A.6.3, amended 2026-08-07):
    // the encoder emits the fleet-canonical form so two writers on one tree do
    // not churn each other's bytes. Same trigger, same value, one style.
    assert_eq!(
        policy::defs::yaml_safe_value("review: pending"),
        Ok("\"review: pending\"".to_string())
    );
}

/// Finding 5 / R5: the KEY half of the composed `{key}: {value}` line gets the
/// same shape S4b gave the value — a fallible owner. `SafeKey`'s field is
/// private, so the only way to obtain the spelling the composition accepts is
/// to discharge `yaml_safe_key`'s `Result`; the refusal cannot be forgotten at
/// a call site the way a boolean helper can (R5).
#[test]
fn set_property_keys_pass_a_fallible_owner_that_refuses_the_forged_spelling() {
    // The reviewer's repro key: a space, a ": " and a newline, each of which
    // composes a frontmatter line the caller never named.
    assert_eq!(
        policy::defs::yaml_safe_key("ti tle: forged\nevil"),
        Err(policy::defs::InvalidPropertyKey)
    );
    assert_eq!(
        policy::defs::yaml_safe_key(""),
        Err(policy::defs::InvalidPropertyKey),
        "an empty key composes a bare ': value' line"
    );
    // Refusal, never sanitization: the legal charset passes VERBATIM.
    assert_eq!(
        policy::defs::yaml_safe_key("review-state_2").map(|k| k.as_str().to_string()),
        Ok("review-state_2".to_string())
    );

    // And the refusal reaches the rebuild door as the golden's own bytes.
    assert_err(
        run(
            DOC,
            &[PlanEdit {
                body: "x".into(),
                ..edit("set_property", "ti tle: forged\nevil")
            }],
        ),
        "E_FAIL_LOUD: invalid frontmatter key \"ti tle: forged\\nevil\" — a property key is [A-Za-z0-9_-]+ (single line, no spaces or ':')",
        "prop-forged-key",
    );
}

#[test]
#[allow(clippy::too_many_lines)] // one sequential Go-semantics script by design
fn rebuild_candidate_bytes_match_go_plan_semantics() {
    // Append: ensureTrailingNL + lands at the section content end (Memo is
    // last; doc ends with \n so no leading newline).
    let cand = run(
        DOC,
        &[PlanEdit {
            body: "appended line".into(),
            ..edit("append", "Memo")
        }],
    )
    .expect("append rebuilds");
    assert!(
        cand.raw.ends_with("# Memo\n\nseed memo\nappended line\n"),
        "append tail: {:?}",
        &cand.raw[cand.raw.len() - 40..]
    );

    // Append to a no-trailing-newline doc: the leading-\n discipline fires.
    let no_nl = DOC.trim_end_matches('\n');
    let cand = run(
        no_nl,
        &[PlanEdit {
            body: "tail".into(),
            ..edit("append", "Memo")
        }],
    )
    .expect("append after no-newline rebuilds");
    assert!(
        cand.raw.ends_with("seed memo\ntail\n"),
        "leading-newline discipline: {:?}",
        &cand.raw[cand.raw.len() - 30..]
    );

    // In-batch pendingNL: two appends to the same no-newline section — the
    // second sees the first's trailing \n as pending, no double newline.
    let cand = run(
        no_nl,
        &[
            PlanEdit {
                body: "one".into(),
                ..edit("append", "Memo")
            },
            PlanEdit {
                body: "two".into(),
                ..edit("append", "Memo")
            },
        ],
    )
    .expect("double append rebuilds");
    assert!(
        cand.raw.ends_with("seed memo\none\ntwo\n"),
        "pendingNL: {:?}",
        &cand.raw[cand.raw.len() - 30..]
    );

    // create_section: EOF placement, "# <Target>" + body.
    let cand = run(
        DOC,
        &[PlanEdit {
            body: "fresh body".into(),
            ..edit("create_section", "Brand")
        }],
    )
    .expect("create rebuilds");
    assert!(
        cand.raw.ends_with("seed memo\n# Brand\nfresh body\n"),
        "create at EOF: {:?}",
        &cand.raw[cand.raw.len() - 40..]
    );

    // set_property: existing key value-span splice (prefix stays).
    let cand = run(
        DOC,
        &[PlanEdit {
            body: "amended".into(),
            ..edit("set_property", "status")
        }],
    )
    .expect("set existing prop rebuilds");
    assert!(
        cand.raw.contains("\nstatus: amended\n"),
        "existing-key splice: {}",
        cand.raw
    );

    // set_property: absent key inserts before the closing --- .
    let cand = run(
        DOC,
        &[PlanEdit {
            body: "worker-d".into(),
            ..edit("set_property", "owner")
        }],
    )
    .expect("set new prop rebuilds");
    assert!(
        cand.raw.contains("status: seeded\nowner: worker-d\n---\n"),
        "absent-key insert: {}",
        cand.raw
    );

    // set_property: unsafe value gets double-quoted (yamlSafeValue, § A.6.3).
    let cand = run(
        DOC,
        &[PlanEdit {
            body: "review: pending".into(),
            ..edit("set_property", "status")
        }],
    )
    .expect("quoted prop rebuilds");
    assert!(
        cand.raw.contains("\nstatus: \"review: pending\"\n"),
        "yamlSafeValue quoting: {}",
        cand.raw
    );

    // replace with a neutralized rev: any non-empty rev passes (CheckWrite
    // neutralization happens in rebuild()).
    let cand = run(
        DOC,
        &[PlanEdit {
            find: "seed memo".into(),
            body: "amended memo".into(),
            rev: "deadbeef".into(),
            ..edit("replace", "Memo")
        }],
    )
    .expect("neutralized-rev replace rebuilds");
    assert!(
        cand.raw.contains("amended memo"),
        "neutralized rev: {}",
        cand.raw
    );

    // Unterminated fence at EOF refuses (would-corrupt delta).
    assert_err(
        run(
            DOC,
            &[PlanEdit {
                body: "```rust\nunclosed".into(),
                ..edit("append", "Memo")
            }],
        ),
        "E_WOULD_CORRUPT: the edit would leave an unterminated code fence that swallows the rest of the document — the edit was refused; the file is unchanged. Close the code fence (matching ``` or ~~~) and retry",
        "fence-open-at-eof",
    );

    // Overlapping edits refuse (assertDisjoint).
    assert_err(
        run(
            DOC,
            &[
                PlanEdit {
                    find: "seed memo".into(),
                    body: "longer replacement".into(),
                    ..edit("replace", "Memo")
                },
                PlanEdit {
                    find: "memo".into(),
                    body: "x".into(),
                    ..edit("replace", "Memo")
                },
            ],
        ),
        "E_WOULD_CORRUPT: edits overlap on the same bytes — the edits target overlapping ranges; split them or resolve to non-overlapping anchors",
        "overlap",
    );

    // Ambiguous heading resolve (duplicate-headings golden strings, dewey +
    // first-content-line candidates).
    let dup = "---\ntype: note\n---\n\n# Notes\n\nfirst notes body\n\n# Notes\n\nsecond notes body\n\n## Child\n\nchild body\n";
    assert_err(
        run(
            dup,
            &[PlanEdit {
                body: "x".into(),
                ..edit("append", "Notes")
            }],
        ),
        "E_AMBIGUOUS: \"Notes\" is ambiguous: 2 sections share this heading — qualify with the full heading path, or pass the occurrence — the read face publishes it as `n` on the ambiguous segment; candidates: 1 (line 6), 2 (line 10)",
        "ambiguous-heading",
    );
}

// ---------------------------------------------------------------------------
// the address the READ face publishes must be WRITABLE here
// ---------------------------------------------------------------------------

/// The dogfood shape that shipped broken: an H1 whose text carries an em-dash,
/// a comma and — decisively — SPACES, with a child addressed beneath it.
const G2D: &str = "---\ntype: note\n---\n\n# Dogfood — the socket MCP surface, exercised by a working agent\n\nintro\n\n## Results\n\nresult body\n\n## The `splice` finding\n\nfinding body\n";

/// The requirement, as a round trip: `read` publishes the containment chain as
/// segments; those exact segments, handed back verbatim, must address the
/// section. The chain was previously keyed through `sanitize_heading` (spaces
/// and `/` both map to `-`), so this exact case refused in production with
/// `E_NO_MATCH` — while a single-segment address survived on the never-sanitized
/// bare-title arm.
#[test]
fn published_multisegment_address_is_writable() {
    let parent = "Dogfood — the socket MCP surface, exercised by a working agent";

    for leaf in ["Results", "The `splice` finding"] {
        let got = run(
            G2D,
            &[PlanEdit {
                op: "append".to_string(),
                target: vec![seg(parent), seg(leaf)],
                body: "appended".to_string(),
                ..PlanEdit::default()
            }],
        )
        .unwrap_or_else(|e| {
            panic!(
                "two-segment address must resolve for {leaf:?}: {}",
                e.render()
            )
        });
        assert!(
            got.raw.contains("appended"),
            "the append must land for {leaf:?}"
        );
    }

    // The single-segment case that always worked keeps working.
    let got = run(
        G2D,
        &[PlanEdit {
            op: "append".to_string(),
            target: vec![seg(parent)],
            body: "one-seg".to_string(),
            ..PlanEdit::default()
        }],
    )
    .expect("single-segment H1 address still resolves");
    assert!(got.raw.contains("one-seg"));
}

/// A segment containing the JOINER is the case escaping exists for, and the one
/// a later reader will try to "simplify" back into a split. `["Notes","Slash/Title Here"]`
/// addresses the real child; `["Notes","Slash","Title Here"]` names a third level
/// that does not exist and must FALL CLOSED rather than collapse onto it.
#[test]
fn a_segment_may_contain_the_joiner_and_stays_distinct() {
    let got = run(
        DOC,
        &[PlanEdit {
            op: "append".to_string(),
            target: segs(&["Notes", "Slash/Title Here"]),
            body: "landed".to_string(),
            ..PlanEdit::default()
        }],
    )
    .expect("a segment carrying '/' is addressable as ONE segment");
    assert!(got.raw.contains("landed"));

    assert_err(
        run(
            DOC,
            &[PlanEdit {
                op: "append".to_string(),
                target: segs(&["Notes", "Slash", "Title Here"]),
                body: "must not land".to_string(),
                ..PlanEdit::default()
            }],
        ),
        "E_NO_MATCH: no section addressed by \"Notes/Slash/Title Here\" — address the section by its `hpath` segments, exactly as the read face's toc publishes them — one array entry per heading, raw text, no joining",
        "split-of-a-slash-bearing-segment-falls-closed",
    );
}

/// The retired law, pinned as a NEGATIVE so it cannot creep back: the sanitized
/// spelling is no longer an address. `sanitize_heading` is many-to-one (C-11),
/// so accepting it is what let `A/B`, `A B` and `A-B` collide.
#[test]
fn the_sanitized_spelling_is_not_an_address() {
    assert_err(
        run(
            G2D,
            &[PlanEdit {
                op: "append".to_string(),
                target: segs(&[
                    "Dogfood-—-the-socket-MCP-surface,-exercised-by-a-working-agent",
                    "Results",
                ]),
                body: "must not land".to_string(),
                ..PlanEdit::default()
            }],
        ),
        "E_NO_MATCH: no section addressed by \"Dogfood-—-the-socket-MCP-surface,-exercised-by-a-working-agent/Results\" — address the section by its `hpath` segments, exactly as the read face's toc publishes them — one array entry per heading, raw text, no joining",
        "sanitized-spelling-refused",
    );
}

// ---------------------------------------------------------------------------
// the occurrence `n` survives the policy boundary
// ---------------------------------------------------------------------------

/// Two same-raw-text siblings under one parent, plus a same-titled section
/// under a DIFFERENT parent (which is NOT an occurrence of the first two —
/// the counting basis is same-parent, `resolve_hpath_node`'s law).
const DUP: &str = "---\ntype: note\n---\n\n# Log\n\n## Entry\n\nfirst entry\n\n## Entry\n\nsecond entry\n\n# Archive\n\n## Entry\n\narchived entry\n";

/// The address the read face publishes over duplicate headings — occurrence on
/// the ambiguous segment — must resolve HERE to the same section the committer
/// picks. This boundary previously dropped `n` (`PlanEdit.target` was
/// `Vec<String>`), making the pre-flight refuse an address the splice resolves.
#[test]
fn published_occurrence_address_resolves_through_the_policy_boundary() {
    // n:2 lands in the SECOND same-parent occurrence, not the first, and not
    // the same-titled section under the other parent.
    let got = run(
        DUP,
        &[PlanEdit {
            op: "append".to_string(),
            target: vec![seg("Log"), segn("Entry", 2)],
            body: "appended to second".to_string(),
            ..PlanEdit::default()
        }],
    )
    .expect("occurrence-qualified address resolves");
    let landed = got.raw.find("appended to second").expect("append landed");
    let second = got.raw.find("second entry").expect("fixture intact");
    let archive = got.raw.find("# Archive").expect("fixture intact");
    assert!(
        landed > second && landed < archive,
        "the append must land inside the SECOND Entry's span: {}",
        got.raw
    );

    // n:1 addresses the first occurrence.
    let got = run(
        DUP,
        &[PlanEdit {
            op: "append".to_string(),
            target: vec![seg("Log"), segn("Entry", 1)],
            body: "appended to first".to_string(),
            ..PlanEdit::default()
        }],
    )
    .expect("n:1 resolves the first occurrence");
    let landed = got.raw.find("appended to first").expect("append landed");
    let second = got.raw.find("## Entry\n\nsecond").expect("fixture intact");
    assert!(
        landed < second,
        "the append must land inside the FIRST Entry's span: {}",
        got.raw
    );

    // n:1 against a UNIQUE heading matches too — a published `n: None` means
    // occurrence 1, the same equivalence `seg_chain_matches` applies.
    run(
        DUP,
        &[PlanEdit {
            op: "append".to_string(),
            target: vec![segn("Archive", 1), seg("Entry")],
            body: "x".to_string(),
            ..PlanEdit::default()
        }],
    )
    .expect("n:1 on a unique heading still resolves");
}

/// Without `n` the duplicate refuses loud — never silently picks — and an
/// out-of-range occurrence falls closed as a miss, never clamps.
#[test]
fn occurrence_abstention_refuses_and_out_of_range_falls_closed() {
    assert_err(
        run(
            DUP,
            &[PlanEdit {
                op: "append".to_string(),
                target: segs(&["Log", "Entry"]),
                body: "must not land".to_string(),
                ..PlanEdit::default()
            }],
        ),
        "E_AMBIGUOUS: \"Log/Entry\" is ambiguous: 2 sections share this heading — qualify with the full heading path, or pass the occurrence — the read face publishes it as `n` on the ambiguous segment; candidates: 1.1 (line 8), 1.2 (line 12)",
        "n-less-duplicate-refuses",
    );

    assert_err(
        run(
            DUP,
            &[PlanEdit {
                op: "append".to_string(),
                target: vec![seg("Log"), segn("Entry", 3)],
                body: "must not land".to_string(),
                ..PlanEdit::default()
            }],
        ),
        "E_NO_MATCH: no section addressed by \"Log/Entry#3\" — address the section by its `hpath` segments, exactly as the read face's toc publishes them — one array entry per heading, raw text, no joining",
        "out-of-range-occurrence-falls-closed",
    );
}

/// **§ A.6.3b at the door the guard actually lives behind.**
///
/// The review gate mutation-proved that `!safe.is_empty()` had zero coverage.
/// Locating it precisely on THIS base: the WIRE's `set_property` lowering
/// (`wire-serve::plan::lower_property_group`) composes a whole `{key}: {value}`
/// line and never reaches this guard, so a wire-door matrix — however thorough
/// — leaves it untested. `rebuild` is the door that reaches it: the def-plane
/// rebuild and the `check_write` candidate that shares its owner.
///
/// The matrix is the same one the wire door gets: every stored colon shape x
/// the empty value x applied TWICE. Each case asserts the STORED LINE, because
/// the two failures this pins are invisible to a round-tripped value —
/// `note:""` is not a property to any external parser, and a bare `note:` is
/// the YAML null § A.6.3 forbids the engine from forging.
///
/// Mutation check: reverting the guard to `!val.is_empty()` reddens the
/// bare-colon row here, and nothing else in the workspace.
#[test]
fn set_property_empty_value_stores_one_line_shape_on_every_colon_shape() {
    // (case, seeded frontmatter line for `note`)
    let shapes: &[(&str, &str)] = &[
        ("with_value", "note: something\n"),
        ("bare_colon", "note:\n"),
        ("colon_space", "note: \n"),
        ("absent", ""),
    ];
    for (name, seeded) in shapes {
        let mut raw = format!("---\n{seeded}keep: 1\n---\n\n# Body\n\ntext\n");
        for pass in 1..=2 {
            let cand = run(
                &raw,
                &[PlanEdit {
                    body: String::new(),
                    ..edit("set_property", "note")
                }],
            )
            .unwrap_or_else(|e| panic!("{name} pass{pass}: rebuild refused: {}", e.render()));
            assert!(
                cand.raw.lines().any(|l| l == r#"note: """#),
                "{name} pass{pass}: the stored line is `note: \"\"` — a \
                 separator-less `note:\"\"` is no property to any external \
                 parser, and a bare `note:` is a forged null; got:\n{}",
                cand.raw
            );
            assert!(
                !cand.raw.contains(r#"note:"""#),
                "{name} pass{pass}: emitted a separator-less line:\n{}",
                cand.raw
            );
            assert!(
                cand.raw.contains("keep: 1"),
                "{name} pass{pass}: the sibling key survived:\n{}",
                cand.raw
            );
            let once = cand.raw.matches("note:").count();
            assert_eq!(
                once, 1,
                "{name} pass{pass}: exactly one `note` line — an update that \
                 appended would leave the stale one behind:\n{}",
                cand.raw
            );
            raw = cand.raw.clone();
        }
        // Idempotency: pass 2 changed nothing pass 1 had not already settled.
        let again = run(
            &raw,
            &[PlanEdit {
                body: String::new(),
                ..edit("set_property", "note")
            }],
        )
        .expect("third pass rebuilds");
        assert_eq!(
            again.raw, raw,
            "{name}: set_property is not byte-idempotent on the empty value"
        );
    }
}
