//! Process wiring: arg parse + locked stdio. Meaning lives in `sidecar::serve`.

use std::io;
use std::process::ExitCode;

fn main() -> ExitCode {
    let Some(root) = std::env::args().nth(1) else {
        // Logs/diagnostics → stderr only (frame layer §3).
        eprintln!("usage: sidecar <workspace-root>");
        return ExitCode::FAILURE;
    };
    let root = fs::WorkspaceRoot(root.into());

    let stdin = io::stdin().lock();
    let stdout = io::stdout().lock();
    // Pack sourcing is the daemon's (§11 row 8). Empty set → splice
    // `verdicts` stay `[]`; §11.1 shape still rides every response.
    match sidecar::serve(&root, stdin, stdout, &[]) {
        // stdin EOF: finish in-flight, flush, exit 0.
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("sidecar: fatal: {e}");
            ExitCode::FAILURE
        }
    }
}
