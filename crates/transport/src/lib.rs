//! Untyped NDJSON message envelope + codec seam (lsp-server pattern): knows
//! framing, never meaning.
//!
//! # Charter
//! **Owns:** the frame layer — one JSON object per line, blank lines ignored,
//! stdout frames-only — and the envelope classification (request / response /
//! notification). The envelope is deliberately UNTYPED (`serde_json::Value`
//! payloads, rust-analyzer's lsp-server pattern): protocol evolution never
//! forces a transport release and vice versa. r-a swapped its entire types
//! crate with zero transport changes; this seam is that payoff, reserved.
//!
//! **Never does:** know what ops mean, validate op fields (the typed edge is
//! `wire`, consulted at the `sidecar` boundary), touch the filesystem or the
//! model. `wire` appears in dev-dependencies at most.
//!
//! # Rungs
//! Rung 1: `NdjsonCodec` over stdin/stdout. Rung 4: notification frames start
//! flowing (events carry no `id`). Rung 6: an LSP Content-Length codec drops in
//! behind the same [`Codec`] seam — the NDJSON→JSON-RPC graduation touches this
//! crate only.

use std::io::{self, BufRead, Write};

use serde::{Deserialize, Serialize};
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
/// inside). The v1 sidecar emits none.
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
    /// that into the `bad_frame` error *response* is the caller's (sidecar's)
    /// move, because responding is meaning, not framing.
    ///
    /// # Errors
    /// I/O failure reading `input`, or a syntactically unparseable frame.
    fn decode(&self, input: &mut dyn BufRead) -> io::Result<Option<Message>>;
}

/// Newline-delimited JSON framing: one JSON object per line, `\n`-terminated,
/// UTF-8. `echo '{"op":"toc",…}' | ./sidecar` debuggability is a contract
/// property this codec preserves.
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
}
