//! The frozen wire vocabulary: `path/span/node_rev/root` + address grammar
//! (`SecRef`/`HpathSeg`) + op/request/response/error types (serde-only, zero
//! I/O) — the only Go-visible surface.
//!
//! # Charter
//! **Owns:** the wire nouns (`path`/`span`/`node_rev`/`root`), the §2.1 mint
//! address grammar, the op request/response shapes, the node object, and the
//! error envelope — everything that crosses the process boundary, exactly as
//! the frozen contract states it (`docs/wire-contract-v2.md`; the contract
//! text is normative, this crate transcribes it and never restates its rules).
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
//!   `guard` DELETED per D-C8): contract v2 §4.7, §8 — FROZEN. The Delta
//!   noun + the Delta-bearing `diff` response body: contract v2 §7 — FROZEN
//!   (D3-DELTA; node-grain at birth per decision 012, §7.4).
//! - Rung 4 (`splice` §4.4 batch shape, armed-facts response, receipts §6,
//!   `no_match`/`not_unique`/`would_corrupt`/`lock_timeout`, the `not_found`
//!   retirement §18 row 6): FROZEN.
//! - Rung 5 (`links` §4.6 view-shaped fact op + the §10.1 staleness triple +
//!   `stale_view` §10.2): contract v2 §4.6, §10 — FROZEN (Q5-LINKS).
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
use std::collections::BTreeMap;

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
// v2 §9 actor and now — wire inputs, never ambient
// ---------------------------------------------------------------------------

/// The §9 `now` format law, transcribed: RFC 3339 date-time —
/// `YYYY-MM-DDTHH:MM:SS[.frac](Z|±HH:MM)` — format-VALIDATED, never
/// generated (the engine reads no wall clock; a malformed `now` is the
/// server's `bad_request`). Pure predicate, zero dependencies — a law
/// transcription like [`ErrorCode::recovery`], not business logic; the
/// dispatch strict-decode pass is its caller (W4-ACTOR / D4-SPLICE).
#[must_use]
pub fn now_is_rfc3339(s: &str) -> bool {
    let b = s.as_bytes();
    let digits = |r: std::ops::Range<usize>| b[r].iter().all(u8::is_ascii_digit);
    let num = |r: std::ops::Range<usize>| -> u32 {
        b[r].iter().fold(0, |a, d| a * 10 + u32::from(d - b'0'))
    };
    // date-time head: YYYY-MM-DDTHH:MM:SS (T or t per RFC 3339)
    if b.len() < 20
        || !digits(0..4)
        || b[4] != b'-'
        || !digits(5..7)
        || b[7] != b'-'
        || !digits(8..10)
        || !(b[10] == b'T' || b[10] == b't')
        || !digits(11..13)
        || b[13] != b':'
        || !digits(14..16)
        || b[16] != b':'
        || !digits(17..19)
    {
        return false;
    }
    let (year, month, day) = (num(0..4), num(5..7), num(8..10));
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let month_len = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    // sec 60 admitted: RFC 3339 leap second
    if day == 0 || day > month_len || num(11..13) > 23 || num(14..16) > 59 || num(17..19) > 60 {
        return false;
    }
    // optional fraction: '.' 1*DIGIT
    let mut i = 19;
    if b[i] == b'.' {
        let frac_start = i + 1;
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == frac_start {
            return false;
        }
    }
    // offset: Z/z or ±HH:MM
    match b.get(i) {
        Some(b'Z' | b'z') => i + 1 == b.len(),
        Some(b'+' | b'-') => {
            i + 6 == b.len()
                && digits(i + 1..i + 3)
                && b[i + 3] == b':'
                && digits(i + 4..i + 6)
                && num(i + 1..i + 3) <= 23
                && num(i + 4..i + 6) <= 59
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// v2 §4.4 the write grammar (batch splice)
// ---------------------------------------------------------------------------

/// One batch edit (v2 §4.4): a §2.1 target + one of exactly two edit shapes +
/// the optional node-grain CAS guard. All targets and guards resolve against
/// the PRE-batch state; targets must be disjoint (`bad_request{overlap}`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Edit {
    pub target: SecRef,
    pub edit: EditShape,
    /// Node-grain guard (§5.1): compared against blake3 of the target's full
    /// span bytes re-derived at execution time — mismatch is `cas_mismatch`
    /// → refresh one thing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub if_node_rev: Option<NodeRev>,
}

/// Exactly two edit shapes (v2 §4.4) — externally tagged on the wire
/// (`{"match":{…}}` / `{"put":{…}}`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditShape {
    /// Edit-exact: `old` must occur exactly once in the target's full span
    /// bytes; zero → `no_match`, two+ → `not_unique{matches}`. No regex, no
    /// fuzz; matched SERVER-side.
    Match { old: String, new: String },
    /// Whole-slot write at a [`PutAt`] position.
    Put { at: PutAt, text: String },
}

/// The three put positions (v2 §4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PutAt {
    /// Replace the full span, heading included.
    All,
    /// Replace the content span, heading preserved.
    Content,
    /// Insert `text` at the span-end byte — the append verb. RAW byte
    /// concatenation, NO synthesized separator (Edit-model exact): `text`
    /// that must begin a new line carries its own leading `\n`; against a
    /// terminator-less final line a separator-less `text` is the caller's to
    /// get right, and a result that loses containment refuses
    /// `would_corrupt`.
    End,
    /// Set a frontmatter key (create-or-replace) — the property UPSERT verb,
    /// valid ONLY on an `fm_key` target. `text` is the VALUE (not the whole
    /// line): the server composes `{key}: {value}` from the target key, so the
    /// `fm_key` is the single source of truth. Replaces the key's line when it
    /// exists; creates it (synthesizing the `---` frontmatter block when the
    /// file has none) when absent. The insertion offset is SERVER-derived from
    /// the document structure — no client byte offset (D-C1). A NON-`fm_key`
    /// target or a multi-line value is `bad_request`.
    Upsert,
}

/// A receipt address (v2 §6.1): ordinary markdown inside the hash domain —
/// any md path + block anchor. Per-request, never a wire requirement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiptAddr {
    pub path: Path,
    pub anchor: String,
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
    ///
    /// `contract` is the v3-amendment negotiation knob (`docs/wire-contract-v3-amendment.md`):
    /// an OPTIONAL client-DECLARED contract rev, absent or `"v2"` ⇒ the frozen
    /// v2 vocabulary (byte-for-byte), `"v3"` ⇒ the `fingerprint` vocabulary from
    /// the hello response onward. A DECLARED rev is not the §3.2-forbidden
    /// version sniffing (the client states its rev; the server never guesses).
    /// Absent ⇒ serialized away, so the v2 request stays byte-identical.
    ///
    /// `workspace` is the resident-engine handshake's workspace-target
    /// (`[[0002-resident-daemon]]` §4): the host path the client binds this
    /// connection to. The daemon resolves it (the ancestor walk), pins its
    /// storage drawer, warms its resident engine, and serves subsequent read ops
    /// from that binding — one round trip. Absent ⇒ a pure version handshake
    /// that binds nothing (the sidecar's per-process stdio hello never sends it,
    /// so the v2 request stays byte-identical). An OPTIONAL additive field on the
    /// frozen shape, exactly like `contract`.
    Hello {
        proto: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        client: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        contract: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        workspace: Option<String>,
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
    /// v2 §4.4 the ONLY write op, batch-only: the Edit-tool semantic model IS
    /// the wire write grammar (D-C1). No client span field exists anywhere in
    /// a request — the class of wrong-offset writes is unrepresentable.
    /// Guardless, actor-less, receipt-less frames are legal at the wire
    /// forever; whether a scope REQUIRES them is the Go ratchet (§5.3).
    /// `actor`/`now` are wire inputs, never ambient (§9): opaque string and
    /// RFC 3339 string, recorded into receipts and Deltas, never generated.
    Splice {
        path: Path,
        #[serde(skip_serializing_if = "Option::is_none")]
        actor: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        now: Option<String>,
        /// Receipts are per-request, never a wire requirement (§6.1): when
        /// named, the receipt append commits in the SAME batch as the content
        /// edit — one exchange, one reparse, ONE root advance (D-C3).
        #[serde(skip_serializing_if = "Option::is_none")]
        receipt: Option<ReceiptAddr>,
        /// World-grain guard, checked FIRST (§5.1): mismatch fails the whole
        /// batch `root_mismatch` → re-plan.
        #[serde(skip_serializing_if = "Option::is_none")]
        if_root: Option<Root>,
        /// §4.4 batch law: everything except disk — same response shape,
        /// `root_after:null`, no receipt written.
        #[serde(skip_serializing_if = "Option::is_none")]
        dry: Option<bool>,
        edits: Vec<Edit>,
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
    /// v2 §4.6 the corpus fact op (the 188-call read-as-oracle pattern, made
    /// an op): the outgoing edge map, `resolvedLinks`/`unresolvedLinks`
    /// shape. `path` absent → whole-corpus edge map. Corpus-wide ⇒ the
    /// response carries the §10.1 staleness triple; `require_root` is the
    /// opt-in strictness knob → `stale_view` refusal (§10.2), retry class.
    Links {
        #[serde(skip_serializing_if = "Option::is_none")]
        path: Option<Path>,
        #[serde(skip_serializing_if = "Option::is_none")]
        require_root: Option<Root>,
    },
    /// v2 §4.7 the push path, live at T5-SUB: `{"op":"sub","from_seq":N}` →
    /// ok (the ack reuses the `{root, seq}` body — the subscription's anchor
    /// tense; advisor-ruled, no frozen frame prints one), then Notification
    /// frames each carrying one Delta batch — the §7.1 noun, transport only
    /// (A6). `from_seq` catchup is valid only within one epoch (§7.1 late
    /// law); outside the retained history → `root_unknown` → diff-by-root.
    Sub { from_seq: u64 },
    /// V2 §Q2 the view-organ **path forwarder** — resolve `cwd` → workspace,
    /// publish `view.duckdb` (the daemon is the sole builder), and return the
    /// stamped filesystem PATH plus a pre-open freshness hint. It marshals
    /// **no rows**, executes no query, maps no result-set errors — a
    /// row-returning `sql` op is a brand-new tabular surface, explicitly
    /// round-2 and OUT of scope. `fresh:true` asks the daemon for a bounded
    /// rebuild (§Q3). Served by the resident daemon only; a per-process sidecar
    /// (which cannot publish — C2 forbids `sidecar`→`view`) answers
    /// `daemon_only`.
    ViewPath {
        /// The consumer's working directory — the daemon resolves it to a
        /// workspace (ancestor walk) and its drawer. A raw HOST path
        /// (absolute), NOT a workspace-relative wire [`Path`], so it carries no
        /// path-law and is a plain string.
        cwd: String,
        /// `true` ⇒ the bounded `--fresh` rebuild (§Q3); absent/`false` ⇒ serve
        /// the published (or first-built) view.
        #[serde(skip_serializing_if = "Option::is_none")]
        fresh: Option<bool>,
    },
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
    ///
    /// `storage` is the pinned storage drawer (`[[0002-resident-daemon]]` §4
    /// storage pin): the cache drawer directory the daemon pinned for the
    /// hello'd workspace, via the canonicalize → deny-ceiling → sentinel path.
    /// Absent on a workspace-less handshake (nothing to pin) and on the sidecar
    /// (which opens its drawer client-side). An OPTIONAL additive field on the
    /// frozen shape.
    Hello {
        proto: u32,
        server: String,
        caps: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        root: Option<Root>,
        #[serde(skip_serializing_if = "Option::is_none")]
        storage: Option<String>,
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
    /// v2 §4.4: what the write ARMED — target identities, rev transitions,
    /// spans after, the receipt fact, the root transition — never delivery
    /// claims (A7). ONE response shape for every batch, dry included.
    Splice {
        armed: Armed,
        /// Present iff the request named a receipt (§6.1) and the batch hit
        /// disk (a dry run writes none).
        #[serde(skip_serializing_if = "Option::is_none")]
        receipt: Option<ReceiptFact>,
        root_before: Root,
        /// ALWAYS serialized — `null` on a dry run (§4.4 worked dry frame),
        /// the one place absence-vs-null is contractual on this shape.
        root_after: Option<Root>,
        #[serde(skip_serializing_if = "Option::is_none")]
        seq: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        dry: Option<bool>,
        /// The rules-as-data surface (§11): shape present from birth, empty
        /// until rung 6 — [`Verdict`] is uninhabited until P6-VERDICTS, so
        /// the type ADMITS only `[]` today and the shape never changes.
        verdicts: Vec<Verdict>,
    },
    /// v2 §4.7: the current root at world grain + `seq`, the monotone
    /// per-workspace batch counter (per-daemon-epoch — a restart resets it;
    /// cross-epoch catchup is diff-by-root, §7.1 laws).
    Root { root: Root, seq: u64 },
    /// v2 §4.7/§7.3 replay: the byte-identical Delta objects that were (or
    /// would have been) emitted as live notifications between the two roots —
    /// each batch IS a notification frame body ([`DeltaFrame`]), so catchup
    /// consumers and live subscribers parse one shape. There is no second
    /// diff dialect.
    Diff { batches: Vec<DeltaFrame> },
    /// v2 §4.6: the outgoing edge map under the §10.1 staleness triple —
    /// `as_of_root` (the root the answer was computed at), `live_root` (the
    /// root now), `changes_seq` (the Delta counter at `as_of_root`, §7.1
    /// per-daemon-epoch semantics). **No lag bounds are promised, ever**
    /// (§10.1 honest-tense law): `as_of_root ≠ live_root` is a legal frame,
    /// never an error. `files` keys are corpus paths; see [`FileLinks`].
    Links {
        as_of_root: Root,
        live_root: Root,
        changes_seq: u64,
        files: BTreeMap<String, FileLinks>,
    },
    /// V2 §Q2 the `view_path` reply: a stamped **PATH** plus a **pre-open**
    /// freshness hint — never rows. `path` is authoritative; the fingerprints,
    /// `state`, and `live_source` are a PRE-OPEN hint the consumer discards once
    /// it opens the file and re-reads the `_meridian_view` stamp (a concurrent
    /// rebuild's rename can stale the reply's fingerprints — §Q3). The
    /// fingerprint slots carry the v2 `root` vocabulary (`as_of_root`/`live_root`)
    /// so the v3 projection re-keys them to `as_of_fingerprint`/`live_fingerprint`
    /// through the ONE existing rename table (`wire-serve::rev`), never a second
    /// dialect.
    ///
    /// Placed last in this untagged enum: its shape is unique (`path` +
    /// `state` + `live_source` + `refresh_in_progress`, and NO `files`), so no
    /// earlier variant captures a `view_path` frame and it captures none of
    /// theirs.
    ViewPath {
        /// The stamped `view.duckdb` filesystem path — **authoritative**, the
        /// one field the consumer trusts (it opens THIS inode).
        path: String,
        /// PRE-OPEN hint: the fingerprint the published file was built at (the
        /// daemon's `at_fingerprint` at publish). v2 `root` vocabulary; v3 →
        /// `as_of_fingerprint`. Non-authoritative — re-read the opened file's
        /// `_meridian_view` stamp for the real `as_of`.
        as_of_root: Root,
        /// PRE-OPEN hint: the daemon's live fingerprint sample (its warm
        /// `at_fingerprint`, a disk fold that may lag). v2 `root` vocabulary;
        /// v3 → `live_fingerprint`.
        live_root: Root,
        /// The per-daemon-epoch delta counter at the sample (§7.1); `0` until
        /// the delta ring lands (the resident daemon holds none in round-1,
        /// mirroring `Root`/`Links` which already report `0`).
        changes_seq: u64,
        /// The daemon's pre-open freshness ASSESSMENT (`as_of` vs `live`) — a
        /// hint label, never a verdict (`stale` stays null).
        state: ViewState,
        /// Provenance of `live_root`: `watch` (the daemon's warm hint) or `none`
        /// (no sample). NEVER `fold` on a pre-open hint — a `fold` verdict comes
        /// only from a consumer's POST-result sample (§Q3 C3).
        live_source: ViewLiveSource,
        /// **ALWAYS null** on this pre-open hint (B5+C3): a hint is never a
        /// freshness verdict. The consumer folds `live` AFTER its own result to
        /// reach `true`/`false`. Serialized as `null`, never omitted.
        stale: Option<bool>,
        /// OD7 advisory telemetry (daemon memory): a rebuild is in flight.
        /// Round-1 rebuilds are SYNCHRONOUS (done before this reply), so it is
        /// always `false`; the async executor is round-2.
        refresh_in_progress: bool,
        /// OD7 advisory telemetry (daemon memory): the last rebuild failure, if
        /// any — it explains WHY a view is stale, never gates freshness.
        #[serde(skip_serializing_if = "Option::is_none")]
        last_error: Option<RefreshError>,
    },
}

/// The daemon's pre-open freshness assessment for a `view_path` reply (§Q3
/// state machine, querier vantage). A HINT label on the reply — decoupled from
/// the null `stale` verdict, which only a consumer's post-result fold can set.
/// Round-1 emits exactly these three; the wider state machine (`REBUILDING`,
/// `NO_VIEW`, `UNKNOWN`) is a query-frame / error concern, never a `view_path`
/// success body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ViewState {
    /// `as_of == live` at the daemon's sample — fresh AT that instant, never
    /// timeless (§Q3).
    FreshAtSample,
    /// `as_of != live` — a legal frame, never an error; both fingerprints ride
    /// the reply.
    Stale,
    /// A bounded `--fresh` rebuild could not reach `as_of == live` within its
    /// one-retry bound (§Q3); a first-class sibling of `STALE`, never a loop,
    /// never a fresh lie.
    Raced,
}

/// The provenance of a `live` fingerprint value (§Q3 C3, source-labeled). The
/// `view_path` pre-open hint only ever uses `watch`/`none`; `fold` labels a
/// consumer's post-result disk fold on the delivered-query surface. Mirrors the
/// schema `CHECK (live_source IN ('fold','watch','none'))`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewLiveSource {
    /// A real post-result `fs::domain_snapshot` fold — a VERDICT source.
    Fold,
    /// The daemon's warm `at_fingerprint` hint (may lag disk) — never a verdict.
    Watch,
    /// No liveness sampled — never a verdict.
    None,
}

/// OD7 refresh-failure telemetry (daemon memory, never the immutable stamp): a
/// rebuild that failed. Advisory only — it explains why a view is stale and
/// never gates a freshness verdict. `fingerprint_attempted` is vocabulary-neutral
/// (like `expected`/`actual`), so the v3 projection leaves it untouched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefreshError {
    pub code: RefreshErrorCode,
    /// Unix seconds at which the failure was recorded.
    pub unix: u64,
    /// The fingerprint the failed rebuild targeted, if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fingerprint_attempted: Option<Root>,
    /// A human-readable cause (never machine-dispatched — `code` is the class).
    pub message: String,
}

/// The closed refresh-failure class (OD7). Flat lowercase `snake_case` on the
/// wire. Round-1 populates what the synchronous rebuild can distinguish; the
/// richer executor taxonomy (backoff, retry) lands with the round-2 async path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefreshErrorCode {
    /// The corpus could not be parsed into the projection source.
    ParseError,
    /// The drawer filesystem is out of space.
    DiskFull,
    /// The build ran out of memory.
    Oom,
    /// The build exceeded its time bound.
    Timeout,
    /// Any other I/O failure (temp create, `chmod`, `fsync`, rename, WAL).
    Io,
}

/// One file's outgoing edges (v2 §4.6, the app's `resolvedLinks`/
/// `unresolvedLinks` shape): per-edge counts, dangling refs first-class.
/// `resolved` keys are the destination corpus paths; `unresolved` keys are
/// the raw linkpaths as written (no vault file to name). Both maps always
/// serialize — a link-less file is `{}`/`{}`, never absent keys.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FileLinks {
    pub resolved: BTreeMap<String, u64>,
    pub unresolved: BTreeMap<String, u64>,
}

/// The armed-fact set for one batch (v2 §4.4): the normative receipt content
/// is exactly this, rendered (§6.4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Armed {
    pub path: Path,
    /// The whole-file rev AFTER the batch commits — same family/width as
    /// [`DeltaFile::file_rev_after`] and a subsequent `toc`'s `file_rev`, so a
    /// consumer learns the new file rev WITHOUT a follow-up `toc`. A latency
    /// fact only; correctness stays the fingerprint/`root_after` world grain.
    /// Absent on a dry run — nothing was written, so the post-write rev does
    /// not exist yet (mirrors `root_after`'s dry-null at file grain).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_rev_after: Option<NodeRev>,
    pub edits: Vec<ArmedEdit>,
}

/// One armed edit: target identity echoed in THE grammar (§2.1), rev
/// transition, span after.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArmedEdit {
    pub target: SecRef,
    pub node_rev_before: NodeRev,
    pub node_rev_after: NodeRev,
    pub span_after: Span,
}

/// The receipt fact armed with the batch (v2 §4.4/§6.3): address + the
/// receipt block's own computed node facts. Receipts carry `root_before`
/// only — a receipt cannot contain the root it produces (§6.2, honest limit).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReceiptFact {
    pub path: Path,
    pub anchor: String,
    pub node_rev: NodeRev,
    pub span_after: Span,
}

/// A rules-as-data verdict (v2 §11.1): a typed FINDING from a loaded rule pack,
/// never a decision (T1 — whether an `error` blocks is Go's action mapping, not
/// engine behavior). The shape rode every splice response as `[]` from birth
/// (D4-SPLICE); P6-VERDICTS inhabits it without changing that shape. The field
/// set is `crates/policy`'s `Violation` verbatim, projected into THE grammar
/// (§2.1): `hpath` segments carry `{h, n?}`, not bare strings. Worked (§11.1):
/// `{rule:"blurb-required", severity:"warn", path:"notes/plan.md",
/// hpath:[{"h":"Goals"}], span:[20,150], node_rev:"5a8faa717fbcdb04",
/// message:"section has no blurb line"}`. `budget_exceeded` is a finding in this
/// array (§8), never a wire error.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Verdict {
    pub rule: String,
    pub severity: Severity,
    pub path: Path,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hpath: Option<Vec<HpathSeg>>,
    pub span: Span,
    pub node_rev: NodeRev,
    pub message: String,
}

/// Verdict severity (v2 §11.1): descriptive policy data — how bad, per the pack's
/// convention — never what to DO about it (Go's action mapping). Serialized flat
/// lowercase (`"error"`/`"warn"`/`"info"`). Independent of `policy::Severity` by
/// construction: the sidecar projects one to the other, so no wire→policy edge
/// exists (the fence holds at the type level, not just cargo tree).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warn,
    Info,
}

// ---------------------------------------------------------------------------
// v2 §7 the Delta noun — the fifth noun, born frozen (A6)
// ---------------------------------------------------------------------------

/// One live notification frame body: `{"delta":{…}}` (v2 §7.1 worked frames —
/// events carry no `id`, §3.1 classification). `diff.batches` elements are
/// exactly this shape, byte-identical to the live emission (§7.3 replay ≡
/// live).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeltaFrame {
    pub delta: Delta,
}

/// The fifth wire noun (v2 §7.1, frozen at contract birth): one Delta = one
/// batch = one root advance. `seq` is the monotone per-workspace batch
/// counter, **per-daemon-epoch** — a daemon restart resets it (no counter
/// survives on disk, §14), so `from_seq` catchup is valid only within one
/// epoch and cross-epoch catchup is diff-by-root (§4.7), the root being the
/// only restart-durable handle. External changes carry `actor`/`now` ABSENT —
/// the engine never invents identity or time it wasn't given (A8/§9).
///
/// Node-grain at birth (decision 012, v2 §7.4): the grain is exactly
/// [`DeltaNode`]. Key-grain arrives later, if ever, ONLY via the additive
/// amendment path named in the contract prose — no such code path exists
/// here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Delta {
    pub seq: u64,
    pub root_before: Root,
    pub root_after: Root,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub now: Option<String>,
    pub files: Vec<DeltaFile>,
}

/// One changed file in a Delta (v2 §7.1): change class, rev transition
/// (`file_rev_before` absent on `created`, `file_rev_after` absent on
/// `deleted`), node-grain entries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeltaFile {
    pub path: Path,
    pub change: FileChange,
    /// `renamed` carries the origin path (v2 §7.1 law).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_path: Option<Path>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_rev_before: Option<NodeRev>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_rev_after: Option<NodeRev>,
    pub nodes: Vec<DeltaNode>,
}

/// v2 §7.1: `change ∈ {created, modified, deleted, renamed}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileChange {
    Created,
    Modified,
    Deleted,
    Renamed,
}

/// One node-grain change entry (v2 §7.1/§7.2): identity echoed in THE §2.1
/// grammar (flattened [`SecRef`] — same vocabulary as toc rows and armed
/// facts: one projection, three tenses), rev transition, span after. Entries
/// name the DEEPEST section containing each changed byte range — ancestor
/// revs change implicitly and are re-readable via `toc`, never duplicated
/// into the delta.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeltaNode {
    #[serde(flatten)]
    pub target: SecRef,
    pub change: NodeChange,
    /// Absent on `added` (v2 §7.1 worked anchor entry).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_rev_before: Option<NodeRev>,
    /// Absent on `removed` (no post-state exists to hash).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_rev_after: Option<NodeRev>,
    /// Absent on `removed` (no post-state span exists).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span_after: Option<Span>,
}

/// v2 §7.1: node `change ∈ {added, edited, removed}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeChange {
    Added,
    Edited,
    Removed,
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
/// `daemon_only` (rule packs, rung 6) lands with P6-VERDICTS — the last v2 code;
/// `stale_view` joined with `links` (Q5-LINKS). v1 `not_found` is RETIRED (§18
/// row 6, split `file_not_found`/`ref_not_found`) — its string no longer parses,
/// pinned by the retirement deviation fixture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    BadFrame,
    BadRequest,
    UnknownOp,
    UnsupportedProto,
    BadPath,
    /// v2 §8/§4.4: guard-passed zero occurrences of `old` in the target —
    /// provably your typo (the diagnosis `--if` buys).
    NoMatch,
    /// v2 §8/§4.4: `old` occurs 2+ times. Extras: `matches` (the count) —
    /// add context bytes to `old`.
    NotUnique,
    /// v2 §8/§4.4: the post-apply reparse would lose containment. Extras:
    /// `lost` (the hpaths that would vanish).
    WouldCorrupt,
    /// v2 §8: the world outside the workspace — the file is gone. Half of the
    /// v1 `not_found` retirement (§18 row 6); echoes `path`.
    FileNotFound,
    /// v2 §8: I/O failure with its `cause` — the other retirement half.
    IoError,
    InvalidUtf8,
    /// v2 §8/§11.3: the env code that names the engine's own deployment — a
    /// corpus-class rule pack (its WHEN needs the resident corpus name index,
    /// e.g. `link_resolves` §11.2) loaded against a sidecar-mode engine that has
    /// no resident index cannot run, so it is refused LOUD at admission
    /// (`BudgetClass::Corpus` law). A single-file op never raises it (every §4
    /// op is served from disk bytes alone, §10.3).
    DaemonOnly,
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
    /// v2 §8: transient lock contention — same request may succeed.
    LockTimeout,
    /// v2 §10.2: a `require_root` demand the current world does not meet —
    /// retry class (retryable, never silent). Extras: `required` (the
    /// demanded root) + `as_of_root`/`live_root` (the world as sampled).
    StaleView,
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
            | ErrorCode::NoMatch
            | ErrorCode::NotUnique
            | ErrorCode::WouldCorrupt
            | ErrorCode::AmbiguousRef => Recovery::Fix,
            ErrorCode::FileNotFound
            | ErrorCode::IoError
            | ErrorCode::InvalidUtf8
            | ErrorCode::DaemonOnly => Recovery::Env,
            ErrorCode::CasMismatch | ErrorCode::RefNotFound => Recovery::Refresh,
            ErrorCode::LockTimeout | ErrorCode::StaleView => Recovery::Retry,
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
    /// `bad_path` / `file_not_found`: the offending/requested path, echoed.
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
    /// `stale_view` (§10.2): the root the request demanded via `require_root`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<Root>,
    /// `stale_view` (§10.2): the root the answer would have been computed at.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as_of_root: Option<Root>,
    /// `stale_view` (§10.2): the root now, as sampled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub live_root: Option<Root>,
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
    /// `no_match` (0) / `not_unique` (2+): the occurrence count of `old` in
    /// the target's full span bytes (v2 §5.2 worked frames).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matches: Option<u32>,
    /// `would_corrupt`: the hpaths the post-apply parse would lose (v2 §4.4
    /// batch laws) — identities in THE grammar's segments.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lost: Option<Vec<Vec<HpathSeg>>>,
    /// `io_error`: the underlying cause, carried (v2 §8).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    /// `bad_request` on non-disjoint batch targets (v2 §4.4): the offending
    /// targets echoed in the §2.1 grammar (a ref-carrying surface).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overlap: Option<Vec<SecRef>>,
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
            required: None,
            as_of_root: None,
            live_root: None,
            stage: None,
            dest: None,
            candidates: None,
            unknown_kinds: None,
            id_raw: None,
            matches: None,
            lost: None,
            cause: None,
            overlap: None,
        }
    }
}
