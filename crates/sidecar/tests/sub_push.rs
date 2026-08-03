//! T5-SUB gate fixtures — the push path through the live serve loop: the
//! §4.7 reserved shape live (`sub` → Root-shaped ack, then Notification
//! frames), replay ≡ live at the TRANSPORT (gate 1: pushed frames
//! byte-identical to `diff`'s batches — one shape, two transports, §7.3),
//! and the §7.1 late-law residue executable (gate 3: stale `from_seq` →
//! `root_unknown`, the ruled shape — pinned as data, no frozen worked value
//! exercises the boundary grain).

use serde_json::{Value, json};
use std::io::Write as _;

const R0: &str = "b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9";
const R1: &str = "b3:10769ae1c77f5646750f3f52df2d055156b411145a02b8361ecd32af1357a1b7";
const R2: &str = "b3:83b4ba591c0291d9f2a05428cac38e5820858fbb9c47720ab352344ddccc8f68";

fn wsfix(rel: &str) -> String {
    std::fs::read_to_string(testsuite::wsfix_dir().join(rel))
        .unwrap_or_else(|e| panic!("wsfix fixture {rel}: {e}"))
}

fn s0() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    for (rel, bytes) in [
        ("notes/plan.md", wsfix("s0/notes/plan.md")),
        ("receipts/2026-07-18.md", wsfix("s0/receipts/2026-07-18.md")),
    ] {
        let abs = dir.path().join(rel);
        std::fs::create_dir_all(abs.parent().expect("parent")).expect("mkdir");
        let mut f = std::fs::File::create(&abs).expect("create");
        f.write_all(bytes.as_bytes()).expect("write");
    }
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    (dir, root)
}

/// Serve raw: the RAW output lines (the byte-identity gate compares line
/// bytes, not parsed values) plus their parsed forms.
fn serve_raw(root: &fs::WorkspaceRoot, input: &str) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    sidecar::serve(root, input.as_bytes(), &mut out, &[]).expect("serve");
    String::from_utf8(out)
        .expect("frames are UTF-8")
        .lines()
        .map(|l| {
            (
                l.to_string(),
                serde_json::from_str(l).expect("frame parses"),
            )
        })
        .collect()
}

const E3_REQ: &str = r#"{"id":42,"op":"splice","path":"notes/plan.md","actor":"agent:b0864fb2","now":"2026-07-18T20:31:04Z","receipt":{"path":"receipts/2026-07-18.md","anchor":"r-000042"},"if_root":"b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9","edits":[{"target":{"hpath":[{"h":"Goals"},{"h":"Q3"}]},"edit":{"match":{"old":"ship by August","new":"ship by September"}},"if_node_rev":"33d5b0e1b27cb48b"}]}"#;
// U10 (decision 18): E4 mutates existing content, so it carries Q4's
// fingerprint the way E3 already carried Q3's. The value is the before-rev the
// §6.3 receipt line for this very edit records (`Goals>Q4 put:end
// 4b8bc385a58da0e0->…`) — E3 above touches Q3 only, so Q4 still stands at its
// s0 rev when this frame arrives.
const E4_REQ: &str = r#"{"id":57,"op":"splice","path":"notes/plan.md","actor":"agent:b0864fb2","now":"2026-07-18T20:33:41Z","receipt":{"path":"receipts/2026-07-18.md","anchor":"r-000043"},"edits":[{"target":{"hpath":[{"h":"Goals"},{"h":"Q4"}]},"edit":{"put":{"at":"end","text":"- new item\n"}},"if_node_rev":"4b8bc385a58da0e0"}]}"#;

/// A frame is a Notification iff it carries NO `id` key (§3.1
/// classification) — here that means the `{"delta":{…}}` push frames.
fn is_notification(v: &Value) -> bool {
    !v.as_object().expect("frame object").contains_key("id")
}

/// GATE 1 — sub-then-splice, one session: `sub{from_seq:0}` acks with the
/// ruled Root-shaped anchor, each commit pushes one Notification AFTER its
/// response (§4.7 order), and the pushed LINES are byte-identical to the
/// serialization of `diff(R0,R2)`'s batches — one shape, two transports.
#[test]
fn sub_then_splice_pushed_frames_byte_identical_to_diff() {
    let (_d, root) = s0();
    let session = format!(
        "{sub}\n{E3_REQ}\n{E4_REQ}\n{{\"id\":95,\"op\":\"diff\",\"from_root\":\"{R0}\",\"to_root\":\"{R2}\"}}\n",
        sub = r#"{"id":100,"op":"sub","from_seq":0}"#,
    );
    let frames = serve_raw(&root, &session);
    // ack, E3-resp, notif-1, E4-resp, notif-2, diff-resp — six frames
    assert_eq!(frames.len(), 6, "six frames: {frames:?}");
    let (ref _ack_raw, ref ack) = frames[0];
    assert_eq!(
        *ack,
        json!({"id":100,"ok":true,"body":{"root":R0,"seq":0}}),
        "the ruled ack: the §4.7 Root body as the subscription anchor"
    );
    assert_eq!(
        frames[1].1["id"], 42,
        "response BEFORE the push (§4.7 order)"
    );
    assert!(is_notification(&frames[2].1), "then the Notification");
    assert_eq!(frames[3].1["id"], 57);
    assert!(is_notification(&frames[4].1));
    // THE byte-identity, through the TYPED vocabulary (an untyped Value
    // round-trip would alphabetize keys — a test artifact, not the wire):
    // the diff response line deserializes into wire types, and each typed
    // batch re-serializes to EXACTLY the pushed line's bytes — same type,
    // same serializer, same stored object. No second dialect.
    let typed: wire::Response = serde_json::from_str(&frames[5].0).expect("diff response types");
    let wire::ResponsePayload::Body {
        body: wire::ResponseBody::Diff { ref batches },
    } = typed.payload
    else {
        panic!("diff response carries batches")
    };
    assert_eq!(batches.len(), 2);
    assert_eq!(
        frames[2].0,
        serde_json::to_string(&batches[0]).unwrap(),
        "pushed E3 ≡ diff batch 1, byte-for-byte"
    );
    assert_eq!(
        frames[4].0,
        serde_json::to_string(&batches[1]).unwrap(),
        "pushed E4 ≡ diff batch 2, byte-for-byte"
    );
    // and the pushed frames ARE the §7.1 printed Deltas (seq/roots spot pin)
    assert_eq!(frames[2].1["delta"]["seq"], 1);
    assert_eq!(frames[2].1["delta"]["root_after"], R1);
    assert_eq!(frames[4].1["delta"]["seq"], 2);
    assert_eq!(frames[4].1["delta"]["root_after"], R2);
}

/// Mid-epoch subscription replays first: `sub{from_seq:0}` AFTER E3 acks at
/// `{R1, seq:1}` and immediately replays frame 1, then E4 pushes live —
/// replay and live ride the same flush, so their bytes cannot differ.
#[test]
fn mid_epoch_sub_replays_then_goes_live() {
    let (_d, root) = s0();
    let session = format!(
        "{E3_REQ}\n{sub}\n{E4_REQ}\n",
        sub = r#"{"id":101,"op":"sub","from_seq":0}"#,
    );
    let frames = serve_raw(&root, &session);
    // E3-resp, ack, replay-notif-1, E4-resp, live-notif-2
    assert_eq!(frames.len(), 5, "{frames:?}");
    assert_eq!(frames[0].1["id"], 42);
    assert_eq!(
        frames[1].1,
        json!({"id":101,"ok":true,"body":{"root":R1,"seq":1}}),
        "anchor tense at subscription time"
    );
    assert!(is_notification(&frames[2].1), "replay of retained frame 1");
    assert_eq!(frames[2].1["delta"]["seq"], 1);
    assert_eq!(frames[3].1["id"], 57);
    assert!(is_notification(&frames[4].1), "live frame 2");
    assert_eq!(frames[4].1["delta"]["seq"], 2);
}

/// `from_seq == current` is legal, live-only: no replay frames precede the
/// next commit's push. (Boundary grain pinned AS DATA — no frozen worked
/// value exercises it; advisor ledger.)
#[test]
fn from_seq_at_current_is_live_only() {
    let (_d, root) = s0();
    let session = format!(
        "{E3_REQ}\n{sub}\n{E4_REQ}\n",
        sub = r#"{"id":102,"op":"sub","from_seq":1}"#,
    );
    let frames = serve_raw(&root, &session);
    // E3-resp, ack, E4-resp, live-notif-2 — NO replay of frame 1
    assert_eq!(frames.len(), 4, "{frames:?}");
    assert_eq!(frames[1].1["body"], json!({"root":R1,"seq":1}));
    assert_eq!(frames[2].1["id"], 57);
    assert_eq!(
        frames[3].1["delta"]["seq"], 2,
        "live only, nothing replayed"
    );
}

/// GATE 3 — the §7.1 late-law residue, executable: a `from_seq` this
/// epoch's ring cannot anchor (ahead of current — a prior epoch's counter
/// or invention) answers `root_unknown` → resync, catch up by diff-by-root.
/// Never wrong data, never a cross-epoch seq comparison; the refusal
/// registers NO subscription (a later commit pushes nothing).
#[test]
fn stale_from_seq_refuses_root_unknown_resync() {
    let (_d, root) = s0();
    let session = format!(
        "{stale}\n{empty}\n{E3_REQ}\n",
        stale = r#"{"id":103,"op":"sub","from_seq":5}"#,
        empty = r#"{"id":104,"op":"sub","from_seq":1}"#, // also unanchorable: ring empty, current 0
    );
    let frames = serve_raw(&root, &session);
    // two refusals + the E3 response — and NO notification frames at all
    assert_eq!(
        frames.len(),
        3,
        "no sub registered, nothing pushed: {frames:?}"
    );
    for refusal in [&frames[0].1, &frames[1].1] {
        assert_eq!(refusal["ok"], false, "{refusal}");
        assert_eq!(refusal["error"]["code"], "root_unknown", "{refusal}");
        assert_eq!(refusal["error"]["recovery"], "resync", "{refusal}");
    }
    assert_eq!(
        frames[2].1["id"], 42,
        "the world moves on; the client resyncs"
    );
}

/// Repeated `sub` in one session ADDS a subscription (advisor-ledgered as
/// data: refusing or replacing would invent a restriction — the duplicate
/// frames are the client's own request): one commit, two Notifications.
#[test]
fn repeated_sub_adds_a_subscription() {
    let (_d, root) = s0();
    let session = format!(
        "{s1}\n{s2}\n{E3_REQ}\n",
        s1 = r#"{"id":105,"op":"sub","from_seq":0}"#,
        s2 = r#"{"id":106,"op":"sub","from_seq":0}"#,
    );
    let frames = serve_raw(&root, &session);
    // ack, ack, E3-resp, notif, notif
    assert_eq!(frames.len(), 5, "{frames:?}");
    assert!(is_notification(&frames[3].1) && is_notification(&frames[4].1));
    assert_eq!(frames[3].0, frames[4].0, "same stored object, same bytes");
}

/// GATE 2 — strict-decode rejections for the newly armed op: `from_seq`
/// required non-negative integer; unknown fields refused by name (every op
/// ships them).
#[test]
fn sub_strict_decode_rejections() {
    let (_d, root) = s0();
    let mut out = Vec::new();
    let input = concat!(
        r#"{"id":1,"op":"sub"}"#,
        "\n",
        r#"{"id":2,"op":"sub","from_seq":"zero"}"#,
        "\n",
        r#"{"id":3,"op":"sub","from_seq":-1}"#,
        "\n",
        r#"{"id":4,"op":"sub","from_seq":0,"filter":"deltas"}"#,
        "\n",
    );
    sidecar::serve(&root, input.as_bytes(), &mut out, &[]).expect("serve");
    let frames: Vec<Value> = String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(frames.len(), 4);
    for frame in &frames {
        assert_eq!(frame["ok"], false, "{frame}");
        assert_eq!(frame["error"]["code"], "bad_request", "{frame}");
        assert_eq!(frame["error"]["recovery"], "fix", "{frame}");
    }
    assert!(
        frames[3]["error"]["message"]
            .as_str()
            .expect("named")
            .contains("unknown request field"),
        "{}",
        frames[3]
    );
}
