//! Intent visibility on the `mrd put` human face — driven through the real
//! binary over its process boundary.
//!
//! REDESIGNED with the middleware door (armed-plane Part A2, wire-contract
//! § A.2.1): the put-path HOOK feed is retired — a put against an armed
//! `rules/hook` workspace prints NO `fired:` line, because the write response
//! carries no reaction envelope any more. What the human face shows instead
//! is the middleware plane: an armed `rules/middleware` page emitting `send`
//! prints ONE `intent:` line per intent (armed, host-realized — never a
//! delivery claim). The control run proves an unarmed workspace prints
//! exactly what it always did.

use std::path::PathBuf;
use std::process::Output;

mod common;

/// The binary every drive goes through — the real CLI, never a library call.
fn mrd_bin() -> PathBuf {
    std::env::var_os("MRD_BIN")
        .map_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_mrd")), PathBuf::from)
}

/// The dogfood hook, minimally: fire on a frontmatter `status` move to
/// `review`, target the card's declared reviewer, receipt at the canonical
/// post-write address.
const STATUS_NOTIFY: &str = r#"---
tags: [type/rule, rules/hook]
id: task-status-notify
kind: hook
severity: info
paths: ["tasks/*.md"]
caps: [proto.send]
budget: { steps: 10000, mem: 4194304 }
how:
  route: { info: channel-review }
---

# task-status-notify

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
            payload = "%s: status %s -> review" % (event.file, delta.old),
            receipt = receipt_addr(event.file, event.fingerprint_after),
        )
```
"#;

/// A governed task card in the hook's declared scope, reviewer declared.
const CARD: &str = "---\ntype: task\nstatus: todo\nreviewer: zt\n---\n\n# a card\n";

/// The dogfood move: the status flip to `review`, as a §4.4 batch on stdin.
const MOVE_TO_REVIEW: &str =
    r#"[{"target":{"fm_key":"status"},"edit":{"match":{"old":"todo","new":"review"}}}]"#;

struct Sandbox {
    #[allow(dead_code)]
    tmp: tempfile::TempDir,
    home: PathBuf,
    cache_home: PathBuf,
    ws: PathBuf,
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        common::reap_daemon(&self.home, &self.cache_home);
    }
}

impl Sandbox {
    fn write(&self, rel: &str, bytes: &str) {
        let path = self.ws.join(rel);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
        std::fs::write(path, bytes).expect("write");
    }

    fn run_stdin(&self, args: &[&str], stdin: &str) -> Output {
        let mut child = common::mrd_command(&self.home, &self.cache_home)
            .args(args)
            .current_dir(&self.ws)
            .env_remove("MERIDIAN_CONFIG")
            .env_remove("MERIDIAN_WORKSPACE")
            .env("MERIDIAN_DAEMON_BIN", mrd_bin())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn mrd");
        common::feed_stdin(&mut child, stdin.as_bytes());
        child.wait_with_output().expect("wait mrd")
    }

    /// stdout of a run asserted to exit 0.
    fn stdout(&self, args: &[&str], stdin: &str) -> String {
        let out = self.run_stdin(args, stdin);
        assert!(
            out.status.success(),
            "mrd {args:?} exited {:?}\n{}{}",
            out.status.code(),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).expect("utf-8 stdout")
    }

    /// The rev a reviewer reads for `id` — the winner line of `mrd rules --json`.
    fn reviewed_rev(&self, id: &str) -> String {
        let out = self.run_stdin(&["rules", ".", "--json"], "");
        assert_ne!(
            out.status.code(),
            Some(2),
            "rules refused the invocation: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let raw = String::from_utf8(out.stdout).expect("utf-8 stdout");
        let json: serde_json::Value = serde_json::from_str(&raw).expect("rules --json parses");
        let rows = json["rules"]["rules"].as_array().expect("rows");
        let row = rows
            .iter()
            .find(|row| row["id"] == id)
            .unwrap_or_else(|| panic!("{id} is not in the effective set: {raw}"));
        row["chain"][0]["rev"]
            .as_str()
            .expect("the winner's rev")
            .to_owned()
    }
}

/// A workspace with the governed card; no rule page, nothing armed.
fn sandbox() -> Sandbox {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().join("home");
    let cache_home = tmp.path().join("xdg-cache");
    let ws = tmp.path().join("ws");
    for dir in [&home, &cache_home, &ws] {
        std::fs::create_dir_all(dir).expect("mkdir");
    }
    let sandbox = Sandbox {
        tmp,
        home,
        cache_home,
        ws,
    };
    sandbox.write(
        "MERIDIAN.md",
        "---\ntype: meridian-root\nversion: 1\nname: ws\n---\n\n# The workspace\n",
    );
    std::fs::create_dir_all(sandbox.home.join(".config")).ok();
    std::fs::write(
        sandbox.home.join("MERIDIAN.md"),
        "---\ntype: meridian-config\nversion: 1\n---\n\n# This machine\n",
    )
    .expect("home anchor");
    sandbox.write("tasks/card.md", CARD);
    sandbox
}

/// The same workspace with the hook authored and armed through the real ARM act.
fn armed() -> Sandbox {
    let s = sandbox();
    s.write("rules/task-status-notify.md", STATUS_NOTIFY);
    let rev = s.reviewed_rev("task-status-notify");
    let out = s.run_stdin(
        &[
            "arm",
            "task-status-notify",
            "--mode",
            "armed",
            "--rev",
            &rev,
        ],
        "",
    );
    assert!(
        out.status.success(),
        "the fixture arms: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    s
}

/// **The retirement half (§ A.2.1).** The dogfood move against the armed
/// HOOK workspace now prints NO fired line: the put-path hook feed is dead,
/// and the commit face is exactly the unarmed face. The hook plane survives
/// on the external-change detector only.
#[test]
fn an_armed_hook_prints_no_fired_line_on_the_put_face() {
    let s = armed();
    let stdout = s.stdout(&["put", "tasks/card.md", "--force"], MOVE_TO_REVIEW);
    assert!(
        !stdout.contains("fired:"),
        "the put-path hook feed is retired — no fired line: {stdout}"
    );
    assert!(
        stdout.contains("committed tasks/card.md (1 edit(s))"),
        "the commit line is unchanged: {stdout}"
    );
}

/// The middleware page that replaces the dogfood hook on the door: on the
/// status flip to `review`, send to the card's declared reviewer.
const STATUS_NOTIFY_MW: &str = r#"---
tags: [type/rule, rules/middleware]
id: status-notify-mw
paths: ["tasks/*.md"]
---

# status-notify-mw

```starlark
def middleware(ctx):
    if ctx.after.frontmatter.get("status") == "review":
        send(to = [ctx.after.frontmatter.get("reviewer")], body = "status -> review")
```
"#;

/// **The firing half, redesigned.** The same move through an armed
/// MIDDLEWARE page prints ONE `intent:` line — rule id, kind, targets — and
/// still claims no delivery.
#[test]
fn the_human_face_prints_one_line_per_middleware_intent() {
    let s = sandbox();
    s.write("rules/status-notify-mw.md", STATUS_NOTIFY_MW);
    let rev = s.reviewed_rev("status-notify-mw");
    let out = s.run_stdin(
        &["arm", "status-notify-mw", "--mode", "block", "--rev", &rev],
        "",
    );
    assert!(
        out.status.success(),
        "the middleware fixture arms: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = s.stdout(&["put", "tasks/card.md", "--force"], MOVE_TO_REVIEW);
    let intents: Vec<&str> = stdout
        .lines()
        .filter(|line| line.starts_with("  intent: "))
        .collect();
    assert_eq!(
        intents.len(),
        1,
        "one armed intent, one line — stdout:\n{stdout}"
    );
    assert_eq!(
        intents[0], "  intent: status-notify-mw send → zt (host realizes)",
        "the line names rule, kind and target, and says who realizes"
    );
    assert!(
        !stdout.contains("delivered") && !stdout.contains(" sent"),
        "the engine face claims no delivery: {stdout}"
    );
}

/// **The card's gate, control half.** A workspace with no armed hooks prints
/// exactly what it always did: the commit line and the fingerprint, no fired
/// line, nothing else.
#[test]
fn a_no_hook_workspace_output_is_unchanged() {
    let s = sandbox();
    let stdout = s.stdout(&["put", "tasks/card.md", "--force"], MOVE_TO_REVIEW);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines.len(),
        2,
        "the unarmed face is exactly two lines: {stdout}"
    );
    assert_eq!(lines[0], "committed tasks/card.md (1 edit(s))");
    assert!(
        lines[1].starts_with("  fingerprint: "),
        "the fingerprint line is unchanged: {stdout}"
    );
}
