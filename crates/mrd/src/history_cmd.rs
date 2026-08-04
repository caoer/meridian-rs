//! `mrd test --history` — the HISTORY runner (U1.6): calibrate a rule PAGE
//! against a workspace's own past, with a golden list of declared exceptions as
//! the gate.
//!
//! # What the tier does (rulings § test --history; ZT 2026-08-03)
//! **Git is the history.** ZT ruled it directly — *"Engine does not have memory.
//! It should not have. History is pin to git when we lock. Anything between locks
//! is not history."* — so the workspace's past is enumerated from `git log
//! --name-status`, which is where it has always actually lived. The receipt
//! journal that used to enumerate it was the engine keeping a memory of its own,
//! and it is gone.
//!
//! One recorded write — one (commit, path) pair — is one row. The commit gives
//! the AFTER bytes, its first parent the BEFORE bytes, and the commit's author
//! and date are carried verbatim as the write's actor and time. From there the
//! tier rebuilds both `Document`s through the fixture doc-building path
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
//! - **C grey** — neither side could be recovered (the path is absent from both
//!   the commit and its parent). A grey row is COUNTED and
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
//! Each row declares one would-refuse item by its ITEM ID plus a reason. An item
//! id is `<commit>:<path>` — the two facts that name one recorded write, both
//! git's, and both stable for as long as the commit is. (It was the journal's
//! `^r-NNNNNN` anchor when the engine kept its own ledger; a golden list written
//! against those anchors names writes nothing can resolve any more, and its rows
//! read as undeclared until they are re-declared against git.)
//! Triage = editing that page through the ordinary write door; exceptions are
//! declared, never erased. `test --history` FAILS (exit 1) on any would-refuse
//! item ABSENT from the list; a declared item passes with its reason rendered.
//! No `--spec` at all means nothing is declared yet — the empty list.
//!
//! # Output + exit codes (§4 preamble law, `docs/status.md`)
//! JSON under `--json`, a human table otherwise. Exit 0 (every would-refuse item
//! is declared), 1 (an undeclared would-refuse item — a finding), 2 (a tool
//! failure: bad usage, an unreadable workspace / rule page / spec page, a git
//! failure, or a CHECK that faulted on real history).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

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
/// [`Fail`] — exit 2 (bad usage, an unreadable workspace / rule page / spec
/// page, a spec whose `rule:` names another page, a git failure, or a faulting
/// CHECK) or exit 1 (an undeclared would-refuse item).
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

/// Load the rule page, read the golden list from the named spec, enumerate the
/// recorded writes off git, run the CHECK, and fold the outcomes into a report.
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

    // 3. The history: every write git recorded, with both sides' bytes.
    let rows = enumerate(&workspace)?;

    // 4. Reconstruct + check each row.
    let mut results = Vec::with_capacity(rows.len());
    for row in &rows {
        results.push(process_row(&rule, &golden, row));
    }

    let mut report = HistoryReport::assemble(&id, page, &rows, results);
    report.golden_spec = spec.map(str::to_owned);
    Ok(report)
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

/// One recorded write, as the enumerator hands it to the evaluator: what git
/// says happened, plus the bytes on each side of it.
///
/// This is the tier's whole coupling to where history lives. Everything below it
/// — the rebuild, the derivation, the CHECK, the verdict, the golden compare —
/// reads these fields and never asks where they came from.
struct HistoryRow {
    /// The item id a golden list declares: `<commit>:<path>`.
    anchor: String,
    /// The path this write landed on, workspace-relative.
    path: String,
    /// The op word (`create` / `splice` / `remove`).
    op: String,
    /// The commit's author, verbatim — git's attribution, not the engine's.
    actor: Option<String>,
    /// The bytes at the commit's first parent, or `None` when that side does not
    /// resolve (a create, a root commit, or non-UTF-8 content).
    before: Option<String>,
    /// The bytes at the commit, or `None` when that side does not resolve (a
    /// remove, or non-UTF-8 content).
    after: Option<String>,
}

/// **The enumerator** — every write git recorded, oldest first, with both sides'
/// bytes recovered.
///
/// Two git calls for the whole walk, never one per commit: one `git log
/// --name-status` for the writes, one `git cat-file --batch` for the 2N sides
/// they need. Both live in `crates/git`, the one auditable shell-out leaf.
fn enumerate(workspace: &Path) -> Result<Vec<HistoryRow>, Fail> {
    let repo = git::Repo::at(workspace);

    // The tier's OWN precondition, stated here rather than left to git's
    // message: the helper cannot know what the caller wanted git for, so a
    // bubbled-up failure names the mechanism and never the requirement that was
    // not met (issue-19). Only this caller knows the history tier needs
    // committed history, so only this caller can say it.
    let changes = repo.path_history(&[]).map_err(|e| {
        Fail::tool(format!(
            "the history tier replays COMMITTED changes, and this workspace could not \
             supply them: {e}. Nothing was replayed — no rule ran and no golden list was \
             compared. Fix: commit the tree, then re-run.",
        ))
    })?;

    // Both sides of every write, in one batched read. The specs are built in
    // pairs so the answers index back onto the walk positionally.
    let mut specs = Vec::with_capacity(changes.len() * 2);
    for change in &changes {
        specs.push(format!("{}^:{}", change.commit, change.path));
        specs.push(format!("{}:{}", change.commit, change.path));
    }
    let refs: Vec<&str> = specs.iter().map(String::as_str).collect();
    let sides = repo
        .blobs_at(&refs)
        .map_err(|e| Fail::tool(format!("cannot read the recorded bytes: {e}")))?;

    // Non-UTF-8 bytes collapse to `None` exactly like an unresolvable side: the
    // tier reconstructs markdown documents, and bytes it cannot read as text are
    // a side it did not recover.
    let text = |side: &Option<Vec<u8>>| -> Option<String> {
        side.clone().and_then(|bytes| String::from_utf8(bytes).ok())
    };

    Ok(changes
        .iter()
        .enumerate()
        .map(|(i, change)| HistoryRow {
            anchor: format!("{}:{}", change.commit, change.path),
            path: change.path.clone(),
            op: change.status.as_str().to_owned(),
            actor: Some(change.author.clone()),
            before: sides.get(i * 2).and_then(text),
            after: sides.get(i * 2 + 1).and_then(text),
        })
        .collect())
}

/// One row's fully-resolved outcome.
struct RowResult {
    anchor: String,
    path: String,
    op: String,
    fidelity: Fidelity,
    verdict: Verdict,
}

/// Reconstruct one recorded write and run the rule over it.
fn process_row(rule: &Rule, golden: &BTreeMap<String, String>, row: &HistoryRow) -> RowResult {
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

    // The two sides the enumerator recovered: the commit's bytes (AFTER) and its
    // first parent's (BEFORE).
    let (before, after) = (row.before.clone(), row.after.clone());

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
    /// The history span the run covered (first .. last item id), or `None` when
    /// git recorded nothing.
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

    fn assemble(id: &str, page: &str, rows: &[HistoryRow], results: Vec<RowResult>) -> Self {
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
            Some((f, l)) => format!("{f} .. {l}"),
            None => "(nothing recorded)".to_owned(),
        };
        let in_scope = self.total_rows - self.out_of_scope;
        let _ = writeln!(
            s,
            "history span: {span}  ({} write(s), {in_scope} in scope, {} out of scope)",
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
            "history_span": span,
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

    /// The enumerator reads the workspace's own git history: one row per
    /// (commit, path), the op word from git's status letter, the actor from the
    /// commit author, and both sides' bytes recovered.
    #[test]
    fn the_enumerator_reads_writes_out_of_git() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let run = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .output()
                .expect("run git");
            assert!(out.status.success(), "git {args:?}: {out:?}");
        };
        run(&["init", "--quiet"]);
        run(&["config", "user.email", "hist@example.test"]);
        run(&["config", "user.name", "History Fixture"]);

        std::fs::write(dir.join("a.md"), "# one\n").unwrap();
        run(&["add", "a.md"]);
        run(&["commit", "--quiet", "-m", "born"]);
        std::fs::write(dir.join("a.md"), "# one\n\nmore\n").unwrap();
        run(&["commit", "--quiet", "-am", "grown"]);
        std::fs::remove_file(dir.join("a.md")).unwrap();
        run(&["commit", "--quiet", "-am", "gone"]);

        let rows = enumerate(dir).expect("the walk reads this repository");
        let ops: Vec<&str> = rows.iter().map(|r| r.op.as_str()).collect();
        assert_eq!(
            ops,
            vec!["create", "splice", "remove"],
            "oldest first, git's own status letters"
        );
        assert!(
            rows.iter()
                .all(|r| r.actor.as_deref() == Some("History Fixture")),
            "the actor is the commit author NAME — the field an owner handle can equal"
        );

        let create = &rows[0];
        assert_eq!(create.before, None, "a create has no before side");
        assert_eq!(create.after.as_deref(), Some("# one\n"));

        let splice = &rows[1];
        assert_eq!(splice.before.as_deref(), Some("# one\n"));
        assert_eq!(splice.after.as_deref(), Some("# one\n\nmore\n"));

        let remove = &rows[2];
        assert_eq!(remove.before.as_deref(), Some("# one\n\nmore\n"));
        assert_eq!(remove.after, None, "a remove has no after side");

        assert!(
            create.anchor.ends_with(":a.md") && create.anchor.len() > 41,
            "the item id is <commit>:<path>: {}",
            create.anchor
        );
    }

    /// A workspace git recorded nothing in enumerates to nothing — an empty
    /// history, never a failure. "Nothing has been committed yet" is a true
    /// answer about the past, and the tier reports over it.
    #[test]
    fn a_repository_with_no_commits_enumerates_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(tmp.path())
            .args(["init", "--quiet"])
            .output()
            .expect("run git");
        assert!(out.status.success());
        assert!(
            enumerate(tmp.path())
                .expect("an empty history is an answer")
                .is_empty()
        );
    }
}
