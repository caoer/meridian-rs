//! Dev tool — emit the U5.1 board-view colors over a real workspace corpus, one JSON line per
//! lock-pin edge (`src_path`, `to_path`, `to_sel`, `color`, `reason`). **Its parity subject is
//! gone (R1.3).** It was the CANDIDATE side of the board-view leg parity, compared
//! field-for-field against the legacy `md status` color over `^inputs` edges.
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!

use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let Some(ws) = std::env::args().nth(1) else {
        eprintln!("usage: board_colors <workspace-dir>");
        return ExitCode::from(2);
    };
    match run(&ws) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("board_colors: {msg}");
            ExitCode::from(2)
        }
    }
}

fn run(ws: &str) -> Result<(), String> {
    let canonical = workspace::canonicalize(Path::new(ws)).map_err(|e| format!("resolve: {e}"))?;
    let root = fs::WorkspaceRoot(canonical);
    let (files, _fingerprint) = fs::domain_snapshot(&root).map_err(|e| format!("snapshot: {e}"))?;
    let (_index, docs, unserved) = fs::build_corpus(files);
    // A dev tool still owes the operator the same fact the CLI faces voice:
    // which members this scan never saw.
    for (member, condition) in &unserved {
        eprintln!("board_colors: warning: {member} {condition} — skipped by this scan");
    }

    // The board off U2.9's locked read face — the same default-face SQL the U5.1 gates assert
    // (empty journal: the trace is not exercised by the color leg).
    let conn = view::read_face::open_board(&docs).map_err(|e| format!("open_board: {e}"))?;
    let mut stmt = conn
        .prepare(
            "SELECT src_path, to_path, to_sel, color, reason \
             FROM board ORDER BY src_path, to_path, to_sel",
        )
        .map_err(|e| format!("prepare: {e}"))?;
    let rows = stmt
        .query_map([], |r| {
            Ok(serde_json::json!({
                "src_path": r.get::<_, String>(0)?,
                "to_path":  r.get::<_, String>(1)?,
                "to_sel":   r.get::<_, String>(2)?,
                "color":    r.get::<_, String>(3)?,
                "reason":   r.get::<_, String>(4)?,
            }))
        })
        .map_err(|e| format!("query: {e}"))?;
    for row in rows {
        let row = row.map_err(|e| format!("row: {e}"))?;
        println!("{row}");
    }
    Ok(())
}
