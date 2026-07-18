//! # wire — the only Go-visible surface
//!
//! **Charter:** wire-contract-v1 vocabulary, node schema, op/req/resp/error
//! types, the `Message` envelope, and the `Codec` framing seam — all in one
//! crate. Nothing else in this workspace is visible across the process
//! boundary.
//!
//! **Owns:** the four wire nouns (`path` / `span` / `node_rev` / `root`), the
//! node schema and kind enum, request/response shapes per op, the §4 error
//! envelope, frame classification, and framing codecs.
//! **Never does:** business logic, file I/O, parsing markdown, or knowing what
//! an op *means* — it types the conversation, the `meridian` lib answers it.
//!
//! **Contract:** `wire-contract-v1.md` (proto 1, revision round 1 — not yet
//! frozen; stubs track the current contract file, post-freeze deltas land in a
//! re-verify pass). Where this file and the contract disagree, the contract
//! wins.
//!
//! ## Graduation claim (why framing lives here and not in its own crate)
//!
//! JSON-RPC/LSP framing lands as a **second [`codec::Codec`] impl in THIS
//! crate**, consumers untouched: the trait is byte-stream shaped, so
//! `NdjsonCodec` (newline frames) and a future `JsonRpcCodec` (Content-Length
//! frames) are interchangeable behind it. The §5 graduation test — "NDJSON →
//! JSON-RPC touches one crate" — is satisfied by an impl, not a crate split.
//! A `framing-jsonrpc` crate is earned only if a *second consumer* of framing
//! (something that is not this workspace's sidecar/daemon) appears; see the
//! branch README's satellite map.
//!
//! ## Rung map
//!
//! - rung 1 (frozen ops): [`op::HelloReq`], [`op::TocReq`], [`op::ExtractReq`]
//! - rung 2 (sketched, contract §6): [`op::ResolveReq`], [`op::SpliceReq`]
//! - rung 3 (reserved): [`op::Op::Root`], [`op::Op::Guard`] — named by the
//!   capability ladder, no contract shape yet; shapes arrive by amendment
//! - rung 4+: event frames only classified here ([`envelope::EventFrame`]);
//!   `subscribe` shapes are entirely rung-4 decisions

pub mod codec;
pub mod envelope;
pub mod error;
pub mod node;
pub mod op;
pub mod vocab;

pub use codec::{Codec, CodecError, NdjsonCodec};
pub use envelope::{EventFrame, Message, RequestFrame, ResponseFrame};
pub use error::{ErrorBody, ErrorCode};
pub use node::{Node, NodeInfo, NodeKind};
pub use op::{Op, OkBody, ResponseBody};
pub use vocab::{NodeRev, RequestId, Root, Span, WirePath, PROTO};
