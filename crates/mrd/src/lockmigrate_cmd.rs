//! `mrd lock migrate --vault <path> [--dry] [--json]` (U9b, SELF-RETIRING) —
//! the one-shot field migration of `meridian-lock` blocks from v1 to R4 v2.
//!
//! A local CLIENT of `lockmigrate::sweep`: mrd names the vault, dials the tool,
//! and renders the house grammar (a human report by default, JSON under
//! `--json`). Every rewrite lands through the governed
//! `wire_serve::write::lock_migrate` door — this verb writes nothing itself.
//!
//! This subcommand DIES with the migration; its removal is in U9b's definition
//! of done. Grep `SELF-RETIRING`.
//!
//! # `--vault` is REQUIRED, and that is a safety property
//! Every other mrd verb resolves the workspace from the cwd. This one refuses
//! to: it rewrites files in a live vault, and the difference between the vault
//! you meant and the one you were standing in is the difference between a
//! migration and an incident. Naming it is cheap; guessing it is not.
//!
//! Exit codes (§4 preamble; `docs/status.md`): 0 = clean (every engine-placed v1
//! lock migrated, or nothing to do), 1 = findings (a page was REFUSED — the
//! migration is not complete), 2 = a tool failure (bad usage, an unreadable
//! vault, a vault with no git and therefore no restore point, a refused write).

use lockmigrate::{Options, sweep};
use serde_json::json;

use crate::{Fail, Format};

/// Run `mrd lock migrate --vault <path> [--dry] [--json]`.
///
/// # Errors
/// A tool failure (exit 2), or a findings exit (1) when any page was refused.
pub(crate) fn run(args: &[String]) -> Result<(), Fail> {
    let parsed = Parsed::parse(args)?;
    let canonical = workspace::canonicalize(std::path::Path::new(&parsed.vault))
        .map_err(|e| Fail::tool(format!("cannot resolve vault {} ({e})", parsed.vault)))?;
    let root = fs::WorkspaceRoot(canonical);

    let opts = Options {
        dry: parsed.dry,
        actor: None,
        now: None,
    };
    let report = sweep(&root, &opts).map_err(|e| Fail::tool(e.to_string()))?;

    match parsed.format {
        Format::Json => println!(
            "{}",
            serde_json::to_string_pretty(&json!({ "lock_migrate": report })).expect("json")
        ),
        Format::Human => print!("{}", report.render()),
    }

    if report.refusals() > 0 {
        return Err(Fail::findings(format!(
            "lock-migrate: {} page(s) REFUSED — the migration is not complete",
            report.refusals()
        )));
    }
    Ok(())
}

/// The parsed `lock-migrate` tail.
struct Parsed {
    vault: String,
    dry: bool,
    format: Format,
}

impl Parsed {
    fn parse(args: &[String]) -> Result<Self, Fail> {
        let mut vault: Option<String> = None;
        let mut dry = false;
        let mut json = false;
        let mut it = args.iter();
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "--vault" => {
                    let v = it
                        .next()
                        .ok_or_else(|| Fail::tool("--vault needs a path".to_owned()))?;
                    vault = Some(v.clone());
                }
                "--dry" => dry = true,
                "--json" => json = true,
                other => return Err(Fail::tool(format!("unexpected argument: {other}"))),
            }
        }
        let vault = vault.ok_or_else(|| {
            Fail::tool(
                "lock-migrate needs --vault <path> — this verb rewrites a live vault and \
                 will not guess which one"
                    .to_owned(),
            )
        })?;
        Ok(Parsed {
            vault,
            dry,
            format: if json { Format::Json } else { Format::Human },
        })
    }
}
