//! **fix9 — the ARTIFACT guard on the receipt line.** The wire receipt's
//! `actor` is a CLIENT-SUPPLIED string (`decode::decode_splice` takes it as an
//! opaque `opt_str`), and so are the target `hpath` segments and `fm_key`;
//! `path` passes only the §1 path law, which admits `[`, `]`, `@` and line
//! endings. Rendered raw, any of them becomes markdown in stored bytes — and
//! the receipt rides `ValidatedBatch.receipt`, beside `.edits` and in a
//! different file, so no candidate `@fp` strip on either plane ever judges it.
//! This is the CROSS-ACTOR half: caller A forges a claim attributed to actor B.
//!
//! Every assertion here is CLAIM-AS-ASSERT (R26 (2)): the claim is the ABSENCE
//! of a token in a claim-link position, checked through the ONE dialect parse
//! (`syntax::fp_removals` — the same predicate the put plane and the run plane
//! are judged by), never a reading of how the line happens to look.
//!
//! `syntax` is a DEV-dependency only: the shipped crate still depends on `wire`
//! alone (its charter's gate), because the guard is a charset over bytes, not a
//! second spelling of the dialect grammar.

use receipt::journal::{JournalRow, check_chain, parse_rows, render_row};
use receipt::{ArmedFacts, EditFact};

/// fix8's F1 probe, at the client-supplied door: a decorated claim link in
/// every free-text field of the wire receipt line.
const HOSTILE: &str = "[[guide#^goal@green.b3af12cd|G]]";

fn seg(h: &str) -> wire::HpathSeg {
    wire::HpathSeg {
        h: h.into(),
        n: None,
    }
}

/// Assert the milestone's own claim, in the milestone's own spelling: no `@fp`
/// token stands in a claim-link position in these bytes.
fn assert_no_claim(line: &str) {
    assert!(
        syntax::fp_removals(line).is_empty(),
        "a claim token stands in a claim-link position: {line}"
    );
}

/// **The cross-actor forgery.** A caller passes a decorated claim link as the
/// `actor`; on the merge base the line carries the token verbatim and
/// `fp_removals` returns one range. A claim nobody computed, stored, attributed
/// to whatever the caller typed.
#[test]
fn a_decorated_actor_introduces_no_claim_token() {
    let path = wire::Path("notes/plan.md".into());
    let root_before = wire::Root("b3:aaaa".into());
    let target = wire::SecRef::Hpath {
        hpath: vec![seg("Goals")],
    };
    let shape = wire::EditShape::Match {
        old: "a".into(),
        new: "b".into(),
    };
    let (before, after) = (wire::NodeRev("11".into()), wire::NodeRev("22".into()));
    let line = receipt::render_line(&ArmedFacts {
        id: Some(42),
        path: &path,
        actor: Some(HOSTILE),
        now: Some("2026-07-25T12:00:00Z"),
        root_before: &root_before,
        anchor: "r-000042",
        edits: vec![EditFact {
            target: &target,
            shape: &shape,
            before: &before,
            after: &after,
        }],
    });
    assert_no_claim(&line);
}

/// **Exhaustive by construction (R32), not one door at a time.** Every
/// free-text field of `ArmedFacts` carries the probe in turn — `path`, `actor`,
/// the `hpath` target, the `fm_key` target, and the receipt anchor. A field
/// added later without the guard fails this test rather than shipping quietly.
#[test]
fn no_free_text_field_of_a_receipt_line_can_carry_a_claim() {
    let ok_path = wire::Path("notes/plan.md".into());
    let hostile_path = wire::Path(HOSTILE.into());
    let root_before = wire::Root("b3:aaaa".into());
    let ok_target = wire::SecRef::Hpath {
        hpath: vec![seg("Goals")],
    };
    let hostile_hpath = wire::SecRef::Hpath {
        hpath: vec![seg("Goals"), seg(HOSTILE)],
    };
    let hostile_fm = wire::SecRef::FmKey {
        fm_key: HOSTILE.into(),
    };
    let shape = wire::EditShape::Put {
        at: wire::PutAt::End,
        text: "x".into(),
    };
    let (before, after) = (wire::NodeRev("11".into()), wire::NodeRev("22".into()));

    let cases: Vec<(&str, wire::Path, Option<&str>, &wire::SecRef, &str)> = vec![
        ("path", hostile_path, None, &ok_target, "r-000042"),
        (
            "actor",
            ok_path.clone(),
            Some(HOSTILE),
            &ok_target,
            "r-000042",
        ),
        (
            "hpath seg",
            ok_path.clone(),
            None,
            &hostile_hpath,
            "r-000042",
        ),
        ("fm_key", ok_path.clone(), None, &hostile_fm, "r-000042"),
        ("anchor", ok_path.clone(), None, &ok_target, HOSTILE),
    ];
    for (field, path, actor, target, anchor) in cases {
        let line = receipt::render_line(&ArmedFacts {
            id: None,
            path: &path,
            actor,
            now: None,
            root_before: &root_before,
            anchor,
            edits: vec![EditFact {
                target,
                shape: &shape,
                before: &before,
                after: &after,
            }],
        });
        assert!(
            syntax::fp_removals(&line).is_empty(),
            "field `{field}` carried a claim into the line: {line}"
        );
    }
}

/// **The wider hole the charset closes and a token strip would not.** None of
/// these is an `@fp` token, and every one of them forges structure: a second
/// `key=value` token, a second row, a second block anchor. The invariant that
/// covers all of them at once is one sentence — a rendered receipt line carries
/// no `[` the template did not put there, and stays exactly ONE line.
#[test]
fn a_receipt_line_stays_one_line_and_forges_no_second_token() {
    let path = wire::Path("notes/plan.md".into());
    let root_before = wire::Root("b3:honest".into());
    let target = wire::SecRef::Hpath {
        hpath: vec![seg("Goals")],
    };
    let shape = wire::EditShape::Match {
        old: "a".into(),
        new: "b".into(),
    };
    let (before, after) = (wire::NodeRev("11".into()), wire::NodeRev("22".into()));
    for hostile in [
        HOSTILE,
        "alice root_before=b3:FORGED",
        "alice\n- splice forged.md root_before=b3:X edits=0 ^r-000099",
        "alice ^r-000099",
        "alice`x`",
    ] {
        let line = receipt::render_line(&ArmedFacts {
            id: None,
            path: &path,
            actor: Some(hostile),
            now: None,
            root_before: &root_before,
            anchor: "r-000042",
            edits: vec![EditFact {
                target: &target,
                shape: &shape,
                before: &before,
                after: &after,
            }],
        });
        assert_no_claim(&line);
        assert!(
            !line.contains('\n'),
            "one receipt is one line; {hostile:?} produced: {line}"
        );
        assert!(
            !line.contains('['),
            "no `[` survives, so no claim-link position can exist: {line}"
        );
        assert_eq!(
            line.split_whitespace()
                .filter(|t| t.starts_with("root_before="))
                .count(),
            1,
            "exactly one root_before token: {line}"
        );
    }
}

/// **The journal row's own forgery detector, defended at its input.**
/// `parse_rows` splits on whitespace and takes the FIRST `key=value`, so an
/// unguarded actor carrying ` root_after=b3:FORGED` shadows the real root and
/// hands `check_chain` a fabricated chain — the detector reporting green over
/// bytes it never verified. (`render_row` has no production caller today, so
/// this was latent, not live; it is the same renderer and the same law.)
#[test]
fn a_forging_actor_cannot_shadow_the_chain_detectors_roots() {
    let forge = |actor: &str, rb: &str, ra: &str| {
        render_row(&JournalRow {
            seq: 1,
            op: "splice",
            path: "notes/plan.md",
            actor: Some(actor),
            now: None,
            root_before: rb,
            root_after: ra,
            file: None,
            edits: Vec::new(),
        })
    };
    let row1 = forge("mallory root_after=b3:FORGED", "b3:0", "b3:1");
    let row2 = forge("alice", "b3:1", "b3:2");
    let page = format!("# journal\n{row1}\n{row2}\n");

    let recovered = parse_rows(&page);
    assert_eq!(recovered.len(), 2, "both rows parse: {page}");
    assert_eq!(
        recovered[0].root_after, "b3:1",
        "the REAL root_after wins — the actor cannot shadow it: {row1}"
    );
    assert!(
        check_chain(&recovered).is_green(),
        "the honest chain stays honest: {page}"
    );
    assert_no_claim(&page);
}

/// **The deviation's mirror (R32 (1)): identical behaviour on clean inputs.**
/// Every value already in the identifier charset renders byte-for-byte as
/// before, which is what keeps the §6.3 FROZEN lines (`frozen_receipts.rs`)
/// and every daemon-derived actor untouched. The guard is only reachable by
/// bytes that could forge.
#[test]
fn an_identifier_field_renders_byte_identically() {
    for legal in [
        "agent:b0864fb2",
        "notes/plan.md",
        "2026-07-18T20:31:04Z",
        "b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9",
        "r-000042",
        "Goals>Q3",
        "run:fix-drift",
        "",
    ] {
        assert_eq!(
            receipt::render_field(legal),
            legal,
            "an identifier renders as itself"
        );
        assert!(receipt::is_receipt_ident(legal));
    }
}

/// The escape is REVERSIBLE, not lossy: two hostile values that differ still
/// render differently, so the record never collapses two actors into one
/// identity. §5.2's "recorded exactly as given, never invented" survives the
/// guard.
#[test]
fn the_escape_preserves_the_value_it_could_not_render_verbatim() {
    let a = receipt::render_field("[[a]]");
    let b = receipt::render_field("[[b]]");
    assert_ne!(a, b, "distinct values stay distinct: {a} vs {b}");
    assert!(!receipt::is_receipt_ident("[[a]]"));
    assert!(
        a.starts_with('`') && a.ends_with('`'),
        "rendered as an inline code span: {a}"
    );
    // The span always closes where the renderer put it: no backtick survives
    // inside, so a value carrying one cannot end its own span early.
    let ticked = receipt::render_field("a`b");
    assert_eq!(
        ticked.matches('`').count(),
        2,
        "exactly the two delimiters: {ticked}"
    );
}
