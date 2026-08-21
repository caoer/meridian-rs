//! `board_residue` — disclosure that makes the board collapse safe.
//!
//! `board` partitions on `verdict_color IS NOT NULL`; NULL-verdict rows vanish
//! silently without this view. Counter sits at zero (NULL unreachable under
//! R4); tests move it deliberately so disclosure is proven, not decorative.
//! Two doors to NULL leave identical SQL evidence; cause-side lives in Rust.

use std::collections::BTreeMap;

use duckdb::Connection;
use view::read_face::{LockItem, open_board};

fn doc(raw: &str) -> std::sync::Arc<model::Document> {
    std::sync::Arc::new(model::build(raw.to_string(), syntax::parse(raw)))
}

fn scalar_i64(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get::<_, i64>(0))
        .expect("scalar query")
}

/// Real R4 page pinning `subject` — board count never vacuous.
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

/// Two pinned pages bracketing a mid-fixture synthetic residue row.
fn fixture() -> model::Docs {
    let mut docs = BTreeMap::new();
    docs.insert("subject.md".to_string(), doc("# Subject\n\nbody. ^claim\n"));
    docs.insert("effect_a.md".to_string(), doc(&pinned_page("subject")));
    docs.insert("effect_z.md".to_string(), doc(&pinned_page("subject")));
    docs
}

/// RAW INSERT — proves residue query over table shape, NOT production writer
/// (NULL verdict unreachable from documents; production path gated elsewhere).
/// Mid-fixture so clause must SELECT, not survive empty view.
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

/// Production path: healthy R4 corpus → zero residue, non-empty board.
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

/// Verdict-less row appears in residue and is ABSENT from board.
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

/// Synthetic row has classified neighbours (clause selects, not empty-survives).
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

/// Public projections agree on every colourable key over real docs (upstream
/// half of co-origination). Private `pin_verdicts` assembly covered by the
/// production-path zero-residue test instead.
#[test]
fn the_two_public_projections_agree_on_every_key_over_real_documents() {
    use std::collections::BTreeSet;

    let docs = fixture();
    let index = view::read_face::corpus_index(&docs);

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

/// `is_colourable` meaning at source: fingerprint or refusal; neither ⇒ uncolourable.
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
        root_absence: None,
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
