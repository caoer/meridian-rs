//! D2-DISPATCH gates 1–5 + §3.1 raw-id emission through live serve
//! (byte-in/frame-out). Gate 1: §0.3 S0 toc/cat goldens + frozen R0 via real
//! corpus fold. Also: strict decode, caps ≡ armed set, S2/L22 `node_rev`,
//! CHARSET mint, resolve, root/diff epoch law.

use serde_json::{Value, json};
use std::io::Write as _;

/// Materialize a workspace from `(rel-path, bytes)` pairs.
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

/// The §0.3 S0 workspace: the two domain files plus the dot-ignored md
/// (addressable, never hashed — its presence proves the domain filter feeds
/// the ambient root).
fn s0() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let plan = wsfix("s0/notes/plan.md");
    let receipts = wsfix("s0/receipts/2026-07-18.md");
    workspace(&[
        ("notes/plan.md", &plan),
        ("receipts/2026-07-18.md", &receipts),
        (".github/README.md", "# CI notes\n"),
    ])
}

/// Feed `input` through the live serve loop; one parsed frame per output line.
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

const R0: &str = "b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9";

// ---------------------------------------------------------------------------
// Gate 1 — wsfix S0 toc/cat worked exchanges, byte-exact (contract §4.1/§4.2)
// ---------------------------------------------------------------------------

/// The §4.1 S0 toc worked exchange, every value computed — including the
/// frozen ambient root R0 via the real corpus fold.
#[test]
fn gate1_worked_s0_toc_exchange() {
    let (_d, root) = s0();
    let got = one(&root, r#"{"id":2,"op":"toc","path":"notes/plan.md"}"#);
    let expected = json!({
        "id":2,"ok":true,"body":{
         "path":"notes/plan.md","file_rev":"e3c4acaceb75b907",
         "root":R0,
         "nodes":[
          {"kind":"frontmatter","span":[0,20],"node_rev":"26796ebec5d0bf1a",
           "text_prefix_16b":"---\ntitle: Plan\n","keys":["title"]},
          {"kind":"heading","level":1,"hpath":[{"h":"Goals"}],"span":[20,136],
           "content_span":[28,136],"node_rev":"a6665baff294bd04","text_prefix_16b":"# Goals\n\nShip th"},
          {"kind":"heading","level":2,"hpath":[{"h":"Goals"},{"h":"Q3"}],"span":[49,72],
           "content_span":[55,72],"node_rev":"33d5b0e1b27cb48b","text_prefix_16b":"## Q3\n\nship by A"},
          {"kind":"heading","level":2,"hpath":[{"h":"Goals"},{"h":"Q4"}],"span":[72,136],
           "content_span":[78,136],"node_rev":"4b8bc385a58da0e0","text_prefix_16b":"## Q4\n\n- item on"}]}
    });
    assert_eq!(got, expected);
}

/// The §4.2 S0 cat worked exchange: full span bytes, heading-inclusive, rev
/// over precisely those bytes.
#[test]
fn gate1_worked_s0_cat_exchange() {
    let (_d, root) = s0();
    let got = one(
        &root,
        r#"{"id":3,"op":"cat","path":"notes/plan.md","sec":{"hpath":[{"h":"Goals"},{"h":"Q3"}]}}"#,
    );
    let expected = json!({
        "id":3,"ok":true,"body":{"span":[49,72],"node_rev":"33d5b0e1b27cb48b",
         "content":"## Q3\n\nship by August\n\n"}
    });
    assert_eq!(got, expected);
}

/// `sec` absent → the whole file and the rev IS the `file_rev` (§4.2).
#[test]
fn gate1_cat_whole_file_serves_file_rev() {
    let (_d, root) = s0();
    let got = one(&root, r#"{"id":9,"op":"cat","path":"notes/plan.md"}"#);
    assert_eq!(got["body"]["span"], json!([0, 136]));
    assert_eq!(got["body"]["node_rev"], "e3c4acaceb75b907");
    assert_eq!(got["body"]["content"].as_str().expect("content").len(), 136);
}

// ---------------------------------------------------------------------------
// Gate 2 — per-op unknown-field + unknown-value rejection, EVERY armed op
// (§3.2 strict server; hand-rolled decode, risk R5: fixture-driven)
// ---------------------------------------------------------------------------

fn assert_bad_request(frame: &Value) {
    assert_eq!(frame["ok"], false, "{frame}");
    assert_eq!(frame["error"]["code"], "bad_request", "{frame}");
    assert_eq!(frame["error"]["recovery"], "fix", "{frame}");
}

#[test]
fn gate2_unknown_field_rejected_per_armed_op() {
    let (_d, root) = s0();
    for req in [
        r#"{"id":1,"op":"hello","proto":1,"flavor":"x"}"#,
        r#"{"id":1,"op":"toc","path":"notes/plan.md","sec":{"fm_key":"title"}}"#,
        r#"{"id":1,"op":"cat","path":"notes/plan.md","kinds":["heading"]}"#,
        r#"{"id":1,"op":"extract","path":"notes/plan.md","from":"x.md"}"#,
        r#"{"id":1,"op":"resolve","from":"notes/plan.md","ref":"plan","path":"x.md"}"#,
    ] {
        let frame = one(&root, req);
        assert_bad_request(&frame);
        assert!(
            frame["error"]["message"]
                .as_str()
                .expect("strict decode names the field")
                .contains("unknown request field"),
            "{frame}"
        );
    }
}

#[test]
fn gate2_unknown_or_mistyped_value_rejected_per_armed_op() {
    let (_d, root) = s0();
    // hello: proto as string; toc: path as number; cat: sec with unknown key
    // inside; extract: kinds bearing an unknown value (D-C5, asserted in its
    // own fixture below); resolve: content as string.
    for req in [
        r#"{"id":1,"op":"hello","proto":"1"}"#,
        r#"{"id":1,"op":"toc","path":7}"#,
        r#"{"id":1,"op":"cat","path":"notes/plan.md","sec":{"anchor":"a","x":1}}"#,
        r#"{"id":1,"op":"cat","path":"notes/plan.md","sec":{"hpath":[{"h":"Goals","k":1}]}}"#,
        r#"{"id":1,"op":"resolve","from":"notes/plan.md","ref":"plan","content":"yes"}"#,
    ] {
        assert_bad_request(&one(&root, req));
    }
}

/// D-C5 (§4.3): an unknown value in `kinds` is `bad_request{unknown_kinds}`,
/// loud — the typo-silently-returns-nothing trap, killed.
#[test]
fn gate2_extract_unknown_kinds_echoed_loud() {
    let (_d, root) = s0();
    let frame = one(
        &root,
        r#"{"id":1,"op":"extract","path":"notes/plan.md","kinds":["heading","sections"]}"#,
    );
    assert_bad_request(&frame);
    assert_eq!(frame["error"]["unknown_kinds"], json!(["sections"]));
}

/// §3.2 proto discipline: a proto this sidecar does not speak is
/// `unsupported_proto` (respawn — a channel property), `supported` echoed.
#[test]
fn gate2_unsupported_proto() {
    let (_d, root) = s0();
    let frame = one(&root, r#"{"id":1,"op":"hello","proto":2}"#);
    assert_eq!(frame["error"]["code"], "unsupported_proto");
    assert_eq!(frame["error"]["recovery"], "respawn");
    assert_eq!(frame["error"]["supported"], json!([1]));
}

/// v2 §1 path law at the dispatch edge: absolute and `..`-bearing paths are
/// `bad_path`, echoed.
#[test]
fn gate2_bad_path_refused() {
    let (_d, root) = s0();
    for req in [
        r#"{"id":1,"op":"toc","path":"/etc/passwd"}"#,
        r#"{"id":1,"op":"cat","path":"../outside.md"}"#,
    ] {
        let frame = one(&root, req);
        assert_eq!(frame["error"]["code"], "bad_path", "{frame}");
        assert_eq!(frame["error"]["recovery"], "fix");
    }
}

// ---------------------------------------------------------------------------
// Gate 3 — hello caps ≡ the ARMED set exactly (§3.2 discovery honesty)
// ---------------------------------------------------------------------------

/// §3.2 caps ≡ frozen full armed list (including dotted field amendments).
#[test]
fn gate3_hello_caps_equal_frozen_full_list() {
    let (_d, root) = s0();
    let frame = one(
        &root,
        r#"{"id":1,"op":"hello","proto":1,"client":"md-cli/0.3"}"#,
    );
    let frozen_printed = [
        "toc",
        "cat",
        "extract",
        "resolve",
        "resolve.content",
        "links",
        "links.require_root",
        "splice",
        "splice.if_node_rev",
        "splice.if_root",
        "splice.dry",
        "splice.receipt",
        "splice.verdicts",
        "root",
        "diff",
        "sub",
    ];
    let expected = json!({
        "id":1,"ok":true,"body":{
            "proto":1,"server":"meridian-sidecar/2.0",
            "caps":frozen_printed,
            "root":R0}
    });
    assert_eq!(frame, expected);
}

/// S2/L22: `splice ∈ caps` ⇒ `node_rev` MUST on every toc/cat/extract node.
#[test]
fn gate3_s2l22_node_rev_must_when_splice_in_caps() {
    let (_d, root) = s0();
    let hello = one(&root, r#"{"id":1,"op":"hello","proto":1}"#);
    let caps = hello["body"]["caps"].as_array().expect("caps");
    assert!(caps.contains(&json!("splice")), "the law's trigger holds");
    let toc = one(&root, r#"{"id":2,"op":"toc","path":"notes/plan.md"}"#);
    let extract = one(&root, r#"{"id":3,"op":"extract","path":"notes/plan.md"}"#);
    for node in toc["body"]["nodes"]
        .as_array()
        .expect("toc nodes")
        .iter()
        .chain(extract["body"]["nodes"].as_array().expect("extract nodes"))
    {
        assert!(
            node["node_rev"].as_str().is_some_and(|r| r.len() == 16),
            "S2/L22: node_rev MUST ride every node while splice ∈ caps: {node}"
        );
    }
    let cat = one(&root, r#"{"id":4,"op":"cat","path":"notes/plan.md"}"#);
    assert!(
        cat["body"]["node_rev"].as_str().is_some(),
        "cat rev (S2/L22)"
    );
}

/// An op is in `caps` or answers `unknown_op` — including ops the wire
/// vocabulary already knows but this rung has not armed.
#[test]
fn gate3_unarmed_and_unknown_ops_answer_unknown_op() {
    let (_d, root) = s0();
    for req in [
        // Every §3.2 op is armed — only genuinely unknown names answer
        // unknown_op.
        r#"{"id":1,"op":"zap"}"#,
        r#"{"id":1,"op":"guard"}"#, // v1 guard, DELETED (D-C8) — never a name again
        r#"{"id":1,"op":"view_path"}"#, // published-view organ, DROPPED (§10.4) — never a name again
    ] {
        let frame = one(&root, req);
        assert_eq!(frame["error"]["code"], "unknown_op", "{frame}");
        assert_eq!(frame["error"]["recovery"], "fix");
    }
}

// ---------------------------------------------------------------------------
// Gate 4 — S2/L22: node_rev present on every toc/cat/extract node
// ---------------------------------------------------------------------------

#[test]
fn gate4_node_rev_on_every_toc_cat_extract_node() {
    let (_d, root) = s0();
    let toc = one(&root, r#"{"id":1,"op":"toc","path":"notes/plan.md"}"#);
    let toc_nodes = toc["body"]["nodes"].as_array().expect("toc nodes");
    assert!(!toc_nodes.is_empty());
    let extract = one(&root, r#"{"id":2,"op":"extract","path":"notes/plan.md"}"#);
    let ex_nodes = extract["body"]["nodes"].as_array().expect("extract nodes");
    assert!(!ex_nodes.is_empty());
    for node in toc_nodes.iter().chain(ex_nodes) {
        assert!(
            node["node_rev"].as_str().is_some_and(|r| r.len() == 16),
            "node_rev MUST ride every node: {node}"
        );
    }
    let cat = one(&root, r#"{"id":3,"op":"cat","path":"notes/plan.md"}"#);
    assert!(cat["body"]["node_rev"].as_str().is_some());
}

/// `extract.kinds` filters to exactly the asked kinds.
#[test]
fn gate4_extract_kinds_filter() {
    let (_d, root) = s0();
    let frame = one(
        &root,
        r#"{"id":1,"op":"extract","path":"notes/plan.md","kinds":["wikilink"]}"#,
    );
    let nodes = frame["body"]["nodes"].as_array().expect("nodes");
    assert_eq!(nodes.len(), 2, "plan S0 carries two wikilinks");
    for n in nodes {
        assert_eq!(n["kind"], "wikilink");
    }
}

// ---------------------------------------------------------------------------
// Gate 5 — CHARSET dispatch mint positions (§2.4, decision 011, re-homed here)
// ---------------------------------------------------------------------------

/// A `_`-bearing anchor in the armed mint position (`cat.sec`) is outside the
/// strict-plane grammar → `bad_request`, loud.
#[test]
fn gate5_charset_underscore_anchor_mint_position_bad_request() {
    let (_d, root) = s0();
    let frame = one(
        &root,
        r#"{"id":1,"op":"cat","path":"notes/plan.md","sec":{"anchor":"under_score"}}"#,
    );
    assert_bad_request(&frame);
    assert!(
        frame["error"]["message"]
            .as_str()
            .expect("charset refusal names the id")
            .contains("under_score"),
        "{frame}"
    );
}

/// The SAME id on the walk plane is not a grammar error — the app never
/// indexes it, so the walk misses (`ref_not_found`, stage 2): both
/// dispositions conform (decision 013 consequence 3).
#[test]
fn gate5_charset_underscore_on_walk_plane_misses_silently() {
    let (_d, root) = s0();
    let frame = one(
        &root,
        r##"{"id":1,"op":"resolve","from":"notes/plan.md","ref":"#^under_score"}"##,
    );
    assert_eq!(frame["error"]["code"], "ref_not_found");
    assert_eq!(frame["error"]["recovery"], "refresh");
    assert_eq!(frame["error"]["stage"], 2);
    assert_eq!(frame["error"]["dest"], "notes/plan.md");
}

// ---------------------------------------------------------------------------
// §3.1 emission — the B2 law through the LIVE loop (id:null + id_raw verbatim)
// ---------------------------------------------------------------------------

fn assert_id_raw(frame: &Value, lexeme: &str) {
    assert_eq!(frame["id"], Value::Null, "{frame}");
    assert_eq!(frame["ok"], false);
    assert_eq!(frame["error"]["code"], "bad_request");
    assert_eq!(frame["error"]["recovery"], "fix");
    assert_eq!(frame["error"]["id_raw"], lexeme, "{frame}");
}

#[test]
fn emission_3_1_bad_id_lexemes_answer_id_null_plus_id_raw_verbatim() {
    let (_d, root) = s0();
    for (line, lexeme) in [
        (r#"{"id":"7","op":"toc","path":"notes/plan.md"}"#, "\"7\""),
        (r#"{"id":3.5,"op":"toc","path":"notes/plan.md"}"#, "3.5"),
        (r#"{"id":3e0,"op":"toc","path":"notes/plan.md"}"#, "3e0"),
        (r#"{"id":-1,"op":"toc","path":"notes/plan.md"}"#, "-1"),
        (r#"{"id":null,"op":"toc","path":"notes/plan.md"}"#, "null"),
        (
            r#"{"id":9007199254740992,"op":"toc","path":"notes/plan.md"}"#,
            "9007199254740992",
        ),
        (
            r#"{"id":18446744073709551616,"op":"toc","path":"notes/plan.md"}"#,
            "18446744073709551616",
        ),
    ] {
        assert_id_raw(&one(&root, line), lexeme);
    }
}

/// An id-less frame CARRYING `op` is a legal single-shot request (shell-pipe
/// debuggability); its response frame serializes `id:null`.
#[test]
fn emission_3_1_idless_request_is_served() {
    let (_d, root) = s0();
    let frame = one(&root, r#"{"op":"hello","proto":1}"#);
    assert_eq!(frame["id"], Value::Null);
    assert_eq!(frame["ok"], true);
}

/// Inbound frames that are not requests — no `op` — are protocol misuse:
/// `bad_frame`, respawn class, id:null; and a non-JSON-object line likewise.
#[test]
fn emission_3_1_non_request_frames_answer_bad_frame() {
    let (_d, root) = s0();
    for line in [
        r#"{"sub":"root"}"#,
        r#"{"id":7}"#,
        r#"{"id":7,"ok":true,"body":{}}"#,
        "[1,2,3]",
        "not json",
    ] {
        let frame = one(&root, line);
        assert_eq!(frame["id"], Value::Null, "{line}");
        assert_eq!(frame["error"]["code"], "bad_frame", "{line}");
        assert_eq!(frame["error"]["recovery"], "respawn");
    }
}

/// Pipelining: one response per request, order preserved, blank lines skipped.
#[test]
fn emission_3_1_pipelined_frames_correlate_in_order() {
    let (_d, root) = s0();
    let input = "{\"id\":1,\"op\":\"hello\",\"proto\":1}\n\n{\"id\":2,\"op\":\"toc\",\"path\":\"notes/plan.md\"}\n{\"id\":\"x\",\"op\":\"toc\"}\n";
    let frames = serve(&root, input);
    assert_eq!(frames.len(), 3);
    assert_eq!(frames[0]["id"], 1);
    assert_eq!(frames[1]["id"], 2);
    assert_eq!(frames[2]["id"], Value::Null);
    assert_eq!(frames[2]["error"]["id_raw"], "\"x\"");
}

// ---------------------------------------------------------------------------
// §8 error split on the read arms (file_not_found / ref_not_found)
// ---------------------------------------------------------------------------

#[test]
fn errors_file_not_found_vs_ref_not_found() {
    let (_d, root) = s0();
    let gone = one(&root, r#"{"id":1,"op":"toc","path":"notes/absent.md"}"#);
    assert_eq!(gone["error"]["code"], "file_not_found");
    assert_eq!(gone["error"]["recovery"], "env");
    assert_eq!(gone["error"]["path"], "notes/absent.md");

    let dangling = one(
        &root,
        r#"{"id":2,"op":"cat","path":"notes/plan.md","sec":{"hpath":[{"h":"Goals"},{"h":"Q9"}]}}"#,
    );
    assert_eq!(dangling["error"]["code"], "ref_not_found");
    assert_eq!(dangling["error"]["recovery"], "refresh");
}

/// The dot-ignored md is addressable (`hash ⊂ addressable`, §12.1): `toc` on
/// `.github/README.md` works while its bytes never enter the ambient root
/// (which gate-1 pins to R0 with this file present in the workspace).
#[test]
fn errors_dot_ignored_md_stays_addressable() {
    let (_d, root) = s0();
    let frame = one(&root, r#"{"id":1,"op":"toc","path":".github/README.md"}"#);
    assert_eq!(frame["ok"], true, "{frame}");
    assert_eq!(frame["body"]["root"], R0);
}

// ---------------------------------------------------------------------------
// §4.5 resolve — worked frames (S2 values where the fixture exists in-tree)
// ---------------------------------------------------------------------------

/// Worked ids 70/71 (case-insensitive walk on computed spans), 73 (stage-2
/// miss carries `dest`), 74 (stage-1 miss carries none) — §4.5, S2 plan.
#[test]
fn resolve_worked_s2_frames() {
    let plan = wsfix("s2/notes/plan.md");
    let (_d, root) = workspace(&[("notes/plan.md", &plan)]);
    let hit = one(
        &root,
        r#"{"id":70,"op":"resolve","from":"notes/plan.md","ref":"plan#Goals#Q3"}"#,
    );
    assert_eq!(
        hit,
        json!({"id":70,"ok":true,"body":{"dest":"notes/plan.md","span":[49,75]}})
    );
    let ci = one(
        &root,
        r#"{"id":71,"op":"resolve","from":"notes/plan.md","ref":"plan#goals#q3"}"#,
    );
    assert_eq!(
        ci,
        json!({"id":71,"ok":true,"body":{"dest":"notes/plan.md","span":[49,75]}})
    );
    let miss2 = one(
        &root,
        r#"{"id":73,"op":"resolve","from":"notes/plan.md","ref":"plan#Goals#Q9"}"#,
    );
    assert_eq!(
        miss2,
        json!({"id":73,"ok":false,"error":{"code":"ref_not_found","recovery":"refresh",
                "stage":2,"dest":"notes/plan.md"}})
    );
    let miss1 = one(
        &root,
        r#"{"id":74,"op":"resolve","from":"notes/plan.md","ref":"roadmap"}"#,
    );
    assert_eq!(
        miss1,
        json!({"id":74,"ok":false,"error":{"code":"ref_not_found","recovery":"refresh","stage":1}})
    );
}

/// `content:true` additionally returns the fragment bytes — still no rev
/// (D-C2: the response type has no rev field to return).
#[test]
fn resolve_content_true_returns_bytes_and_never_a_rev() {
    let (_d, root) = s0();
    let frame = one(
        &root,
        r##"{"id":1,"op":"resolve","from":"notes/plan.md","ref":"#Goals#Q3","content":true}"##,
    );
    assert_eq!(frame["ok"], true);
    assert_eq!(frame["body"]["content"], "## Q3\n\nship by August\n\n");
    let body = frame["body"].as_object().expect("body");
    for key in ["node_rev", "file_rev", "rev"] {
        assert!(!body.contains_key(key), "mint partition: no `{key}`");
    }
}

// ---------------------------------------------------------------------------
// D3-DELTA — root/diff arms + §7.1 per-daemon-epoch law through the LIVE loop
// ---------------------------------------------------------------------------

/// v2 §4.7 root: the current computed root + this epoch's seq (0 — no batch
/// has been emitted this epoch; rung 4 moves it).
#[test]
fn d3_root_op_serves_current_root_and_epoch_seq() {
    let (_d, root) = s0();
    let got = one(&root, r#"{"id":90,"op":"root"}"#);
    assert_eq!(got, json!({"id":90,"ok":true,"body":{"root":R0,"seq":0}}));
}

/// §7.3: same-root diff → empty batches; else `root_unknown` → resync.
#[test]
fn d3_diff_empty_or_root_unknown_truthfully() {
    let (_d, root) = s0();
    let same = one(
        &root,
        &format!(r#"{{"id":95,"op":"diff","from_root":"{R0}","to_root":"{R0}"}}"#),
    );
    assert_eq!(same, json!({"id":95,"ok":true,"body":{"batches":[]}}));

    let r1 = "b3:10769ae1c77f5646750f3f52df2d055156b411145a02b8361ecd32af1357a1b7";
    let stale = one(
        &root,
        &format!(r#"{{"id":96,"op":"diff","from_root":"{R0}","to_root":"{r1}"}}"#),
    );
    assert_eq!(stale["ok"], false);
    assert_eq!(stale["error"]["code"], "root_unknown");
    assert_eq!(stale["error"]["recovery"], "resync");
}

/// §7.1: restart = new epoch; seq resets; no cross-epoch correlation.
#[test]
fn d3_restart_boundary_never_correlates_epochs() {
    let (_d, root) = s0();
    let e1 = one(&root, r#"{"id":1,"op":"root"}"#);
    assert_eq!(e1["body"]["seq"], 0);
    // restart: fresh serve call
    let e2 = one(&root, r#"{"id":2,"op":"root"}"#);
    assert_eq!(e2["body"]["seq"], 0, "seq is born 0 in every epoch");
    let same = one(
        &root,
        &format!(r#"{{"id":3,"op":"diff","from_root":"{R0}","to_root":"{R0}"}}"#),
    );
    assert_eq!(same["body"]["batches"], json!([]));
}

/// Gate 5 (every op ships rejection fixtures): strict decode on the newly
/// armed ops — root takes NO fields; diff requires string roots and rejects
/// unknown fields loudly.
#[test]
fn d3_gate5_root_diff_strict_decode_rejections() {
    let (_d, root) = s0();
    for req in [
        r#"{"id":1,"op":"root","path":"notes/plan.md"}"#,
        r#"{"id":1,"op":"diff","from_root":"b3:00","to_root":"b3:00","seq":1}"#,
    ] {
        let frame = one(&root, req);
        assert_bad_request(&frame);
        assert!(
            frame["error"]["message"]
                .as_str()
                .expect("strict decode names the field")
                .contains("unknown request field"),
            "{frame}"
        );
    }
    for req in [
        r#"{"id":1,"op":"diff","from_root":"b3:00"}"#,
        r#"{"id":1,"op":"diff","from_root":7,"to_root":"b3:00"}"#,
    ] {
        assert_bad_request(&one(&root, req));
    }
}

// ---------------------------------------------------------------------------
// Q5-LINKS — §4.6 edge map + §10.1 triple + §10.2 refusal (pack §8 gates),
// all through the live serve loop over the §0.3 S2 world
// ---------------------------------------------------------------------------

const R2: &str = "b3:83b4ba591c0291d9f2a05428cac38e5820858fbb9c47720ab352344ddccc8f68";

/// §6.3 frozen receipt lines (also pinned in `crates/receipt` and
/// `testsuite/tests/delta_e3e4.rs`) — the S2 receipts state derives from the
/// committed S0 bytes + these, never hand-tuned.
const E3_LINE: &str = "- splice notes/plan.md id=42 actor=agent:b0864fb2 now=2026-07-18T20:31:04Z root_before=b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9 edits=1 Goals>Q3 match 33d5b0e1b27cb48b->41f643f034e5681f ^r-000042";
const E4_LINE: &str = "- splice notes/plan.md id=57 actor=agent:b0864fb2 now=2026-07-18T20:33:41Z root_before=b3:10769ae1c77f5646750f3f52df2d055156b411145a02b8361ecd32af1357a1b7 edits=1 Goals>Q4 put:end 4b8bc385a58da0e0->f43203a1f0b4c9a3 ^r-000043";

/// The §0.3 S2 workspace: committed s2 plan + derived S2 receipts (byte-count
/// guarded) + the dot-ignored md (addressable, never hashed — its absence
/// from the edge map proves the §12 domain filter feeds the fact plane).
fn s2() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let plan = wsfix("s2/notes/plan.md");
    let receipts = format!(
        "{}{E3_LINE}\n{E4_LINE}\n",
        wsfix("s0/receipts/2026-07-18.md")
    );
    assert_eq!(receipts.len(), 474, "S2 receipts bytes (§0.3)");
    workspace(&[
        ("notes/plan.md", &plan),
        ("receipts/2026-07-18.md", &receipts),
        (".github/README.md", "# CI notes\n\nsee [[plan]]\n"),
    ])
}

/// Gate 1 (pack §8): the §10.2 worked refusal, byte-exact through the live
/// loop — the client demands R0, the world is at R2. `required` echoes the
/// demand; `as_of_root`/`live_root` are COMPUTED by the real corpus fold and
/// must land on the frozen R2. Retry class, no message.
#[test]
fn q5_gate1_worked_stale_view_refusal_computed() {
    let (_d, root) = s2();
    let got = one(
        &root,
        &format!(r#"{{"id":81,"op":"links","path":"notes/plan.md","require_root":"{R0}"}}"#),
    );
    let expected = json!({
        "id":81,"ok":false,"error":{"code":"stale_view","recovery":"retry",
         "required":R0,
         "as_of_root":R2,
         "live_root":R2}
    });
    assert_eq!(got, expected);
}

/// The §4.6 worked exchange through the live loop: the edge map value-for-
/// value (resolved basename edge + dangling ref, per-edge counts) with the
/// triple's roots computed onto the frozen R2. `changes_seq` is this serve
/// epoch's truthful counter — 0, no batch emitted yet.
#[test]
fn q5_worked_links_exchange_computed() {
    let (_d, root) = s2();
    let got = one(&root, r#"{"id":80,"op":"links","path":"notes/plan.md"}"#);
    let expected = json!({
        "id":80,"ok":true,"body":{
         "as_of_root":R2,
         "live_root":R2,
         "changes_seq":0,
         "files":{"notes/plan.md":{
           "resolved":{"receipts/2026-07-18.md":1},
           "unresolved":{"roadmap":1}}}}
    });
    assert_eq!(got, expected);
}

/// Gate 3 (pack §8): the whole-corpus edge map — `path` absent. Exactly the
/// two §12-domain files: link-less files carry empty maps (never vanish);
/// the dot-ignored md never edges and is never a destination (the domain
/// filter feeds the fact plane, §10.3).
#[test]
fn q5_gate3_whole_corpus_edge_map() {
    let (_d, root) = s2();
    let got = one(&root, r#"{"id":82,"op":"links"}"#);
    let expected = json!({
        "id":82,"ok":true,"body":{
         "as_of_root":R2,
         "live_root":R2,
         "changes_seq":0,
         "files":{
           "notes/plan.md":{
             "resolved":{"receipts/2026-07-18.md":1},
             "unresolved":{"roadmap":1}},
           "receipts/2026-07-18.md":{"resolved":{},"unresolved":{}}}}
    });
    assert_eq!(got, expected);
}

/// Gate 2 (pack §8), dispatch side of the honest-tense law: the corpus
/// mutates between reads and the un-strict read never refuses — it serves
/// the new world's triple. Staleness becomes an error only through the
/// `require_root` opt-in; no assertion bounds the two roots' distance.
#[test]
fn q5_gate2_honest_tense_unstrict_read_never_refuses() {
    let (dir, root) = s2();
    let before = one(&root, r#"{"id":83,"op":"links","path":"notes/plan.md"}"#);
    assert_eq!(before["body"]["as_of_root"], R2);
    // the concurrent splice, simulated at the disk seam
    std::fs::write(
        dir.path().join("notes/plan.md"),
        "# Goals\n\n- blocked on [[roadmap]] and [[roadmap]]\n",
    )
    .expect("mutate corpus");
    let after = one(&root, r#"{"id":84,"op":"links","path":"notes/plan.md"}"#);
    assert_eq!(after["ok"], true, "drift never errors un-strictly: {after}");
    assert_ne!(after["body"]["as_of_root"], json!(R2), "the world moved");
    assert_eq!(
        after["body"]["files"]["notes/plan.md"]["unresolved"]["roadmap"], 2,
        "per-edge counts over the moved world"
    );
}

/// `require_root` naming the CURRENT root is the strict read succeeding —
/// the knob demands a tense, it does not forbid reading.
#[test]
fn q5_require_root_current_succeeds() {
    let (_d, root) = s2();
    let got = one(
        &root,
        &format!(r#"{{"id":85,"op":"links","path":"notes/plan.md","require_root":"{R2}"}}"#),
    );
    assert_eq!(got["ok"], true, "{got}");
    assert_eq!(got["body"]["as_of_root"], R2);
}

/// Gate 4 (pack §8): strict-decode rejection fixtures for the newly armed op
/// — unknown field (named loud), mistyped values, and the §1 path law on the
/// optional `path`. Every op ships them.
#[test]
fn q5_gate4_links_strict_decode_rejections() {
    let (_d, root) = s2();
    for req in [
        r#"{"id":1,"op":"links","path":"notes/plan.md","from":"x.md"}"#,
        r#"{"id":1,"op":"links","require":"b3:00"}"#,
    ] {
        let frame = one(&root, req);
        assert_bad_request(&frame);
        assert!(
            frame["error"]["message"]
                .as_str()
                .expect("strict decode names the field")
                .contains("unknown request field"),
            "{frame}"
        );
    }
    for req in [
        r#"{"id":1,"op":"links","path":7}"#,
        r#"{"id":1,"op":"links","require_root":5}"#,
    ] {
        assert_bad_request(&one(&root, req));
    }
    let frame = one(&root, r#"{"id":1,"op":"links","path":"../outside.md"}"#);
    assert_eq!(frame["error"]["code"], "bad_path", "{frame}");
}

/// A `path` outside the corpus the fact op serves — a missing file, or an
/// on-disk file outside the §12 hash domain — is `file_not_found`, path
/// echoed (env class). The fact plane answers for the domain its root
/// stamps; the wider addressable set is the walk plane's (§4.5/§10.3).
#[test]
fn q5_links_path_outside_domain_is_file_not_found() {
    let (_d, root) = s2();
    for path in [".github/README.md", "notes/missing.md"] {
        let frame = one(
            &root,
            &format!(r#"{{"id":1,"op":"links","path":"{path}"}}"#),
        );
        assert_eq!(frame["error"]["code"], "file_not_found", "{frame}");
        assert_eq!(frame["error"]["recovery"], "env");
        assert_eq!(frame["error"]["path"], path);
    }
}
