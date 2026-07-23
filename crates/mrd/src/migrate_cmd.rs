//! `mrd migrate inputs [--dry]` (U3.2, SELF-RETIRING) — the one-shot mechanical
//! corpus rename `draws-from:` → `inputs:` (d1 § Migration plan). A local CLIENT
//! of `migrate::migrate_inputs`: mrd resolves the workspace, dials the kit, and
//! renders the house grammar (a human table by default, JSON under `--json`).
//!
//! This subcommand DIES with the migration (its removal is in the pin leg's
//! definition of done; the delete rides the U3.3 storm). Grep `SELF-RETIRING`.
//!
//! Exit codes (§4 preamble; `docs/status.md`): 0 = clean (every `draws-from:`
//! renamed, or nothing to do), 1 = a finding (a page carries BOTH keys — a
//! conflict a human must resolve), 2 = a tool failure (bad usage, unreadable
//! workspace, refused write).

use migrate::{MigrateOptions, MigrateReport, migrate_inputs};
use serde_json::json;

use crate::{Fail, Format, current_dir};

/// Dispatch `mrd migrate <sub>` — only `inputs` exists.
pub(crate) fn dispatch(args: &[String]) -> Result<(), Fail> {
    let Some(sub) = args.first() else {
        return Err(Fail::tool("migrate needs a subcommand (inputs)".to_owned()));
    };
    match sub.as_str() {
        "inputs" => run_inputs(&args[1..]),
        other => Err(Fail::tool(format!("unknown migrate subcommand: {other}"))),
    }
}

/// Run `mrd migrate inputs [--dry] [--json]`.
///
/// # Errors
/// A tool failure (exit 2) — bad usage, an unreadable workspace, or a refused
/// write — or a findings exit (1) when any page carries both keys.
fn run_inputs(args: &[String]) -> Result<(), Fail> {
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

    let opts = MigrateOptions {
        dry: parsed.dry,
        actor: None,
        now: None,
    };
    let report = migrate_inputs(&root, &opts).map_err(|e| Fail::tool(e.to_string()))?;

    match parsed.format {
        Format::Json => print_json(&report),
        Format::Human => print_human(&report),
    }

    // A conflict page is a finding (exit 1); an all-clean run is exit 0.
    if report.conflicts() > 0 {
        return Err(Fail::findings(format!(
            "migrate: {} page(s) carry both draws-from: and inputs: — resolve by hand",
            report.conflicts()
        )));
    }
    Ok(())
}

/// Print the migration report as JSON (the machine face).
fn print_json(report: &MigrateReport) {
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({ "migrate": report })).expect("json")
    );
}

/// Print the migration report as a human table.
fn print_human(report: &MigrateReport) {
    let mode = if report.dry { "dry" } else { "migrate" };
    println!(
        "migrate inputs [{mode}] — {} renamed, {} conflict(s)",
        report.renamed(),
        report.conflicts()
    );
    for page in &report.pages {
        match page {
            migrate::PageOutcome::Renamed { path, receipt } => match receipt {
                Some(r) => println!("  {path} — draws-from -> inputs  ^{r}"),
                None => println!("  {path} — draws-from -> inputs  (dry)"),
            },
            migrate::PageOutcome::Conflict { path } => {
                println!("  {path} — CONFLICT: both draws-from: and inputs: present (refused)");
            }
        }
    }
}

/// The parsed `migrate inputs` tail: `--dry`, `--json`.
struct Parsed {
    dry: bool,
    format: Format,
}

impl Parsed {
    fn parse(args: &[String]) -> Result<Self, Fail> {
        let mut dry = false;
        let mut json = false;
        for arg in args {
            match arg.as_str() {
                "--dry" => dry = true,
                "--json" => json = true,
                other => return Err(Fail::tool(format!("unexpected argument: {other}"))),
            }
        }
        Ok(Parsed {
            dry,
            format: if json { Format::Json } else { Format::Human },
        })
    }
}
