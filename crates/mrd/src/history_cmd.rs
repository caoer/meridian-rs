//! `mrd test --history` — the tier-1 HISTORY runner (U1.6): calibrate a
//! convention against a workspace's own past, with a golden list of declared
//! exceptions as the gate.
//!
//! # What the tier does (rulings § test --history)
//! The receipt journal ([`fs::domain::RESERVED_JOURNAL_PATH`]) is the append-only
//! ledger of every guarded write; git is the witness that carries the actual
//! bytes. This tier JOINs the two: for each journal row it finds the git commit
//! that appended the row's `^r-NNNNNN` anchor (the **git-anchor JOIN**), reads the
//! written path at that commit (the AFTER bytes) and at its parent (the BEFORE
//! bytes), rebuilds both `Document`s through the fixture doc-building path
//! (`syntax::parse` → `model::build`), derives the [`rulepack-api@2`](policy)
//! change, and runs the convention's `check_change` over it — the SAME loader and
//! full-`EvalLimits` evaluator the door uses ([`policy::load_convention`],
//! [`policy::Convention::check_change`]).
//!
//! # Fidelity is counted, rendered, never guessed
//! A row reconstructs at one of three fidelities:
//! - **B full-bytes** — both the before and after bytes were recovered; the
//!   change is exact.
//! - **A structural** — exactly one side was recovered (a create has no before, a
//!   remove no after); the recovered side is real, the absent side is the empty
//!   document, so the doc facts a CHECK reads are honest even though the state
//!   diff is structural.
//! - **C grey** — neither side could be recovered (the row's anchor is in no
//!   commit, or the path is absent from the tree). A grey row is COUNTED and
//!   RENDERED but NEVER run — the tier refuses to guess a change it cannot
//!   reconstruct.
//!
//! # The golden list (rulings § test --history — the golden-list mechanism)
//! The golden list is a pinned page inside the convention folder
//! (`conventions/<slug>/GOLDEN.md`). Each row declares one would-refuse item by
//! its journal anchor plus a reason. Triage = editing that page through the
//! ordinary write door; exceptions are declared, never erased. `test --history`
//! FAILS (exit 1) on any would-refuse item ABSENT from the pinned list; a declared
//! item passes with its reason rendered.
//!
//! # Output + exit codes (§4 preamble law, `docs/status.md`)
//! JSON under `--json`, a human table otherwise. Exit 0 (every would-refuse item
//! is declared), 1 (an undeclared would-refuse item — a finding), 2 (a tool
//! failure: bad usage, an unreadable workspace / convention / journal, a git
//! failure, or a CHECK that faulted on real history).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use model::{Document, Edit, NodeKind};
use policy::{
    ChangeOp, CheckLimits, Convention, ConventionFiles, EdgeDecl, Invocation, derive_change,
    load_convention,
};
use serde_json::{Value, json};

use crate::{Fail, Format, current_dir};

/// The golden-list page inside a convention folder.
const GOLDEN_FILE: &str = "GOLDEN.md";

/// Run `mrd test --history WORKSPACE --convention SLUG [--json]`.
///
/// # Errors
/// [`Fail`] — exit 2 (bad usage, an unreadable workspace / convention / journal, a
/// git failure, or a faulting CHECK) or exit 1 (an undeclared would-refuse item).
pub(crate) fn dispatch(args: &[String]) -> Result<(), Fail> {
    let mut workspace: Option<String> = None;
    let mut convention: Option<String> = None;
    let mut json = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--history" => {}
            "--json" => json = true,
            "--convention" => {
                i += 1;
                convention = Some(
                    args.get(i)
                        .cloned()
                        .ok_or_else(|| Fail::tool("--convention needs a SLUG".to_owned()))?,
                );
            }
            flag if flag.starts_with('-') => {
                return Err(Fail::tool(format!("unknown flag: {flag}")));
            }
            value if workspace.is_none() => workspace = Some(value.to_owned()),
            value => return Err(Fail::tool(format!("unexpected argument: {value}"))),
        }
        i += 1;
    }
    let workspace =
        workspace.ok_or_else(|| Fail::tool("test --history needs a WORKSPACE path".to_owned()))?;
    let convention = convention
        .ok_or_else(|| Fail::tool("test --history needs --convention SLUG".to_owned()))?;
    let format = if json { Format::Json } else { Format::Human };

    let report = run_history(Path::new(&workspace), &convention)?;
    match format {
        Format::Json => println!("{}", report.to_json()),
        Format::Human => print!("{}", report.to_human()),
    }

    if !report.errors.is_empty() {
        return Err(Fail::tool(format!(
            "{} row(s) faulted the CHECK on real history",
            report.errors.len()
        )));
    }
    if report.undeclared > 0 {
        return Err(Fail::findings(format!(
            "{} would-refuse item(s) undeclared in {GOLDEN_FILE}",
            report.undeclared
        )));
    }
    Ok(())
}

/// Load the convention, read the golden list, parse the journal, JOIN each row
/// against git, run the CHECK, and fold the outcomes into a report.
fn run_history(workspace_arg: &Path, slug: &str) -> Result<HistoryReport, Fail> {
    let workspace = resolve_workspace(workspace_arg)?;

    // 1. Load the convention from its in-tree folder (the same loader the door uses).
    let conv_root = workspace.join("conventions").join(slug);
    if !conv_root.is_dir() {
        return Err(Fail::tool(format!(
            "no convention folder at conventions/{slug} under {}",
            workspace.display()
        )));
    }
    let files = DiskConventionFiles {
        root: conv_root.clone(),
    };
    let convention = load_convention(slug, &files, CheckLimits::default())
        .map_err(|e| Fail::tool(format!("cannot load convention '{slug}': {e}")))?;

    // 2. The golden list of declared exceptions (absent ⇒ nothing declared yet).
    let golden = load_golden(&conv_root)?;

    // 3. The receipt journal — the append-only ledger of guarded writes.
    let journal_rel = fs::domain::RESERVED_JOURNAL_PATH;
    let journal_abs = workspace.join(journal_rel);
    let journal_text = std::fs::read_to_string(&journal_abs).map_err(|e| {
        Fail::tool(format!(
            "no readable receipt journal at {journal_rel} under {}: {e}",
            workspace.display()
        ))
    })?;
    let rows = receipt::journal::parse_rows(&journal_text);

    // 4. The git-anchor JOIN: which commit appended each row's anchor.
    let anchor_commit = anchor_commits(&workspace, journal_rel)?;

    // 5. Reconstruct + check each row.
    let mut results = Vec::with_capacity(rows.len());
    for row in &rows {
        results.push(process_row(
            &workspace,
            &convention,
            &golden,
            &anchor_commit,
            row,
        ));
    }

    Ok(HistoryReport::assemble(slug, &rows, results))
}

/// Resolve the workspace argument against cwd and confirm it is a directory.
fn resolve_workspace(arg: &Path) -> Result<PathBuf, Fail> {
    let base = if arg.is_absolute() {
        arg.to_path_buf()
    } else {
        current_dir()?.join(arg)
    };
    if !base.is_dir() {
        return Err(Fail::tool(format!(
            "workspace {} is not a directory",
            base.display()
        )));
    }
    Ok(base)
}

// ── one row: JOIN → rebuild → derive → check → classify ──────────────────────

/// The reconstruction fidelity of one journal row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fidelity {
    /// Both sides recovered — an exact change.
    FullBytes,
    /// One side recovered (create / remove) — the recovered side is real.
    Structural,
    /// Neither side recovered — counted, rendered, never run.
    Grey,
}

impl Fidelity {
    fn label(self) -> &'static str {
        match self {
            Fidelity::FullBytes => "B full-bytes",
            Fidelity::Structural => "A structural",
            Fidelity::Grey => "C grey",
        }
    }
}

/// One row's verdict after the CHECK (or its absence).
enum Verdict {
    /// Out of the convention's `paths:` scope — not its concern, never run.
    OutOfScope,
    /// Grey (class C) — could not be reconstructed, so never run.
    Grey,
    /// The CHECK ran and emitted no refusal.
    Pass,
    /// The CHECK fired AND the item is declared in the golden list (its reason).
    Declared { reason: String, message: String },
    /// The CHECK fired and the item is NOT in the golden list — the finding.
    Undeclared { message: String },
    /// The CHECK faulted on a real reconstructed change — a tool failure.
    Error { detail: String },
}

/// One row's fully-resolved outcome.
struct RowResult {
    anchor: String,
    path: String,
    op: String,
    fidelity: Fidelity,
    verdict: Verdict,
}

/// Reconstruct one journal row and run the convention over it.
fn process_row(
    workspace: &Path,
    convention: &Convention,
    golden: &BTreeMap<String, String>,
    anchor_commit: &BTreeMap<String, String>,
    row: &receipt::journal::ParsedRow,
) -> RowResult {
    let base = |fidelity, verdict| RowResult {
        anchor: row.anchor.clone(),
        path: row.path.clone(),
        op: row.op.clone(),
        fidelity,
        verdict,
    };

    // A row outside the convention's scope is not its concern (the CHECK is never
    // run against it) — the scoping law the door obeys, held here too.
    if !convention.matches_path(&row.path) {
        return base(Fidelity::Grey, Verdict::OutOfScope);
    }

    // The git-anchor JOIN: the commit that appended this anchor gives the AFTER
    // tree; its parent gives the BEFORE tree.
    let (before, after) = match anchor_commit.get(&row.anchor) {
        Some(commit) => (
            git_show(workspace, &format!("{commit}^"), &row.path),
            git_show(workspace, commit, &row.path),
        ),
        None => (None, None),
    };

    let fidelity = match (before.is_some(), after.is_some()) {
        (true, true) => Fidelity::FullBytes,
        (true, false) | (false, true) => Fidelity::Structural,
        (false, false) => return base(Fidelity::Grey, Verdict::Grey),
    };

    // Rebuild both states through the fixture doc-building path (the absent side of
    // a create/remove is the empty document).
    let before_doc = build_doc(&row.path, before.unwrap_or_default());
    let after_doc = build_doc(&row.path, after.unwrap_or_default());
    let no_edges = |_: &str| -> Option<(String, Document)> { None };
    let edits: &[Edit] = &[];
    let decls: &[EdgeDecl] = &[];
    let change = derive_change(
        &before_doc,
        &after_doc,
        edits,
        Invocation {
            op: op_of(&row.op),
            actor: row.actor.as_deref(),
            force: false,
        },
        decls,
        &no_edges,
    );

    match convention.check_change(&change) {
        Err(e) => base(
            fidelity,
            Verdict::Error {
                detail: e.to_string(),
            },
        ),
        Ok(outcome) => {
            let Some(first) = outcome.refusals.first() else {
                return base(fidelity, Verdict::Pass);
            };
            let message = first.message.clone();
            match golden.get(&row.anchor) {
                Some(reason) => base(
                    fidelity,
                    Verdict::Declared {
                        reason: reason.clone(),
                        message,
                    },
                ),
                None => base(fidelity, Verdict::Undeclared { message }),
            }
        }
    }
}

/// Map a journal op string to the change op the derivation records. An unknown op
/// falls back to `splice` (the op only sets `change.op`; the CHECK reads facts).
fn op_of(op: &str) -> ChangeOp {
    match op {
        "create" => ChangeOp::Create,
        "remove" => ChangeOp::Remove,
        _ => ChangeOp::Splice,
    }
}

/// Build a `Document` from raw bytes through the fixture doc-building path,
/// stamping the path (`model::build` leaves it empty; the disk edge sets it).
fn build_doc(path: &str, raw: String) -> Document {
    let nodes = syntax::parse(&raw);
    let mut doc = model::build(raw, nodes);
    if let NodeKind::Document { path: p, .. } = &mut doc.root.kind {
        *p = path.to_string();
    }
    doc
}

// ── the git-anchor JOIN ──────────────────────────────────────────────────────

/// Build the anchor → appending-commit map by walking the journal's git history
/// oldest→newest: the FIRST commit an anchor appears in is the commit that
/// appended its row. A row whose anchor is in no commit (uncommitted, or hand
/// appended) simply does not appear — its reconstruction fails closed to grey.
///
/// # Errors
/// git is unavailable, or the path is not a repository (`log` fails).
fn anchor_commits(workspace: &Path, journal_rel: &str) -> Result<BTreeMap<String, String>, Fail> {
    let list = git_text(
        workspace,
        &[
            "log",
            "--reverse",
            "--first-parent",
            "--format=%H",
            "--",
            journal_rel,
        ],
    )?;
    let mut map = BTreeMap::new();
    for commit in list.lines().filter(|l| !l.is_empty()) {
        let Some(text) = git_show(workspace, commit, journal_rel) else {
            continue;
        };
        for parsed in receipt::journal::parse_rows(&text) {
            map.entry(parsed.anchor)
                .or_insert_with(|| commit.to_owned());
        }
    }
    Ok(map)
}

/// Read one blob's UTF-8 text at `rev:path`. `None` when the path is absent from
/// the tree, the rev does not resolve (a root commit's `^`), or the bytes are not
/// UTF-8 — every "cannot recover this side" collapses to `None`, which the caller
/// folds into the fidelity class.
fn git_show(workspace: &Path, rev: &str, path: &str) -> Option<String> {
    let out = run_git(workspace, &["show", &format!("{rev}:{path}")]).ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

/// Run a git command that must succeed and yields UTF-8 text.
fn git_text(workspace: &Path, args: &[&str]) -> Result<String, Fail> {
    let out = run_git(workspace, args)?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(Fail::tool(format!(
            "git {} failed: {}",
            args.first().copied().unwrap_or(""),
            stderr.trim()
        )));
    }
    String::from_utf8(out.stdout).map_err(|e| Fail::tool(format!("git emitted non-UTF-8: {e}")))
}

/// Run `git -C workspace <args>`, capturing the output.
fn run_git(workspace: &Path, args: &[&str]) -> Result<std::process::Output, Fail> {
    Command::new("git")
        .arg("-C")
        .arg(workspace)
        .args(args)
        .output()
        .map_err(|e| Fail::tool(format!("cannot run git: {e}")))
}

// ── the convention folder + the golden list ──────────────────────────────────

/// A disk-backed [`ConventionFiles`] rooted at `conventions/<slug>/` — the loader
/// stays I/O-free, so the CLI injects the file access here at the disk edge.
struct DiskConventionFiles {
    root: PathBuf,
}

impl ConventionFiles for DiskConventionFiles {
    fn read(&self, rel_path: &str) -> std::io::Result<String> {
        std::fs::read_to_string(self.root.join(rel_path))
    }

    fn exists(&self, rel_path: &str) -> bool {
        self.root.join(rel_path).exists()
    }
}

/// Read + parse `conventions/<slug>/GOLDEN.md` into an `anchor → reason` map. An
/// absent file is the empty map (nothing declared yet).
///
/// # Errors
/// The file exists but a declared exception row carries no reason — every
/// exception must carry a declared reason (rulings § the golden-list mechanism).
fn load_golden(conv_root: &Path) -> Result<BTreeMap<String, String>, Fail> {
    let path = conv_root.join(GOLDEN_FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => {
            return Err(Fail::tool(format!("cannot read {GOLDEN_FILE}: {e}")));
        }
    };
    parse_golden(&text).map_err(Fail::tool)
}

/// Parse a golden page into `anchor → reason`. An exception row is a list item
/// (`- `) carrying an `item=<anchor>` token; every other line (prose, headings,
/// frontmatter) is skipped. A row with an `item=` but no `reason="…"` is a
/// malformed golden list — a declared exception must state why.
fn parse_golden(text: &str) -> Result<BTreeMap<String, String>, String> {
    let mut map = BTreeMap::new();
    for raw in text.lines() {
        let line = raw.trim();
        let Some(body) = line.strip_prefix("- ") else {
            continue;
        };
        let Some(anchor) = body
            .split_whitespace()
            .find_map(|t| t.strip_prefix("item="))
        else {
            continue;
        };
        let reason = extract_reason(body).ok_or_else(|| {
            format!(
                "golden exception `item={anchor}` carries no `reason=\"…\"` — every declared \
                 exception must state why"
            )
        })?;
        map.insert(anchor.to_owned(), reason);
    }
    Ok(map)
}

/// Extract the `reason="…"` value from a golden row (the first double-quoted run
/// after `reason=`). `None` when the row declares no reason.
fn extract_reason(body: &str) -> Option<String> {
    let after = body.split_once("reason=\"")?.1;
    let end = after.find('"')?;
    Some(after[..end].to_owned())
}

// ── report ───────────────────────────────────────────────────────────────────

/// The finished history report.
struct HistoryReport {
    slug: String,
    /// The journal span the run covered (first .. last row anchor), or `None` for
    /// an empty journal.
    span: Option<(String, String)>,
    total_rows: usize,
    out_of_scope: usize,
    full_bytes: usize,
    structural: usize,
    grey: usize,
    passed: usize,
    declared: usize,
    undeclared: usize,
    rows: Vec<RowResult>,
    /// CHECK faults on real history (each collapses the run to exit 2).
    errors: Vec<String>,
}

impl HistoryReport {
    fn assemble(slug: &str, rows: &[receipt::journal::ParsedRow], results: Vec<RowResult>) -> Self {
        let span = match (rows.first(), rows.last()) {
            (Some(f), Some(l)) => Some((f.anchor.clone(), l.anchor.clone())),
            _ => None,
        };
        let mut report = HistoryReport {
            slug: slug.to_owned(),
            span,
            total_rows: rows.len(),
            out_of_scope: 0,
            full_bytes: 0,
            structural: 0,
            grey: 0,
            passed: 0,
            declared: 0,
            undeclared: 0,
            rows: results,
            errors: Vec::new(),
        };
        for r in &report.rows {
            // The verdict drives the pass/declared/undeclared/error tallies; grey is
            // counted from the fidelity pass below (a grey verdict IS an in-scope
            // grey fidelity — counting it here too would double it).
            match &r.verdict {
                Verdict::OutOfScope | Verdict::Grey => {}
                Verdict::Pass => report.passed += 1,
                Verdict::Declared { .. } => report.declared += 1,
                Verdict::Undeclared { .. } => report.undeclared += 1,
                Verdict::Error { detail } => {
                    report.errors.push(format!("`{}`: {detail}", r.anchor));
                }
            }
            if matches!(r.verdict, Verdict::OutOfScope) {
                report.out_of_scope += 1;
                continue;
            }
            // Fidelity counts cover only rows the convention owns (in scope). An
            // out-of-scope row is not this convention's history.
            match r.fidelity {
                Fidelity::FullBytes => report.full_bytes += 1,
                Fidelity::Structural => report.structural += 1,
                Fidelity::Grey => report.grey += 1,
            }
        }
        report
    }

    fn to_human(&self) -> String {
        let mut s = String::new();
        let _ = writeln!(s, "# mrd test --history — {}\n", self.slug);

        let span = match &self.span {
            Some((f, l)) => format!("^{f}..^{l}"),
            None => "(empty journal)".to_owned(),
        };
        let in_scope = self.total_rows - self.out_of_scope;
        let _ = writeln!(
            s,
            "journal span: {span}  ({} row(s), {in_scope} in scope, {} out of scope)",
            self.total_rows, self.out_of_scope
        );
        let _ = writeln!(
            s,
            "fidelity: B full-bytes={}  A structural={}  C grey={}\n",
            self.full_bytes, self.structural, self.grey
        );

        s.push_str("| item | path | op | fidelity | check |\n");
        s.push_str("|------|------|----|----------|-------|\n");
        for r in &self.rows {
            let (fidelity, check) = match &r.verdict {
                Verdict::OutOfScope => ("—".to_owned(), "out of scope".to_owned()),
                Verdict::Grey => (
                    r.fidelity.label().to_owned(),
                    "grey (not reconstructable)".to_owned(),
                ),
                Verdict::Pass => (r.fidelity.label().to_owned(), "pass".to_owned()),
                Verdict::Declared { .. } => (
                    r.fidelity.label().to_owned(),
                    "would-refuse — declared".to_owned(),
                ),
                Verdict::Undeclared { .. } => (
                    r.fidelity.label().to_owned(),
                    "would-refuse — UNDECLARED".to_owned(),
                ),
                Verdict::Error { .. } => (r.fidelity.label().to_owned(), "CHECK ERROR".to_owned()),
            };
            let _ = writeln!(
                s,
                "| `{}` | {} | {} | {fidelity} | {check} |",
                r.anchor, r.path, r.op
            );
        }
        s.push('\n');

        // Detail lines: declared reasons, undeclared findings, and CHECK errors.
        for r in &self.rows {
            match &r.verdict {
                Verdict::Declared { reason, message } => {
                    let _ = writeln!(
                        s,
                        "- declared `{}` ({}): {message} — reason: \"{reason}\"",
                        r.anchor, r.path
                    );
                }
                Verdict::Undeclared { message } => {
                    let _ = writeln!(
                        s,
                        "- **UNDECLARED would-refuse** `{}` ({}): {message} — absent from \
                         {GOLDEN_FILE}; declare it with a reason, or fix the history",
                        r.anchor, r.path
                    );
                }
                Verdict::Error { detail } => {
                    let _ = writeln!(s, "- **CHECK ERROR** `{}` ({}): {detail}", r.anchor, r.path);
                }
                _ => {}
            }
        }

        let _ = write!(
            s,
            "\n{} row(s): {} passed, {} declared, {} UNDECLARED would-refuse, {} grey, {} out of scope.\n",
            self.total_rows,
            self.passed,
            self.declared,
            self.undeclared,
            self.grey,
            self.out_of_scope,
        );
        s
    }

    fn to_json(&self) -> String {
        let rows: Vec<Value> = self
            .rows
            .iter()
            .map(|r| {
                let (check, reason, message) = match &r.verdict {
                    Verdict::OutOfScope => ("out_of_scope", None, None),
                    Verdict::Grey => ("grey", None, None),
                    Verdict::Pass => ("pass", None, None),
                    Verdict::Declared { reason, message } => (
                        "would_refuse_declared",
                        Some(reason.clone()),
                        Some(message.clone()),
                    ),
                    Verdict::Undeclared { message } => {
                        ("would_refuse_undeclared", None, Some(message.clone()))
                    }
                    Verdict::Error { detail } => ("check_error", None, Some(detail.clone())),
                };
                json!({
                    "item": r.anchor,
                    "path": r.path,
                    "op": r.op,
                    "fidelity": r.fidelity.label(),
                    "check": check,
                    "reason": reason,
                    "message": message,
                })
            })
            .collect();
        let span = self
            .span
            .as_ref()
            .map(|(f, l)| json!({ "first": f, "last": l }));
        let value = json!({
            "convention": self.slug,
            "journal_span": span,
            "rows": rows,
            "fidelity": {
                "full_bytes": self.full_bytes,
                "structural": self.structural,
                "grey": self.grey,
            },
            "summary": {
                "total": self.total_rows,
                "in_scope": self.total_rows - self.out_of_scope,
                "out_of_scope": self.out_of_scope,
                "passed": self.passed,
                "declared": self.declared,
                "undeclared": self.undeclared,
                "grey": self.grey,
                "errors": self.errors.len(),
            },
        });
        serde_json::to_string_pretty(&value).expect("json")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn op_of_maps_the_three_ops_and_defaults_to_splice() {
        assert_eq!(op_of("create"), ChangeOp::Create);
        assert_eq!(op_of("remove"), ChangeOp::Remove);
        assert_eq!(op_of("splice"), ChangeOp::Splice);
        assert_eq!(op_of("weird"), ChangeOp::Splice);
    }

    #[test]
    fn parse_golden_reads_item_and_reason() {
        let page = "\
---
convention: reviewer-not-owner
---

# Golden list

Prose bullets without item= are skipped:
- just a note, not an exception

- item=r-000002 reason=\"legacy self-close predates the rule\"
- item=r-000007 reason=\"migration batch, reviewer signed off out of band\"
";
        let map = parse_golden(page).unwrap();
        assert_eq!(map.len(), 2);
        assert_eq!(
            map.get("r-000002").map(String::as_str),
            Some("legacy self-close predates the rule")
        );
        assert_eq!(
            map.get("r-000007").map(String::as_str),
            Some("migration batch, reviewer signed off out of band")
        );
    }

    #[test]
    fn parse_golden_rejects_an_exception_with_no_reason() {
        let page = "- item=r-000002 no reason here\n";
        let err = parse_golden(page).unwrap_err();
        assert!(err.contains("r-000002"), "names the item: {err}");
        assert!(err.contains("reason"), "names the missing reason: {err}");
    }

    #[test]
    fn extract_reason_reads_the_first_quoted_run() {
        assert_eq!(
            extract_reason("item=r-1 reason=\"a b c\" trailing").as_deref(),
            Some("a b c")
        );
        assert_eq!(extract_reason("item=r-1 no reason"), None);
    }
}
