//! Contract-rev negotiation + v3 vocabulary projection (one implementation for
//! the serving host). Frozen v2 wire types stay byte-for-byte; v3 is envelope re-key
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
                caps.push(Value::String("splice.plan_edits".to_string()));
                caps.push(Value::String("splice.pin".to_string()));
                // Cross-root pin (design D-E): `splice.pin.target` admits the
                // ruled `name:rel` spelling. The face keeps its own
                // `pin_cross_root` refusal until this cap is present, so an
                // old engine refuses with today's teaching instead of a
                // confusing `pin_target_missing` from a spelling it cannot
                // parse.
                caps.push(Value::String("pin-cross-root".to_string()));
                // Pin proof rides the request (§ A.3 proof law): `splice.pin`
                // takes `fingerprint` (required for session actors) +
                // optional `sec_rev`, sections reads serve a `fingerprint`
                // per row, and no server-side read state exists.
                caps.push(Value::String("splice.pin.proof".to_string()));
                // The §4.4 set form: `files[]` on `splice`, sealed across the
                // set (validate-all-then-apply, one fingerprint advance).
                caps.push(Value::String("splice.set".to_string()));
                // The middleware door (§ A.2.1): the `fields` passthrough on
                // splice + create, and `armed.intents` on their responses.
                caps.push(Value::String("splice.fields".to_string()));
                // Law A-1 at the create door: the plan `create` row honors a
                // parent-section `rev` (§ A.3).
                caps.push(Value::String("splice.create_rev".to_string()));
                // Birth op at op grain (no dotted create.<field>).
                caps.push(Value::String("create".to_string()));
                // Death op at op grain (§ A.3 remove door; no dotted
                // remove.<field>, and no force field exists on the op).
                caps.push(Value::String("remove".to_string()));
                // Mount-table discovery at op grain (§ A.5, the create
                // precedent); the frozen v2 caps stay byte-identical.
                caps.push(Value::String("mounts".to_string()));
                // Field-only amendment, dotted per § A.2: the mounts row
                // carries the declared-primary designation (§ A.5).
                caps.push(Value::String("mounts.primary".to_string()));
                caps.push(Value::String("hello.identity".to_string()));
                // In-process script submission at op grain (§ A.7, the same
                // precedent — no dotted script.<field> at birth).
                caps.push(Value::String("script".to_string()));
                // Page-task execution at op grain (§ A.8, the same
                // precedent — no dotted run.<field> at birth).
                caps.push(Value::String("run".to_string()));
                // Pin-graph context assembly at op grain (§ A.10, the same
                // precedent — no dotted walk.<field> at birth).
                caps.push(Value::String("walk".to_string()));
                // Corpus SQL at op grain (§ A.11, the same precedent — no
                // dotted sql.<field> at birth).
                caps.push(Value::String("sql".to_string()));
                // The scoped-premise family, LAST and family-whole (§3.2 /
                // §5.4): decode, coverage, scoped fold, mint arm, refusal
                // `scope` are all served when this string is present.
                caps.push(Value::String("scoped-guards".to_string()));
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

/// Demote a response for a frozen v2 session: drop the v3-additive error extras
/// a v2 caller was never promised. Returns `None` when the frame already carries
/// none, so the frozen path serializes the typed value directly and stays
/// byte-identical.
///
/// Two propositions, deliberately not merged:
///
/// - Vintage — `rung`, `diff`, `new_content`, `new_fingerprint` postdate frozen
///   v2, so they are rows in [`V2_RESERVED_FIELDS`].
/// - Provenance — `message` and `path` are v2-legal fields; only the ladder's
///   are stripped, keyed on `rung` as the ladder's authorship mark.
#[must_use]
pub fn demote_v2(response: &wire::Response) -> Option<wire::Response> {
    let error = match &response.payload {
        wire::ResponsePayload::Error { error } => error,
        wire::ResponsePayload::Body { body } => {
            return demote_body_v2(body).map(|body| wire::Response {
                id: response.id,
                ok: response.ok,
                payload: wire::ResponsePayload::Body { body },
            });
        }
    };
    // Provenance: only the ladder's `message`/`path` are its to strip.
    let ladder_authored = error.rung.is_some();
    // Vintage: does this envelope carry any field the registry says postdates v2?
    let post_v2 = [
        ("rung", error.rung.is_some()),
        ("diff", error.diff.is_some()),
        ("new_content", error.new_content.is_some()),
        ("new_fingerprint", error.new_fingerprint.is_some()),
        ("scope", error.scope.is_some()),
        ("uncovered", error.uncovered.is_some()),
    ]
    .iter()
    .any(|(key, present)| *present && is_reserved(key, Position::ErrorPayload));
    if !ladder_authored && !post_v2 {
        return None; // nothing a v2 session may not see
    }
    let mut demoted = response.clone();
    if let wire::ResponsePayload::Error { error } = &mut demoted.payload {
        if is_reserved("rung", Position::ErrorPayload) {
            error.rung = None;
        }
        if is_reserved("diff", Position::ErrorPayload) {
            error.diff = None;
        }
        if is_reserved("new_content", Position::ErrorPayload) {
            error.new_content = None;
        }
        if is_reserved("new_fingerprint", Position::ErrorPayload) {
            error.new_fingerprint = None;
        }
        if is_reserved("scope", Position::ErrorPayload) {
            error.scope = None;
        }
        if is_reserved("uncovered", Position::ErrorPayload) {
            error.uncovered = None;
        }
        if ladder_authored {
            error.message = None;
            error.path = None;
        }
    }
    Some(demoted)
}

/// Attach `meta: {duration_us}` beside body/error (v3-only by caller).
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

    /// Every v3-era splice field amendment advertised as `splice.<field>`,
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
            let want = match *field {
                // `files` is the §4.4 SET FORM — a form amendment riding one
                // field, advertised under its ruled name `splice.set` (Draft A,
                // 2026-08-14), not the mechanical `splice.files`.
                "files" => "splice.set".to_owned(),
                // `scope` + `guards` are the §5.4 family — one flag, not
                // dotted field caps (ruling item 6; contract §3.2).
                "scope" | "guards" => "scoped-guards".to_owned(),
                other => format!("splice.{other}"),
            };
            assert!(
                caps.contains(&want.as_str()),
                "v3-era splice field `{field}` is honoured by the decoder but \
                 NOT advertised: the v3 caps projection must push `{want}` \
                 (R23 — a field-only amendment ships as a dotted `op.field` \
                 cap; the set form's ruled cap name is `splice.set`). \
                 caps = {caps:?}"
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
                "pin-cross-root",
                "splice.pin.proof",
                "splice.set",
                "splice.fields",
                "splice.create_rev",
                "create",
                "remove",
                "mounts",
                "mounts.primary",
                "hello.identity",
                "script",
                "run",
                "walk",
                "sql",
                "scoped-guards"
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

// ---------------------------------------------------------------------------
// The v2-reserved-field registry (v2 demotion law)
// ---------------------------------------------------------------------------

/// Where in a frame a reserved field appears. A key is stripped at its declared
/// position only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Position {
    /// The root of a Notification frame (`{"delta":…}`, no `id`).
    NotificationRoot,
    /// The root of a response frame.
    ResponseRoot,
    /// Inside a response's `error` payload.
    ErrorPayload,
    /// Inside a splice response's `body.armed` fact.
    ResponseArmed,
}

/// One field that postdates frozen v2 and must never reach a v2 session.
#[derive(Debug, Clone, Copy)]
pub struct ReservedField {
    /// The serialized key.
    pub key: &'static str,
    /// Where it appears.
    pub position: Position,
    /// Who added it.
    pub author: &'static str,
    /// Why it postdates v2, in one line.
    pub why: &'static str,
}

/// The table — append-only. Every field that postdates frozen v2, in one
/// place, consulted by the v2 projection at the serving host (Law 3: one
/// implementation, one host).
///
/// # The discipline
/// A new v3-additive field ships with its row in the same commit. The key-set
/// pins (`u20b_v2_notification_shape.rs` and the ladder's frozen frame) assert
/// an exact key set, so a field that forgets its row fails loudly; a
/// value-pinning sweep cannot see this class.
///
/// # Vintage, not provenance
/// A row asserts a field vintage: this key postdates frozen v2, regardless of
/// who wrote the value. Provenance is per-envelope and stays in `demote_v2` —
/// `message` and `path` are v2-legal fields, so a row for them would strip
/// legitimate messages from every non-ladder refusal.
pub const V2_RESERVED_FIELDS: &[ReservedField] = &[
    ReservedField {
        key: "effects",
        position: Position::NotificationRoot,
        author: "C3 reaction plane",
        why: "the reaction sibling of the frozen §7.1 Delta; `skip_serializing_if` \
              skips on an EMPTY VALUE, never on a v2 SESSION, so a fired HOOK put \
              it on a v2 wire",
    },
    ReservedField {
        key: "effects",
        position: Position::ResponseArmed,
        author: "C3 reaction plane",
        why: "the SAME producer as the notification-side row — `write.rs` fills \
              one `armed_effects` and writes it to BOTH the ring frame and the \
              response body, so one field leaked at two exits; absent from the \
              frozen §4.4/§5.2 armed key set {path, edits}",
    },
    ReservedField {
        key: "rescope",
        position: Position::NotificationRoot,
        author: "§ A.9 re-scope honesty",
        why: "the re-scope batch summary (cause + membership counts); frozen \
              v2 enumerated a re-scope as per-file rows and never carried a \
              summary",
    },
    ReservedField {
        key: "overflow",
        position: Position::NotificationRoot,
        author: "§ A.9 re-scope honesty",
        why: "the assembly-bound overflow marker; no frozen v2 frame bounded \
              its own enumeration",
    },
    // The scoped-guard family's two error extras (§5.5/§5.7, amended
    // 2026-08-15). `scope` is v3-only-reachable (premises are cap-gated);
    // `uncovered` can fire for a v2 mixed batch (A.1 amendment), where the
    // §8.2 message names the same set — the demotion costs no fact.
    ReservedField {
        key: "scope",
        position: Position::ErrorPayload,
        author: "§5.4 scoped-guard family",
        why: "the scoped premise's scope on fingerprint_mismatch and \
              scope_unresolved; no frozen v2 refusal carries one",
    },
    ReservedField {
        key: "uncovered",
        position: Position::ErrorPayload,
        author: "§5.5 Coverage Law",
        why: "the uncovered caller-authored target set on \
              scope_does_not_cover; postdates frozen v2 — the register-law \
              message names the same set for demoted sessions",
    },
    // The ladder's four post-v2 slots; `message`/`path` are deliberately absent.
    ReservedField {
        key: "rung",
        position: Position::ErrorPayload,
        author: "U11 mismatch-recovery ladder",
        why: "the ladder's rung number; no frozen v2 refusal prints one",
    },
    ReservedField {
        key: "diff",
        position: Position::ErrorPayload,
        author: "U11 mismatch-recovery ladder",
        why: "rung-1 rendered diff; v3-additive recovery vocabulary",
    },
    ReservedField {
        key: "new_content",
        position: Position::ErrorPayload,
        author: "U11 mismatch-recovery ladder",
        why: "rung-2 current content; v3-additive recovery vocabulary",
    },
    ReservedField {
        key: "new_fingerprint",
        position: Position::ErrorPayload,
        author: "U11 mismatch-recovery ladder",
        why: "the fingerprint to retry against; v3-additive recovery vocabulary",
    },
];

/// Is `key` reserved at `position` — i.e. must a v2 session never see it?
#[must_use]
pub fn is_reserved(key: &str, position: Position) -> bool {
    V2_RESERVED_FIELDS
        .iter()
        .any(|field| field.key == key && field.position == position)
}

/// The BODY half of the v2 demotion: strip post-v2 fields from a response body,
/// or `None` when it carries none. Typed, like every other strip here — a
/// `Value` pass would alphabetize the key set (`preserve_order` is off).
fn demote_body_v2(body: &wire::ResponseBody) -> Option<wire::ResponseBody> {
    let wire::ResponseBody::Splice { armed, .. } = body else {
        return None;
    };
    if armed.effects.is_empty() || !is_reserved("effects", Position::ResponseArmed) {
        return None;
    }
    let mut demoted = body.clone();
    if let wire::ResponseBody::Splice { armed, .. } = &mut demoted {
        armed.effects.clear();
    }
    Some(demoted)
}
