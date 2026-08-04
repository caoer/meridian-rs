//! Contract-rev negotiation + v3 vocabulary projection (one implementation for
//! both hosts). Frozen v2 wire types stay byte-for-byte; v3 is envelope re-key
//! (`root` ↔ `fingerprint`) so decode/arms stay v2-only. No dual-emit: one
//! epoch, one rev.

use serde_json::{Map, Value};

/// Negotiated contract rev (per-session). Defaults to [`Rev::V2`] until `hello`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Rev {
    /// Frozen contract v2 vocabulary (`root`).
    #[default]
    V2,
    /// v3-amendment vocabulary (`fingerprint`).
    V3,
}

impl Rev {
    /// Map `hello.contract` to rev. Absent/`"v2"` → v2; `"v3"` → v3.
    /// Unknown values never reach here (`is_known_rev` refused them).
    #[must_use]
    pub fn from_contract(contract: Option<&str>) -> Rev {
        match contract {
            Some("v3") => Rev::V3,
            _ => Rev::V2,
        }
    }
}

// v3 rename table (two directions). Fingerprint-valued slots only;
// expected/actual/required/changed stay vocabulary-neutral.

/// Response/notification key renames (v2 → v3): `(v2_key, v3_key)`.
const RESPONSE_KEYS: [(&str, &str); 5] = [
    ("root", "fingerprint"),
    ("root_before", "fingerprint_before"),
    ("root_after", "fingerprint_after"),
    ("as_of_root", "as_of_fingerprint"),
    ("live_root", "live_fingerprint"),
];

/// Request key renames (v3 → v2): top-level only, collision-free.
const REQUEST_KEYS: [(&str, &str); 4] = [
    ("if_fingerprint", "if_root"),
    ("from_fingerprint", "from_root"),
    ("to_fingerprint", "to_root"),
    ("require_fingerprint", "require_root"),
];

/// Error `code` renames (v2 → v3); recovery class unchanged.
const ERROR_CODES: [(&str, &str); 2] = [
    ("root_mismatch", "fingerprint_mismatch"),
    ("root_unknown", "fingerprint_unknown"),
];

/// Hello `caps` renames (v2 → v3).
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

// Request side: v3 → v2 (strict decoder stays v2-only)

/// Re-key a v3 request into v2 form before strict decode. No-op for absent keys
/// (input leniency). Never called for a v2 session.
pub fn rename_request(obj: &mut Map<String, Value>) {
    if obj.get("op").and_then(Value::as_str) == Some("fingerprint") {
        obj.insert("op".to_string(), Value::String("root".to_string()));
    }
    for (v3, v2) in REQUEST_KEYS {
        rename_key(obj, v3, v2);
    }
}

// Response/notification side: v2 → v3

/// Re-shape a serialized v2 response into v3. Touches only known fingerprint
/// slots under `body`/`error` — never arbitrary-key maps (`files`, etc.).
pub fn project_response(frame: &mut Value) {
    let Some(obj) = frame.as_object_mut() else {
        return;
    };
    if let Some(body) = obj.get_mut("body").and_then(Value::as_object_mut) {
        for (v2, v3) in RESPONSE_KEYS {
            rename_key(body, v2, v3);
        }
        // Hello: rewrite fingerprint caps + echo negotiated rev. v3-only caps
        // pushed here so frozen v2 caps stay byte-identical.
        if body.contains_key("server") {
            if let Some(caps) = body.get_mut("caps").and_then(Value::as_array_mut) {
                for cap in caps.iter_mut() {
                    rewrite_cap(cap);
                }
                caps.push(Value::String("read".to_string()));
                caps.push(Value::String("check_write".to_string()));
                // M1 U8b: plan-level splice batch — v3-only composite cap.
                caps.push(Value::String("splice.plan_edits".to_string()));
                // S3/R23: pin field amendment as dotted op.field cap.
                caps.push(Value::String("splice.pin".to_string()));
                // Birth op at OP grain (no dotted create.<field>).
                caps.push(Value::String("create".to_string()));
            }
            body.insert("contract".to_string(), Value::String("v3".to_string()));
        }
        // diff: each batch is a {"delta":{…}} frame.
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

/// Demote a response for a FROZEN v2 session: drop the v3-ADDITIVE error extras
/// a v2 caller was never promised. Returns `None` when the frame already carries
/// none, so the frozen path serializes the typed value directly and stays
/// byte-identical — the round-trip is paid only by the frames that grew.
///
/// The extras are U11's mismatch-recovery ladder. They are v3-only for the same
/// reason the v3 caps are pushed in [`project_response`] rather than declared on
/// the typed value: the amendment's law is that a v2 session never grows a field.
///
/// **`rung` is the authorship mark.** An envelope carrying one was written by
/// `ladder::enrich`, which authors SIX slots — the four extras plus the `message`
/// and `path` that teach them. Demotion clears all six, so a frozen v2
/// `cas_mismatch` prints exactly the `{code, recovery, expected, actual}` it
/// always printed. Envelopes with no rung are nobody else's business here:
/// U10's rung-0 refusals keep their message on v2 untouched.
///
/// This is the ONE place the ladder meets contract rev. The engine mints the
/// richest rung it can regardless; whether the caller is entitled to read it is
/// a vocabulary question, decided here at the projection seam, so no arm learns
/// about revs.
#[must_use]
pub fn demote_v2(response: &wire::Response) -> Option<wire::Response> {
    let wire::ResponsePayload::Error { error } = &response.payload else {
        return None;
    };
    error.rung?;
    let mut demoted = response.clone();
    if let wire::ResponsePayload::Error { error } = &mut demoted.payload {
        error.rung = None;
        error.diff = None;
        error.new_content = None;
        error.new_fingerprint = None;
        error.message = None;
        error.path = None;
    }
    Some(demoted)
}

/// Attach `meta: {duration_us}` beside body/error (U7 timing; v3-only by caller).
pub fn attach_meta(frame: &mut Value, duration_us: u64) {
    if let Some(obj) = frame.as_object_mut() {
        obj.insert(
            "meta".to_string(),
            serde_json::json!({ "duration_us": duration_us }),
        );
    }
}

/// Re-shape a live `{"delta":{…}}` notification: fingerprint slots only, not `files`.
pub fn project_delta_frame(frame: &mut Value) {
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
        // Corpus map keys must not be re-keyed — only fingerprint slots move.
        let mut frame = json!({"id":1,"ok":true,"body":{
            "as_of_root":"b3:a","live_root":"b3:a","changes_seq":1,
            "files":{"notes/plan.md":{"resolved":{},"unresolved":{"root":2}}}}});
        project_response(&mut frame);
        assert_eq!(frame["body"]["as_of_fingerprint"], json!("b3:a"));
        assert_eq!(frame["body"]["live_fingerprint"], json!("b3:a"));
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

    /// R23: every v3-era splice field amendment advertised as `splice.<field>`,
    /// derived from `SPLICE_V3_FIELDS \ SPLICE_V2_FIELDS` (not hand-copied).
    #[test]
    fn v3_splice_amendments_are_all_advertised() {
        let amendments: Vec<&str> = crate::decode::SPLICE_V3_FIELDS
            .iter()
            .copied()
            .filter(|f| !crate::decode::SPLICE_V2_FIELDS.contains(f))
            .collect();
        // Anti-vacuity: empty difference would make the loop assert nothing.
        assert!(
            !amendments.is_empty(),
            "the v3-era splice amendments must be a non-empty set — an empty \
             difference makes this enumeration vacuous"
        );

        let mut frame = json!({"id":1,"ok":true,"body":{
            "proto":1,"server":"meridian-sidecar/2.0",
            "caps":["toc","splice","splice.if_root"],
            "root":"b3:a"}});
        project_response(&mut frame);
        let caps: Vec<&str> = frame["body"]["caps"]
            .as_array()
            .expect("caps array")
            .iter()
            .map(|c| c.as_str().expect("cap is a string"))
            .collect();

        for field in &amendments {
            let want = format!("splice.{field}");
            assert!(
                caps.contains(&want.as_str()),
                "v3-era splice field `{field}` is honoured by the decoder but \
                 NOT advertised: the v3 caps projection must push `{want}` \
                 (R23 — a field-only amendment ships as a dotted `op.field` \
                 cap). caps = {caps:?}"
            );
        }
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
                "diff",
                "read",
                "check_write",
                "splice.plan_edits",
                "splice.pin",
                "create"
            ])
        );
    }

    /// Birth op advertised at OP grain only — no dotted `create.<field>` (no v2 twin).
    #[test]
    fn v3_advertises_create_at_op_grain_and_v2_never_does() {
        let hello = || {
            json!({"id":1,"ok":true,"body":{
                "proto":1,"server":"meridian-sidecar/2.0",
                "caps":["toc","splice"],"root":"b3:a"}})
        };
        let mut v3 = hello();
        project_response(&mut v3);
        let caps: Vec<&str> = v3["body"]["caps"]
            .as_array()
            .expect("caps array")
            .iter()
            .map(|c| c.as_str().expect("cap is a string"))
            .collect();
        assert!(
            caps.contains(&"create"),
            "v3 advertises the birth op: {caps:?}"
        );
        assert!(
            !caps.iter().any(|c| c.starts_with("create.")),
            "the birth op ships at OP grain — no dotted create.<field> cap: {caps:?}"
        );

        // Unprojected (v2) hello never carries create — projection is the only source.
        let v2 = hello();
        let v2_caps: Vec<&str> = v2["body"]["caps"]
            .as_array()
            .expect("caps array")
            .iter()
            .map(|c| c.as_str().expect("cap is a string"))
            .collect();
        assert!(
            !v2_caps.contains(&"create"),
            "a v2 session never advertises the birth op: {v2_caps:?}"
        );
    }

    #[test]
    fn attach_meta_rides_beside_body() {
        let mut frame = json!({"id":1,"ok":true,"body":{"fingerprint":"b3:x","seq":0}});
        attach_meta(&mut frame, 412);
        assert_eq!(frame["meta"], json!({"duration_us": 412}));
        // meta is a sibling, never a body field
        assert_eq!(frame["body"], json!({"fingerprint":"b3:x","seq":0}));
    }

    #[test]
    fn attach_meta_rides_beside_error() {
        let mut frame = json!({"id":2,"ok":false,"error":{
            "code":"fingerprint_mismatch","recovery":"resync"}});
        attach_meta(&mut frame, u64::MAX);
        assert_eq!(frame["meta"]["duration_us"], json!(u64::MAX));
        assert_eq!(frame["error"]["code"], json!("fingerprint_mismatch"));
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
