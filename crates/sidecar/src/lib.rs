//! Thin NDJSON serve loop — ONE of the two HOSTS of the shared typed edge
//! (`wire-serve`; the resident `registry` daemon is the other), wiring only
//! (law 3 as re-attested 2026-07-24, `docs/laws.md`).
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
mod policy_bridge;
pub mod ring;
mod watch;

// The v3 vocabulary projection lives in `wire-serve`, the shared typed-edge home
// both hosts project through (arch map A6: "lift, don't duplicate"). Imported as
// `rev` so the serve loop reads the same as before.
use wire_serve::rev;

/// Admit a rule pack sidecar-mode (P6-VERDICTS): compile through policy's load
/// gate over the REAL parse→facts plane, refusing a corpus-class pack LOUD
/// (`daemon_only`, §8/§11.3). The admitted [`policy::CompiledRuleset`]s are the
/// `serve` rulesets whose findings ride splice `verdicts`.
pub use policy_bridge::admit;

/// v2 §3.2: the server name in the `hello` body.
pub const SERVER_NAME: &str = "meridian-sidecar/2.0";
/// v2 §3.2: the one protocol this sidecar speaks (proto-1 retained).
pub const PROTO: u32 = 1;
/// The ARMED op set — exactly the frozen §3.2 printed list, COMPLETE:
/// T5-SUB armed `sub` and deleted the D4-SPLICE subtraction, so the caps
/// fixture is now the naked §3.2 full-list ≡ (P6-VERDICTS re-asserts it as
/// its own pack acceptance row). Every entry is TRUE of this build — armed
/// ops + dotted field amendments (`splice.verdicts` names the verdicts
/// surface, served `[]` from birth; variants are P6's). `hello` answers but
/// is not itself a cap. S2/L22 law is LIVE: `splice ∈ caps` ⇒ `node_rev`
/// MUST on every `toc`/`cat`/`extract` node (pinned in the caps fixture).
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

/// The stdin loop: raw-id scan → strict decode → dispatch → exactly one
/// response frame, flushed per frame (shell-pipe debuggability is a contract
/// property) — then the push path (T5-SUB, the serve loop's ONE structural
/// change, still wiring-only): every ring frame not yet delivered to an
/// active subscription is written as a Notification frame after the
/// response. Malformed input answers `bad_frame`/`bad_request`; the sidecar
/// never terminates because of a bad frame. EOF: in-flight work finished,
/// output flushed, `Ok(())`.
///
/// `rulesets` are the admitted rule packs (P6-VERDICTS) whose findings ride every
/// splice response's `verdicts` (§11.1); empty = no packs loaded, so verdicts stay
/// `[]`. Where the daemon SOURCES packs is Go's business (§11, row 8: loaded-pack
/// listing is a Go surface) — this loop only evaluates what it is handed.
///
/// # Errors
/// I/O failure on the streams themselves — never a content condition.
pub fn serve(
    root: &fs::WorkspaceRoot,
    mut input: impl BufRead,
    mut output: impl Write,
    rulesets: &[policy::CompiledRuleset],
) -> io::Result<()> {
    // One serve lifetime = one daemon EPOCH (§7.1 late law): the ring and its
    // seq are born here and die here; nothing persists across restarts —
    // subscriptions included (a restart is a new epoch; catch up diff-by-root).
    let mut epoch = ring::RootRing::new();
    let mut subs: Vec<SubState> = Vec::new();
    let mut watch = watch::WatchState::new(root);
    // Per-serve-session contract rev (one epoch, one rev), negotiated at
    // `hello` (docs/wire-contract-v3-amendment.md). Defaults to v2 so an
    // un-negotiated session is byte-for-byte the frozen contract.
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
        // Decode BEFORE the reconcile — the loop cannot know what this line
        // costs until it knows which op it holds (the demand law,
        // `watch::observes_ring`). A line answered at the frame layer runs no
        // arm, so it observes nothing and owes no fold at all.
        let decoded = decode_line(&mut rev, &line);
        // F5-WATCH reconcile BEFORE dispatch, ON DEMAND: an external change is
        // emitted (and pushed) before this request's answer reads the ring —
        // for the lines that read the ring, plus every line of a subscribed
        // session (a standing observer of the delta stream). A reconcile error
        // never fails the request — stderr, retry next cycle.
        if (decoded.observes_ring() || !subs.is_empty())
            && let Err(e) = watch::reconcile(root, &mut epoch, &mut watch)
        {
            eprintln!("watch reconcile: {e:?}");
        }
        flush_subs(&mut output, &epoch, &mut subs, rev)?;
        let advances = decoded.advances_ring();
        let (response, duration_us) = respond(root, &mut epoch, &mut subs, rulesets, rev, decoded);
        write_response(&mut output, &response, rev, duration_us)?;
        // Post-dispatch reconcile: an internal commit syncs the baseline
        // silently (so the next external delta chains from the commit, not
        // from the root it replaced); an external landing mid-dispatch is
        // emitted here for the subscribers who are waiting on it.
        if (advances || !subs.is_empty())
            && let Err(e) = watch::reconcile(root, &mut epoch, &mut watch)
        {
            eprintln!("watch reconcile: {e:?}");
        }
        // The push path: ok first, THEN Notification frames (§4.7 order) —
        // a fresh subscription's replay and every live emission ride the
        // same flush, so replay ≡ live holds at the transport too.
        flush_subs(&mut output, &epoch, &mut subs, rev)?;
        output.flush()?;
    }
}

/// One active subscription: the highest `delta.seq` already delivered.
/// Registered by the `sub` arm at `from_seq` — the first flush replays the
/// retained frames above it (§4.7: "ok, then Notification frames").
struct SubState {
    delivered: u64,
}

/// Write every undelivered ring frame to each subscription, in emission
/// order — the frames are the STORED ring objects serialized directly
/// (`{"delta":{…}}`, no `id` key — §3.1 classification): the d4
/// single-constructor law extends to the push path by construction. Under a
/// v3 session each frame is re-shaped `root_before/after` → `fingerprint_*`
/// before write; v2 serializes the ring object directly (byte-identical).
fn flush_subs(
    output: &mut impl Write,
    epoch: &ring::RootRing,
    subs: &mut [SubState],
    rev: rev::Rev,
) -> io::Result<()> {
    for sub in subs.iter_mut() {
        for frame in epoch.frames_after(sub.delivered) {
            if rev == rev::Rev::V3 {
                let mut v = serde_json::to_value(&frame)?;
                rev::project_delta_frame(&mut v);
                serde_json::to_writer(&mut *output, &v)?;
            } else {
                serde_json::to_writer(&mut *output, &frame)?;
            }
            output.write_all(b"\n")?;
            sub.delivered = frame.delta.seq;
        }
    }
    Ok(())
}

/// Write one response frame, shaped per the negotiated rev. v2 serializes the
/// typed `wire::Response` directly — the frozen path, byte-identical. v3
/// projects the serialized frame `root` → `fingerprint` at the envelope layer
/// (the typed layer never changes), then attaches the in-band timing block
/// `meta: {duration_us}` when this frame answered a dispatched op (U7: the
/// sidecar measure point is the `arms::dispatch` call — engine work only).
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
        // A frozen v2 session never grows a field: U11's v3-additive ladder
        // extras are dropped here, not withheld at mint time.
        let demoted = rev::demote_v2(response);
        serde_json::to_writer(&mut *output, demoted.as_ref().unwrap_or(response))?;
    }
    output.write_all(b"\n")
}

/// v2 §4.7 `sub`, live at T5-SUB. The §7.1 late law's residue: `from_seq`
/// catchup is valid only WITHIN this epoch — anchorable positions are
/// `current` (live-only) and anything the retained ring can replay from;
/// everything else (evicted, ahead, a prior epoch's counter) answers
/// `root_unknown` → resync, catch up by diff-by-root (the only
/// restart-durable handle). Never wrong data, never a cross-epoch seq
/// comparison — the old counter died with its ring. The ack reuses the
/// §4.7 `{root, seq}` body: the subscription's anchor tense (advisor-ruled;
/// no frozen frame prints an ack body).
fn subscribe(
    root: &fs::WorkspaceRoot,
    epoch: &ring::RootRing,
    subs: &mut Vec<SubState>,
    from_seq: u64,
) -> Result<wire::ResponseBody, Box<ErrorBody>> {
    let current = epoch.seq();
    let anchored = from_seq == current
        || (from_seq < current && epoch.oldest_seq().is_some_and(|o| from_seq >= o - 1));
    if !anchored {
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

/// What one input line turned out to be. The split exists so the loop can
/// price the line before it runs it: only a decoded op can be classified
/// against the demand law, and a frame-layer verdict — which reaches no arm —
/// costs nothing at all.
enum Decoded {
    /// Answered at the frame layer (§3.1 classification, B2 id law). No arm
    /// runs, so nothing observes the ring. BOXED: a `Response` carries the whole
    /// error envelope, so by-value it would make every decoded line as wide as
    /// the widest refusal.
    Answer(Box<Response>),
    /// A validated op beside its correlation token. BOXED for the same reason
    /// as `Answer`: both arms stay pointer-wide, so one line's classification
    /// never costs the width of the widest op.
    Op(Option<u64>, Box<wire::Op>),
}

impl Decoded {
    fn observes_ring(&self) -> bool {
        matches!(self, Decoded::Op(_, op) if watch::observes_ring(op))
    }

    fn advances_ring(&self) -> bool {
        matches!(self, Decoded::Op(_, op) if watch::advances_ring(op))
    }
}

/// One frame in → one verdict (§3.1). Order is law: the raw `id` lexeme
/// verdict comes BEFORE typed decode (B2), so no typed decode can rescue or
/// corrupt frame classification. Negotiating the session rev happens here too
/// — at the `hello` declaration, before any frame is shaped by it.
fn decode_line(rev: &mut rev::Rev, line: &str) -> Decoded {
    let id = match scan_id(line) {
        // not a JSON object → the channel is broken for this line
        Err(_) => {
            return Decoded::Answer(Box::new(error_frame(
                None,
                ErrorBody::new(ErrorCode::BadFrame),
            )));
        }
        // §3.1 emission: id:null + the offending lexeme verbatim in id_raw
        Ok(IdScan::BadId(lexeme)) => {
            let mut e = ErrorBody::new(ErrorCode::BadRequest);
            e.id_raw = Some(lexeme);
            return Decoded::Answer(Box::new(error_frame(None, e)));
        }
        Ok(IdScan::Request(n)) => Some(n),
        // id key absent: a legal id-less request if `op` rides the frame
        // (shell-pipe debuggability), else an inbound notification — misuse.
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
        // Inbound frames that aren't requests (responses, notifications) are
        // protocol misuse → bad_frame; un-correlatable by design.
        return Decoded::Answer(Box::new(error_frame(
            None,
            ErrorBody::new(ErrorCode::BadFrame),
        )));
    }
    // v3 session: re-key the request into its v2 form so the strict decoder
    // and every arm stay v2-only. `hello` itself always arrives in the base
    // vocabulary + the `contract` knob, so a not-yet-negotiated session (still
    // v2 here) never mangles it.
    if *rev == rev::Rev::V3 {
        rev::rename_request(&mut obj);
    }
    match wire_serve::decode::decode(&obj, *rev) {
        Ok(op) => {
            // Negotiate the session rev from the hello declaration, so THIS
            // hello response (and every frame after) is shaped for it.
            if let wire::Op::Hello { contract, .. } = &op {
                *rev = rev::Rev::from_contract(contract.as_deref());
            }
            Decoded::Op(id, Box::new(op))
        }
        Err(e) => Decoded::Answer(Box::new(error_frame(id, *e))),
    }
}

/// One decoded line → one response frame. Returns the response plus the U7
/// in-band duration: `Some(µs)` exactly when the frame reached `arms::dispatch`
/// (the sidecar measure point — engine work only, success or refusal alike).
/// Frame-layer verdicts and the serve-layer `sub` never carry one, and the
/// demand-driven reconcile is deliberately OUTSIDE the timer: the fold budget
/// is proven by counting folds, never by reading a clock.
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
        // The push-path op registers at the serve layer — the loop owns the
        // subscription list; everything else routes to the arms.
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
    // U7 measure point: the dispatch call alone (after decode, before the
    // response write) — checked µs, never a lossy `as`.
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
