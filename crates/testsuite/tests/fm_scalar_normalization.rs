//! Frontmatter scalar normalization — the § A.3 value-plane law, proved at
//! the wire door (wire-contract § A.3, docs-first 2026-08-07; dogfood-season-1
//! findings 1–2, R43).
//!
//! Two halves, one defect: the read plane must serve DECODED scalars (no
//! quote bytes), and the write plane must emit the fleet-canonical form (an
//! `[[id]]`-shaped value lands double-quoted instead of re-parsing as a
//! nested flow sequence and refusing). The regression table is the measured
//! season-1 finding-1 table verbatim, plus the write-side round trip.

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

/// `props[].value` as a key→value list off one composed read.
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

/// READ HALF — the season-1 finding-1 table, decoded on the props plane.
///
/// On the unfixed base every quoted row serves its quote bytes (`owner: ""`
/// is length 2), the silent fail-INERT face: a comparison against the
/// unquoted form is false and no rule arms.
#[test]
fn props_serve_decoded_scalars_the_season_1_table() {
    let dir = ws(
        "cards/one.md",
        concat!(
            "---\n",
            "owner: \"\"\n",
            "quoted_hex: \"3f9a1c07\"\n",
            "link: \"[[1ed98864]]\"\n",
            "status: doing\n",
            "single: 'it''s'\n",
            "escaped: \"a\\\"b\\\\c\"\n",
            "---\n",
            "# Body\n",
            "\n",
            "text\n"
        ),
    );
    let kv = read_props(dir.path(), "cards/one.md");
    assert_eq!(
        kv,
        vec![
            ("owner".to_owned(), String::new()),
            ("quoted_hex".to_owned(), "3f9a1c07".to_owned()),
            ("link".to_owned(), "[[1ed98864]]".to_owned()),
            ("status".to_owned(), "doing".to_owned()),
            ("single".to_owned(), "it's".to_owned()),
            ("escaped".to_owned(), "a\"b\\c".to_owned()),
        ],
        "the value plane serves decoded scalars — no quote bytes reach a consumer"
    );
}

/// One splice carrying one `set_property`, against `cards/one.md` in `dir` —
/// the honest call pattern: a read mints the file-grain token, the write
/// threads it as `rev` (the guard the door itself teaches).
fn set_property(dir: &std::path::Path, key: &str, value: &str) -> Value {
    let frames = serve(
        dir,
        &[json!({"id":1,"op":"read","path":"cards/one.md"})],
    );
    assert_eq!(frames[1]["ok"], json!(true), "the minting read serves");
    let rev = frames[1]["body"]["file_rev"]
        .as_str()
        .expect("file_rev")
        .to_owned();
    let frames = serve(
        dir,
        &[json!({
            "id":2,"op":"splice","path":"cards/one.md",
            "actor":"agent:d5654f18","now":"2026-08-07T12:00:00Z",
            "plan_edits":[{"set_property":{"key":key,"value":value,"rev":rev}}],
        })],
    );
    frames[1].clone()
}

const SEED: &str = "---\nseed: x\n---\n# Body\n\ntext\n";

/// WRITE HALF + ROUND TRIP — a `set_property` of each fleet-canonical class
/// lands, emits the canonical bytes, and reads back as the caller's string.
///
/// On the unfixed base the `[[id]]`-shaped value either refuses (nested
/// frontmatter, the measured fail-CLOSED) or lands unquoted — both fail the
/// stored-bytes assertion.
#[test]
fn set_property_emits_fleet_canonical_bytes_and_round_trips() {
    // (caller value, stored key line)
    let cases: &[(&str, &str, &str)] = &[
        ("reviewer", "[[1ed98864]]", "reviewer: \"[[1ed98864]]\""),
        ("owner", "", "owner: \"\""),
        ("note", "review: pending", "note: \"review: pending\""),
        ("plain", "doing", "plain: doing"),
        ("stamp", "2026-08-07", "stamp: 2026-08-07"),
        ("tags", "[type/a, topic/b]", "tags: [type/a, topic/b]"),
    ];
    for (key, value, stored_line) in cases {
        let dir = ws("cards/one.md", SEED);
        let reply = set_property(dir.path(), key, value);
        assert_eq!(
            reply["ok"],
            json!(true),
            "set_property {key}={value:?} lands (what the fleet can read, the engine can write): {reply}"
        );
        let disk = std::fs::read_to_string(dir.path().join("cards/one.md")).expect("read back");
        assert!(
            disk.lines().any(|l| l == *stored_line),
            "stored form for {key}={value:?} is the fleet-canonical line {stored_line:?}; file:\n{disk}"
        );
        let kv = read_props(dir.path(), "cards/one.md");
        let served = &kv
            .iter()
            .find(|(k, _)| k == key)
            .unwrap_or_else(|| panic!("{key} served on the props plane"))
            .1;
        assert_eq!(
            served, value,
            "round trip: decode(emitted line) is the caller's string for {key}"
        );
    }
}

/// The measured season-1 write symptom verbatim: the fleet's own canonical
/// `owner` value — what `ccc-cli task claim` writes — is writable, and the
/// nested-frontmatter refusal is unreachable from a flat string.
#[test]
fn the_fleet_canonical_owner_value_is_writable() {
    let dir = ws("cards/one.md", SEED);
    let reply = set_property(dir.path(), "owner", "[[b1892b5a]]");
    assert_eq!(
        reply["ok"],
        json!(true),
        "the write face can write the value it can read: {reply}"
    );
    let disk = std::fs::read_to_string(dir.path().join("cards/one.md")).expect("read back");
    assert!(
        disk.contains("owner: \"[[b1892b5a]]\""),
        "two writers, one value grammar — the emit matches `ccc-cli task claim`'s:\n{disk}"
    );
}
