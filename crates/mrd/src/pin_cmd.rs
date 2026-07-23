//! `mrd pin <PAGE> [--dry]` (U2.4): read a page's `inputs:` manifest, resolve +
//! compose every ref atomically, evaluate declared `check:` predicates, and
//! splice the `^inputs` lock in ONE CAS-guarded write minting one receipt. A
//! local CLIENT of `pin::pin` — mrd holds no markdown semantics (docs/laws.md
//! charter); it resolves the workspace, dials the op, and renders the house
//! grammar (a human table by default, JSON under `--json`).
//!
//! Exit codes (§4 preamble; `docs/status.md`): 0 = pinned (or an idempotent
//! no-op), 1 = the pin refused (a finding — or a `dry` `would_refuse`), 2 = a
//! tool failure (bad usage, unreadable workspace, missing page, refused write).

use pin::{PinOptions, PinResult, pin};
use serde_json::json;

use crate::{Fail, Format, current_dir};

/// Exit 1 — a finding (the pin refused). Distinct from `Fail::tool` (exit 2).
fn refused(message: String) -> Fail {
    Fail { code: 1, message }
}

/// Run `mrd pin <PAGE> [--dry] [--json]`.
///
/// # Errors
/// A tool failure (exit 2) — bad usage, an unreadable workspace, a missing page,
/// or a refused/failed write — or a pin refusal (exit 1) rendered from the
/// per-item reasons.
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

    let opts = PinOptions {
        dry: parsed.dry,
        actor: None,
        now: None,
    };
    let result = pin(&root, &parsed.page, &opts).map_err(|e| Fail::tool(e.to_string()))?;

    match parsed.format {
        Format::Json => print_json(&result),
        Format::Human => print_human(&result),
    }

    // Exit code: a refusal (or dry would_refuse) is a finding (1); a pin is clean.
    match &result {
        PinResult::Pinned(_) => Ok(()),
        PinResult::Refused(r) => Err(refused(format!(
            "pin {} — {} ref(s) refused",
            if r.would_refuse {
                "would refuse"
            } else {
                "refused"
            },
            r.items.iter().filter(|i| i.reason.is_some()).count()
        ))),
    }
}

/// Print the pin result as JSON (the machine face).
fn print_json(result: &PinResult) {
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({ "pin": result })).expect("json")
    );
}

/// Print the pin result as a human table.
fn print_human(result: &PinResult) {
    match result {
        PinResult::Pinned(report) => {
            let state = if report.dry {
                "dry"
            } else if report.wrote {
                "pinned"
            } else {
                "unchanged"
            };
            println!("pin {} [{state}]", report.target);
            if let Some(r) = &report.receipt {
                println!("  receipt: ^{r}");
            }
            for item in &report.items {
                let rev = item.rev.as_deref().unwrap_or("-");
                println!(
                    "  {} -> {} {} {}",
                    item.declared_ref, item.to, item.color, rev
                );
            }
            for s in &report.id_mint {
                println!(
                    "  hint: mint a ^block-id for '{}' before the {} dependents re-pin (E5 storm)",
                    s.target, s.dependents
                );
            }
        }
        PinResult::Refused(refusal) => {
            let verb = if refusal.would_refuse {
                "would refuse"
            } else {
                "refused"
            };
            println!("pin {} [{verb}]", refusal.target);
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

/// The parsed `pin` tail: the required `PAGE`, `--dry`, `--json`.
struct Parsed {
    page: String,
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
                "--dry" => dry = true,
                "--json" => json = true,
                flag if flag.starts_with('-') => {
                    return Err(Fail::tool(format!("unknown flag: {flag}")));
                }
                value if page.is_none() => page = Some(value.to_owned()),
                value => return Err(Fail::tool(format!("unexpected argument: {value}"))),
            }
        }
        let page = page.ok_or_else(|| Fail::tool("pin needs a PAGE argument".to_owned()))?;
        Ok(Parsed {
            page,
            dry,
            format: if json { Format::Json } else { Format::Human },
        })
    }
}
