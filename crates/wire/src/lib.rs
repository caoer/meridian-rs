//! The frozen wire vocabulary: path/span/node_rev/root + op/request/response/error
//! types (serde-only, zero I/O) — the only Go-visible surface.
//!
//! # Charter
//! **Owns:** the four wire nouns (`path`/`span`/`node_rev`/`root`), the op
//! request/response shapes, the node object, and the error envelope — everything
//! that crosses the process boundary, exactly as `wire-contract-v1.md` states it
//! (session `18-02-meridian-rs/results/wire-contract-v1.md`; contract text is
//! normative, this crate transcribes it and never restates its rules).
//!
//! **Never does:** framing, transport, I/O, business logic. Dependencies are serde
//! only, by law — this crate must be consumable by any future client (tests, Go
//! codegen, fixture tooling) without dragging a runtime.
//!
//! # Law enforcement (this crate's part), law 3 as amended by review C1
//! `model`'s types carry no serde derives; only this crate's types serialize.
//! **Only the named `wire-map` seam and the `sidecar` bin see wire and model
//! together** — projection is a tested library function in `wire-map`, never bin
//! code; the bin stays wiring-only.
//!
//! # Rungs
//! - Rung 1 (`hello`/`toc`/`extract`, error envelope, node object): contract §3–§5,
//!   FROZEN 2026-07-18 (`proto: 1`).
//! - Rung 2 (`resolve`/`splice`): contract §6.1–§6.2, NON-FROZEN sketches.
//! - Rung 3 (`root`/`guard`): contract §6.4 reserved shapes; exact shapes
//!   authoritative in node-rev-merkle-spec §7 — no longer vision-ladder guesses.
//!
//! # Build-out obligations (contract laws the types alone cannot enforce)
//! - **§7 rule 2, server side:** unknown request fields MUST be rejected with
//!   `bad_request` — serde's default ignores them, and `deny_unknown_fields`
//!   does not compose with `flatten`; rung-1 build-out owes a strict decode pass.
//! - **§4/§7 rule 1, client side:** a client-side consumer of these types must
//!   tolerate unknown error codes (non-retryable) and unknown node kinds
//!   (ignore) — the closed enums below serve the EMITTING side; a Rust client
//!   crate would need catch-all decode variants by amendment.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// §2 wire vocabulary — the four nouns
// ---------------------------------------------------------------------------

/// Workspace-relative file path (contract §2): forward slashes, UTF-8, no leading
/// `/`, no `.`/`..` segments. Violations are the server's `bad_path`, not a client
/// panic — this newtype does not validate, it names.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Path(pub String);

/// Half-open byte range `[start, end)` into raw on-disk bytes (contract §2 span
/// laws 1–5). Serializes as the two-element array `[start, end]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span(pub u64, pub u64);

/// Opaque node content hash (contract §2): compare for equality only — never
/// parse, never order. Algorithm/length are a rung-2 amendment; the CAS-token
/// role is frozen.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodeRev(pub String);

/// Opaque Merkle root cursor, algorithm-prefixed hex, e.g. `"b3:88d2aa"`
/// (contract §2). Equality comparison only. Rung 3–4 integrity cursor.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Root(pub String);

/// Client-chosen correlation token: a JSON integer in `[0, 2^53)` (contract §3).
/// `u64` here; the `< 2^53` bound is the server's `bad_request` to enforce.
pub type RequestId = u64;

// ---------------------------------------------------------------------------
// §3 requests — rungs 1–3 op enum
// ---------------------------------------------------------------------------

/// A request frame: `id` beside the op-tagged fields, exactly the §3 layout.
/// `id` is optional (shell-pipe debuggability); pipelining clients MUST send it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<RequestId>,
    #[serde(flatten)]
    pub op: Op,
}

/// The op vocabulary, rungs 1–3. Tag field is `op` (contract §3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    /// §5.1 version handshake. FROZEN.
    Hello {
        proto: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        client: Option<String>,
    },
    /// §5.3 section map: frontmatter + heading nodes only. FROZEN.
    Toc { path: Path },
    /// §5.4 full node inventory; `kinds` filters, unknown names match nothing. FROZEN.
    Extract {
        path: Path,
        #[serde(skip_serializing_if = "Option::is_none")]
        kinds: Option<Vec<String>>,
    },
    /// §6.1 ref → span + node_rev. Rung 2, NON-FROZEN sketch.
    Resolve {
        path: Path,
        #[serde(rename = "ref")]
        r#ref: String,
    },
    /// §6.2 node-level CAS write. Rung 2, NON-FROZEN sketch.
    Splice {
        path: Path,
        span: Span,
        if_node_rev: NodeRev,
        text: String,
    },
    /// Rung 3 integrity read: current cursor at workspace, subtree, or file
    /// scope (a file-scope root is the file's leaf hash). Contract §6.4
    /// reserved shape; exact shapes per node-rev-merkle-spec §7.
    Root {
        #[serde(skip_serializing_if = "Option::is_none")]
        path: Option<Path>,
    },
    /// Rung 3 commit guard: validate a held root; on mismatch the error names
    /// the drift (`root_mismatch` w/ `expected`/`actual`/`scope`/`changed`).
    /// Contract §6.4 reserved shape (field is `root`, per the frozen text);
    /// exact shapes per node-rev-merkle-spec §7.
    Guard {
        root: Root,
        #[serde(skip_serializing_if = "Option::is_none")]
        path: Option<Path>,
    },
}

// ---------------------------------------------------------------------------
// §5.2 the node object (shared by toc and extract)
// ---------------------------------------------------------------------------

/// Node kinds, v1 enum — declaration order IS the frozen sort-tiebreak ordinal
/// (`frontmatter` = 0 … `comment` = 10, contract §5.2 node ordering).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NodeKind {
    Frontmatter,
    Heading,
    Fence,
    InlineCode,
    Anchor,
    Wikilink,
    Embed,
    Callout,
    Task,
    Table,
    Comment,
}

/// The wire node (contract §5.2): kind + span + prefix window, `hpath` on
/// headings only (array form, one element per heading, delimiter-free),
/// `unterminated` present only when true, `info` per-kind.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub kind: NodeKind,
    pub span: Span,
    /// Prefix law (§5.2, frozen): 16-byte window clamped at EOF, valid-UTF-8
    /// head decoded, trailing split-multibyte bytes escaped `\xhh`. Both sides
    /// compute it and MUST agree bit-for-bit.
    pub text_prefix_16b: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hpath: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unterminated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub info: Option<Info>,
    /// §5.2 (frozen, optional): the node's opaque CAS token, minted over its
    /// exact span bytes. A minting sidecar SHOULD attach it to every node;
    /// clients MUST tolerate its absence. Admitted onto the frozen node object
    /// because `toc` is where CAS tokens are born (else one `resolve` round
    /// trip per node). [post-freeze re-verify addition]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_rev: Option<NodeRev>,
}

/// Per-kind `info` payloads (contract §5.2 table). Untagged: the sibling `kind`
/// field discriminates; kinds with no `info` shape simply omit the key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Info {
    Frontmatter {
        keys: Vec<String>,
    },
    Fence {
        info_string: String,
    },
    /// `wikilink` and `embed` share this shape. `heading` and `block` are
    /// mutually exclusive; `block` uses the anchor charset `[A-Za-z0-9_-]+`
    /// (one charset everywhere a block ID appears — B-decision 1).
    Wikilink {
        target: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        heading: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        block: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        alias: Option<String>,
    },
    Callout {
        r#type: String,
        fold: String,
    },
    Task {
        checked: bool,
        depth: u32,
    },
}

// ---------------------------------------------------------------------------
// §3/§5 responses
// ---------------------------------------------------------------------------

/// A response frame: `id` echoed by value — serialized even when `null` (a frame
/// *containing* the `id` key is a response; one without is an event, §3 frame
/// classification) — plus `ok` and the per-op success body or error envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    /// Always serialized; `None` ⇒ `"id":null` (no-id request, bad frame).
    pub id: Option<RequestId>,
    pub ok: bool,
    #[serde(flatten)]
    pub body: ResponseBody,
}

/// Success bodies per op, or the error envelope. Untagged: `ok` + field shape
/// discriminate on the wire; typed clients match on what they asked for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ResponseBody {
    /// §5.1: `proto` now in effect, server name, and the COMPLETE op-name set
    /// (`caps` includes `hello` itself; capability discovery, never version
    /// sniffing).
    Hello {
        proto: u32,
        server: String,
        caps: Vec<String>,
    },
    /// §5.3/§5.4: `toc` and `extract` share this shape (`toc` ≡ `extract`
    /// filtered to frontmatter+heading). Nodes in frozen node order.
    Nodes { path: Path, nodes: Vec<Node> },
    /// §6.1 sketch: the section (or anchor-host-block) span + its CAS token.
    /// `content_span` (heading refs only): after the heading line to the same
    /// end — the write target for heading-preserving section replacement;
    /// mints no separate rev. [post-freeze re-verify addition]
    Resolve {
        path: Path,
        span: Span,
        node_rev: NodeRev,
        #[serde(skip_serializing_if = "Option::is_none")]
        content_span: Option<Span>,
    },
    /// §6.2 sketch: new span + new rev of the spliced region, so clients chain
    /// edits without re-`resolve`. A `root` field joins additively at rung 3.
    Splice { span: Span, node_rev: NodeRev },
    /// Rung 3 reserved (contract §6.4): the current root at the requested scope.
    Root { root: Root },
    /// Rung 3 `guard` success (contract §6.4): the root that matched. Mismatch
    /// is the error envelope, `root_mismatch` — §6.5 reserved code, shapes per
    /// node-rev-merkle-spec §8.
    Guard { root: Root },
    /// §4 error envelope — see [`ErrorBody`].
    Error(ErrorBody),
}

// ---------------------------------------------------------------------------
// §4 error envelope
// ---------------------------------------------------------------------------

/// Error codes, v1 namespace (contract §4 table, verbatim; flat lowercase
/// snake_case on the wire). Clients treat unrecognized codes as non-retryable;
/// `cas_mismatch` is the one code whose *purpose* is retry-after-refresh.
///
/// §6.5 reserved codes join this enum only via the amendments that freeze
/// their rungs: `would_corrupt`, `ambiguous_ref`, `lock_timeout` (rung 2);
/// `root_mismatch` (extra fields `expected`/`actual`/`scope`/`changed`),
/// `root_unknown` (rung 3, shapes per node-rev-merkle-spec §8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    BadFrame,
    BadRequest,
    UnknownOp,
    UnsupportedProto,
    BadPath,
    NotFound,
    InvalidUtf8,
    Internal,
    CasMismatch,
}

/// §4: `error` at the top level, optional human `message`, code-specific fields
/// beside them (never nested). Serde-only constraint (no `serde_json::Value`
/// grab-bag) makes the extras typed options — exactly the fields the v1 table
/// names, absent unless their code sets them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorBody {
    pub error: ErrorCode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// `bad_path` / `not_found`: the offending/requested path, echoed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<Path>,
    /// `unsupported_proto`: protos this sidecar speaks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supported: Option<Vec<u32>>,
    /// `cas_mismatch`: the rev the client pinned vs the rev that is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<NodeRev>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual: Option<NodeRev>,
}
