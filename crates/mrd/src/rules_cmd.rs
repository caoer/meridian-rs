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
//! The verb walks the workspace hash domain and the user rung and calls pure
//! functions over the bytes: no arm, no receipt, no cap spend, no write path.
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

use policy::armed::{ArmedArtifact, ArmedRow, PageSource, Redness};
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
    let resolved = crate::resolve::resolve_runtime(&cwd).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace for {}: {e}",
            cwd.display()
        ))
    })?;
    let workspace = workspace::canonicalize(&resolved.workspace).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace {} ({e})",
            resolved.workspace.display()
        ))
    })?;

    // The default view narrows to the PATH argument; a single-layer view narrows
    // to the layer's own root.
    let at = match parsed.view {
        View::Effective => workspace_relative(&workspace, parsed.path.as_deref(), &cwd)?,
        View::Workspace | View::User => String::new(),
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
}

impl ArmedCell {
    /// The rendered cell: the mode word, its redness, and the pinned page when
    /// the arm and discovery disagree.
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
    /// Files whose bytes are not UTF-8, so their tags cannot be read.
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

/// The workspace pages, as `(path, bytes)` — the disk edge `policy` may not have.
fn workspace_pages(workspace: &Path) -> Result<fs::DomainFiles, Fail> {
    let root = fs::WorkspaceRoot(workspace.to_path_buf());
    let (files, _root) = fs::domain_snapshot(&root)
        .map_err(|e| Fail::tool(format!("cannot read the workspace corpus: {e}")))?;
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

/// The corpus as a [`PageSource`], so the armed set's rev check runs against the
/// same bytes discovery read — one read, one answer.
struct CorpusPages<'a>(&'a BTreeMap<String, String>);

impl PageSource for CorpusPages<'_> {
    fn read(&self, page: &str) -> std::io::Result<String> {
        self.0.get(page).cloned().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, format!("{page} is not here"))
        })
    }
}

/// Walk, discover, narrow, resolve, join the armed set — every judgement from
/// `policy`, every byte from disk.
fn build(workspace: &Path, at: &str, view: View) -> Result<RulesReport, Fail> {
    let mut raw: Vec<(ScopeLayer, String, Vec<u8>)> = workspace_pages(workspace)?
        .into_iter()
        .map(|(page, bytes)| (ScopeLayer::Workspace, page, bytes))
        .collect();
    let user_scope = user_pages(&mut raw);

    // Non-UTF-8 bytes cannot carry a readable tag. Named, never silently
    // skipped: a file the verb could not read is a hole in its own answer.
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

    let (armed, artifact) = load_armed(&corpus);
    let rows = rows(&effective, artifact.as_ref(), at, &CorpusPages(&corpus));

    // ⛔ Only a view that admits the WORKSPACE layer can have orphans. The
    // armed artifact's `page` column is a workspace spelling by construction
    // (`policy::armed::ArmFault::UserLayerDeferred` refuses anything else), so
    // under `--user` the resolved set holds no workspace id at all and EVERY
    // armed row would look orphaned. That would be a manufactured finding in
    // the direction that certifies a defect — the worst direction — so the
    // view is asked first.
    let (workspace_declined, workspace_undecidable) = declined_workspace(workspace);
    let (user_declined, user_undecidable) = declined_user();
    let mut undecidable = workspace_undecidable;
    undecidable.extend(user_undecidable);
    undecidable.sort();
    undecidable.dedup();

    // ⛔ Same view guard as the orphans, for the same reason: under `--user` the
    // resolved set holds no workspace id, so every workspace-spelled armed row
    // would be manufactured into a finding.
    let (armed_orphans, armed_elsewhere) = if view.admits(ScopeLayer::Workspace) {
        (
            orphans(artifact.as_ref(), at, &effective, workspace),
            armed_elsewhere(artifact.as_ref(), at, &effective, &CorpusPages(&corpus)),
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
        undecidable,
        armed_orphans,
        armed_elsewhere,
    })
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
/// registration and the pages whose rule-ness CANNOT BE ANSWERED.
///
/// ⛔ THERE ARE THREE STATES HERE, NOT TWO, and collapsing them is how this
/// function shipped wrong twice. `RegisterFault::FrontmatterUnparsed` says in
/// its own words that whether the page carries a registration tag *cannot be
/// answered*; EVERY OTHER fault variant presupposes the tag was there. So a
/// page with a broken frontmatter block is neither a dropped rule page nor
/// established not to be one.
///
/// Receipt: this repo's own `frontmatter-unparseable.md` is a `meridian-config`
/// fixture with an unclosed flow sequence. Counting it as a dropped rule page
/// was a FALSE SENTENCE about a page that is not one; dropping it silently
/// would be the very defect this card exists to end. It gets its own verdict
/// string instead — a state without one is not enumerated, it is mentioned.
///
/// The registrar decides, never a predicate here: a second reading of what
/// makes a rule page would be a fork of `policy`'s law that could disagree
/// with it.
fn rule_pages_among(candidates: &[(String, String)]) -> (Vec<String>, Vec<String>) {
    let index = RuleIndex::discover(candidates.iter().map(|(page, bytes)| PageRef {
        layer: ScopeLayer::Workspace,
        page,
        bytes,
    }));
    let mut offered: Vec<String> = index
        .registered()
        .iter()
        .map(|r| r.page().to_owned())
        .collect();
    let mut undecidable = Vec::new();
    for refusal in index.refused() {
        if matches!(
            refusal.fault(),
            policy::RegisterFault::FrontmatterUnparsed { .. }
        ) {
            undecidable.push(refusal.page().to_owned());
        } else {
            offered.push(refusal.page().to_owned());
        }
    }
    offered.sort();
    offered.dedup();
    undecidable.sort();
    undecidable.dedup();
    (offered, undecidable)
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
        })
        .collect()
}

/// Read the attested armed set out of the corpus bytes already in hand.
fn load_armed(corpus: &BTreeMap<String, String>) -> (ArmedSource, Option<ArmedArtifact>) {
    let path = policy::armed::ARMED_RULES_PATH.to_owned();
    let Some(page) = corpus.get(&path) else {
        return (ArmedSource::Absent { path }, None);
    };
    match policy::armed::parse_artifact(page) {
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
            (None, armed) => {
                let cell = armed
                    .as_ref()
                    .map_or_else(|| "-".to_owned(), ArmedCell::render);
                let _ = writeln!(out, "  {}  armed={cell}", row.id);
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
                "  {}  armed={} at scope={} — pinned {} ({})",
                orphan.id,
                orphan.mode,
                display_path(&orphan.scope),
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
                "  {}  armed={} at scope={} — pinned {}{redness}",
                row.id,
                row.mode,
                display_path(&row.scope),
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
            })).collect::<Vec<_>>(),
            "not_offered": {
                "workspace": report.declined_workspace,
                "user": report.declined_user,
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
                redness: None
            }
            .render(),
            "armed"
        );
        assert_eq!(
            ArmedCell {
                mode: "block".to_owned(),
                elsewhere: None,
                redness: Some("drifted".to_owned())
            }
            .render(),
            "block(drifted)"
        );
        assert_eq!(
            ArmedCell {
                mode: "armed".to_owned(),
                elsewhere: Some("notify.md".to_owned()),
                redness: Some("missing".to_owned())
            }
            .render(),
            "armed(missing)@notify.md"
        );
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
