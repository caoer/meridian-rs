//! U5.1 acceptance gates — the board view (colors) + the co-edit trace, wired
//! over the U2.9 locked read face (plan U5.1; d2 §5.3). Four named gates:
//!
//! - `gate_doctored_verdict_board_and_trace` — a doctored verdict renders RED via
//!   the default-face `board` SQL, AND the co-edit trace is visible
//!   convention-free in `co_edit_trace` (the journal's mechanical write-facts).
//!   Falsified: the un-doctored fixture is GREEN (red is not spurious), and a
//!   single-write journal shows no co-edit (the second write is load-bearing).
//! - `gate_ungated_close_grey` — an ungated close (a declared-unpinned edge that
//!   never froze a verdict rev) renders GREY, never green, never silently clean.
//!   Falsified: a red-only board (the pre-U5.1 surface) DROPS the edge entirely
//!   (silently clean) — the `grey` branch is what surfaces it.
//! - `gate_board_one_color_per_edge` — the board renders EXACTLY one color per
//!   `^inputs` edge (green | red | grey), the composed-legend invariant.
//! - `gate_locked_face_holds_with_board` — the U2.9 blocked-`ATTACH` assertion
//!   still refuses with the `board` + `co_edit_trace` views loaded (no face
//!   widening).

use std::collections::BTreeMap;

use duckdb::Connection;
use model::Document;
use view::facts::JournalRowInput;
use view::read_face::open_board;

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn doc(raw: &str) -> Document {
    model::build(raw.to_string(), syntax::parse(raw))
}

fn scalar_i64(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).expect("scalar query")
}

/// The live `node_rev` of one addressable selector — the rev a pin would record.
fn live_node_rev(path: &str, raw: &str, selector: &str) -> String {
    let mut docs = BTreeMap::new();
    docs.insert(path.to_string(), doc(raw));
    let conn = open_board(&docs, &[]).expect("open board");
    conn.query_row(
        "SELECT node_rev FROM node WHERE path = ? AND selector = ?",
        [path, selector],
        |r| r.get::<_, String>(0),
    )
    .unwrap_or_else(|e| panic!("no node row for {path}#{selector}: {e}"))
}

/// A verdict page whose `^inputs` lock PINS `subject.md#^claim` at `pinned_rev`
/// (the verdict frozen at close).
fn review_page(pinned_rev: &str) -> String {
    format!(
        "## Verdict\n\napproved\n\n```yaml ^inputs\nhash-algo: node-rev\nitems:\n  - {{ref: 'subject.md', to: 'subject.md#^claim', rev: '{pinned_rev}', rev_class: 'content'}}\n```\n"
    )
}

/// An UNGATED close page: it declares `subject.md#^claim` as an input but never
/// pins a rev (a bare flip that froze no verdict — declared-unpinned).
fn ungated_review_page() -> String {
    "## Verdict\n\napproved (bare flip)\n\n```yaml ^inputs\nitems:\n  - {ref: 'subject.md', to: 'subject.md#^claim', claim: 'closed with no frozen verdict rev'}\n```\n"
        .to_string()
}

/// One journal write row (source-3 mechanical fact) at position `line_no`.
fn journal_row(anchor: &str, op: &str, path: &str, actor: &str, line_no: u64) -> JournalRowInput {
    JournalRowInput {
        anchor: anchor.to_string(),
        op: op.to_string(),
        path: path.to_string(),
        actor: Some(actor.to_string()),
        now: None,
        root_before: format!("b3:{}", line_no - 1),
        root_after: format!("b3:{line_no}"),
        edits: 1,
        line_no,
    }
}

// ---------------------------------------------------------------------------
// Gate 1 — doctored verdict: red via board SQL + co-edit trace convention-free
// ---------------------------------------------------------------------------

/// A verdict pins `subject.md#^claim` at close (rev R1 — frozen). The subject is
/// then edited to R2. TWO independent facts land, both in the DEFAULT face:
///   (a) `board` renders the review→subject edge RED (a reading of the frozen pin
///       vs live — "verdicts freeze at close");
///   (b) `co_edit_trace` shows the subject written at the close and EDITED AGAIN
///       afterwards — the co-edit, visible with NO armed convention.
#[test]
fn gate_doctored_verdict_board_and_trace() {
    let subject_v1 = "# Subject\n\nThe reviewed claim body. ^claim\n";
    let subject_v2 = "# Subject\n\nThe body was quietly edited after close. ^claim\n";
    let r1 = live_node_rev("subject.md", subject_v1, "^claim");
    let review = review_page(&r1);

    // The mechanical write-facts (source 3): subject created at R1, the verdict
    // closed (pins R1), then subject doctored to R2 — a later write on subject.
    let journal = [
        journal_row("r-000001", "create", "subject.md", "worker", 1),
        journal_row("r-000002", "create", "review.md", "reviewer", 2),
        journal_row("r-000003", "splice", "subject.md", "doctorer", 3),
    ];

    // --- Freshly closed (un-doctored): the FALSIFICATION control. -------------
    // With the subject still at R1, the board renders the edge GREEN — so the red
    // detection is not spurious; only an actual post-close drift makes it red.
    let mut fresh = BTreeMap::new();
    fresh.insert("subject.md".to_string(), doc(subject_v1));
    fresh.insert("review.md".to_string(), doc(&review));
    let conn_fresh = open_board(&fresh, &journal[..2]).expect("open board fresh");
    assert_eq!(
        scalar_i64(
            &conn_fresh,
            "SELECT count(*) FROM board WHERE to_path='subject.md' AND color='green'",
        ),
        1,
        "FALSIFY: a freshly closed verdict is GREEN — red is not always-on",
    );
    assert_eq!(
        scalar_i64(
            &conn_fresh,
            "SELECT count(*) FROM board WHERE to_path='subject.md' AND color='red'",
        ),
        0,
        "FALSIFY: no drift yet ⇒ no red",
    );

    // --- Doctored: the subject is edited after close (live R2 ≠ frozen R1). ----
    let mut doctored = BTreeMap::new();
    doctored.insert("subject.md".to_string(), doc(subject_v2));
    doctored.insert("review.md".to_string(), doc(&review));
    let conn = open_board(&doctored, &journal).expect("open board doctored");

    // (a) board SQL renders RED in the DEFAULT face — no optional pack.
    assert_eq!(
        scalar_i64(
            &conn,
            "SELECT count(*) FROM board WHERE src_path='review.md' AND to_path='subject.md' AND to_sel='^claim' AND color='red' AND reason='content-drifted'",
        ),
        1,
        "the doctored verdict renders red via default-face board SQL",
    );
    // The red row cites the frozen pin (R1) and the drifted live rev (R2).
    let (color, pinned): (String, String) = conn
        .query_row(
            "SELECT color, pinned_rev FROM board WHERE src_path='review.md'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(color, "red");
    assert_eq!(
        pinned, r1,
        "the frozen verdict rev (R1) is cited — freeze-at-close"
    );

    // (b) co-edit trace visible CONVENTION-FREE: the subject was written twice —
    // co-edited at the close, then edited again — surfaced by the journal alone.
    let subject_edits = scalar_i64(
        &conn,
        "SELECT edits_on_path FROM co_edit_trace WHERE path='subject.md' LIMIT 1",
    );
    assert_eq!(
        subject_edits, 2,
        "the subject was co-edited then edited again"
    );
    // The doctoring edit (edit_ord 2) POST-DATES the close write on review.md.
    let close_line = scalar_i64(
        &conn,
        "SELECT line_no FROM co_edit_trace WHERE path='review.md'",
    );
    let doctor_line = scalar_i64(
        &conn,
        "SELECT line_no FROM co_edit_trace WHERE path='subject.md' AND edit_ord=2",
    );
    assert!(
        doctor_line > close_line,
        "the doctoring edit ({doctor_line}) post-dates the close ({close_line}) — 'edited again at R2'",
    );

    // --- Falsify the trace: a single-write journal shows NO co-edit. ----------
    let single = [journal_row("r-000001", "create", "subject.md", "worker", 1)];
    let conn_single = open_board(&doctored, &single).expect("open board single-write");
    assert_eq!(
        scalar_i64(
            &conn_single,
            "SELECT edits_on_path FROM co_edit_trace WHERE path='subject.md' LIMIT 1",
        ),
        1,
        "FALSIFY: one write ⇒ no co-edit — the second write is what the trace surfaces",
    );
}

// ---------------------------------------------------------------------------
// Gate 2 — ungated close renders grey (never green, never silently clean)
// ---------------------------------------------------------------------------

/// A close that never froze a verdict rev (declared-unpinned) renders GREY on the
/// board — the ledger cannot verify it, so it is never green and never silently
/// dropped. Falsified against the red-only surface, which drops it entirely.
#[test]
fn gate_ungated_close_grey() {
    let subject = "# Subject\n\nThe reviewed claim body. ^claim\n";
    let mut docs = BTreeMap::new();
    docs.insert("subject.md".to_string(), doc(subject));
    docs.insert("ungated.md".to_string(), doc(&ungated_review_page()));
    let conn = open_board(&docs, &[]).expect("open board");

    // The ungated close renders GREY, present and named.
    let (color, reason): (String, String) = conn
        .query_row(
            "SELECT color, reason FROM board WHERE src_path='ungated.md' AND to_path='subject.md'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("the ungated close must appear on the board");
    assert_eq!(color, "grey", "an ungated close is grey, never green");
    assert_eq!(reason, "declared-unpinned");

    // It is NEVER green.
    assert_eq!(
        scalar_i64(
            &conn,
            "SELECT count(*) FROM board WHERE src_path='ungated.md' AND color='green'",
        ),
        0,
        "an ungated close must never render green",
    );

    // --- FALSIFICATION: let the ungated close "render clean". -----------------
    // The pre-U5.1 red-only surface (board_red) DROPS the declared-unpinned edge
    // entirely — a reader that treated "absent from board_red" as clean/green
    // would render it silently clean. The `grey` branch is exactly what forbids
    // that: the edge is absent from board_red, present-and-grey on `board`.
    assert_eq!(
        scalar_i64(
            &conn,
            "SELECT count(*) FROM board_red WHERE src_path='ungated.md'",
        ),
        0,
        "FALSIFY: the red-only surface silently drops the ungated close",
    );
    assert_eq!(
        scalar_i64(
            &conn,
            "SELECT count(*) FROM board WHERE src_path='ungated.md'",
        ),
        1,
        "the board surfaces the ungated close (grey) — never silently clean",
    );
}

// ---------------------------------------------------------------------------
// Gate 3 — exactly one color per edge (the composed-legend invariant)
// ---------------------------------------------------------------------------

/// Every `^inputs` edge renders EXACTLY one board color — green, red, or grey.
/// A green (matched pin), a red (drift), and a grey (declared-unpinned) edge all
/// coexist and each appears once.
#[test]
fn gate_board_one_color_per_edge() {
    let subject_v1 = "# Subject\n\nbody one. ^claim\n";
    let subject_v2 = "# Subject\n\nbody two (drifted). ^claim\n";
    let other = "# Other\n\nother body. ^item\n";
    let r_other = live_node_rev("other.md", other, "^item");

    // green.md pins other.md#^item at its live rev (matches); drift.md pins
    // subject.md#^claim at R1 while the live subject is R2 (drift); grey.md is an
    // ungated close (declared-unpinned).
    let green_page = format!(
        "# Green\n\n```yaml ^inputs\nitems:\n  - {{ref: 'other.md', to: 'other.md#^item', rev: '{r_other}', rev_class: 'content'}}\n```\n"
    );
    let r1 = live_node_rev("subject.md", subject_v1, "^claim");
    let drift_page = review_page(&r1); // pins subject at R1

    let mut docs = BTreeMap::new();
    docs.insert("other.md".to_string(), doc(other));
    docs.insert("subject.md".to_string(), doc(subject_v2)); // live R2 ≠ pinned R1
    docs.insert("green.md".to_string(), doc(&green_page));
    docs.insert("review.md".to_string(), doc(&drift_page));
    docs.insert("grey.md".to_string(), doc(&ungated_review_page()));
    let conn = open_board(&docs, &[]).expect("open board");

    // Three edges, three distinct colors, one row each.
    assert_eq!(scalar_i64(&conn, "SELECT count(*) FROM board"), 3);
    for (src, color) in [
        ("green.md", "green"),
        ("review.md", "red"),
        ("grey.md", "grey"),
    ] {
        assert_eq!(
            scalar_i64(
                &conn,
                &format!("SELECT count(*) FROM board WHERE src_path='{src}' AND color='{color}'"),
            ),
            1,
            "{src} renders exactly one {color} row",
        );
    }
    // No edge is double-colored: the board row count equals the input_lock count.
    assert_eq!(
        scalar_i64(&conn, "SELECT count(*) FROM board"),
        scalar_i64(&conn, "SELECT count(*) FROM input_lock"),
        "exactly one color per ^inputs edge",
    );
}

// ---------------------------------------------------------------------------
// Gate 3b — archive fixture: a v1 (foreign-algo) pin renders superseded-algo grey
// ---------------------------------------------------------------------------

/// Archive fixture (U3.4): a lock pinned under `hash-algo: v1` — an algo this
/// engine does not compute — renders `grey superseded-algo` on the board, NEVER
/// red. The v1 rev cannot equal the live node-rev, so without the algo gate the
/// board would report drift (a false red) on an archived block that must stay
/// grey forever (d2 §6.3; U0.2/U3.4). The negative control is the same edge
/// under native `node-rev`, which reds on the same mismatch.
#[test]
fn gate_archived_v1_pin_renders_superseded_algo_grey() {
    let subject = "# Subject\n\nbody. ^claim\n";
    // A v1 (sha256) rev — foreign; never equals the live node-rev.
    let v1_page = "## Verdict\n\napproved\n\n```yaml ^inputs\nhash-algo: v1\nitems:\n  - {ref: 'subject.md', to: 'subject.md#^claim', rev: 'a1b2c3d4e5f60718', rev_class: 'content'}\n```\n";

    let mut docs = BTreeMap::new();
    docs.insert("subject.md".to_string(), doc(subject));
    docs.insert("v1.md".to_string(), doc(v1_page));
    let conn = open_board(&docs, &[]).expect("open board");

    let (color, reason): (String, String) = conn
        .query_row(
            "SELECT color, reason FROM board WHERE src_path='v1.md' AND to_path='subject.md'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("the v1 edge has a board row");
    assert_eq!(color, "grey", "a v1-algo pin is grey, never red drift");
    assert_eq!(reason, "superseded-algo");
    // Never counted as a board red.
    assert_eq!(
        scalar_i64(
            &conn,
            "SELECT count(*) FROM board_red WHERE src_path='v1.md'",
        ),
        0,
        "a superseded-algo edge is never a board red",
    );
    // Exactly one color for the edge.
    assert_eq!(
        scalar_i64(&conn, "SELECT count(*) FROM board WHERE src_path='v1.md'"),
        1
    );

    // Negative control: the SAME mismatch under native node-rev reds (drift).
    let native_page = review_page("a1b2c3d4e5f60718"); // node-rev header, wrong rev
    let mut nd = BTreeMap::new();
    nd.insert("subject.md".to_string(), doc(subject));
    nd.insert("review.md".to_string(), doc(&native_page));
    let conn2 = open_board(&nd, &[]).expect("open board native");
    let ncolor: String = conn2
        .query_row(
            "SELECT color FROM board WHERE src_path='review.md' AND to_path='subject.md'",
            [],
            |r| r.get(0),
        )
        .expect("the native edge has a board row");
    assert_eq!(ncolor, "red", "native algo + rev mismatch = red drift");
}

// ---------------------------------------------------------------------------
// Gate 3c — form-2 (ratified SCHEMA.md effect-receipt) chain renders grey
// ---------------------------------------------------------------------------

/// U3.4 compat reader: the ratified SCHEMA.md effect-receipt form (form-2) — a
/// plain fenced `yaml` chain block whose TRAILING block-anchor line is `^inputs`,
/// block-SEQUENCE `- ref:/hash:` items, and a bare `hash-algo: v1` header.
/// Pre-U3.4 the board saw ZERO rows for such a page (mrd was blind to form-2);
/// now the block PARSES and renders `grey superseded-algo` on the board — never
/// red, never silently empty. A MULTI-item form-2 block projects every item.
#[test]
fn gate_form2_chain_renders_superseded_algo_grey() {
    let raw = "## Chain\n\n```yaml\n- ref: '[[llm-wiki-skill-compilation]]'\n  claim:\n  hash: 'merkle-v1:247e292cc3c62e103424ad04cecb36517711cdfe42bc245ef516cfe54b83073d'\nhash-algo: v1\n```\n\n^inputs\n";
    let mut docs = BTreeMap::new();
    docs.insert("effect.md".to_string(), doc(raw));
    let conn = open_board(&docs, &[]).expect("open board");

    // NOT empty — the pre-change behavior was zero items (form-2 was invisible).
    assert_eq!(
        scalar_i64(
            &conn,
            "SELECT count(*) FROM input_lock WHERE src_path='effect.md'",
        ),
        1,
        "the form-2 chain parses one lock item (pre-U3.4: zero — the board was blind)",
    );
    let (color, reason): (String, String) = conn
        .query_row(
            "SELECT color, reason FROM board WHERE src_path='effect.md'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("the form-2 edge has a board row");
    assert_eq!(
        color, "grey",
        "a v1-algo form-2 pin is grey, never red drift"
    );
    assert_eq!(reason, "superseded-algo");
    // Never counted as a board red.
    assert_eq!(
        scalar_i64(
            &conn,
            "SELECT count(*) FROM board_red WHERE src_path='effect.md'",
        ),
        0,
        "a superseded-algo form-2 edge is never a board red",
    );

    // A MULTI-item form-2 block projects EVERY item, each grey superseded-algo.
    let multi = "## Chain\n\n```yaml\n- ref: '[[alpha]]'\n  hash: 'merkle-v1:aaaa'\n- ref: '[[beta]]'\n  hash: 'merkle-v1:bbbb'\nhash-algo: v1\n```\n\n^inputs\n";
    let mut mdocs = BTreeMap::new();
    mdocs.insert("multi.md".to_string(), doc(multi));
    let mconn = open_board(&mdocs, &[]).expect("open board multi");
    assert_eq!(
        scalar_i64(
            &mconn,
            "SELECT count(*) FROM board WHERE src_path='multi.md' AND color='grey' AND reason='superseded-algo'",
        ),
        2,
        "both form-2 block-sequence items project as grey superseded-algo",
    );
}

// ---------------------------------------------------------------------------
// Gate 3d — post-sweep: a v2 form-2 pin with a wikilink ref renders GREEN
// ---------------------------------------------------------------------------

/// U3.4 (the mrd-v2 leg of Gate B): after the v1→v2 supersede, a form-2 effect
/// page carries `hash-algo: v2` and a `[[wikilink]]`-by-NAME ref pinned at the
/// target's live `node_rev`. The board must render it GREEN — proving BOTH new
/// pieces on the board plane: (1) the wikilink NAME resolves to the real corpus
/// path (else the JOIN misses and it reds unresolved), and (2) `v2` is native
/// `{node-rev, v2}` (else it greys superseded-algo). This is what conserves the
/// pre-sweep-green pages green under mrd-v2. Negative controls: the SAME pin left
/// at `hash-algo: v1` greys, and a drifted v2 rev reds.
#[test]
fn gate_form2_v2_wikilink_pin_renders_green() {
    let target = "# Target\n\nsource body\n";
    // The target lives at a nested path; the ref is the bare NAME.
    let target_rev = live_node_rev("sources/target-page.md", target, "");
    let effect_v2 = format!(
        "## Chain\n\n```yaml\n- ref: '[[target-page]]'\n  claim:\n  hash: '{target_rev}'\nhash-algo: v2\n```\n\n^inputs\n"
    );

    let mut docs = BTreeMap::new();
    docs.insert("sources/target-page.md".to_string(), doc(target));
    docs.insert("effects/e.md".to_string(), doc(&effect_v2));
    let conn = open_board(&docs, &[]).expect("open board");

    let (color, reason, to_path): (String, String, String) = conn
        .query_row(
            "SELECT color, reason, to_path FROM board WHERE src_path='effects/e.md'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("the v2 form-2 edge has a board row");
    assert_eq!(
        to_path, "sources/target-page.md",
        "the wikilink NAME resolved to the real corpus path (else the JOIN misses)",
    );
    assert_eq!(color, "green", "v2 + wikilink + rev==live ⇒ green");
    assert_eq!(reason, "attested");

    // Negative control 1: the SAME pin still labeled v1 greys (not a false green).
    let effect_v1 = format!(
        "## Chain\n\n```yaml\n- ref: '[[target-page]]'\n  claim:\n  hash: '{target_rev}'\nhash-algo: v1\n```\n\n^inputs\n"
    );
    let mut d1 = BTreeMap::new();
    d1.insert("sources/target-page.md".to_string(), doc(target));
    d1.insert("effects/e.md".to_string(), doc(&effect_v1));
    let c1 = open_board(&d1, &[]).expect("open board v1");
    assert_eq!(
        scalar_i64(
            &c1,
            "SELECT count(*) FROM board WHERE src_path='effects/e.md' AND color='grey' AND reason='superseded-algo'",
        ),
        1,
        "the pre-sweep v1 label greys — the sweep to v2 is what turns it green",
    );

    // Negative control 2: a v2 pin whose rev no longer matches live reds (drift),
    // proving v2 is VERIFIED, never a blanket green.
    let effect_v2_stale = "## Chain\n\n```yaml\n- ref: '[[target-page]]'\n  hash: 'a1b2c3d4e5f60718'\nhash-algo: v2\n```\n\n^inputs\n";
    let mut d2 = BTreeMap::new();
    d2.insert("sources/target-page.md".to_string(), doc(target));
    d2.insert("effects/e.md".to_string(), doc(effect_v2_stale));
    let c2 = open_board(&d2, &[]).expect("open board v2 stale");
    assert_eq!(
        scalar_i64(
            &c2,
            "SELECT count(*) FROM board WHERE src_path='effects/e.md' AND color='red' AND reason='content-drifted'",
        ),
        1,
        "a v2 pin that drifts reds — v2 is verified, not blanket-green",
    );
}

// ---------------------------------------------------------------------------
// Gate 4 — the locked-face guard holds with the board view loaded
// ---------------------------------------------------------------------------

/// The board + trace views are read-only SQL over the SAME locked read face: they
/// read fine, but `ATTACH`/`COPY`/external access STILL refuse (no face widening,
/// A10; §2.1). This is the U2.9 blocked-ATTACH guard re-asserted with the U5.1
/// views loaded.
#[test]
fn gate_locked_face_holds_with_board() {
    let subject = "# Subject\n\nbody. ^claim\n";
    let mut docs = BTreeMap::new();
    docs.insert("subject.md".to_string(), doc(subject));
    docs.insert("ungated.md".to_string(), doc(&ungated_review_page()));
    let conn = open_board(&docs, &[]).expect("open board");

    // The board + trace views READ fine over the locked face.
    assert_eq!(scalar_i64(&conn, "SELECT count(*) FROM board"), 1);
    let _ = scalar_i64(&conn, "SELECT count(*) FROM co_edit_trace");

    // ATTACH still refuses — the face is not widened by the new views.
    let tmp = tempfile::tempdir().unwrap();
    let attach = format!("ATTACH '{}' AS x", tmp.path().join("side.duckdb").display());
    let err = conn.execute_batch(&attach).unwrap_err().to_string();
    let m = err.to_lowercase();
    assert!(
        m.contains("disabled by configuration")
            || m.contains("external access")
            || m.contains("enable_external_access")
            || m.contains("permission error"),
        "ATTACH must still refuse with the board view loaded, got: {err}",
    );
    // Re-raising the capability is still refused (configuration locked).
    assert!(
        conn.execute_batch("SET enable_external_access=true")
            .is_err(),
        "re-enabling external access must still refuse (configuration locked)",
    );
}
