//! Pin verdict on the SQL board plane (wire-contract colors amendment).
//!
//! Projected verdict from `lock_pin_colors` into `input_lock.verdict_*` — one
//! colour plane, not two. Gates: (1) plane agreement over six outcomes,
//! (2) board renders that verdict, (3) one board row per lock row.

use std::collections::BTreeMap;

use model::Document;
use view::read_face::open_board;
use view::walk::{color_detail, color_reason, color_tone, lock_pin_colors};

fn doc(raw: &str) -> Document {
    model::build(raw.to_string(), syntax::parse(raw))
}

/// Live fingerprint of a page root (correct pin).
fn live_token(raw: &str) -> String {
    let d = doc(raw);
    model::fingerprint::fingerprint(&d, &d.root)
        .expect("the fixture page has content")
        .into_string()
}

/// Effect page pinning `declared_ref` at `token` (real lock render).
fn effect_page(declared_ref: &str, token: &str) -> String {
    let (target, fragment) = match declared_ref.split_once('#') {
        Some((t, f)) => (t, f),
        None => (declared_ref, ""),
    };
    let object = target.strip_suffix(".md").unwrap_or(target);
    let selector = if fragment.is_empty() {
        lock::Selector::Path(Vec::new())
    } else {
        lock::Selector::Path(fragment.split('/').map(str::to_string).collect())
    };
    let mut l = lock::Lock::new();
    l.upsert_pin(lock::PinEntry::new(
        object,
        "9ae3f1deadbeef",
        selector,
        token,
    ));
    format!("# Effect\n\ndraws from it\n\n{}\n", lock::render(&l))
}

/// Six outcomes, one lock row each — `src_path` keys both planes.
fn six_state_corpus() -> BTreeMap<String, Document> {
    let body = "# Target\n\nbody v1\n";
    let hex64 = "0".repeat(64);
    let mut docs = BTreeMap::new();
    docs.insert("sources/target.md".to_string(), doc(body));

    docs.insert(
        "green.md".to_string(),
        doc(&effect_page("sources/target.md", &live_token(body))),
    );
    docs.insert(
        "drifted.md".to_string(),
        doc(&effect_page(
            "sources/target.md",
            &live_token("# Target\n\nbody v2 edited\n"),
        )),
    );
    docs.insert(
        "dangling.md".to_string(),
        doc(&effect_page(
            "sources/target.md#^goal",
            &format!("fp1.span2.b3.{hex64}"),
        )),
    );
    docs.insert(
        "unverifiable.md".to_string(),
        doc(&effect_page(
            "sources/target.md",
            &format!("fp9.span2.b3.{hex64}"),
        )),
    );
    docs.insert(
        "malformed.md".to_string(),
        doc(&effect_page("sources/target.md", "780d2fb4cf68f60f")),
    );
    docs.insert(
        "refused.md".to_string(),
        doc("# Refused\n\n```meridian-lock\nversion: 2\ngarbage here\n```\n"),
    );
    docs
}

fn board_rows(conn: &duckdb::Connection) -> Vec<(String, String, String)> {
    let mut stmt = conn
        .prepare("SELECT src_path, color, reason FROM board ORDER BY src_path, seq")
        .expect("prepare board");
    let rows = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .expect("query board");
    rows.map(|r| r.expect("board row")).collect()
}

fn scalar_i64(conn: &duckdb::Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).expect("scalar")
}

/// Plane agreement: projected verdict equals colour plane, whole listings.
#[test]
fn gate_projected_verdict_equals_the_color_planes_answer() {
    let docs = six_state_corpus();
    let conn = open_board(&docs).expect("open board");

    let walk_rows: Vec<(String, String, Option<String>, Option<String>)> = lock_pin_colors(&docs)
        .into_iter()
        .map(|pin| {
            (
                pin.src_path,
                color_tone(&pin.color).to_string(),
                color_reason(&pin.color).map(str::to_string),
                color_detail(&pin.color),
            )
        })
        .collect();

    let mut stmt = conn
        .prepare(
            "SELECT src_path, verdict_color, verdict_reason, verdict_detail \
             FROM input_lock WHERE verdict_color IS NOT NULL ORDER BY src_path, seq",
        )
        .expect("prepare");
    let board_verdicts: Vec<(String, String, Option<String>, Option<String>)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .expect("query")
        .map(|r| r.expect("verdict row"))
        .collect();

    assert_eq!(
        board_verdicts.len(),
        6,
        "all six meridian-lock rows carry a verdict: {board_verdicts:?}",
    );
    assert_eq!(
        board_verdicts, walk_rows,
        "the board's verdict must BE the color plane's answer, row for row",
    );

    let labelled: Vec<(String, String)> = board_verdicts
        .iter()
        .map(|(src, color, reason, detail)| {
            let label = match (reason, detail) {
                (Some(r), Some(d)) => format!("{color} {r} ({d})"),
                (Some(r), None) => format!("{color} {r}"),
                _ => color.clone(),
            };
            (src.clone(), label)
        })
        .collect();
    assert_eq!(
        labelled,
        vec![
            (
                "dangling.md".to_string(),
                "red dangling-anchor".to_string()
            ),
            (
                "drifted.md".to_string(),
                "red content-drifted".to_string()
            ),
            ("green.md".to_string(), "green".to_string()),
            (
                "malformed.md".to_string(),
                "grey malformed-fingerprint".to_string()
            ),
            (
                "refused.md".to_string(),
                "grey lock-refused (malformed at line 3: unrecognized line (canonical order: version, pins))".to_string()
            ),
            (
                "unverifiable.md".to_string(),
                "grey unverifiable-fingerprint (unknown version)".to_string()
            ),
        ],
        "six distinct outcomes, each naming its own reason",
    );
}

/// Board view renders the same six verdicts as the walk.
#[test]
fn gate_board_renders_the_same_verdict_the_walk_renders() {
    let docs = six_state_corpus();
    let conn = open_board(&docs).expect("open board");

    assert_eq!(
        board_rows(&conn),
        vec![
            (
                "dangling.md".to_string(),
                "red".to_string(),
                "dangling-anchor".to_string()
            ),
            (
                "drifted.md".to_string(),
                "red".to_string(),
                "content-drifted".to_string()
            ),
            (
                "green.md".to_string(),
                "green".to_string(),
                "attested".to_string()
            ),
            (
                "malformed.md".to_string(),
                "grey".to_string(),
                "malformed-fingerprint".to_string()
            ),
            (
                "refused.md".to_string(),
                "grey".to_string(),
                "lock-refused".to_string()
            ),
            (
                "unverifiable.md".to_string(),
                "grey".to_string(),
                "unverifiable-fingerprint".to_string()
            ),
        ],
        "the board renders the pin verdict; `grey superseded-algo` is gone from every lock row",
    );

    assert_eq!(
        scalar_i64(
            &conn,
            "SELECT count(*) FROM board b JOIN input_lock il USING (src_path, seq) \
             WHERE il.verdict_color IS NOT NULL AND b.reason = 'superseded-algo'",
        ),
        0,
        "a judged pin never renders `superseded-algo` — that was the two-plane divergence",
    );

    // board_red agrees with board.
    // the greys are not (grey measured nothing, so it can never be a red).
    let red_srcs: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT src_path FROM board_red ORDER BY src_path")
            .expect("prepare");
        let rows = stmt.query_map([], |r| r.get(0)).expect("query");
        rows.map(|r| r.expect("red row")).collect()
    };
    assert_eq!(
        red_srcs,
        vec!["dangling.md".to_string(), "drifted.md".to_string()],
        "board_red carries exactly the fingerprint reds — never a grey, never the green",
    );
}

/// Exactly one board row per `input_lock` row (no double colour, none dropped).
#[test]
fn gate_exactly_one_board_row_per_lock_row_across_both_planes() {
    let docs = six_state_corpus();
    let conn = open_board(&docs).expect("open board");

    let locks = scalar_i64(&conn, "SELECT count(*) FROM input_lock");
    assert_eq!(
        locks, 6,
        "six lock rows — one form per page, no legacy plane"
    );
    assert_eq!(
        scalar_i64(&conn, "SELECT count(*) FROM board"),
        locks,
        "exactly one color per lock item — no row colored twice, none dropped",
    );
    assert_eq!(
        scalar_i64(
            &conn,
            "SELECT count(*) FROM (SELECT src_path, seq FROM board GROUP BY 1, 2 HAVING count(*) <> 1)",
        ),
        0,
        "no (src_path, seq) is colored twice",
    );
}
