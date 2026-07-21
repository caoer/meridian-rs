//! The contract-rev negotiation state + the v3 vocabulary projection
//! (`docs/wire-contract-v3-amendment.md`).
//!
//! The frozen v2 `wire` types serialize BYTE-FOR-BYTE as contract v2 forever —
//! nothing in this module touches them. v3 is expressed as a pure projection at
//! the envelope layer: outgoing v2-shaped frames are re-keyed `root` →
//! `fingerprint` on the way out, and incoming v3 requests are re-keyed
//! `fingerprint` → `root` on the way in, so the strict decoder and every arm
//! stay v2-only. One rename table, applied in two directions.
//!
//! # Why a projection, not a serde rename
//! A serde attribute change on the `wire` types would break the frozen v2
//! goldens (`crates/wire/tests/contract_v2.rs`, `crates/testsuite/tests/
//! wire_vocab.rs`) and the byte-identical guarantee live v2 consumers pin via
//! `hello`. The projection keeps v2 emission on the untouched typed path (the
//! serve loop serializes `wire::Response` directly for v2) and only re-shapes
//! when the session negotiated v3.
//!
//! # No dual-emit (the amendment's hard rule)
//! A v2 session emits `root` and never `fingerprint`; a v3 session emits
//! `fingerprint` and never `root`. One epoch, one rev.

use serde_json::{Map, Value};

/// The negotiated contract rev — per-process serve-session state (one epoch,
/// one rev). Defaults to [`Rev::V2`] until a `hello` declares otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Rev {
    /// The frozen contract v2 vocabulary (`root`), byte-for-byte.
    #[default]
    V2,
    /// The v3-amendment vocabulary (`fingerprint`).
    V3,
}

impl Rev {
    /// Map the `hello` `contract` declaration to the negotiated rev. Absent or
    /// `"v2"` is v2 (today's behavior); `"v3"` is v3. Any other value never
    /// reaches here — the decoder rejected it (`is_known`).
    pub(crate) fn from_contract(contract: Option<&str>) -> Rev {
        match contract {
            Some("v3") => Rev::V3,
            _ => Rev::V2,
        }
    }
}

// ---------------------------------------------------------------------------
// The v3 rename table, applied in two directions.
//
// Fields (the fingerprint-VALUED slots): the plain value and every compound
// name that spells the concept. `expected`/`actual`/`required`/`changed` keep
// their names — already vocabulary-neutral, they never spell "root".
// ---------------------------------------------------------------------------

/// Response/notification key renames (v2 → v3): `(v2_key, v3_key)`.
const RESPONSE_KEYS: [(&str, &str); 5] = [
    ("root", "fingerprint"),
    ("root_before", "fingerprint_before"),
    ("root_after", "fingerprint_after"),
    ("as_of_root", "as_of_fingerprint"),
    ("live_root", "live_fingerprint"),
];

/// Request key renames (v3 → v2): `(v3_key, v2_key)`. These ride only requests
/// (splice/diff/links), all at the flattened top level — no arbitrary-key map
/// sits there, so a top-level rename is collision-free.
const REQUEST_KEYS: [(&str, &str); 4] = [
    ("if_fingerprint", "if_root"),
    ("from_fingerprint", "from_root"),
    ("to_fingerprint", "to_root"),
    ("require_fingerprint", "require_root"),
];

/// Error `code` string renames (v2 → v3). The recovery class is unchanged;
/// only the spelling follows the fingerprint vocabulary.
const ERROR_CODES: [(&str, &str); 2] = [
    ("root_mismatch", "fingerprint_mismatch"),
    ("root_unknown", "fingerprint_unknown"),
];

/// Hello `caps` string renames (v2 → v3): the op name and the two dotted
/// field-amendment caps that spell the concept.
const CAP_STRINGS: [(&str, &str); 3] = [
    ("root", "fingerprint"),
    ("splice.if_root", "splice.if_fingerprint"),
    ("links.require_root", "links.require_fingerprint"),
];

/// Rename `from` → `to` in place if present, preserving the value.
fn rename_key(obj: &mut Map<String, Value>, from: &str, to: &str) {
    if let Some(v) = obj.remove(from) {
        obj.insert(to.to_string(), v);
    }
}

// ---------------------------------------------------------------------------
// Request side: v3 → v2 (so the strict decoder stays v2-only)
// ---------------------------------------------------------------------------

/// Re-key a v3 request into its v2 form BEFORE strict decode: the `fingerprint`
/// op tag becomes `root`, and the four fingerprint request fields become their
/// `root` names. A no-op for keys the frame does not carry, so a request that
/// happens to use the v2 spelling passes through untouched (input leniency,
/// amendment §"Input acceptance"). Never called for a v2 session.
pub(crate) fn rename_request(obj: &mut Map<String, Value>) {
    if obj.get("op").and_then(Value::as_str) == Some("fingerprint") {
        obj.insert("op".to_string(), Value::String("root".to_string()));
    }
    for (v3, v2) in REQUEST_KEYS {
        rename_key(obj, v3, v2);
    }
}

// ---------------------------------------------------------------------------
// Response/notification side: v2 → v3
// ---------------------------------------------------------------------------

/// Re-shape one already-serialized v2 response frame into v3 in place. Touches
/// only the known fingerprint slots under `body`/`error` — never descends into
/// the arbitrary-key maps (`files`, `resolved`, `unresolved`), where a corpus
/// path or raw linkpath could legitimately be the string `"root"`.
pub(crate) fn project_response(frame: &mut Value) {
    let Some(obj) = frame.as_object_mut() else {
        return;
    };
    if let Some(body) = obj.get_mut("body").and_then(Value::as_object_mut) {
        for (v2, v3) in RESPONSE_KEYS {
            rename_key(body, v2, v3);
        }
        // hello body: rewrite the fingerprint caps + echo the negotiated rev.
        // `server` is present on the hello body alone.
        if body.contains_key("server") {
            if let Some(caps) = body.get_mut("caps").and_then(Value::as_array_mut) {
                for cap in caps.iter_mut() {
                    rewrite_cap(cap);
                }
            }
            body.insert("contract".to_string(), Value::String("v3".to_string()));
        }
        // diff response: each batch is a `{"delta":{…}}` frame.
        if let Some(batches) = body.get_mut("batches").and_then(Value::as_array_mut) {
            for batch in batches.iter_mut() {
                project_delta_frame(batch);
            }
        }
    }
    if let Some(err) = obj.get_mut("error").and_then(Value::as_object_mut) {
        for (v2, v3) in RESPONSE_KEYS {
            rename_key(err, v2, v3);
        }
        if let Some(Value::String(code)) = err.get_mut("code") {
            for (v2, v3) in ERROR_CODES {
                if code == v2 {
                    *code = v3.to_string();
                    break;
                }
            }
        }
    }
}

/// Re-shape one live notification frame (`{"delta":{…}}`) into v3 in place:
/// the two Delta fingerprint slots only, never the `files` array beneath.
pub(crate) fn project_delta_frame(frame: &mut Value) {
    if let Some(delta) = frame
        .as_object_mut()
        .and_then(|o| o.get_mut("delta"))
        .and_then(Value::as_object_mut)
    {
        rename_key(delta, "root_before", "fingerprint_before");
        rename_key(delta, "root_after", "fingerprint_after");
    }
}

/// Rewrite one hello `caps` entry to its v3 spelling, if it names the concept.
fn rewrite_cap(cap: &mut Value) {
    if let Some(s) = cap.as_str() {
        for (v2, v3) in CAP_STRINGS {
            if s == v2 {
                *cap = Value::String(v3.to_string());
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn from_contract_maps_declared_rev() {
        assert_eq!(Rev::from_contract(None), Rev::V2);
        assert_eq!(Rev::from_contract(Some("v2")), Rev::V2);
        assert_eq!(Rev::from_contract(Some("v3")), Rev::V3);
    }

    #[test]
    fn request_op_and_fields_rekey_v3_to_v2() {
        let mut obj = json!({"id":1,"op":"fingerprint"})
            .as_object()
            .unwrap()
            .clone();
        rename_request(&mut obj);
        assert_eq!(obj["op"], json!("root"));

        let mut splice = json!({"op":"splice","if_fingerprint":"b3:x"})
            .as_object()
            .unwrap()
            .clone();
        rename_request(&mut splice);
        assert!(!splice.contains_key("if_fingerprint"));
        assert_eq!(splice["if_root"], json!("b3:x"));
    }

    #[test]
    fn response_body_root_becomes_fingerprint() {
        let mut frame = json!({"id":1,"ok":true,"body":{"root":"b3:x","seq":2}});
        project_response(&mut frame);
        assert_eq!(frame["body"]["fingerprint"], json!("b3:x"));
        assert!(frame["body"].as_object().unwrap().get("root").is_none());
    }

    #[test]
    fn links_files_map_key_named_root_is_untouched() {
        // A raw linkpath `[[root]]` is a legitimate map KEY — projection must
        // never re-key it. Only the fingerprint SLOTS move.
        let mut frame = json!({"id":1,"ok":true,"body":{
            "as_of_root":"b3:a","live_root":"b3:a","changes_seq":1,
            "files":{"notes/plan.md":{"resolved":{},"unresolved":{"root":2}}}}});
        project_response(&mut frame);
        assert_eq!(frame["body"]["as_of_fingerprint"], json!("b3:a"));
        assert_eq!(frame["body"]["live_fingerprint"], json!("b3:a"));
        // the corpus map key survives verbatim
        assert_eq!(
            frame["body"]["files"]["notes/plan.md"]["unresolved"]["root"],
            json!(2)
        );
    }

    #[test]
    fn error_code_and_extras_rekey() {
        let mut frame = json!({"id":1,"ok":false,"error":{
            "code":"root_mismatch","recovery":"resync",
            "expected":"b3:a","actual":"b3:b","changed":["x.md"]}});
        project_response(&mut frame);
        assert_eq!(frame["error"]["code"], json!("fingerprint_mismatch"));
        // vocabulary-neutral extras keep their names
        assert_eq!(frame["error"]["expected"], json!("b3:a"));
        assert_eq!(frame["error"]["changed"], json!(["x.md"]));
    }

    #[test]
    fn hello_body_caps_and_contract_echo() {
        let mut frame = json!({"id":1,"ok":true,"body":{
            "proto":1,"server":"meridian-sidecar/2.0",
            "caps":["toc","splice.if_root","root","links.require_root","diff"],
            "root":"b3:a"}});
        project_response(&mut frame);
        assert_eq!(frame["body"]["fingerprint"], json!("b3:a"));
        assert_eq!(frame["body"]["contract"], json!("v3"));
        assert_eq!(
            frame["body"]["caps"],
            json!([
                "toc",
                "splice.if_fingerprint",
                "fingerprint",
                "links.require_fingerprint",
                "diff"
            ])
        );
    }

    #[test]
    fn delta_frame_root_slots_rekey_files_untouched() {
        let mut frame = json!({"delta":{
            "seq":1,"root_before":"b3:a","root_after":"b3:b",
            "files":[{"path":"root","change":"modified","nodes":[]}]}});
        project_delta_frame(&mut frame);
        assert_eq!(frame["delta"]["fingerprint_before"], json!("b3:a"));
        assert_eq!(frame["delta"]["fingerprint_after"], json!("b3:b"));
        // a file literally named "root" keeps its path
        assert_eq!(frame["delta"]["files"][0]["path"], json!("root"));
    }
}
