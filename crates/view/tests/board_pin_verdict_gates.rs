//! S9b — the pin verdict on the SQL board plane (exit criterion 3's board-color
//! leg; `docs/wire-contract-v2-colors-amendment.md` § Colors).
//!
//! S9 gave a `meridian-lock` pin a verdict on the COLOR plane (`mrd walk`,
//! `mrd status`). The board plane still answered `grey superseded-algo` for the
//! same pin, because a fingerprint verdict is a blake3 re-hash of the target's
//! bytes and the read face's SQL cannot express one. Two planes, two colors, one
//! question — and the board fails in the dangerous direction: grey means
//! "outside sight" on the surface most likely to be GLANCED at rather than read.
//!
//! The fix is a projected verdict, never a second computer: `project_input_locks`
//! asks `view::walk::lock_pin_colors` (the same `edge_color` the walk renders)
//! and carries the answer into `input_lock.verdict_{color,reason,detail}`.
//!
//! What these gates prove:
//!
//! 1. **Plane agreement, value for value** — every `meridian-lock` row's
//!    projected verdict equals the color plane's own answer, over all SIX
//!    outcomes in ONE corpus.
//! 2. **The board renders that verdict** — `red content-drifted` on the board
//!    where the walk says `red content-drifted`, and so on for the other five.
//! 3. **No double color** — exactly one board row per `input_lock` row with
//!    legacy and lock rows side by side.
//! 4. **Additive for the legacy plane** — every legacy `^inputs` row carries a
//!    NULL verdict and keeps the color U5.1 gave it.

use std::collections::BTreeMap;

use model::Document;
use view::read_face::open_board;
use view::walk::{color_detail, color_reason, color_tone, lock_pin_colors};

fn doc(raw: &str) -> Document {
    model::build(raw.to_string(), syntax::parse(raw))
}

/// The live fingerprint token of a page root — what a CORRECT pin holds. Minted
/// through the engine's own mint, so a fixture cannot pin a token the reader
/// would not recompute.
fn live_token(raw: &str) -> String {
    let d = doc(raw);
    model::fingerprint::fingerprint(&d, &d.root)
        .expect("the fixture page has content")
        .into_string()
}

/// An effect page whose `meridian-lock` block pins `declared_ref` at `token`,
/// rendered by the format's own sole writer so the fixture is the real byte form.
fn effect_page(declared_ref: &str, token: &str) -> String {
    let mut l = lock::Lock::new();
    l.upsert_pin(lock::PinEntry {
        declared_ref: declared_ref.to_string(),
        fingerprint: token.to_string(),
    });
    format!("# Effect\n\ndraws from it\n\n{}\n", lock::render(&l))
}

/// The SIX-state corpus: one effect page per outcome, each pinning a target that
/// makes exactly that outcome true, plus a legacy form-1 page as the control
/// that the `node_rev` plane is untouched.
///
/// Every page carries exactly ONE lock row, so `src_path` keys a row on both
/// planes and the two listings compare directly.
fn six_state_corpus() -> BTreeMap<String, Document> {
    let body = "# Target\n\nbody v1\n";
    let hex64 = "0".repeat(64);
    let mut docs = BTreeMap::new();
    docs.insert("sources/target.md".to_string(), doc(body));

    // green — the pin holds the target's live token.
    docs.insert(
        "green.md".to_string(),
        doc(&effect_page("sources/target.md", &live_token(body))),
    );
    // red content-drifted — a well-formed token holding a digest the target no
    // longer has (here: the token of a DIFFERENT body).
    docs.insert(
        "drifted.md".to_string(),
        doc(&effect_page(
            "sources/target.md",
            &live_token("# Target\n\nbody v2 edited\n"),
        )),
    );
    // red dangling-anchor — the address names a block anchor the target lost.
    docs.insert(
        "dangling.md".to_string(),
        doc(&effect_page(
            "sources/target.md#^goal",
            &format!("fp1.span2.b3.{hex64}"),
        )),
    );
    // grey unverifiable-fingerprint — the token PARSES but names a version this
    // build does not implement.
    docs.insert(
        "unverifiable.md".to_string(),
        doc(&effect_page(
            "sources/target.md",
            &format!("fp9.span2.b3.{hex64}"),
        )),
    );
    // grey malformed-fingerprint — the pinned value is not a token at all.
    docs.insert(
        "malformed.md".to_string(),
        doc(&effect_page("sources/target.md", "780d2fb4cf68f60f")),
    );
    // grey lock-refused — the whole block is unreadable.
    docs.insert(
        "refused.md".to_string(),
        doc("# Refused\n\n```meridian-lock\nversion: 1\ngarbage here\n```\n"),
    );
    // The legacy control: a form-1 `^inputs` pin at the target's live node_rev,
    // which the board's node_rev compare must still green on its own.
    let target_rev = doc(body).root.node_rev.0.clone();
    docs.insert(
        "legacy.md".to_string(),
        doc(&format!(
            "# Legacy\n\n```yaml ^inputs\nhash-algo: node-rev\nitems:\n  - {{ref: 'sources/target.md', rev: '{target_rev}', rev_class: 'content'}}\n```\n"
        )),
    );
    docs
}

/// Read `(src_path, color, reason)` off the board, ordered.
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

/// The live `node_rev` of one addressable selector — the rev a legacy pin would
/// record (read off the same `node` table the board joins).
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

// ---------------------------------------------------------------------------
// Gate 1 — the projected verdict IS the color plane's answer, value for value
// ---------------------------------------------------------------------------

/// THE plane-agreement gate. For every `meridian-lock` row in the six-state
/// corpus, the projected `verdict_{color,reason,detail}` equals
/// `view::walk::{color_tone, color_reason, color_detail}` of the SAME row's
/// color, compared as whole listings — not spot-checked, and not transcribed by
/// hand into an expectation the board could quietly drift away from.
///
/// A second color computer in the read face would show up here the moment the
/// two laws diverged on any one of the six outcomes.
#[test]
fn gate_projected_verdict_equals_the_color_planes_answer() {
    let docs = six_state_corpus();
    let conn = open_board(&docs, &[]).expect("open board");

    // The color plane's listing (what `mrd walk` / `mrd status` render).
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

    // The board plane's listing, off the locked read face.
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

    // Named, so a future reader sees the six outcomes spelled out.
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
                "grey lock-refused (malformed at line 3: unrecognized line (canonical order: version, objects, pins))".to_string()
            ),
            (
                "unverifiable.md".to_string(),
                "grey unverifiable-fingerprint (unknown version)".to_string()
            ),
        ],
        "six distinct outcomes, each naming its own reason",
    );
}

// ---------------------------------------------------------------------------
// Gate 2 — the BOARD renders that verdict (the surface a reader glances at)
// ---------------------------------------------------------------------------

/// The `board` view — the colors layer a reader actually queries — renders the
/// same verdict for each of the six outcomes, and the legacy control keeps the
/// color U5.1 gave it. This is the criterion-3 claim in one assertion: board
/// says `red content-drifted` exactly where the walk says `red content-drifted`.
#[test]
fn gate_board_renders_the_same_verdict_the_walk_renders() {
    let docs = six_state_corpus();
    let conn = open_board(&docs, &[]).expect("open board");

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
                "legacy.md".to_string(),
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

    // Not one lock row renders the pre-S9b answer any more.
    assert_eq!(
        scalar_i64(
            &conn,
            "SELECT count(*) FROM board b JOIN input_lock il USING (src_path, seq) \
             WHERE il.verdict_color IS NOT NULL AND b.reason = 'superseded-algo'",
        ),
        0,
        "a judged pin never renders `superseded-algo` — that was the two-plane divergence",
    );

    // The ONE red surface agrees with `board`: both reds are present there, and
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

// ---------------------------------------------------------------------------
// Gate 3 — one color per row, with both planes live in one corpus
// ---------------------------------------------------------------------------

/// The composed-legend invariant survives the new arm: the board emits EXACTLY
/// one row per `input_lock` row, with legacy `^inputs` rows and `meridian-lock`
/// rows side by side. The arms partition on `verdict_color`, so a lock row can
/// never be colored both by the `node_rev` compare and by its verdict.
#[test]
fn gate_exactly_one_board_row_per_lock_row_across_both_planes() {
    let docs = six_state_corpus();
    let conn = open_board(&docs, &[]).expect("open board");

    let locks = scalar_i64(&conn, "SELECT count(*) FROM input_lock");
    assert_eq!(locks, 7, "six lock rows + one legacy row");
    assert_eq!(
        scalar_i64(&conn, "SELECT count(*) FROM board"),
        locks,
        "exactly one color per lock item — no row colored twice, none dropped",
    );
    // Stated per row, so a duplicate on ONE page cannot hide in a total.
    assert_eq!(
        scalar_i64(
            &conn,
            "SELECT count(*) FROM (SELECT src_path, seq FROM board GROUP BY 1, 2 HAVING count(*) <> 1)",
        ),
        0,
        "no (src_path, seq) is colored twice",
    );
}

// ---------------------------------------------------------------------------
// Gate 4 — additive for the legacy plane (the frozen-DDL question)
// ---------------------------------------------------------------------------

/// The addition is additive for the READER, not only the writer: every legacy
/// `^inputs` row carries a NULL verdict, so all four U5.1 arms see exactly the
/// rows they saw before, and each legacy color is unchanged — green (matched
/// pin), red (drift), grey (superseded-algo), grey (declared-unpinned).
#[test]
fn gate_legacy_inputs_rows_are_untouched_by_the_verdict_column() {
    let subject_v1 = "# Subject\n\nbody one. ^claim\n";
    let subject_v2 = "# Subject\n\nbody two (drifted). ^claim\n";
    let claim_rev = live_node_rev("subject.md", subject_v1, "^claim");

    let mut docs = BTreeMap::new();
    docs.insert("subject.md".to_string(), doc(subject_v1));
    docs.insert(
        "green.md".to_string(),
        doc(&format!(
            "# G\n\n```yaml ^inputs\nhash-algo: node-rev\nitems:\n  - {{ref: 'subject.md', to: 'subject.md#^claim', rev: '{claim_rev}'}}\n```\n"
        )),
    );
    docs.insert(
        "v1.md".to_string(),
        doc("# V1\n\n```yaml ^inputs\nhash-algo: v1\nitems:\n  - {ref: 'subject.md', to: 'subject.md#^claim', rev: 'a1b2c3d4e5f60718'}\n```\n"),
    );
    docs.insert(
        "unpinned.md".to_string(),
        doc("# U\n\n```yaml ^inputs\nitems:\n  - {ref: 'subject.md', claim: 'declared-only'}\n```\n"),
    );

    let conn = open_board(&docs, &[]).expect("open board");
    assert_eq!(
        scalar_i64(
            &conn,
            "SELECT count(*) FROM input_lock WHERE verdict_color IS NOT NULL",
        ),
        0,
        "a legacy `^inputs` row carries NO verdict — the node_rev plane still owns it",
    );
    assert_eq!(
        board_rows(&conn),
        vec![
            (
                "green.md".to_string(),
                "green".to_string(),
                "attested".to_string()
            ),
            (
                "unpinned.md".to_string(),
                "grey".to_string(),
                "declared-unpinned".to_string()
            ),
            (
                "v1.md".to_string(),
                "grey".to_string(),
                "superseded-algo".to_string()
            ),
        ],
        "the three legacy colors are exactly what U5.1 rendered",
    );

    // And the legacy drift arm still fires: edit the subject after the pin.
    let mut drifted = docs;
    drifted.insert("subject.md".to_string(), doc(subject_v2));
    let conn2 = open_board(&drifted, &[]).expect("open board drifted");
    assert_eq!(
        scalar_i64(
            &conn2,
            "SELECT count(*) FROM board WHERE src_path='green.md' AND color='red' AND reason='content-drifted'",
        ),
        1,
        "the legacy node_rev drift compare is untouched by the verdict guard",
    );
}
