//! The orphan lint (G3): run receipts that record an authorisation with no
//! completion.
//!
//! # What an orphan is
//! The run plane writes up to two receipt lines per invocation, joined by the
//! invocation id: a PRE-EXEC receipt anchored `^p-run-<invocation>` when a run
//! is authorised, and a COMPLETION receipt anchored `^r-run-<invocation>` when
//! it finishes. A pre-exec with no completion means the record cannot show that
//! run finished — a run that died mid-exec is exactly this shape, and the
//! pre-exec receipt exists to make it visible.
//!
//! The design named this lint three times and nobody built it, so until now
//! nothing read the receipts back. An anchor whose reader does not exist is a
//! plan, not a detector.
//!
//! # The era boundary, and why it is derived from the page itself
//! Builds before the completion marker landed wrote NO completion receipt when
//! a task emitted no descriptors — so a zero-effect run that genuinely
//! succeeded left exactly the bytes an abandoned run leaves. Those invocations
//! are **unauditable by construction**: the record cannot prove they finished,
//! and a lint that quietly excused them would assert what the bytes do not
//! support.
//!
//! They are still orphans and still flagged. What changes is the RENDERING:
//! four permanently-red items that can never clear train exactly the
//! bump-without-reading habit that made stale pins go green for weeks, so the
//! era is named where a reader sees it.
//!
//! The boundary is read off the page rather than configured: the first
//! completion receipt proves the writer was capable of emitting one, so
//! pre-exec receipts BEFORE it belong to the era that could not, and any
//! after it is a live incident. No build id, no date, no external state — the
//! evidence for the boundary is in the same bytes as the finding.

use std::collections::{BTreeMap, BTreeSet};

use model::Document;

/// One run receipt recording an authorisation whose completion never landed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrphanedRun {
    /// The receipt page carrying the line (workspace-relative).
    pub page: String,
    /// The invocation id joining the pre-exec and completion anchors.
    pub invocation: String,
    /// `true` when this orphan predates the page's first completion receipt —
    /// unauditable by construction rather than a live incident.
    pub before_completions_existed: bool,
}

/// The pre-exec anchor prefix (`^p-run-<invocation>`).
const PRE_EXEC: &str = "^p-run-";
/// The completion anchor prefix (`^r-run-<invocation>`).
const COMPLETION: &str = "^r-run-";

/// Every orphaned run receipt in the corpus, in page then document order.
///
/// Reads the parsed corpus rather than the disk: receipt pages are ordinary
/// markdown in the hash domain, so they are already here, and a detector that
/// re-read them could disagree with the bytes every other plane was computed
/// from.
#[must_use]
pub fn orphaned_runs(docs: &BTreeMap<String, Document>) -> Vec<OrphanedRun> {
    let mut out = Vec::new();
    for (page, doc) in docs {
        out.extend(orphans_on_page(page, &doc.raw));
    }
    out
}

fn orphans_on_page(page: &str, raw: &str) -> Vec<OrphanedRun> {
    // One pass for the completions, so the join and the era boundary are both
    // answered without re-scanning.
    let mut completed: BTreeSet<&str> = BTreeSet::new();
    let mut first_completion_line: Option<usize> = None;
    for (i, line) in receipt_lines(raw) {
        if let Some(id) = anchor_id(line, COMPLETION) {
            completed.insert(id);
            first_completion_line.get_or_insert(i);
        }
    }

    let mut out = Vec::new();
    for (i, line) in receipt_lines(raw) {
        let Some(id) = anchor_id(line, PRE_EXEC) else {
            continue;
        };
        if completed.contains(id) {
            continue;
        }
        out.push(OrphanedRun {
            page: page.to_owned(),
            invocation: id.to_owned(),
            // No completion anywhere on the page ⇒ the whole page predates the
            // writer that emits them.
            before_completions_existed: first_completion_line.is_none_or(|first| i < first),
        });
    }
    out
}

/// The page's lines that may carry a receipt, paired with their index —
/// everything OUTSIDE a fenced code block.
///
/// Found the hard way: the first live run of this lint flagged an invocation on
/// `s4-u4-adoption-evidence.md`, a page whose whole purpose is to QUOTE a
/// receipt line while explaining this defect. A lint that reads documentation
/// of a problem as an instance of it reports its own evidence file, and the
/// same shape — the instrument observing itself — has now cost this lane a
/// measurement, a negative control, and this.
///
/// Fence tracking, not full parsing: a receipt line is a top-level list item
/// and the only realistic false positive is prose quoting the format.
fn receipt_lines(raw: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut in_fence = false;
    raw.lines().enumerate().filter(move |(_, line)| {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            return false;
        }
        !in_fence
    })
}

/// The invocation id of an anchor with `prefix`, or `None` when the line
/// carries no such anchor. The anchor is the LAST token of a receipt line.
fn anchor_id<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
    let token = line
        .rsplit_once(char::is_whitespace)
        .map_or(line, |(_, t)| t);
    let id = token.strip_prefix(prefix)?;
    (!id.is_empty()).then_some(id)
}

/// The lint's render: one line naming the count and the era split, then one
/// line per orphan. `None` when nothing is orphaned.
///
/// The verdict does not soften for the unauditable era — every orphan is
/// reported. Only the WORDS differ, so a reader can tell a run nobody can ever
/// audit from one that just went missing.
#[must_use]
pub fn render(orphans: &[OrphanedRun]) -> Option<String> {
    if orphans.is_empty() {
        return None;
    }
    let (historic, live): (Vec<_>, Vec<_>) =
        orphans.iter().partition(|o| o.before_completions_existed);

    let mut lines = Vec::new();
    lines.push(format!(
        "  run receipts: {} orphaned ({} unauditable-by-construction · {} live)",
        orphans.len(),
        historic.len(),
        live.len(),
    ));
    for o in &live {
        lines.push(format!(
            "    ORPHAN {} {} — authorised, no completion receipt; the run did not finish, or finished without recording it",
            o.page, o.invocation,
        ));
    }
    if !historic.is_empty() {
        lines.push(format!(
            "    {} orphan(s) predate the first completion receipt on their page — that writer emitted none for a run with no effects, so a succeeded run and an abandoned one left identical bytes. Unauditable by construction, not by neglect; they will never clear.",
            historic.len(),
        ));
        for o in &historic {
            lines.push(format!("    ORPHAN(historic) {} {}", o.page, o.invocation));
        }
    }
    Some(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(lines: &[&str]) -> String {
        lines.join("\n")
    }

    #[test]
    fn a_pre_exec_without_its_completion_is_an_orphan() {
        let raw = page(&[
            "- run {\"task\":\"a\"} ^p-run-inv-1",
            "- run {\"task\":\"a\"} ^r-run-inv-1",
            "- run {\"task\":\"b\"} ^p-run-inv-2",
        ]);
        let found = orphans_on_page("receipts/run.md", &raw);
        assert_eq!(found.len(), 1, "only inv-2 is unmatched: {found:?}");
        assert_eq!(found[0].invocation, "inv-2");
        assert!(
            !found[0].before_completions_existed,
            "inv-2 comes AFTER a completion, so its page's writer could emit one"
        );
    }

    #[test]
    fn a_completed_run_is_never_flagged() {
        let raw = page(&[
            "- run {\"task\":\"a\"} ^p-run-inv-1",
            "- run {\"task\":\"a\"} ^r-run-inv-1",
        ]);
        assert!(orphans_on_page("receipts/run.md", &raw).is_empty());
    }

    #[test]
    fn orphans_before_the_first_completion_are_marked_historic() {
        let raw = page(&[
            "- run {} ^p-run-old-1",
            "- run {} ^p-run-old-2",
            "- run {} ^p-run-new-1",
            "- run {} ^r-run-new-1",
            "- run {} ^p-run-live-1",
        ]);
        let found = orphans_on_page("receipts/run.md", &raw);
        let historic: Vec<_> = found
            .iter()
            .filter(|o| o.before_completions_existed)
            .map(|o| o.invocation.as_str())
            .collect();
        let live: Vec<_> = found
            .iter()
            .filter(|o| !o.before_completions_existed)
            .map(|o| o.invocation.as_str())
            .collect();
        assert_eq!(historic, vec!["old-1", "old-2"]);
        assert_eq!(live, vec!["live-1"], "after the boundary it is an incident");
    }

    #[test]
    fn a_page_with_no_completions_at_all_is_entirely_historic() {
        let raw = page(&["- run {} ^p-run-a", "- run {} ^p-run-b"]);
        let found = orphans_on_page("receipts/run.md", &raw);
        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|o| o.before_completions_existed));
    }

    #[test]
    fn the_render_reports_every_orphan_and_separates_only_the_words() {
        let raw = page(&[
            "- run {} ^p-run-old",
            "- run {} ^p-run-done",
            "- run {} ^r-run-done",
            "- run {} ^p-run-live",
        ]);
        let found = orphans_on_page("receipts/run.md", &raw);
        let rendered = render(&found).expect("two orphans render");
        assert!(rendered.contains("2 orphaned"), "{rendered}");
        assert!(
            rendered.contains("1 unauditable-by-construction"),
            "{rendered}"
        );
        assert!(rendered.contains("1 live"), "{rendered}");
        // Both are named — the era changes the wording, never the verdict.
        assert!(
            rendered.contains("run-live") || rendered.contains("live"),
            "{rendered}"
        );
        assert!(rendered.contains("old"), "{rendered}");
    }

    #[test]
    fn nothing_orphaned_renders_nothing() {
        assert!(render(&[]).is_none());
    }

    /// A page that DOCUMENTS the receipt format is not a page that has
    /// receipts. Regression for the first live run, which flagged the very
    /// evidence file explaining this defect.
    #[test]
    fn a_receipt_quoted_inside_a_code_fence_is_not_a_finding() {
        let raw = page(&[
            "# Why receipts have no outcome field",
            "",
            "```",
            "- run {\"page\":\"notes/task.md\"} ^p-run-1785269972228-79593",
            "```",
            "",
            "That line is an example, not a receipt.",
        ]);
        assert!(
            orphans_on_page("docs/evidence.md", &raw).is_empty(),
            "documentation of the format must not read as an instance of it"
        );
    }

    /// The fence guard must not swallow real receipts that follow a fence.
    #[test]
    fn receipts_after_a_closed_fence_are_still_found() {
        let raw = page(&[
            "```",
            "- run {} ^p-run-quoted",
            "```",
            "- run {} ^p-run-real",
        ]);
        let found = orphans_on_page("receipts/run.md", &raw);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].invocation, "real");
    }

    #[test]
    fn a_line_without_a_run_anchor_is_ignored() {
        let raw = page(&[
            "# Receipts",
            "prose about ^p-run- things",
            "- splice {} ^r-000001",
        ]);
        assert!(orphans_on_page("receipts/run.md", &raw).is_empty());
    }
}
