//! `mrd test --history` — the HISTORY runner (U1.6): calibrate a rule PAGE
//! against a workspace's own past, with a golden list of declared exceptions as
//! the gate.
//!
//! # What the tier does (rulings § test --history)
//! The receipt journal ([`fs::domain::RESERVED_JOURNAL_PATH`]) is the append-only
//! ledger of every guarded write; git is the witness that carries the actual
//! bytes. This tier JOINs the two: for each journal row it finds the git commit
//! that appended the row's `^r-NNNNNN` anchor (the **git-anchor JOIN**), reads the
//! written path at that commit (the AFTER bytes) and at its parent (the BEFORE
//! bytes), rebuilds both `Document`s through the fixture doc-building path
//! (`syntax::parse` → `model::build`), derives the [`rulepack-api@2`](policy)
//! change, and runs the rule's `check_change` over it — the SAME registration,
//! loader and full-`EvalLimits` evaluator the door uses
//! ([`policy::register_page`], [`policy::load_rule`],
//! [`policy::Rule::check_change`]).
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
//! # The golden list (D2a — a fenced block of a spec page that names the rule)
//! The golden list lives in a fenced `golden` block of a SPEC page, which is the
//! corpus tier's D2 fixture shape. That page is NAMED with `--spec`; it is never
//! derived from the rule's own path. The spec declares which rule it excepts
//! through a `rule:` frontmatter reference resolved relative to the spec's own
//! directory — the corpus tier's structural confinement, preserved — and a
//! reference that does not resolve to the calibrated page is a malformed spec
//! (exit 2). The join is checked, never assumed.
//!
//! There is no filename axis. A `<page>.golden.md` sibling would carry a
//! semantic relationship in a filename suffix and make reading it a heuristic
//! search near the page, which the corpus-tier ruling forbids verbatim. A spec
//! page carries no registration tag, so it registers nothing by construction
//! (§1 is tag-opt-in) and no exclusion rule is owed for it.
//!
//! Each row declares one would-refuse item by its journal anchor plus a reason.
//! Triage = editing that page through the ordinary write door; exceptions are
//! declared, never erased. `test --history` FAILS (exit 1) on any would-refuse
//! item ABSENT from the list; a declared item passes with its reason rendered.
//! No `--spec` at all means nothing is declared yet — the empty list.
//!
//! # Output + exit codes (§4 preamble law, `docs/status.md`)
//! JSON under `--json`, a human table otherwise. Exit 0 (every would-refuse item
//! is declared), 1 (an undeclared would-refuse item — a finding), 2 (a tool
//! failure: bad usage, an unreadable workspace / rule page / journal, a git
//! failure, or a CHECK that faulted on real history).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use model::{Document, Edit, NodeKind};
use policy::{
    ChangeOp, CheckLimits, EdgeDecl, Invocation, PageRef, Rule, ScopeLayer, derive_change,
    load_rule, register_page,
};
use serde_json::{Value, json};

use crate::test_cmd::{confine, parse_frontmatter, scan_blocks};
use crate::{Fail, Format, current_dir};

/// The fence a spec page keeps its golden list in (D2a).
const GOLDEN_FENCE: &str = "golden";

/// Run `mrd test --history WORKSPACE --rule PAGE [--spec PAGE] [--json]`.
///
/// # Errors
/// [`Fail`] — exit 2 (bad usage, an unreadable workspace / rule page / journal /
/// spec page, a spec whose `rule:` names another page, a git failure, or a
/// faulting CHECK) or exit 1 (an undeclared would-refuse item).
pub(crate) fn dispatch(args: &[String]) -> Result<(), Fail> {
    let mut workspace: Option<String> = None;
    let mut rule: Option<String> = None;
    let mut spec: Option<String> = None;
    let mut json = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--history" => {}
            "--json" => json = true,
            "--rule" => {
                i += 1;
                rule = Some(
                    args.get(i)
                        .cloned()
                        .ok_or_else(|| Fail::tool("--rule needs a PAGE path".to_owned()))?,
                );
            }
            "--spec" => {
                i += 1;
                spec = Some(
                    args.get(i)
                        .cloned()
                        .ok_or_else(|| Fail::tool("--spec needs a PAGE path".to_owned()))?,
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
    let rule = rule.ok_or_else(|| Fail::tool("test --history needs --rule PAGE".to_owned()))?;
    let format = if json { Format::Json } else { Format::Human };

    let report = run_history(Path::new(&workspace), &rule, spec.as_deref())?;
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
            "{} would-refuse item(s) undeclared in the `{GOLDEN_FENCE}` list of {}",
            report.undeclared,
            report.golden_source(),
        )));
    }
    Ok(())
}

/// Load the rule page, read the golden list from the named spec, parse the
/// journal, JOIN each row against git, run the CHECK, and fold the outcomes into
/// a report.
fn run_history(
    workspace_arg: &Path,
    page: &str,
    spec: Option<&str>,
) -> Result<HistoryReport, Fail> {
    let workspace = resolve_workspace(workspace_arg)?;

    // 1. Load the rule from its in-tree PAGE, through the registration + load pair
    //    the door uses. The page path is workspace-relative and mount-confined:
    //    `--rule` names a page inside the workspace being calibrated, never one
    //    outside it.
    let rel = confine(page).map_err(|message| Fail::tool(format!("--rule {page:?}: {message}")))?;
    let page_abs = workspace.join(&rel);
    let bytes = std::fs::read_to_string(&page_abs).map_err(|e| {
        Fail::tool(format!(
            "no readable rule page at {page} under {}: {e}",
            workspace.display()
        ))
    })?;
    let registration = register_page(PageRef {
        layer: ScopeLayer::Workspace,
        page,
        bytes: &bytes,
    })
    .map_err(|e| Fail::tool(format!("rule page `{page}` is refused: {e}")))?
    .ok_or_else(|| {
        Fail::tool(format!(
            "`{page}` carries no `rules/*` registration tag — the history tier calibrates a \
             rule PAGE, and a page registers by tag"
        ))
    })?;
    let id = registration.id().to_string();
    let rule = load_rule(&registration, &bytes, CheckLimits::default())
        .map_err(|e| Fail::tool(format!("cannot load rule page `{page}`: {e}")))?;

    // 2. The golden list of declared exceptions, from the spec page that names
    //    this rule (no `--spec` ⇒ nothing declared yet).
    let golden = load_golden(&workspace, spec, page)?;

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
        results.push(process_row(&workspace, &rule, &golden, &anchor_commit, row));
    }

    let mut report = HistoryReport::assemble(&id, page, &rows, results);
    report.archived = archived_boundary(&workspace, &rows);
    report.golden_spec = spec.map(str::to_owned);
    Ok(report)
}

/// The genesis boundary (G2): what this tier did NOT calibrate over.
///
/// A journal that opens with an `op=genesis` row is a POST-RESET journal, and
/// that row's `path` is the archive holding everything before it. Reading it
/// here costs one file read and turns a silent truncation into a stated one:
/// the tier reports a number, and a number without its population is the
/// failure this lane keeps re-learning.
///
/// This CONSUMES the pointer G2 records rather than scanning for archive-shaped
/// filenames — the row is the authority on where the rows went.
///
/// Note what this is NOT: traversal. The rows in the archive are not
/// calibrated, because their git-anchor JOIN would have to target the path they
/// were APPENDED to, not the path they now live in. That is its own card.
fn archived_boundary(
    workspace: &Path,
    rows: &[receipt::journal::ParsedRow],
) -> Option<(String, usize)> {
    let first = rows.first()?;
    if first.op != "genesis" {
        return None;
    }
    let archive_rel = first.path.clone();
    let text = std::fs::read_to_string(workspace.join(&archive_rel)).ok()?;
    Some((archive_rel, receipt::journal::parse_rows(&text).len()))
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
    /// Out of the rule's `paths:` scope — not its concern, never run.
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

/// Reconstruct one journal row and run the rule over it.
fn process_row(
    workspace: &Path,
    rule: &Rule,
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

    // A row outside the rule's scope is not its concern (the CHECK is never run
    // against it) — the scoping law the door obeys, held here too.
    if !rule.matches_path(&row.path) {
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

    match rule.check_change(&change) {
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
    // The tier's OWN precondition, stated here rather than inside `git_text`:
    // the helper is generic and cannot know what the caller wanted git for, so a
    // bubbled-up `git log failed: …` names the mechanism that failed and never
    // the requirement that was not met (issue-19). Only this caller knows the
    // history tier needs committed history, so only this caller can say it.
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
    )
    .map_err(|e| {
        Fail::tool(format!(
            "the history tier replays COMMITTED changes, and this workspace could not \
             supply them: {}. Nothing was replayed — no rule ran and no golden list was \
             compared. Fix: commit the tree (the receipt journal `{journal_rel}` must be \
             in git history), then re-run.",
            e.message
        ))
    })?;
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

// ── the golden list ──────────────────────────────────────────────────────────

/// Resolve a spec's `rule:` reference against the spec's own directory, the way
/// a corpus spec resolves its rule page (D1 — the structural confinement is a
/// feature, so a spec can only name what its own directory can reach).
///
/// The result is workspace-relative and lexically normalized: `.` segments drop,
/// `..` pops. A reference that pops past the workspace root escapes the mount and
/// is refused rather than clamped — a spec that reaches outside the workspace is
/// naming a page this tier cannot calibrate.
///
/// # Errors
/// The reference escapes the workspace root, or resolves to nothing.
fn resolve_page_ref(spec_rel: &str, spelled: &str) -> Result<String, String> {
    let mut segs: Vec<&str> = spec_rel.split('/').collect();
    segs.pop(); // the spec's own filename — references resolve from its DIRECTORY
    for seg in spelled.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if segs.pop().is_none() {
                    return Err(format!(
                        "`rule: {spelled}` escapes the workspace root from spec `{spec_rel}`"
                    ));
                }
            }
            other => segs.push(other),
        }
    }
    if segs.is_empty() {
        return Err(format!(
            "`rule: {spelled}` from spec `{spec_rel}` resolves to no page"
        ));
    }
    Ok(segs.join("/"))
}

/// Read the spec page named by `--spec` and parse its `golden` fence into an
/// `anchor → reason` map. No spec at all is the empty map (nothing declared yet).
///
/// The spec must name the calibrated rule through its `rule:` frontmatter
/// reference. That check is the whole point of the D2a shape: the relationship is
/// DECLARED in the page, not inferred from where the page sits, so a spec pointed
/// at the wrong rule fails loudly instead of silently excusing another law's
/// findings.
///
/// # Errors
/// The spec path escapes the mount, is unreadable, declares no `rule:`, names a
/// page other than the one under calibration, or carries an exception row with no
/// declared reason.
fn load_golden(
    workspace: &Path,
    spec: Option<&str>,
    page: &str,
) -> Result<BTreeMap<String, String>, Fail> {
    let Some(spec) = spec else {
        return Ok(BTreeMap::new());
    };
    let rel = confine(spec)
        .map_err(|message| Fail::tool(format!("--spec {spec:?}: {message}")))?
        .to_string_lossy()
        .into_owned();
    let text = std::fs::read_to_string(workspace.join(&rel)).map_err(|e| {
        Fail::tool(format!(
            "no readable golden spec at {rel} under {}: {e}",
            workspace.display()
        ))
    })?;

    // The declared join: the spec says which rule it excepts, and it must be this
    // one. An absent `rule:` is as malformed as a wrong one — an unattributed
    // golden list excuses findings for a law it never named.
    let spelled = parse_frontmatter(&text)
        .get("rule")
        .filter(|value| !value.is_empty())
        .cloned()
        .ok_or_else(|| {
            Fail::tool(format!(
                "golden spec `{rel}` declares no `rule:` — a golden list names the rule it \
                 excepts"
            ))
        })?;
    let referenced = resolve_page_ref(&rel, &spelled)
        .map_err(|message| Fail::tool(format!("golden spec `{rel}`: {message}")))?;
    if referenced != page {
        return Err(Fail::tool(format!(
            "golden spec `{rel}` declares `rule: {spelled}` (→ `{referenced}`), but this run \
             calibrates `{page}` — a golden list excepts the rule it names"
        )));
    }

    parse_golden(&text).map_err(Fail::tool)
}

/// Parse a spec page's `golden` fence into `anchor → reason`. An exception row is
/// a list item (`- `) carrying an `item=<anchor>` token; every other line (prose,
/// headings) is skipped. A row with an `item=` but no `reason="…"` is a malformed
/// golden list — a declared exception must state why.
///
/// Only the fence is read. A row in the page body outside it is prose, not a
/// declaration: the ruled home is the fenced block, so an operator who writes an
/// exception in the wrong place gets an undeclared finding, never a silent excuse.
fn parse_golden(text: &str) -> Result<BTreeMap<String, String>, String> {
    let mut map = BTreeMap::new();
    let fenced = scan_blocks(text)
        .into_iter()
        .filter(|(info, _)| info.split_whitespace().next() == Some(GOLDEN_FENCE))
        .map(|(_, body)| body)
        .collect::<Vec<_>>()
        .join("\n");
    for raw in fenced.lines() {
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
    /// The rule's `id:` — its identity, and what every report header names.
    id: String,
    /// The page the rule was loaded from — its provenance, kept beside the id
    /// because a reader who has to go fix the law needs the file, not the name.
    page: String,
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
    /// The genesis boundary (G2): `(archive path, rows it holds)` when this
    /// journal opens with a genesis row. `None` means no reset has happened —
    /// never "the archive is empty".
    archived: Option<(String, usize)>,
    /// The spec page the golden list was read from, or `None` when the run
    /// declared no `--spec`. Reported rather than derived: a reader who has to go
    /// declare an exception needs the page the run actually read.
    golden_spec: Option<String>,
}

impl HistoryReport {
    /// How the report names where an exception would be declared. Without a
    /// `--spec` there is no page to name, and saying so is the honest report: the
    /// operator's next move is to write the spec, not to edit a file we invented
    /// a path for.
    fn golden_source(&self) -> String {
        match &self.golden_spec {
            Some(spec) => format!("golden spec `{spec}`"),
            None => "no golden spec (`--spec` was not given)".to_owned(),
        }
    }

    fn assemble(
        id: &str,
        page: &str,
        rows: &[receipt::journal::ParsedRow],
        results: Vec<RowResult>,
    ) -> Self {
        let span = match (rows.first(), rows.last()) {
            (Some(f), Some(l)) => Some((f.anchor.clone(), l.anchor.clone())),
            _ => None,
        };
        let mut report = HistoryReport {
            id: id.to_owned(),
            page: page.to_owned(),
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
            // Filled by the caller, which holds the workspace path the archive
            // is read from (this fold is path-free by construction).
            archived: None,
            // Filled by the caller, which holds the invocation's `--spec`.
            golden_spec: None,
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
            // Fidelity counts cover only rows the rule owns (in scope). An
            // out-of-scope row is not this rule's history.
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
        let _ = writeln!(s, "# mrd test --history — {} ({})\n", self.id, self.page);

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
        if let Some((archive, count)) = &self.archived {
            let _ = writeln!(
                s,
                "not calibrated: {count} row(s) predate a genesis reset and live in {archive} \
                 — this run did not traverse them"
            );
        }
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
                        "- **UNDECLARED would-refuse** `{anchor}` ({path}): {message} — absent \
                         from the `{GOLDEN_FENCE}` fence of {golden}; declare it with a reason, \
                         or fix the history",
                        anchor = r.anchor,
                        path = r.path,
                        golden = self.golden_source(),
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
            "rule": self.id,
            "rule_page": self.page,
            // The spec the golden list came from — `null` when the run declared
            // no `--spec`, so a consumer reads "nothing declared" as the absence
            // of a list rather than as an empty one.
            "golden_spec": self.golden_spec,
            "journal_span": span,
            // The genesis boundary (G2) — absent when no reset has happened,
            // never an empty object, so a consumer cannot read "no archive" as
            // "an archive with nothing in it".
            "not_calibrated": self.archived.as_ref().map(|(archive, rows)| json!({
                "archive": archive,
                "rows": rows,
            })),
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
rule: ../rules/reviewer-not-owner.md
---

# Golden list

Prose bullets without item= are skipped:
- just a note, not an exception

```golden
- item=r-000002 reason=\"legacy self-close predates the rule\"
- item=r-000007 reason=\"migration batch, reviewer signed off out of band\"
```
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
        let page = "```golden\n- item=r-000002 no reason here\n```\n";
        let err = parse_golden(page).unwrap_err();
        assert!(err.contains("r-000002"), "names the item: {err}");
        assert!(err.contains("reason"), "names the missing reason: {err}");
    }

    /// The fence is the home (D2a). A row written in the page BODY is prose, and
    /// prose does not excuse a finding — otherwise the ruled home would be
    /// decorative and an exception could be declared anywhere on the page.
    #[test]
    fn a_row_outside_the_golden_fence_declares_nothing() {
        let page = "\
---
rule: ../rules/reviewer-not-owner.md
---

- item=r-000002 reason=\"written in the body, not the fence\"

```golden
- item=r-000007 reason=\"the declared one\"
```
";
        let map = parse_golden(page).unwrap();
        assert_eq!(map.len(), 1, "only the fenced row declares: {map:?}");
        assert!(map.contains_key("r-000007"));
        assert!(
            !map.contains_key("r-000002"),
            "a body row is prose, not a declaration"
        );
    }

    /// A spec's `rule:` resolves from the SPEC's directory, never the workspace
    /// root — the corpus tier's structural confinement (D1), preserved.
    #[test]
    fn a_spec_reference_resolves_from_the_spec_directory() {
        assert_eq!(
            resolve_page_ref(
                "specs/reviewer-not-owner.md",
                "../rules/reviewer-not-owner.md"
            )
            .unwrap(),
            "rules/reviewer-not-owner.md"
        );
        assert_eq!(
            resolve_page_ref("teams/a/specs/close.md", "../rules/close.md").unwrap(),
            "teams/a/rules/close.md",
            "the reference stays inside the team's own subtree"
        );
        assert_eq!(
            resolve_page_ref("specs/x.md", "./sibling.md").unwrap(),
            "specs/sibling.md",
            "a `.` segment drops"
        );
    }

    /// Popping past the workspace root is refused, not clamped: a spec that
    /// reaches outside the workspace names a page this tier cannot calibrate.
    #[test]
    fn a_spec_reference_cannot_escape_the_workspace_root() {
        let err = resolve_page_ref("specs/x.md", "../../elsewhere/rules/r.md").unwrap_err();
        assert!(err.contains("escapes"), "names the escape: {err}");
    }

    #[test]
    fn extract_reason_reads_the_first_quoted_run() {
        assert_eq!(
            extract_reason("item=r-1 reason=\"a b c\" trailing").as_deref(),
            Some("a b c")
        );
        assert_eq!(extract_reason("item=r-1 no reason"), None);
    }

    /// G2 boundary: a post-genesis journal states what it did NOT calibrate
    /// over, using the pointer the genesis row records.
    #[test]
    fn the_genesis_boundary_is_read_from_the_pointer() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("meridian")).unwrap();
        std::fs::write(
            tmp.path().join("meridian/arch.md"),
            "- op=splice path=a.md root_before=b3:1 root_after=b3:2 edits=0 ^r-000001\n\
             - op=splice path=b.md root_before=b3:2 root_after=b3:3 edits=0 ^r-000002\n",
        )
        .unwrap();
        let live = "- op=genesis path=meridian/arch.md root_before=b3:3 root_after=b3:4 edits=0 ^r-000001\n";
        let rows = receipt::journal::parse_rows(live);

        let found = archived_boundary(tmp.path(), &rows).expect("the pointer resolves");
        assert_eq!(found, ("meridian/arch.md".to_owned(), 2));
    }

    /// A journal that never had a reset reports NO boundary — absent, never a
    /// zero, so a reader cannot mistake "no archive" for "an empty archive".
    #[test]
    fn a_journal_without_a_genesis_row_has_no_boundary() {
        let tmp = tempfile::tempdir().unwrap();
        let live = "- op=splice path=a.md root_before=b3:1 root_after=b3:2 edits=0 ^r-000001\n";
        let rows = receipt::journal::parse_rows(live);
        assert!(archived_boundary(tmp.path(), &rows).is_none());
    }
}
