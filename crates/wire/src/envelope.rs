//! **Charter:** the frame envelope — request/response/event classification
//! and the `id` echo law, independent of how frames are delimited (that's
//! [`crate::codec`]).
//!
//! Classification rule (contract §3, frozen): a sidecar→client frame carrying
//! the `id` key is a **response**; a frame without `id` is an **event**
//! (rung 4+; the v1 sidecar emits none). Client→sidecar frames are requests,
//! identified by `op`.

use serde::{Deserialize, Serialize};

use crate::op::{Op, ResponseBody};
use crate::vocab::RequestId;

/// Any frame either side of the wire.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    Request(RequestFrame),
    Response(ResponseFrame),
    Event(EventFrame),
}

/// One request (contract §3): `op` + optional correlation `id` + op fields.
/// `id` is optional to keep `echo '{"op":…}' | ./sidecar` frictionless;
/// pipelining clients MUST send it, MUST NOT reuse one while outstanding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequestFrame {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<RequestId>,
    #[serde(flatten)]
    pub op: Op,
}

/// One response (contract §3): every request gets exactly one. `id` is the
/// request's id echoed by value — `null` (here `None`) when the request had
/// no id, a non-conforming id, or could not be parsed. The `id` key is
/// always serialized; correlation is by id, never by order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResponseFrame {
    pub id: Option<RequestId>,
    /// `ok:true` body per op, or the §4 error envelope.
    #[serde(flatten)]
    pub body: ResponseBody,
}

/// An event frame: no `id` key; carries `sub` to name its subscription.
/// Shape is entirely a rung-4 decision — only the classification rule and
/// the recovery law ("a subscriber that missed events recovers by
/// `diff(my_root, live_root)`; events are a latency optimization, never a
/// correctness dependency") are binding now.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EventFrame(pub serde_json::Value);
