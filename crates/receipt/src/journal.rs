//! The receipt journal — the append-only ledger of guarded writes, and its
//! chain-continuity detector (d2 §2.1 A3/A9/A5).
//!
//! # What the journal is
//! ONE reserved in-vault markdown page (`fs::domain::RESERVED_JOURNAL_PATH`),
//! append-only by discipline, git-tracked, and **root-EXCLUDED**: the workspace
//! tree merkle never covers it. Because it is excluded, a row may carry BOTH
//! the `root_before` and the `root_after` of the write it receipts — the
//! `root_after` no longer covers the journal's own bytes, so it is not
//! self-referential (the no-self-rooting limit of the in-domain wire receipt,
//! wire-contract-v2 §6.2, is lifted here precisely by exclusion). Carrying both
//! roots is what makes the chain-continuity detector below possible.
//!
//! Every field of a row is a **source-3 fact** (the engine's own acts, A5:
//! write-mechanics only — op, path, actor, now, roots, edit rev transitions;
//! never an outcome that carries meaning).
//!
//! # Append-only is a discipline with a two-sided detector, not a property
//! - **Chain continuity (this module):** `root_after(N)` must equal
//!   `root_before(N+1)` for consecutive rows, so a hand-inserted row breaks the
//!   chain mechanically and a forger must re-forge the entire suffix.
//!   [`check_chain`] recomputes it and cites the breaking row — the library
//!   surface the `check --core` verb (U2.10) mounts.
//! - **The git witness (outside this module):** the journal is git-tracked
//!   content, so an offline rewrite is a history divergence `origin` exposes.
//!
//! # Integrity residuals — both named, stated not hidden (d2 §2.1)
//! 1. **Pre-push offline rewrite** — a full offline rewrite of the journal
//!    before the first push is undetectable from inside the engine (git's own
//!    trust floor; cryptographic closure is a deferred door).
//! 2. **Root-preserving online forged-row insertion** — a forged row whose
//!    `root_before`/`root_after` already chain (`root_before` = the prior
//!    row's `root_after`, `root_after` = the next row's `root_before`) is NOT
//!    caught by [`check_chain`]: the chain stays continuous. Detection of such
//!    a row rests on the **receipt-engine-only write restriction** (an ordinary
//!    `^put`/splice at the reserved path refuses — enforced in `wire-serve`)
//!    plus the git witness, never on chain continuity alone.
//!
//! The write restriction itself lives at the write choke-point
//! (`wire-serve::write::splice`); this module owns the FORMAT and the detector.

use std::fmt::Write;

use crate::{EditFact, anchor, shape_display, target_display};

/// One journal row's facts — every field a source-3 fact (A5 write-mechanics
/// bound). Rendered by [`render_row`], recovered by [`parse_rows`].
#[derive(Debug, Clone)]
pub struct JournalRow<'a> {
    /// The row counter → the block anchor `r-{seq:06}` (`^r-NNNNNN`).
    pub seq: u64,
    /// The guarded op that produced the row: `splice` | `create` | `remove`.
    pub op: &'a str,
    /// The content target the write landed on.
    pub path: &'a str,
    /// The recorded actor (§5.2: `Option` — recorded exactly as given, never
    /// invented; a mandatory type would force a fake actor).
    pub actor: Option<&'a str>,
    /// The recorded timestamp (recorded exactly as given, never invented).
    pub now: Option<&'a str>,
    /// The workspace tree merkle BEFORE the write.
    pub root_before: &'a str,
    /// The workspace tree merkle AFTER the write — recordable here only because
    /// the journal is root-EXCLUDED (it does not cover the row's own bytes).
    pub root_after: &'a str,
    /// The write's edits (rev transitions per target), same order as the batch.
    pub edits: Vec<EditFact<'a>>,
}

/// Render one journal row as a single markdown list item — the block-leaf
/// bytes, no trailing newline (the appender owns terminators). Absent inputs
/// produce absent tokens (no `actor=`/`now=` when the fact is absent). The
/// trailing `^r-NNNNNN` block anchor is the row's addressable identity.
///
/// Shape:
/// `- op=splice path=notes/plan.md actor=… now=… root_before=b3:… \
/// root_after=b3:… edits=1 Goals>Q3:match:33d5…->41f6… ^r-000042`
#[must_use]
pub fn render_row(row: &JournalRow<'_>) -> String {
    let mut out = String::new();
    let _ = write!(out, "- op={} path={}", row.op, row.path);
    if let Some(actor) = row.actor {
        let _ = write!(out, " actor={actor}");
    }
    if let Some(now) = row.now {
        let _ = write!(out, " now={now}");
    }
    let _ = write!(
        out,
        " root_before={} root_after={} edits={}",
        row.root_before,
        row.root_after,
        row.edits.len()
    );
    for edit in &row.edits {
        let _ = write!(
            out,
            " {}:{}:{}->{}",
            target_display(edit.target),
            shape_display(edit.shape),
            edit.before.0,
            edit.after.0
        );
    }
    let _ = write!(out, " ^{}", anchor(row.seq));
    out
}

/// A journal row recovered from the page — the scalar columns the detector and
/// the fact-table projections read. `line_no` is 1-based (for citation).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedRow {
    /// The block anchor without the leading `^`, e.g. `r-000042`.
    pub anchor: String,
    pub op: String,
    pub path: String,
    pub actor: Option<String>,
    pub now: Option<String>,
    pub root_before: String,
    pub root_after: String,
    /// The declared edit count (`edits=N`).
    pub edits: usize,
    /// 1-based line number in the page — the citation coordinate.
    pub line_no: usize,
}

/// Parse a journal page into its rows, in page order. A line is a row iff it is
/// a list item (`- `) carrying `root_before=`, `root_after=`, and a trailing
/// `^r-…` anchor — so headings and prose are skipped, and a plausible-looking
/// hand-inserted row (the forgery the detector must catch) IS parsed as a row.
#[must_use]
pub fn parse_rows(page: &str) -> Vec<ParsedRow> {
    let mut rows = Vec::new();
    for (i, raw) in page.lines().enumerate() {
        let line = raw.trim();
        let Some(body) = line.strip_prefix("- ") else {
            continue;
        };
        let tokens: Vec<&str> = body.split_whitespace().collect();
        let anchor = tokens
            .iter()
            .rev()
            .find_map(|t| t.strip_prefix('^').filter(|a| a.starts_with("r-")));
        let (Some(anchor), Some(root_before), Some(root_after)) = (
            anchor,
            token_value(&tokens, "root_before"),
            token_value(&tokens, "root_after"),
        ) else {
            continue;
        };
        rows.push(ParsedRow {
            anchor: anchor.to_string(),
            op: token_value(&tokens, "op").unwrap_or("").to_string(),
            path: token_value(&tokens, "path").unwrap_or("").to_string(),
            actor: token_value(&tokens, "actor").map(str::to_string),
            now: token_value(&tokens, "now").map(str::to_string),
            root_before: root_before.to_string(),
            root_after: root_after.to_string(),
            edits: token_value(&tokens, "edits")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
            line_no: i + 1,
        });
    }
    rows
}

/// The value of the first `key=value` token, or `None` if the key is absent.
fn token_value<'a>(tokens: &[&'a str], key: &str) -> Option<&'a str> {
    tokens
        .iter()
        .find_map(|t| t.strip_prefix(key).and_then(|r| r.strip_prefix('=')))
}

/// One break in the chain: a row whose `root_before` does not continue the
/// prior row's `root_after`. The row is cited by anchor and line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainBreak {
    /// The anchor of the row that fails to continue the chain (the cited row).
    pub row_anchor: String,
    /// 1-based line number of that row.
    pub line_no: usize,
    /// What `root_before` should have been = the prior row's `root_after`.
    pub expected_root_before: String,
    /// What `root_before` the row actually carries.
    pub found_root_before: String,
}

/// The chain-continuity verdict over a parsed journal. Green ⇔ no breaks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainReport {
    /// Every discontinuity, in page order. Empty ⇔ the chain is continuous.
    pub breaks: Vec<ChainBreak>,
}

impl ChainReport {
    /// The chain is continuous (no spliced/forged row broke it).
    #[must_use]
    pub fn is_green(&self) -> bool {
        self.breaks.is_empty()
    }

    /// The chain is broken — at least one row does not continue it.
    #[must_use]
    pub fn is_red(&self) -> bool {
        !self.breaks.is_empty()
    }

    /// A red render citing each broken row, or `None` when green. This is the
    /// string the `check --core` verb (U2.10) renders under its red verdict.
    #[must_use]
    pub fn red_summary(&self) -> Option<String> {
        if self.is_green() {
            return None;
        }
        let mut out = String::from("journal chain broken:");
        for b in &self.breaks {
            let _ = write!(
                out,
                " row ^{} (line {}) root_before={} does not continue prior root_after={};",
                b.row_anchor, b.line_no, b.found_root_before, b.expected_root_before
            );
        }
        Some(out)
    }
}

/// **The detection primitive** (d2 §2.1 chain-continuity; U2.10 mounts this):
/// recompute chain continuity over the parsed rows. For each consecutive pair
/// `(N, N+1)`, `root_after(N)` must equal `root_before(N+1)`; every row that
/// fails to continue the chain is cited as a [`ChainBreak`]. A hand-inserted
/// row with fabricated roots breaks the pair it sits in and is cited.
///
/// Chain continuity does NOT catch a root-preserving forged row (residual 2,
/// module docs): a row crafted so its roots already chain leaves no break.
#[must_use]
pub fn check_chain(rows: &[ParsedRow]) -> ChainReport {
    let mut breaks = Vec::new();
    for pair in rows.windows(2) {
        let (prev, cur) = (&pair[0], &pair[1]);
        if prev.root_after != cur.root_before {
            breaks.push(ChainBreak {
                row_anchor: cur.anchor.clone(),
                line_no: cur.line_no,
                expected_root_before: prev.root_after.clone(),
                found_root_before: cur.root_before.clone(),
            });
        }
    }
    ChainReport { breaks }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(seq: u64, rb: &str, ra: &str) -> String {
        render_row(&JournalRow {
            seq,
            op: "splice",
            path: "notes/plan.md",
            actor: Some("alice"),
            now: Some("2026-07-23T12:00:00Z"),
            root_before: rb,
            root_after: ra,
            edits: Vec::new(),
        })
    }

    /// A row renders with both roots and a trailing anchor, and round-trips
    /// through the parser (the projection + detector read what the engine
    /// wrote).
    #[test]
    fn render_then_parse_round_trips() {
        let line = row(7, "b3:aaa", "b3:bbb");
        assert!(line.starts_with("- op=splice path=notes/plan.md"));
        assert!(line.contains("root_before=b3:aaa root_after=b3:bbb"));
        assert!(line.ends_with("^r-000007"));
        let parsed = parse_rows(&line);
        assert_eq!(parsed.len(), 1);
        let p = &parsed[0];
        assert_eq!(p.anchor, "r-000007");
        assert_eq!(p.op, "splice");
        assert_eq!(p.path, "notes/plan.md");
        assert_eq!(p.actor.as_deref(), Some("alice"));
        assert_eq!(p.root_before, "b3:aaa");
        assert_eq!(p.root_after, "b3:bbb");
    }

    /// Absent actor/now render as absent tokens and parse back as `None`.
    #[test]
    fn absent_facts_are_absent_tokens() {
        let line = render_row(&JournalRow {
            seq: 1,
            op: "create",
            path: "a.md",
            actor: None,
            now: None,
            root_before: "b3:x",
            root_after: "b3:y",
            edits: Vec::new(),
        });
        assert!(!line.contains("actor="));
        assert!(!line.contains("now="));
        let p = &parse_rows(&line)[0];
        assert_eq!(p.actor, None);
        assert_eq!(p.now, None);
    }

    /// A continuous chain (`root_after(N)` == `root_before(N+1)`) is green.
    #[test]
    fn continuous_chain_is_green() {
        let page = format!(
            "# journal\n{}\n{}\n{}\n",
            row(1, "b3:0", "b3:1"),
            row(2, "b3:1", "b3:2"),
            row(3, "b3:2", "b3:3"),
        );
        let report = check_chain(&parse_rows(&page));
        assert!(report.is_green());
        assert!(report.red_summary().is_none());
    }

    /// Prose and headings between rows are skipped, not parsed as rows.
    #[test]
    fn non_row_lines_are_skipped() {
        let page = format!(
            "# Receipt journal\n\nnotes here\n{}\n",
            row(1, "b3:0", "b3:1")
        );
        assert_eq!(parse_rows(&page).len(), 1);
    }

    /// **The spliced-row negative** (d2 §2.1; task gate): a hand-inserted row
    /// with fabricated roots is dropped between two honest rows. The forged
    /// row's `root_before` does not continue the prior row's `root_after`, so
    /// the chain breaks and the detector renders RED citing the forged row by
    /// anchor and line. A `check --core` verb (U2.10) mounts exactly this.
    #[test]
    fn spliced_row_breaks_chain_and_is_cited() {
        // Honest rows 1 and 2 chain: b3:0 -> b3:1 -> b3:2. A forged row is
        // spliced in with roots that belong to no real write.
        let forged = render_row(&JournalRow {
            seq: 99,
            op: "splice",
            path: "notes/plan.md",
            actor: Some("mallory"),
            now: Some("2026-07-23T13:00:00Z"),
            root_before: "b3:FORGED_BEFORE",
            root_after: "b3:FORGED_AFTER",
            edits: Vec::new(),
        });
        let page = format!(
            "# journal\n{}\n{}\n{}\n",
            row(1, "b3:0", "b3:1"),
            forged,
            row(2, "b3:1", "b3:2"),
        );

        let report = check_chain(&parse_rows(&page));
        assert!(report.is_red(), "a spliced row must break the chain");

        // The FORGED row is cited first: its root_before did not continue the
        // prior honest row's root_after.
        let first = &report.breaks[0];
        assert_eq!(first.row_anchor, "r-000099", "the forged row is cited");
        assert_eq!(first.expected_root_before, "b3:1");
        assert_eq!(first.found_root_before, "b3:FORGED_BEFORE");

        // The red render names the row — the citation a red verdict shows.
        let summary = report.red_summary().expect("red render present");
        assert!(summary.contains("^r-000099"), "render cites the forged row");
    }
}
