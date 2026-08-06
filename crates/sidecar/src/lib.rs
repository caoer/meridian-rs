//! Thin NDJSON serve loop — one of two hosts of the shared typed edge
//! (`wire-serve`; the resident `registry` daemon is the other). Wiring only
//! (law 3, `docs/laws.md`).
//!
//! **Owns:** transport frames → strict-decode into `wire` types (v2 §3.2:
//! unknown fields/enum values rejected; serde `deny_unknown_fields` does not
//! compose with `flatten`) → `model`/`fs` → `wire-map` projection. The bin
//! is process wiring only.
//!
//! **Never:** parse (`syntax`), tree law (`model`), projection (`wire-map`),
//! disk (`fs`), frame meaning (`transport`).
//!
//! # Frame law (v2 §3.1)
//! One JSON object per line; stdout = frames only, logs → stderr. Raw `id`
//! is scanned before typed decode (B2, `transport::scan_id`): a bad id answers
//! `bad_request` with `id:null` and the lexeme in `id_raw` — never as a valid
//! id, never as a notification.

use std::io::{self, BufRead, Write};

use serde_json::{Map, Value};
use transport::{IdScan, scan_id};
use wire::{ErrorBody, ErrorCode, Response, ResponsePayload};

mod arms;
mod demand;
mod policy_bridge;
/// Delta plane (shared retention/classifier with registry). Re-exported: this
/// crate's ring remains a per-serve-lifetime epoch.
pub use wire_serve::ring;
use wire_serve::watch;

// v3 vocabulary projection — shared typed-edge home (lift, don't duplicate).
use wire_serve::rev;

/// Admit a rule pack sidecar-mode: compile over the real parse→facts plane;
/// refuse corpus-class LOUD (`daemon_only`, §8/§11.3). Admitted rulesets feed
/// splice `verdicts`.
pub use policy_bridge::admit;

/// v2 §3.2: server name in the `hello` body.
pub const SERVER_NAME: &str = "meridian-sidecar/2.0";
/// v2 §3.2: protocol this sidecar speaks (proto-1 retained).
pub const PROTO: u32 = 1;
/// Armed op set — frozen §3.2 printed list, complete. Armed ops + dotted field
/// amendments (`splice.verdicts` served `[]` from birth). `hello` answers but
/// is not a cap. S2/L22: `splice ∈ caps` ⇒ `node_rev` MUST on every
/// `toc`/`cat`/`extract` node.
pub const CAPS: [&str; 16] = [
    "toc",
    "cat",
    "extract",
    "resolve",
    "resolve.content",
    "links",
    "links.require_root",
    "splice",
    "splice.if_node_rev",
    "splice.if_root",
    "splice.dry",
    "splice.receipt",
    "splice.verdicts",
    "root",
    "diff",
    "sub",
];

/// Stdin loop: raw-id scan → strict decode → dispatch → one response frame
/// (flushed per frame), then the push path: undelivered ring frames as
/// Notification frames after the response. Malformed input →
/// `bad_frame`/`bad_request`; never terminate on a bad frame. EOF: finish
/// in-flight work, flush, `Ok(())`.
///
/// `rulesets` feed splice `verdicts` (§11.1); empty → `[]`. Pack sourcing is
/// the daemon's (§11 row 8); this loop only evaluates what it is handed.
///
/// # Errors
/// Stream I/O failure only — never a content condition.
pub fn serve(
    root: &fs::WorkspaceRoot,
    mut input: impl BufRead,
    mut output: impl Write,
    rulesets: &[policy::CompiledRuleset],
) -> io::Result<()> {
    // One serve lifetime = one daemon epoch (§7.1): ring + seq die with the
    // process; restart = new epoch; catch up by diff-by-root.
    let mut epoch = ring::RootRing::new();
    let mut subs: Vec<SubState> = Vec::new();
    let mut watch = watch::WatchState::new(root);
    // Per-session contract rev, negotiated at `hello`
    // (docs/wire-contract.md). Default v2 = frozen contract.
    let mut rev = rev::Rev::V2;
    let mut line = String::new();
    loop {
        line.clear();
        if input.read_line(&mut line)? == 0 {
            return output.flush();
        }
        if line.trim().is_empty() {
            continue; // blank lines ignored per frame layer
        }
        // Decode before reconcile: demand law needs the op
        // (`demand::observes_ring`). Frame-layer answers observe nothing.
        let decoded = decode_line(&mut rev, &line);
        // Reconcile before dispatch, on demand: ring readers + subscribed
        // sessions. Reconcile error never fails the request — stderr, retry.
        if (decoded.observes_ring() || !subs.is_empty())
            && let Err(e) = watch::reconcile(root, &mut epoch, &mut watch)
        {
            eprintln!("watch reconcile: {e:?}");
        }
        flush_subs(&mut output, &epoch, &mut subs, rev)?;
        let advances = decoded.advances_ring();
        let (response, duration_us) = respond(root, &mut epoch, &mut subs, rulesets, rev, decoded);
        write_response(&mut output, &response, rev, duration_us)?;
        // Post-dispatch: internal commit rebases baseline; mid-dispatch
        // external changes emit for waiting subscribers.
        if (advances || !subs.is_empty())
            && let Err(e) = watch::reconcile(root, &mut epoch, &mut watch)
        {
            eprintln!("watch reconcile: {e:?}");
        }
        // Push: ok first, then Notification frames (§4.7) — replay ≡ live.
        flush_subs(&mut output, &epoch, &mut subs, rev)?;
        output.flush()?;
    }
}

/// Active subscription: highest `delta.seq` delivered. Registered at
/// `from_seq`; first flush replays retained frames above it (§4.7).
struct SubState {
    delivered: u64,
}

/// Emit undelivered ring frames per sub, in order, via shared
/// `ring::write_frame`.
fn flush_subs(
    output: &mut impl Write,
    epoch: &ring::RootRing,
    subs: &mut [SubState],
    rev: rev::Rev,
) -> io::Result<()> {
    for sub in subs.iter_mut() {
        for frame in epoch.frames_after(sub.delivered) {
            ring::write_frame(output, &frame, rev == rev::Rev::V3)?;
            sub.delivered = frame.delta.seq;
        }
    }
    Ok(())
}

/// One response frame for the negotiated rev. v2: typed `wire::Response`
/// (frozen, byte-identical). v3: envelope `root` → `fingerprint`, plus
/// `meta: {duration_us}` on dispatched ops (measure point: `arms::dispatch`).
fn write_response(
    output: &mut impl Write,
    response: &Response,
    rev: rev::Rev,
    duration_us: Option<u64>,
) -> io::Result<()> {
    if rev == rev::Rev::V3 {
        let mut v = serde_json::to_value(response)?;
        rev::project_response(&mut v);
        if let Some(us) = duration_us {
            rev::attach_meta(&mut v, us);
        }
        serde_json::to_writer(&mut *output, &v)?;
    } else {
        // Frozen v2 session never grows a field: v3-additive extras demoted
        // here, not withheld at mint.
        let demoted = rev::demote_v2(response);
        serde_json::to_writer(&mut *output, demoted.as_ref().unwrap_or(response))?;
    }
    output.write_all(b"\n")
}

/// v2 §4.7 `sub`. `from_seq` catchup is valid only within this epoch
/// (§7.1): `current` or retained-ring positions; else `root_unknown` →
/// resync by diff-by-root. Never wrong data, never cross-epoch seq compare.
/// Ack body is §4.7 `{root, seq}` (subscription anchor tense).
fn subscribe(
    root: &fs::WorkspaceRoot,
    epoch: &ring::RootRing,
    subs: &mut Vec<SubState>,
    from_seq: u64,
) -> Result<wire::ResponseBody, Box<ErrorBody>> {
    let current = epoch.seq();
    if !epoch.can_anchor(from_seq) {
        let mut e = ErrorBody::new(ErrorCode::RootUnknown);
        e.message = Some(
            "from_seq outside this epoch's retained history — catch up by diff-by-root (§7.1)"
                .into(),
        );
        return Err(Box::new(e));
    }
    subs.push(SubState {
        delivered: from_seq,
    });
    Ok(wire::ResponseBody::Root {
        root: wire_serve::ambient_root(root)?,
        seq: current,
    })
}

/// One input line, classified so the loop can price it before run
/// (`demand::*`). Frame-layer verdicts reach no arm and cost nothing.
enum Decoded {
    /// Frame-layer answer (§3.1 / B2). No arm; ring not observed. Boxed so
    /// every line stays pointer-wide.
    Answer(Box<Response>),
    /// Validated op + correlation id. Boxed for the same width reason.
    Op(Option<u64>, Box<wire::Op>),
}

impl Decoded {
    fn observes_ring(&self) -> bool {
        matches!(self, Decoded::Op(_, op) if demand::observes_ring(op))
    }

    fn advances_ring(&self) -> bool {
        matches!(self, Decoded::Op(_, op) if demand::advances_ring(op))
    }
}

/// One frame → one verdict (§3.1). Order is law: raw `id` before typed decode
/// (B2). Session rev negotiates at `hello`, before any frame is shaped by it.
fn decode_line(rev: &mut rev::Rev, line: &str) -> Decoded {
    let id = match scan_id(line) {
        // not a JSON object → channel broken for this line
        Err(_) => {
            return Decoded::Answer(Box::new(error_frame(
                None,
                ErrorBody::new(ErrorCode::BadFrame),
            )));
        }
        // §3.1: id:null + offending lexeme in id_raw
        Ok(IdScan::BadId(lexeme)) => {
            let mut e = ErrorBody::new(ErrorCode::BadRequest);
            e.id_raw = Some(lexeme);
            return Decoded::Answer(Box::new(error_frame(None, e)));
        }
        Ok(IdScan::Request(n)) => Some(n),
        // id absent: legal id-less request if `op` present; else misuse.
        Ok(IdScan::Notification) => None,
    };
    // scan_id proved the line is a JSON object.
    let Ok(mut obj) = serde_json::from_str::<Map<String, Value>>(line) else {
        return Decoded::Answer(Box::new(error_frame(
            None,
            ErrorBody::new(ErrorCode::BadFrame),
        )));
    };
    if !obj.contains_key("op") {
        // Non-request inbound (response/notification) → bad_frame.
        return Decoded::Answer(Box::new(error_frame(
            None,
            ErrorBody::new(ErrorCode::BadFrame),
        )));
    }
    // v3 session: re-key request to v2 form so decoder/arms stay v2-only.
    // `hello` always arrives in base vocabulary + `contract`, so a still-v2
    // session never mangles it.
    if *rev == rev::Rev::V3 {
        rev::rename_request(&mut obj);
    }
    match wire_serve::decode::decode(&obj, *rev) {
        Ok(op) => {
            // Negotiate session rev from hello so this hello and later frames
            // are shaped for it.
            if let wire::Op::Hello { contract, .. } = &op {
                *rev = rev::Rev::from_contract(contract.as_deref());
            }
            Decoded::Op(id, Box::new(op))
        }
        Err(e) => Decoded::Answer(Box::new(error_frame(id, *e))),
    }
}

/// Decoded line → response + optional U7 duration (`Some(µs)` only when the
/// frame reached `arms::dispatch`). Frame-layer and serve-layer `sub` carry
/// none. Demand-driven reconcile is outside the timer: fold budget is a count,
/// not a clock.
fn respond(
    root: &fs::WorkspaceRoot,
    epoch: &mut ring::RootRing,
    subs: &mut Vec<SubState>,
    rulesets: &[policy::CompiledRuleset],
    rev: rev::Rev,
    decoded: Decoded,
) -> (Response, Option<u64>) {
    let (id, op) = match decoded {
        Decoded::Answer(response) => return (*response, None),
        // `sub` registers at the serve layer; other ops go to arms.
        Decoded::Op(id, op) => match *op {
            wire::Op::Sub { from_seq } => {
                return match subscribe(root, epoch, subs, from_seq) {
                    Ok(body) => (
                        Response {
                            id,
                            ok: true,
                            payload: ResponsePayload::Body { body },
                        },
                        None,
                    ),
                    Err(e) => (error_frame(id, *e), None),
                };
            }
            op => (id, op),
        },
    };
    // U7 measure point: dispatch alone — checked µs, never lossy `as`.
    let started = std::time::Instant::now();
    let outcome = arms::dispatch(root, epoch, id, op, rulesets, rev == rev::Rev::V3);
    let duration_us = Some(u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX));
    let response = match outcome {
        Ok(body) => Response {
            id,
            ok: true,
            payload: ResponsePayload::Body { body },
        },
        Err(e) => error_frame(id, *e),
    };
    (response, duration_us)
}

fn error_frame(id: Option<u64>, error: ErrorBody) -> Response {
    Response {
        id,
        ok: false,
        payload: ResponsePayload::Error { error },
    }
}
