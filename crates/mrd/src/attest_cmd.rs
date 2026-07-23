//! `mrd attest <PAGE> [--dry] [--board DIR]` (U2.5): `attest = realised-gate +
//! pin + receipt`. A local CLIENT of `attest::attest` — mrd holds no markdown
//! semantics (docs/laws.md charter); it resolves the workspace, evaluates the
//! realised-gate over the realise board (`--board`, default `board/`), dials the
//! op, and renders the house grammar (a human table by default, JSON under
//! `--json`).
//!
//! Exit codes (§4 preamble; `docs/status.md`): 0 = attested (pinned / re-pinned
//! drift / idempotent no-op), 1 = attest refused — class (a) resolution, (b) the
//! realised-gate, or (c) a false `check_claim` (a `dry` `would_refuse` all the
//! same), 2 = a tool failure (bad usage, unreadable workspace, missing page,
//! faulting gate, refused write).

use attest::board::BoardGate;
use attest::{AttestOptions, AttestRefusal, AttestResult, attest};
use serde_json::json;

use crate::{Fail, Format, current_dir};

/// The default realise board directory the realised-gate reads pending-agent
/// cards from (one card per unrealised claim; `--board` overrides).
const DEFAULT_BOARD_DIR: &str = "board";

/// Exit 1 — a finding (attest refused). Distinct from `Fail::tool` (exit 2).
fn refused(message: String) -> Fail {
    Fail { code: 1, message }
}

/// Run `mrd attest <PAGE> [--dry] [--board DIR] [--json]`.
///
/// # Errors
/// A tool failure (exit 2) — bad usage, an unreadable workspace, a missing page,
/// a faulting realised-gate, or a refused/failed write — or an attest refusal
/// (exit 1) rendered from the refusing class.
pub(crate) fn run(args: &[String]) -> Result<(), Fail> {
    let parsed = Parsed::parse(args)?;
    let cwd = current_dir()?;
    let resolved = crate::resolve::resolve_runtime(&cwd).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace for {}: {e}",
            cwd.display()
        ))
    })?;
    let canonical = workspace::canonicalize(&resolved.workspace).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace {} ({e})",
            resolved.workspace.display()
        ))
    })?;
    let root = fs::WorkspaceRoot(canonical);

    let opts = AttestOptions {
        dry: parsed.dry,
        actor: None,
        now: None,
    };
    let gate = BoardGate::new(parsed.board);
    let result =
        attest(&root, &parsed.page, &gate, &opts).map_err(|e| Fail::tool(e.to_string()))?;

    match parsed.format {
        Format::Json => print_json(&result),
        Format::Human => print_human(&result),
    }

    // Exit code: a refusal (or dry would_refuse) is a finding (1); an attest is clean.
    match &result {
        AttestResult::Attested(_) => Ok(()),
        AttestResult::Refused(r) => Err(refused(format!(
            "attest {} {}",
            if r.would_refuse() {
                "would refuse"
            } else {
                "refused"
            },
            r.target()
        ))),
    }
}

/// Print the attest result as JSON (the machine face).
fn print_json(result: &AttestResult) {
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({ "attest": result })).expect("json")
    );
}

/// Print the attest result as a human table.
fn print_human(result: &AttestResult) {
    match result {
        AttestResult::Attested(report) => {
            let state = if report.pin.dry {
                "dry"
            } else if report.pin.wrote {
                "attested"
            } else {
                "unchanged"
            };
            println!("attest {} [{state}]", report.target);
            if let Some(r) = &report.pin.receipt {
                println!("  receipt: ^{r}");
            }
            for item in &report.pin.items {
                let rev = item.rev.as_deref().unwrap_or("-");
                println!(
                    "  {} -> {} {} {}",
                    item.declared_ref, item.to, item.color, rev
                );
            }
        }
        AttestResult::Refused(AttestRefusal::RealisedGate(gate)) => {
            let verb = if gate.would_refuse {
                "would refuse"
            } else {
                "refused"
            };
            println!("attest {} [{verb}]", gate.target);
            let receipt = gate.last_receipt.as_deref().unwrap_or("(none)");
            println!(
                "  realised-gate unmet — claim {} is {} (last receipt ^{receipt}) [{}/{}]",
                gate.claim, gate.state, gate.code, gate.recovery
            );
        }
        AttestResult::Refused(AttestRefusal::Pin(refusal)) => {
            let verb = if refusal.would_refuse {
                "would refuse"
            } else {
                "refused"
            };
            println!("attest {} [{verb}]", refusal.target);
            for item in &refusal.items {
                match &item.reason {
                    Some(reason) => {
                        println!(
                            "  {} — {} [{}/{}]",
                            item.declared_ref, reason.message, reason.code, reason.recovery
                        );
                        if !reason.candidates.is_empty() {
                            println!("      candidates: {}", reason.candidates.join(", "));
                        }
                    }
                    None => println!("  {} — (would resolve)", item.declared_ref),
                }
            }
        }
    }
}

/// The parsed `attest` tail: the required `PAGE`, `--dry`, `--board DIR`, `--json`.
struct Parsed {
    page: String,
    dry: bool,
    board: String,
    format: Format,
}

impl Parsed {
    fn parse(args: &[String]) -> Result<Self, Fail> {
        let mut page: Option<String> = None;
        let mut dry = false;
        let mut board = DEFAULT_BOARD_DIR.to_owned();
        let mut json = false;
        let mut it = args.iter();
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "--dry" => dry = true,
                "--json" => json = true,
                "--board" => {
                    board
                        .clone_from(it.next().ok_or_else(|| {
                            Fail::tool("--board needs a DIR argument".to_owned())
                        })?);
                }
                flag if flag.starts_with('-') => {
                    return Err(Fail::tool(format!("unknown flag: {flag}")));
                }
                value if page.is_none() => page = Some(value.to_owned()),
                value => return Err(Fail::tool(format!("unexpected argument: {value}"))),
            }
        }
        let page = page.ok_or_else(|| Fail::tool("attest needs a PAGE argument".to_owned()))?;
        Ok(Parsed {
            page,
            dry,
            board,
            format: if json { Format::Json } else { Format::Human },
        })
    }
}
