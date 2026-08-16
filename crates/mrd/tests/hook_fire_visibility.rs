//! Hook-fire visibility on the `mrd put` human face — the card's gate, driven
//! through the real binary over its process boundary.
//!
//! The measured asymmetry this file pins shut: CHECK refusals were loud on
//! both faces, REACTIONS were silent on the human face — a workspace with an
//! armed notification plane committed identically to an unarmed one, and the
//! operator who armed it could only see it fire through `--json`
//! (`put.armed.effects[]`). The gate re-runs the dogfood move that surfaced it
//! (a status move to `review` firing the status-notify hook) WITHOUT `--json`
//! and reads the fired intent off stdout; the control run proves a workspace
//! with no armed hooks prints exactly what it always did.

use std::path::PathBuf;
use std::process::{Command, Output};

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
        common::reap_daemon(&self.cache_home);
    }
}

impl Sandbox {
    fn write(&self, rel: &str, bytes: &str) {
        let path = self.ws.join(rel);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
        std::fs::write(path, bytes).expect("write");
    }

    fn run_stdin(&self, args: &[&str], stdin: &str) -> Output {
        use std::io::Write as _;
        let mut child = Command::new(mrd_bin())
            .args(args)
            .current_dir(&self.ws)
            .env("HOME", &self.home)
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env_remove("MERIDIAN_CONFIG")
            .env_remove("MERIDIAN_WORKSPACE")
            .env("MERIDIAN_DAEMON_BIN", mrd_bin())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn mrd");
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(stdin.as_bytes())
            .expect("feed stdin");
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

/// **The card's gate, firing half.** The dogfood move re-run WITHOUT `--json`:
/// the human commit face shows the fired intent on ONE line — rule id, action,
/// target, and the canonical receipt address VERBATIM (the pairing key the
/// delivery faces echo as `correlation`) — beside the fingerprint it already
/// printed.
#[test]
fn the_human_face_prints_one_line_per_fired_intent() {
    let s = armed();
    let stdout = s.stdout(&["put", "tasks/card.md", "--force"], MOVE_TO_REVIEW);
    let fired: Vec<&str> = stdout
        .lines()
        .filter(|line| line.starts_with("  fired: "))
        .collect();
    assert_eq!(
        fired.len(),
        1,
        "one fired intent, one line — stdout:\n{stdout}"
    );
    assert!(
        fired[0].starts_with("  fired: task-status-notify notify → zt (receipt tasks/card.md#^r-")
            && fired[0].ends_with(')'),
        "the line names rule, action, target and the verbatim receipt address: {}",
        fired[0]
    );
    assert!(
        stdout.contains("committed tasks/card.md (1 edit(s))"),
        "the commit line is unchanged beside it: {stdout}"
    );
    assert!(
        !stdout.contains("delivered") && !stdout.contains("sent"),
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
