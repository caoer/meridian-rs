//! U9c acceptance gate — `board_residue`, the disclosure that makes the board
//! collapse safe.
//!
//! `board`'s two arms partition `input_lock` on `verdict_color IS NOT NULL`. A
//! row with no verdict therefore matches NO arm and would leave the surface
//! SILENTLY — absence reading as nothing-to-report, which is the fail-open the
//! collapse would otherwise introduce. `board_residue` is that row's only trace.
//!
//! # Why the counter sits at zero, and why that is not a decoration
//! A NULL verdict is unreachable at this revision. That is the POINT of the
//! gate: a counter that has never moved and CANNOT move proves nothing, so
//! these tests move it deliberately and assert the disclosure notices. What is
//! being gated is not "does residue exist today" — it does not — but "would we
//! be told if it ever did".
//!
//! # The two doors are ONE row shape here, deliberately, and that is measured
//! A NULL verdict has two causes: a row failing `LockItem::is_colourable`, and a
//! COLOURABLE row whose verdict lookup missed. **They leave identical evidence
//! in `input_lock`**, which carries neither the fingerprint slot nor the refusal
//! slot as its own column. So at THIS layer they are not two arms — they are one
//! row shape twice, and asserting them as two would be exactly the redundancy
//! that is really one arm. The cause-side arm lives in Rust, where the evidence
//! is: `predicate_is_one_definition_shared_by_both_projections`.

use std::collections::BTreeMap;

use duckdb::Connection;
use model::Document;
use view::read_face::{LockItem, open_board};

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn doc(raw: &str) -> Document {
    model::build(raw.to_string(), syntax::parse(raw))
}

fn scalar_i64(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get::<_, i64>(0))
        .expect("scalar query")
}

/// A real R4 page pinning `subject.md` — so the fixture's classified rows are
/// genuine and a `count(*)` over `board` is never vacuous.
fn pinned_page(object: &str) -> String {
    let mut l = lock::Lock::new();
    l.upsert_pin(lock::PinEntry::new(
        object,
        "9ae3f1deadbeef",
        lock::Selector::Path(Vec::new()),
        &format!("fp1.span2.b3.{}", "0".repeat(64)),
    ));
    format!("# Effect\n\ndraws from {object}\n\n{}\n", lock::render(&l))
}

/// Two REAL pinned pages plus their subject. `effect_a` and `effect_z` bracket
/// the synthetic row inserted between them.
fn fixture() -> BTreeMap<String, Document> {
    let mut docs = BTreeMap::new();
    docs.insert("subject.md".to_string(), doc("# Subject\n\nbody. ^claim\n"));
    docs.insert("effect_a.md".to_string(), doc(&pinned_page("subject")));
    docs.insert("effect_z.md".to_string(), doc(&pinned_page("subject")));
    docs
}

/// **W2 / CONSTRAINT 2 — READ THIS BEFORE TRUSTING THE TESTS BELOW THAT USE IT.**
///
/// This is a RAW INSERT, so the tests built on it prove the residue query works
/// over a table SHAPE. They do **not** prove the production writer can deliver a
/// NULL verdict into `input_lock`.
///
/// That second claim cannot be proved by a fixture at this revision, and the
/// reason is the finding itself rather than a gap in the test: both doors to a
/// NULL verdict are UNREACHABLE from documents. A route-A row is refused by the
/// R4 parser before it can be projected, and a route-B row requires the two
/// projections to disagree about a key that is minted once and never
/// reassigned. A document that produced either would be a defect report, not a
/// fixture. **Unreachability is what the gate exists to preserve, so it cannot
/// also be the thing the fixture demonstrates.**
///
/// The production-path claim is carried instead by
/// `the_two_projections_agree_on_every_key_over_real_documents`, which runs REAL
/// documents through the REAL projections and fails if the guarantee behind
/// route B ever breaks. That is the honest split: this helper gates the QUERY,
/// that test gates the INVARIANT.
///
/// Insert ONE synthetic verdict-less row, **MID-FIXTURE**: `effect_m.md` sorts
/// between `effect_a.md` and `effect_z.md`, so the row has classified rows on
/// BOTH sides of it.
///
/// This is all-hands 24 applied to a fixture instead of a parser. A residue
/// clause tested on a view where the synthetic row is the ONLY row cannot
/// distinguish "the clause caught it" from "the view was empty anyway" —
/// absence and truncation emitting the same bytes. Bracketing it forces the
/// clause to SELECT rather than merely to not-exclude.
fn insert_synthetic_residue_row(conn: &Connection) {
    conn.execute(
        "INSERT INTO input_lock \
         (src_path, seq, declared_ref, to_path, to_sel, pinned_rev, rev_class, \
          hash_algo, src_doc_rev, verdict_color, verdict_reason, verdict_detail) \
         VALUES ('effect_m.md', 0, 'subject.md', 'subject.md', '', NULL, NULL, \
                 NULL, 'deadbeefdeadbeef', NULL, NULL, NULL)",
        [],
    )
    .expect("the read face is locked against EXTERNAL access, not against its own inserts");
}

// ---------------------------------------------------------------------------
// gate — the counter is at zero, and the zero is real
// ---------------------------------------------------------------------------

/// **THE PRODUCTION-PATH DETECTOR (W2 / constraint 2).** Real documents through
/// the real writer: a healthy R4 corpus discloses ZERO residue while the board
/// is NOT empty. Both halves matter — a zero over an empty board is the vacuous
/// pass this whole gate exists to refuse.
///
/// This is the arm that actually sees route B. It reads the count AFTER
/// `project_input_locks` has run, so every key site is in scope including the
/// private assembly in `pin_verdicts`. Verified by mutating that private site:
/// two real rows immediately became residue and this test went red, 0 → 2.
///
/// **The mutation direction is the feared one (constraint 3).** The failure to
/// fear is UNDER-report — silence where there should be a count. Disabling the
/// residue clause drives the count DOWN, 1 → 0, and the disclosure test reddens
/// on exactly that; breaking co-origination drives it UP here. Both directions
/// are pinned, and neither reddens for the other's reason.
#[test]
fn residue_is_zero_over_a_healthy_corpus_and_the_board_is_not_empty() {
    let conn = open_board(&fixture()).expect("open board");

    assert_eq!(
        scalar_i64(&conn, "SELECT count(*) FROM board_residue"),
        0,
        "every R4 row carries a verdict, so nothing falls out of the board"
    );
    assert!(
        scalar_i64(&conn, "SELECT count(*) FROM board") > 0,
        "the zero above must be measured over a board that HAS rows — otherwise \
         it is a statement about an empty fixture, not about residue"
    );
}

// ---------------------------------------------------------------------------
// gate — the counter CAN move, and the row it counts is the row that vanished
// ---------------------------------------------------------------------------

/// Move the counter deliberately. A verdict-less row inserted between two
/// classified rows must (a) appear in `board_residue`, and (b) be ABSENT from
/// `board` — the vanishing this disclosure exists to make visible.
///
/// The second assertion is the load-bearing one. Residue that also rendered in
/// `board` would need no disclosure at all.
#[test]
fn a_verdict_less_row_vanishes_from_board_and_is_disclosed_by_residue() {
    let conn = open_board(&fixture()).expect("open board");
    let board_before = scalar_i64(&conn, "SELECT count(*) FROM board");

    insert_synthetic_residue_row(&conn);

    assert_eq!(
        scalar_i64(&conn, "SELECT count(*) FROM board_residue"),
        1,
        "the verdict-less row is disclosed"
    );
    assert_eq!(
        scalar_i64(
            &conn,
            "SELECT count(*) FROM board_residue WHERE src_path = 'effect_m.md'"
        ),
        1,
        "and the disclosure names the row that fell out, not merely a total"
    );
    assert_eq!(
        scalar_i64(&conn, "SELECT count(*) FROM board"),
        board_before,
        "the row matched NO arm of `board` — it vanished, which is precisely \
         what `board_residue` exists to report"
    );
    assert_eq!(
        scalar_i64(
            &conn,
            "SELECT count(*) FROM board WHERE src_path = 'effect_m.md'"
        ),
        0,
        "named, not just counted: the vanished row is this one"
    );
}

/// The bracketing is real: the synthetic row has a classified row on each side,
/// so the residue clause is SELECTING rather than surviving an empty view.
#[test]
fn the_synthetic_row_sits_between_classified_rows() {
    let conn = open_board(&fixture()).expect("open board");
    insert_synthetic_residue_row(&conn);

    assert_eq!(
        scalar_i64(
            &conn,
            "SELECT count(*) FROM input_lock WHERE src_path < 'effect_m.md' \
             AND verdict_color IS NOT NULL"
        ),
        1,
        "a classified row sorts BEFORE the synthetic one"
    );
    assert_eq!(
        scalar_i64(
            &conn,
            "SELECT count(*) FROM input_lock WHERE src_path > 'effect_m.md' \
             AND verdict_color IS NOT NULL"
        ),
        1,
        "and another sorts AFTER it — the residue row is mid-fixture, never last"
    );
}

// ---------------------------------------------------------------------------
// gate — the cause-side arm, where the evidence actually lives
// ---------------------------------------------------------------------------

/// The walk's colour roll-up SKIPS a row it cannot colour; the board projection
/// writes a NULL verdict for one. Those two decisions must agree, because
/// `board_residue` counts exactly the rows the walk skipped.
///
/// **They agree because there is ONE definition**, `LockItem::is_colourable`,
/// not two copies. This test pins the predicate's meaning at the source rather
/// than re-asserting it per call site: a row with neither a fingerprint nor a
/// refusal is uncolourable, and anything carrying either is colourable.
///
/// The predicate previously existed as two spellings — exact negations of each
/// other, which is why the duplication survived: NEGATED COPIES DO NOT LOOK LIKE
/// COPIES. Nothing tested that they agreed, and had they drifted, a well-formed
/// row would have taken a verdict on one plane and a NULL on the other with
/// nothing failing.
/// The two PUBLIC projections agree on every key over real documents — the
/// upstream half of the co-origination guarantee that closes route B.
///
/// **WHAT THIS DOES NOT COVER, stated because I measured it rather than assumed
/// it.** Production does not compare these two sets directly: it builds the
/// lookup key a third time, inside `pin_verdicts`, from the same `PinColor`
/// fields. That final assembly is private and this test cannot reach it — so a
/// divergence introduced THERE passes this test untouched. I verified that by
/// mutating exactly that site: route B opened, two real rows became residue, and
/// **this assertion still passed** while the residue gates went red.
///
/// So this is a real arm with a named boundary, not the whole detector. The
/// production-path detector for route B is
/// `residue_is_zero_over_a_healthy_corpus_and_the_board_is_not_empty`, which
/// runs documents through the REAL writer and therefore sees every key site
/// including the private one. Keeping both is deliberate: this one localises a
/// break upstream, that one catches it wherever it happens.
#[test]
fn the_two_public_projections_agree_on_every_key_over_real_documents() {
    use std::collections::BTreeSet;

    let docs = fixture();
    let index = view::read_face::corpus_index(&docs);

    // The BOARD side: every colourable row the projection will try to look up.
    let mut board_keys = BTreeSet::new();
    for (path, doc) in &docs {
        for item in view::read_face::page_lock_items_in_corpus(path, doc, &index, &docs) {
            if item.is_colourable() {
                board_keys.insert((
                    path.clone(),
                    item.declared_ref.clone(),
                    item.fingerprint.clone(),
                ));
            }
        }
    }

    // The VERDICT side: every key the colour plane actually published.
    let verdict_keys: BTreeSet<_> = view::walk::lock_pin_colors(&docs)
        .into_iter()
        .map(|p| (p.src_path, p.declared_ref, p.fingerprint))
        .collect();

    assert!(
        !board_keys.is_empty(),
        "the fixture must produce colourable rows, or this test passes vacuously \
         by comparing two empty sets"
    );
    assert_eq!(
        board_keys, verdict_keys,
        "every colourable row must find its verdict. A key present on the board \
         side and absent here is route B: the row lands in `input_lock` with a \
         NULL verdict, vanishes from `board`, and NOTHING FAILS"
    );
}

#[test]
fn predicate_is_one_definition_shared_by_both_projections() {
    let bare = LockItem {
        declared_ref: String::new(),
        declared_addr: None,
        to_path: String::new(),
        to_sel: String::new(),
        pinned_rev: None,
        rev_class: None,
        hash_algo: None,
        fingerprint: None,
        selector: None,
        object: String::new(),
        lock_refusal: None,
        to_root: None,
        root_refusal: None,
    };
    assert!(
        !bare.is_colourable(),
        "neither fingerprint nor refusal: no compare on either plane can answer it"
    );

    let pinned = LockItem {
        fingerprint: Some(format!("fp1.span2.b3.{}", "0".repeat(64))),
        ..bare.clone()
    };
    assert!(
        pinned.is_colourable(),
        "a fingerprint is evidence to verify — the colour plane answers it"
    );

    let refused = LockItem {
        lock_refusal: Some("more than one meridian-lock block".to_string()),
        ..bare.clone()
    };
    assert!(
        refused.is_colourable(),
        "a refusal is a STATED failure to read — grey lock-refused, never silence"
    );
}
