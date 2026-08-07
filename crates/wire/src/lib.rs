//! The frozen wire vocabulary: `path/span/node_rev/root` + address grammar
//! (`SecRef`/`HpathSeg`) + op/request/response/error types (serde-only, zero
//! I/O) — the only Go-visible surface.
//!
//! Owns everything that crosses the process boundary, exactly as the frozen
//! contract states it (`docs/wire-contract.md` is normative; this crate
//! transcribes it and never restates its rules). Never does framing,
//! transport, I/O, or business logic; dependencies are serde only, so any
//! client can consume it without dragging a runtime.
//!
//! `model`'s types carry no serde derives; only this crate's types serialize.
//! Bridge behavior lives in the `wire-map` projection seam and the
//! `wire-serve` serve choke-point (`docs/laws.md` Law 3).
//!
//! Contract laws the types alone cannot enforce:
//! - Server side (v2 §3.2): unknown request fields MUST be rejected with
//!   `bad_request` — serde's default ignores them, and `deny_unknown_fields`
//!   does not compose with `flatten`; dispatch owes a strict decode pass.
//! - Client side (v2 §3.2): tolerate unknown error codes, dispatching on the
//!   closed `recovery` class alone (v2 §8); ignore unknown open-kind strings.

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
/// sub-laws). UTF-8 bytes, never chars, never UTF-16. Serializes as the
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
/// v2 §2.1). Per-segment byte-equality against the real containment tree.
///
/// The object form `{"h":…}` / `{"h":…,"n":…}` is the ONLY form, both
/// directions (v2 §2.1); the v1 bare string is refused loud. This refusal
/// text is single-sourced and shared by every door that can meet it (this
/// type's `Deserialize`, `wire-serve`'s decode); each door appends the
/// offending value.
pub const HPATH_SEG_V1_REFUSAL: &str =
    "hpath segment must be the object form `{h, n?}`; the v1 bare string is refused (v2 §2.1)";

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
        // The string arm survives only to name the refusal — without it a
        // retired v1 spelling gets serde's generic "invalid type".
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
        match Repr::deserialize(deserializer)? {
            Repr::Str(h) => Err(serde::de::Error::custom(format!(
                "{HPATH_SEG_V1_REFUSAL}: `{h}`"
            ))),
            Repr::Seg { h, n } => Ok(HpathSeg { h, n }),
        }
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

/// The composed-read section selector — the read face's addressing grammar,
/// tagged, with one arm per plane.
///
/// The heading arm carries segments — the same §2.1 grammar `put` takes in
/// `target.hpath` and [`ReadRow::hpath`] publishes — so a read-then-write loop
/// closes with no join, no split, and no projection to invert. The dewey and
/// anchor arms carry opaque ids, which are not addresses.
///
/// Conversion from a human string happens once, at an ingress door, through
/// [`ReadSel::parse`] — never inward of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ReadSel {
    /// `{"hpath":[{"h":"Goals"},{"h":"Q3"}]}` — the heading plane, per-segment
    /// byte-equality on RAW heading text. A segment's `n` pins the occurrence
    /// among same-text siblings; absent, a unique match resolves and matching
    /// more than one node refuses `ambiguous_ref` naming the candidates
    /// (wire-contract A.3 — the strict plane never silently picks).
    Hpath { hpath: Vec<HpathSeg> },
    /// `{"n":"1.2.1"}` — the dewey ordinal the read face mints per heading
    /// row. Positional and NOT round-trippable across an edit; it addresses a
    /// row of a table the caller is holding, never a document address.
    Dewey { n: String },
    /// `{"anchor":"r-000042"}` — the `^id` block plane, id WITHOUT the `^`
    /// marker (charset `[A-Za-z0-9-]+`, v2 §2.4).
    Anchor { anchor: String },
}

impl ReadSel {
    /// The one human-string→selector door: a CLI `--section`, a `#Fragment`
    /// on a ref, a `pin` spec. Three disjoint spellings, decided in order:
    ///
    /// - `^id` → [`ReadSel::Anchor`].
    /// - digits and dots only (`1`, `1.2.1`) → [`ReadSel::Dewey`].
    /// - anything else → [`ReadSel::Hpath`], `/`-split into raw heading texts.
    ///
    /// The order is the disambiguation: a heading literally named `1.2` is
    /// unaddressable from a human string here, and the structured arm is what
    /// a caller meaning the heading uses.
    ///
    /// Heading text is taken verbatim — no sanitize, no `#n` sub-grammar, since
    /// `#` inside a human string is itself a live ingress delimiter (wikilinks,
    /// `path#frag`), so the occurrence index rides the structured form only.
    #[must_use]
    pub fn parse(s: &str) -> Self {
        if let Some(id) = s.strip_prefix('^') {
            return ReadSel::Anchor {
                anchor: id.to_owned(),
            };
        }
        if !s.is_empty() && s.chars().all(|c| c.is_ascii_digit() || c == '.') {
            return ReadSel::Dewey { n: s.to_owned() };
        }
        ReadSel::Hpath {
            hpath: s
                .split('/')
                .map(|h| HpathSeg {
                    h: h.to_owned(),
                    n: None,
                })
                .collect(),
        }
    }

    /// The caller's own spelling, for a refusal message to name back. Display
    /// only — nothing addresses anything with this string, and no door parses
    /// it back.
    #[must_use]
    pub fn display(&self) -> String {
        match self {
            ReadSel::Hpath { hpath } => hpath
                .iter()
                .map(|s| match s.n {
                    Some(n) => format!("{}#{n}", s.h),
                    None => s.h.clone(),
                })
                .collect::<Vec<_>>()
                .join("/"),
            ReadSel::Dewey { n } => n.clone(),
            ReadSel::Anchor { anchor } => format!("^{anchor}"),
        }
    }
}

// ---------------------------------------------------------------------------
// v2 §9 actor and now — wire inputs, never ambient
// ---------------------------------------------------------------------------

/// The §9 `now` format law, transcribed: RFC 3339 date-time —
/// `YYYY-MM-DDTHH:MM:SS[.frac](Z|±HH:MM)` — format-validated, never generated
/// (the engine reads no wall clock; a malformed `now` is the server's
/// `bad_request`). Pure predicate, zero dependencies; the dispatch
/// strict-decode pass is its caller.
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
/// the PRE-batch state; the edits' replaced regions must be pairwise disjoint
/// (`bad_request{overlap}` — §4.4 region grain).
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
    /// Insert `text` at the span-end byte — the append verb. Raw byte
    /// concatenation with no synthesized separator: `text` that must begin a
    /// new line carries its own leading `\n`, and a result that loses
    /// containment refuses `would_corrupt`.
    End,
    /// Set a frontmatter key (create-or-replace) — the property upsert verb,
    /// valid only on an `fm_key` target. `text` is the value, not the whole
    /// line: the server composes `{key}: {value}` from the target key, so the
    /// `fm_key` is the single source of truth. Replaces the key's line when it
    /// exists; creates it (synthesizing the `---` frontmatter block when the
    /// file has none) when absent. The insertion offset is server-derived from
    /// the document structure — no client byte offset (D-C1). A non-`fm_key`
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

/// The pin riding a splice: a splice-sibling optional field, never its own op.
/// The splice's `path` is the pinning page (the page whose `meridian-lock`
/// block records the claim); these two fields say what it pins.
///
/// No actor field: a pin's mint identity is the splice's own `actor`, and a
/// caller-settable one here would let a caller forge a pin as somebody else.
///
/// `target` is carried verbatim into the lock's `ref` and `objects:` key —
/// this type parses no address, so a later `root:` prefix rides through
/// untouched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinSpec {
    /// The page holding the pinned content (workspace-relative).
    pub target: Path,
    /// The selector inside `target`, tagged. The human spelling lives in the
    /// CLI coat (`mrd pin --section`), which converts through
    /// [`ReadSel::parse`] at its own door; that coat splits on `/`, so a
    /// `/`-bearing heading is pinnable through this field and not through the
    /// CLI sugar.
    pub selector: ReadSel,
    /// `--vibe`: additionally write the target's blob into git's object store
    /// (`git hash-object -w`), so the pin is retrievable before any commit
    /// references it. Absent/`false` computes the oid read-only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vibe: Option<bool>,
}

/// What a pin actually minted — the response half of [`PinSpec`], present
/// only when the request carried one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinFact {
    /// The pinned page, as given.
    pub target: Path,
    /// The canonical selector the engine resolved (never the caller's spelling,
    /// never a dewey ordinal) — the same key the read-mint gate looked up.
    pub selector: ReadSel,
    /// The minted `fp1.…` CID-token over the selector's own span.
    pub fingerprint: String,
    /// The lock's `objects[]` blob oid for the target file; absent when git
    /// could not answer (honest degradation — never a fabricated sha).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blob: Option<String>,
    /// The target's stable block id (slug-derived) — the handle a claim
    /// link decorates and a later rename-heal relocates by.
    pub anchor: String,
    /// `true` when this pin wrote `anchor` into the target (a re-pin reuses the
    /// id and promotes nothing).
    pub promoted: bool,
}

/// One v3 plan-level splice edit (`splice.plan_edits`): the put-plan
/// vocabulary, externally tagged.
///
/// Addresses are segments ([`HpathSeg`]) — the same §2.1 grammar
/// [`SecRef::Hpath`] takes and [`ReadRow::hpath`] publishes — so an address a
/// read publishes is an address a plan edit accepts, and two headings that
/// differ cannot collide.
///
/// The engine lowers each shape to native [`Edit`]s at the splice intake
/// (`wire-serve::plan`). v3-only at decode: a v2 session's `plan_edits` hits
/// the frozen unknown-field wall.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanEdit {
    /// Append to a section's content end — `ensureTrailingNL` + the
    /// leading-`\n` discipline applied engine-side. `rev` is the node-grain
    /// guard token, threaded to `if_node_rev`: an append changes existing
    /// content and is guarded like every other change.
    Append {
        hpath: Vec<HpathSeg>,
        body: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rev: Option<String>,
    },
    /// Anchored replace; `all: true` replaces every occurrence (the host's
    /// read-modify-write moved engine-side). `rev` is the v2-domain node rev
    /// (blake3) threaded to `if_node_rev`; empty/absent = the relaxed write.
    Match {
        hpath: Vec<HpathSeg>,
        old: String,
        new: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        all: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rev: Option<String>,
    },
    /// Whole-section rewrite (destructive — requires `rev`).
    ReplaceSection {
        hpath: Vec<HpathSeg>,
        body: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rev: Option<String>,
    },
    /// Create a new section — parent-append placement. An empty
    /// `parent_hpath` (top-level create) refuses. `rev` is the PARENT
    /// section's node-grain token, threaded to the lowered append's
    /// `if_node_rev`: the birth changes the parent's bytes, so the parent's
    /// rev is the honest grain (Law A-1 at the create door).
    Create {
        parent_hpath: Vec<HpathSeg>,
        title: String,
        body: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rev: Option<String>,
    },
    /// Frontmatter set — value-span replace / insert-after-last-key /
    /// conditional quote (`yaml_safe_value`). Not native `at:upsert`: upsert
    /// inserts absent keys at first-key position, this inserts after the last
    /// key — divergent bytes.
    SetProperty {
        key: String,
        value: String,
        /// The file-grain guard: frontmatter semantics are file-scoped, so a
        /// key-line rev would guard the wrong grain; this carries the
        /// document's `file_rev`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rev: Option<String>,
    },
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
    /// `contract` is an optional client-declared contract rev: absent or
    /// `"v2"` ⇒ the frozen v2 vocabulary (byte-for-byte), `"v3"` ⇒ the
    /// `fingerprint` vocabulary from the hello response onward. Absent ⇒
    /// serialized away, so the v2 request stays byte-identical.
    ///
    /// `workspace` is the resident-daemon handshake's workspace-target: the
    /// host path the client binds this connection to. The daemon resolves it
    /// (the ancestor walk), pins its storage drawer, warms its resident
    /// engine, and serves subsequent ops from that binding. Absent ⇒ a pure
    /// version handshake that binds nothing. An optional additive field on the
    /// frozen shape, like `contract`.
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
    /// `kinds` is `bad_request{unknown_kinds}`, loud.
    Extract {
        path: Path,
        #[serde(skip_serializing_if = "Option::is_none")]
        kinds: Option<Vec<String>>,
    },
    /// v2 §4.5 the walk plane: best-effort app-compatible two-stage walk over
    /// the raw Obsidian linktext. Location facts only — the response type has
    /// no rev field to return. `from` is mandatory: resolution is
    /// source-relative. `content:true` additionally returns the fragment
    /// bytes — still no rev.
    Resolve {
        from: Path,
        #[serde(rename = "ref")]
        r#ref: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        content: Option<bool>,
    },
    /// v2 §4.4 the only write op under v2, batch-only (v3 adds the birth op
    /// [`Op::Create`]; `splice` stays the only op that edits an existing
    /// file): the Edit-tool semantic model is the wire write grammar. No
    /// client span field exists anywhere in a request, so wrong-offset writes
    /// are unrepresentable. Guardless, actor-less, receipt-less frames are
    /// legal at the wire forever; whether a scope requires them is the host's
    /// ratchet (§5.3). `actor`/`now` are wire inputs, never ambient (§9):
    /// opaque string and RFC 3339 string, recorded into receipts and Deltas,
    /// never generated.
    Splice {
        path: Path,
        #[serde(skip_serializing_if = "Option::is_none")]
        actor: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        now: Option<String>,
        /// Receipts are per-request, never a wire requirement (§6.1): when
        /// named, the receipt append commits in the same batch as the content
        /// edit — one exchange, one reparse, one root advance (D-C3).
        #[serde(skip_serializing_if = "Option::is_none")]
        receipt: Option<ReceiptAddr>,
        /// World-grain guard, checked first (§5.1): mismatch fails the whole
        /// batch `root_mismatch` → re-plan.
        #[serde(skip_serializing_if = "Option::is_none")]
        if_root: Option<Root>,
        /// §4.4 batch law: everything except disk — same response shape,
        /// `root_after:null`, no receipt written.
        #[serde(skip_serializing_if = "Option::is_none")]
        dry: Option<bool>,
        /// The sanctioned bypass (§11.1 pt 3): a `--force` write escapes an
        /// armed binding-break / block refusal, and the skip is journaled and
        /// rendered, never silent. Absent/`false` is an ordinary gated write.
        #[serde(skip_serializing_if = "Option::is_none")]
        force: Option<bool>,
        edits: Vec<Edit>,
        /// `splice.plan_edits` (v3-only at decode): the plan-level batch,
        /// mutually exclusive with `edits` — the engine lowers these to
        /// native edits at the splice intake. Empty = the native form;
        /// serialization skips it so the frozen v2 request bytes never change.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        plan_edits: Vec<PlanEdit>,
        /// `splice.pin` (v3-only at decode): the pin rides the write
        /// choke-point as a sibling field, so its lock write lands in the same
        /// `commit_batch` — one flock, one rename — instead of a second
        /// flocked call. A pin-only splice carries no `edits`. Serialization
        /// skips it, so the frozen v2 request bytes never change.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pin: Option<PinSpec>,
    },
    /// The birth op (v3-only at dispatch): births one file through the same
    /// guarded door every in-process caller uses
    /// (`wire_serve::write::create`) — path confinement → reserved-journal
    /// guard → world guard → the four birth guards → the gate seam → the
    /// `if_absent` CAS at the disk edge → root advance → birth Delta →
    /// journal row. This op forwards; it does not re-implement.
    ///
    /// `body` is the newborn's full bytes, byte-transparent — the engine mints
    /// no template here. An occupied path refuses `cas_mismatch` (recovery
    /// `refresh`) and nothing lands.
    ///
    /// No `force` field: the guarded door carries no forced-birth escape, so
    /// admitting the key would advertise a bypass that does not exist. A
    /// `force` on this op hits the strict field wall.
    Create {
        path: Path,
        /// The newborn's full bytes, verbatim (no template, no engine authoring).
        body: String,
        /// §9: recorded exactly as given into the birth Delta and journal row.
        #[serde(skip_serializing_if = "Option::is_none")]
        actor: Option<String>,
        /// §9: RFC 3339, format-validated never generated — the journal row's
        /// clock comes from the caller, so a malformed `now` is `bad_request`.
        #[serde(skip_serializing_if = "Option::is_none")]
        now: Option<String>,
        /// The §5.1 world-grain guard: mismatch refuses `root_mismatch` and
        /// nothing is born.
        #[serde(skip_serializing_if = "Option::is_none")]
        if_root: Option<Root>,
        /// Rehearsal: everything except disk — no file, no journal row, no root
        /// advance. A dry birth still refuses a would-be clobber.
        #[serde(skip_serializing_if = "Option::is_none")]
        dry: Option<bool>,
    },
    /// v2 §4.7 integrity read: the current workspace root cursor + `seq`.
    /// No parameters — the root is world-grain (the only root guard is
    /// `splice.if_root`, §5.1; the v1 scoped/`path` variant is gone with
    /// `guard`).
    Root,
    /// v2 §4.7 replay. The response carries Delta batches byte-identical to
    /// the live notification frames (§7.3).
    Diff { from_root: Root, to_root: Root },
    /// v2 §4.6 the corpus fact op: the outgoing edge map,
    /// `resolvedLinks`/`unresolvedLinks` shape. `path` absent → whole-corpus
    /// edge map. Corpus-wide ⇒ the response carries the §10.1 staleness
    /// triple; `require_root` is the opt-in strictness knob → `stale_view`
    /// refusal (§10.2), retry class.
    Links {
        #[serde(skip_serializing_if = "Option::is_none")]
        path: Option<Path>,
        #[serde(skip_serializing_if = "Option::is_none")]
        require_root: Option<Root>,
    },
    /// v2 §4.7 the push path: `{"op":"sub","from_seq":N}` → ok (the ack
    /// reuses the `{root, seq}` body), then Notification frames each carrying
    /// one Delta batch (§7.1). `from_seq` catchup is valid only within one
    /// epoch; outside the retained history → `root_unknown` → diff-by-root.
    Sub { from_seq: u64 },
    /// The COMPOSED read op (v3-only): addressing + content + render at ONE
    /// engine snapshot — one round trip replacing the `extract`→`cat`→render
    /// 3-hop split. Absent from the frozen v2 `caps`, so a v2 session
    /// answers `unknown_op`; the v3 hello projection advertises it.
    ///
    /// `sections` IS the mode: present → a sections read, absent → the toc
    /// read (an explicit `mode` on the wire is an unknown field the strict
    /// decode refuses). `frag` scopes to one section subtree; `display_path`
    /// is the caller's path spelling for the rendered header line (defaults
    /// to `path`) — the engine never invents host paths.
    ///
    /// `actor` is the §9 read-provenance slot: the daemon-derived actor
    /// stamped on the request — a wire input, never ambient, never
    /// MCP-caller-settable.
    Read {
        path: Path,
        /// The whole-call subtree scope, as segments: the section itself
        /// plus its descendants, matched per-segment.
        #[serde(skip_serializing_if = "Option::is_none")]
        frag: Option<Vec<HpathSeg>>,
        /// Document-absolute section selectors, each in the tagged read
        /// grammar ([`ReadSel`]).
        #[serde(skip_serializing_if = "Option::is_none")]
        sections: Option<Vec<ReadSel>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        display_path: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        actor: Option<String>,
    },
    /// The def-conformance verdict op (v3-only, like `read`): rebuild the
    /// candidate from put-plan-vocabulary `edits` over the current bytes at
    /// `path`, judge prev→candidate against the def layer, return the
    /// refuse/repairs/forced verdict. Never a write path: no flock, no CAS,
    /// no journal, no disk mutation.
    ///
    /// `target` is the caller's absolute path spelling (a raw host string, not
    /// a wire [`Path`]): it labels the refusal strings verbatim and anchors
    /// the def-layer discovery walk. `now` is the caller's clock
    /// (RFC3339); close-stamp repairs derive from it (§9: the engine mints no
    /// time). `actor` rides for verdict context, never authz.
    CheckWrite {
        path: Path,
        target: String,
        actor: String,
        now: String,
        edits: Vec<CheckWriteEdit>,
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
/// on headings only — in the §2.1 mint grammar (`HpathSeg`) —
/// `unterminated` present only when true, `info` per-kind.
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
    /// v3-additive: the dewey ordinal ("1.2.1") on heading nodes. Never
    /// emitted on a v2 session — the frozen v2 bytes carry no such key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n: Option<String>,
    /// v3-additive: the `strings.Fields` word count over the heading's
    /// subtree-inclusive content span.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub words: Option<u64>,
}

/// One `toc` row (v2 §4.1): the complete write kit for one node. Row shapes,
/// worked in the contract: frontmatter rows carry `keys`; heading rows carry
/// `level` + `hpath` + `content_span`; anchor-bearing block rows carry
/// `anchor` and echo their host block kind (the worked anchor row is
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
    /// Serves heading-preserving display; mints nothing (v2 §1 — one rev per
    /// node, over full span bytes).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_span: Option<Span>,
    pub node_rev: NodeRev,
    pub text_prefix_16b: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keys: Option<Vec<String>>,
}

/// One composed-read toc row (v3-only): the host-face addressing facts —
/// dewey ordinal `n`, heading `depth`, raw `title`, the `hpath` address,
/// `strings.Fields` `words` over the subtree-inclusive content span, and the
/// section CAS token `sec_rev` — plus the authz facts `span` and
/// `content_span`. With the [`ReadAnchor`] plane, the response answers
/// governing-section derivation by byte containment: keep every heading row
/// whose span contains an anchor's start byte.
///
/// One row shape, the heading: `depth >= 1` always; `content_span` present
/// when the section has content (heading-excluded, subtree-inclusive). The
/// `^id` anchor rows live in `anchors[]`, never in this array — a consumer
/// that iterates `toc` structurally cannot meet a second row class.
///
/// `hpath` stays root-prefix-learnable, and spans are intra-file byte
/// offsets, root-independent by construction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadRow {
    pub n: String,
    pub depth: u32,
    pub title: String,
    /// The address, as segments — the §2.1 grammar `put` takes in
    /// `target.hpath`, so the address this row publishes is one the write
    /// plane accepts unmodified; the read→put loop closes off this row.
    ///
    /// Per-segment `n` rides only where the raw text is ambiguous among its
    /// same-parent siblings. An unconditional `n` would keep silently
    /// resolving after a duplicate appeared; a minimal address refuses loud
    /// instead.
    pub hpath: Vec<HpathSeg>,
    pub words: u64,
    pub sec_rev: NodeRev,
    /// Full node span, heading-inclusive and subtree-inclusive — the
    /// containment fact an anchor's start byte is tested against.
    pub span: Span,
    /// The heading-excluded, subtree-inclusive content span. Absent on
    /// content-less headings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_span: Option<Span>,
}

/// One composed-read `^id` anchor: the block-anchor plane of the addressing
/// table, served in its own always-emitted array so no `toc` consumer can
/// receive a row class it does not expect.
///
/// Carries exactly what containment needs: the block id and the block-leaf
/// span. Spans are intra-file byte offsets, root-independent by
/// construction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadAnchor {
    /// The block id without the `^` marker.
    pub anchor: String,
    /// The block-leaf span.
    pub span: Span,
}

/// One composed-read resolved section (v3-only): the selector that hit, its
/// address + CAS token, the raw content — the verbatim bytes `sec_rev` was
/// minted over, so the row is self-verifying and a `put` built from it
/// round-trips (elision applies to `rendered_text` only) — and the word count
/// over that raw content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadSectionOut {
    /// The selector that hit, echoed in the caller's own tagged grammar — a
    /// caller pairing responses with requests compares structure to structure,
    /// never re-parsing a string to learn which plane it asked about.
    pub sel: ReadSel,
    /// The section's address as segments (see [`ReadRow::hpath`]). Empty on
    /// `^id` sections: their put grammar is `{"anchor":id}`, and the id rides
    /// `sel` un-sanitized, so there is no heading address to publish and none
    /// is invented.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hpath: Vec<HpathSeg>,
    pub sec_rev: NodeRev,
    pub words: u64,
    pub content: String,
}

/// One `check_write` plan edit (v3-only): the put-plan vocabulary the
/// daemon face speaks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckWriteEdit {
    pub op: String,
    /// The address, as segments — the same `{h, n?}` grammar `sec.hpath`
    /// takes and the read face publishes; the committer takes the same
    /// shape, so pre-flight and committer cannot be two answers to one
    /// question. The single-segment forms ride here too: `[{h:"^task1"}]`
    /// for a block, `[{h:"status"}]` for a frontmatter key.
    pub at: Vec<HpathSeg>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub find: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub rev: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub all: bool,
}

/// The `check_write` refusal: `class` picks the host's render template —
/// `rebuild` (the candidate could not be built) vs `verdict` (the severity
/// ladder refused). `code`/`message`/`remedy` ride verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckWriteRefuse {
    pub class: String,
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub remedy: String,
}

/// One close-stamp autofill: the host folds it into the same atomic write as
/// a system-authored property set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckWriteRepair {
    pub key: String,
    pub value: String,
}

/// The answering binary's build identity, carried by a v3 `hello` body.
///
/// An object rather than a bare string so a later fact joins it without
/// re-typing the slot. `build` is the commit sha baked at compile time (the
/// value `mrd --version` prints), or the literal `unknown` — which reaches the
/// wire verbatim: a present-but-unknown identity and an absent one are
/// different facts, and a client's mismatch policy needs both.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub build: String,
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
    /// (one charset, both planes — v2 §2.4).
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
/// typed clients match on what they asked for. Variant order is load-bearing
/// for deserialization: a shape-superset variant must precede its subset
/// (Toc before Nodes, Cat before Splice; Read first — `rendered_text` is
/// unique to it).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ResponseBody {
    /// The composed-read reply (v3-only): every fact at one engine snapshot —
    /// `file_rev` + ambient `root` (the atomicity witness), the host-face
    /// addressing table (`toc`, mode toc) or the selected sections
    /// (`sections`, mode sections; `truncated`+`notice` = the partial-read
    /// rule), the `anchors` plane, and `rendered_text`.
    Read {
        path: Path,
        file_rev: NodeRev,
        root: Root,
        words_total: u64,
        /// The heading plane, `frag`-scoped. Mode toc only.
        #[serde(skip_serializing_if = "Option::is_none")]
        toc: Option<Vec<ReadRow>>,
        /// The `^id` anchor plane, `frag`-scoped by the same byte containment
        /// the host applies. Always emitted — empty means "this scope has no
        /// addressable block anchor", never "ask again with a flag".
        /// `serde(default)` keeps decoding tolerant of older recorded frames;
        /// serialization is unconditional.
        #[serde(default)]
        anchors: Vec<ReadAnchor>,
        #[serde(skip_serializing_if = "Option::is_none")]
        sections: Option<Vec<ReadSectionOut>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        truncated: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        notice: Option<String>,
        rendered_text: String,
    },
    /// The `check_write` verdict (v3-only). `refuse` absent = the write may
    /// proceed; `repairs` are autofill edits the host folds into the same
    /// atomic write; `forced` echoes overridden warn rule-ids.
    /// `repairs`/`forced` always serialize so the body is never shapeless.
    CheckWrite {
        #[serde(skip_serializing_if = "Option::is_none")]
        refuse: Option<CheckWriteRefuse>,
        repairs: Vec<CheckWriteRepair>,
        forced: Vec<String>,
    },
    /// v2 §3.2: `proto` in effect, server name, the complete op-name set
    /// (`caps` includes dotted `op.field` strings for field-only amendments),
    /// optional first ambient `root`.
    ///
    /// `storage` is the pinned storage drawer for the hello'd workspace.
    /// Absent on a workspace-less handshake (nothing to pin). An optional
    /// additive field on the frozen shape.
    ///
    /// `workspace` is the canonical root that actually bound — it may differ
    /// from the string the caller declared, because canonicalization rewrites
    /// symlinks and on-disk case, so the caller learns the real root here
    /// rather than assuming its own spelling survived. Absent exactly when
    /// `storage` is.
    ///
    /// `identity` is the answering binary's build identity — v3-only, the one
    /// hello fact about the server process rather than the corpus or the
    /// binding (`proto` cannot separate two builds of one contract, which is
    /// what a deploy check must catch). Populated under a negotiated v3
    /// session only, so the frozen v2 hello body never grows a key.
    Hello {
        proto: u32,
        server: String,
        caps: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        root: Option<Root>,
        #[serde(skip_serializing_if = "Option::is_none")]
        storage: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        workspace: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        identity: Option<Identity>,
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
    /// v2 §4.5: location facts only — no rev field exists to return (D-C2, the
    /// mint partition as a type-level fact). `content` rides only when the
    /// request set `content:true`; still no rev.
    Resolve {
        dest: Path,
        span: Span,
        #[serde(skip_serializing_if = "Option::is_none")]
        content: Option<String>,
    },
    /// v2 §4.4: what the write armed — target identities, rev transitions,
    /// spans after, the receipt fact, the root transition — never delivery
    /// claims (A7). One response shape for every batch, dry included.
    Splice {
        armed: Armed,
        /// Present iff the request named a receipt (§6.1) and the batch hit
        /// disk (a dry run writes none).
        #[serde(skip_serializing_if = "Option::is_none")]
        receipt: Option<ReceiptFact>,
        root_before: Root,
        /// Always serialized — `null` on a dry run (§4.4 worked dry frame),
        /// the one place absence-vs-null is contractual on this shape.
        root_after: Option<Root>,
        #[serde(skip_serializing_if = "Option::is_none")]
        seq: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        dry: Option<bool>,
        /// The rules-as-data surface (§11): shape present from birth, empty
        /// until rule packs land.
        verdicts: Vec<Verdict>,
        /// What the request's `pin` minted. Absent unless the request carried
        /// a pin, so the frozen v2 response bytes are untouched. Boxed: the
        /// fact is wide next to every other response body, and the enum is
        /// passed by value on every reply.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pin: Option<Box<PinFact>>,
    },
    /// The birth reply (the `create` op, v3-only): what the birth landed —
    /// the born path, its whole-file rev, and the root transition. Never a
    /// delivery claim (A7), exactly like `splice`.
    ///
    /// Shape-unique in this untagged enum: `file_rev_after` appears on no
    /// other variant, and `armed` (which `Splice` requires) appears on none of
    /// this one — so neither body can capture the other's frame. Pinned by
    /// `crates/wire/tests/contract_v2.rs`.
    Create {
        path: Path,
        /// The born file's whole-file rev — computed from the body, so present
        /// on a dry run too (a fact about the spec, not about the disk).
        file_rev_after: NodeRev,
        root_before: Root,
        /// Always serialized — `null` on a dry run, the same absence-vs-null
        /// contract `splice` carries.
        root_after: Option<Root>,
        #[serde(skip_serializing_if = "Option::is_none")]
        seq: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        dry: Option<bool>,
        /// The §11 rules-as-data surface over the birth's after-state — the
        /// same shape `splice` carries, `[]` on an unarmed workspace.
        verdicts: Vec<Verdict>,
    },
    /// v2 §4.7: the current root at world grain + `seq`, the monotone
    /// per-workspace batch counter (per-daemon-epoch — a restart resets it;
    /// cross-epoch catchup is diff-by-root, §7.1 laws).
    Root { root: Root, seq: u64 },
    /// v2 §4.7/§7.3 replay: the byte-identical Delta objects that were (or
    /// would have been) emitted as live notifications between the two roots —
    /// each batch is a notification frame body ([`DeltaFrame`]), so catchup
    /// consumers and live subscribers parse one shape. There is no second
    /// diff dialect.
    Diff { batches: Vec<DeltaFrame> },
    /// v2 §4.6: the outgoing edge map under the §10.1 staleness triple —
    /// `as_of_root` (the root the answer was computed at), `live_root` (the
    /// root now), `changes_seq` (the Delta counter at `as_of_root`, §7.1
    /// per-daemon-epoch semantics). No lag bounds are promised (§10.1
    /// honest-tense law): `as_of_root ≠ live_root` is a legal frame, never an
    /// error. `files` keys are corpus paths; see [`FileLinks`].
    Links {
        as_of_root: Root,
        live_root: Root,
        changes_seq: u64,
        files: BTreeMap<String, FileLinks>,
    },
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
    /// v2 §4.6 — edges that resolved inside a mounted root, keyed by root
    /// name and then by the path inside that root. Two levels, never one
    /// joined `root:path` key: a joined string would collide with any ambient
    /// path that legitimately contains a colon. Omitted when empty, so a
    /// single-root corpus's bytes are unchanged.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub resolved_rooted: BTreeMap<String, BTreeMap<String, u64>>,
    /// v2 §4.6 — edges the address plane refused, keyed by the linkpath as
    /// written. Separate from `unresolved`: a dangling link is an ordinary
    /// authoring state; a refused edge means the author believed a mount
    /// relationship that does not hold, or wrote something that is not an
    /// address. Omitted when empty.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub refused: BTreeMap<String, RefusedEdge>,
}

/// One refused edge on the §4.6 map: the colour plane's own verdict,
/// rendered. The `color`/`reason`/`detail` triple is the same vocabulary the
/// walk plane renders (`view::walk::color_tone` / `color_reason` /
/// `color_detail`) — a second spelling here is how a board and a walk start
/// disagreeing about one address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefusedEdge {
    /// `grey` or `red` — the tone.
    pub color: String,
    /// The reason word behind the tone (`unmounted`, `path-unseeable`,
    /// `file-not-found`, `bad-ref`).
    pub reason: String,
    /// What the reason word does not say by itself — absent when it says it
    /// all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// The full teaching refusal, verbatim: subject, cause at its grain,
    /// partial state, and a runnable fix.
    pub message: String,
    /// How many times the page wrote this linkpath.
    pub count: u64,
}

/// The armed-fact set for one batch (v2 §4.4): the normative receipt content
/// is exactly this, rendered (§6.4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Armed {
    pub path: Path,
    /// The whole-file rev after the batch commits — same family/width as
    /// [`DeltaFile::file_rev_after`] and a subsequent `toc`'s `file_rev`, so a
    /// consumer learns the new file rev without a follow-up `toc`. A latency
    /// fact only; correctness stays the fingerprint/`root_after` world grain.
    /// Absent on a dry run — nothing was written, so the post-write rev does
    /// not exist yet (mirrors `root_after`'s dry-null at file grain).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_rev_after: Option<NodeRev>,
    pub edits: Vec<ArmedEdit>,
    /// Reaction outputs this landed batch armed synchronously. Empty on a dry,
    /// refused, out-of-scope, or never-armed write and omitted to preserve the
    /// pre-reaction response bytes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<EffectEnvelope>,
}

/// One armed edit: target identity echoed in the §2.1 grammar, rev
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

/// A rules-as-data verdict (v2 §11.1): a typed finding from a loaded rule
/// pack, never a decision (whether an `error` blocks is the host's action
/// mapping, not engine behavior). The field set is `crates/policy`'s
/// `Violation` verbatim, projected into the §2.1 grammar: `hpath` segments
/// carry `{h, n?}`, not bare strings. `budget_exceeded` is a finding in this
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

/// Verdict severity (v2 §11.1): descriptive policy data — how bad, per the
/// pack's convention — never what to do about it. Serialized flat lowercase
/// (`"error"`/`"warn"`/`"info"`). Independent of `policy::Severity` by
/// construction: the serving host projects one to the other, so no
/// wire→policy edge exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warn,
    Info,
}

// ---------------------------------------------------------------------------
// v2 §7 the Delta noun
// ---------------------------------------------------------------------------

/// One live notification frame body: `{"delta":{…}}` (v2 §7.1; events carry
/// no `id`, §3.1 classification). `diff.batches` elements are exactly this
/// shape, byte-identical to the live emission (§7.3 replay ≡ live).
///
/// The reaction plane is an additive sibling of the frozen [`Delta`]. Each
/// envelope owns reaction outputs at one evaluation boundary, so a later
/// schedule consumer can add `wake_at` beside `intents` without reshaping
/// the existing field.
///
/// `effects` postdates frozen v2 and is stripped for a v2 session by the
/// projection, not by `skip_serializing_if` (which skips on an empty value,
/// never on a v2 session). The guarantee is
/// `wire_serve::rev::V2_RESERVED_FIELDS`, the shared append-only table both
/// hosts consult before writing a v2 frame; a new v3-additive field here
/// ships with its row in the same commit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeltaFrame {
    pub delta: Delta,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<EffectEnvelope>,
}

/// One reaction evaluation's wire envelope: the complete hook outcome;
/// later reaction consumers extend this object additively.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectEnvelope {
    /// Intents admitted by the hook's declared capability ceiling.
    pub intents: Vec<Intent>,
    /// Complete intents dropped by the capability ceiling, retained as report
    /// data rather than silently discarded.
    pub narrowed: Vec<Intent>,
    /// Advisory evaluation findings that emitted no intent.
    pub findings: Vec<EffectFinding>,
    /// The declaration's `how:` block, byte-for-byte and uninterpreted.
    pub how: String,
}

/// One armed reaction descriptor. It says only what the evaluation armed;
/// delivery has not happened, so delivery state is intentionally unrepresentable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Intent {
    pub rule_id: String,
    pub seq: u32,
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<String>,
    /// The canonical receipt address minted before delivery.
    pub receipt: String,
}

/// A named advisory finding from reaction evaluation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EffectFinding {
    /// The predicate exhausted its declared evaluation budget.
    BudgetExceeded {
        rule_id: String,
        steps: u64,
        mem: u64,
    },
    /// The workspace's attested armed law could not be honored, so a reaction the
    /// artifact attests did not run. Reported, never a refusal: everything on the
    /// reaction plane runs after the write has landed, and failing a write on a
    /// reaction's behalf would hand a hook the veto the ruling denies it.
    ///
    /// This is the reaction host's channel onto the one artifact-fault surface
    /// (`policy::armed_law::ArmedFault`); `detail` is that surface's own rendering,
    /// so the operator reads the same words the door refuses with.
    ArmedFault {
        /// The armed id the fault is about — absent when the fault is about the
        /// artifact itself rather than any one rule.
        #[serde(skip_serializing_if = "Option::is_none")]
        rule_id: Option<String>,
        /// The operator-facing teaching text.
        detail: String,
    },
}

/// The fifth wire noun (v2 §7.1): one Delta = one batch = one root advance.
/// `seq` is the monotone per-workspace batch counter, per-daemon-epoch — a
/// daemon restart resets it (no counter survives on disk, §14), so `from_seq`
/// catchup is valid only within one epoch and cross-epoch catchup is
/// diff-by-root (§4.7), the root being the only restart-durable handle.
/// External changes carry `actor`/`now` absent — the engine never invents
/// identity or time it wasn't given (§9). The grain is exactly
/// [`DeltaNode`] (v2 §7.4).
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

/// One node-grain change entry (v2 §7.1/§7.2): identity echoed in the §2.1
/// grammar (flattened [`SecRef`] — same vocabulary as toc rows and armed
/// facts: one projection, three tenses), rev transition, span after. Entries
/// name the deepest section containing each changed byte range — ancestor
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

/// The closed six-class recovery enum (v2 §8). Every error frame carries one;
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
/// code is statically bound to exactly one recovery class —
/// [`ErrorCode::recovery`] is that binding, verbatim from the frozen table.
/// Clients treat unrecognized codes as `recovery`-dispatched. v1 `not_found`
/// is retired (split `file_not_found`/`ref_not_found`) — its string no longer
/// parses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    BadFrame,
    BadRequest,
    UnknownOp,
    UnsupportedProto,
    BadPath,
    /// v2 §8/§4.4: guard-passed zero occurrences of `old` in the target.
    NoMatch,
    /// v2 §8/§4.4: `old` occurs 2+ times. Extras: `matches` (the count) —
    /// add context bytes to `old`.
    NotUnique,
    /// v2 §8/§4.4: the post-apply reparse would lose containment. Extras:
    /// `lost` (the hpaths that would vanish).
    WouldCorrupt,
    /// v2 §8: the world outside the workspace — the file is gone. Echoes
    /// `path`.
    FileNotFound,
    /// v2 §8: I/O failure with its `cause`.
    IoError,
    InvalidUtf8,
    /// v2 §8/§11.3: a corpus-class capability (one that needs the resident
    /// corpus index) requested of an engine that has none — refused loud at
    /// admission. **RETIRED, unmintable** (hosts ruling, §3.3, 2026-08-06):
    /// with the stdio sidecar host deleted, every wire door is daemon-backed
    /// and the resident index is always reachable, so nothing mints this code.
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
    /// `changed`.
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
    /// The armed law cannot be honored — an attested index is absent on a
    /// once-armed workspace, is corrupt/unparseable, or an armed convention
    /// cannot load or evaluate. The door fails closed, bytes never land.
    /// Extras: `path` (the index page) + `message`. Env class — the
    /// workspace's own armed state is broken, not one request.
    ConventionFault,
    /// An armed convention's live evidence rev no longer equals its pinned
    /// armed rev. Extras: `expected` (`armed_rev`) + `actual` (`report_rev`).
    /// Refresh class — re-arm at the live rev, or revert the law.
    ArmedDrift,
    /// A one-sided file↔index change stops at the door: an ordinary write to
    /// the engine-managed attested index or a direct edit of an armed
    /// convention's `CHECK.md` is refused — the two sides must move through
    /// the one-act arming path, `--truth`, or realise. Extras: `path` +
    /// `message`. Fix class — arm properly, or `--force`.
    BindingBreak,
    /// Deletion or rename of the attested index or the once-armed marker is
    /// refused. A structural guard on the enforcement substrate — not
    /// force-escapable (a silent disarm by deleting the marker is the attack
    /// this defeats). Extras: `path` + `message`. Fix class.
    IndexIntegrity,
    /// TOCTOU refusal: the splice target's live disk bytes no longer equal
    /// the bytes the sealed batch validated against — an out-of-band writer
    /// landed between validate and commit; refusing beats blind-splicing
    /// validated spans into drifted bytes. Extras: `path` + `message`.
    /// Refresh class — re-read the file and re-plan the batch.
    WriteConflict,
    /// Another cooperating meridian writer holds the workspace write lock
    /// (`.meridian/write.lock`) — the choke-point refuses fast (`LOCK_NB`,
    /// never a wait). Extras: `message`. Retry class.
    WorkspaceBusy,
    /// A `splice.pin` from a real session actor whose selector no receipt
    /// covers — you cannot attest content that was never in your context.
    /// Extras: `path` + `message`. Fix class — read the exact selector in
    /// sections mode, then pin. The bare CLI (`actor` absent) is
    /// local-operator-trusted and never raises it; only the v3 pin path can
    /// emit it.
    ReadMintRequired,
    /// The `splice.pin` target page or selector does not exist, so there is
    /// nothing to fingerprint — a pin over an unresolvable address would mint
    /// a dangling claim; refusing at mint time is the honest door. Extras:
    /// `path` + `message`. Fix class.
    PinTargetMissing,
    /// A wire-origin write that changes existing content carried no
    /// fingerprint and no `force`. Extras: `path` + `message` (names the
    /// grain and the runnable command that mints the token). Fix class —
    /// send the token, or `force`. The CLI in-process door is exempt
    /// (local-operator trust) and never raises it.
    GuardRequired,
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
            | ErrorCode::AmbiguousRef
            | ErrorCode::BindingBreak
            | ErrorCode::IndexIntegrity
            | ErrorCode::ReadMintRequired
            | ErrorCode::PinTargetMissing
            | ErrorCode::GuardRequired => Recovery::Fix,
            ErrorCode::FileNotFound
            | ErrorCode::IoError
            | ErrorCode::InvalidUtf8
            | ErrorCode::DaemonOnly
            | ErrorCode::ConventionFault => Recovery::Env,
            ErrorCode::CasMismatch
            | ErrorCode::RefNotFound
            | ErrorCode::ArmedDrift
            | ErrorCode::WriteConflict => Recovery::Refresh,
            ErrorCode::LockTimeout | ErrorCode::StaleView | ErrorCode::WorkspaceBusy => {
                Recovery::Retry
            }
            ErrorCode::RootMismatch | ErrorCode::RootUnknown => Recovery::Resync,
            ErrorCode::BadFrame | ErrorCode::UnsupportedProto | ErrorCode::Internal => {
                Recovery::Respawn
            }
        }
    }
}

/// v2 §8: the error envelope — a nested object under the response's `error`
/// key: `code` + the required closed `recovery` class + optional human
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
    /// `unsupported_proto`: protos this server speaks.
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
    /// (occurrence index or anchor) — a ref-carrying surface.
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
    /// batch laws) — identities in the §2.1 grammar's segments.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lost: Option<Vec<Vec<HpathSeg>>>,
    /// `io_error`: the underlying cause, carried (v2 §8).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    /// `bad_request` on non-disjoint batch targets (v2 §4.4): the offending
    /// targets echoed in the §2.1 grammar (a ref-carrying surface).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overlap: Option<Vec<SecRef>>,
    /// `cas_mismatch` (v3-additive): which rung of the mismatch-recovery ladder
    /// this refusal reached — `1` change diff, `2` new content + new
    /// fingerprint, `3` the bare mismatch floor. The discriminant lets a caller
    /// dispatch on richness without probing which extras happen to be present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rung: Option<u32>,
    /// `cas_mismatch` rung 1: the change from the caller's own pinned picture to
    /// the node's current bytes — a unified line diff for section bodies, ops
    /// form for frontmatter. Scoped to the target the caller addressed and
    /// capped by size; a caller applies it and resends without a re-read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff: Option<String>,
    /// `cas_mismatch` rung 2: the targeted node's current bytes, whole. The
    /// ETag-412-with-body shape — sent when no diff is computable or the diff
    /// exceeded its cap.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_content: Option<String>,
    /// `cas_mismatch` rungs 1–2: the token to resend with. It rides whichever
    /// rung carries recoverable content, because that is the rung the caller
    /// resends from — a rung that made the read unnecessary must not send them
    /// back for the token.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_fingerprint: Option<NodeRev>,
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
            rung: None,
            diff: None,
            new_content: None,
            new_fingerprint: None,
        }
    }
}
