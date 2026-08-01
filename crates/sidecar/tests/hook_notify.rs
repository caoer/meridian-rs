//! C3 — process-boundary reaction feeding through the real sidecar binary.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use policy::{CheckLimits, ConventionFiles, Enforcement};
use serde_json::{Value, json};

const CHECK: &str = "---\npaths:\n  - tasks/*.md\n---\n\n# task-status-notify\n\n```starlark\ndef check_change(change):\n    pass\n```\n";
const HOOK: &str = r#"---
kind: hook
severity: info
paths: ["tasks/*.md"]
caps:  [proto.send]
budget: { steps: 10000, mem: 4194304 }
how:
  route: { info: channel-review }
---

```starlark
def on_change(event):
    for delta in event.changes:
        if delta.kind != "frontmatter" or delta.key != "status":
            continue
        if delta.new != "review":
            continue
        return intent(
            action = "notify",
            target = event.facts.fm.get("reviewer"),
            severity = "info",
            payload = "task moved to review",
            receipt = receipt_addr(event.file, event.fingerprint_after),
        )
```
"#;

const TASK_IN_PROGRESS: &str =
    "---\ntype: task\nstatus: in-progress\nreviewer: e4201e72\n---\n\n# Task\n";

struct Folder(std::path::PathBuf);

impl ConventionFiles for Folder {
    fn read(&self, rel: &str) -> std::io::Result<String> {
        std::fs::read_to_string(self.0.join(rel))
    }

    fn exists(&self, rel: &str) -> bool {
        self.0.join(rel).exists()
    }
}

fn workspace(armed: bool) -> tempfile::TempDir {
    let temp = tempfile::tempdir().expect("temp workspace");
    for path in ["tasks/x.md", "notes/x.md"] {
        let absolute = temp.path().join(path);
        std::fs::create_dir_all(absolute.parent().expect("file parent")).expect("file parent");
        std::fs::write(absolute, TASK_IN_PROGRESS).expect("task fixture");
    }
    if armed {
        arm_hook(temp.path());
    }
    temp
}

fn arm_hook(root: &std::path::Path) {
    let folder = root.join("conventions/task-status-notify");
    std::fs::create_dir_all(&folder).expect("convention folder");
    std::fs::write(folder.join("CHECK.md"), CHECK).expect("CHECK.md");
    std::fs::write(folder.join("HOOK.md"), HOOK).expect("HOOK.md");
    let swept = policy::sweep(
        &Folder(folder),
        "task-status-notify",
        CheckLimits::default(),
    )
    .expect("convention sweeps");
    let rev = policy::evidence_rev(CHECK);
    let armed = policy::arm(swept, &rev, Enforcement::Warn).expect("convention arms");
    std::fs::write(
        root.join("conventions/INDEX.md"),
        policy::generate_index(&[armed]),
    )
    .expect("INDEX.md");
    let marker = root.join(fs::domain::ATTESTED_MARKER_PATH);
    std::fs::create_dir_all(marker.parent().expect("marker parent")).expect("marker parent");
    std::fs::write(marker, "attested\n").expect("attested marker");
}

fn splice(id: u64, path: &str, status: &str) -> String {
    json!({
        "id": id,
        "op": "splice",
        "path": path,
        "actor": "agent:worker",
        "edits": [{
            "target": {"fm_key": "status"},
            "edit": {"put": {"at": "upsert", "text": status}}
        }]
    })
    .to_string()
}

fn refused_splice(id: u64) -> String {
    json!({
        "id": id,
        "op": "splice",
        "path": "tasks/x.md",
        "actor": "agent:worker",
        "edits": [{
            "target": {"fm_key": "status"},
            "edit": {"put": {"at": "upsert", "text": "review"}},
            "if_node_rev": "0000000000000000"
        }]
    })
    .to_string()
}

struct LiveSidecar {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}

impl LiveSidecar {
    fn spawn(root: &std::path::Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_sidecar"))
            .arg(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn sidecar");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = BufReader::new(child.stdout.take().expect("stdout"));
        Self {
            child,
            stdin: Some(stdin),
            stdout,
        }
    }

    fn send(&mut self, line: &str) {
        let stdin = self.stdin.as_mut().expect("live stdin");
        writeln!(stdin, "{line}").expect("write request");
        stdin.flush().expect("flush request");
    }

    fn receive(&mut self) -> (String, Value) {
        let mut line = String::new();
        let read = self.stdout.read_line(&mut line).expect("read frame");
        assert!(read > 0, "sidecar closed before the expected frame");
        let raw = line.trim_end().to_string();
        let value = serde_json::from_str(&raw).expect("frame JSON");
        (raw, value)
    }

    fn finish(mut self) {
        drop(self.stdin.take());
        let mut trailing = String::new();
        self.stdout
            .read_to_string(&mut trailing)
            .expect("drain stdout");
        let status = self.child.wait().expect("wait for sidecar");
        let mut stderr = String::new();
        self.child
            .stderr
            .take()
            .expect("stderr")
            .read_to_string(&mut stderr)
            .expect("read stderr");
        assert!(status.success(), "sidecar failed: {stderr}");
        assert!(
            trailing.trim().is_empty(),
            "unexpected trailing frames: {trailing}"
        );
    }
}

fn assert_armed_intent(effects: &Value) {
    let envelopes = effects.as_array().expect("effects array");
    assert_eq!(envelopes.len(), 1, "one matched HOOK: {effects}");
    let intents = envelopes[0]["intents"].as_array().expect("intents");
    assert_eq!(intents.len(), 1, "one armed intent: {effects}");
    let intent = &intents[0];
    assert_eq!(intent["rule_id"], "task-status-notify");
    assert_eq!(intent["action"], "notify");
    assert_eq!(intent["target"], "e4201e72");
    let receipt = intent["receipt"].as_str().expect("canonical receipt");
    assert!(receipt.starts_with("tasks/x.md#^r-"), "{receipt}");
    assert_eq!(receipt.len(), "tasks/x.md#^r-".len() + 16);
    assert!(!effects.to_string().contains("delivered"));
}

#[test]
fn splice_response_arms_before_live_notification_delivery() {
    let workspace = workspace(true);
    let mut sidecar = LiveSidecar::spawn(workspace.path());
    sidecar.send(r#"{"id":1,"op":"sub","from_seq":0}"#);
    assert_eq!(sidecar.receive().1["id"], 1);

    sidecar.send(&splice(2, "tasks/x.md", "review"));
    let (response_raw, response) = sidecar.receive();
    assert_eq!(response["id"], 2, "the caller response arrives first");
    let armed = &response["body"]["armed"]["effects"];
    assert_armed_intent(armed);
    assert!(!response_raw.contains("delivered"));

    let (notification_raw, notification) = sidecar.receive();
    assert!(
        notification.get("id").is_none(),
        "notification follows response"
    );
    assert_eq!(notification["effects"], *armed);
    assert!(!notification_raw.contains("delivered"));
    sidecar.finish();
}

#[test]
fn external_edit_with_no_caller_emits_intent_with_actor_absent() {
    let workspace = workspace(true);
    let mut sidecar = LiveSidecar::spawn(workspace.path());
    sidecar.send(r#"{"id":10,"op":"sub","from_seq":0}"#);
    assert_eq!(sidecar.receive().1["id"], 10);

    std::fs::write(
        workspace.path().join("tasks/x.md"),
        TASK_IN_PROGRESS.replace("status: in-progress", "status: review"),
    )
    .expect("external edit");
    sidecar.send(r#"{"id":11,"op":"root"}"#);

    let (_, notification) = sidecar.receive();
    assert!(notification.get("id").is_none(), "external notification");
    assert_armed_intent(&notification["effects"]);
    let delta = notification["delta"].as_object().expect("delta");
    assert!(!delta.contains_key("actor"), "external actor stays absent");

    let (_, response) = sidecar.receive();
    assert_eq!(response["id"], 11);
    assert!(
        response["body"].get("armed").is_none(),
        "no caller exists for the external edit, so its later root response carries no armed feedback"
    );
    sidecar.finish();
}

#[test]
fn refused_and_out_of_scope_writes_emit_no_reaction() {
    let workspace = workspace(true);
    let mut sidecar = LiveSidecar::spawn(workspace.path());
    sidecar.send(r#"{"id":20,"op":"sub","from_seq":0}"#);
    assert_eq!(sidecar.receive().1["id"], 20);

    sidecar.send(&refused_splice(21));
    let (_, refused) = sidecar.receive();
    assert_eq!(refused["id"], 21);
    assert_eq!(refused["ok"], false);
    assert!(
        refused.get("body").is_none(),
        "no armed response on refusal"
    );

    sidecar.send(&splice(22, "notes/x.md", "review"));
    let (_, out_of_scope) = sidecar.receive();
    assert_eq!(
        out_of_scope["id"], 22,
        "the refused write emitted no intervening notification"
    );
    assert!(
        out_of_scope["body"]["armed"].get("effects").is_none(),
        "out-of-scope response omits reaction data"
    );
    let (_, notification) = sidecar.receive();
    assert!(notification.get("id").is_none());
    assert!(
        notification.get("effects").is_none(),
        "out-of-scope Delta keeps pre-effects bytes"
    );
    sidecar.finish();
}

#[test]
fn zero_subscribers_still_ring_delta_and_return_armed_feedback() {
    let workspace = workspace(true);
    let mut sidecar = LiveSidecar::spawn(workspace.path());
    sidecar.send(&splice(30, "tasks/x.md", "review"));
    let (_, response) = sidecar.receive();
    let armed = response["body"]["armed"]["effects"].clone();
    assert_armed_intent(&armed);

    let from = response["body"]["root_before"]
        .as_str()
        .expect("root before");
    let to = response["body"]["root_after"].as_str().expect("root after");
    sidecar.send(&json!({"id":31,"op":"diff","from_root":from,"to_root":to}).to_string());
    let (_, diff) = sidecar.receive();
    let batches = diff["body"]["batches"].as_array().expect("ring batches");
    assert_eq!(batches.len(), 1, "Delta was ringed without a subscriber");
    assert_eq!(batches[0]["effects"], armed);
    sidecar.finish();
}

#[test]
fn never_armed_process_output_omits_reaction_fields() {
    let workspace = workspace(false);
    let mut sidecar = LiveSidecar::spawn(workspace.path());
    sidecar.send(&splice(40, "tasks/x.md", "review"));
    let (raw, response) = sidecar.receive();
    assert!(
        !raw.contains("\"effects\""),
        "never-armed response bytes: {raw}"
    );

    let from = response["body"]["root_before"]
        .as_str()
        .expect("root before");
    let to = response["body"]["root_after"].as_str().expect("root after");
    sidecar.send(&json!({"id":41,"op":"diff","from_root":from,"to_root":to}).to_string());
    let (raw, _) = sidecar.receive();
    assert!(
        !raw.contains("\"effects\""),
        "never-armed Delta bytes: {raw}"
    );
    sidecar.finish();
}
