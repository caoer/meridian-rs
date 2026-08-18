//! `mrd unfold <preset> [--dry] [--actor A] [--now T]` (U5.3): materialize a presets declared
//! scaffold — every `Unfold` file is born through the U2.6 guarded create, so each carries a
//! birth receipt; an occupied path refuses via the `if_absent` CAS and is left byte-untouched.

use preset::{BirthOptions, FileOutcome, UnfoldReport};
use serde_json::json;

use crate::{Fail, Format};

/// Exit 1 — a finding (a scaffold path already existed). Distinct from
/// `Fail::tool` (exit 2).
fn findings(message: String) -> Fail {
    Fail::findings(message)
}

/// Run `mrd unfold <preset> [--dry] [--actor A] [--now T] [--json]`.
///
/// # Errors
/// A tool failure (exit 2) — bad usage, an unreadable workspace, a preset that is not a def, or
/// a faulting write — or a findings exit (1) when a declared scaffold path already existed.
pub(crate) fn run(args: &[String]) -> Result<(), Fail> {
    let parsed = Parsed::parse(args)?;
    // Not yet converted to the rooted lane — see [`crate::preset_cmd::refuse_rooted`].
    crate::preset_cmd::refuse_rooted(&parsed.preset, "unfold", "Nothing was born.")?;
    let root = crate::preset_cmd::resolve_root()?;
    let preset_path = crate::preset_cmd::def_path(&parsed.preset);

    let opts = BirthOptions {
        actor: parsed.actor,
        now: parsed.now,
        dry: parsed.dry,
    };
    let report =
        preset::unfold(&root, &preset_path, &opts).map_err(|e| Fail::tool(e.to_string()))?;

    match parsed.format {
        Format::Json => println!(
            "{}",
            serde_json::to_string_pretty(&json!({ "unfold": report })).expect("json")
        ),
        Format::Human => print_human(&report, parsed.dry),
    }

    if report.is_clean() {
        Ok(())
    } else {
        let occupied = report
            .files
            .iter()
            .filter(|f| matches!(f, FileOutcome::Occupied { .. }))
            .count();
        Err(findings(format!(
            "unfold {}: {occupied} scaffold path(s) already exist",
            report.preset
        )))
    }
}

/// Print the unfold report as a human table.
fn print_human(report: &UnfoldReport, dry: bool) {
    println!("unfold {}", report.preset);
    if !report.floor.is_empty() {
        println!("  floor pins: {}", report.floor.join(", "));
    }
    for file in &report.files {
        match file {
            FileOutcome::Born { path } => {
                if dry {
                    println!("  would-birth {path}");
                } else {
                    println!("  born {path}");
                }
            }
            FileOutcome::Occupied { path, reason } => {
                println!(
                    "  occupied {path} — {} [{}/{}]",
                    reason.message, reason.code, reason.recovery
                );
            }
        }
    }
}

/// The parsed `unfold` tail: `<preset>`, `--dry`, `--actor`, `--now`, `--json`.
struct Parsed {
    preset: String,
    dry: bool,
    actor: Option<String>,
    now: Option<String>,
    format: Format,
}

impl Parsed {
    fn parse(args: &[String]) -> Result<Self, Fail> {
        let mut preset: Option<String> = None;
        let mut dry = false;
        let mut json = false;
        let mut actor = None;
        let mut now = None;
        let mut it = args.iter();
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "--dry" => dry = true,
                "--json" => json = true,
                "--actor" => {
                    actor = Some(
                        it.next()
                            .ok_or_else(|| Fail::tool("--actor needs a value".to_owned()))?
                            .clone(),
                    );
                }
                "--now" => {
                    now = Some(
                        it.next()
                            .ok_or_else(|| Fail::tool("--now needs a value".to_owned()))?
                            .clone(),
                    );
                }
                flag if flag.starts_with('-') => {
                    return Err(Fail::tool(format!("unknown flag: {flag}")));
                }
                value if preset.is_none() => preset = Some(value.to_owned()),
                value => return Err(Fail::tool(format!("unexpected argument: {value}"))),
            }
        }
        let preset =
            preset.ok_or_else(|| Fail::tool("unfold needs a PRESET argument".to_owned()))?;
        Ok(Parsed {
            preset,
            dry,
            actor,
            now,
            format: if json { Format::Json } else { Format::Human },
        })
    }
}
