//! The frozen wire vocabulary: `path/span/node_rev/root` + address grammar
//! (`SecRef`/`HpathSeg`) + op/request/response/error types (serde-only, zero
//! I/O) — the only Go-visible surface.
//!
//! # Charter
//! **Owns:** the wire nouns (`path`/`span`/`node_rev`/`root`), the §2.1 mint
//! address grammar, the op request/response shapes, the node object, and the
//! error envelope — everything that crosses the process boundary, exactly as
//! `wire-contract-v2.md` states it (session
//! `18-02-meridian-rs/results/wire-contract-v2.md`, FROZEN 2026-07-18,
//! decision 014; contract text is normative, this crate transcribes it and
//! never restates its rules).
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
//! # Rungs (contract v2 §4 op table)
//! - Rung 1 (`hello`, error envelope, node object): v1 FROZEN 2026-07-18
//!   (`proto: 1`), as amended by contract v2 (W2-AMEND): `Node.hpath` carries
//!   `HpathSeg` (v2 §2.1, dual-deserialization for the v1 string form), the
//!   error envelope is the nested `{code, recovery, …}` object (v2 §8).
//! - Rung 2 (`toc` v2 response, `cat`, `extract` + D-C5, `resolve` walk plane,
//!   `SecRef` mint grammar): contract v2 §4.1–§4.5, §2.1 — FROZEN.
//! - Rung 3 (`root` reshape, `diff` request, `root_mismatch`/`root_unknown`,
//!   `guard` DELETED per D-C8): contract v2 §4.7, §8 — FROZEN. The
//!   Delta-bearing `diff` response body lands with the Delta noun (D3-DELTA).
//! - Rung 4 (`splice`): v1 §6.2 NON-FROZEN sketch; the §4.4 batch shape lands
//!   with the rung-4 amendment (W4-AMEND).
//!
//! # Build-out obligations (contract laws the types alone cannot enforce)
//! - **v2 §3.2 evolution, server side:** unknown request fields MUST be rejected
//!   with `bad_request` — serde's default ignores them, and `deny_unknown_fields`
//!   does not compose with `flatten`; the dispatch build-out owes a strict
//!   decode pass.
//! - **v2 §3.2 evolution, client side:** a client-side consumer of these types
//!   must tolerate unknown error codes — dispatching on the closed `recovery`
//!   class alone (v2 §8) — and ignore unknown open-kind strings.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// v2 §1 wire vocabulary — the nouns
// ---------------------------------------------------------------------------

/// Workspace-relative file path (v2 §1): forward slashes, UTF-8, no leading
/// `/`, no `.`/`..` segments. Violations are the server's `bad_path`, not a client
/// panic — this newtype does not validate, it names.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Path(pub String);

/// Half-open byte range `[start, end)` into raw on-disk bytes (v2 §1 span
/// sub-laws). UTF-8 **bytes**, never chars, never UTF-16. Serializes as the
/// two-element array `[start, end]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span(pub u64, pub u64);

/// Opaque node content hash (v2 §1): 16 lowercase hex =
/// `blake3-256(full span bytes)[:16]`. Compare for equality only — never
/// parse, never order. `file_rev` is the same family and width over whole-file
/// bytes; both ride in this newtype.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodeRev(pub String);

/// Opaque Merkle root cursor: `"b3:" + 64 hex`, full width, never truncated
/// (v2 §1). Algorithm+domain prefixed; the prefix bumps on domain-rule change
/// (v2 §12.3). Equality comparison only.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Root(pub String);

/// Client-chosen correlation token: a JSON integer in `[0, 2^53)` (v2 §3.1).
/// `u64` here; the raw-lexeme B2 law is the transport seam's to enforce.
pub type RequestId = u64;

// ---------------------------------------------------------------------------
// v2 §2.1 the mint address grammar (the ONE fleet grammar)
// ---------------------------------------------------------------------------

/// One hpath segment: raw heading text `h` + optional 1-based occurrence
/// index `n` (document order among identical raw texts at that position —
/// v2 §2.1). Per-segment byte-equality against the real containment tree; no
/// join string exists, so `#A#a/b` vs `#A#a#b` is unrepresentable.
///
/// Serializes as the object form `{"h":…}` / `{"h":…,"n":…}`. Deserializes
/// from BOTH the object form and the v1 bare string (`"Goals"` ≡
/// `{"h":"Goals"}`) — the dual-serialization bridge for the one amendment to
/// the frozen node object (v2 §2.1; deviation row in the W2-AMEND fixtures).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct HpathSeg {
    pub h: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n: Option<u32>,
}

impl<'de> Deserialize<'de> for HpathSeg {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Str(String),
            Seg {
                h: String,
                #[serde(default)]
                n: Option<u32>,
            },
        }
        Ok(match Repr::deserialize(deserializer)? {
            Repr::Str(h) => HpathSeg { h, n: None },
            Repr::Seg { h, n } => HpathSeg { h, n },
        })
    }
}

/// A strict mint-plane section ref — the three §2.1 forms, and no other
/// grammar on any ref-carrying wire surface (`cat`/`splice` targets and the
/// echoes in `toc` rows, receipts, deltas, verdicts). Stale names fail loud
/// (`ref_not_found`); duplicates refuse loud (`ambiguous_ref`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SecRef {
    /// `{"hpath":[{"h":"Goals"},{"h":"Q3"}]}` — per-segment byte-equality.
    Hpath { hpath: Vec<HpathSeg> },
    /// `{"anchor":"r-000042"}` — block id, exact match, charset
    /// `[A-Za-z0-9-]+` on both planes (v2 §2.4, decision 011).
    Anchor { anchor: String },
    /// `{"fm_key":"title"}` — top-level frontmatter key; the node is the full
    /// key line (frontmatter plane is nodes, never ref grammar).
    FmKey { fm_key: String },
}

// ---------------------------------------------------------------------------
// v2 §4 requests — the op vocabulary
// ---------------------------------------------------------------------------

/// A request frame: `id` beside the op-tagged fields, exactly the §3.1 layout.
/// `id` is optional (shell-pipe debuggability); pipelining clients MUST send it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<RequestId>,
    #[serde(flatten)]
    pub op: Op,
}

/// The op vocabulary. Tag field is `op` (v2 §3.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    /// v2 §3.2 version handshake; `caps` discovery, never version sniffing.
    Hello {
        proto: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        client: Option<String>,
    },
    /// v2 §4.1 the map: complete write kit (hpath + rev per section, anchors
    /// with revs, fm keys), header `file_rev` + ambient `root` in the response.
    Toc { path: Path },
    /// v2 §4.2 read one section, not the disk: full span bytes, rev = blake3
    /// of precisely those bytes. `sec` absent → whole file + `file_rev`.
    Cat {
        path: Path,
        #[serde(skip_serializing_if = "Option::is_none")]
        sec: Option<SecRef>,
    },
    /// v2 §4.3 full node inventory; `kinds` filters. An unknown value in
    /// `kinds` is `bad_request{unknown_kinds}`, loud (D-C5 — reverses v1's
    /// "unknown names match nothing"; deviation row v2 §18 row 7).
    Extract {
        path: Path,
        #[serde(skip_serializing_if = "Option::is_none")]
        kinds: Option<Vec<String>>,
    },
    /// v2 §4.5 the walk plane: best-effort app-compatible two-stage walk over
    /// the raw Obsidian linktext. Location facts only — the response type has
    /// no rev field to return (D-C2, mint partition as type law). `from` is
    /// mandatory: resolution is source-relative. `content:true` additionally
    /// returns the fragment bytes — still no rev.
    Resolve {
        from: Path,
        #[serde(rename = "ref")]
        r#ref: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        content: Option<bool>,
    },
    /// v1 §6.2 node-level CAS write, NON-FROZEN sketch. The v2 §4.4 batch
    /// shape lands with the rung-4 amendment (W4-AMEND).
    Splice {
        path: Path,
        span: Span,
        if_node_rev: NodeRev,
        text: String,
    },
    /// v2 §4.7 integrity read: the current workspace root cursor + `seq`.
    /// No parameters — the root is world-grain (the only root guard is
    /// `splice.if_root`, §5.1; the v1 scoped/`path` variant is gone with
    /// `guard`).
    Root,
    /// v2 §4.7 replay, reserved AT the integrity rung with its shape frozen
    /// now (A6/L55 — the compound front door). The response carries Delta
    /// batches byte-identical to the live notification frames (§7.3); the
    /// Delta-bearing response body lands with the Delta noun (D3-DELTA).
    Diff { from_root: Root, to_root: Root },
}

// ---------------------------------------------------------------------------
// v1 §5.2 the node object (extract) + v2 §4.1 the toc row
// ---------------------------------------------------------------------------

/// Node kinds, v1 enum — declaration order IS the frozen sort-tiebreak ordinal
/// (`frontmatter` = 0 … `comment` = 10, v2 §4.3 carries the v1 §5.2 order).
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

/// The wire node (v1 §5.2, as amended): kind + span + prefix window, `hpath`
/// on headings only — now in the §2.1 mint grammar (`HpathSeg`, the one v2
/// touch on this FROZEN shape; dual-deserialization keeps v1 string arrays
/// readable), `unterminated` present only when true, `info` per-kind.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub kind: NodeKind,
    pub span: Span,
    /// Prefix law (v1 §5.2, frozen): 16-byte window clamped at EOF, valid-UTF-8
    /// head decoded, trailing split-multibyte bytes escaped `\xhh`. Both sides
    /// compute it and MUST agree bit-for-bit.
    pub text_prefix_16b: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hpath: Option<Vec<HpathSeg>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unterminated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub info: Option<Info>,
    /// v2 §3.2 S2/L22 law: MUST on every `toc`/`cat`/`extract` node whenever
    /// `splice ∈ caps`; clients MUST tolerate its absence.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_rev: Option<NodeRev>,
}

/// One `toc` row (v2 §4.1): the complete write kit for one node. Row shapes,
/// worked in the contract: frontmatter rows carry `keys`; heading rows carry
/// `level` + `hpath` + `content_span`; anchor-bearing block rows carry
/// `anchor` and echo their HOST block kind (the worked anchor row is
/// `"kind":"list_item"` — outside the closed extract enum, so `kind` here is
/// the open string the tolerant-client law already covers, v2 §3.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TocNode {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hpath: Option<Vec<HpathSeg>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor: Option<String>,
    pub span: Span,
    /// Serves heading-preserving display; mints NOTHING (v2 §1 — one rev per
    /// node, over full span bytes).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_span: Option<Span>,
    pub node_rev: NodeRev,
    pub text_prefix_16b: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keys: Option<Vec<String>>,
}

/// Per-kind `info` payloads (v1 §5.2 table). Untagged: the sibling `kind`
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
    /// mutually exclusive; `block` uses the block-id charset `[A-Za-z0-9-]+`
    /// (ONE charset, both planes — v2 §2.4, decision 011).
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
// v2 §4 responses
// ---------------------------------------------------------------------------

/// A response frame: `id` echoed by value — serialized even when `null` (a frame
/// *containing* the `id` key is a response; one without is a Notification, §3.1
/// frame classification) — plus `ok` and the payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    /// Always serialized; `None` ⇒ `"id":null` (no-id request, bad frame).
    pub id: Option<RequestId>,
    pub ok: bool,
    #[serde(flatten)]
    pub payload: ResponsePayload,
}

/// The v2 frame law, uniform across every worked exchange (§3.2–§10.2):
/// a success payload nests under the `body` key; a failure carries the §8
/// error envelope under the `error` key. `ok` mirrors which arm rides.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ResponsePayload {
    Body { body: ResponseBody },
    Error { error: ErrorBody },
}

/// Success bodies per op. Untagged: field shape discriminates on the wire;
/// typed clients match on what they asked for. Variant ORDER is load-bearing
/// for deserialization: a shape-superset variant must precede its subset
/// (Toc before Nodes, Cat before Splice).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ResponseBody {
    /// v2 §3.2: `proto` in effect, server name, the COMPLETE op-name set
    /// (`caps` includes dotted `op.field` strings for field-only amendments),
    /// optional first ambient `root`.
    Hello {
        proto: u32,
        server: String,
        caps: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        root: Option<Root>,
    },
    /// v2 §4.1: the map — header `file_rev` + ambient `root` (the commit-guard
    /// idiom made ambient: read a toc, later pass `if_root`), rows in frozen
    /// node order.
    Toc {
        path: Path,
        file_rev: NodeRev,
        root: Root,
        nodes: Vec<TocNode>,
    },
    /// v2 §4.3: `extract`'s full node inventory, frozen node order.
    Nodes { path: Path, nodes: Vec<Node> },
    /// v2 §4.2: the full span bytes (heading-inclusive) and the rev over
    /// precisely those bytes — the ambient-rev property with zero fine print.
    /// `sec` absent in the request ⇒ `span` is the whole file and the rev IS
    /// the `file_rev` (same family, same width, same bytes — v2 §1).
    Cat {
        span: Span,
        node_rev: NodeRev,
        content: String,
    },
    /// v2 §4.5: location facts only — NO rev field exists to return (D-C2, the
    /// mint partition as a type-level fact). `content` rides only when the
    /// request set `content:true`; still no rev.
    Resolve {
        dest: Path,
        span: Span,
        #[serde(skip_serializing_if = "Option::is_none")]
        content: Option<String>,
    },
    /// v1 §6.2 sketch; the v2 §4.4 armed-facts shape lands with W4-AMEND.
    Splice { span: Span, node_rev: NodeRev },
    /// v2 §4.7: the current root at world grain + `seq`, the monotone
    /// per-workspace batch counter (per-daemon-epoch — a restart resets it;
    /// cross-epoch catchup is diff-by-root, §7.1 laws).
    Root { root: Root, seq: u64 },
}

// ---------------------------------------------------------------------------
// v2 §8 error taxonomy — six recovery classes
// ---------------------------------------------------------------------------

/// The CLOSED six-class recovery enum (v2 §8). Every error frame carries one;
/// a client that doesn't recognize a `code` dispatches on `recovery` alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Recovery {
    /// Your request is wrong; change it.
    Fix,
    /// The world outside the workspace is wrong.
    Env,
    /// Your picture of one node is stale; re-read one thing.
    Refresh,
    /// Transient; the same request may succeed.
    Retry,
    /// Your picture of the world is stale; re-plan.
    Resync,
    /// The channel itself is broken.
    Respawn,
}

/// Error codes (v2 §8 table; flat lowercase `snake_case` on the wire). Each
/// code is statically bound to exactly one recovery class — [`ErrorCode::recovery`]
/// is that binding, verbatim from the frozen table. Clients treat unrecognized
/// codes as `recovery`-dispatched.
///
/// Remaining v2 codes join via the amendments that freeze their rungs:
/// `root_mismatch`/`root_unknown` (W3-AMEND); `no_match`/`not_unique`/
/// `would_corrupt`/`lock_timeout` + the `not_found` retirement into
/// `file_not_found`/`io_error` (W4-AMEND); `stale_view`/`daemon_only` with
/// their ops.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    BadFrame,
    BadRequest,
    UnknownOp,
    UnsupportedProto,
    BadPath,
    /// v1 code, RETIRED by v2 §8 (split `file_not_found` env / `ref_not_found`
    /// refresh); the retirement lands with W4-AMEND. Until then it binds `env`
    /// (its file-gone successor's class).
    NotFound,
    InvalidUtf8,
    Internal,
    CasMismatch,
    /// v2 §8: the name dangles — refresh one thing. Extras: `stage` (1 =
    /// vault-namespace miss, no `dest`; 2 = subpath miss, `dest` present).
    RefNotFound,
    /// v2 §8: the strict plane refuses to pick where the walk would silently
    /// choose. Extras: `candidates` (§2.1 refs naming each target exactly).
    AmbiguousRef,
    /// v2 §8/§5.1: a failed world guard (`if_root`) — the plan is invalid,
    /// not one node's picture. Extras: `expected`/`actual` (roots) +
    /// `changed`. Ships WITHOUT the repo-reserved `scope` field (§18 row 2,
    /// WAIVED: no scoped-root construct exists for it to describe).
    RootMismatch,
    /// v2 §4.7: a root range outside the retained history — full resync; the
    /// root is the only restart-durable handle. No extras.
    RootUnknown,
}

impl ErrorCode {
    /// The static code→class binding, v2 §8 verbatim — including the two
    /// declared rebinds (`unsupported_proto` fix→`respawn`: a protocol
    /// mismatch is a channel property; `root_mismatch` refresh→`resync`: a
    /// failed world guard invalidates the plan, not one node's picture) and
    /// `bad_id` absent (folded into `bad_request` + `id:null`/`id_raw`,
    /// v2 §3.1).
    #[must_use]
    pub const fn recovery(self) -> Recovery {
        match self {
            ErrorCode::BadRequest
            | ErrorCode::UnknownOp
            | ErrorCode::BadPath
            | ErrorCode::AmbiguousRef => Recovery::Fix,
            ErrorCode::NotFound | ErrorCode::InvalidUtf8 => Recovery::Env,
            ErrorCode::CasMismatch | ErrorCode::RefNotFound => Recovery::Refresh,
            ErrorCode::RootMismatch | ErrorCode::RootUnknown => Recovery::Resync,
            ErrorCode::BadFrame | ErrorCode::UnsupportedProto | ErrorCode::Internal => {
                Recovery::Respawn
            }
        }
    }
}

/// v2 §8: the error envelope — a nested object under the response's `error`
/// key: `code` + the REQUIRED closed `recovery` class + optional human
/// `message` + code-specific extras beside them (never nested further).
/// Serde-only constraint (no `serde_json::Value` grab-bag) makes the extras
/// typed options — absent unless their code sets them. Construct via
/// [`ErrorBody::new`] to get the §8 binding by construction; `recovery` being
/// non-optional is the type-level "no error frame without it".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorBody {
    pub code: ErrorCode,
    pub recovery: Recovery,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// `bad_path` / v1 `not_found`: the offending/requested path, echoed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<Path>,
    /// `unsupported_proto`: protos this sidecar speaks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supported: Option<Vec<u32>>,
    /// The pinned vs current comparison token — `cas_mismatch`: node revs;
    /// `root_mismatch`: full-width roots. One wire key per the frozen §8
    /// table; `code` discriminates the grain (both tokens are opaque,
    /// equality-only — the newtype names the slot, not the algebra).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<NodeRev>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual: Option<NodeRev>,
    /// `root_mismatch`: the files that drifted under the plan — read it,
    /// re-`toc` the affected files, re-plan, re-arm with the fresh root.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub changed: Option<Vec<Path>>,
    /// `ref_not_found`: the failing walk stage (1 or 2), observable in every
    /// transcript (v2 §4.5).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<u32>,
    /// `ref_not_found` stage 2: `dest` rides every stage-2 outcome, success or
    /// failure (v2 §4.5).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dest: Option<Path>,
    /// `ambiguous_ref`: each candidate named exactly in the §2.1 mint grammar
    /// (occurrence index or anchor) — a ref-carrying surface, so THE grammar.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidates: Option<Vec<SecRef>>,
    /// `bad_request` on `extract` (D-C5, v2 §4.3): the unknown `kinds` values,
    /// echoed loud.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unknown_kinds: Option<Vec<String>>,
    /// `bad_request` on a non-conforming raw `id` lexeme (B2 law, v2 §3.1):
    /// the offending lexeme verbatim, beside `id:null`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id_raw: Option<String>,
}

impl ErrorBody {
    /// An envelope for `code` with the §8-bound recovery class and no extras.
    #[must_use]
    pub fn new(code: ErrorCode) -> Self {
        ErrorBody {
            code,
            recovery: code.recovery(),
            message: None,
            path: None,
            supported: None,
            expected: None,
            actual: None,
            changed: None,
            stage: None,
            dest: None,
            candidates: None,
            unknown_kinds: None,
            id_raw: None,
        }
    }
}
