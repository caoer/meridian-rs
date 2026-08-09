//! `mrd new <kind> <id> [--dry] [--actor A] [--now T]` (U5.3): file birth — resolve the def
//! cascade, fill the defs `^template`, validate the filled record against the defs
//! `^properties`, and birth the first rev through the U2.6 guarded create (inline birth
//! receipt). A local CLIENT of `preset::new_record` — mrd holds no markdown semantics
//! (docs/laws.md).

use preset::{BirthOptions, NewOutcome};
use serde_json::json;

use crate::{Fail, Format};

/// Exit 1 — a finding (the birth refused). Distinct from `Fail::tool` (exit 2).
fn refused(message: String) -> Fail {
    Fail::findings(message)
}

/// Run `mrd new <kind> <id> [--dry] [--actor A] [--now T] [--json]`.
///
/// # Errors
/// A tool failure (exit 2) — bad usage, an unreadable workspace, a def that is not a def, or a
/// faulting write — or a birth refusal (exit 1) rendered from the closed-taxonomy reason.
pub(crate) fn run(args: &[String]) -> Result<(), Fail> {
    let parsed = Parsed::parse(args)?;
    let root = crate::preset_cmd::resolve_root()?;
    let def_path = crate::preset_cmd::def_path(&parsed.kind);

    let opts = BirthOptions {
        actor: parsed.actor,
        now: parsed.now,
        dry: parsed.dry,
    };
    let outcome = preset::new_record(&root, &def_path, &parsed.id, &opts)
        .map_err(|e| Fail::tool(e.to_string()))?;

    match parsed.format {
        Format::Json => println!(
            "{}",
            serde_json::to_string_pretty(&json!({ "new": outcome })).expect("json")
        ),
        Format::Human => print_human(&outcome),
    }

    match &outcome {
        NewOutcome::Born(_) => Ok(()),
        NewOutcome::Refused(r) => Err(refused(format!(
            "new refused {} — {} [{}/{}]",
            r.target, r.reason.message, r.reason.code, r.reason.recovery
        ))),
    }
}

/// Print the birth outcome as a human table.
fn print_human(outcome: &NewOutcome) {
    match outcome {
        NewOutcome::Born(report) => {
            let state = if report.dry { "would-birth" } else { "born" };
            println!("new {} [{state}]", report.target);
            println!("  rev: {}", report.rev);
        }
        NewOutcome::Refused(refusal) => {
            println!("new {} [refused]", refusal.target);
            println!(
                "  {} [{}/{}]",
                refusal.reason.message, refusal.reason.code, refusal.reason.recovery
            );
            if let Some(rule) = &refusal.reason.rule {
                println!("  def rule: {rule}");
            }
        }
    }
}

/// The parsed `new` tail: `<kind>`, `<id>`, `--dry`, `--actor`, `--now`, `--json`.
struct Parsed {
    kind: String,
    id: String,
    dry: bool,
    actor: Option<String>,
    now: Option<String>,
    format: Format,
}

impl Parsed {
    fn parse(args: &[String]) -> Result<Self, Fail> {
        let mut positionals: Vec<String> = Vec::new();
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
                value => positionals.push(value.to_owned()),
            }
        }
        let [kind, id] = positionals.as_slice() else {
            return Err(Fail::tool("new needs a KIND and an ID argument".to_owned()));
        };
        Ok(Parsed {
            kind: kind.clone(),
            id: id.clone(),
            dry,
            actor,
            now,
            format: if json { Format::Json } else { Format::Human },
        })
    }
}
