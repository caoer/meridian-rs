//! Contract-rev negotiation gate (docs/wire-contract-v3-amendment.md), through
//! the LIVE serve loop. Two sessions, one hard rule: a v2 session emits `root`
//! and never `fingerprint`; a v3 session emits `fingerprint` and never `root`,
//! in every message class that carried it. No dual-emit within one rev.
//!
//! The frozen v2 goldens live in `dispatch_v2.rs` and `crates/wire/tests/
//! contract_v2.rs` — this file proves v2 is byte-identical to them AND that v3
//! is the fingerprint re-shaping, negotiated per session.

use serde_json::{Value, json};
use std::io::Write as _;

fn workspace(files: &[(&str, &str)]) -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    for (rel, bytes) in files {
        let abs = dir.path().join(rel);
        std::fs::create_dir_all(abs.parent().expect("parent")).expect("mkdir");
        let mut f = std::fs::File::create(&abs).expect("create");
        f.write_all(bytes.as_bytes()).expect("write");
    }
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    (dir, root)
}

fn wsfix(rel: &str) -> String {
    std::fs::read_to_string(testsuite::wsfix_dir().join(rel))
        .unwrap_or_else(|e| panic!("wsfix fixture {rel}: {e}"))
}

/// The §0.3 S0 workspace — the same fixture the v2 goldens ride.
fn s0() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let plan = wsfix("s0/notes/plan.md");
    let receipts = wsfix("s0/receipts/2026-07-18.md");
    workspace(&[
        ("notes/plan.md", &plan),
        ("receipts/2026-07-18.md", &receipts),
        (".github/README.md", "# CI notes\n"),
    ])
}

/// Raw serve output (the exact bytes a consumer reads).
fn serve_raw(root: &fs::WorkspaceRoot, input: &str) -> String {
    let mut out = Vec::new();
    sidecar::serve(root, input.as_bytes(), &mut out, &[]).expect("serve");
    String::from_utf8(out).expect("frames are UTF-8")
}

/// One serve session, `input` lines in, parsed frames out (order preserved) —
/// the negotiated rev is per-session, so a hello and its follow-ups must ride
/// ONE call.
fn serve(root: &fs::WorkspaceRoot, input: &str) -> Vec<Value> {
    serve_raw(root, input)
        .lines()
        .map(|l| serde_json::from_str(l).expect("frame parses"))
        .collect()
}

const R0: &str = "b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9";

/// Every key in a JSON value tree, recursively (for the no-dual-emit sweep).
fn all_keys(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(m) => {
            for (k, child) in m {
                out.push(k.clone());
                all_keys(child, out);
            }
        }
        Value::Array(a) => a.iter().for_each(|c| all_keys(c, out)),
        _ => {}
    }
}

fn frame_keys(v: &Value) -> Vec<String> {
    let mut out = Vec::new();
    all_keys(v, &mut out);
    out
}

// ---------------------------------------------------------------------------
// v2 session (no `contract`): root present, fingerprint NEVER, byte-identical
// ---------------------------------------------------------------------------

/// A v2 session (contract absent) is the frozen contract, unchanged: hello,
/// toc and root all carry `root` and no `fingerprint`/`contract` key — the raw
/// bytes match the pre-change goldens.
#[test]
fn v2_session_emits_root_never_fingerprint() {
    let (_d, root) = s0();
    let raw = serve_raw(
        &root,
        "{\"id\":1,\"op\":\"hello\",\"proto\":1}\n\
         {\"id\":2,\"op\":\"toc\",\"path\":\"notes/plan.md\"}\n\
         {\"id\":3,\"op\":\"root\"}\n",
    );
    assert!(
        !raw.contains("fingerprint"),
        "v2 session must never emit fingerprint: {raw}"
    );
    let frames = serve(&root, &raw_input_v2());
    // hello body: the frozen v2 shape exactly (no `contract` key added)
    assert_eq!(
        frames[0],
        json!({"id":1,"ok":true,"body":{
            "proto":1,"server":"meridian-sidecar/2.0",
            "caps":[
                "toc","cat","extract","resolve","resolve.content","links",
                "links.require_root","splice","splice.if_node_rev","splice.if_root",
                "splice.dry","splice.receipt","splice.verdicts","root","diff","sub"],
            "root":R0}})
    );
    assert!(!frame_keys(&frames[0]).contains(&"contract".to_string()));
    // toc + root carry the v2 `root` key
    assert_eq!(frames[1]["body"]["root"], json!(R0));
    assert_eq!(
        frames[2],
        json!({"id":3,"ok":true,"body":{"root":R0,"seq":0}})
    );
}

fn raw_input_v2() -> String {
    "{\"id\":1,\"op\":\"hello\",\"proto\":1}\n\
     {\"id\":2,\"op\":\"toc\",\"path\":\"notes/plan.md\"}\n\
     {\"id\":3,\"op\":\"root\"}\n"
        .to_string()
}

// ---------------------------------------------------------------------------
// v3 session (contract:"v3"): fingerprint present, root NEVER, every class
// ---------------------------------------------------------------------------

/// A v3 session re-shapes every fingerprint-bearing message: hello (body key +
/// caps + echoed `contract`), toc, the renamed `fingerprint` op, and a dry
/// splice's before/after — none of them spell `root`.
#[test]
fn v3_session_emits_fingerprint_never_root() {
    let (_d, root) = s0();
    let input = "{\"id\":1,\"op\":\"hello\",\"proto\":1,\"contract\":\"v3\"}\n\
         {\"id\":2,\"op\":\"toc\",\"path\":\"notes/plan.md\"}\n\
         {\"id\":3,\"op\":\"fingerprint\"}\n\
         {\"id\":4,\"op\":\"splice\",\"path\":\"notes/plan.md\",\"dry\":true,\
           \"edits\":[{\"target\":{\"hpath\":[{\"h\":\"Goals\"},{\"h\":\"Q3\"}]},\
           \"edit\":{\"match\":{\"old\":\"ship by August\",\"new\":\"ship by September\"}}}]}\n";
    let raw = serve_raw(&root, input);
    assert!(
        !raw.contains("\"root\""),
        "v3 session must never emit a `root` key: {raw}"
    );
    let frames = serve(&root, input);

    // hello: body key renamed, caps re-spelled, contract echoed
    let hello = &frames[0];
    assert_eq!(hello["body"]["fingerprint"], json!(R0));
    assert_eq!(hello["body"]["contract"], json!("v3"));
    assert!(hello["body"].as_object().unwrap().get("root").is_none());
    let caps: Vec<&str> = hello["body"]["caps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap())
        .collect();
    assert!(caps.contains(&"fingerprint"));
    assert!(caps.contains(&"splice.if_fingerprint"));
    assert!(caps.contains(&"links.require_fingerprint"));
    assert!(!caps.contains(&"root"));
    assert!(!caps.contains(&"splice.if_root"));
    assert!(!caps.contains(&"links.require_root"));

    // toc: the ambient key is fingerprint
    assert_eq!(frames[1]["body"]["fingerprint"], json!(R0));
    assert!(frames[1]["body"].as_object().unwrap().get("root").is_none());

    // the renamed op returns the fingerprint body (plus the U7 in-band timing
    // block — nondeterministic value, so it is asserted by shape and peeled
    // before the exact comparison)
    let mut cursor_frame = frames[2].clone();
    let meta = cursor_frame
        .as_object_mut()
        .unwrap()
        .remove("meta")
        .expect("v3 dispatch frames carry meta");
    assert!(meta["duration_us"].is_u64(), "meta carries µs: {meta}");
    assert_eq!(
        cursor_frame,
        json!({"id":3,"ok":true,"body":{"fingerprint":R0,"seq":0}})
    );

    // dry splice: before/after re-spelled, after literally null
    let splice = &frames[3]["body"];
    assert_eq!(splice["fingerprint_before"], json!(R0));
    assert!(
        splice
            .as_object()
            .unwrap()
            .contains_key("fingerprint_after")
    );
    assert_eq!(splice["fingerprint_after"], Value::Null);
    assert!(splice.as_object().unwrap().get("root_before").is_none());
    assert!(splice.as_object().unwrap().get("root_after").is_none());
}

/// v3 links: the §10.1 triple re-spelled (`as_of_fingerprint`/`live_fingerprint`),
/// and the `require_fingerprint` knob accepted on input — the corpus map keys
/// beneath are NEVER re-keyed.
#[test]
fn v3_links_triple_and_require_knob() {
    let (_d, root) = s0();
    let input = "{\"id\":1,\"op\":\"hello\",\"proto\":1,\"contract\":\"v3\"}\n\
         {\"id\":2,\"op\":\"links\",\"require_fingerprint\":\"{R0}\"}\n"
        .replace("{R0}", R0);
    let frames = serve(&root, &input);
    let links = &frames[1]["body"];
    assert_eq!(links["as_of_fingerprint"], json!(R0));
    assert_eq!(links["live_fingerprint"], json!(R0));
    assert!(links.as_object().unwrap().get("as_of_root").is_none());
    assert!(links.as_object().unwrap().get("live_root").is_none());
    // require_fingerprint met the world → a success view, not a stale_view refusal
    assert_eq!(frames[1]["ok"], json!(true));
}

/// v3 error codes follow the vocabulary: a failed `if_fingerprint` world guard
/// is `fingerprint_mismatch` (resync), and a diff over an unknown range is
/// `fingerprint_unknown` — the recovery class is unchanged, only the spelling.
#[test]
fn v3_error_codes_respell_root_family() {
    let (_d, root) = s0();
    let stale = "b3:0000000000000000000000000000000000000000000000000000000000000000";
    let other = "b3:1111111111111111111111111111111111111111111111111111111111111111";
    let input = format!(
        "{{\"id\":1,\"op\":\"hello\",\"proto\":1,\"contract\":\"v3\"}}\n\
         {{\"id\":2,\"op\":\"splice\",\"path\":\"notes/plan.md\",\"if_fingerprint\":\"{stale}\",\
           \"edits\":[{{\"target\":{{\"hpath\":[{{\"h\":\"Goals\"}},{{\"h\":\"Q3\"}}]}},\
           \"edit\":{{\"match\":{{\"old\":\"ship by August\",\"new\":\"x\"}}}}}}]}}\n\
         {{\"id\":3,\"op\":\"diff\",\"from_fingerprint\":\"{stale}\",\"to_fingerprint\":\"{other}\"}}\n"
    );
    let raw = serve_raw(&root, &input);
    assert!(!raw.contains("root_mismatch"), "code re-spelled: {raw}");
    assert!(!raw.contains("root_unknown"), "code re-spelled: {raw}");
    let frames = serve(&root, &input);

    assert_eq!(frames[1]["error"]["code"], json!("fingerprint_mismatch"));
    assert_eq!(frames[1]["error"]["recovery"], json!("resync"));
    // vocabulary-neutral extras keep their names
    assert!(frames[1]["error"]["expected"].is_string());

    assert_eq!(frames[2]["error"]["code"], json!("fingerprint_unknown"));
    assert_eq!(frames[2]["error"]["recovery"], json!("resync"));
}

// ---------------------------------------------------------------------------
// Unknown rev: loud typed error, never a silent fallback
// ---------------------------------------------------------------------------

/// An unknown declared rev is refused LOUD at hello — a `bad_request` (fix),
/// never a silent downgrade to v2.
#[test]
fn unknown_contract_rev_is_typed_error() {
    let (_d, root) = s0();
    let frames = serve(
        &root,
        "{\"id\":1,\"op\":\"hello\",\"proto\":1,\"contract\":\"v4\"}\n",
    );
    assert_eq!(frames[0]["ok"], json!(false));
    assert_eq!(frames[0]["error"]["code"], json!("bad_request"));
    assert_eq!(frames[0]["error"]["recovery"], json!("fix"));
    assert!(
        frames[0]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("unknown contract rev"),
        "{}",
        frames[0]
    );
}

// ---------------------------------------------------------------------------
// U7 in-band timing: v3 dispatch frames carry meta.duration_us, v2 never
// ---------------------------------------------------------------------------

/// A v3 session's dispatched frames — success AND refusal alike — carry the
/// in-band timing block `meta: {duration_us}` (integer µs, the sidecar
/// measure point is `arms::dispatch`). The body/error shape is untouched:
/// meta is a top-level sibling.
#[test]
fn v3_dispatch_frames_carry_meta_duration_us() {
    let (_d, root) = s0();
    let input = "{\"id\":1,\"op\":\"hello\",\"proto\":1,\"contract\":\"v3\"}\n\
         {\"id\":2,\"op\":\"toc\",\"path\":\"notes/plan.md\"}\n\
         {\"id\":3,\"op\":\"toc\",\"path\":\"no/such/file.md\"}\n";
    let frames = serve(&root, input);
    // hello + toc ride arms::dispatch → both timed
    for frame in &frames[..2] {
        assert!(
            frame["meta"]["duration_us"].is_u64(),
            "v3 dispatch frame carries meta.duration_us: {frame}"
        );
    }
    // a refusal is engine work too — the error frame is timed as well
    assert_eq!(frames[2]["ok"], json!(false));
    assert!(
        frames[2]["meta"]["duration_us"].is_u64(),
        "v3 error frame carries meta.duration_us: {}",
        frames[2]
    );
    // meta rides beside body/error, never inside them
    assert!(frames[1]["body"].as_object().unwrap().get("meta").is_none());
    assert!(
        frames[2]["error"]
            .as_object()
            .unwrap()
            .get("meta")
            .is_none()
    );
}

// ---------------------------------------------------------------------------
// U2 addressing facts: v3 extract enriches headings, v2 never
// ---------------------------------------------------------------------------

/// A v3 session's `extract` carries the host-face addressing facts on every
/// heading node — dewey `n`, sanitized `hpath_text`, subtree `words` — the
/// facts ccc-statusd re-derived host-side (U2). Non-heading nodes carry none.
#[test]
fn v3_extract_enriches_heading_nodes() {
    let (_d, root) = s0();
    let input = "{\"id\":1,\"op\":\"hello\",\"proto\":1,\"contract\":\"v3\"}\n\
         {\"id\":2,\"op\":\"extract\",\"path\":\"notes/plan.md\"}\n";
    let frames = serve(&root, input);
    let nodes = frames[1]["body"]["nodes"].as_array().expect("nodes");
    let headings: Vec<&Value> = nodes
        .iter()
        .filter(|n| n["kind"] == json!("heading"))
        .collect();
    assert_eq!(headings.len(), 3, "S0 plan has 3 headings");
    let facts: Vec<(&str, &str, u64)> = headings
        .iter()
        .map(|h| {
            (
                h["n"].as_str().expect("n"),
                h["hpath_text"].as_str().expect("hpath_text"),
                h["words"].as_u64().expect("words"),
            )
        })
        .collect();
    assert_eq!(
        facts,
        vec![
            ("1", "Goals", 20),
            ("1.1", "Goals/Q3", 3),
            ("1.2", "Goals/Q4", 10),
        ]
    );
    // non-heading nodes never carry the addressing keys
    for n in nodes.iter().filter(|n| n["kind"] != json!("heading")) {
        for key in ["n", "hpath_text", "words"] {
            assert!(
                n.get(key).is_none(),
                "non-heading node must not carry `{key}`: {n}"
            );
        }
    }
}

/// A v2 session's `extract` is the frozen shape: ZERO `n`/`hpath_text`/
/// `words` keys anywhere in the frame.
#[test]
fn v2_extract_never_carries_addressing_keys() {
    let (_d, root) = s0();
    let raw = serve_raw(
        &root,
        "{\"id\":1,\"op\":\"hello\",\"proto\":1}\n\
         {\"id\":2,\"op\":\"extract\",\"path\":\"notes/plan.md\"}\n",
    );
    for key in ["\"hpath_text\"", "\"words\"", "\"n\":"] {
        assert!(
            !raw.contains(key),
            "v2 extract must never emit {key}: {raw}"
        );
    }
}

/// A v2 session NEVER emits a `meta` key — the frozen contract is
/// byte-identical, and timing is a v3-only additive slot.
#[test]
fn v2_session_never_emits_meta() {
    let (_d, root) = s0();
    let raw = serve_raw(
        &root,
        "{\"id\":1,\"op\":\"hello\",\"proto\":1}\n\
         {\"id\":2,\"op\":\"toc\",\"path\":\"notes/plan.md\"}\n\
         {\"id\":3,\"op\":\"toc\",\"path\":\"no/such/file.md\"}\n",
    );
    assert!(
        !raw.contains("\"meta\""),
        "v2 must never emit a meta key: {raw}"
    );
    assert!(
        !raw.contains("duration_us"),
        "v2 must never emit duration_us: {raw}"
    );
}

/// `contract:"v2"` is explicitly the frozen vocabulary — same as absent.
#[test]
fn explicit_v2_contract_is_the_frozen_vocabulary() {
    let (_d, root) = s0();
    let frames = serve(
        &root,
        "{\"id\":1,\"op\":\"hello\",\"proto\":1,\"contract\":\"v2\"}\n{\"id\":2,\"op\":\"root\"}\n",
    );
    assert_eq!(
        frames[1],
        json!({"id":2,"ok":true,"body":{"root":R0,"seq":0}})
    );
}
