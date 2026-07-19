//! Thin NDJSON serve loop + the typed edge — the only place wire and model meet
//! (with the named `wire-map` seam, law 3 as amended by review C1).
//!
//! # Charter
//! **Owns:** the typed edge: untyped `transport` frames validated into `wire`
//! types (the hand-rolled strict-decode pass — v2 §3.2 server law: unknown
//! request fields and unknown enum values are rejected loudly, because serde's
//! `deny_unknown_fields` does not compose with `flatten`), dispatched to
//! `model`/`fs`, results projected back to wire shapes at the `wire-map` seam.
//! The bin (`main.rs`) stays process wiring only.
//!
//! **Never does:** anything a crate could own — parsing (`syntax`), tree law
//! (`model`), projection behavior (`wire-map`), disk (`fs`), framing meaning
//! (`transport`). Growth pressure here is the signal a capability is missing
//! its crate; the serve/decode wiring targets a few hundred auditable lines.
//!
//! # Frame law (v2 §3.1)
//! One JSON object per line; stdout carries frames only, logs go to stderr.
//! The raw `id` lexeme is scanned BEFORE any typed decode (B2 law,
//! `transport::scan_id`): a non-conforming id answers `bad_request` with
//! `id:null` and the offending lexeme verbatim in `id_raw` — never echoed as
//! a valid id, never reclassified as a notification.
//!
//! # Rungs
//! Rung 2 (D2-DISPATCH): `hello`/`toc`/`cat`/`extract`/`resolve` arms +
//! strict decode; ops known to the wire but not yet armed answer `unknown_op`
//! (§3.2 discovery honesty). Rung 3+ adds arms, not structure.

use std::io::{self, BufRead, Write};

use serde_json::{Map, Value};
use transport::{IdScan, scan_id};
use wire::{ErrorBody, ErrorCode, Response, ResponsePayload};

mod arms;
mod decode;
pub mod ring;

/// v2 §3.2: the server name in the `hello` body.
pub const SERVER_NAME: &str = "meridian-sidecar/2.0";
/// v2 §3.2: the one protocol this sidecar speaks (proto-1 retained).
pub const PROTO: u32 = 1;
/// The ARMED op set at this rung, exactly (§3.2 discovery honesty: an op is in
/// `caps` or answers `unknown_op`; the ≡-full-§3.2-list assertion lands at
/// P6-VERDICTS). `hello` answers but is not itself a cap; `resolve.content` is
/// the one armed dotted field amendment. D3-DELTA arms `root` + `diff` (`diff`
/// truthfully serves empty-or-`root_unknown` until rung 4 emits). Q5-LINKS
/// arms `links` (§4.6 edge map + the §10.1 triple).
pub const CAPS: [&str; 8] = [
    "toc",
    "cat",
    "extract",
    "resolve",
    "resolve.content",
    "root",
    "diff",
    "links",
];

/// The stdin loop: raw-id scan → strict decode → dispatch → exactly one
/// response frame, flushed per frame (shell-pipe debuggability is a contract
/// property). Malformed input answers `bad_frame`/`bad_request`; the sidecar
/// never terminates because of a bad frame. EOF: in-flight work finished,
/// output flushed, `Ok(())`.
///
/// # Errors
/// I/O failure on the streams themselves — never a content condition.
pub fn serve(
    root: &fs::WorkspaceRoot,
    mut input: impl BufRead,
    mut output: impl Write,
) -> io::Result<()> {
    // One serve lifetime = one daemon EPOCH (§7.1 late law): the ring and its
    // seq are born here and die here; nothing persists across restarts.
    let epoch = ring::RootRing::new();
    let mut line = String::new();
    loop {
        line.clear();
        if input.read_line(&mut line)? == 0 {
            return output.flush();
        }
        if line.trim().is_empty() {
            continue; // blank lines ignored per frame layer
        }
        let response = respond_line(root, &epoch, &line);
        serde_json::to_writer(&mut output, &response)?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
}

/// One frame in → one response out (§3.1). Order is law: the raw `id` lexeme
/// verdict comes BEFORE typed decode (B2), so no typed decode can rescue or
/// corrupt frame classification.
fn respond_line(root: &fs::WorkspaceRoot, epoch: &ring::RootRing, line: &str) -> Response {
    let id = match scan_id(line) {
        // not a JSON object → the channel is broken for this line
        Err(_) => return error_frame(None, ErrorBody::new(ErrorCode::BadFrame)),
        // §3.1 emission: id:null + the offending lexeme verbatim in id_raw
        Ok(IdScan::BadId(lexeme)) => {
            let mut e = ErrorBody::new(ErrorCode::BadRequest);
            e.id_raw = Some(lexeme);
            return error_frame(None, e);
        }
        Ok(IdScan::Request(n)) => Some(n),
        // id key absent: a legal id-less request if `op` rides the frame
        // (shell-pipe debuggability), else an inbound notification — misuse.
        Ok(IdScan::Notification) => None,
    };
    // scan_id proved the line is a JSON object.
    let Ok(obj) = serde_json::from_str::<Map<String, Value>>(line) else {
        return error_frame(None, ErrorBody::new(ErrorCode::BadFrame));
    };
    if !obj.contains_key("op") {
        // Inbound frames that aren't requests (responses, notifications) are
        // protocol misuse → bad_frame; un-correlatable by design.
        return error_frame(None, ErrorBody::new(ErrorCode::BadFrame));
    }
    match decode::decode(&obj) {
        Ok(op) => match arms::dispatch(root, epoch, op) {
            Ok(body) => Response {
                id,
                ok: true,
                payload: ResponsePayload::Body { body },
            },
            Err(e) => error_frame(id, *e),
        },
        Err(e) => error_frame(id, *e),
    }
}

fn error_frame(id: Option<u64>, error: ErrorBody) -> Response {
    Response {
        id,
        ok: false,
        payload: ResponsePayload::Error { error },
    }
}

/// A `bad_request` with a human message — the strict-decode workhorse.
pub(crate) fn bad_request(message: impl Into<String>) -> Box<ErrorBody> {
    let mut e = ErrorBody::new(ErrorCode::BadRequest);
    e.message = Some(message.into());
    Box::new(e)
}
