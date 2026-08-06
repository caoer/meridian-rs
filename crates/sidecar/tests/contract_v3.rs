//! Contract-rev negotiation (docs/wire-contract.md) via live serve.
//! v2 session: `root`, never `fingerprint`. v3 session: `fingerprint`, never
//! `root`. No dual-emit within one rev. Also pins U7 meta, U2 extract facts,
//! and frozen v2 extract key-set.

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

/// One serve session (rev is per-session — hello + follow-ups in one call).
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
// v2 session (no `contract`): root present, fingerprint never
// ---------------------------------------------------------------------------

/// v2 session = frozen contract: `root`, no `fingerprint`/`contract`.
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
// v3 session (contract:"v3"): fingerprint present, root never
// ---------------------------------------------------------------------------

/// v3 re-shapes fingerprint-bearing messages; none spell `root`.
#[test]
fn v3_session_emits_fingerprint_never_root() {
    let (_d, root) = s0();
    // U10: dry needs the section fingerprint too — from toc.
    let q3_rev = toc_node_rev(&root, "Q3");
    let input = format!(
        "{{\"id\":1,\"op\":\"hello\",\"proto\":1,\"contract\":\"v3\"}}\n\
         {{\"id\":2,\"op\":\"toc\",\"path\":\"notes/plan.md\"}}\n\
         {{\"id\":3,\"op\":\"fingerprint\"}}\n\
         {{\"id\":4,\"op\":\"splice\",\"path\":\"notes/plan.md\",\"dry\":true,\
           \"edits\":[{{\"target\":{{\"hpath\":[{{\"h\":\"Goals\"}},{{\"h\":\"Q3\"}}]}},\
           \"if_node_rev\":\"{q3_rev}\",\
           \"edit\":{{\"match\":{{\"old\":\"ship by August\",\"new\":\"ship by September\"}}}}}}]}}\n"
    );
    let input = input.as_str();
    let raw = serve_raw(&root, input);
    assert!(
        !raw.contains("\"root\""),
        "v3 session must never emit a `root` key: {raw}"
    );
    let frames = serve(&root, input);

    // hello: renamed key, re-spelled caps, echoed contract
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
    // v3 advertises splice.pin / plan_edits; frozen v2 caps must not.
    assert!(
        caps.contains(&"splice.pin"),
        "a v3 session honours `pin`, so its caps advertise `splice.pin`: {caps:?}"
    );
    assert!(caps.contains(&"splice.plan_edits"));

    assert_eq!(frames[1]["body"]["fingerprint"], json!(R0));
    assert!(frames[1]["body"].as_object().unwrap().get("root").is_none());

    // Renamed op + U7 meta (shape only; peel before exact compare).
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

    // dry splice: before/after re-spelled; after null
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

/// v3 links: §10.1 triple re-spelled; map keys never re-keyed.
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

/// v3 error codes re-spell the root family; recovery class unchanged.
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

/// Unknown contract rev → `bad_request` (fix), never silent v2 fallback.
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

/// v3 dispatch frames (ok and error) carry top-level `meta.duration_us`.
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

/// v3 extract: headings carry `n`/`words`/`hpath` (segments); no `hpath_text`
/// (U14: no string address forms on machine surfaces). Non-headings carry none.
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
    let facts: Vec<(&str, Value, u64)> = headings
        .iter()
        .map(|h| {
            (
                h["n"].as_str().expect("n"),
                h["hpath"].clone(),
                h["words"].as_u64().expect("words"),
            )
        })
        .collect();
    assert_eq!(
        facts,
        vec![
            ("1", json!([{"h": "Goals"}]), 20),
            ("1.1", json!([{"h": "Goals"}, {"h": "Q3"}]), 3),
            ("1.2", json!([{"h": "Goals"}, {"h": "Q4"}]), 10),
        ],
        "the address is the SEGMENT array (U14), never a joined string"
    );
    // U14: no joined address on this face.
    for h in &headings {
        assert!(
            h.get("hpath_text").is_none(),
            "ZT decision 14 / U14: `hpath_text` is retired from the machine \
             surface — the node already carries `hpath` as segments. Restoring \
             it needs a ruling, not a field: {h}"
        );
    }
    for n in nodes.iter().filter(|n| n["kind"] != json!("heading")) {
        for key in ["n", "words"] {
            assert!(
                n.get(key).is_none(),
                "non-heading node must not carry `{key}`: {n}"
            );
        }
    }
}

/// Composed read accepts `actor` (§9); serves normally (no receipt mint yet).
#[test]
fn v3_read_carries_the_derived_actor() {
    let (_d, root) = s0();
    let input = "{\"id\":1,\"op\":\"hello\",\"proto\":1,\"contract\":\"v3\"}\n\
         {\"id\":2,\"op\":\"read\",\"path\":\"notes/plan.md\",\"actor\":\"agent:b0864fb2\"}\n";
    let frames = serve(&root, input);
    assert_eq!(
        frames[1]["ok"],
        json!(true),
        "an actor-stamped read serves: {}",
        frames[1]
    );
    assert!(
        frames[1]["body"]["rendered_text"].is_string(),
        "the composed body rides: {}",
        frames[1]
    );
}

/// v2 extract: zero addressing keys (list includes retired `hpath_text`).
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

/// Frozen v2 extract node key set — exact allow-list (not subset). Catches
/// v3-additive fields on a v2 session; `skip_serializing_if` is not a version
/// gate.
#[test]
fn a_frozen_v2_extract_node_never_grows_a_field() {
    let (_d, root) = s0();
    let input = "{\"id\":1,\"op\":\"hello\",\"proto\":1}\n\
         {\"id\":2,\"op\":\"extract\",\"path\":\"notes/plan.md\"}\n";
    let frames = serve(&root, input);
    let nodes = frames[1]["body"]["nodes"].as_array().expect("nodes");
    assert!(!nodes.is_empty(), "the fixture must produce nodes");
    for n in nodes {
        let mut got: Vec<&str> = n
            .as_object()
            .expect("a node is an object")
            .keys()
            .map(String::as_str)
            .collect();
        got.sort_unstable();
        // Frozen v2 §5.2 node keys only.
        let allowed = [
            "hpath",
            "info",
            "kind",
            "node_rev",
            "span",
            "text_prefix_16b",
            "unterminated",
        ];
        for key in &got {
            assert!(
                allowed.contains(key),
                "a v2 `extract` node grew the key `{key}` — the frozen §5.2 shape \
                 admits only {allowed:?}. `skip_serializing_if` does not gate on \
                 session version; gate at the projection seam. Node: {n}"
            );
        }
    }
}

/// v2 session never emits `meta` / `duration_us`.
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

/// `contract:"v2"` ≡ absent (frozen vocabulary).
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

/// Live `node_rev` of a heading from toc (U10 fingerprint for wire writes).
fn toc_node_rev(root: &fs::WorkspaceRoot, heading: &str) -> String {
    let frames = serve(
        root,
        "{\"id\":1,\"op\":\"hello\",\"proto\":1,\"contract\":\"v3\"}\n\
         {\"id\":2,\"op\":\"toc\",\"path\":\"notes/plan.md\"}\n",
    );
    frames[1]["body"]["nodes"]
        .as_array()
        .expect("toc nodes")
        .iter()
        .find(|n| {
            n["hpath"]
                .as_array()
                .and_then(|h| h.last())
                .is_some_and(|seg| seg["h"] == json!(heading))
        })
        .unwrap_or_else(|| panic!("heading {heading} in toc: {:?}", frames[1]))["node_rev"]
        .as_str()
        .expect("node_rev")
        .to_string()
}
