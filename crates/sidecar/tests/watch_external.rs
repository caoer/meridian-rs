//! F5-WATCH: external changes through live serve (mutate between lines).
//! External deltas: `actor`/`now` key-absent (never null); `seq` at detection;
//! rename by byte-identity (pinned as data).

use serde_json::Value;
use std::io::{self, BufRead, Read, Write as _};

const R0: &str = "b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9";

/// The corpus root the external E3-only edit lands on — plan at S1,
/// receipts still at S0 (no splice ran, so no receipt line): a valid
/// non-frozen root, computed through the engine's own fold (the standing
/// oracle pattern).
fn r1_external() -> String {
    let plan_s1 = wsfix("s0/notes/plan.md").replace("ship by August", "ship by September");
    let receipts_s0 = wsfix("s0/receipts/2026-07-18.md");
    let files: Vec<(&str, &[u8])> = vec![
        ("notes/plan.md", plan_s1.as_bytes()),
        ("receipts/2026-07-18.md", receipts_s0.as_bytes()),
    ];
    model::merkle_root(&files, 0).0
}

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

/// One scripted step: a request line to feed, or a workspace mutation to
/// run before the next read — the "human in Obsidian".
enum Step {
    Line(String),
    Mutate(Box<dyn FnMut()>),
}

/// A `BufRead` whose reads execute the script: mutations run at the exact
/// between-lines boundary the serve loop reads at.
struct Scripted {
    steps: std::collections::VecDeque<Step>,
    buf: Vec<u8>,
    pos: usize,
}

impl Scripted {
    fn new(steps: Vec<Step>) -> Self {
        Scripted {
            steps: steps.into(),
            buf: Vec::new(),
            pos: 0,
        }
    }
}

impl Read for Scripted {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let available = self.fill_buf()?;
        let n = available.len().min(out.len());
        out[..n].copy_from_slice(&available[..n]);
        self.consume(n);
        Ok(n)
    }
}

impl BufRead for Scripted {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        while self.pos >= self.buf.len() {
            match self.steps.pop_front() {
                None => return Ok(&[]),
                Some(Step::Mutate(mut f)) => f(),
                Some(Step::Line(l)) => {
                    self.buf = format!("{l}\n").into_bytes();
                    self.pos = 0;
                }
            }
        }
        Ok(&self.buf[self.pos..])
    }
    fn consume(&mut self, amt: usize) {
        self.pos += amt;
    }
}

fn line(s: &str) -> Step {
    Step::Line(s.to_string())
}

fn serve_scripted(root: &fs::WorkspaceRoot, steps: Vec<Step>) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    sidecar::serve(root, Scripted::new(steps), &mut out, &[]).expect("serve");
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

fn is_notification(v: &Value) -> bool {
    !v.as_object().expect("frame object").contains_key("id")
}

/// Write S1 plan bytes out-of-band — byte-for-byte the E3 edit, but landed
/// by "a human", not a splice.
fn external_e3_edit(root: &fs::WorkspaceRoot) -> Box<dyn FnMut()> {
    let path = root.0.join("notes/plan.md");
    Box::new(move || {
        let s1 = wsfix("s0/notes/plan.md").replace("ship by August", "ship by September");
        std::fs::write(&path, s1).expect("external edit");
    })
}

/// GATE 1 — out-of-band edit: the watcher emits ONE Delta with `actor`/`now`
/// ABSENT (key-absence, §7.1 verbatim law) and the correct derived roots;
/// `root` reflects the advanced seq; a detection cycle finding nothing emits
/// nothing (seq stable across the trailing no-op line).
#[test]
fn out_of_band_edit_emits_absent_actor_now_delta() {
    let (_d, root) = s0();
    let r1x = r1_external();
    let frames = serve_scripted(
        &root,
        vec![
            line(r#"{"id":1,"op":"root"}"#), // primes the baseline at R0
            Step::Mutate(external_e3_edit(&root)),
            line(r#"{"id":2,"op":"root"}"#), // pre-dispatch reconcile emits
            line(&format!(
                r#"{{"id":3,"op":"diff","from_root":"{R0}","to_root":"{r1x}"}}"#
            )),
            line(r#"{"id":4,"op":"root"}"#), // nothing new: seq stable
        ],
    );
    assert_eq!(frames.len(), 4, "{frames:?}");
    assert_eq!(frames[0].1["body"], serde_json::json!({"root":R0,"seq":0}));
    assert_eq!(
        frames[1].1["body"],
        serde_json::json!({"root":r1x,"seq":1}),
        "the external delta advanced the epoch onto the computed fold"
    );
    let batches = frames[2].1["body"]["batches"].as_array().expect("batches");
    assert_eq!(batches.len(), 1);
    let delta = batches[0]["delta"].as_object().expect("delta");
    assert!(
        !delta.contains_key("actor"),
        "§7.1: actor ABSENT: {delta:?}"
    );
    assert!(!delta.contains_key("now"), "§7.1: now ABSENT: {delta:?}");
    assert_eq!(delta["seq"], 1, "seq assigned at detection");
    assert_eq!(delta["root_before"], R0);
    assert_eq!(delta["root_after"].as_str(), Some(r1_external().as_str()));
    // the change facts are the real E3 node facts, derived
    assert_eq!(delta["files"][0]["path"], "notes/plan.md");
    assert_eq!(delta["files"][0]["change"], "modified");
    assert_eq!(
        delta["files"][0]["nodes"][0]["node_rev_after"],
        "41f643f034e5681f"
    );
    assert_eq!(frames[3].1["body"], serde_json::json!({"root":r1x,"seq":1}));
}

/// GATES 2 + 5 — push-path delivery and replay parity: with a live `sub`,
/// the external edit arrives as a Notification frame BEFORE the next
/// response, and `diff` over its range re-serializes (typed) to the exact
/// pushed bytes — one shape, two transports, external emissions included.
#[test]
fn external_edit_reaches_live_sub_and_replays_identically() {
    let (_d, root) = s0();
    let r1x = r1_external();
    let frames = serve_scripted(
        &root,
        vec![
            line(r#"{"id":10,"op":"sub","from_seq":0}"#),
            Step::Mutate(external_e3_edit(&root)),
            line(r#"{"id":11,"op":"root"}"#),
            line(&format!(
                r#"{{"id":12,"op":"diff","from_root":"{R0}","to_root":"{r1x}"}}"#
            )),
        ],
    );
    // ack, notif (external, pushed pre-response), root-resp, diff-resp
    assert_eq!(frames.len(), 4, "{frames:?}");
    assert_eq!(
        frames[0].1["body"],
        serde_json::json!({"root":R0,"seq":0}),
        "sub ack primes at R0"
    );
    assert!(
        is_notification(&frames[1].1),
        "external delta pushed before the next response: {}",
        frames[1].1
    );
    assert_eq!(frames[2].1["id"], 11);
    let typed: wire::Response = serde_json::from_str(&frames[3].0).expect("diff types");
    let wire::ResponsePayload::Body {
        body: wire::ResponseBody::Diff { ref batches },
    } = typed.payload
    else {
        panic!("diff response carries batches")
    };
    assert_eq!(batches.len(), 1);
    assert_eq!(
        frames[1].0,
        serde_json::to_string(&batches[0]).unwrap(),
        "pushed external frame ≡ diff batch, byte-for-byte"
    );
}

/// GATE 4 — the ruled rename form, pinned as data: a pure out-of-band
/// rename (one removed + one added, byte-equal) classifies `renamed` with
/// `from_path`, same `file_rev` both tenses, no node entries.
#[test]
fn pure_external_rename_classifies_renamed_with_from_path() {
    let (_d, root) = s0();
    let from = root.0.join("notes/plan.md");
    let to = root.0.join("notes/plan-v2.md");
    let frames = serve_scripted(
        &root,
        vec![
            line(r#"{"id":20,"op":"root"}"#),
            Step::Mutate(Box::new(move || {
                std::fs::rename(&from, &to).expect("external rename");
            })),
            line(r#"{"id":21,"op":"root"}"#),
        ],
    );
    assert_eq!(frames.len(), 2);
    let seq1_root = frames[1].1["body"]["root"].as_str().expect("new root");
    assert_ne!(seq1_root, R0, "rename moves the corpus root");
    assert_eq!(frames[1].1["body"]["seq"], 1);
    // read the emitted frame back through diff over the advanced range
    let frames2 = serve_scripted(
        &root,
        vec![line(r#"{"id":22,"op":"root"}"#)], // fresh epoch just to fold
    );
    assert_eq!(frames2[0].1["body"]["root"], seq1_root);
    // assert the delta within one session:
    let (_d2, root2) = s0();
    let from2 = root2.0.join("notes/plan.md");
    let to2 = root2.0.join("notes/plan-v2.md");
    let frames3 = serve_scripted(
        &root2,
        vec![
            line(r#"{"id":30,"op":"sub","from_seq":0}"#),
            Step::Mutate(Box::new(move || {
                std::fs::rename(&from2, &to2).expect("external rename");
            })),
            line(r#"{"id":31,"op":"root"}"#),
        ],
    );
    assert_eq!(frames3.len(), 3, "{frames3:?}");
    let delta = frames3[1].1["delta"].as_object().expect("pushed delta");
    let files = delta["files"].as_array().expect("files");
    assert_eq!(files.len(), 1);
    let f = files[0].as_object().expect("file entry");
    assert_eq!(f["path"], "notes/plan-v2.md");
    assert_eq!(f["change"], "renamed");
    assert_eq!(f["from_path"], "notes/plan.md");
    assert_eq!(
        f["file_rev_before"], f["file_rev_after"],
        "same bytes, same rev, both tenses"
    );
    assert_eq!(f["nodes"].as_array().expect("nodes").len(), 0, "no entries");
    assert!(!delta.contains_key("actor") && !delta.contains_key("now"));
}

/// The refuse-to-guess fallback: a rename WITH a content edit in the same
/// detection window is NOT byte-equal — emitted as delete+create,
/// `from_path` unwired (pinned as data per the ruling).
#[test]
fn rename_plus_edit_refuses_to_guess_delete_create() {
    let (_d, root) = s0();
    let dir = root.0.clone();
    let frames = serve_scripted(
        &root,
        vec![
            line(r#"{"id":40,"op":"sub","from_seq":0}"#),
            Step::Mutate(Box::new(move || {
                let s1 = wsfix("s0/notes/plan.md").replace("ship by August", "ship by September");
                std::fs::remove_file(dir.join("notes/plan.md")).expect("rm");
                std::fs::write(dir.join("notes/plan-v2.md"), s1).expect("write");
            })),
            line(r#"{"id":41,"op":"root"}"#),
        ],
    );
    assert_eq!(frames.len(), 3, "{frames:?}");
    let delta = frames[1].1["delta"].as_object().expect("pushed delta");
    let files = delta["files"].as_array().expect("files");
    assert_eq!(files.len(), 2, "delete + create, no guessing: {files:?}");
    let changes: Vec<&str> = files
        .iter()
        .map(|f| f["change"].as_str().expect("change"))
        .collect();
    assert!(changes.contains(&"deleted") && changes.contains(&"created"));
    for f in files {
        assert!(
            !f.as_object().expect("entry").contains_key("from_path"),
            "from_path stays unwired on refusal: {f}"
        );
    }
}
