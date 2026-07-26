//! `mrd realise <page> [--dry] [--truth index|file] [--json]` (U3.5b): the
//! reconciliation-loop verb — **observe → check → apply (only on drift) →
//! re-check** over one page's declared claim, wiring the U3.5a engine
//! ([`realise::realise`]). The md→mrd cutover of the legacy `md realise`; the
//! apply rides the ordinary run plane ([`run::runner::run`]) — "run grammars
//! reconcile into `mrd run`", no second execution path.
//!
//! # The mrd-native claim grammar
//! A realising page declares its claim in frontmatter (each tool reads its NATIVE
//! serialization — the field-equivalence parity gate, `decisions/2026-07-23-u3-1-
//! parity-gate-ruling.md`):
//!
//! - `realise.field` + `realise.expected` — the check (d2 §5.4 "check = rev
//!   compare", at field grain): converged iff the frontmatter field equals the
//!   expected value. Pure read, no cap.
//! - `realise.apply` — the apply task name addressed on the page (`task.<name>`);
//!   absent ⇒ the claim is NOT apply-capable, a drift classifies `pending-agent`.
//!
//! The retry budget is ONE apply then one re-check — parity with the legacy
//! engine's single-apply budget.
//!
//! # Terminal states (parity vocabulary)
//! `converged` (check clean, no apply) · `drifted-fixed` (apply fixed the drift) ·
//! `non-convergent` (apply ran, drift persists) · `pending-agent` (drift, no
//! apply-capable claim) · `preview` (`--dry`). The mrd render grammar and CLI
//! grammar differ from `md realise`'s (OUT of the parity gate's scope); the
//! terminal STATE is field-equivalent.
//!
//! # `--truth index|file` — realise-as-deploy (U4.3)
//! Resolve a file↔index convention divergence in an explicit direction through
//! [`policy::converge`]: `file` re-pins the attested INDEX at the live evidence rev
//! (deploy the edited law) and writes it; `index` keeps the attested rev and
//! reports the file-side restore the live tree needs (writes nothing).
//!
//! Exit triad (§4 preamble): 0 = converged / drifted-fixed / preview (or a clean
//! `--truth` deploy), 1 = non-convergent / pending-agent, 2 = a tool failure (bad
//! usage, unreadable workspace, missing page, faulting run).

use std::collections::BTreeMap;
use std::path::Path;

use effects::EvalLimits;
use realise::{ApplyBinding, ClaimState, FieldEquals, RealiseSpec};
use serde_json::json;

use crate::{Fail, Format};

/// The finding leg of the triad (exit 1): the invocation was well-formed, the
/// claim did not converge (non-convergent or pending-agent).
const EXIT_FINDING: u8 = 1;

/// The default realise board directory pending-agent cards are born in (mirrors
/// `mrd attest`'s realised-gate default).
const DEFAULT_BOARD_DIR: &str = "board";

/// Run `mrd realise <tail>`.
///
/// # Errors
/// [`Fail`] on the triad's 1/2 legs — see the module docs for the mapping.
pub(crate) fn run(args: &[String]) -> Result<(), Fail> {
    let parsed = Parsed::parse(args)?;
    let root = crate::preset_cmd::resolve_root()?;

    if let Some(truth) = parsed.truth {
        return truth_deploy(&root, truth, &parsed);
    }

    let page = parsed
        .page
        .clone()
        .ok_or_else(|| Fail::tool("realise needs a PAGE argument".to_owned()))?;

    realise_page(&root, &page, &parsed)
}

/// The reconciliation loop over one page's declared claim.
fn realise_page(root: &fs::WorkspaceRoot, page: &str, parsed: &Parsed) -> Result<(), Fail> {
    let doc = fs::load(root, Path::new(page))
        .map_err(|e| Fail::tool(format!("cannot read page {page}: {e}")))?;

    let field = fm_scalar(&doc, "realise.field").ok_or_else(|| {
        Fail::tool(format!(
            "{page} declares no `realise.field` — not a realising page"
        ))
    })?;
    let expected = fm_scalar(&doc, "realise.expected").ok_or_else(|| {
        Fail::tool(format!(
            "{page} declares `realise.field` but no `realise.expected`"
        ))
    })?;

    let apply = fm_scalar(&doc, "realise.apply").map(|task| ApplyBinding {
        page: page.to_owned(),
        task,
        args: Vec::new(),
        env: BTreeMap::new(),
    });

    let claim = realise::Claim {
        selector: format!("{page}#{field}"),
        check: Box::new(FieldEquals {
            page: page.to_owned(),
            field: field.clone(),
            expected: expected.clone(),
        }),
        apply,
        // Parity with the legacy engine: exactly one apply, then one re-check.
        retry_budget: 1,
    };

    let (invocation_id, now) = mint_identity()?;
    let scratch = root.0.join(".meridian/scratch").join(&invocation_id);
    std::fs::create_dir_all(&scratch).map_err(|e| Fail::tool(format!("scratch dir: {e}")))?;
    let timeout = run::exec::configured_timeout(&root.0).map_err(|e| Fail::tool(e.to_string()))?;

    let spec = RealiseSpec {
        invocation_id,
        now: Some(now),
        actor: DEPLOY_ACTOR.to_owned(),
        board_dir: DEFAULT_BOARD_DIR.to_owned(),
        scratch: scratch.clone(),
        dry_run: parsed.dry,
        limits: EvalLimits::default(),
        timeout,
    };

    let report = realise::realise(root, std::slice::from_ref(&claim), &spec);
    let _ = std::fs::remove_dir_all(&scratch);
    let report = report.map_err(|e| Fail::tool(format!("realise faulted: {e}")))?;

    let result = &report.claims[0];
    let state = classify(parsed.dry, &result.state, result.applies);
    render(parsed.format, page, state, result.applies, &result.receipts);
    exit_for(state)
}

/// The parity-vocabulary terminal state, derived from the engine's A4 classifier
/// plus the apply count (the engine collapses `converged`/`drifted-fixed`, which
/// the legacy vocabulary splits by whether an apply fired).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// `--dry`: the phases that WOULD run; nothing executed, zero caps.
    Preview,
    /// The check held with no apply — the green terminal.
    Converged,
    /// The check drifted, one apply fixed it, the re-check is clean.
    DriftedFixed,
    /// The apply ran but the drift persists (budget exhausted).
    NonConvergent,
    /// The check drifted with no apply-capable claim in scope.
    PendingAgent,
}

impl State {
    fn as_str(self) -> &'static str {
        match self {
            State::Preview => "preview",
            State::Converged => "converged",
            State::DriftedFixed => "drifted-fixed",
            State::NonConvergent => "non-convergent",
            State::PendingAgent => "pending-agent",
        }
    }
}

/// Map the engine's [`ClaimState`] + apply count onto the legacy parity vocabulary.
fn classify(dry: bool, state: &ClaimState, applies: u32) -> State {
    if dry {
        return State::Preview;
    }
    match state {
        ClaimState::Converged if applies == 0 => State::Converged,
        ClaimState::Converged => State::DriftedFixed,
        ClaimState::NonConvergent => State::NonConvergent,
        ClaimState::PendingAgent { .. } => State::PendingAgent,
    }
}

/// The exit leg for a terminal state: converged / drifted-fixed / preview are
/// clean (0); non-convergent / pending-agent are findings (1).
fn exit_for(state: State) -> Result<(), Fail> {
    match state {
        State::Converged | State::DriftedFixed | State::Preview => Ok(()),
        State::NonConvergent => Err(Fail {
            code: EXIT_FINDING,
            message: "realise non-convergent — apply ran, drift persists (budget exhausted)"
                .to_owned(),
        }),
        State::PendingAgent => Err(Fail {
            code: EXIT_FINDING,
            message:
                "realise pending-agent — drift with no apply-capable claim; a board card was minted"
                    .to_owned(),
        }),
    }
}

/// Render one realise result — a human line or the `--json` object.
fn render(format: Format, page: &str, state: State, applies: u32, receipts: &[String]) {
    match format {
        Format::Json => println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "realise": {
                    "page": page,
                    "state": state.as_str(),
                    "applies": applies,
                    "receipts": receipts,
                }
            }))
            .expect("json")
        ),
        Format::Human => {
            println!("realise {page} — {}", state.as_str());
            if applies > 0 {
                // Show just the receipt anchors (`^r-NNNNNN`), not the whole
                // journal line — the full receipt rides `--json`.
                let anchors: Vec<&str> = receipts
                    .iter()
                    .filter_map(|line| line.rsplit(' ').next())
                    .collect();
                println!("  applies: {applies} (receipts: {})", anchors.join(", "));
            }
        }
    }
}

// ── `--truth index|file` — realise-as-deploy (U4.3) ──────────────────────────

/// Deploy a file↔index convention divergence in the stated direction through
/// [`policy::converge`]. `file` re-pins + writes the INDEX at the live rev; `index`
/// keeps the attested rev and reports the file-side restores (writes nothing).
fn truth_deploy(
    root: &fs::WorkspaceRoot,
    truth: policy::Truth,
    parsed: &Parsed,
) -> Result<(), Fail> {
    let index_path = root.0.join(policy::RESERVED_INDEX_PATH);
    let current = std::fs::read_to_string(&index_path).map_err(|e| {
        Fail::tool(format!(
            "cannot read the attested INDEX {}: {e}",
            policy::RESERVED_INDEX_PATH
        ))
    })?;
    let source = DiskConventions { root };
    let convergence = policy::converge(&current, &source, policy::CheckLimits::default(), truth)
        .map_err(|e| Fail::tool(e.to_string()))?;

    // file-truth deploys the edited law: write the re-pinned INDEX (realise-as
    // -deploy). index-truth keeps the attested INDEX and only reports the restores.
    //
    // U31: the INDEX is the ARMED POLICY the whole check plane reads its rules
    // from, and it used to land through a bare `std::fs::write` — no candidate,
    // and not even the crate's atomic tmp+fsync+rename discipline, so a crash
    // mid-write left a torn policy file. It now rides `fs::replace_file` like
    // every other whole-file write, which DEMANDS the sealed candidate: the
    // document this deploy is about to land is built and parsed before a byte
    // moves. (The flock and the rev-CAS this door still lacks are reported as
    // findings, not fixed here — the seal was U31's bound.)
    //
    // U35: and it JOURNALS. Sealing and journaling are different properties
    // (S3-R12(a)): U31 made this door unable to land bytes off-path, which is not
    // the same as leaving a receipt. Unjournaled, the deploy advanced the tree
    // root while the journal's last row still described the tree before it — so
    // `check`'s baseline went stale, both detectors refused `grey(cannot-assess)`,
    // and the installed fence refused the operator's next commit even though the
    // ONLY write in it was governed (U34 measured exactly that, both doors).
    let wrote = matches!(truth, policy::Truth::File) && !parsed.dry;
    if wrote {
        let candidate =
            model::candidate_of_body(policy::RESERVED_INDEX_PATH, convergence.index_after.clone());
        // The row's facts are read AROUND the write itself — the tree before, the
        // tree after, and the INDEX's own rev transition. `current` is the exact
        // pre-image `converge` decided against, so the recorded `before` rev is
        // the law that was deployed FROM, never a re-read that may have drifted.
        let rev_before = body_rev(&current);
        let root_before = tree_root(root)?;
        fs::replace_file(root, Path::new(policy::RESERVED_INDEX_PATH), &candidate)
            .map_err(|e| Fail::tool(format!("cannot write the converged INDEX: {e}")))?;
        let root_after = tree_root(root)?;
        // The §9 time fact the realise plane already mints; this door mints no
        // invocation id, so the identity's other half is deliberately dropped.
        let (_, now) = mint_identity()?;
        journal_deploy(
            root,
            &Deployed {
                now: &now,
                root_before: &root_before,
                root_after: &root_after,
                rev_before: &rev_before,
                rev_after: &candidate.document().root.node_rev.0,
            },
        )?;
    }

    render_truth(parsed.format, truth, &convergence, wrote);
    Ok(())
}

/// What one INDEX deploy did, as the journal records it: the §9 time fact, the
/// tree roots read around the write, and the INDEX's own rev transition. A struct
/// rather than six positional `&str`s — six same-typed parameters at one call
/// site is a transposition waiting to be written into the ledger.
struct Deployed<'a> {
    now: &'a str,
    root_before: &'a str,
    root_after: &'a str,
    rev_before: &'a str,
    rev_after: &'a str,
}

/// The actor this door records — the same name `realise_page` gives the engine,
/// spelled once so the ledger and the realise plane cannot drift apart.
const DEPLOY_ACTOR: &str = "mrd:realise";

/// **U35 — journal one `mrd realise --truth file` deploy.** The row rides U32's
/// row writer (`receipt::journal::render_row`); this door adds FACTS, never a
/// second renderer and never a second counter.
///
/// `op=realise` with a whole-file rev transition and `edits=0` is the `op=lock`
/// row shape (`wire-serve::lock_write`), for the same reason: the deploy replaces
/// a whole file, so its transition is file-grain and there are no node-grain edits
/// to list. The row is appended AFTER the bytes land, so a crash between the two
/// leaves a stale baseline — grey, the honest unknown — rather than a row claiming
/// a write that never happened.
///
/// # Errors
/// [`Fail`] (exit 2) when the journal page cannot be read or appended.
fn journal_deploy(root: &fs::WorkspaceRoot, deployed: &Deployed<'_>) -> Result<(), Fail> {
    let page = fs::read_journal_page(root)
        .map_err(|e| Fail::tool(format!("cannot read the receipt journal: {e}")))?;
    let line = receipt::journal::render_row(&receipt::journal::JournalRow {
        seq: receipt::journal::next_seq(&page),
        op: "realise",
        path: policy::RESERVED_INDEX_PATH,
        actor: Some(DEPLOY_ACTOR),
        now: Some(deployed.now),
        root_before: deployed.root_before,
        root_after: deployed.root_after,
        file: Some(receipt::journal::FileTransition {
            before: Some(deployed.rev_before),
            after: Some(deployed.rev_after),
        }),
        edits: Vec::new(),
    });
    fs::append_line(root, Path::new(fs::domain::RESERVED_JOURNAL_PATH), &line)
        .map_err(|e| Fail::tool(format!("cannot append the deploy's journal row: {e}")))
}

/// The live workspace tree root — the SAME `fs::domain_snapshot` fold `check`'s
/// baseline detector reads (`check::layer0::journal_trace`). A row written against
/// any other fold would date the tree in a unit its reader does not use.
///
/// # Errors
/// [`Fail`] (exit 2) when the domain snapshot cannot be read or folded.
fn tree_root(root: &fs::WorkspaceRoot) -> Result<String, Fail> {
    Ok(fs::domain_snapshot(root)
        .map_err(|e| Fail::tool(format!("cannot fold the workspace tree: {e}")))?
        .1
        .0)
}

/// The whole-file rev of a page's bytes, through the document build every other
/// rev in the system comes from — never a second hash of the same bytes.
fn body_rev(body: &str) -> String {
    model::candidate_of_body(policy::RESERVED_INDEX_PATH, body.to_owned())
        .document()
        .root
        .node_rev
        .0
        .clone()
}

/// Render a `--truth` convergence: the direction, whether the INDEX was written,
/// and the file-side actions the live tree still needs.
fn render_truth(
    format: Format,
    truth: policy::Truth,
    convergence: &policy::Convergence,
    wrote: bool,
) {
    let dir = match truth {
        policy::Truth::File => "file",
        policy::Truth::Index => "index",
    };
    match format {
        Format::Json => {
            let actions: Vec<_> = convergence
                .file_actions
                .iter()
                .map(|a| match a {
                    policy::FileAction::RestoreFile { slug, to_rev } => {
                        json!({ "action": "restore_file", "slug": slug, "to_rev": to_rev })
                    }
                    policy::FileAction::MissingFile { slug, to_rev } => {
                        json!({ "action": "missing_file", "slug": slug, "to_rev": to_rev })
                    }
                })
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "realise_truth": {
                        "truth": dir,
                        "index_written": wrote,
                        "file_actions": actions,
                    }
                }))
                .expect("json")
            );
        }
        Format::Human => {
            let verb = if wrote {
                "INDEX re-pinned + written (deploy)"
            } else {
                "INDEX unchanged"
            };
            println!("realise --truth {dir} — {verb}");
            for action in &convergence.file_actions {
                match action {
                    policy::FileAction::RestoreFile { slug, to_rev } => {
                        println!("  restore {slug} → attested rev {to_rev}");
                    }
                    policy::FileAction::MissingFile { slug, to_rev } => {
                        println!(
                            "  missing {slug} — recreate at attested rev {to_rev} (or disarm)"
                        );
                    }
                }
            }
        }
    }
}

/// A disk-backed [`policy::ConventionSource`]: reads `conventions/<slug>/` under
/// the workspace root (the same seam `wire-serve`'s gate injects — policy does no
/// I/O). The convergence decision all lives in `policy`; this hands over bytes.
struct DiskConventions<'a> {
    root: &'a fs::WorkspaceRoot,
}

impl policy::ConventionSource for DiskConventions<'_> {
    fn files_for<'a>(&'a self, slug: &str) -> Box<dyn policy::ConventionFiles + 'a> {
        Box::new(DiskConventionFolder {
            base: self.root.0.join("conventions").join(slug),
        })
    }
}

/// One convention folder on disk (`conventions/<slug>/`).
struct DiskConventionFolder {
    base: std::path::PathBuf,
}

impl policy::ConventionFiles for DiskConventionFolder {
    fn read(&self, rel: &str) -> std::io::Result<String> {
        std::fs::read_to_string(self.base.join(rel))
    }
    fn exists(&self, rel: &str) -> bool {
        self.base.join(rel).exists()
    }
}

// ── helpers ──────────────────────────────────────────────────────────────────

/// Read a scalar frontmatter value off a parsed document (the public
/// [`model::YamlMap`] surface) — dotted keys (`realise.field`) are ordinary map
/// entries, exactly as `mrd run` reads `task.<name>`.
fn fm_scalar(doc: &model::Document, key: &str) -> Option<String> {
    fn find(node: &model::Node) -> Option<&model::YamlMap> {
        if let model::NodeKind::Frontmatter { map } = &node.kind {
            return Some(map);
        }
        node.children.iter().find_map(find)
    }
    find(&doc.root)
        .and_then(|m| m.0.iter().find(|(k, _)| k == key))
        .map(|(_, v)| v.clone())
}

/// Mint the realise identity at the §9 boundary: a unique, path-safe invocation id
/// and a unix-seconds time fact (the same shape `mrd run` mints).
fn mint_identity() -> Result<(String, String), Fail> {
    let elapsed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| Fail::tool(format!("system clock: {e}")))?;
    let secs = elapsed.as_secs();
    let id = format!(
        "realise-{secs}{:03}-{}",
        elapsed.subsec_millis(),
        std::process::id()
    );
    Ok((id, secs.to_string()))
}

/// The parsed `realise` invocation.
struct Parsed {
    page: Option<String>,
    dry: bool,
    truth: Option<policy::Truth>,
    format: Format,
}

impl Parsed {
    fn parse(args: &[String]) -> Result<Self, Fail> {
        let mut page: Option<String> = None;
        let mut dry = false;
        let mut truth = None;
        let mut json = false;
        let mut it = args.iter();
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "--dry" | "--dry-run" => dry = true,
                "--json" => json = true,
                "--truth" => {
                    let val = it
                        .next()
                        .ok_or_else(|| Fail::tool("--truth needs `index` or `file`".to_owned()))?;
                    truth = Some(match val.as_str() {
                        "index" => policy::Truth::Index,
                        "file" => policy::Truth::File,
                        other => {
                            return Err(Fail::tool(format!(
                                "--truth must be `index` or `file`, got `{other}`"
                            )));
                        }
                    });
                }
                flag if flag.starts_with('-') => {
                    return Err(Fail::tool(format!("unknown flag: {flag}")));
                }
                value if page.is_none() => page = Some(value.to_owned()),
                value => return Err(Fail::tool(format!("unexpected argument: {value}"))),
            }
        }
        Ok(Parsed {
            page,
            dry,
            truth,
            format: if json { Format::Json } else { Format::Human },
        })
    }
}
