//! Typed protobuf transport — the opt-in high-performance binary path beside
//! the untyped NDJSON seam.
//!
//! # Charter
//! **Owns:** `proto/meridian.proto` (the protobuf TRANSCRIPTION of
//! wire-contract-v1: ops, node object, error envelope) and varint
//! length-delimited framing over the generated [`pb::Frame`]. Decode lands
//! directly in typed prost structs — no text parse, no string-keyed maps —
//! which is where protobuf's performance win actually lives.
//!
//! **Never does:** business logic, contract amendment, or I/O beyond framing.
//! `wire` remains the canonical contract; this crate mirrors it, and
//! `tests/wire_agreement.rs` pins the mirror at compile time (`wire` is a
//! dev-dependency only). A lib-level conversion seam to `wire` types joins
//! when the sidecar grows proto negotiation, not before.
//!
//! # Law 3 as amended (ZT ruling 2026-07-18)
//! The Go-visible surface is `wire` plus this crate's `.proto`, drift pinned
//! by exhaustive-match agreement tests: a new wire variant or field breaks the
//! agreement test's build until the `.proto` catches up. The untyped
//! `transport` seam stays the default path; contract evolution now touches
//! two transcriptions, and the pin is what keeps that honest.
//!
//! # Framing
//! Standard protobuf streaming: each frame is a LEB128 varint byte length
//! followed by that many bytes of an encoded [`pb::Frame`]. EOF at a frame
//! boundary is clean end-of-stream ([`Ok(None)`](decode)); EOF inside a frame
//! is an error; a length above [`MAX_FRAME_BYTES`] is rejected as corruption —
//! a flipped bit in a length prefix must fail loud, never allocate unbounded.

use std::io::{self, BufRead, Write};

use prost::Message as _;

/// Generated protobuf types for `meridian.v1` (see `proto/meridian.proto`).
#[allow(
    unreachable_pub,
    unused_qualifications,
    rust_2018_idioms,
    clippy::pedantic
)]
pub mod pb {
    include!(concat!(env!("OUT_DIR"), "/meridian.v1.rs"));
}

/// Frames longer than this are rejected as corruption. Frames are editor-op
/// sized (requests are small; a node-list response for even a monster file is
/// megabytes) — a length prefix beyond this bound is a corrupt or hostile
/// stream, and the correct failure is an error, not an allocation.
pub const MAX_FRAME_BYTES: usize = 256 * 1024 * 1024;

/// Encode one frame: varint length prefix + protobuf body.
///
/// # Errors
/// I/O failure writing to `out`.
pub fn encode(frame: &pb::Frame, out: &mut dyn Write) -> io::Result<()> {
    let mut buf = Vec::with_capacity(frame.encoded_len() + 10);
    frame
        .encode_length_delimited(&mut buf)
        .map_err(io::Error::other)?;
    out.write_all(&buf)
}

/// Decode the next frame; `Ok(None)` at a clean EOF (stream end at a frame
/// boundary). An undecodable body or corrupt length prefix is an `Err` here —
/// turning that into a `bad_frame` error *response* is the caller's move,
/// because responding is meaning, not framing (same split as the NDJSON seam).
///
/// # Errors
/// I/O failure reading `input`, EOF inside a frame, a length prefix above
/// [`MAX_FRAME_BYTES`], or a body protobuf cannot decode.
pub fn decode(input: &mut dyn BufRead) -> io::Result<Option<pb::Frame>> {
    let Some(len) = read_frame_len(input)? else {
        return Ok(None);
    };
    let mut buf = vec![0u8; len];
    input.read_exact(&mut buf)?;
    pb::Frame::decode(buf.as_slice())
        .map(Some)
        .map_err(io::Error::other)
}

/// Read a LEB128 length prefix. `Ok(None)` only when EOF lands exactly on a
/// frame boundary (zero bytes read).
fn read_frame_len(input: &mut dyn BufRead) -> io::Result<Option<usize>> {
    let mut value: u64 = 0;
    let mut shift: u32 = 0;
    loop {
        let mut byte = [0u8; 1];
        if input.read(&mut byte)? == 0 {
            return if shift == 0 {
                Ok(None)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "EOF inside a frame length prefix",
                ))
            };
        }
        value |= u64::from(byte[0] & 0x7f) << shift;
        if byte[0] & 0x80 == 0 {
            let len = usize::try_from(value).map_err(io::Error::other)?;
            if len > MAX_FRAME_BYTES {
                return Err(io::Error::other(format!(
                    "frame length {len} exceeds MAX_FRAME_BYTES {MAX_FRAME_BYTES} — corrupt length prefix"
                )));
            }
            return Ok(Some(len));
        }
        shift += 7;
        if shift >= 64 {
            return Err(io::Error::other("length prefix varint overflow"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toc_frame(id: u64, path: &str) -> pb::Frame {
        pb::Frame {
            kind: Some(pb::frame::Kind::Request(pb::Request {
                id: Some(id),
                op: Some(pb::request::Op::Toc(pb::TocRequest { path: path.into() })),
            })),
        }
    }

    #[test]
    fn roundtrip_two_frames_then_clean_eof() {
        let a = toc_frame(1, "tasks/x.md");
        let b = toc_frame(2, "notes/y.md");
        let mut buf = Vec::new();
        encode(&a, &mut buf).unwrap();
        encode(&b, &mut buf).unwrap();

        let mut input: &[u8] = &buf;
        assert_eq!(decode(&mut input).unwrap(), Some(a));
        assert_eq!(decode(&mut input).unwrap(), Some(b));
        assert_eq!(decode(&mut input).unwrap(), None); // frame-boundary EOF
    }

    #[test]
    fn eof_inside_payload_is_an_error() {
        let mut buf = Vec::new();
        encode(&toc_frame(1, "tasks/x.md"), &mut buf).unwrap();
        let mut truncated: &[u8] = &buf[..buf.len() - 3];
        let err = decode(&mut truncated).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn eof_inside_length_prefix_is_an_error() {
        let mut input: &[u8] = &[0x80]; // continuation bit set, then EOF
        let err = decode(&mut input).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn oversized_length_prefix_is_rejected_without_allocating() {
        // varint for 1 GiB (> MAX_FRAME_BYTES), no payload behind it
        let mut input: &[u8] = &[0x80, 0x80, 0x80, 0x80, 0x04];
        let err = decode(&mut input).unwrap_err();
        assert!(err.to_string().contains("corrupt length prefix"), "{err}");
    }

    #[test]
    fn undecodable_body_is_an_error() {
        // length 3, then bytes that are not a valid Frame (field 15 wire type 7)
        let mut input: &[u8] = &[0x03, 0x7f, 0x7f, 0x7f];
        assert!(decode(&mut input).is_err());
    }
}
