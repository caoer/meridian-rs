//! The BIRTH op (`{"op":"create"}`) through the LIVE serve loop — the design
//! tests for its contract.
//!
//! The op's whole value is that it FORWARDS to the guarded in-process door
//! (`wire_serve::write::create`) instead of opening a second birth path. A
//! daemon-side `O_EXCL` birth would be trivially green and would bypass all
//! four engine birth guards while writing no journal row. So these tests do not
//! merely assert "a file appeared": they assert the file appeared THROUGH the
//! door — the journal row that dates it, the CAS that refuses a clobber, and a
//! guard refusal that only the door can produce.
//!
//! Every refusal arm is asserted by its CAUSE (code + the teaching message),
//! never by "it failed": two arms that both refuse for different reasons are two
//! detectors, and asserting only the code would let one silently become the
//! other.

use serde_json::{Value, json};

fn workspace() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("seed.md"),
        "---\ntype: seed\n---\n\n# Seed\n\nthe workspace is non-empty\n",
    )
    .expect("seed");
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    (dir, root)
}

/// One serve session: `input` lines in, parsed frames out. The negotiated rev
/// is per-session, so a hello and its follow-ups MUST ride one call.
fn serve(root: &fs::WorkspaceRoot, input: &str) -> Vec<Value> {
    let mut out = Vec::new();
    sidecar::serve(root, input.as_bytes(), &mut out, &[]).expect("serve");
    String::from_utf8(out)
        .expect("frames are UTF-8")
        .lines()
        .map(|l| serde_json::from_str(l).expect("frame parses"))
        .collect()
}

const HELLO_V3: &str = r#"{"id":1,"op":"hello","proto":1,"client":"design-test","contract":"v3"}"#;
const HELLO_V2: &str = r#"{"id":1,"op":"hello","proto":1,"client":"design-test"}"#;

/// A live handshake, asserted live. A REJECTED hello makes every later op on
/// the session refuse, which is indistinguishable from a missing capability —
/// so no test here reads an absence without first proving the session is alive.
fn assert_handshake_ok(frame: &Value) {
    assert_eq!(
        frame["ok"], json!(true),
        "the handshake must succeed before any capability claim is believable — \
         a refused hello fakes a missing op: {frame}"
    );
}

/// The whole-file rev of `bytes`, computed through the model's own document
/// law (`node_rev = blake3(span bytes)[:16]`, node-rev-merkle-spec §1) — the
/// test's OWN recomputation, so an assertion against it fails when the engine
/// reports a rev over bytes other than the ones it landed.
fn whole_file_rev(path: &str, bytes: &str) -> String {
    model::candidate_of_body(path, bytes.to_owned())
        .document()
        .root
        .node_rev
        .0
        .clone()
}

fn create_frame(id: u64, path: &str, body: &str) -> String {
    json!({"id": id, "op": "create", "path": path, "body": body,
           "actor": "agent:design-test", "now": "2026-07-26T13:30:00-04:00"})
    .to_string()
}

// ---------------------------------------------------------------------------
// Discovery honesty (§3.2): the op is in `caps` or it answers `unknown_op`.
// ---------------------------------------------------------------------------

#[test]
fn v3_hello_advertises_create_and_a_v2_session_refuses_the_op() {
    let (_dir, root) = workspace();

    let v3 = serve(&root, &format!("{HELLO_V3}\n"));
    assert_handshake_ok(&v3[0]);
    let caps = v3[0]["body"]["caps"].as_array().expect("caps");
    assert!(
        caps.contains(&json!("create")),
        "the v3 hello advertises the birth op: {caps:?}"
    );

    // The other side of the same law: a v2 session never advertises it AND
    // never serves it. Both halves asserted on a PROVEN-LIVE session.
    let v2 = serve(&root, &format!("{HELLO_V2}\n{}\n", create_frame(2, "a.md", "# A\n")));
    assert_handshake_ok(&v2[0]);
    let v2_caps = v2[0]["body"]["caps"].as_array().expect("caps");
    assert!(
        !v2_caps.contains(&json!("create")),
        "a v2 session never advertises the birth op: {v2_caps:?}"
    );
    assert_eq!(v2[1]["error"]["code"], json!("unknown_op"));
    assert_eq!(v2[1]["error"]["recovery"], json!("fix"));
    assert!(
        !root.0.join("a.md").exists(),
        "a refused op births nothing"
    );
}

// ---------------------------------------------------------------------------
// The birth itself — and the journal row that proves which door it came through.
// ---------------------------------------------------------------------------

#[test]
fn create_births_the_callers_exact_bytes_and_journals_the_birth() {
    let (_dir, root) = workspace();
    let body = "---\ntype: note\n---\n\n# Newborn\n\nborn over the wire\n";

    let frames = serve(
        &root,
        &format!("{HELLO_V3}\n{}\n", create_frame(2, "notes/newborn.md", body)),
    );
    assert_handshake_ok(&frames[0]);
    let reply = &frames[1];
    assert_eq!(reply["ok"], json!(true), "the birth landed: {reply}");

    // The bytes are the CALLER's, verbatim — the engine mints no template here.
    let landed = std::fs::read_to_string(root.0.join("notes/newborn.md")).expect("newborn exists");
    assert_eq!(landed, body, "birth is byte-transparent");

    // The response names the birth: path, the newborn's whole-file rev, the
    // root transition, and the journal anchor that dates it.
    let b = &reply["body"];
    assert_eq!(b["path"], json!("notes/newborn.md"));
    // The reported rev must describe the bytes that LANDED, recomputed here from
    // disk rather than echoed from the reply. A birth that reported the rev of a
    // draft — or of a templated body — fails this even though a file exists.
    assert_eq!(
        b["file_rev_after"].as_str().expect("the reply carries the newborn's rev"),
        whole_file_rev("notes/newborn.md", &landed),
        "the rev the birth reports is the rev of the bytes on disk"
    );
    assert_ne!(
        b["fingerprint_before"], b["fingerprint_after"],
        "a birth advances the world root: {b}"
    );
    assert!(
        b["fingerprint_after"].as_str().is_some(),
        "a real birth reports the advanced root (never null): {b}"
    );
    assert!(b["dry"].is_null(), "an ordinary birth carries no `dry` key: {b}");

    // v3 vocabulary honesty: the birth frame spells `fingerprint`, never `root`.
    assert!(
        b.get("root_before").is_none() && b.get("root_after").is_none(),
        "a v3 session emits no `root` spelling: {b}"
    );

    // THE DOOR TEST. A journal row exists, it is a `create` row, and its anchor
    // is the one the reply handed back. A daemon-side birth writes no row —
    // this assertion is what a bypass cannot satisfy.
    let anchor = b["journal_anchor"].as_str().expect("the birth reports its journal anchor");
    let journal = std::fs::read_to_string(root.0.join("meridian/journal.md"))
        .expect("the birth wrote the reserved journal");
    let row = journal
        .lines()
        .find(|l| l.contains(&format!("^{anchor}")))
        .unwrap_or_else(|| panic!("the reply's anchor `{anchor}` names a real row:\n{journal}"));
    assert!(row.contains("create"), "the row records the birth op: {row}");
    assert!(
        row.contains("notes/newborn.md"),
        "the row names the born path: {row}"
    );
    assert!(
        row.contains("agent:design-test"),
        "the row records the caller's actor verbatim (§9): {row}"
    );
}

// ---------------------------------------------------------------------------
// The refusal arms, each by its own cause.
// ---------------------------------------------------------------------------

#[test]
fn a_second_create_refuses_cas_mismatch_and_leaves_the_occupant_untouched() {
    let (_dir, root) = workspace();
    let first = "# First\n\nthe original\n";
    let second = "# Second\n\nthe clobber attempt\n";

    let frames = serve(
        &root,
        &format!(
            "{HELLO_V3}\n{}\n{}\n",
            create_frame(2, "once.md", first),
            create_frame(3, "once.md", second)
        ),
    );
    assert_handshake_ok(&frames[0]);
    assert_eq!(frames[1]["ok"], json!(true), "the first birth lands");

    let refusal = &frames[2];
    assert_eq!(refusal["ok"], json!(false));
    assert_eq!(
        refusal["error"]["code"],
        json!("cas_mismatch"),
        "an occupied path is a CAS failure, not a generic error: {refusal}"
    );
    assert_eq!(
        refusal["error"]["recovery"],
        json!("refresh"),
        "the recovery class teaches `re-read, it exists`: {refusal}"
    );

    // The occupant is byte-untouched: there is no clobbering birth.
    assert_eq!(
        std::fs::read_to_string(root.0.join("once.md")).expect("occupant survives"),
        first,
        "a refused birth never overwrote the occupant"
    );

    // And no second journal row was written for the refusal.
    let journal = std::fs::read_to_string(root.0.join("meridian/journal.md")).expect("journal");
    assert_eq!(
        journal.matches("once.md").count(),
        1,
        "the refused birth journaled nothing:\n{journal}"
    );
}

#[test]
fn a_dry_birth_writes_nothing_and_reports_a_null_root_after() {
    let (_dir, root) = workspace();
    let before = std::fs::read_to_string(root.0.join("meridian/journal.md")).unwrap_or_default();

    let dry = json!({"id":2,"op":"create","path":"rehearsal.md","body":"# R\n",
                     "actor":"agent:design-test","dry":true})
    .to_string();
    let frames = serve(&root, &format!("{HELLO_V3}\n{dry}\n"));
    assert_handshake_ok(&frames[0]);
    let b = &frames[1]["body"];

    assert_eq!(frames[1]["ok"], json!(true), "a rehearsal succeeds");
    assert_eq!(b["dry"], json!(true));
    assert!(
        b["fingerprint_after"].is_null(),
        "a rehearsal advances no root — null, not absent: {b}"
    );
    assert!(
        b["journal_anchor"].is_null(),
        "a rehearsal writes no journal row, so it reports no anchor: {b}"
    );
    assert!(
        b["file_rev_after"].as_str().is_some(),
        "the rev IS computable from the spec, so a rehearsal still reports it: {b}"
    );
    assert!(!root.0.join("rehearsal.md").exists(), "a rehearsal births no file");
    assert_eq!(
        std::fs::read_to_string(root.0.join("meridian/journal.md")).unwrap_or_default(),
        before,
        "a rehearsal leaves the journal byte-identical"
    );
}

#[test]
fn a_dry_birth_over_an_occupied_path_still_refuses_the_clobber() {
    let (_dir, root) = workspace();
    let frames = serve(
        &root,
        &format!(
            "{HELLO_V3}\n{}\n{}\n",
            create_frame(2, "taken.md", "# Taken\n"),
            json!({"id":3,"op":"create","path":"taken.md","body":"# X\n","dry":true})
        ),
    );
    assert_handshake_ok(&frames[0]);
    assert_eq!(frames[1]["ok"], json!(true));
    assert_eq!(
        frames[2]["error"]["code"],
        json!("cas_mismatch"),
        "a rehearsal of a clobber refuses exactly where the real birth would: {}",
        frames[2]
    );
}

#[test]
fn a_stale_world_guard_refuses_fingerprint_mismatch_and_births_nothing() {
    let (_dir, root) = workspace();
    let stale = json!({"id":2,"op":"create","path":"guarded.md","body":"# G\n",
                       "if_fingerprint":"b3:0000000000000000000000000000000000000000000000000000000000000000"})
    .to_string();
    let frames = serve(&root, &format!("{HELLO_V3}\n{stale}\n"));
    assert_handshake_ok(&frames[0]);

    assert_eq!(
        frames[1]["error"]["code"],
        json!("fingerprint_mismatch"),
        "the v3 spelling of the stale world guard: {}",
        frames[1]
    );
    assert_eq!(frames[1]["error"]["recovery"], json!("resync"));
    assert!(!root.0.join("guarded.md").exists(), "a stale guard births nothing");
}

/// **The proof that this op forwards rather than re-implements.**
///
/// The S10/R25 document-grain `@fp` strip lives inside `write::create` and
/// nowhere else. A daemon-side `O_EXCL` birth would land the forged claim token
/// verbatim — so this test discriminates precisely between the guarded door and
/// a bypass, and it fails in BOTH bypass directions: a birth that lands the
/// token, and a birth that reports the un-stripped draft's rev.
#[test]
fn the_birth_runs_the_engines_own_guards_not_a_parallel_path() {
    let (_dir, root) = workspace();
    // A claim-link carrying a fingerprint token the engine never minted.
    let forged = "# Forged\n\nsee [[guide#^goal@green.b3af12cd]] for the plan\n";
    let stripped = "# Forged\n\nsee [[guide#^goal]] for the plan\n";

    let frames = serve(&root, &format!("{HELLO_V3}\n{}\n", create_frame(2, "forged.md", forged)));
    assert_handshake_ok(&frames[0]);
    let reply = &frames[1];
    assert_eq!(reply["ok"], json!(true), "the strip admits the birth: {reply}");

    // The forged claim is gone and the ADDRESS survives — the guard strips the
    // token, it does not discard the link.
    let landed = std::fs::read_to_string(root.0.join("forged.md")).expect("newborn");
    assert_eq!(
        landed, stripped,
        "the document-grain strip removed the @fp claim and kept the address"
    );

    // And the reported rev is the rev of the STRIPPED bytes, never of the
    // decorated draft the caller sent (`write::create`'s own stated law).
    assert_eq!(
        reply["body"]["file_rev_after"].as_str().expect("rev"),
        whole_file_rev("forged.md", stripped),
        "the birth reports the rev of what landed, not of the draft"
    );
    assert_ne!(
        reply["body"]["file_rev_after"].as_str().expect("rev"),
        whole_file_rev("forged.md", forged),
        "anti-vacuity: the two revs genuinely differ, so the assertion above discriminates"
    );
}

#[test]
fn the_reserved_journal_cannot_be_born_over() {
    let (_dir, root) = workspace();
    let frames = serve(
        &root,
        &format!("{HELLO_V3}\n{}\n", create_frame(2, "meridian/journal.md", "# tamper\n")),
    );
    assert_handshake_ok(&frames[0]);

    assert_eq!(frames[1]["error"]["code"], json!("bad_request"));
    let msg = frames[1]["error"]["message"].as_str().unwrap_or_default();
    assert!(
        msg.contains("reserved receipt journal"),
        "the refusal teaches WHY the journal is untouchable: {msg}"
    );
}

#[test]
fn a_path_escaping_the_workspace_refuses_bad_path_at_the_wire() {
    let (_dir, root) = workspace();
    let frames = serve(
        &root,
        &format!("{HELLO_V3}\n{}\n", create_frame(2, "../escape.md", "# out\n")),
    );
    assert_handshake_ok(&frames[0]);

    assert_eq!(
        frames[1]["error"]["code"],
        json!("bad_path"),
        "the jail is the FIRST wall — an escape cannot even be spelled: {}",
        frames[1]
    );
    assert_eq!(frames[1]["error"]["path"], json!("../escape.md"));
    assert!(
        !root.0.parent().expect("parent").join("escape.md").exists(),
        "nothing was written outside the workspace"
    );
}

/// A malformed `now` refuses at the wire, because the journal row is stamped
/// with the CALLER's clock (§9: the engine validates time, never generates it).
/// Landing the row first and discovering the garbage later is not an option —
/// the row is the baseline `check` dates the tree against.
#[test]
fn a_malformed_now_refuses_before_anything_is_born() {
    let (_dir, root) = workspace();
    let bad = json!({"id":2,"op":"create","path":"clock.md","body":"# C\n","now":"yesterday"})
        .to_string();
    let frames = serve(&root, &format!("{HELLO_V3}\n{bad}\n"));
    assert_handshake_ok(&frames[0]);

    assert_eq!(frames[1]["error"]["code"], json!("bad_request"));
    let msg = frames[1]["error"]["message"].as_str().unwrap_or_default();
    assert!(msg.contains("RFC 3339"), "the refusal names the format law: {msg}");
    assert!(!root.0.join("clock.md").exists(), "nothing was born");
}

/// The birth is visible to the read plane in the SAME session — it is a real
/// commit, not a staged one. Proves the root advance the reply reported is the
/// root the next op reads.
#[test]
fn the_newborn_is_immediately_readable_at_the_root_the_birth_reported() {
    let (_dir, root) = workspace();
    let body = "---\ntype: note\n---\n\n# Fresh\n\ncontent\n";
    let frames = serve(
        &root,
        &format!(
            "{HELLO_V3}\n{}\n{}\n{}\n",
            create_frame(2, "fresh.md", body),
            json!({"id":3,"op":"toc","path":"fresh.md"}),
            json!({"id":4,"op":"fingerprint"})
        ),
    );
    assert_handshake_ok(&frames[0]);
    let birth = &frames[1]["body"];
    assert_eq!(frames[2]["ok"], json!(true), "the newborn is readable: {}", frames[2]);

    assert_eq!(
        frames[2]["body"]["file_rev"], birth["file_rev_after"],
        "the rev the birth reported IS the rev the read sees"
    );
    assert_eq!(
        frames[3]["body"]["fingerprint"], birth["fingerprint_after"],
        "the root the birth advanced to IS the ambient root now"
    );
}
