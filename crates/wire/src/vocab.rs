//! **Charter:** the four wire nouns — `path` / `span` / `node_rev` / `root` —
//! introduced at rung 1, never replaced (contract §2), plus the request-id and
//! proto constants.
//!
//! No line numbers on the wire: positions are byte spans, exclusively. Any
//! future line-number field is an amendment problem and MUST NOT inherit
//! meridian's physical+1 convention (contract §2).

use serde::{Deserialize, Serialize};

/// Protocol version this workspace speaks (contract §5.1).
pub const PROTO: u32 = 1;

/// Workspace-relative file path (contract §2): forward slashes, UTF-8, no
/// leading `/`, no `.`/`..` segments. [`WirePath::validate`] enforces the
/// grammar; violations are the `bad_path` error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WirePath(pub String);

impl WirePath {
    /// Path law check (contract §2). `Err` carries the human message for the
    /// `bad_path` envelope.
    pub fn validate(&self) -> Result<(), String> {
        todo!("empty / absolute / dot-segment rejection per contract §2")
    }
}

/// Half-open byte range `[start, end)` into a file's raw on-disk bytes,
/// serialized as a two-element JSON array (contract §2).
///
/// Span laws: byte offsets on UTF-8 character boundaries; block spans exclude
/// the final line terminator; inline spans include their delimiters; spans are
/// only issued against valid-UTF-8 files (which is what makes `splice`
/// byte-safe).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span(pub u64, pub u64);

/// Short lowercase-hex content hash of a node's bytes (contract §2). Role
/// frozen (the CAS token for `splice`); algorithm and length are a rung-2
/// amendment. **Opaque**: equality only — deliberately no `Ord`, never parse.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodeRev(pub String);

/// Merkle root cursor, algorithm-prefixed hex, e.g. `"b3:88d2aa"` (contract
/// §2). Role frozen (rung 3–4 integrity cursor); detail non-frozen. Opaque:
/// equality only — deliberately no `Ord`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Root(pub String);

/// Client-chosen correlation token: a JSON integer in `[0, 2^53)` (contract
/// §3). The bound makes the idiomatic Go decode path (float64) exact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RequestId(u64);

impl RequestId {
    pub const MAX: u64 = (1 << 53) - 1;

    /// `None` when out of range — the caller answers `bad_request` with
    /// `id:null` (contract §3).
    pub fn new(v: u64) -> Option<Self> {
        (v <= Self::MAX).then_some(Self(v))
    }

    pub fn get(self) -> u64 {
        self.0
    }
}
