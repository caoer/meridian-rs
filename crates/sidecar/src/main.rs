//! Thin NDJSON stdin/stdout binary — the only place wire and model meet.
//!
//! # Charter
//! **Owns:** process wiring only — arg parsing, the stdin loop, and the typed
//! edge: untyped `transport` frames validated into `wire` types, dispatched to
//! `model`/`fs`, results projected back to wire nodes (including the
//! `text_prefix_16b` computation, which needs raw bytes + wire shape — the one
//! job that legitimately requires seeing both sides).
//!
//! **Never does:** anything a crate could own. Target: under 300 lines
//! including `main` — growth pressure here is the signal a capability is
//! missing its crate.
//!
//! # Law enforcement (candidate thesis, this crate's part)
//! This is the ONLY crate depending on `wire` and `model` together — the
//! compile-time expression of law 3. `model`'s types can't serialize and
//! `wire`'s can't reach the parser; the bridge is these few hundred lines,
//! auditable in one sitting.
//!
//! # Rungs
//! Rung 1: hello/toc/extract over `NdjsonCodec`. Rungs 2–3 add match arms, not
//! structure.

use std::io::{self, BufRead, Write};
use std::process::ExitCode;

use transport::{Codec, Message, NdjsonCodec};

fn main() -> ExitCode {
    let Some(root) = std::env::args().nth(1) else {
        // Logs and diagnostics go to stderr, never stdout (frame layer §3).
        eprintln!("usage: sidecar <workspace-root>");
        return ExitCode::FAILURE;
    };
    let root = fs::WorkspaceRoot(root.into());

    let stdin = io::stdin().lock();
    let stdout = io::stdout().lock();
    match serve(&root, stdin, stdout) {
        // stdin EOF: in-flight work finished, stdout flushed, exit 0.
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("sidecar: fatal: {e}");
            ExitCode::FAILURE
        }
    }
}

/// The stdin loop: decode frame → dispatch → encode exactly one response.
/// Malformed input answers `bad_frame`/`bad_request`; the sidecar never
/// terminates because of a bad frame.
fn serve(
    root: &fs::WorkspaceRoot,
    mut input: impl BufRead,
    mut output: impl Write,
) -> io::Result<()> {
    let codec = NdjsonCodec;
    while let Some(frame) = codec.decode(&mut input)? {
        let response = match frame {
            Message::Request(req) => dispatch(root, req),
            // Inbound frames that aren't requests are protocol misuse → bad_frame.
            Message::Response(_) | Message::Notification(_) => todo!("rung 1: bad_frame envelope"),
        };
        codec.encode(&Message::Response(response), &mut output)?;
    }
    output.flush()
}

/// The typed edge: untyped params → `wire::Request` (unknown/mistyped fields →
/// `bad_request`, loud — a silently dropped CAS guard corrupts data), then one
/// match arm per op onto `model`/`fs`.
fn dispatch(root: &fs::WorkspaceRoot, req: transport::Request) -> transport::Response {
    let _ = (root, req);
    todo!("rung 1: validate into wire::Request; hello/toc/extract arms over model+fs")
}
