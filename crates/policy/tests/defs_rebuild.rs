//! U8c rebuild-fixture gate: `policy::defs::rebuild` must reproduce the Go
//! `ApplyForConformance` refusal strings byte-exactly. Expected values are
//! copied VERBATIM from the U0 rebuild goldens (ccc-statusd
//! `internal/mcpserver/testdata/parity/goldens/rebuild.json` @ 1f0b1ef) with
//! the Go face's `put: def-conformance check could not run: ` prefix stripped
//! (that wrap is putsidecar's; the verdict half renders `CODE: msg — remedy`).
//! Candidate-content cases assert the byte mechanics (newline discipline, EOF
//! create, property injection) against Go `planEditResolved` semantics.

use policy::defs::{PlanEdit, rebuild, rev8};

// The rebuild corpus doc IS the basic doc (same bytes, per the golden).
const DOC: &str = "---\ntype: note\nstatus: seeded\n---\n\n# Todo\n\n- [ ] first item\n- [ ] second item ^task1\n\n# Notes\n\nseed note line one\nseed note line two\n\n## Slash/Title Here\n\ndeep content\n\n# Memo\n\nseed memo\n";

fn doc(raw: &str) -> model::Document {
    model::build(raw.to_string(), syntax::parse(raw))
}

fn run(prev_raw: &str, edits: &[PlanEdit]) -> Result<model::Document, policy::defs::BodyError> {
    let prev = doc(prev_raw);
    rebuild(&prev, edits, &|raw| doc(raw))
}

fn edit(op: &str, target: &str) -> PlanEdit {
    PlanEdit {
        op: op.to_string(),
        target: target.to_string(),
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
    // 1. rev-less replace_section (ECAS, remedy embeds the Go-domain rev).
    assert_err(
        run(
            DOC,
            &[PlanEdit {
                body: "no rev given".into(),
                ..edit("replace_section", "Notes")
            }],
        ),
        "ECAS: replace_section on \"Notes\" requires a fresh rev (whole-section rewrite is destructive) — read the section (md read) and pass its rev; current rev is 7a90261e",
        "replace-section-no-rev",
    );

    // 2. rev-less all-occurrence replace (ECAS, rev in remedy).
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
        "ECAS: an all-occurrence replace requires a rev — read the section (md read) and pass its rev (7a90261e) to confirm the current content",
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

    // set_property: unsafe value gets single-quoted (yamlSafeValue).
    let cand = run(
        DOC,
        &[PlanEdit {
            body: "review: pending".into(),
            ..edit("set_property", "status")
        }],
    )
    .expect("quoted prop rebuilds");
    assert!(
        cand.raw.contains("\nstatus: 'review: pending'\n"),
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
        "E_AMBIGUOUS: \"Notes\" is ambiguous: 2 sections share this heading — qualify with the full heading path or an ordinal; candidates: 1 (line 6), 2 (line 10)",
        "ambiguous-heading",
    );
}
