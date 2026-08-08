//! The § A.6 value-plane WRITE law, proved at the real doors — wire-contract
//! § A.6.3 (the encoder), § A.6.3a (the two write doors), § A.6.3b (what the
//! splice consumer reads).
//!
//! Every assertion here is on the STORED LINE, byte for byte, and only then on
//! the round trip (§ A.6.4). The engine's own decode is tolerant by design, so
//! a value-only assertion passes over bytes no external parser accepts —
//! `note:""` round-trips through this engine and voids the frontmatter block
//! for yaml.v3, `PyYAML`, Obsidian and `ccc-cli` alike. That escape hatch is what
//! the A.6.3b defect hid behind, so the empty-value matrix below also parses
//! the result with `PyYAML`: a foreign parser, never the engine's own.

use serde_json::{Value, json};

/// Drive one v3 serve session over `doc_dir`; one frame per line.
fn serve(doc_dir: &std::path::Path, requests: &[Value]) -> Vec<Value> {
    let mut input = crate::daemon_door::hello_line(Some("v3"), doc_dir);
    for r in requests {
        input.push_str(&serde_json::to_string(r).expect("request serializes"));
        input.push('\n');
    }
    crate::daemon_door::serve_frames(&input)
}

/// A workspace holding one seeded file, owned by the test.
fn ws(rel: &str, body: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path().join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).expect("mkdir");
    }
    std::fs::write(p, body).expect("seed");
    dir
}

/// The file-grain guard token a write threads — minted by the read the door
/// itself teaches a caller to make.
fn file_rev(dir: &std::path::Path, path: &str) -> String {
    let frames = serve(dir, &[json!({"id":1,"op":"read","path":path})]);
    assert_eq!(frames[1]["ok"], json!(true), "the minting read serves");
    frames[1]["body"]["file_rev"]
        .as_str()
        .expect("file_rev")
        .to_owned()
}

/// `props[].value` as a key→value list off one composed read — the decoded
/// plane, used here only for the round-trip half of each assertion.
fn read_props(dir: &std::path::Path, path: &str) -> Vec<(String, String)> {
    let frames = serve(dir, &[json!({"id":1,"op":"read","path":path})]);
    assert_eq!(frames[1]["ok"], json!(true), "read serves: {}", frames[1]);
    frames[1]["body"]["props"]
        .as_array()
        .expect("props plane")
        .iter()
        .map(|p| {
            (
                p["key"].as_str().expect("key").to_owned(),
                p["value"].as_str().expect("value").to_owned(),
            )
        })
        .collect()
}

/// One splice carrying one `set_property` — the plan-edit door.
fn set_property(dir: &std::path::Path, path: &str, key: &str, value: &str) -> Value {
    let rev = file_rev(dir, path);
    let frames = serve(
        dir,
        &[json!({
            "id":2,"op":"splice","path":path,
            "actor":"agent:fuse0001","now":"2026-08-08T12:00:00Z",
            "plan_edits":[{"set_property":{"key":key,"value":value,"rev":rev}}],
        })],
    );
    frames[1].clone()
}

/// The node-grain token for one frontmatter key, or `None` when the key is
/// absent — the guard a wire write over EXISTING content must carry.
fn prop_rev(dir: &std::path::Path, path: &str, key: &str) -> Option<String> {
    let frames = serve(dir, &[json!({"id":1,"op":"read","path":path})]);
    assert_eq!(frames[1]["ok"], json!(true), "the minting read serves");
    frames[1]["body"]["props"]
        .as_array()
        .expect("props plane")
        .iter()
        .find(|p| p["key"].as_str() == Some(key))
        .map(|p| {
            p["prop_rev"]
                .as_str()
                .expect("prop_rev on the row")
                .to_owned()
        })
}

/// One splice carrying one native `put{at:"upsert"}` on an `fm_key` target —
/// the OTHER write door (§ A.6.3a), reached with no plan-edit lowering in
/// between. An existing key carries its node-grain guard; an absent one has no
/// content to guard.
fn put_upsert(dir: &std::path::Path, path: &str, key: &str, value: &str) -> Value {
    let mut edit = json!({
        "target":{"fm_key":key},
        "edit":{"put":{"at":"upsert","text":value}},
    });
    if let Some(rev) = prop_rev(dir, path, key) {
        edit["if_node_rev"] = json!(rev);
    }
    let frames = serve(
        dir,
        &[json!({
            "id":2,"op":"splice","path":path,
            "actor":"agent:fuse0001","now":"2026-08-08T12:00:00Z",
            "edits":[edit],
        })],
    );
    frames[1].clone()
}

const SEED: &str = "---\nseed: x\n---\n# Body\n\ntext\n";

/// The canonical emission table — one row per § A.6.3 class, asserted as the
/// stored LINE and then round-tripped.
///
/// (key, caller value, stored line)
const TABLE: &[(&str, &str, &str)] = &[
    ("reviewer", "[[1ed98864]]", "reviewer: \"[[1ed98864]]\""),
    ("owner", "", "owner: \"\""),
    ("note", "review: pending", "note: \"review: pending\""),
    ("plain", "doing", "plain: doing"),
    ("stamp", "2026-08-07", "stamp: 2026-08-07"),
    ("tags", "[type/a, topic/b]", "tags: [type/a, topic/b]"),
];

/// WRITE HALF, `set_property` door — canonical bytes on disk, then the round
/// trip through the props plane.
#[test]
fn set_property_emits_canonical_bytes_and_round_trips() {
    for (key, value, stored_line) in TABLE {
        let dir = ws("cards/one.md", SEED);
        let reply = set_property(dir.path(), "cards/one.md", key, value);
        assert_eq!(
            reply["ok"],
            json!(true),
            "set_property {key}={value:?} lands — what the fleet can read, the engine can write: {reply}"
        );
        let disk = std::fs::read_to_string(dir.path().join("cards/one.md")).expect("read back");
        assert!(
            disk.lines().any(|l| l == *stored_line),
            "the STORED LINE for {key}={value:?} is {stored_line:?}; file:\n{disk}"
        );
        assert_round_trip(dir.path(), "cards/one.md", key, value);
    }
}

/// WRITE HALF, the native `put{at:"upsert"}` door (§ A.6.3a) — the same table
/// through the OTHER door, because two write doors that disagree are the
/// defect this clause closes.
///
/// On the unfused base this door passed the caller's string through raw:
/// `[[1ed98864]]` landed as a nested flow sequence and the I4 substrate law
/// refused the write, so the wire could not write the value its own read seam
/// decodes.
#[test]
fn put_upsert_emits_the_same_canonical_bytes() {
    for (key, value, stored_line) in TABLE {
        let dir = ws("cards/one.md", SEED);
        let reply = put_upsert(dir.path(), "cards/one.md", key, value);
        assert_eq!(
            reply["ok"],
            json!(true),
            "put at:upsert {key}={value:?} lands: {reply}"
        );
        let disk = std::fs::read_to_string(dir.path().join("cards/one.md")).expect("read back");
        assert!(
            disk.lines().any(|l| l == *stored_line),
            "the two write doors emit ONE grammar — {key}={value:?} must store \
             {stored_line:?}; file:\n{disk}"
        );
        assert_round_trip(dir.path(), "cards/one.md", key, value);
    }
}

/// Both write doors refuse a multi-line value — uniform refusal at every
/// value-plane door (§ A.6.3a). A newline is refused, never sanitized.
#[test]
fn both_write_doors_refuse_a_multi_line_value() {
    let dir = ws("cards/one.md", SEED);
    let reply = set_property(dir.path(), "cards/one.md", "note", "one\ntwo");
    assert_eq!(
        reply["ok"],
        json!(false),
        "set_property refuses a newline: {reply}"
    );
    let dir = ws("cards/one.md", SEED);
    let reply = put_upsert(dir.path(), "cards/one.md", "note", "one\ntwo");
    assert_eq!(
        reply["ok"],
        json!(false),
        "the upsert door refuses a newline the same way: {reply}"
    );
    let disk = std::fs::read_to_string(dir.path().join("cards/one.md")).expect("read back");
    assert_eq!(disk, SEED, "a refused write leaves the bytes alone");
}

/// The season-1 finding-2 value verbatim: the fleet's own canonical `owner` —
/// what `ccc-cli task claim` writes — is writable through both doors.
#[test]
fn the_fleet_canonical_owner_value_is_writable() {
    for (door, write) in [
        (
            "set_property",
            set_property as fn(&std::path::Path, &str, &str, &str) -> Value,
        ),
        ("put at:upsert", put_upsert),
    ] {
        let dir = ws("cards/one.md", SEED);
        let reply = write(dir.path(), "cards/one.md", "owner", "[[b1892b5a]]");
        assert_eq!(
            reply["ok"],
            json!(true),
            "{door} writes the value the read seam decodes: {reply}"
        );
        let disk = std::fs::read_to_string(dir.path().join("cards/one.md")).expect("read back");
        assert!(
            disk.contains("owner: \"[[b1892b5a]]\""),
            "{door}: two writers, one value grammar — the emit matches \
             `ccc-cli task claim`'s:\n{disk}"
        );
    }
}

// ── § A.6.3b — the empty-value matrix, the P0 ────────────────────────────────

/// The three colon shapes a stored key can be in when an empty value is
/// written over it. Each is a different splice path: shapes 1 and 2 take the
/// UPDATE path (the separator guard), shape 3 takes the CREATE path.
///
/// (name, seed file, the line the write must store)
const COLON_SHAPES: &[(&str, &str, &str)] = &[
    (
        "valued key",
        "---\nnote: something\nother: keep\n---\n# Body\n",
        "note: \"\"",
    ),
    (
        "bare key",
        "---\nnote:\nother: keep\n---\n# Body\n",
        "note: \"\"",
    ),
    (
        "absent key",
        "---\nother: keep\n---\n# Body\n",
        "note: \"\"",
    ),
];

/// The same matrix through the OTHER write door — the upsert door reaches the
/// same splice consumer, so it carries the same guarantee.
#[test]
fn the_upsert_door_stores_one_line_shape_on_every_colon_shape() {
    for (name, seed, stored_line) in COLON_SHAPES {
        let dir = ws("cards/one.md", seed);
        for pass in 1..=2 {
            let reply = put_upsert(dir.path(), "cards/one.md", "note", "");
            assert_eq!(
                reply["ok"],
                json!(true),
                "{name}, pass {pass}: the empty upsert lands: {reply}"
            );
            let disk = std::fs::read_to_string(dir.path().join("cards/one.md")).expect("read back");
            assert!(
                disk.lines().any(|l| l == *stored_line),
                "{name}, pass {pass}: the upsert door stores {stored_line:?}; file:\n{disk}"
            );
            assert_foreign_parse(&disk, &[("note", ""), ("other", "keep")], name, pass);
        }
    }
}

/// **The malformed-quoting canary — the read half this fuse deliberately KEPT.**
///
/// § A.6.1 unquotes a value only when it is a WELL-FORMED quoted scalar, and
/// serves everything else verbatim: *malformed quoting, which no reader may
/// guess at*. `owner: "zt" # is "them"` is that class — a stored line whose
/// interior carries an unescaped `"`.
///
/// A looser decoder (strip the outer quote bytes, no well-formedness check, no
/// comment rule) mints `zt" # is "them` — a string that appears in neither the
/// stored bytes nor any parser's reading of them. This engine serves the value
/// decoded to ITSELF instead. The review gate named the stricter half as the
/// one to keep (2026-08-08); this pins it so no later merge loosens it
/// silently.
#[test]
fn malformed_quoting_decodes_to_itself() {
    const STORED: &str = r#""zt" # is "them""#;
    let dir = ws(
        "cards/one.md",
        &format!("---\nowner: {STORED}\n---\n# Body\n\ntext\n"),
    );
    let kv = read_props(dir.path(), "cards/one.md");
    assert_eq!(
        kv,
        vec![("owner".to_owned(), STORED.to_owned())],
        "malformed quoting is served verbatim — a reader that guessed would \
         forge a string present in neither the bytes nor any parser's reading"
    );
}

// ── helpers ─────────────────────────────────────────────────────────────────

/// The round-trip half: the props plane serves back exactly the caller's
/// string. Asserted only AFTER the stored line, never instead of it.
fn assert_round_trip(dir: &std::path::Path, path: &str, key: &str, value: &str) {
    let kv = read_props(dir, path);
    let served = &kv
        .iter()
        .find(|(k, _)| k == key)
        .unwrap_or_else(|| panic!("{key} served on the props plane"))
        .1;
    assert_eq!(
        served, value,
        "round trip: decode(the emitted line) is the caller's string for {key}"
    );
}

/// Parse the file's frontmatter with an EXTERNAL YAML parser (`PyYAML`) and
/// assert the expected pairs. The engine's own decode is tolerant, so only a
/// foreign parser can answer the question this law is actually about: does the
/// rest of the fleet still read this file?
pub(crate) fn assert_foreign_parse(disk: &str, expected: &[(&str, &str)], name: &str, pass: u32) {
    let Some(fm) = disk
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---").map(|(fm, _)| fm))
    else {
        panic!("{name}, pass {pass}: no frontmatter block to parse:\n{disk}");
    };
    let out = std::process::Command::new("python3")
        .args(["-c", PY_PARSE])
        .arg(fm)
        .output()
        .expect("python3 runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{name}, pass {pass}: PyYAML rejects the frontmatter the engine wrote — \
         the block is void for every reader that is not this engine.\n\
         stderr: {}\nblock:\n{fm}",
        String::from_utf8_lossy(&out.stderr)
    );
    for (k, v) in expected {
        let want = format!("{k}={v}\n");
        assert!(
            stdout.contains(&want),
            "{name}, pass {pass}: PyYAML reads {k:?} as {v:?}; it read:\n{stdout}block:\n{fm}"
        );
    }
}

/// Parse argv[1] as a YAML mapping and print `key=value` per entry. A `None`
/// value (the forged null) prints as `key=<null>`, never as the empty string,
/// so a null cannot pass for an empty string here.
const PY_PARSE: &str = r#"
import sys, yaml
d = yaml.safe_load(sys.argv[1])
if not isinstance(d, dict):
    sys.exit("frontmatter is not a mapping: %r" % (d,))
for k, v in d.items():
    print("%s=%s" % (k, "<null>" if v is None else v))
"#;
