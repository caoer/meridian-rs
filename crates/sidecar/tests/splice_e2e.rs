//! D4-SPLICE end-to-end: §4.4 E3/E4 (armed, disk, receipts, R0→R1→R2), §5.2
//! failure split, `fm_key` dry, §4.6 live-triple. Values engine-derived.

use serde_json::{Value, json};
use std::io::Write as _;

const R0: &str = "b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9";
const R1: &str = "b3:10769ae1c77f5646750f3f52df2d055156b411145a02b8361ecd32af1357a1b7";
const R2: &str = "b3:83b4ba591c0291d9f2a05428cac38e5820858fbb9c47720ab352344ddccc8f68";

/// §6.3 frozen receipt lines — here they are the EXPECTED DISK BYTES the
/// engine's own receipt renderer must produce (never inputs).
const E3_LINE: &str = "- splice notes/plan.md id=42 actor=agent:b0864fb2 now=2026-07-18T20:31:04Z root_before=b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9 edits=1 Goals>Q3 match 33d5b0e1b27cb48b->41f643f034e5681f ^r-000042";
const E4_LINE: &str = "- splice notes/plan.md id=57 actor=agent:b0864fb2 now=2026-07-18T20:33:41Z root_before=b3:10769ae1c77f5646750f3f52df2d055156b411145a02b8361ecd32af1357a1b7 edits=1 Goals>Q4 put:end 4b8bc385a58da0e0->f43203a1f0b4c9a3 ^r-000043";

fn wsfix(rel: &str) -> String {
    std::fs::read_to_string(testsuite::wsfix_dir().join(rel))
        .unwrap_or_else(|e| panic!("wsfix fixture {rel}: {e}"))
}

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

fn s0() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let plan = wsfix("s0/notes/plan.md");
    let receipts = wsfix("s0/receipts/2026-07-18.md");
    workspace(&[
        ("notes/plan.md", &plan),
        ("receipts/2026-07-18.md", &receipts),
        (".github/README.md", "# CI notes\n"),
    ])
}

fn serve(root: &fs::WorkspaceRoot, input: &str) -> Vec<Value> {
    let mut out = Vec::new();
    sidecar::serve(root, input.as_bytes(), &mut out, &[]).expect("serve");
    String::from_utf8(out)
        .expect("frames are UTF-8")
        .lines()
        .map(|l| serde_json::from_str(l).expect("frame parses"))
        .collect()
}

fn one(root: &fs::WorkspaceRoot, line: &str) -> Value {
    let mut frames = serve(root, &format!("{line}\n"));
    assert_eq!(frames.len(), 1, "exactly one response per request");
    frames.remove(0)
}

/// Live `node_rev` for U10 wire-origin content changes (derived, not typed).
fn rev_of(root: &fs::WorkspaceRoot, sec: Value) -> String {
    let doc = fs::load(root, std::path::Path::new("notes/plan.md")).expect("load");
    let sec: wire::SecRef = serde_json::from_value(sec).expect("selector decodes");
    match wire_serve::read::cat(&doc, Some(sec)).expect("cat") {
        wire::ResponseBody::Cat { node_rev, .. } => node_rev.0,
        other => panic!("cat returned {other:?}"),
    }
}

fn q3(root: &fs::WorkspaceRoot) -> String {
    rev_of(root, json!({"hpath": [{"h": "Goals"}, {"h": "Q3"}]}))
}

fn q4(root: &fs::WorkspaceRoot) -> String {
    rev_of(root, json!({"hpath": [{"h": "Goals"}, {"h": "Q4"}]}))
}

/// Frozen §4.4 session: E3/E4 + §5.2 failures + dry/links/root. U10 revs derived.
fn session(root: &fs::WorkspaceRoot) -> String {
    let q3 = q3(root);
    let q4 = q4(root);
    // E4 appends to Q4, so the later companion addresses the POST-E4 node. That
    // rev is already a pinned fact of this fixture: E4's receipt line records it
    // as the after-rev of the very edit above.
    let q4_after = E4_LINE
        .rsplit_once("->")
        .expect("E4 receipt names the after-rev")
        .1
        .split_whitespace()
        .next()
        .expect("after-rev token")
        .to_owned();
    let title = rev_of(root, json!({"fm_key": "title"}));
    [
        format!(r#"{{"id":42,"op":"splice","path":"notes/plan.md","actor":"agent:b0864fb2","now":"2026-07-18T20:31:04Z","receipt":{{"path":"receipts/2026-07-18.md","anchor":"r-000042"}},"if_root":"b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9","edits":[{{"target":{{"hpath":[{{"h":"Goals"}},{{"h":"Q3"}}]}},"edit":{{"match":{{"old":"ship by August","new":"ship by September"}}}},"if_node_rev":"{q3}"}}]}}"#),
        format!(r#"{{"id":57,"op":"splice","path":"notes/plan.md","actor":"agent:b0864fb2","now":"2026-07-18T20:33:41Z","receipt":{{"path":"receipts/2026-07-18.md","anchor":"r-000043"}},"edits":[{{"target":{{"hpath":[{{"h":"Goals"}},{{"h":"Q4"}}]}},"edit":{{"put":{{"at":"end","text":"- new item\n"}}}},"if_node_rev":"{q4}"}}]}}"#),
        format!(r#"{{"id":88,"op":"splice","path":"notes/plan.md","edits":[{{"target":{{"hpath":[{{"h":"Goals"}},{{"h":"Q3"}}]}},"edit":{{"match":{{"old":"ship by September","new":"ship by October"}}}},"if_node_rev":"{q3}"}}]}}"#),
        // The one companion whose rev is NOT the live pre-image: this frame
        // deliberately pins Q3's POST-E3 rev to exercise the §5.2 stale-CAS
        // split, so it is a literal rather than a derived value.
        r#"{"id":89,"op":"splice","path":"notes/plan.md","edits":[{"target":{"hpath":[{"h":"Goals"},{"h":"Q3"}]},"edit":{"match":{"old":"ship by August","new":"ship by October"}},"if_node_rev":"41f643f034e5681f"}]}"#
            .to_owned(),
        format!(r#"{{"id":91,"op":"splice","path":"notes/plan.md","edits":[{{"target":{{"hpath":[{{"h":"Goals"}},{{"h":"Q4"}}]}},"edit":{{"match":{{"old":"item","new":"entry"}}}},"if_node_rev":"{q4_after}"}}]}}"#),
        format!(r#"{{"id":60,"op":"splice","path":"notes/plan.md","dry":true,"edits":[{{"target":{{"fm_key":"title"}},"edit":{{"match":{{"old":"Plan","new":"Plan v2"}}}},"if_node_rev":"{title}"}}]}}"#),
        r#"{"id":80,"op":"links","path":"notes/plan.md"}"#.to_owned(),
        r#"{"id":90,"op":"root"}"#.to_owned(),
    ]
    .join("\n")
        + "\n"
}

/// Gate 1 (+ gates 3 and 5, one epoch): the whole frozen worked world,
/// derived. Eight printed exchanges byte-exact through one serve session,
/// then the disk cross-check (S2 plan ≡ committed fixture; receipts ≡ S0 +
/// the two §6.3 lines the engine's OWN renderer produced).
#[test]
fn worked_session_e3_e4_failures_dry_links_root_byte_exact() {
    let (_d, root) = s0();
    let frames = serve(&root, &session(&root));
    assert_eq!(frames.len(), 8, "eight exchanges");

    // §4.4 E3 armed response, printed (S0→S1).
    assert_eq!(
        frames[0],
        json!({"id":42,"ok":true,"body":{
         "armed":{"path":"notes/plan.md","file_rev_after":"a9794a262e67ed02","edits":[
           {"target":{"hpath":[{"h":"Goals"},{"h":"Q3"}]},
            "node_rev_before":"33d5b0e1b27cb48b","node_rev_after":"41f643f034e5681f",
            "span_after":[49,75]}]},
         "receipt":{"path":"receipts/2026-07-18.md","anchor":"r-000042",
                    "node_rev":"639a2dca46f6fcc8","span_after":[26,248]},
         "root_before":R0,
         "root_after":R1,
         "seq":1,"verdicts":[]}}),
        "E3"
    );

    // §4.4 E4 armed response, printed (S1→S2) — the append verb.
    assert_eq!(
        frames[1],
        json!({"id":57,"ok":true,"body":{
         "armed":{"path":"notes/plan.md","file_rev_after":"5f27a2814b517680","edits":[
           {"target":{"hpath":[{"h":"Goals"},{"h":"Q4"}]},
            "node_rev_before":"4b8bc385a58da0e0","node_rev_after":"f43203a1f0b4c9a3",
            "span_after":[75,150]}]},
         "receipt":{"path":"receipts/2026-07-18.md","anchor":"r-000043",
                    "node_rev":"c912d4578883f288","span_after":[249,473]},
         "root_before":R1,
         "root_after":R2,
         "seq":2,"verdicts":[]}}),
        "E4"
    );

    // §5.2 failure split against S2, printed: stale rev → cas_mismatch.
    assert_eq!(
        frames[2],
        json!({"id":88,"ok":false,"error":{"code":"cas_mismatch","recovery":"refresh",
         "expected":"33d5b0e1b27cb48b","actual":"41f643f034e5681f"}}),
        "88"
    );
    // rev PASSED and old absent → no_match, provably a typo.
    assert_eq!(
        frames[3],
        json!({"id":89,"ok":false,"error":{"code":"no_match","recovery":"fix","matches":0}}),
        "89"
    );
    // `item` occurs twice at S2 → not_unique{matches:2}, computed.
    assert_eq!(
        frames[4],
        json!({"id":91,"ok":false,"error":{"code":"not_unique","recovery":"fix","matches":2}}),
        "91"
    );

    // §4.4 fm_key dry against S2, printed: root_after NULL serialized, no
    // receipt, no seq, dry echoed, verdicts [].
    assert_eq!(
        frames[5],
        json!({"id":60,"ok":true,"body":{
         "armed":{"path":"notes/plan.md","edits":[
           {"target":{"fm_key":"title"},
            "node_rev_before":"fa77480c79a853bc","node_rev_after":"fb49e9df2257fab8",
            "span_after":[4,18]}]},
         "root_before":R2,
         "root_after":null,"dry":true,"verdicts":[]}}),
        "60"
    );

    // GATE 5 — §4.6 live-triple closure: emission exists now, so the live
    // links exchange serves the PRINTED body computed-real — changes_seq:2,
    // roots at R2. q5's truthful-0 caveat retires here.
    assert_eq!(
        frames[6],
        json!({"id":80,"ok":true,"body":{
         "as_of_root":R2,
         "live_root":R2,
         "changes_seq":2,
         "files":{"notes/plan.md":{
           "resolved":{"receipts/2026-07-18.md":1},
           "unresolved":{"roadmap":1}}}}}),
        "80 — the q5 FLIP closure"
    );

    // §4.7 root worked exchange, printed: R2 + this epoch's seq 2.
    assert_eq!(
        frames[7],
        json!({"id":90,"ok":true,"body":{"root":R2,"seq":2}}),
        "90"
    );

    // Disk cross-check: derived ≡ committed (§0.3). The receipt bytes are
    // the engine's OWN renderer output — asserted against the frozen §6.3
    // lines byte-for-byte.
    let plan = std::fs::read_to_string(root.0.join("notes/plan.md")).unwrap();
    assert_eq!(
        plan,
        wsfix("s2/notes/plan.md"),
        "derived S2 plan ≡ committed"
    );
    let receipts = std::fs::read_to_string(root.0.join("receipts/2026-07-18.md")).unwrap();
    let expected_receipts = format!(
        "{}{E3_LINE}\n{E4_LINE}\n",
        wsfix("s0/receipts/2026-07-18.md")
    );
    assert_eq!(
        receipts, expected_receipts,
        "rendered receipts ≡ §6.3 bytes"
    );
    assert_eq!(receipts.len(), 474, "S2 receipts byte count (§0.3)");
}

/// Gate 2: `dry:true` runs everything except disk — same response shape,
/// `root_after:null`, no receipt written even when one is NAMED, no root
/// advance, no Delta in the ring (a following `root` still shows seq 0 and
/// R0), no byte anywhere.
#[test]
fn dry_run_touches_nothing() {
    let (_d, root) = s0();
    let frames = serve(
        &root,
        &format!(
            concat!(
                r#"{{"id":1,"op":"splice","path":"notes/plan.md","dry":true,"receipt":{{"path":"receipts/2026-07-18.md","anchor":"r-000042"}},"edits":[{{"target":{{"hpath":[{{"h":"Goals"}},{{"h":"Q3"}}]}},"edit":{{"match":{{"old":"ship by August","new":"ship by September"}}}},"if_node_rev":"{}"}}]}}"#,
                "\n",
                r#"{{"id":2,"op":"root"}}"#,
                "\n",
            ),
            q3(&root)
        ),
    );
    assert_eq!(frames[0]["ok"], true, "{}", frames[0]);
    assert_eq!(
        frames[0]["body"]["root_after"],
        Value::Null,
        "null, serialized"
    );
    assert_eq!(frames[0]["body"]["dry"], true);
    assert!(
        frames[0]["body"].get("receipt").is_none(),
        "no receipt fact on dry — none was written"
    );
    assert!(frames[0]["body"].get("seq").is_none(), "no seq on dry");
    // no root advance, no Delta: the epoch still reports seq 0 at R0
    assert_eq!(frames[1]["body"], json!({"root":R0,"seq":0}));
    // and the bytes never moved
    assert_eq!(
        std::fs::read_to_string(root.0.join("notes/plan.md")).unwrap(),
        wsfix("s0/notes/plan.md")
    );
    assert_eq!(
        std::fs::read_to_string(root.0.join("receipts/2026-07-18.md")).unwrap(),
        wsfix("s0/receipts/2026-07-18.md")
    );
}

/// W-5 `file_rev_after`: a real splice reports the post-write whole-file rev
/// on its armed fact, and it matches a SUBSEQUENT `toc`'s `file_rev`
/// byte-for-byte — a consumer learns the new rev without the follow-up `toc`.
/// The DRY twin carries NO `file_rev_after` (nothing was written), and the
/// `root_after` world grain is the advanced root either way (unchanged).
#[test]
fn armed_file_rev_after_matches_subsequent_toc() {
    let (_d, root) = s0();
    let frames = serve(
        &root,
        &format!(
            concat!(
                // the E3 real commit (plan edit + receipt) — root advances R0→R1
                r#"{{"id":1,"op":"splice","path":"notes/plan.md","actor":"agent:b0864fb2","now":"2026-07-18T20:31:04Z","receipt":{{"path":"receipts/2026-07-18.md","anchor":"r-000042"}},"edits":[{{"target":{{"hpath":[{{"h":"Goals"}},{{"h":"Q3"}}]}},"edit":{{"match":{{"old":"ship by August","new":"ship by September"}}}},"if_node_rev":"{q3}"}}]}}"#,
                "\n",
                // read the map — its file_rev is the ground truth a consumer would fetch
                r#"{{"id":2,"op":"toc","path":"notes/plan.md"}}"#,
                "\n",
                // a DRY splice on the committed S1 — writes nothing. Q4 is
                // untouched by the E3 commit above, so its pre-session rev stands.
                r#"{{"id":3,"op":"splice","path":"notes/plan.md","dry":true,"edits":[{{"target":{{"hpath":[{{"h":"Goals"}},{{"h":"Q4"}}]}},"edit":{{"put":{{"at":"end","text":"- x\n"}}}},"if_node_rev":"{q4}"}}]}}"#,
                "\n",
            ),
            q3 = q3(&root),
            q4 = q4(&root),
        ),
    );

    // the armed post-write file rev rides the real commit ...
    assert_eq!(
        frames[0]["body"]["armed"]["file_rev_after"], "a9794a262e67ed02",
        "armed reports the post-write file rev"
    );
    // ... and equals the SUBSEQUENT toc's file_rev — no drift, no extra round trip
    assert_eq!(
        frames[0]["body"]["armed"]["file_rev_after"], frames[1]["body"]["file_rev"],
        "armed file_rev_after ≡ subsequent toc file_rev"
    );
    // the world grain is untouched: root_after is still a real advanced root
    // on the real commit (never dry-null) — file_rev_after rides beside it
    assert!(
        frames[0]["body"]["root_after"].is_string(),
        "root_after world grain present on the real commit"
    );

    // the DRY twin wrote nothing, so it carries NO post-write file rev
    assert_eq!(frames[2]["body"]["dry"], true);
    assert!(
        frames[2]["body"]["armed"].get("file_rev_after").is_none(),
        "dry splice carries no file_rev_after"
    );
}

/// Gate 7: the receipt parent-dir obligation is the CALLER's (fs does not
/// mkdir — F4 seam memo). A real splice naming a receipt under an absent
/// directory succeeds — the arm creates the dir; the SAME request dry
/// creates NOTHING, not even the directory.
#[test]
fn receipt_parent_dir_created_by_caller_real_only() {
    // The Q3 rev is identical in both fixtures below — same s0 pre-image — so it
    // is taken once, from the first.
    let (_d1, root_dry) = s0();
    let req = format!(
        r#"{{"id":7,"op":"splice","path":"notes/plan.md","receipt":{{"path":"logs/2026/receipts.md","anchor":"r-000099"}},"edits":[{{"target":{{"hpath":[{{"h":"Goals"}},{{"h":"Q3"}}]}},"edit":{{"match":{{"old":"ship by August","new":"ship by September"}}}},"if_node_rev":"{}"}}]}}"#,
        q3(&root_dry)
    );

    // dry first: zero disk effects means zero — no file, no dir.
    let dry_req = req.replace(r#""op":"splice","#, r#""op":"splice","dry":true,"#);
    let frame = one(&root_dry, &dry_req);
    assert_eq!(frame["ok"], true, "{frame}");
    assert!(
        !root_dry.0.join("logs").exists(),
        "dry creates nothing — not even the parent dir"
    );

    // real: the caller mkdirs, the commit lands, the anchor resolves.
    let (_d2, root_real) = s0();
    let frame = one(&root_real, &req);
    assert_eq!(frame["ok"], true, "{frame}");
    assert_eq!(frame["body"]["receipt"]["path"], "logs/2026/receipts.md");
    assert_eq!(frame["body"]["receipt"]["anchor"], "r-000099");
    let receipt = std::fs::read_to_string(root_real.0.join("logs/2026/receipts.md")).unwrap();
    assert!(
        receipt.contains("^r-000099") && receipt.ends_with('\n'),
        "receipt file created under the caller-made dir: {receipt:?}"
    );
}

fn assert_bad_request(frame: &Value) {
    assert_eq!(frame["ok"], false, "{frame}");
    assert_eq!(frame["error"]["code"], "bad_request", "{frame}");
    assert_eq!(frame["error"]["recovery"], "fix", "{frame}");
}

/// Gate 4: strict-decode rejection fixtures for the newly armed op — every
/// op ships them (§3.2 strict server; hand-rolled decode, risk R5).
#[test]
fn splice_strict_decode_rejections() {
    let (_d, root) = s0();
    // unknown field, named loud
    let frame = one(
        &root,
        r#"{"id":1,"op":"splice","path":"notes/plan.md","mode":"fast","edits":[{"target":{"fm_key":"title"},"edit":{"match":{"old":"a","new":"b"}}}]}"#,
    );
    assert_bad_request(&frame);
    assert!(
        frame["error"]["message"]
            .as_str()
            .expect("names the field")
            .contains("unknown request field"),
        "{frame}"
    );
    // mistyped / malformed shapes, each refused
    for req in [
        // missing edits
        r#"{"id":1,"op":"splice","path":"notes/plan.md"}"#,
        // edits not an array
        r#"{"id":1,"op":"splice","path":"notes/plan.md","edits":{}}"#,
        // empty batch (derived: unrepresentable under §7.1 one-advance law)
        r#"{"id":1,"op":"splice","path":"notes/plan.md","edits":[]}"#,
        // unknown key inside an edit
        r#"{"id":1,"op":"splice","path":"notes/plan.md","edits":[{"target":{"fm_key":"title"},"edit":{"match":{"old":"a","new":"b"}},"span":[0,4]}]}"#,
        // both shapes at once
        r#"{"id":1,"op":"splice","path":"notes/plan.md","edits":[{"target":{"fm_key":"title"},"edit":{"match":{"old":"a","new":"b"},"put":{"at":"end","text":"x"}}}]}"#,
        // unknown put position
        r#"{"id":1,"op":"splice","path":"notes/plan.md","edits":[{"target":{"fm_key":"title"},"edit":{"put":{"at":"middle","text":"x"}}}]}"#,
        // malformed now (§9: RFC 3339 validated, never generated)
        r#"{"id":1,"op":"splice","path":"notes/plan.md","now":"yesterday","edits":[{"target":{"fm_key":"title"},"edit":{"match":{"old":"a","new":"b"}}}]}"#,
        // receipt with an unknown key
        r#"{"id":1,"op":"splice","path":"notes/plan.md","receipt":{"path":"r.md","anchor":"r-1","note":"x"},"edits":[{"target":{"fm_key":"title"},"edit":{"match":{"old":"a","new":"b"}}}]}"#,
        // receipt anchor outside the one charset (§2.4 mint position)
        r#"{"id":1,"op":"splice","path":"notes/plan.md","receipt":{"path":"r.md","anchor":"r_42"},"edits":[{"target":{"fm_key":"title"},"edit":{"match":{"old":"a","new":"b"}}}]}"#,
        // mistyped if_root
        r#"{"id":1,"op":"splice","path":"notes/plan.md","if_root":5,"edits":[{"target":{"fm_key":"title"},"edit":{"match":{"old":"a","new":"b"}}}]}"#,
        // mistyped if_node_rev inside an edit
        r#"{"id":1,"op":"splice","path":"notes/plan.md","edits":[{"target":{"fm_key":"title"},"edit":{"match":{"old":"a","new":"b"}},"if_node_rev":7}]}"#,
    ] {
        assert_bad_request(&one(&root, req));
    }
    // path law still binds
    let frame = one(
        &root,
        r#"{"id":1,"op":"splice","path":"../out.md","edits":[{"target":{"fm_key":"title"},"edit":{"match":{"old":"a","new":"b"}}}]}"#,
    );
    assert_eq!(frame["error"]["code"], "bad_path", "{frame}");
}

/// §5.1 order: the world guard is checked FIRST — a wrong `if_root` refuses
/// `root_mismatch` (resync, full-width roots in the §8 comparison slots)
/// before any per-target resolution can answer for a bad ref.
#[test]
fn if_root_checked_first_root_mismatch() {
    let (_d, root) = s0();
    let frame = one(
        &root,
        &format!(
            r#"{{"id":9,"op":"splice","path":"notes/plan.md","if_root":"{R2}","edits":[{{"target":{{"hpath":[{{"h":"No"}},{{"h":"Such"}}]}},"edit":{{"match":{{"old":"a","new":"b"}}}}}}]}}"#
        ),
    );
    assert_eq!(frame["error"]["code"], "root_mismatch", "{frame}");
    assert_eq!(frame["error"]["recovery"], "resync");
    assert_eq!(frame["error"]["expected"], R2);
    assert_eq!(
        frame["error"]["actual"], R0,
        "world guard first, ref second"
    );
}

/// W-8 `put at:upsert` — the frontmatter-key create-or-replace verb, END-TO-END
/// through the live serve loop. Three cases (design): a key that EXISTS is
/// replaced in place; a key ABSENT from an existing frontmatter block inserts a
/// new line right after the opening fence; a file with NO frontmatter gets a
/// synthesized `---` block at byte 0. The server composes `{key}: {value}` from
/// the target key — the request carries only the VALUE, never a byte offset
/// (D-C1). Every disk state derived (real decode → validate → apply → commit).
#[test]
fn fm_upsert_creates_and_replaces() {
    // Case 1 — key EXISTS → its line is replaced. Same `title: Plan` line, span,
    // and revs as the frozen dry `fm_key` fixture (before fa77…, after fb49…):
    // the upsert of `Plan v2` produces the identical committed leaf.
    let (_d1, root) = workspace(&[("notes/a.md", "---\ntitle: Plan\n---\n# Body\n")]);
    let frame = one(
        &root,
        // U10 (decision 18): the key EXISTS, so this upsert mutates existing
        // content and carries its fingerprint. The rev is `node_rev_before` from
        // the armed fact asserted just below — the same derived value, not a
        // second source of truth. Cases 2 and 3 are BIRTHS of an absent key and
        // carry none: absence is their guard.
        r#"{"id":1,"op":"splice","path":"notes/a.md","edits":[{"target":{"fm_key":"title"},"edit":{"put":{"at":"upsert","text":"Plan v2"}},"if_node_rev":"fa77480c79a853bc"}]}"#,
    );
    assert_eq!(frame["ok"], true, "{frame}");
    assert_eq!(
        frame["body"]["armed"]["edits"][0],
        json!({"target":{"fm_key":"title"},
               "node_rev_before":"fa77480c79a853bc","node_rev_after":"fb49e9df2257fab8",
               "span_after":[4,18]}),
        "existing key replaced in place"
    );
    assert_eq!(
        std::fs::read_to_string(root.0.join("notes/a.md")).unwrap(),
        "---\ntitle: Plan v2\n---\n# Body\n",
        "case 1: existing key line replaced"
    );

    // Case 2 — key ABSENT, frontmatter EXISTS → the new line inserts right after
    // the opening `---` fence (first-key position); the block stays well-formed.
    let (_d2, root) = workspace(&[("notes/a.md", "---\ntitle: Plan\n---\n# Body\n")]);
    let frame = one(
        &root,
        r#"{"id":2,"op":"splice","path":"notes/a.md","edits":[{"target":{"fm_key":"status"},"edit":{"put":{"at":"upsert","text":"active"}}}]}"#,
    );
    assert_eq!(frame["ok"], true, "{frame}");
    assert_eq!(
        frame["body"]["armed"]["edits"][0]["span_after"],
        json!([4, 18]),
        "new key `status: active` at the first-key position (span [4,18])"
    );
    assert_eq!(
        std::fs::read_to_string(root.0.join("notes/a.md")).unwrap(),
        "---\nstatus: active\ntitle: Plan\n---\n# Body\n",
        "case 2: new key inserted after the opening fence"
    );

    // Case 3 — NO frontmatter → a `---` block is synthesized at byte 0; the
    // pre-existing body bytes are untouched (server-derived offset, not client).
    let (_d3, root) = workspace(&[("notes/b.md", "# Body\n\ntext\n")]);
    let frame = one(
        &root,
        r#"{"id":3,"op":"splice","path":"notes/b.md","edits":[{"target":{"fm_key":"title"},"edit":{"put":{"at":"upsert","text":"Plan"}}}]}"#,
    );
    assert_eq!(frame["ok"], true, "{frame}");
    assert_eq!(
        std::fs::read_to_string(root.0.join("notes/b.md")).unwrap(),
        "---\ntitle: Plan\n---\n# Body\n\ntext\n",
        "case 3: frontmatter block synthesized at byte 0"
    );
}

/// W-8 upsert guards (write path → `bad_request`): the verb is defined ONLY for
/// an `fm_key` target, and the value must be single-line — the server composes
/// `{key}: {value}`, so a non-fm_key target or a newline-bearing value is refused
/// loud before any byte moves.
#[test]
fn fm_upsert_guards_reject_bad_domain() {
    let (_d, root) = s0();
    // a non-fm_key target (hpath) is outside the verb's domain → bad_request,
    // even though the hpath itself resolves.
    let frame = one(
        &root,
        r#"{"id":1,"op":"splice","path":"notes/plan.md","edits":[{"target":{"hpath":[{"h":"Goals"}]},"edit":{"put":{"at":"upsert","text":"x"}}}]}"#,
    );
    assert_bad_request(&frame);
    // a newline-bearing value would forge extra frontmatter lines → bad_request.
    let frame = one(
        &root,
        r#"{"id":2,"op":"splice","path":"notes/plan.md","edits":[{"target":{"fm_key":"title"},"edit":{"put":{"at":"upsert","text":"a\nb"}}}]}"#,
    );
    assert_bad_request(&frame);
    // the file never moved under either refusal.
    assert_eq!(
        std::fs::read_to_string(root.0.join("notes/plan.md")).unwrap(),
        wsfix("s0/notes/plan.md"),
        "guards refuse before any byte lands"
    );
}
