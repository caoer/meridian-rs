//! **Charter:** request and response shapes per op — rung 1 frozen (`hello`,
//! `toc`, `extract`), rung 2 sketched per contract §6 (`resolve`, `splice`),
//! rung 3 reserved (`root`, `guard` — named by the capability ladder, no
//! contract shape yet).
//!
//! Evolution law this module lives under (contract §7): the server rejects
//! unknown request fields loudly (`bad_request`) — a silently ignored CAS
//! guard would corrupt data quietly; clients ignore unknown response fields.

use serde::{Deserialize, Serialize};

use crate::error::ErrorBody;
use crate::node::Node;
use crate::vocab::{NodeRev, Span, WirePath};

/// The op vocabulary, rungs 1–3. Tagged by the wire `op` field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    // ---- rung 1: FROZEN (contract §5) ------------------------------------
    Hello(HelloReq),
    Toc(TocReq),
    Extract(ExtractReq),
    // ---- rung 2: sketched, NON-FROZEN (contract §6) ----------------------
    Resolve(ResolveReq),
    Splice(SpliceReq),
    // ---- rung 3: reserved ------------------------------------------------
    /// Integrity cursor read (rung 3). **No wire shape exists yet** —
    /// wire-contract-v1 does not define this op; the variant reserves the
    /// name so rung 3 is additive here. Fields arrive by contract amendment.
    Root,
    /// Commit guard (rung 3). Same status as [`Op::Root`]: name reserved,
    /// shape by amendment.
    Guard,
}

/// `hello` — version handshake (§5.1). Optional but recommended first
/// request; absent a handshake the sidecar assumes proto 1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HelloReq {
    pub proto: u32,
    /// Client name/version, free-form, for logs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client: Option<String>,
}

/// `toc` — section map (§5.3): frontmatter + heading nodes only.
/// `toc(path)` ≡ `extract(path, kinds:["frontmatter","heading"])`; its own op
/// because the section map is the dominant read pattern.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TocReq {
    pub path: WirePath,
}

/// `extract` — full node inventory (§5.4). One file per request; batching is
/// the client's loop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtractReq {
    pub path: WirePath,
    /// Kind filter. Open strings, not [`crate::NodeKind`], by contract: a
    /// client may ask for a kind an older sidecar doesn't emit — unknown
    /// names match nothing, never error.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kinds: Option<Vec<String>>,
}

/// `resolve` — ref → span + node_rev (§6.1, NON-FROZEN). Ref grammar:
/// `#hpath` (`/`-joined input form) or `#^anchor-id`; grammar details are
/// rung-2 decisions confined to the input side.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolveReq {
    pub path: WirePath,
    #[serde(rename = "ref")]
    pub r#ref: String,
}

/// `splice` — node-level CAS write (§6.2, NON-FROZEN): replace exactly the
/// bytes of `span`, guarded by `if_node_rev`.
///
/// Axis E position (b): the sidecar splices unconditionally for an
/// already-authorized caller — write authorization is Go-side, so no
/// actor/authorization field exists or will exist on this request. The
/// oracle sees states, not actors.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpliceReq {
    pub path: WirePath,
    pub span: Span,
    pub if_node_rev: NodeRev,
    pub text: String,
}

/// Success body per op. The `ok:true` boolean and `id` echo live on the
/// envelope ([`crate::envelope::ResponseFrame`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OkBody {
    Hello(HelloOk),
    /// Shared by `toc` and `extract` — same shape by contract (§5.3/§5.4).
    Nodes(NodesOk),
    Resolve(ResolveOk),
    Splice(SpliceOk),
}

/// Success or failure — the `ok` boolean on the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ResponseBody {
    Ok(OkBody),
    Err(ErrorBody),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HelloOk {
    /// The version now in effect (equals the requested version).
    pub proto: u32,
    /// e.g. `"meridian-rs/0.1.0"`.
    pub server: String,
    /// The **complete** op-name set this sidecar answers, `hello` included.
    /// Capability discovery is `X ∈ caps` — never version sniffing.
    pub caps: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodesOk {
    /// Echoed.
    pub path: WirePath,
    /// In node order (§5.2); `[]` when none.
    pub nodes: Vec<Node>,
}

/// §6.1 sketch. For a heading ref the span is the **section** (heading line
/// through end of subtree); for an anchor ref, the anchor's host block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolveOk {
    pub path: WirePath,
    pub span: Span,
    pub node_rev: NodeRev,
}

/// §6.2 sketch: the new span of the spliced region and its new `node_rev`,
/// so a client chains edits without re-`resolve`. A `root` field joins
/// additively at rung 3.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpliceOk {
    pub span: Span,
    pub node_rev: NodeRev,
}
