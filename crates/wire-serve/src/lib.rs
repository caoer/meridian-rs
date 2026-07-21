//! The shared typed edge — strict decode + the read-op arms — lifted out of the
//! sidecar so the resident registry daemon and the per-workspace sidecar answer
//! the wire contract through ONE implementation (arch map A6: "lift, don't
//! duplicate").
//!
//! # Charter
//! **Owns:** the hand-rolled strict-decode pass ([`decode`] — wire §3.2 server
//! law: unknown request fields and unknown enum values are rejected loudly,
//! because serde's `deny_unknown_fields` does not compose with `flatten`) and
//! the read-op arms ([`read`] — `toc`/`cat`/`extract`/`links`/`resolve`)
//! served over BORROWED parsed state.
//!
//! **Never does:** own the corpus, read disk, parse, or hold a serve loop.
//! Every read arm takes already-built `model` state (`&Document`, or the
//! `&CorpusIndex` + document map) and the ambient root as data; the CALLER
//! obtains that state — the sidecar builds it per request, the daemon reuses
//! its warm engine. That split is the whole point: one projection, two state
//! sources, no drift.
//!
//! # Root / diff live with the caller
//! `root` and `diff` are cursor/history plumbing whose source genuinely differs
//! by host (the sidecar owns a per-epoch ring; the resident daemon has none
//! until the watcher lands), so they are NOT arms here — each host builds those
//! two responses from its own `seq`/history. This crate holds only the corpus
//! reads whose logic is identical across hosts.

pub mod decode;
pub mod read;

use wire::{ErrorBody, ErrorCode};

/// The one protocol both hosts speak (wire §3.2, proto-1). The strict decode
/// validates a `hello`'s `proto` against this.
pub const PROTO: u32 = 1;

/// Is `rev` a contract rev the server serves (wire v3 amendment)? An unknown
/// declared rev is refused LOUD at `hello` decode, never silently downgraded.
/// The negotiation itself (which rev a session runs) is the caller's; this is
/// only the decode-time known-set check.
#[must_use]
pub fn is_known_rev(rev: &str) -> bool {
    matches!(rev, "v2" | "v3")
}

/// A `bad_request` carrying a human message — the strict-decode workhorse
/// (wire §8: `bad_request` ⇒ recovery `fix`).
#[must_use]
pub fn bad_request(message: impl Into<String>) -> Box<ErrorBody> {
    let mut e = ErrorBody::new(ErrorCode::BadRequest);
    e.message = Some(message.into());
    Box::new(e)
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use wire::{ErrorCode, Op};

    use super::{bad_request, is_known_rev};

    fn obj(v: Value) -> serde_json::Map<String, Value> {
        match v {
            Value::Object(map) => map,
            _ => panic!("test frame is an object"),
        }
    }

    #[test]
    fn known_rev_set_is_v2_and_v3() {
        assert!(is_known_rev("v2"));
        assert!(is_known_rev("v3"));
        assert!(!is_known_rev("v4"));
        assert!(!is_known_rev(""));
    }

    #[test]
    fn bad_request_carries_the_fix_class_and_message() {
        let e = bad_request("boom");
        assert_eq!(e.code, ErrorCode::BadRequest);
        assert_eq!(e.recovery, wire::Recovery::Fix);
        assert_eq!(e.message.as_deref(), Some("boom"));
    }

    #[test]
    fn decode_accepts_a_read_op() {
        let op =
            super::decode::decode(&obj(json!({"op": "toc", "path": "a.md"}))).expect("toc decodes");
        assert!(matches!(op, Op::Toc { path } if path.0 == "a.md"));
    }

    #[test]
    fn decode_rejects_an_unknown_field_by_name() {
        // serde would silently drop `bogus`; the strict pass refuses it loud.
        let e = super::decode::decode(&obj(json!({"op": "toc", "path": "a.md", "bogus": 1})))
            .expect_err("unknown field is refused");
        assert_eq!(e.code, ErrorCode::BadRequest);
        assert!(
            e.message.as_deref().is_some_and(|m| m.contains("bogus")),
            "the refusal names the field: {:?}",
            e.message
        );
    }

    #[test]
    fn decode_answers_unknown_op_for_an_unarmed_name() {
        let e = super::decode::decode(&obj(json!({"op": "nope"}))).expect_err("unknown op refused");
        assert_eq!(e.code, ErrorCode::UnknownOp);
    }
}
