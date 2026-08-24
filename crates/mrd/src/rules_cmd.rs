//! `mrd rules` — the effective-rules print verb.
//!
//! ```text
//! mrd rules [PATH] [--workspace | --user] [--json]
//! ```
//!
//! Registration by tag plus id-based override makes the effective rule set a
//! computed quantity. This verb shows it: per rule id, the page that governs at
//! PATH, its scope, and the pages it shadows — winner first, shadowed entries
//! visible, never silently collapsed (`git config --show-origin`).
//!
//! # One resolver, two consumers
//! Every judgement rendered here comes from `policy`: [`policy::RuleIndex`]
//! discovers, [`policy::RuleIndex::narrowed_to`] applies the § 3 narrowing,
//! [`policy::RuleIndex::resolve`] decides, and
//! [`policy::armed::ArmedArtifact::verify_at`] answers what is armed and whether
//! it still stands, in one composed call. This module contains no override law:
//! it compares no scopes, groups no ids, and does no depth arithmetic. The scope
//! column renders whatever `policy` computed.
//!
//! # Refusal scoping (§ 3)
//! A scoped query reddens for the refusals on its chain — the exact subtree each
//! refused page would have governed — and not for a stranger's. The narrowing is
//! `policy`'s. A corpus-wide walk reports all refusals, because a walk reads the
//! un-narrowed index.
//!
//! # Read-only
//! The verb reads PATH's § 3 chain of the workspace hash domain (the walk
//! pre-filter [`policy::governing_dirs`] enumerates the only directories whose
//! direct files can mount at-or-above PATH — completeness is that function's
//! contract, and [`policy::RuleIndex::narrowed_to`] still filters page by
//! page), the user rung, and the declined enumerations, and calls pure
//! functions over the bytes: no arm, no receipt, no cap spend, no write path.
//! It does NOT snapshot the whole domain: a full `fs::domain_snapshot` read and
//! digested 37k files / 219 MB per invocation (measured 2026-08-19, ~2.5 CPU-s
//! and 548 MB peak RSS) to answer about a dozen chain pages, and this verb is
//! hook-adjacent — its cost multiplies by every caller.
//!
//! # Registered here vs armed here
//! The chain columns are what discovery found; the `armed=` cell is what the ARM
//! artifact attested. An id may be registered and unarmed (`armed=-`), armed on
//! the page that governs (`armed=armed`), or armed on a different page than the
//! one discovery now resolves (`armed=armed@<page>`) — arming pins resolution,
//! and later discovery never changes it.
//!
//! `armed=` means WHAT GOVERNS HERE and nothing else, so `-` is never asked to
//! carry a cause. An armed row whose arm root does not contain this path governs
//! nothing here and gets its own line, beneath the rows — the same treatment as
//! an armed row whose pinned page left the corpus. Containment is a FACT and
//! reddens nothing: arming a sibling scope is normal. Redness is a FAULT
//! wherever it lives, so it is named on that line and counted in the findings —
//! which is what makes this verb and `mrd status` agree about one artifact
//! instead of one reading clean while the other reads drifted.
//!
//! Exit triad: **0** clean · **1** findings (a collision, a refused rule page,
//! an armed row whose pinned page drifted or vanished, an unreadable armed
//! artifact) · **2** bad invocation or an unreadable workspace.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use policy::armed::{ArmedArtifact, ArmedRow, Drift, PageSource, Redness};
use policy::{Effective, EffectiveSet, PageRef, RuleIndex, ScopeLayer};
use serde_json::{Value, json};

use crate::{Fail, Format, current_dir};

/// The finding leg of the triad: the invocation was well-formed, the law is not.
const EXIT_FINDING: u8 = 1;

/// Which layer of the § 3 ladder the invocation asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum View {
    /// The default: every layer, narrowed to PATH — the rules in EFFECT there.
    Effective,
    /// `--workspace`: the workspace-root layer alone.
    Workspace,
    /// `--user`: the user-space layer alone.
    User,
}

impl View {
    fn label(self) -> &'static str {
        match self {
            View::Effective => "effective",
            View::Workspace => "workspace",
            View::User => "user",
        }
    }

    /// Whether a discovered page's layer belongs in this view.
    fn admits(self, layer: ScopeLayer) -> bool {
        match self {
            View::Effective => true,
            View::Workspace => layer == ScopeLayer::Workspace,
            View::User => layer == ScopeLayer::User,
        }
    }
}

/// Run `mrd rules [PATH] [--workspace|--user] [--json]`. Errors [`Fail`] exit 2 on a bad
/// invocation, a PATH outside the workspace, or an unreadable workspace; exit 1 when the
/// printed law carries a finding.
pub(crate) fn dispatch(args: &[String]) -> Result<(), Fail> {
    let parsed = Rules::parse(args)?;
    let cwd = current_dir()?;
    // The rooted lane (§4.1 colon law), under the 2026-08-18 authority ruling
    // (rooted-refs-everywhere): a head-colon PATH asks what governs at that
    // path IN THE NAMED ROOT — the report reads that tree's rule index (plus
    // the user rung), exactly as if the caller stood there. The rel half is
    // root-relative by definition, so the ambient cwd fitting below never
    // touches it.
    let entered = match parsed.path.as_deref() {
        Some(p) => crate::rooted::enter(p, "rules", "Nothing was reported."),
        None => Ok(None),
    };
    let ambient = || -> Result<PathBuf, Fail> {
        Ok(crate::resolve::resolve_runtime(workspace::Base::Cwd(&cwd))
            .map_err(|e| {
                Fail::tool(format!(
                    "cannot resolve workspace for {}: {e}",
                    cwd.display()
                ))
            })?
            .workspace)
    };
    let (base_workspace, rooted_rel) = match entered {
        Ok(Some((rel, rooted))) => (rooted.workspace, Some(rel)),
        Ok(None) => (ambient()?, None),
        // The refusal frames with the workspace the caller stands in — no
        // target workspace exists to name.
        Err(error) => {
            let ambient = ambient()?;
            return Err(crate::engine::json_refusal(parsed.format, &ambient, &error));
        }
    };
    let workspace = workspace::canonicalize(&base_workspace).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace {} ({e})",
            base_workspace.display()
        ))
    })?;

    // The default view narrows to the PATH argument; a single-layer view narrows
    // to the layer's own root.
    let at = match (parsed.view, rooted_rel) {
        (View::Effective, Some(rel)) => rel,
        (View::Effective, None) => workspace_relative(&workspace, parsed.path.as_deref(), &cwd)?,
        (View::Workspace | View::User, _) => String::new(),
    };

    let report = build(&workspace, &at, parsed.view)?;
    match parsed.format {
        Format::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(&to_json(&workspace, &report)).expect("json")
            );
        }
        Format::Human => print!("{}", render_human(&report)),
    }

    let findings = report.findings();
    if findings.is_empty() {
        Ok(())
    } else {
        Err(Fail::with_code(
            EXIT_FINDING,
            format!("{} in the rule set", findings.join(", ")),
        ))
    }
}

// ── the invocation ────────────────────────────────────────────────────────────

/// The parsed `rules` invocation.
#[derive(Debug)]
struct Rules {
    path: Option<String>,
    view: View,
    format: Format,
}

impl Rules {
    fn parse(args: &[String]) -> Result<Self, Fail> {
        let mut path: Option<String> = None;
        let mut view: Option<View> = None;
        let mut json = false;

        for arg in args {
            let flag = match arg.as_str() {
                "--json" => {
                    json = true;
                    continue;
                }
                "--workspace" => View::Workspace,
                "--user" => View::User,
                other if other.starts_with('-') => {
                    return Err(Fail::tool(format!("unknown flag: {other}")));
                }
                value if path.is_none() => {
                    path = Some(value.to_owned());
                    continue;
                }
                value => return Err(Fail::tool(format!("unexpected argument: {value}"))),
            };
            // Two layer flags name two different single-layer views; picking one
            // silently would answer a question that was not asked.
            if let Some(held) = view {
                return Err(Fail::tool(format!(
                    "--{} and --{} each print ONE layer — pass one",
                    held.label(),
                    flag.label()
                )));
            }
            view = Some(flag);
        }

        Ok(Rules {
            path,
            view: view.unwrap_or(View::Effective),
            format: if json { Format::Json } else { Format::Human },
        })
    }
}

/// The workspace-relative spelling of the PATH argument (default: the cwd). A
/// path outside the workspace, or one that is not on disk, is refused rather
/// than answered empty — an empty rule set is a claim about a real place.
fn workspace_relative(workspace: &Path, path: Option<&str>, cwd: &Path) -> Result<String, Fail> {
    let raw = path.map_or_else(|| cwd.to_path_buf(), PathBuf::from);
    let absolute = if raw.is_absolute() {
        raw
    } else {
        cwd.join(raw)
    };
    let absolute = std::fs::canonicalize(&absolute).map_err(|e| {
        Fail::tool(format!(
            "cannot read {}: {e} — `mrd rules` answers about a folder or page that \
             exists, because an empty rule set is a claim about a real place",
            absolute.display()
        ))
    })?;
    let relative = absolute.strip_prefix(workspace).map_err(|_| {
        Fail::tool(format!(
            "{} is outside the workspace {}",
            absolute.display(),
            workspace.display()
        ))
    })?;
    Ok(relative.to_string_lossy().replace('\\', "/"))
}

// ── the report ────────────────────────────────────────────────────────────────

/// One entry of a rendered override chain.
#[derive(Debug, PartialEq, Eq)]
struct ChainEntry {
    role: &'static str,
    page: String,
    rev: String,
    layer: &'static str,
    depth: usize,
    kinds: String,
}

impl ChainEntry {
    fn of(role: &'static str, registration: &policy::Registration) -> Self {
        let scope = registration.scope();
        ChainEntry {
            role,
            page: registration.page().to_owned(),
            rev: registration.rev().to_owned(),
            layer: scope.layer().as_str(),
            depth: scope.depth(),
            kinds: registration
                .kinds()
                .iter()
                .map(|k| k.as_str())
                .collect::<Vec<_>>()
                .join("+"),
        }
    }
}

/// What the ARM artifact says about one id at this path.
#[derive(Debug, PartialEq, Eq)]
struct ArmedCell {
    mode: String,
    /// The page the arm pinned, when it is not the page discovery resolves.
    elsewhere: Option<String>,
    /// Why the pinned page no longer stands, when it does not.
    redness: Option<String>,
    /// The drift join for this ledger row — see [`DriftCell`].
    drift: DriftCell,
}

impl ArmedCell {
    /// The rendered cell: the mode word, its redness, and the pinned page when
    /// the arm and discovery disagree.
    ///
    /// ⛔ The drift word is NOT in here. `armed=` means what governs and how,
    /// and an `off` row's drift governs nothing — folding it in would spell
    /// `off(off-drifted)`, which reads as a redness and is not one. It is its
    /// own column: [`DriftCell::render`].
    fn render(&self) -> String {
        let mut cell = self.mode.clone();
        if let Some(why) = &self.redness {
            cell.push('(');
            cell.push_str(why);
            cell.push(')');
        }
        if let Some(page) = &self.elsewhere {
            cell.push('@');
            cell.push_str(page);
        }
        cell
    }
}

/// **The drift column** — one per LEDGER ROW: does the pinned page still read at
/// the rev the row was attested at?
///
/// Ruled by advisor `4dab0746` 2026-08-23 02:27 EDT (card
/// `rules-drift-invisible-on-off-rows`): the redness contract STANDS, so
/// [`policy::armed::verify_rows`] keeps answering nothing for an `off` row and
/// `armed=` keeps its vocabulary. The gap it leaves is OBSERVABILITY — a rule
/// flipped off and then edited rendered `armed=off` and nothing else, on a
/// surface that printed the ledger's row count four lines above — and it closes
/// additively, here: every ledger row carries this cell, and the `off` case has
/// its own word, `off-drifted`, which **is not a redness state and trips no
/// gate** ([`RulesReport::findings`] never reads this field).
///
/// The join is [`policy::armed::ArmedArtifact::drift`] — one rev law with the
/// gate, so this column and the `armed=` redness cannot disagree about a row
/// that fires. The reference implementation it folds in is fleet watcher
/// `c38541e3`'s `rules-drift.py`, which joined the ledger's pinned rev against
/// the live winner rev out of `--json`; the machine face now publishes both revs
/// (`pinned_rev`, `live_rev`), so that script's whole reason to exist is gone.
#[derive(Debug, PartialEq, Eq)]
struct DriftCell {
    /// The rev the ledger row attests.
    pinned_rev: String,
    /// The rev the pinned page reads NOW — `None` when it could not be read at
    /// all, which is why [`DriftCell::word`] is not derived from comparing two
    /// `Option`s: unreadable and unchanged are opposite answers.
    live_rev: Option<String>,
    /// The word, or `None` when the join held.
    word: Option<&'static str>,
}

impl DriftCell {
    /// The cell from one row's drift. `off-` prefixes the whole `off` vocabulary
    /// for one reason: a reader who greps `drifted` must not match a row that
    /// gates nothing, and a reader who greps `off-` must find every one of them.
    ///
    /// `off` admits every rule kind ([`policy::armed::Mode::admits`]), so an off
    /// row can never be kind-mismatched — there is no third word to spell.
    fn of(row: &policy::armed::DriftRow) -> Self {
        let fires = row.row().mode().fires();
        let (live_rev, word) = match row.drift() {
            Drift::Clean => (Some(row.row().rev().to_owned()), None),
            Drift::Drifted { report_rev } => (
                Some(report_rev.clone()),
                Some(if fires { "drifted" } else { "off-drifted" }),
            ),
            Drift::Missing { .. } => (None, Some(if fires { "missing" } else { "off-missing" })),
        };
        DriftCell {
            pinned_rev: row.row().rev().to_owned(),
            live_rev,
            word,
        }
    }

    /// The rendered cell. A clean join prints `-` rather than nothing: the whole
    /// defect this column answers is that an absent marker read as "nothing
    /// drifted" when it meant "nobody asked".
    fn render(&self) -> &str {
        self.word.unwrap_or("-")
    }
}

/// One id's row: its chain, and what is armed for it here.
#[derive(Debug, PartialEq, Eq)]
struct RuleRow {
    id: String,
    /// `resolved`, or `collision` — a collided id resolves to nothing.
    state: &'static str,
    /// The collision's resolution scope — (layer, depth) — when this row is
    /// one. Carried as the two facts; each face spells them itself: the human
    /// render as `layer=… depth=…`, the JSON as the composite it always shipped.
    /// Neither face labels them `scope=` — at this surface that word is the
    /// armed artifact's ARM-ROOT column, a directory, and one label carrying
    /// two vocabularies was the measured copy-paste trap.
    collision_scope: Option<(&'static str, usize)>,
    armed: Option<ArmedCell>,
    chain: Vec<ChainEntry>,
}

/// Where the user rung came from, or why there is none.
#[derive(Debug, PartialEq, Eq)]
enum UserScope {
    /// The anchor exists: the user scope, and the anchor that declared it.
    Declared { scope: String, anchor: String },
    /// No user layer, and the reason — never a silent empty.
    Absent { reason: String },
}

/// The state of the attested armed set.
#[derive(Debug, PartialEq, Eq)]
enum ArmedSource {
    /// Parsed: how many rows the artifact carries.
    Present { path: String, rows: usize },
    /// The artifact is not on disk — nothing was ever armed through it.
    Absent { path: String },
    /// The artifact is there and does not parse. Never read as "nothing armed":
    /// a corrupt attestation is a finding, not an absence.
    Unreadable { path: String, detail: String },
}

/// An armed row that the answer is still COUNTING and no longer SHOWING: the
/// artifact pins this id, the selection law picks the row up at this path, and
/// discovery no longer resolves the id at all — so no [`RuleRow`] carries it.
///
/// ⛔ Before this existed the row was dropped on the floor. The header kept
/// printing `armed-set … (N row(s))` while the body printed `(no rules in
/// effect)`, four lines apart, at exit 0 with an empty stderr — an answer
/// contradicting itself with no error. Worse, the redness was COMPUTED and
/// DISCARDED: `verify_at` reddens the row, `reddened` records it, and the
/// armed cell is only ever reached for a row discovery still resolves.
#[derive(Debug, PartialEq, Eq)]
struct ArmedOrphan {
    id: String,
    /// The page the arm attested — a workspace spelling, from the artifact.
    page: String,
    /// The arm root the row was pinned at.
    scope: String,
    mode: String,
    /// Why discovery no longer carries the page, ESTABLISHED rather than
    /// assumed. See [`orphan_cause`].
    cause: &'static str,
    /// This ledger row's drift column. An orphan is a ledger row like any other,
    /// and the ruling says EVERY ledger row carries one.
    drift: DriftCell,
}

/// An armed row for an id this answer RESOLVES whose arm root does not contain
/// the queried path — so it governs nothing here and no [`ArmedCell`] is about
/// it.
///
/// ⛔ Before this existed the row was silent, and `armed=-` spelled two
/// unrelated truths: "no armed row exists for this id anywhere" and "an armed
/// row exists and is armed elsewhere". The header four lines above printed
/// `armed-set … (N row(s))` in both cases, so the reader had a count they could
/// not reconcile with any row. Worse, the row's REDNESS went with it: a drifted
/// arm outside the queried path exited 0 here while `mrd status` exited 1 on the
/// same workspace.
///
/// The cell is NOT where this belongs. `armed=` means WHAT GOVERNS HERE, and a
/// cell carrying two facts is how the silent cause masked the loud one in the
/// first place. This is a sibling of [`ArmedOrphan`] and prints on the same
/// pattern.
#[derive(Debug, PartialEq, Eq)]
struct ArmedElsewhere {
    id: String,
    /// The page the arm attested — a workspace spelling, from the artifact.
    page: String,
    /// The arm root the row was pinned at, which does NOT contain this path.
    scope: String,
    mode: String,
    /// Why the pinned page no longer stands, when it does not. Containment is a
    /// FACT and rides the section header; redness is a FAULT and rides here.
    redness: Option<String>,
    /// This ledger row's drift column. Redness is silent on an `off` row here
    /// for exactly the reason it is silent in the cell — `verify_elsewhere_at`
    /// shares `verify_rows` — so this section needs the column just as much.
    drift: DriftCell,
}

/// Why an armed row's pinned page is absent from the corpus — decided by
/// looking, never by inheriting `verify_at`'s word.
///
/// ⛔ `policy` reddens this row `Missing`, because a [`PageSource`] that cannot
/// serve a page reports exactly that and can report nothing else. **`missing`
/// would be an HONEST STATE AND THE WRONG CAUSE** for the case that motivated
/// this: the page is on disk, at its own path, byte-for-byte unchanged, and
/// only the DECLARED DOMAIN moved. Sending a reader to look for a deleted file
/// costs them the search before they find the ignore rule. The two cases are
/// distinguished by one `stat`, so there is no excuse for minting a cause.
fn orphan_cause(workspace: &Path, page: &str) -> &'static str {
    if Path::new(page)
        .components()
        .all(|c| matches!(c, std::path::Component::Normal(_)))
        && workspace.join(page).is_file()
    {
        "on disk, outside the hash domain — an ignore rule or a dot segment excludes it"
    } else {
        "not on disk"
    }
}

/// Everything the verb prints.
#[derive(Debug, PartialEq, Eq)]
struct RulesReport {
    at: String,
    view: View,
    workspace: String,
    user_scope: UserScope,
    armed: ArmedSource,
    rows: Vec<RuleRow>,
    /// Pages that offered themselves to registration and were refused, narrowed
    /// to this query's path by `policy` exactly as the rules are.
    refused: Vec<String>,
    /// Chain files whose bytes are not UTF-8, so their tags cannot be read.
    /// Chain-scoped like the refusals (§ 3): the enumeration only reads the
    /// pages that could govern this query, so only those can be found broken.
    unreadable: Vec<String>,
    /// Markdown the workspace's CUSTOM-IGNORE rules decline — vault-visible,
    /// operator-declared, so its drop is voiced. Enumerated by
    /// [`fs::declined_markdown`], the projection's own walk law: a dot path is
    /// invisible here exactly as the record projection serves none of it
    /// (dogfood F11) — never from another face's population.
    declined_workspace: Vec<String>,
    /// Markdown under the user `rules/` tree that a dot segment declined. A
    /// SECOND and INDEPENDENT exclusion: this feed never consults the residency
    /// filter, so a workspace ignore rule does not reach it and its own skip
    /// does not reach the workspace.
    declined_user: Vec<String>,
    /// RULE PAGES a dot-prefixed segment keeps out of the workspace hash
    /// domain — the third declined population (card
    /// rules-silent-nonregistration, amending F11 narrowly). Registrar-narrowed
    /// like its two siblings, and CAPPED in prose unlike them: a dot tree holds
    /// archived corpora (the F11 measurement was 16 rule-tagged pages in one
    /// dot-named snapshot dir), so "narrowed" does not bound it. Complete list
    /// on `not_offered.workspace_dot`. Exit-neutral: findings stay
    /// served-corpus conditions.
    declined_dot_workspace: Vec<String>,
    /// Excluded markdown whose rule-ness CANNOT BE ANSWERED, because its
    /// frontmatter does not parse. Neither a dropped rule page nor established
    /// not to be one — so it carries its own verdict string rather than being
    /// folded into either answer.
    undecidable: Vec<String>,
    /// Armed rows this answer counts in its header and shows nowhere.
    armed_orphans: Vec<ArmedOrphan>,
    /// Armed rows this answer counts in its header whose arm root does not
    /// contain the queried path, so they govern nothing here.
    armed_elsewhere: Vec<ArmedElsewhere>,
}

impl RulesReport {
    /// The findings that gate exit 1, named for the diagnostic.
    fn findings(&self) -> Vec<String> {
        let mut findings = Vec::new();
        let collisions = self
            .rows
            .iter()
            .filter(|row| row.state == "collision")
            .count();
        if collisions > 0 {
            findings.push(format!("{collisions} collided id(s)"));
        }
        if !self.refused.is_empty() {
            findings.push(format!("{} refused rule page(s)", self.refused.len()));
        }
        // Redness is a FAULT WHEREVER IT LIVES. Containment is a fact and never
        // reaches this list — arming a sibling scope is normal and must not
        // redden an unrelated query — but a red row does, whether or not its arm
        // root contains this path. That is what makes this verb and `mrd status`
        // agree on one artifact instead of contradicting each other.
        let red = self
            .rows
            .iter()
            .filter(|row| row.armed.as_ref().is_some_and(|a| a.redness.is_some()))
            .count()
            + self
                .armed_elsewhere
                .iter()
                .filter(|row| row.redness.is_some())
                .count();
        if red > 0 {
            findings.push(format!("{red} red armed row(s)"));
        }
        if !self.unreadable.is_empty() {
            findings.push(format!("{} unreadable file(s)", self.unreadable.len()));
        }
        // The published exit contract already promises this leg — `mrd rules
        // --help`: "1 finding (collision | refused rule page | RED ARMED ROW)".
        // An orphan IS a red armed row: `verify_at` reddens it. Counting it here
        // is not a new contract, it is the shipped one being honoured for the
        // first time.
        if !self.armed_orphans.is_empty() {
            findings.push(format!(
                "{} armed row(s) whose pinned page is not in the corpus",
                self.armed_orphans.len()
            ));
        }
        if let ArmedSource::Unreadable { .. } = self.armed {
            findings.push("an unreadable armed set".to_owned());
        }
        findings
    }
}

// ── building it ───────────────────────────────────────────────────────────────

/// The workspace pages that could govern `at`, as `(path, bytes)` — the § 3
/// chain, ENUMERATED rather than walked: the direct files of each
/// [`policy::governing_dirs`] directory, gated by the hash-domain predicate
/// ([`fs::domain::Domain::contains`] — md-only, dot rule, custom ignore), sorted
/// by path exactly as `fs::domain_snapshot`'s walk returned them.
///
/// Completeness rides `governing_dirs`' contract (every page the narrowing
/// keeps lives directly in a listed directory); correctness stays with
/// [`policy::RuleIndex::narrowed_to`], which still filters page by page — an
/// extra page read here is dropped there, never rendered.
///
/// A listed directory that is absent (an ancestor with no `rules/` child) or a
/// PATH leaf that names a file holds no pages and is skipped; any other read
/// failure aborts loudly, as the full walk did. A non-UTF-8 file NAME is
/// skipped exactly as the snapshot skipped it: wire paths are UTF-8, so such a
/// file is unservable and unnameable alike.
fn chain_pages(
    workspace: &Path,
    domain: &fs::domain::Domain,
    at: &str,
) -> Result<fs::DomainFiles, Fail> {
    let corpus_fail =
        |e: std::io::Error| Fail::tool(format!("cannot read the workspace corpus: {e}"));
    let mut files = fs::DomainFiles::new();
    for dir in policy::governing_dirs(at) {
        let abs = if dir.is_empty() {
            workspace.to_path_buf()
        } else {
            workspace.join(&dir)
        };
        if !abs.is_dir() {
            continue;
        }
        for entry in std::fs::read_dir(&abs).map_err(corpus_fail)? {
            let entry = entry.map_err(corpus_fail)?;
            // `file_type` does not follow symlinks — the walk's own law.
            if !entry.file_type().map_err(corpus_fail)?.is_file() {
                continue;
            }
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let rel = if dir.is_empty() {
                name.to_owned()
            } else {
                format!("{dir}/{name}")
            };
            if !domain.contains(Path::new(&rel)) {
                continue;
            }
            let bytes = std::fs::read(entry.path()).map_err(corpus_fail)?;
            files.push((rel, bytes));
        }
    }
    // Path-ascending, as the snapshot's walk returned them — the tuple sort IS
    // the path sort, since paths are unique. Never scope arithmetic: ordering
    // by the ladder is `policy`'s resolve, and this layer holds no second copy.
    files.sort();
    Ok(files)
}

/// The user rung, plus the scope it came from. The enumeration law lives in
/// [`fs::user_rule_pages`], shared with the discovery feed; the anchor is the
/// config plane's answer, never a guess.
fn user_pages(pages: &mut Vec<(ScopeLayer, String, Vec<u8>)>) -> UserScope {
    let anchor = match config::resolve_path(&config::Env::from_process()) {
        Ok(anchor) => anchor,
        Err(e) => {
            return UserScope::Absent {
                reason: format!("the config plane refused: {e}"),
            };
        }
    };
    if !anchor.is_file() {
        return UserScope::Absent {
            reason: format!("no anchor at {}", anchor.display()),
        };
    }
    let scope = anchor
        .parent()
        .map_or_else(|| anchor.display().to_string(), |p| p.display().to_string());
    match fs::user_rule_pages(&anchor) {
        Ok(found) => {
            pages.extend(
                found
                    .into_iter()
                    .map(|(page, bytes)| (ScopeLayer::User, page, bytes)),
            );
            UserScope::Declared {
                scope,
                anchor: anchor.display().to_string(),
            }
        }
        Err(e) => UserScope::Absent {
            reason: format!("{}/{} is unreadable: {e}", scope, fs::USER_RULES_DIR),
        },
    }
}

/// The domain's pages as a [`PageSource`]: the chain pages discovery already
/// read (seed — one read, one answer where the bytes are in hand), then a
/// domain-gated disk read for a page the chain does not carry. An armed row may
/// pin a page anywhere in the workspace, so the source must reach past the
/// chain; the gate keeps the answer the snapshot always gave — a path outside
/// the hash domain (non-md, dot segment, custom-ignored, or not workspace-
/// relative) is NOT HERE, whatever sits on disk, which is what keeps
/// [`ArmedOrphan`]'s "on disk, outside the hash domain" cause reachable.
struct DomainPages<'a> {
    workspace: &'a Path,
    domain: &'a fs::domain::Domain,
    seed: &'a BTreeMap<String, String>,
}

impl DomainPages<'_> {
    fn read_page(&self, page: &str) -> std::io::Result<String> {
        if let Some(bytes) = self.seed.get(page) {
            return Ok(bytes.clone());
        }
        let rel = Path::new(page);
        let workspace_relative = rel
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)));
        if !workspace_relative || !self.domain.contains(rel) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("{page} is not here"),
            ));
        }
        std::fs::read_to_string(self.workspace.join(rel))
    }
}

impl PageSource for DomainPages<'_> {
    fn read(&self, page: &str) -> std::io::Result<String> {
        self.read_page(page)
    }
}

/// Enumerate the chain, discover, narrow, resolve, join the armed set — every
/// judgement from `policy`, every byte from disk.
fn build(workspace: &Path, at: &str, view: View) -> Result<RulesReport, Fail> {
    let root = fs::WorkspaceRoot(workspace.to_path_buf());
    let domain = fs::domain::Domain::load(&root)
        .map_err(|e| Fail::tool(format!("cannot read the workspace corpus: {e}")))?;
    let mut raw: Vec<(ScopeLayer, String, Vec<u8>)> = chain_pages(workspace, &domain, at)?
        .into_iter()
        .map(|(page, bytes)| (ScopeLayer::Workspace, page, bytes))
        .collect();
    let user_scope = user_pages(&mut raw);

    // Non-UTF-8 bytes cannot carry a readable tag. Named, never silently
    // skipped: a file the verb could not read is a hole in its own answer.
    // Chain-scoped exactly as the refusals are (§ 3): a scoped query reddens
    // for the unreadable pages on its chain, not for a stranger's.
    let mut unreadable = Vec::new();
    let mut text: Vec<(ScopeLayer, String, String)> = Vec::new();
    let mut corpus: BTreeMap<String, String> = BTreeMap::new();
    for (layer, page, bytes) in raw {
        match String::from_utf8(bytes) {
            Ok(content) => {
                if layer == ScopeLayer::Workspace {
                    corpus.insert(page.clone(), content.clone());
                }
                text.push((layer, page, content));
            }
            Err(_) => unreadable.push(page),
        }
    }

    let index = RuleIndex::discover(text.iter().filter(|(layer, ..)| view.admits(*layer)).map(
        |(layer, page, bytes)| PageRef {
            layer: *layer,
            page,
            bytes,
        },
    ));
    // Narrowing is the consumer's step: narrow, then resolve through the shared
    // resolver.
    let narrowed = index.narrowed_to(at);
    let effective = narrowed.resolve();

    let pages = DomainPages {
        workspace,
        domain: &domain,
        seed: &corpus,
    };
    let (armed, artifact) = load_armed(&pages);
    // ONE join for the whole answer: a ledger row renders in at most one of the
    // three sections below, but every one of them needs the column, and a
    // pinned page must not be read three times on a hook-adjacent verb.
    let drift = drift_column(artifact.as_ref(), &pages);
    let rows = rows(&effective, artifact.as_ref(), at, &pages, &drift);

    // ⛔ Only a view that admits the WORKSPACE layer can have orphans. The
    // armed artifact's `page` column is a workspace spelling by construction
    // (`policy::armed::ArmFault::UserLayerDeferred` refuses anything else), so
    // under `--user` the resolved set holds no workspace id at all and EVERY
    // armed row would look orphaned. That would be a manufactured finding in
    // the direction that certifies a defect — the worst direction — so the
    // view is asked first.
    let (workspace_declined, workspace_undecidable) = declined_workspace(workspace);
    let (dot_declined, dot_undecidable) = declined_dot_workspace(workspace);
    let (user_declined, user_undecidable) = declined_user();
    let mut undecidable = workspace_undecidable;
    undecidable.extend(dot_undecidable);
    undecidable.extend(user_undecidable);
    undecidable.sort();
    undecidable.dedup();

    // ⛔ Same view guard as the orphans, for the same reason: under `--user` the
    // resolved set holds no workspace id, so every workspace-spelled armed row
    // would be manufactured into a finding.
    let (armed_orphans, armed_elsewhere) = if view.admits(ScopeLayer::Workspace) {
        (
            orphans(artifact.as_ref(), at, &effective, workspace, &drift),
            armed_elsewhere(artifact.as_ref(), at, &effective, &pages, &drift),
        )
    } else {
        (Vec::new(), Vec::new())
    };

    Ok(RulesReport {
        at: at.to_owned(),
        view,
        workspace: workspace.display().to_string(),
        user_scope,
        armed,
        rows,
        refused: narrowed.refused().iter().map(ToString::to_string).collect(),
        unreadable,
        declined_workspace: workspace_declined,
        declined_user: user_declined,
        declined_dot_workspace: dot_declined,
        undecidable,
        armed_orphans,
        armed_elsewhere,
    })
}

/// RULE PAGES a dot-prefixed segment keeps out of the workspace hash domain —
/// the dot twin of [`declined_workspace`], amending F11 for the one population
/// its silence turned dangerous: a registration candidate that reads as
/// working law while governing nothing (card rules-silent-nonregistration;
/// measured live on the mw-face e2e, `.hidden/rules/x.md`).
///
/// The F11 guards survive the amendment: the population is narrowed by ASKING
/// THE REGISTRAR ([`rule_pages_among`], the same offers-itself law as both
/// sibling feeds), never a path predicate, so dot NOISE — markdown with no
/// rule intent, the 36-fixtures class — stays invisible; and the voice is
/// exit-neutral. The enumeration is [`fs::dot_declined_markdown`]: the one dot
/// predicate, dot subtrees only, never a second walk law here.
fn declined_dot_workspace(workspace: &Path) -> (Vec<String>, Vec<String>) {
    let root = fs::WorkspaceRoot(workspace.to_path_buf());
    let Ok(declined) = fs::dot_declined_markdown(&root) else {
        return (Vec::new(), Vec::new());
    };
    let outside: Vec<(String, String)> = declined
        .into_iter()
        .filter_map(|rel| {
            // An unreadable or non-UTF-8 excluded file cannot be shown to carry
            // a rule tag, so it is not claimed as one.
            std::fs::read_to_string(workspace.join(&rel))
                .ok()
                .map(|bytes| (rel, bytes))
        })
        .collect();
    rule_pages_among(&outside)
}

/// RULE PAGES the workspace hash domain does not carry — never "markdown the
/// domain does not carry", which is a different and much larger population.
///
/// ⛔ THIS FUNCTION SHIPPED WRONG ONCE AND A PRE-EXISTING GATE CAUGHT IT. Its
/// first form named every out-of-domain markdown file, which in this very repo
/// is THIRTY-SIX test-data fixtures that carry no rule tag and never could.
/// That is the wrong-population defect **this card exists to prevent, arriving
/// one level up in the fix for it**: `rules` lists RULE PAGES, so a sentence it
/// prints must be about rule pages. An operator told "36 files are outside the
/// hash domain" learns nothing about their law and stops reading the line.
///
/// ⛔ AND ITS SECOND FORM WALKED A TREE THE ENGINE DOES NOT SERVE (dogfood
/// F11, 2026-08-15). It enumerated `fs::walk` minus the snapshot — the
/// ADDRESSABLE set, which enters dot directories — so a dot-named snapshot
/// dir the record projection holds ZERO records for produced 16 of 20 caveat
/// lines. The enumerator is now [`fs::declined_markdown`], which walks by the
/// projection's own dir law (one shared dot-segment predicate) and reports
/// the CUSTOM-IGNORE class: operator-declared, vault-visible exclusions,
/// whose silent drop is the defect session decision 0017 ended. A dot path
/// is invisible here exactly as it is invisible to everything the engine
/// serves.
///
/// The population is then narrowed by ASKING THE REGISTRAR, never by a path
/// predicate: a page counts when it OFFERS ITSELF to registration, whether it
/// then registers or is refused. Both are rule pages whose law is missing
/// from this answer, and a refused one is arguably worse.
fn declined_workspace(workspace: &Path) -> (Vec<String>, Vec<String>) {
    let root = fs::WorkspaceRoot(workspace.to_path_buf());
    let Ok(declined) = fs::declined_markdown(&root) else {
        return (Vec::new(), Vec::new());
    };
    let outside: Vec<(String, String)> = declined
        .into_iter()
        .filter_map(|rel| {
            // An unreadable or non-UTF-8 excluded file cannot be shown to carry
            // a rule tag, so it is not claimed as one.
            std::fs::read_to_string(workspace.join(&rel))
                .ok()
                .map(|bytes| (rel, bytes))
        })
        .collect();
    rule_pages_among(&outside)
}

/// Split `candidates` into the pages that OFFER THEMSELVES to rule
/// registration and the pages whose rule-ness CANNOT BE ANSWERED — the
/// three-state law, whose ⛔ block rides the one shared narrowing:
/// [`crate::rules_walk::rule_candidates_among`].
///
/// Receipt: this repo's own `frontmatter-unparseable.md` is a `meridian-config`
/// fixture with an unclosed flow sequence. Counting it as a dropped rule page
/// was a FALSE SENTENCE about a page that is not one; dropping it silently
/// would be the very defect this card exists to end. It gets its own verdict
/// string instead — a state without one is not enumerated, it is mentioned.
///
/// The registrar decides, never a predicate here: a second reading of what
/// makes a rule page would be a fork of `policy`'s law that could disagree
/// with it. This adapter only drops the ids the voices here never print.
fn rule_pages_among(candidates: &[(String, String)]) -> (Vec<String>, Vec<String>) {
    let (offered, undecidable) = crate::rules_walk::rule_candidates_among(candidates);
    (
        offered.into_iter().map(|(page, _id)| page).collect(),
        undecidable,
    )
}

/// RULE PAGES under the user `rules/` tree that a dot segment declined, from
/// the rung's own traversal ([`fs::user_rule_pages_declined`]) rather than a
/// second enumeration here that could disagree with the one that declined them.
///
/// Narrowed by the registrar for the same reason as [`declined_workspace`]: a
/// stray `.notes/scratch.md` under `rules/` is not a dropped rule page and
/// naming it as one would teach the operator to ignore the line.
fn declined_user() -> (Vec<String>, Vec<String>) {
    let Ok(anchor) = config::resolve_path(&config::Env::from_process()) else {
        return (Vec::new(), Vec::new());
    };
    let Some(user_scope) = anchor.parent().map(Path::to_path_buf) else {
        return (Vec::new(), Vec::new());
    };
    let Ok(declined) = fs::user_rule_pages_declined(&anchor) else {
        return (Vec::new(), Vec::new());
    };
    let candidates: Vec<(String, String)> = declined
        .into_iter()
        .filter_map(|rel| {
            std::fs::read_to_string(user_scope.join(&rel))
                .ok()
                .map(|bytes| (rel, bytes))
        })
        .collect();
    rule_pages_among(&candidates)
}

/// The armed rows selected at `at` whose id discovery no longer resolves.
///
/// Keyed on the id's ABSENCE FROM THE RESOLVED SET, never on a path predicate:
/// a consumer can inherit this defect without the spelling ever changing, and a
/// predicate keyed on which path a door prints is structurally blind there
/// (charter 09, consumer axis).
fn orphans(
    artifact: Option<&ArmedArtifact>,
    at: &str,
    effective: &EffectiveSet,
    workspace: &Path,
    drift: &BTreeMap<(String, String), DriftCell>,
) -> Vec<ArmedOrphan> {
    let Some(artifact) = artifact else {
        return Vec::new();
    };
    artifact
        .select_at(at)
        .into_iter()
        .filter(|row| effective.resolved().get(row.id().as_str()).is_none())
        .map(|row| ArmedOrphan {
            id: row.id().as_str().to_owned(),
            page: row.page().to_owned(),
            scope: row.scope().as_str().to_owned(),
            mode: row.mode().as_str().to_owned(),
            cause: orphan_cause(workspace, row.page()),
            drift: drift_of(drift, row),
        })
        .collect()
}

/// Read the attested armed set through the domain-gated source. The artifact
/// page usually sits OFF the queried chain, so this is a disk read; the gate
/// keeps the snapshot's answer — an artifact outside the hash domain (or not
/// UTF-8) reads as absent, exactly as it never entered the snapshot.
fn load_armed(pages: &DomainPages<'_>) -> (ArmedSource, Option<ArmedArtifact>) {
    let path = policy::armed::ARMED_RULES_PATH.to_owned();
    let Ok(page) = pages.read_page(&path) else {
        return (ArmedSource::Absent { path }, None);
    };
    match policy::armed::parse_artifact(&page) {
        Ok(artifact) => {
            let rows = artifact.rows().len();
            (ArmedSource::Present { path, rows }, Some(artifact))
        }
        Err(corrupt) => (
            ArmedSource::Unreadable {
                path,
                detail: corrupt.to_string(),
            },
            None,
        ),
    }
}

/// The drift column for EVERY attested row, keyed by the row key (id, arm root)
/// — the ruling's "per-row drift column for every ledger row", joined once.
///
/// ⛔ The comparison is [`ArmedArtifact::drift`]'s, never one written here: a rev
/// law at this layer would be the second resolver the CLI is forbidden to hold,
/// and it would be free to disagree with the gate about a row that fires.
/// `mrd status` holds exactly such a second law (`status_cmd::page_drifted`) and
/// that is why the two verbs could report different drift counts on one
/// artifact.
fn drift_column(
    artifact: Option<&ArmedArtifact>,
    pages: &dyn PageSource,
) -> BTreeMap<(String, String), DriftCell> {
    let Some(artifact) = artifact else {
        return BTreeMap::new();
    };
    artifact
        .drift(pages)
        .iter()
        .map(|row| {
            (
                (
                    row.row().id().as_str().to_owned(),
                    row.row().scope().as_str().to_owned(),
                ),
                DriftCell::of(row),
            )
        })
        .collect()
}

/// One row's drift cell out of the joined column. A row the column does not
/// carry cannot happen — the column is built from the same artifact these
/// sections select from — but a `map_or_else` over an absent key would silently
/// print `-`, which is the exact lie this column exists to stop. So the
/// fallback carries the pinned rev it does know and no clean claim.
fn drift_of(column: &BTreeMap<(String, String), DriftCell>, row: &ArmedRow) -> DriftCell {
    column
        .get(&(
            row.id().as_str().to_owned(),
            row.scope().as_str().to_owned(),
        ))
        .map_or_else(
            || DriftCell {
                pinned_rev: row.rev().to_owned(),
                live_rev: None,
                word: Some("unjoined"),
            },
            |cell| DriftCell {
                pinned_rev: cell.pinned_rev.clone(),
                live_rev: cell.live_rev.clone(),
                word: cell.word,
            },
        )
}

/// The word a [`Redness`] renders as. One mapping, shared by the armed cell and
/// the elsewhere section, so the two faces cannot drift apart.
fn redness_word(why: &Redness) -> &'static str {
    match why {
        Redness::Drifted { .. } => "drifted",
        Redness::Missing { .. } => "missing",
        // A row whose mode is outside its page's kind vocabulary reads as a
        // mismatch, not drift: the page is untouched and its rev still matches,
        // so "drifted" would send a reader to diff a page that never moved.
        Redness::ModeOutsideKind { .. } => "kind-mismatch",
    }
}

/// The armed rows for ids this answer RESOLVES whose arm root does not contain
/// `at` — everything [`ArmedArtifact::select_at`] drops, which is precisely what
/// used to be silent.
///
/// ⛔ The containment question and its verification are `policy`'s single
/// [`ArmedArtifact::verify_elsewhere_at`] call, never re-derived here: a path
/// predicate written at this layer would be the second resolver the CLI is
/// forbidden to hold. This function only drops the rows whose id discovery no
/// longer resolves — those are [`ArmedOrphan`]s and have their own section, so
/// the two "shown nowhere" populations never double-count.
fn armed_elsewhere(
    artifact: Option<&ArmedArtifact>,
    at: &str,
    effective: &EffectiveSet,
    pages: &dyn PageSource,
    drift: &BTreeMap<(String, String), DriftCell>,
) -> Vec<ArmedElsewhere> {
    let Some(artifact) = artifact else {
        return Vec::new();
    };
    artifact
        .verify_elsewhere_at(at, pages)
        .into_iter()
        .filter(|found| {
            effective
                .resolved()
                .get(found.row().id().as_str())
                .is_some()
        })
        .map(|found| ArmedElsewhere {
            id: found.row().id().as_str().to_owned(),
            page: found.row().page().to_owned(),
            scope: found.row().scope().as_str().to_owned(),
            mode: found.row().mode().as_str().to_owned(),
            redness: found.why().map(|why| redness_word(why).to_owned()),
            drift: drift_of(drift, found.row()),
        })
        .collect()
}

/// One row per resolved id and one per collision, id-ascending within each, the
/// resolved set first.
fn rows(
    effective: &EffectiveSet,
    artifact: Option<&ArmedArtifact>,
    at: &str,
    pages: &dyn PageSource,
    drift: &BTreeMap<(String, String), DriftCell>,
) -> Vec<RuleRow> {
    // The selection law, from the artifact: per id, the deepest armed row whose
    // arm root contains this path. Keyed by (id, arm root), never by id alone.
    let selected: Vec<&ArmedRow> = artifact.map(|a| a.select_at(at)).unwrap_or_default();
    // The redness of each armed row, keyed by (id, arm root) — the artifact's own
    // fail-closed rev check, never a second hash law here.
    let mut reddened: BTreeMap<(String, String), &'static str> = BTreeMap::new();
    if let Some(artifact) = artifact {
        let verdict = artifact.verify_at(at, pages);
        for red in verdict.red() {
            reddened.insert(
                (
                    red.row().id().as_str().to_owned(),
                    red.row().scope().as_str().to_owned(),
                ),
                redness_word(red.why()),
            );
        }
    }
    let cell = |id: &str, winner: &policy::Registration| -> Option<ArmedCell> {
        let row = selected.iter().find(|row| row.id().as_str() == id)?;
        Some(ArmedCell {
            mode: row.mode().as_str().to_owned(),
            elsewhere: (row.page() != winner.page()).then(|| row.page().to_owned()),
            redness: reddened
                .get(&(
                    row.id().as_str().to_owned(),
                    row.scope().as_str().to_owned(),
                ))
                .map(|why| (*why).to_owned()),
            drift: drift_of(drift, row),
        })
    };

    let mut rows: Vec<RuleRow> = effective
        .resolved()
        .iter()
        .map(|(id, resolution)| RuleRow {
            id: id.as_str().to_owned(),
            state: "resolved",
            collision_scope: None,
            armed: cell(id.as_str(), resolution.winner()),
            chain: chain_of(resolution),
        })
        .collect();

    for collision in effective.collisions() {
        rows.push(RuleRow {
            id: collision.id().as_str().to_owned(),
            state: "collision",
            collision_scope: Some((
                collision.scope().layer().as_str(),
                collision.scope().depth(),
            )),
            // A collided id resolves to nothing, so there is no winner for an
            // armed row to be about.
            armed: None,
            chain: collision
                .tied()
                .iter()
                .map(|tied| ChainEntry::of("tied", tied))
                .chain(
                    collision
                        .shadowed()
                        .iter()
                        .map(|page| ChainEntry::of("shadowed", page)),
                )
                .collect(),
        });
    }
    rows
}

/// The chain, winner first then outward — [`Effective::chain`]'s own order.
fn chain_of(resolution: &Effective) -> Vec<ChainEntry> {
    resolution
        .chain()
        .enumerate()
        .map(|(position, registration)| {
            ChainEntry::of(
                if position == 0 { "winner" } else { "shadowed" },
                registration,
            )
        })
        .collect()
}

// ── rendering ─────────────────────────────────────────────────────────────────

/// The human render: a header naming what was read, then one block per id.
#[allow(clippy::too_many_lines)] // one sequential render pass by design
fn render_human(report: &RulesReport) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let at = match report.view {
        View::Effective => format!("rules at {}", display_path(&report.at)),
        View::Workspace => "rules at the workspace-root layer".to_owned(),
        View::User => "rules at the user layer".to_owned(),
    };
    let _ = writeln!(out, "{at}");
    let _ = writeln!(out, "  workspace  {}", report.workspace);
    match &report.user_scope {
        UserScope::Declared { scope, anchor } => {
            let _ = writeln!(out, "  user-scope {scope}  (anchor {anchor})");
        }
        UserScope::Absent { reason } => {
            let _ = writeln!(out, "  user-scope none  ({reason})");
        }
    }
    match &report.armed {
        ArmedSource::Present { path, rows } => {
            let _ = writeln!(out, "  armed-set  {path} ({rows} row(s))");
        }
        // `none` is the whole honest answer. Where an armed set WOULD live is
        // teaching, and teaching lives in docs on demand — never a footnote
        // charged to every invocation (ZT ruling 4, 2026-08-15). The present
        // and corrupt arms keep their path: there it is the diagnostic.
        ArmedSource::Absent { .. } => {
            let _ = writeln!(out, "  armed-set  none");
        }
        ArmedSource::Unreadable { path, detail } => {
            let _ = writeln!(out, "  armed-set  UNREADABLE  ({path}: {detail})");
        }
    }

    if report.rows.is_empty() {
        let _ = writeln!(out, "  (no rules in effect)");
    }
    for row in &report.rows {
        match (&row.collision_scope, &row.armed) {
            (Some((layer, depth)), _) => {
                let _ = writeln!(
                    out,
                    "  {}  REFUSED collision at layer={layer} depth={depth} — this id resolves \
                     to nothing",
                    row.id
                );
            }
            // The drift column rides only where a LEDGER ROW does. `armed=-`
            // means no ledger row governs this id here, and `drift=-` beside it
            // would claim a join that was never made — the same lie in the
            // opposite direction from the one this column closes.
            (None, Some(armed)) => {
                let _ = writeln!(
                    out,
                    "  {}  armed={}  drift={}",
                    row.id,
                    armed.render(),
                    armed.drift.render()
                );
            }
            (None, None) => {
                let _ = writeln!(out, "  {}  armed=-", row.id);
            }
        }
        for entry in &row.chain {
            let _ = writeln!(
                out,
                "      {:8}  {}  rev={}  layer={} depth={}  kinds={}",
                entry.role, entry.page, entry.rev, entry.layer, entry.depth, entry.kinds
            );
        }
    }

    // Printed BEFORE `refused:`, because an armed row the header counts and the
    // body never showed is the one thing a reader of this answer cannot
    // reconstruct from anything else on the page.
    if !report.armed_orphans.is_empty() {
        let _ = writeln!(
            out,
            "armed rows counted above whose pinned page is NOT in this answer:"
        );
        for orphan in &report.armed_orphans {
            // `scope` here IS the armed artifact's arm-root column — the one
            // place this render says "scope" — and the workspace root prints
            // `.` exactly as the artifact's own cell does.
            let _ = writeln!(
                out,
                "  {}  armed={} at scope={}  drift={} — pinned {} ({})",
                orphan.id,
                orphan.mode,
                display_path(&orphan.scope),
                orphan.drift.render(),
                orphan.page,
                orphan.cause
            );
        }
    }
    // Same reason as the orphan section above, one step less severe: the header
    // counts these rows and no `armed=` cell is about them, so the reader cannot
    // reconstruct them from anything else on the page. The section header
    // carries the containment fact once; each line carries its own redness, so
    // the loud cause is never swallowed by the silent one.
    if !report.armed_elsewhere.is_empty() {
        let _ = writeln!(
            out,
            "armed rows counted above whose arm root does NOT contain this path:"
        );
        for row in &report.armed_elsewhere {
            // `scope` here IS the armed artifact's arm-root column — the same
            // vocabulary the orphan section uses, and the workspace root prints
            // `.` exactly as the artifact's own cell does.
            let redness = row
                .redness
                .as_ref()
                .map_or_else(String::new, |why| format!(" ({why})"));
            let _ = writeln!(
                out,
                "  {}  armed={} at scope={}  drift={} — pinned {}{redness}",
                row.id,
                row.mode,
                display_path(&row.scope),
                row.drift.render(),
                row.page
            );
        }
    }
    if !report.refused.is_empty() {
        let _ = writeln!(out, "refused:");
        for refusal in &report.refused {
            let _ = writeln!(out, "  {refusal}");
        }
    }
    // The two declined populations are named SEPARATELY and never summed. They
    // come from two different feeds through two different mechanisms, and one
    // count over both would be a sentence about a population neither feed has.
    if !report.declined_workspace.is_empty() {
        let _ = writeln!(
            out,
            "not offered to registration — {} markdown file(s) under the workspace root are outside the hash domain, so no rule they carry is in this answer: {}",
            report.declined_workspace.len(),
            report.declined_workspace.join(", ")
        );
    }
    // The dot class, registrar-narrowed like its siblings and CAPPED unlike
    // them: a dot tree holds archived corpora (the F11 measurement), so the
    // narrowing does not bound this population the way the dot-free tree
    // bounds the custom class. Count full, sample capped, complete list on
    // the machine key — the decision-0017 shape.
    if !report.declined_dot_workspace.is_empty() {
        let _ = writeln!(
            out,
            "not offered to registration — {} rule page(s) under the workspace root are declined by a dot-prefixed path segment, so no rule they carry is in this answer: {}. The complete list is the `not_offered.workspace_dot` key of this verb's `--json`.",
            report.declined_dot_workspace.len(),
            crate::capped_sample(&report.declined_dot_workspace)
        );
    }
    // The one declined population with no registrar narrowing in front of it:
    // `register` refuses on unparseable frontmatter BEFORE reading any tag, so
    // every malformed-frontmatter excluded file lands here, rule-intent or not
    // — unbounded in a generated corpus. Hence the cap its two bounded
    // neighbours do not take (card rules-undecidable-carrier). The count stays
    // the full population; the complete list rides `not_offered.undecidable`.
    if !report.undecidable.is_empty() {
        let _ = writeln!(
            out,
            "cannot be answered — {} excluded markdown file(s) have frontmatter that does not parse, so whether they carry a rule is unknown, not decided: {}. The complete list is the `not_offered.undecidable` key of this verb's `--json`.",
            report.undecidable.len(),
            crate::capped_sample(&report.undecidable)
        );
    }
    if !report.declined_user.is_empty() {
        let _ = writeln!(
            out,
            "not offered to registration — {} markdown file(s) under the user rules/ tree are declined by a dot-prefixed segment, so no rule they carry is in this answer: {}",
            report.declined_user.len(),
            report.declined_user.join(", ")
        );
    }
    if !report.unreadable.is_empty() {
        let _ = writeln!(out, "unreadable:");
        for page in &report.unreadable {
            let _ = writeln!(out, "  {page} (not UTF-8, so its tags cannot be read)");
        }
    }
    out
}

/// The workspace root prints as `.` — an empty spelling would read as missing.
fn display_path(at: &str) -> &str {
    if at.is_empty() { "." } else { at }
}

/// The `--json` shape: the header facts, then the rows with their chains.
fn to_json(workspace: &Path, report: &RulesReport) -> Value {
    let rows: Vec<Value> = report
        .rows
        .iter()
        .map(|row| {
            let chain: Vec<Value> = row
                .chain
                .iter()
                .map(|entry| {
                    json!({
                        "role": entry.role,
                        "page": entry.page,
                        "rev": entry.rev,
                        "layer": entry.layer,
                        "depth": entry.depth,
                        "kinds": entry.kinds.split('+').collect::<Vec<_>>(),
                    })
                })
                .collect();
            json!({
                "id": row.id,
                "state": row.state,
                // The composite spelling the JSON always shipped — the machine
                // face is byte-stable while the human face de-collides.
                "collision_scope": row.collision_scope.map(|(layer, depth)| format!("{layer}:{depth}")),
                "armed": row.armed.as_ref().map(|armed| json!({
                    "mode": armed.mode,
                    "pinned_page": armed.elsewhere,
                    "redness": armed.redness,
                    "rendered": armed.render(),
                    // STRICTLY ADDITIVE, and the whole point of the machine
                    // half: `drift` is the word, `pinned_rev` and `live_rev`
                    // are the two facts it was computed from. The instrument
                    // this folds in (`rules-drift.py`) existed only because
                    // this face published the live rev and not the pinned one,
                    // so a reader had to parse the ledger markdown to ask.
                    "drift": armed.drift.word,
                    "pinned_rev": armed.drift.pinned_rev,
                    "live_rev": armed.drift.live_rev,
                })),
                "chain": chain,
            })
        })
        .collect();
    let user_scope = match &report.user_scope {
        UserScope::Declared { scope, anchor } => json!({ "scope": scope, "anchor": anchor }),
        UserScope::Absent { reason } => json!({ "scope": Value::Null, "reason": reason }),
    };
    let armed = match &report.armed {
        ArmedSource::Present { path, rows } => {
            json!({ "path": path, "state": "present", "rows": rows })
        }
        ArmedSource::Absent { path } => json!({ "path": path, "state": "absent" }),
        ArmedSource::Unreadable { path, detail } => {
            json!({ "path": path, "state": "unreadable", "detail": detail })
        }
    };
    json!({
        "workspace": workspace.display().to_string(),
        "rules": {
            "at": display_path(&report.at),
            "view": report.view.label(),
            "user_scope": user_scope,
            "armed_set": armed,
            "rules": rows,
            "refused": report.refused,
            "unreadable": report.unreadable,
            "armed_orphans": report.armed_orphans.iter().map(|orphan| json!({
                "id": orphan.id,
                "page": orphan.page,
                "scope": orphan.scope,
                "mode": orphan.mode,
                "cause": orphan.cause,
                "drift": orphan.drift.word,
                "pinned_rev": orphan.drift.pinned_rev,
                "live_rev": orphan.drift.live_rev,
            })).collect::<Vec<_>>(),
            // STRICTLY ADDITIVE: `armed` above keeps meaning WHAT GOVERNS HERE
            // and stays null for these rows, so no consumer that reads
            // `armed.mode` as governing is broken by the diagnosis. The cause
            // rides its own key.
            "armed_elsewhere": report.armed_elsewhere.iter().map(|row| json!({
                "id": row.id,
                "page": row.page,
                "scope": row.scope,
                "mode": row.mode,
                "redness": row.redness,
                "drift": row.drift.word,
                "pinned_rev": row.drift.pinned_rev,
                "live_rev": row.drift.live_rev,
            })).collect::<Vec<_>>(),
            "not_offered": {
                "workspace": report.declined_workspace,
                "user": report.declined_user,
                // STRICTLY ADDITIVE (card rules-silent-nonregistration): the
                // workspace dot class, complete — the machine half of the
                // capped prose line above it.
                "workspace_dot": report.declined_dot_workspace,
                "undecidable": report.undecidable,
            },
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(role: &'static str, page: &str, depth: usize) -> ChainEntry {
        ChainEntry {
            role,
            page: page.to_owned(),
            rev: "a".repeat(16),
            layer: "workspace",
            depth,
            kinds: "hook".to_owned(),
        }
    }

    /// A clean drift cell: pinned rev == live rev, no word.
    fn clean_drift() -> DriftCell {
        DriftCell {
            pinned_rev: "a".repeat(16),
            live_rev: Some("a".repeat(16)),
            word: None,
        }
    }

    /// A drifted cell, with the word the caller's mode earns.
    fn drifted(word: &'static str) -> DriftCell {
        DriftCell {
            pinned_rev: "a".repeat(16),
            live_rev: Some("b".repeat(16)),
            word: Some(word),
        }
    }

    fn report(rows: Vec<RuleRow>) -> RulesReport {
        RulesReport {
            at: "sessions/s1".to_owned(),
            view: View::Effective,
            workspace: "/ws".to_owned(),
            user_scope: UserScope::Absent {
                reason: "no anchor at /home/u/MERIDIAN.md".to_owned(),
            },
            armed: ArmedSource::Absent {
                path: policy::armed::ARMED_RULES_PATH.to_owned(),
            },
            rows,
            refused: Vec::new(),
            unreadable: Vec::new(),
            declined_workspace: Vec::new(),
            declined_user: Vec::new(),
            declined_dot_workspace: Vec::new(),
            undecidable: Vec::new(),
            armed_orphans: Vec::new(),
            armed_elsewhere: Vec::new(),
        }
    }

    #[test]
    fn render_human_shows_the_shadowed_page_beneath_its_winner() {
        let rows = vec![RuleRow {
            id: "task.review-notify".to_owned(),
            state: "resolved",
            collision_scope: None,
            armed: None,
            chain: vec![
                entry("winner", "sessions/s1/notify.md", 2),
                entry("shadowed", "notify.md", 0),
            ],
        }];
        let expected = "\
rules at sessions/s1
  workspace  /ws
  user-scope none  (no anchor at /home/u/MERIDIAN.md)
  armed-set  none
  task.review-notify  armed=-
      winner    sessions/s1/notify.md  rev=aaaaaaaaaaaaaaaa  layer=workspace depth=2  kinds=hook
      shadowed  notify.md  rev=aaaaaaaaaaaaaaaa  layer=workspace depth=0  kinds=hook
";
        assert_eq!(render_human(&report(rows)), expected);
    }

    /// The drift column rides EVERY ledger row and only ledger rows: an id with
    /// an armed cell carries `drift=`, and `armed=-` — no ledger row here —
    /// carries none, because `drift=-` beside it would claim a join nobody made.
    #[test]
    fn render_human_puts_the_drift_column_on_ledger_rows_and_nowhere_else() {
        let rows = vec![
            RuleRow {
                id: "task.off-and-edited".to_owned(),
                state: "resolved",
                collision_scope: None,
                armed: Some(ArmedCell {
                    mode: "off".to_owned(),
                    elsewhere: None,
                    redness: None,
                    drift: drifted("off-drifted"),
                }),
                chain: vec![entry("winner", "off.md", 0)],
            },
            RuleRow {
                id: "task.steady".to_owned(),
                state: "resolved",
                collision_scope: None,
                armed: Some(ArmedCell {
                    mode: "armed".to_owned(),
                    elsewhere: None,
                    redness: None,
                    drift: clean_drift(),
                }),
                chain: vec![entry("winner", "steady.md", 0)],
            },
            RuleRow {
                id: "task.unarmed".to_owned(),
                state: "resolved",
                collision_scope: None,
                armed: None,
                chain: vec![entry("winner", "unarmed.md", 0)],
            },
        ];
        let rendered = render_human(&report(rows));
        assert!(
            rendered.contains("  task.off-and-edited  armed=off  drift=off-drifted\n"),
            "the off row names its drift and keeps `armed=off`: {rendered}"
        );
        assert!(
            rendered.contains("  task.steady  armed=armed  drift=-\n"),
            "a clean ledger row still carries the column: {rendered}"
        );
        assert!(
            rendered.contains("  task.unarmed  armed=-\n") && !rendered.contains("armed=-  drift"),
            "no ledger row, no drift cell: {rendered}"
        );
    }

    #[test]
    fn render_human_names_an_empty_set() {
        let rendered = render_human(&report(Vec::new()));
        assert!(rendered.contains("(no rules in effect)"), "{rendered}");
    }

    #[test]
    fn render_human_renders_a_collision_as_a_refusal_with_both_pages() {
        let rows = vec![RuleRow {
            id: "shared".to_owned(),
            state: "collision",
            collision_scope: Some(("workspace", 1)),
            armed: None,
            chain: vec![
                entry("tied", "s/a.md", 1),
                entry("tied", "s/b.md", 1),
                entry("shadowed", "root.md", 0),
            ],
        }];
        let rendered = render_human(&report(rows));
        assert!(
            rendered.contains("shared  REFUSED collision at layer=workspace depth=1"),
            "{rendered}"
        );
        for page in ["s/a.md", "s/b.md", "root.md"] {
            assert!(rendered.contains(page), "names {page}: {rendered}");
        }
        assert!(
            !rendered.contains("winner"),
            "a collided id has no winner: {rendered}"
        );
    }

    #[test]
    fn the_armed_cell_renders_mode_redness_and_a_divergent_pin() {
        assert_eq!(
            ArmedCell {
                mode: "armed".to_owned(),
                elsewhere: None,
                redness: None,
                drift: clean_drift(),
            }
            .render(),
            "armed"
        );
        assert_eq!(
            ArmedCell {
                mode: "block".to_owned(),
                elsewhere: None,
                redness: Some("drifted".to_owned()),
                drift: drifted("drifted"),
            }
            .render(),
            "block(drifted)"
        );
        assert_eq!(
            ArmedCell {
                mode: "armed".to_owned(),
                elsewhere: Some("notify.md".to_owned()),
                redness: Some("missing".to_owned()),
                drift: DriftCell {
                    pinned_rev: "a".repeat(16),
                    live_rev: None,
                    word: Some("missing"),
                },
            }
            .render(),
            "armed(missing)@notify.md"
        );
    }

    /// The armed cell NEVER absorbs the drift word — `off(off-drifted)` reads as
    /// a redness and is not one. The two are separate columns, and an `off` row
    /// that drifted still renders its mode alone.
    #[test]
    fn the_drift_word_stays_out_of_the_armed_cell() {
        let cell = ArmedCell {
            mode: "off".to_owned(),
            elsewhere: None,
            redness: None,
            drift: drifted("off-drifted"),
        };
        assert_eq!(cell.render(), "off");
        assert_eq!(cell.drift.render(), "off-drifted");
    }

    /// A clean join prints `-`, not nothing: the defect this column closes is an
    /// ABSENT marker being read as "nothing drifted" when it meant "nobody
    /// asked".
    #[test]
    fn a_clean_join_renders_a_dash_rather_than_an_empty_cell() {
        assert_eq!(clean_drift().render(), "-");
        assert_eq!(drifted("off-drifted").render(), "off-drifted");
    }

    /// `off-drifted` IS NOT A REDNESS STATE AND TRIPS NO GATE — the ruling's own
    /// words. An off row whose pinned page moved is a clean exit.
    #[test]
    fn an_off_drifted_row_is_not_a_finding() {
        let r = report(vec![RuleRow {
            id: "task.notify".to_owned(),
            state: "resolved",
            collision_scope: None,
            armed: Some(ArmedCell {
                mode: "off".to_owned(),
                elsewhere: None,
                redness: None,
                drift: drifted("off-drifted"),
            }),
            chain: Vec::new(),
        }]);
        assert!(
            r.findings().is_empty(),
            "the drift column gates nothing: {:?}",
            r.findings()
        );
    }

    /// The complement, so the pair pins the boundary: a row that FIRES and
    /// drifted still reddens and still gates, exactly as before this column.
    #[test]
    fn a_firing_drifted_row_still_gates_the_exit() {
        let r = report(vec![RuleRow {
            id: "task.notify".to_owned(),
            state: "resolved",
            collision_scope: None,
            armed: Some(ArmedCell {
                mode: "armed".to_owned(),
                elsewhere: None,
                redness: Some("drifted".to_owned()),
                drift: drifted("drifted"),
            }),
            chain: Vec::new(),
        }]);
        assert_eq!(r.findings(), vec!["1 red armed row(s)"]);
    }

    #[test]
    fn findings_name_every_cause() {
        let mut r = report(vec![RuleRow {
            id: "shared".to_owned(),
            state: "collision",
            collision_scope: Some(("workspace", 1)),
            armed: None,
            chain: Vec::new(),
        }]);
        r.refused.push("`bad.md` declares no id".to_owned());
        r.rows.push(RuleRow {
            id: "drifty".to_owned(),
            state: "resolved",
            collision_scope: None,
            armed: Some(ArmedCell {
                mode: "armed".to_owned(),
                elsewhere: None,
                redness: Some("drifted".to_owned()),
                drift: drifted("drifted"),
            }),
            chain: Vec::new(),
        });
        r.armed = ArmedSource::Unreadable {
            path: policy::armed::ARMED_RULES_PATH.to_owned(),
            detail: "the header is not byte-exact".to_owned(),
        };
        let findings = r.findings();
        assert_eq!(
            findings,
            vec![
                "1 collided id(s)",
                "1 refused rule page(s)",
                "1 red armed row(s)",
                "an unreadable armed set",
            ]
        );
    }

    /// A clean report exits 0 — an empty population is not a finding.
    #[test]
    fn an_empty_report_has_no_findings() {
        assert!(report(Vec::new()).findings().is_empty());
    }

    #[test]
    fn json_carries_the_chain_and_the_armed_cell() {
        let rows = vec![RuleRow {
            id: "task.review-notify".to_owned(),
            state: "resolved",
            collision_scope: None,
            armed: Some(ArmedCell {
                mode: "armed".to_owned(),
                elsewhere: Some("notify.md".to_owned()),
                redness: None,
                drift: clean_drift(),
            }),
            chain: vec![
                entry("winner", "sessions/s1/notify.md", 2),
                entry("shadowed", "notify.md", 0),
            ],
        }];
        let value = to_json(Path::new("/ws"), &report(rows));
        let rules = &value["rules"]["rules"];
        assert_eq!(rules[0]["id"], json!("task.review-notify"));
        assert_eq!(rules[0]["chain"][0]["role"], json!("winner"));
        assert_eq!(rules[0]["chain"][1]["role"], json!("shadowed"));
        assert_eq!(rules[0]["chain"][0]["depth"], json!(2));
        assert_eq!(
            rules[0]["collision_scope"],
            Value::Null,
            "a resolved row has no collision"
        );
        assert_eq!(rules[0]["armed"]["pinned_page"], json!("notify.md"));
        assert_eq!(rules[0]["armed"]["rendered"], json!("armed@notify.md"));
        // The machine half of the drift column: the word, and the two revs it
        // was computed from — so a reader re-derives the join instead of
        // parsing the ledger markdown, which is why `rules-drift.py` existed.
        assert_eq!(rules[0]["armed"]["drift"], Value::Null, "a clean join");
        assert_eq!(rules[0]["armed"]["pinned_rev"], json!("a".repeat(16)));
        assert_eq!(rules[0]["armed"]["live_rev"], json!("a".repeat(16)));
        assert_eq!(value["rules"]["view"], json!("effective"));
        assert_eq!(value["rules"]["armed_set"]["state"], json!("absent"));
        assert_eq!(value["rules"]["user_scope"]["scope"], Value::Null);
    }

    /// The machine face is byte-stable across the human rename: `collision_scope`
    /// ships the composite `layer:depth` string it always did, while the human
    /// render spells the same two facts `layer=… depth=…`.
    #[test]
    fn json_ships_the_collision_scope_composite_unchanged() {
        let rows = vec![RuleRow {
            id: "shared".to_owned(),
            state: "collision",
            collision_scope: Some(("workspace", 1)),
            armed: None,
            chain: Vec::new(),
        }];
        let value = to_json(Path::new("/ws"), &report(rows));
        assert_eq!(
            value["rules"]["rules"][0]["collision_scope"],
            json!("workspace:1")
        );
    }

    // ── the invocation ────────────────────────────────────────────────────────

    fn parse(args: &[&str]) -> Result<Rules, Fail> {
        let owned: Vec<String> = args.iter().map(|a| (*a).to_string()).collect();
        Rules::parse(&owned)
    }

    #[test]
    fn parse_defaults_to_the_effective_view_at_the_cwd() {
        let r = parse(&[]).expect("parse");
        assert_eq!(r.view, View::Effective);
        assert!(r.path.is_none());
        assert!(matches!(r.format, Format::Human));
    }

    #[test]
    fn parse_accepts_a_path_and_each_single_layer_flag() {
        let r = parse(&["sessions/s1", "--json"]).expect("parse");
        assert_eq!(r.path.as_deref(), Some("sessions/s1"));
        assert!(matches!(r.format, Format::Json));
        assert_eq!(parse(&["--workspace"]).unwrap().view, View::Workspace);
        assert_eq!(parse(&["--user"]).unwrap().view, View::User);
    }

    #[test]
    fn parse_refuses_two_layer_flags() {
        let err = parse(&["--workspace", "--user"]).expect_err("one layer");
        assert_eq!(err.code, 2);
        assert!(
            err.message.contains("workspace") && err.message.contains("user"),
            "{}",
            err.message
        );
    }

    #[test]
    fn parse_rejects_an_unknown_flag_and_a_second_path() {
        assert_eq!(parse(&["--nope"]).unwrap_err().code, 2);
        assert_eq!(parse(&["a", "b"]).unwrap_err().code, 2);
    }

    #[test]
    fn a_single_layer_view_admits_one_layer() {
        assert!(View::Workspace.admits(ScopeLayer::Workspace));
        assert!(!View::Workspace.admits(ScopeLayer::User));
        assert!(View::User.admits(ScopeLayer::User));
        assert!(!View::User.admits(ScopeLayer::Workspace));
        assert!(View::Effective.admits(ScopeLayer::User));
        assert!(View::Effective.admits(ScopeLayer::Workspace));
    }

    #[test]
    fn a_path_outside_the_workspace_is_refused() {
        let ws = tempfile::tempdir().expect("tempdir");
        let outside = tempfile::tempdir().expect("tempdir");
        let root = std::fs::canonicalize(ws.path()).unwrap();
        std::fs::create_dir_all(root.join("sessions/s1")).expect("mkdir");
        assert_eq!(
            workspace_relative(&root, None, &root).expect("the root itself"),
            ""
        );
        assert_eq!(
            workspace_relative(&root, Some("sessions/s1"), &root).expect("a relative path"),
            "sessions/s1"
        );
        let err = workspace_relative(&root, Some(outside.path().to_str().unwrap()), &root)
            .expect_err("outside");
        assert_eq!(err.code, 2);
    }

    #[test]
    fn a_path_that_is_not_on_disk_is_refused_rather_than_answered_empty() {
        let ws = tempfile::tempdir().expect("tempdir");
        let root = std::fs::canonicalize(ws.path()).unwrap();
        let err = workspace_relative(&root, Some("sessions/typo"), &root)
            .expect_err("a path that is not there");
        assert_eq!(err.code, 2);
        assert!(
            err.message.contains("sessions/typo") && err.message.contains("exists"),
            "{}",
            err.message
        );
        assert_eq!(
            workspace_relative(&root, Some("replay"), &root)
                .expect_err("no such folder")
                .code,
            2
        );
    }
}
