//! **Charter:** framing — how [`Message`]s become bytes on a stream and come
//! back. The codec seam is the graduation point: `NdjsonCodec` today, a
//! `JsonRpcCodec` (Content-Length framing, rung 6 LSP) lands as a second impl
//! **in this crate**, consumers untouched. Framing never knows what ops mean.

use crate::envelope::Message;

/// A framing error (not a protocol error): the bytes could not be framed or
/// unframed. The caller maps this to the contract's `bad_frame` envelope
/// (server side) or to stream-corrupt recovery (pipelining client side:
/// fail all outstanding requests, respawn the sidecar — contract §3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodecError {
    pub message: String,
}

/// The framing seam. Byte-stream shaped on purpose: newline-delimited (NDJSON)
/// and length-prefixed (JSON-RPC/LSP) framings both fit behind it, which is
/// what makes the §5 "NDJSON → JSON-RPC touches one crate" promise an impl
/// swap instead of a redesign.
pub trait Codec {
    /// Append the encoded frame for `msg` to `out`.
    fn encode(&mut self, msg: &Message, out: &mut Vec<u8>) -> Result<(), CodecError>;

    /// Try to decode one complete frame from the front of `buf`.
    /// `Ok(Some((msg, consumed)))` on a complete frame, `Ok(None)` when more
    /// bytes are needed, `Err` on a malformed frame (the caller decides
    /// whether that's `bad_frame` or stream-corrupt).
    fn decode(&mut self, buf: &[u8]) -> Result<Option<(Message, usize)>, CodecError>;
}

/// NDJSON framing (contract §3, frozen): one JSON object per line, `\n`
/// terminated, UTF-8; empty/whitespace lines ignored; stdout carries frames
/// only. `echo '{"op":"toc","path":"x.md"}' | ./sidecar` is a contract
/// property, not an accident.
#[derive(Debug, Default)]
pub struct NdjsonCodec;

impl Codec for NdjsonCodec {
    fn encode(&mut self, _msg: &Message, _out: &mut Vec<u8>) -> Result<(), CodecError> {
        todo!("serialize one frame per line; JSON string escaping covers embedded newlines")
    }

    fn decode(&mut self, _buf: &[u8]) -> Result<Option<(Message, usize)>, CodecError> {
        todo!("split on newline; skip blank lines; classify request/response/event per envelope rules")
    }
}
