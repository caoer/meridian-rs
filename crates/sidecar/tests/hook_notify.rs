//! C3 — process-boundary reaction feeding through the real sidecar binary.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use policy::armed::{ArmRequest, ArmRoot, Mode};
use policy::{PageRef, RuleId, RuleIndex, ScopeLayer, page_rev};
use serde_json::{Value, json};

/// The armed rule's id — the § 2 grammar, not a folder slug.
const HOOK_ID: &str = "task.review-notify";

/// The tag-registered HOOK page. It carries BOTH key sets: `tags:` + `id:` are the
/// REGISTRATION layer the ARM act reads (ruling § 1/§ 2), and
/// `kind:`/`severity:`/`paths:`/`caps:`/`budget:`/`how:` are C1's declaration layer,
/// which ruling § 5 leaves untouched.
///
/// The page sits DIRECTLY in the folder it governs (§ 3 "gitignore-style"), so it
/// mounts at the workspace root. The ruled layout-folder style (`rules/*.md` mounting
/// at the parent) is not used: that mount rule is ruled but not yet implemented, and
/// this card consumes armed rows rather than minting mount rules.
const HOOK_PAGE_PATH: &str = "task-review-notify.md";

const HOOK_PAGE: &str = r#"---
tags: [type/rule, rules/hook]
id: task.review-notify
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

fn workspace(armed: bool) -> tempfile::TempDir {
    armed_workspace(if armed { Some(Mode::Armed) } else { None })
}

/// A live workspace whose rule page is armed at `mode` through the REAL ARM act.
/// `None` never arms it at all — no artifact is written, which is the never-armed
/// state the bit-for-bit gate depends on.
fn armed_workspace(mode: Option<Mode>) -> tempfile::TempDir {
    let temp = tempfile::tempdir().expect("temp workspace");
    for path in ["tasks/x.md", "notes/x.md"] {
        let absolute = temp.path().join(path);
        std::fs::create_dir_all(absolute.parent().expect("file parent")).expect("file parent");
        std::fs::write(absolute, TASK_IN_PROGRESS).expect("task fixture");
    }
    if let Some(mode) = mode {
        arm_hook(temp.path(), mode);
    }
    temp
}

fn arm_hook(root: &std::path::Path, mode: Mode) {
    std::fs::write(root.join(HOOK_PAGE_PATH), HOOK_PAGE).expect("rule page");

    let index = RuleIndex::discover([PageRef {
        layer: ScopeLayer::Workspace,
        page: HOOK_PAGE_PATH,
        bytes: HOOK_PAGE,
    }]);
    let artifact = policy::armed::arm(
        &index,
        &ArmRoot::workspace(),
        [ArmRequest {
            id: RuleId::parse(HOOK_ID).expect("a legal id"),
            mode,
            attested_rev: page_rev(HOOK_PAGE),
        }],
    )
    .expect("the fixture arms");

    let artifact_path = root.join(policy::armed::ARMED_RULES_PATH);
    std::fs::create_dir_all(artifact_path.parent().expect("artifact parent"))
        .expect("artifact parent");
    std::fs::write(artifact_path, artifact.render()).expect("armed-rules artifact");

    // The once-armed MARKER, beside the artifact. Arming is ONE act over TWO files:
    // the marker is the pivot BOTH armed-law surfaces read, so an artifact without
    // it describes a workspace the engine considers never armed — the notification
    // this test waits for would never be produced, and the wait would hang rather
    // than fail.
    let marker_path = root.join(fs::domain::ATTESTED_MARKER_PATH);
    std::fs::create_dir_all(marker_path.parent().expect("marker parent")).expect("marker parent");
    std::fs::write(marker_path, "").expect("once-armed marker");
}

/// U10: `status` already exists on these fixtures, so an upsert of it is a
/// content change and a wire-origin write must carry the key node's fingerprint.
/// `rev` comes from a `cat` of that very key — read first, write what you read.
fn splice(id: u64, path: &str, status: &str, rev: &str) -> String {
    json!({
        "id": id,
        "op": "splice",
        "path": path,
        "actor": "agent:worker",
        "edits": [{
            "target": {"fm_key": "status"},
            "edit": {"put": {"at": "upsert", "text": status}},
            "if_node_rev": rev
        }]
    })
    .to_string()
}

/// The live `node_rev` of one frontmatter key, over the wire the test already
/// speaks: `cat` that key, keep its token.
fn fm_rev(sidecar: &mut LiveSidecar, id: u64, path: &str) -> String {
    sidecar.send(
        &json!({
            "id": id,
            "op": "cat",
            "path": path,
            "sec": {"fm_key": "status"}
        })
        .to_string(),
    );
    let (_, frame) = sidecar.receive();
    frame["body"]["node_rev"]
        .as_str()
        .unwrap_or_else(|| panic!("cat serves the key's node_rev: {frame}"))
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

    /// Negotiate v3 — **the contract the reaction plane belongs to.**
    ///
    /// `effects` postdates frozen v2, so a v2 session is DEMOTED past it
    /// (`wire_serve::rev::V2_RESERVED_FIELDS`). A reaction test left on the
    /// default v2 session therefore asserts the v2 WIRE SHAPE while its name
    /// claims it covers ARMING — and the "no reaction" half goes vacuous, since
    /// a v2 notification never carries `effects` whether or not one was armed.
    /// Moving the fixture onto v3 is what keeps these tests pointed at their own
    /// subject.
    fn negotiate_v3(&mut self, id: u64) {
        self.send(&format!(
            r#"{{"id":{id},"op":"hello","proto":1,"contract":"v3"}}"#
        ));
        let (_, hello) = self.receive();
        assert_eq!(hello["id"], id, "hello is answered: {hello}");
        assert_eq!(hello["ok"], true, "v3 negotiated: {hello}");
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
    assert_eq!(intent["rule_id"], HOOK_ID);
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
    sidecar.negotiate_v3(100);
    sidecar.send(r#"{"id":1,"op":"sub","from_seq":0}"#);
    assert_eq!(sidecar.receive().1["id"], 1);

    let rev = fm_rev(&mut sidecar, 900, "tasks/x.md");
    sidecar.send(&splice(2, "tasks/x.md", "review", &rev));
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
    sidecar.negotiate_v3(110);
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
    sidecar.negotiate_v3(120);
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

    let rev = fm_rev(&mut sidecar, 922, "notes/x.md");
    sidecar.send(&splice(22, "notes/x.md", "review", &rev));
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
        "out-of-scope Delta keeps pre-effects bytes — and on a v3 session this \
         can actually FAIL: v3 carries `effects` whenever one is armed, so the \
         absence here means no reaction was armed, not that the wire hid it"
    );
    sidecar.finish();
}

#[test]
fn zero_subscribers_still_ring_delta_and_return_armed_feedback() {
    let workspace = workspace(true);
    let mut sidecar = LiveSidecar::spawn(workspace.path());
    let rev = fm_rev(&mut sidecar, 930, "tasks/x.md");
    sidecar.send(&splice(30, "tasks/x.md", "review", &rev));
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
    let rev = fm_rev(&mut sidecar, 940, "tasks/x.md");
    sidecar.send(&splice(40, "tasks/x.md", "review", &rev));
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

#[test]
fn a_row_armed_off_fires_nothing_through_the_live_loop() {
    // The page is present, in scope, and pinned at its live rev — only the ATTESTED
    // MODE differs. Hook activation is binary by law, so `off` emits nothing while
    // the write itself lands exactly as it would unarmed.
    let workspace = armed_workspace(Some(Mode::Off));
    let mut sidecar = LiveSidecar::spawn(workspace.path());
    sidecar.send(r#"{"id":50,"op":"sub","from_seq":0}"#);
    assert_eq!(sidecar.receive().1["id"], 50);

    let rev = fm_rev(&mut sidecar, 951, "tasks/x.md");
    sidecar.send(&splice(51, "tasks/x.md", "review", &rev));
    let (raw, response) = sidecar.receive();
    assert_eq!(response["id"], 51, "the write still lands");
    assert!(
        response["body"]["armed"].get("effects").is_none(),
        "an attested-off row arms nothing: {raw}"
    );

    let (notification_raw, notification) = sidecar.receive();
    assert!(notification.get("id").is_none(), "the Delta still flows");
    assert!(
        notification.get("effects").is_none(),
        "an attested-off row puts nothing on the Delta: {notification_raw}"
    );
    sidecar.finish();
}
