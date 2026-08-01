//! `mrd realise <page> [--dry] [--json]` (U3.5b): the
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
//! # `--truth index|file` is GONE (registration cutover)
//! This verb used to carry a second job: realise-as-deploy, resolving a
//! file↔INDEX convention divergence in an explicit direction through
//! `policy::converge`. Its whole subject was `conventions/INDEX.md`, which stopped
//! being engine substrate — there is no INDEX to re-pin and no folder to restore,
//! and `policy::binding::converge` was deleted rather than re-keyed. The
//! convergence law is re-owed at the ARM disk edge, where the artifact is written.
//!
//! The flag is REMOVED, not kept as a refusal: a flag whose only behaviour is to
//! explain itself is a compat door in sentiment, and it would keep the deploy
//! shape alive in every reader's head long after the thing it deployed was gone.
//!
//! Exit triad (§4 preamble): 0 = converged / drifted-fixed / preview, 1 =
//! non-convergent / pending-agent, 2 = a tool failure (bad usage, unreadable
//! workspace, missing page, faulting run).

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

/// The actor this verb records on every write it drives through the run plane.
const REALISE_ACTOR: &str = "mrd:realise";

/// Run `mrd realise <tail>`.
///
/// # Errors
/// [`Fail`] on the triad's 1/2 legs — see the module docs for the mapping.
pub(crate) fn run(args: &[String]) -> Result<(), Fail> {
    let parsed = Parsed::parse(args)?;
    let root = crate::preset_cmd::resolve_root()?;

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
    // `resolve_root` has already committed to a workspace (ladder, daemon
    // adoption, or ephemeral), so unlike `mrd run` there is no unanswered case
    // to represent here — this root is the one that declares. Substituting the
    // declaration for the retired marker keeps today's behavior: the same
    // directory is consulted, only the filename and the parse law change.
    let declaring_root = Some(root.0.clone());
    let timeout = run::exec::configured_timeout(declaring_root.as_deref())
        .map_err(|e| Fail::tool(e.to_string()))?;

    let spec = RealiseSpec {
        invocation_id,
        now: Some(now),
        actor: REALISE_ACTOR.to_owned(),
        board_dir: DEFAULT_BOARD_DIR.to_owned(),
        scratch: scratch.clone(),
        dry_run: parsed.dry,
        limits: EvalLimits::default(),
        timeout,
        declaring_root,
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
    format: Format,
}

impl Parsed {
    fn parse(args: &[String]) -> Result<Self, Fail> {
        let mut page: Option<String> = None;
        let mut dry = false;
        let mut json = false;
        for arg in args {
            match arg.as_str() {
                "--dry" | "--dry-run" => dry = true,
                "--json" => json = true,
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
            format: if json { Format::Json } else { Format::Human },
        })
    }
}
