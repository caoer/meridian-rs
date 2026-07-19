//! Process wiring for the sidecar bin: arg parsing + the locked stdio streams.
//! Everything with meaning lives in the crate library (`sidecar::serve` — the
//! typed edge, testable byte-for-byte); this file stays under a screenful.

use std::io;
use std::process::ExitCode;

fn main() -> ExitCode {
    let Some(root) = std::env::args().nth(1) else {
        // Logs and diagnostics go to stderr, never stdout (frame layer §3).
        eprintln!("usage: sidecar <workspace-root>");
        return ExitCode::FAILURE;
    };
    let root = fs::WorkspaceRoot(root.into());

    let stdin = io::stdin().lock();
    let stdout = io::stdout().lock();
    match sidecar::serve(&root, stdin, stdout) {
        // stdin EOF: in-flight work finished, stdout flushed, exit 0.
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("sidecar: fatal: {e}");
            ExitCode::FAILURE
        }
    }
}
