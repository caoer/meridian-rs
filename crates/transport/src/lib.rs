//! Untyped NDJSON message envelope + codec seam: knows framing, never meaning.
//!
//! # Charter
//! **Owns:** the frame layer — one JSON object per line, blank lines ignored,
//! stdout frames-only — and envelope classification (request / response /
//! notification). Envelope is deliberately UNTYPED (`serde_json::Value`
//! payloads): protocol evolution never forces a transport release and vice
//! versa. Alternate framings implement the same [`Codec`] seam.
//!
//! **Never does:** know what ops mean, validate op fields (typed edge is
//! `wire` at the serving host's boundary), touch the filesystem or the model.

use std::collections::BTreeMap;
use std::io::{self, BufRead, Write};

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use serde_json::{Map, Value};

/// The untyped envelope. Classification mirrors the wire contract's frame rule:
/// a frame with `op` is a request, a frame carrying `ok` (+ `id`, possibly
/// null) is a response, anything else is a notification/event (rung 4+).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Message {
    Request(Request),
    Response(Response),
    Notification(Notification),
}

/// A request frame: `op` + optional `id`, all op-specific fields untyped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    pub op: String,
    #[serde(flatten)]
    pub params: Map<String, Value>,
}

/// A response frame: `id` always serialized (`null` for un-correlatable
/// errors — presence of the key is what makes a frame a response), `ok`, and
/// the body untyped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub id: Option<u64>,
    pub ok: bool,
    #[serde(flatten)]
    pub body: Map<String, Value>,
}

/// An event frame (rung 4+): no `id` key, payload untyped (`sub` etc. live
/// inside). The v1 server emits none.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notification {
    #[serde(flatten)]
    pub body: Map<String, Value>,
}

/// The framing seam. `NdjsonCodec` is the first implementation; the rung-6
/// LSP-framing codec implements the same trait and nothing above notices.
pub trait Codec {
    /// Encode one message as one frame.
    ///
    /// # Errors
    /// I/O failure writing to `out`, or a message serde cannot serialize.
    fn encode(&self, msg: &Message, out: &mut dyn Write) -> io::Result<()>;
    /// Decode the next frame; `Ok(None)` at EOF. Blank/whitespace-only lines
    /// are skipped. A syntactically unparseable line is an `Err` here — turning
    /// that into the `bad_frame` error *response* is the caller's (the serving
    /// host's) move, because responding is meaning, not framing.
    ///
    /// # Errors
    /// I/O failure reading `input`, or a syntactically unparseable frame.
    fn decode(&self, input: &mut dyn BufRead) -> io::Result<Option<Message>>;
}

/// Newline-delimited JSON framing: one JSON object per line, `\n`-terminated,
/// UTF-8. `echo '{"op":"toc",…}' | nc -U "$SOCKET"` debuggability is a
/// contract property this codec preserves (§3.1 — the pipe test outlived the
/// sidecar's DROP because the daemon socket speaks the same line dialogue).
#[derive(Debug, Default)]
pub struct NdjsonCodec;

impl Codec for NdjsonCodec {
    fn encode(&self, msg: &Message, out: &mut dyn Write) -> io::Result<()> {
        serde_json::to_writer(&mut *out, msg)?;
        out.write_all(b"\n")
    }

    fn decode(&self, input: &mut dyn BufRead) -> io::Result<Option<Message>> {
        let mut line = String::new();
        loop {
            line.clear();
            if input.read_line(&mut line)? == 0 {
                return Ok(None); // EOF: finish in-flight work, flush, exit 0
            }
            if line.trim().is_empty() {
                continue; // blank lines ignored per frame layer
            }
            return serde_json::from_str(&line)
                .map(Some)
                .map_err(io::Error::other);
        }
    }
}

/// The verdict of scanning the raw `id` key of one NDJSON line, run BEFORE any
/// typed decode (contract §3.1, "B2 law"). Frame classification AND id validation
/// are decided on the *raw* JSON lexeme, so no typed decode can rescue or corrupt
/// them: an oversized numeric id (`2^64`, which overflows `u64`) is a bad request,
/// never silently reclassified as a notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdScan {
    /// No `id` key on the line → notification frame (events carry no id, §7).
    Notification,
    /// `id` key present with a conforming lexeme — a JSON integer in `[0, 2^53)`.
    /// Carries the parsed value; the response echoes it by value.
    Request(u64),
    /// `id` key present but the lexeme is non-conforming: a string (`"7"`), a
    /// float form (`3.5`, `3e0`), negative (`-1`), null, or out of range
    /// (`>= 2^53`, including `2^64`). Carries the offending lexeme VERBATIM (raw
    /// source characters) — §3.1 has the server answer `bad_request` with
    /// `id:null` plus this string in `id_raw`, never echoing it as a valid id.
    BadId(String),
}

/// Valid JSON `id` lexemes are integers in `[0, 2^53)` — `2^53` is the largest
/// contiguous-integer boundary of an IEEE-754 double, so every id round-trips
/// through a JSON/JS client without silent rounding.
const ID_MAX_EXCLUSIVE: u64 = 1 << 53; // 9_007_199_254_740_992

/// Scan the raw `id` lexeme of one NDJSON line and return its frame verdict,
/// BEFORE typed decode (contract §3.1). Sits beside [`NdjsonCodec`]: framing
/// only, never meaning — emitting the `bad_request` frame for an [`IdScan::BadId`]
/// (`id:null` + verbatim `id_raw`) is the serving host's move, not this crate's.
///
/// The raw lexeme is authoritative: `3e0` is JSON-equal to `3` yet is not an
/// integer lexeme, so it is an [`IdScan::BadId`], not a valid `3`. Key *presence*
/// (not the parsed value) decides notification vs correlated, so a present-but-
/// invalid id (`2^64`, `null`) is a bad request, never a notification.
///
/// # Errors
/// The line is not a JSON object — a `bad_frame`, the caller's to answer — same
/// error convention as [`Codec::decode`].
pub fn scan_id(line: &str) -> io::Result<IdScan> {
    let obj: BTreeMap<String, Box<RawValue>> =
        serde_json::from_str(line).map_err(io::Error::other)?;
    Ok(match obj.get("id") {
        None => IdScan::Notification,
        Some(raw) => {
            let lexeme = raw.get();
            match lexeme.parse::<u64>() {
                Ok(n) if n < ID_MAX_EXCLUSIVE => IdScan::Request(n),
                _ => IdScan::BadId(lexeme.to_owned()),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round-trip through the codec, and prove the untyped envelope carries a
    /// typed `wire` request byte-compatibly — the typed/untyped agreement the
    /// lsp-server pattern rests on.
    #[test]
    fn ndjson_roundtrip_agrees_with_wire_types() {
        let line = r#"{"id":2,"op":"toc","path":"tasks/x.md"}"#;

        // untyped view
        let mut reader = io::BufReader::new(line.as_bytes());
        let msg = NdjsonCodec.decode(&mut reader).unwrap().unwrap();
        let Message::Request(req) = &msg else {
            panic!("frame with op must classify as request")
        };
        assert_eq!(req.op, "toc");
        assert_eq!(req.id, Some(2));

        // typed view (wire is a dev-dependency only)
        let typed: wire::Request = serde_json::from_str(line).unwrap();
        assert_eq!(
            typed,
            wire::Request {
                id: Some(2),
                op: wire::Op::Toc {
                    path: wire::Path("tasks/x.md".into())
                }
            }
        );

        // encode round-trip
        let mut out = Vec::new();
        NdjsonCodec.encode(&msg, &mut out).unwrap();
        let reparsed: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(reparsed, serde_json::from_str::<Value>(line).unwrap());
    }

    /// A response serializes `id` even when null; a frame without `id` is an
    /// event (frame classification rule).
    #[test]
    fn response_id_null_is_serialized() {
        let resp = Message::Response(Response {
            id: None,
            ok: false,
            body: serde_json::from_str(r#"{"error":"bad_frame"}"#).unwrap(),
        });
        let mut out = Vec::new();
        NdjsonCodec.encode(&resp, &mut out).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "{\"id\":null,\"ok\":false,\"error\":\"bad_frame\"}\n"
        );
    }

    /// Helper: scan a well-formed frame, panicking on a `bad_frame` line.
    fn scan(line: &str) -> IdScan {
        scan_id(line).expect("well-formed JSON object")
    }

    // ---- Contract §3.1 discrimination set (the B2 law), one row per test. ----

    #[test]
    fn scan_id_row_integer_7_valid() {
        assert_eq!(scan(r#"{"id":7,"op":"toc"}"#), IdScan::Request(7));
    }

    #[test]
    fn scan_id_row_string_7_bad() {
        // string, not integer — verbatim lexeme keeps its quotes.
        assert_eq!(
            scan(r#"{"id":"7","op":"toc"}"#),
            IdScan::BadId("\"7\"".into())
        );
    }

    #[test]
    fn scan_id_row_float_3_5_bad() {
        assert_eq!(
            scan(r#"{"id":3.5,"op":"toc"}"#),
            IdScan::BadId("3.5".into())
        );
    }

    #[test]
    fn scan_id_row_negative_1_bad() {
        assert_eq!(scan(r#"{"id":-1,"op":"toc"}"#), IdScan::BadId("-1".into()));
    }

    #[test]
    fn scan_id_row_exponent_3e0_bad() {
        // raw-lexeme law: JSON-equal to 3, but `3e0` is not an integer lexeme.
        assert_eq!(
            scan(r#"{"id":3e0,"op":"toc"}"#),
            IdScan::BadId("3e0".into())
        );
    }

    #[test]
    fn scan_id_row_2p53_minus_1_valid() {
        assert_eq!(
            scan(r#"{"id":9007199254740991,"op":"toc"}"#),
            IdScan::Request(9_007_199_254_740_991)
        );
    }

    #[test]
    fn scan_id_row_2p53_bad() {
        // exactly 2^53 — the exclusive ceiling.
        assert_eq!(
            scan(r#"{"id":9007199254740992,"op":"toc"}"#),
            IdScan::BadId("9007199254740992".into())
        );
    }

    /// Keystone of the B2 law: an oversized numeric id (2^64, overflows `u64`)
    /// is a `bad_request` carrying the verbatim lexeme — NEVER a notification.
    #[test]
    fn scan_id_row_oversized_2p64_bad_never_notification() {
        let v = scan(r#"{"id":18446744073709551616,"op":"toc"}"#);
        assert_eq!(v, IdScan::BadId("18446744073709551616".into()));
        assert!(!matches!(v, IdScan::Notification));
    }

    // ---- Classification law: key presence, not parsed value, decides. ----

    #[test]
    fn scan_id_absent_key_is_notification() {
        assert_eq!(scan(r#"{"op":"toc","path":"x.md"}"#), IdScan::Notification);
        assert_eq!(scan(r#"{"sub":"root"}"#), IdScan::Notification);
    }

    /// A present-but-null id is NOT a notification: the key is present, so the
    /// frame is correlated; `null` is a non-conforming lexeme → `bad_request`.
    #[test]
    fn scan_id_present_null_is_bad_not_notification() {
        let v = scan(r#"{"id":null,"op":"toc"}"#);
        assert_eq!(v, IdScan::BadId("null".into()));
        assert!(!matches!(v, IdScan::Notification));
    }

    #[test]
    fn scan_id_zero_is_valid_low_boundary() {
        assert_eq!(scan(r#"{"id":0,"op":"toc"}"#), IdScan::Request(0));
    }

    /// A non-object line is a `bad_frame` (Err), not an id verdict.
    #[test]
    fn scan_id_non_object_errs() {
        assert!(scan_id("[1,2,3]").is_err());
        assert!(scan_id("7").is_err());
        assert!(scan_id("not json").is_err());
    }
}
