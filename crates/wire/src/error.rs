//! **Charter:** the §4 error envelope — every failure response, one shape,
//! machine code at the top level, code-specific fields beside it.

use serde::{Deserialize, Serialize};

use crate::vocab::NodeRev;

/// v1 error-code namespace (contract §4). Flat lowercase snake_case on the
/// wire. New codes are additive; clients treat unrecognized codes as
/// non-retryable. `cas_mismatch` is the one code whose *purpose* is to be
/// retried (after re-`resolve`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Rung 2, sketched (contract §6.2): `if_node_rev` no longer matches.
    CasMismatch,
}

/// The failure body: `{"ok":false, "error":..., ...}` minus the `id` key,
/// which the envelope layer owns ([`crate::envelope::ResponseFrame`]).
/// Code-specific fields sit at the top level beside `error` (contract §4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorBody {
    pub error: ErrorCode,
    /// Human-oriented, never parsed by clients.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// `cas_mismatch`: the rev the request pinned.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<NodeRev>,
    /// `cas_mismatch`: the rev the world holds now.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual: Option<NodeRev>,
    /// `unsupported_proto`: protos this sidecar speaks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supported: Option<Vec<u32>>,
    /// `bad_path` / `not_found`: the offending path, echoed as sent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}
