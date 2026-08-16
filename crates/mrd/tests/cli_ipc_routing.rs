//! Quality gates for card `cli-ipc-routing`: routine CLI writes are IPC; the
//! direct-publication lane is gone; a down daemon is taught, never a local write.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

fn mrd_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mrd")
}

const DOC: &str = "# Alpha\n\none two three\n\n## Beta\n\nfour five\n";
const BETA_EDIT: &str = r#"[{"target":{"hpath":[{"h":"Alpha"},{"h":"Beta"}]},"edit":{"match":{"old":"four five","new":"four five six"}}}]"#;

struct Sandbox {
    tmp: tempfile::TempDir,
    cache_home: PathBuf,
    home: PathBuf,
}

fn sandbox() -> Sandbox {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cache_home = tmp.path().join("xdg-cache");
    let home = tmp.path().join("home");
    std::fs::create_dir_all(&home).expect("home");
    Sandbox {
        tmp,
        cache_home,
        home,
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        reap_daemon(&self.cache_home);
    }
}

fn reap_daemon(cache_home: &Path) {
    let pidfile = cache_home
        .join("meridian")
        .join("registry")
        .join("daemon.pid");
    let Ok(text) = std::fs::read_to_string(pidfile) else {
        return;
    };
    let Ok(pid) = text.trim().parse::<i32>() else {
        return;
    };
    // SAFETY: pid came from this sandbox's own pidfile.
    unsafe {
        libc::kill(pid, libc::SIGTERM);
    }
}

impl Sandbox {
    fn workspace(&self) -> PathBuf {
        let ws = self.tmp.path().join("project");
        std::fs::create_dir_all(&ws).expect("mkdir");
        std::fs::write(ws.join("doc.md"), DOC).expect("doc");
        let out = self.run(&ws, &["init"], None, false);
        assert!(out.status.success(), "init: {}", stderr(&out));
        ws
    }

    fn run(&self, cwd: &Path, args: &[&str], stdin: Option<&str>, live: bool) -> Output {
        let mut cmd = Command::new(mrd_bin());
        cmd.args(args)
            .current_dir(cwd)
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
            .env_remove("MERIDIAN_WORKSPACE");
        if live {
            cmd.env("MERIDIAN_DAEMON_BIN", mrd_bin());
        } else {
            cmd.env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-daemon");
        }
        if let Some(bytes) = stdin {
            let mut child = cmd
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("spawn mrd");
            child
                .stdin
                .as_mut()
                .expect("stdin")
                .write_all(bytes.as_bytes())
                .expect("write stdin");
            child.wait_with_output().expect("wait")
        } else {
            cmd.output().expect("spawn mrd")
        }
    }

    fn start_daemon(&self) -> Child {
        let child = Command::new(mrd_bin())
            .arg("daemon")
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
            .env_remove("MERIDIAN_WORKSPACE")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn daemon");
        let socket = self
            .cache_home
            .join("meridian")
            .join("registry")
            .join("daemon.sock");
        let client = registry::Client::new(socket);
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if client.ping().unwrap_or(false) {
                return child;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        panic!("daemon did not answer a ping");
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}
fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Gate — production write verbs no longer call the in-process choke-point.
#[test]
fn production_write_verbs_have_no_direct_splice() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    for name in ["put_cmd.rs", "pin_cmd.rs", "rm_cmd.rs", "retire_cmd.rs"] {
        let src = std::fs::read_to_string(root.join(name)).expect(name);
        assert!(
            !src.contains("wire_serve::write::splice")
                && !src.contains("wire_serve::write::remove")
                && !src.contains("splice(&root")
                && !src.contains("remove(&root"),
            "{name} still calls the in-process write door"
        );
        assert!(
            src.contains("write_ipc"),
            "{name} must route through write_ipc"
        );
    }
}

/// Gate — IPC unavailable produces the taught refusal and never a local write.
#[test]
fn daemon_down_is_taught_and_writes_nothing() {
    let sb = sandbox();
    let ws = sb.workspace();
    let before = std::fs::read_to_string(ws.join("doc.md")).expect("before");
    let out = sb.run(&ws, &["put", "doc.md", "--force"], Some(BETA_EDIT), false);
    assert_eq!(
        out.status.code(),
        Some(2),
        "a down daemon is the CLI's tool leg: stdout={} stderr={}",
        stdout(&out),
        stderr(&out)
    );
    let said = stderr(&out);
    assert!(
        said.contains("no direct-publication fallback"),
        "the face teaches the migration: {said}"
    );
    assert!(
        said.contains("daemon must come up"),
        "the face names the recovery: {said}"
    );
    assert!(
        !said.contains("workspace_busy"),
        "daemon-down is not the old lock class: {said}"
    );
    assert_eq!(
        std::fs::read_to_string(ws.join("doc.md")).expect("after"),
        before,
        "a down daemon must not write the corpus"
    );
}

/// Gate — help and status.md carry the same migration teaching.
#[test]
fn migration_teaching_is_on_the_face_and_in_docs() {
    let help = Command::new(mrd_bin())
        .args(["put", "--help"])
        .output()
        .expect("help");
    let text = stdout(&help);
    assert!(
        text.contains("IPC") || text.contains("daemon"),
        "put --help names the daemon route: {text}"
    );
    assert!(
        text.contains("no direct-write fallback") || text.contains("daemon must come up"),
        "put --help teaches the fallback is gone: {text}"
    );

    let docs = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("crate → workspace")
        .join("docs/status.md");
    let status = std::fs::read_to_string(&docs).expect("status.md");
    assert!(
        status.contains("wire clients") && status.contains("no direct-publication fallback"),
        "docs/status.md carries the migration teaching"
    );
}

/// Gate — a routine CLI write through a live daemon lands, and is not
/// `workspace_busy` (the CLI process no longer takes LOCK_NB).
#[test]
fn live_put_commits_and_is_not_workspace_busy() {
    let sb = sandbox();
    let ws = sb.workspace();
    let mut daemon = sb.start_daemon();
    let out = sb.run(&ws, &["put", "doc.md", "--force"], Some(BETA_EDIT), true);
    let _ = daemon.kill();
    let _ = daemon.wait();
    assert_eq!(
        out.status.code(),
        Some(0),
        "live put: stdout={} stderr={}",
        stdout(&out),
        stderr(&out)
    );
    let after = std::fs::read_to_string(ws.join("doc.md")).expect("after");
    assert!(after.contains("four five six"), "the edit landed: {after}");
    let combined = format!("{}{}", stdout(&out), stderr(&out));
    assert!(
        !combined.contains("workspace_busy"),
        "a single-writer CLI put must not see workspace_busy: {combined}"
    );
}

/// Gate — two sequential CLI writes through one daemon never mint
/// `workspace_busy` from the CLI path. (Overlapping daemon-side LOCK_NB
/// during parallel publish is the parallel-commits half of step 6.)
#[test]
fn sequential_cli_writes_are_not_workspace_busy() {
    let sb = sandbox();
    let ws = sb.workspace();
    std::fs::write(ws.join("other.md"), "# Other\n\nzzz\n").expect("other");
    let mut daemon = sb.start_daemon();
    let first = sb.run(&ws, &["put", "doc.md", "--force"], Some(BETA_EDIT), true);
    let second = sb.run(
        &ws,
        &["put", "other.md", "--force"],
        Some(
            r#"[{"target":{"hpath":[{"h":"Other"}]},"edit":{"match":{"old":"zzz","new":"yyy"}}}]"#,
        ),
        true,
    );
    let _ = daemon.kill();
    let _ = daemon.wait();
    assert_eq!(first.status.code(), Some(0), "first: {}", stderr(&first));
    assert_eq!(second.status.code(), Some(0), "second: {}", stderr(&second));
    for (name, out) in [("first", &first), ("second", &second)] {
        let combined = format!("{}{}", stdout(out), stderr(out));
        assert!(
            !combined.contains("workspace_busy"),
            "{name} saw workspace_busy: {combined}"
        );
    }
}
