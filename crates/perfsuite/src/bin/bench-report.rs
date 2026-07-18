//! Join staged claim measurements + criterion estimates against `claims.toml`
//! and write the three renderings: `results/RESULTS.md` (human),
//! `results/latest.json` (agent), `results/<date>-<host>.json` (history).
//! The markdown also prints to stdout for CI logs and PR comments.

use std::path::PathBuf;
use std::process::ExitCode;

use perfsuite::{claims, report};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("bench-report: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let results_dir = args
        .iter()
        .position(|a| a == "--results")
        .and_then(|i| args.get(i + 1))
        .map_or_else(
            || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("results"),
            PathBuf::from,
        );
    let claims = claims::load_bundled()?;
    let run_report = report::build(&claims).map_err(|e| e.to_string())?;
    let md = report::render_markdown(&run_report, &claims);
    print!("{md}");
    let written = report::write(&results_dir, &run_report, &claims).map_err(|e| e.to_string())?;
    for path in written {
        eprintln!("wrote {}", path.display());
    }
    Ok(())
}
