//! `mrd realise <page> [--dry] [--json]`: the reconciliation-loop verb — observe → check →
//! apply (only on drift) → re-check over one page's declared claim, wiring [`realise::realise`].
//! The apply rides the ordinary run plane ([`run::runner::run`]); there is no second execution
//! path.
//!
//! # The claim grammar
//! A realising page declares its claim in frontmatter:
//!
//! - `realise.field` + `realise.expected` — the check: converged iff the frontmatter field
//!   equals the expected value. Pure read, no cap.
//! - `realise.apply` — the apply task name addressed on the page (`task.<name>`); absent means
//!   the claim is not apply-capable and a drift classifies `pending-agent`.
//! - `realise.rule` — the id of the rule this claim realises. A pending-agent card references it
//!   by id and never embeds the rule body. Optional; validated against the `policy::RuleId`
//!   grammar at parse time.
//! - `realise.board_dir` — the workspace-relative directory pending-agent cards are born in.
//!   Optional; defaults to `board`. The folder a user files cards in is THEIR flow, so it is read
//!   off their markdown and only defaulted in code (`docs/laws.md` § Amendment — no hard-coded
//!   flow, rule 1).
//!
//! The retry budget is one apply then one re-check.
//!
//! # Terminal states
//! `converged` (check clean, no apply) · `drifted-fixed` (apply fixed the drift) ·
//! `non-convergent` (apply ran, drift persists) · `pending-agent` (drift, no apply-capable
//! claim) · `preview` (`--dry`).
//!
//! Exit triad: 0 = converged / drifted-fixed / preview, 1 = non-convergent / pending-agent,
//! 2 = a tool failure (bad usage, unreadable workspace, missing page, faulting run).

use std::collections::BTreeMap;
use std::path::Path;

use effects::EvalLimits;
use realise::{ApplyBinding, ClaimState, FieldEquals, RealiseSpec};
use serde_json::json;

use crate::{Fail, Format};

/// The finding leg of the triad (exit 1): the invocation was well-formed, the
/// claim did not converge (non-convergent or pending-agent).
const EXIT_FINDING: u8 = 1;

/// The fallback realise board directory pending-agent cards are born in when the page declares
/// no `realise.board_dir` (mirrors `mrd attest`'s realised-gate default).
const DEFAULT_BOARD_DIR: &str = "board";

/// The actor this verb records on every write it drives through the run plane.
const REALISE_ACTOR: &str = "mrd:realise";

/// Run `mrd realise <tail>`. Errors [`Fail`] on the triad's 1/2 legs — see the module docs for
/// the mapping.
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

    // A pending-agent card references the rule by id and never copies its body, so the id is
    // validated here, at the page edge, rather than minting a card pointing at nothing parseable.
    let rule = match fm_scalar(&doc, "realise.rule") {
        None => None,
        Some(id) => {
            policy::RuleId::parse(&id).map_err(|e| {
                Fail::tool(format!(
                    "{page} declares `realise.rule: {id}` — not a rule id: {e:?}"
                ))
            })?;
            Some(id)
        }
    };

    let board_dir = board_dir(&doc, page)?;

    let claim = realise::Claim {
        selector: format!("{page}#{field}"),
        rule,
        check: Box::new(FieldEquals {
            page: page.to_owned(),
            field: field.clone(),
            expected: expected.clone(),
        }),
        apply,
        // Exactly one apply, then one re-check.
        retry_budget: 1,
    };

    let (invocation_id, now) = mint_identity()?;
    let scratch = root.0.join(".meridian/scratch").join(&invocation_id);
    std::fs::create_dir_all(&scratch).map_err(|e| Fail::tool(format!("scratch dir: {e}")))?;
    // `resolve_root` has already committed to a workspace, so unlike `mrd run` there is no
    // unanswered case here — this root is the one that declares.
    let declaring_root = Some(root.0.clone());
    let timeout = run::exec::configured_timeout(declaring_root.as_deref())
        .map_err(|e| Fail::tool(e.to_string()))?;

    let spec = RealiseSpec {
        invocation_id,
        now: Some(now),
        actor: REALISE_ACTOR.to_owned(),
        board_dir,
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

/// The terminal state, derived from the engine's classifier plus the apply count — the engine
/// collapses `converged`/`drifted-fixed`, which this vocabulary splits by whether an apply fired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// `--dry`: the phases that would run; nothing executed, zero caps.
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

/// Map the engine's [`ClaimState`] + apply count onto the terminal vocabulary.
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
        State::NonConvergent => Err(Fail::with_code(
            EXIT_FINDING,
            "realise non-convergent — apply ran, drift persists (budget exhausted)".to_owned(),
        )),
        State::PendingAgent => Err(Fail::with_code(
            EXIT_FINDING,
            "realise pending-agent — drift with no apply-capable claim; a board card was minted"
                .to_owned(),
        )),
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

/// Read a scalar frontmatter value off a parsed document, DECODED through the one scalar owner
/// (wire-contract § A.6.1). Dotted keys (`realise.field`) are ordinary map entries, exactly as
/// `mrd run` reads `task.<name>`. This is the DECLARED half of the `FieldEquals` comparison —
/// `realise.expected` — and it decodes for the same reason the observed half does: a
/// fleet-canonical `realise.expected: "done"` read raw never equals an observed `done`.
fn fm_scalar(doc: &model::Document, key: &str) -> Option<String> {
    fn find(node: &model::Node) -> Option<&model::YamlMap> {
        if let model::NodeKind::Frontmatter { map } = &node.kind {
            return Some(map);
        }
        node.children.iter().find_map(find)
    }
    find(&doc.root)
        .and_then(|m| m.0.iter().find(|(k, _)| k == key))
        .map(|(_, v)| model::scalar::text(v))
}

/// The workspace-relative directory pending-agent cards are born in: the page's own
/// `realise.board_dir`, or [`DEFAULT_BOARD_DIR`] when it declares none. A declared-but-blank value
/// is refused loud rather than defaulted — an author who wrote the key meant a folder, and
/// silently minting into `board/` would file their cards where they are not looking.
fn board_dir(doc: &model::Document, page: &str) -> Result<String, Fail> {
    let Some(declared) = fm_scalar(doc, "realise.board_dir") else {
        return Ok(DEFAULT_BOARD_DIR.to_owned());
    };
    let trimmed = declared.trim();
    if trimmed.is_empty() {
        return Err(Fail::tool(format!(
            "{page} declares an empty `realise.board_dir` — name a directory or drop the key"
        )));
    }
    Ok(trimmed.to_owned())
}

/// Mint the realise identity: a unique, path-safe invocation id and an RFC3339 time fact. The
/// clock is read here, once, and passed in — the engine reads none (`wire::now_is_rfc3339`
/// validates, never generates).
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
    Ok((id, rfc3339_utc(secs)))
}

/// Format unix seconds as an RFC3339 UTC date-time (`YYYY-MM-DDTHH:MM:SSZ`) — the `now` format
/// law `wire::now_is_rfc3339` validates. Pure and dependency-free (the workspace carries no date
/// crate); the civil-date split is Hinnant's `civil_from_days`, exact for every day in the
/// proleptic Gregorian calendar.
fn rfc3339_utc(secs: u64) -> String {
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let (hour, min, sec) = (rem / 3600, (rem % 3600) / 60, rem % 60);

    // Shift the epoch to 0000-03-01 so leap day lands at the end of the era.
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097; // day of era, [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // March-based month, [0, 11]
    let day = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = era * 400 + yoe + u64::from(month <= 2);

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{min:02}:{sec:02}Z")
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The minted clock satisfies the engine's own format predicate.
    #[test]
    fn minted_now_is_rfc3339() {
        let (_id, now) = mint_identity().expect("system clock");
        assert!(wire::now_is_rfc3339(&now), "now={now:?}");
    }

    /// Known instants, including the leap-year and century-rule edges the
    /// civil-date split has to get right.
    #[test]
    fn rfc3339_utc_formats_known_instants() {
        for (secs, expect) in [
            (0, "1970-01-01T00:00:00Z"),
            (1, "1970-01-01T00:00:01Z"),
            (86_399, "1970-01-01T23:59:59Z"),
            (86_400, "1970-01-02T00:00:00Z"),
            (951_782_400, "2000-02-29T00:00:00Z"), // leap year, century rule
            (1_709_164_800, "2024-02-29T00:00:00Z"),
            (1_753_264_800, "2025-07-23T10:00:00Z"),
            (4_102_444_800, "2100-01-01T00:00:00Z"), // 2100 is NOT a leap year
        ] {
            assert_eq!(rfc3339_utc(secs), expect, "secs={secs}");
        }
    }

    /// Every formatted instant validates across the whole span the calendar split covers, not
    /// only the sampled instants.
    #[test]
    fn rfc3339_utc_output_always_validates() {
        let mut secs: u64 = 0;
        while secs < 4_200_000_000 {
            let s = rfc3339_utc(secs);
            assert!(wire::now_is_rfc3339(&s), "secs={secs} → {s}");
            secs += 1_000_003; // a prime-ish stride: walks every weekday/month
        }
    }
}
