//! REVIEW GATE ARTIFACT — the splice-level scenario matrix mandated after
//! `bakeoff/1538cfe6` was blocked on a splice-arithmetic defect its
//! encoder-level tests could not see.
//!
//! **Ported verbatim in shape** from the review gate's own artifact
//! (`results/review-gate/splice_matrix.rs`, authored and passing on
//! `bakeoff/d5654f18`), together with the one-line guard fix it covers. It is
//! the mutation-proven coverage for that guard: reverting `!safe.is_empty()`
//! to `!val.is_empty()` in `policy::defs::rebuild` leaves every OTHER test in
//! this workspace green and reddens this file alone.
//!
//! Two changes from the artifact, both stated: the foreign-parser judgment is
//! made INLINE here (PyYAML on every pass, not only on the dumped artifacts),
//! and the artifact's `read_then_write_back_is_byte_stable` is NOT ported —
//! its claim is false on both bake-off branches (a quoted read written back
//! re-spells plain), which is carded as its own follow-up rather than
//! smuggled into this fuse.
//!
//! Matrix: three stored colon shapes x the empty value x applied TWICE
//! (idempotency), driven through the REAL `set_property` wire door. Each
//! resulting document is dumped to `$MRD_MATRIX_OUT` so an EXTERNAL YAML
//! parser (PyYAML) judges the bytes — the engine's own parse is not the
//! oracle here, because that is precisely the blind spot that let the
//! sibling branch emit `note:""` and pass its own suite.

use serde_json::{Value, json};

fn serve(doc_dir: &std::path::Path, requests: &[Value]) -> Vec<Value> {
    let mut input = crate::daemon_door::hello_line(Some("v3"), doc_dir);
    for r in requests {
        input.push_str(&serde_json::to_string(r).expect("request serializes"));
        input.push('\n');
    }
    crate::daemon_door::serve_frames(&input)
}

fn ws(rel: &str, body: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path().join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).expect("mkdir");
    }
    std::fs::write(p, body).expect("seed");
    dir
}

/// One `set_property` through the real door, honestly rev-guarded.
fn set_property(dir: &std::path::Path, key: &str, value: &str) -> Value {
    let frames = serve(dir, &[json!({"id":1,"op":"read","path":"cards/one.md"})]);
    assert_eq!(frames[1]["ok"], json!(true), "minting read: {}", frames[1]);
    let rev = frames[1]["body"]["file_rev"]
        .as_str()
        .expect("file_rev")
        .to_owned();
    let frames = serve(
        dir,
        &[json!({
            "id":2,"op":"splice","path":"cards/one.md",
            "actor":"agent:review-gate","now":"2026-08-08T00:00:00Z",
            "plan_edits":[{"set_property":{"key":key,"value":value,"rev":rev}}],
        })],
    );
    frames[1].clone()
}

/// `props[].value` for one key off a composed read, if present.
fn served(dir: &std::path::Path, key: &str) -> Option<String> {
    let frames = serve(dir, &[json!({"id":1,"op":"read","path":"cards/one.md"})]);
    if frames[1]["ok"] != json!(true) {
        return None;
    }
    frames[1]["body"]["props"]
        .as_array()?
        .iter()
        .find(|p| p["key"].as_str() == Some(key))
        .and_then(|p| p["value"].as_str().map(str::to_owned))
}

fn dump(scenario: &str, pass: usize, bytes: &str) {
    let Ok(out) = std::env::var("MRD_MATRIX_OUT") else {
        return;
    };
    let d = std::path::Path::new(&out);
    std::fs::create_dir_all(d).expect("mkdir out");
    std::fs::write(d.join(format!("{scenario}__pass{pass}.md")), bytes).expect("dump");
}

/// The three stored colon shapes the sibling branch's defect lived across,
/// plus the absent-key create path. Each seeded doc keys on `note`.
fn shapes() -> Vec<(&'static str, String)> {
    vec![
        // key present, with a value
        ("with_value", "---\nnote: x\nkeep: 1\n---\n# B\n\nt\n".to_string()),
        // key present, BARE colon (no trailing space) — the sibling's P0 cell
        ("bare_colon", "---\nnote:\nkeep: 1\n---\n# B\n\nt\n".to_string()),
        // key present, colon + trailing space, empty value
        ("colon_space", "---\nnote: \nkeep: 1\n---\n# B\n\nt\n".to_string()),
        // key ABSENT — the create path
        ("absent", "---\nkeep: 1\n---\n# B\n\nt\n".to_string()),
    ]
}

/// THE MANDATED MATRIX: shape x empty value x twice.
#[test]
fn splice_matrix_empty_value_across_colon_shapes_twice() {
    for (name, seed) in shapes() {
        let dir = ws("cards/one.md", &seed);
        for pass in 1..=2 {
            let reply = set_property(dir.path(), "note", "");
            assert_eq!(
                reply["ok"],
                json!(true),
                "{name} pass{pass}: set_property(note,\"\") lands: {reply}"
            );
            let disk =
                std::fs::read_to_string(dir.path().join("cards/one.md")).expect("read back");
            dump(name, pass, &disk);

            // The engine must serve back exactly the caller's string.
            assert_eq!(
                served(dir.path(), "note").as_deref(),
                Some(""),
                "{name} pass{pass}: round trip serves the caller's empty string; file:\n{disk}"
            );
            // The composed line must carry a real `: ` separator.
            assert!(
                disk.lines().any(|l| l == "note: \"\""),
                "{name} pass{pass}: expected the canonical `note: \"\"` line; file:\n{disk}"
            );
            // The sibling branch's exact defect, pinned as a negative.
            assert!(
                !disk.contains("note:\"\""),
                "{name} pass{pass}: emitted a separator-less `note:\"\"`; file:\n{disk}"
            );
            // `keep` must survive every pass — a void block loses it.
            assert!(
                disk.contains("keep: 1"),
                "{name} pass{pass}: the sibling key survived; file:\n{disk}"
            );
            // The oracle that matters: an EXTERNAL parser, judged inline.
            crate::fm_scalar_normalization::assert_foreign_parse(
                &disk,
                &[("note", ""), ("keep", "1")],
                name,
                pass as u32,
            );
        }
        // Idempotency: pass 2 must not differ from pass 1.
        let out = std::env::var("MRD_MATRIX_OUT").ok();
        if let Some(o) = out {
            let d = std::path::Path::new(&o);
            let p1 = std::fs::read_to_string(d.join(format!("{name}__pass1.md"))).expect("p1");
            let p2 = std::fs::read_to_string(d.join(format!("{name}__pass2.md"))).expect("p2");
            assert_eq!(p1, p2, "{name}: set_property is not byte-idempotent");
        }
    }
}

/// The same matrix for the values the fleet actually writes — idempotency
/// beyond the empty case, since a read->write churn moves every fingerprint.
#[test]
fn splice_matrix_fleet_values_are_byte_idempotent() {
    for value in ["[[b1892b5a]]", "doing", "review: pending", "2026-08-07", "[a, b]"] {
        for (name, seed) in shapes() {
            let dir = ws("cards/one.md", &seed);
            let r1 = set_property(dir.path(), "note", value);
            assert_eq!(r1["ok"], json!(true), "{name}/{value:?} pass1: {r1}");
            let d1 = std::fs::read_to_string(dir.path().join("cards/one.md")).expect("d1");
            let r2 = set_property(dir.path(), "note", value);
            assert_eq!(r2["ok"], json!(true), "{name}/{value:?} pass2: {r2}");
            let d2 = std::fs::read_to_string(dir.path().join("cards/one.md")).expect("d2");
            assert_eq!(d1, d2, "{name}/{value:?}: not byte-idempotent");
            assert_eq!(
                served(dir.path(), "note").as_deref(),
                Some(value),
                "{name}/{value:?}: round trip"
            );
            dump(&format!("val_{}_{name}", value.replace([' ', ':', '[', ']', '/'], "_")), 1, &d1);
        }
    }
}
