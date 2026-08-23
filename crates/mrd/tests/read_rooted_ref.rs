//! The read door's rooted lane — `[root:]path` on the CLI, resolved to the
//! named root's bound workspace (address-grammar §4.1, the colon law).
//!
//! The measured absence this closes (card cli-root-ref-unexposed): MCP served
//! `root:path` at every door while `mrd read` sent the spelling to the engine
//! as a literal path — a real bound root failed exactly like a typo'd one,
//! and the teaching prescribed a workspace-relative respelling, advice for a
//! different mistake.
//!
//! The law under gate, by section of `docs/address-grammar.md`:
//! - §4.1 root-wins: a head colon is an address, never a literal path — a
//!   file literally named `sessionz:notes.md` in the ambient root must NOT
//!   be served by that spelling (no fallback);
//! - §5.1 C-2/F3/F4: a rooted read serves bytes from the NAMED root, never
//!   the ambient same-basename decoy; a miss is scoped to that root;
//! - §4.5: the malformed heads refuse with the grammar's own teaching;
//! - D3: a colon after the first `/` stays an ordinary path byte.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

mod common;

/// The TRUE target, in the `sessions` root.
const TARGET: &str = "# Notes\n\n## Design\n\nthe sessions root's real design note.\n";
const TARGET_PHRASE: &str = "the sessions root's real design note.";
/// The decoy: same basename, ambient root, different bytes — the file a
/// defective lane resolves `sessions:notes.md` onto.
const DECOY: &str = "# Notes\n\n## Design\n\nAMBIENT ROOT FILE, the wrong document.\n";
const DECOY_PHRASE: &str = "AMBIENT ROOT FILE";

/// The two teachings a rooted refusal must NOT carry — each diagnoses an
/// ambient-lane mistake the caller did not make (the card's measured defect).
const AMBIENT_RESPELL_PHRASE: &str = "check the workspace-relative spelling";
const CWD_RESPELL_PHRASE: &str = "Did you mean";

struct Sandbox {
    /// Held for its Drop — the sandbox tree dies with it.
    #[allow(dead_code)]
    tmp: tempfile::TempDir,
    cache_home: PathBuf,
    home: PathBuf,
    /// The ambient workspace (git-anchored), holding the decoy.
    ws: PathBuf,
    /// The mounted root's directory, holding the true target.
    sessions: PathBuf,
}

fn sandbox() -> Sandbox {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().join("home");
    let ws = tmp.path().join("project");
    let sessions = tmp.path().join("sessions");
    std::fs::create_dir_all(ws.join(".git")).expect("git anchor");
    for d in [&home, &sessions] {
        std::fs::create_dir_all(d).expect("mkdir");
    }

    // The mounted root declares its own canonical name (INV-5) — without this
    // the bind renders grey(undeclared) and every acceptance below is vacuous.
    std::fs::write(
        sessions.join("MERIDIAN.md"),
        "---\ntype: meridian-root\nversion: 1\nname: sessions\n---\n\n# Sessions root\n",
    )
    .expect("root declaration");
    std::fs::write(sessions.join("notes.md"), TARGET).expect("target");
    std::fs::write(ws.join("notes.md"), DECOY).expect("decoy");

    let config = home.join("MERIDIAN.md");
    let raw = format!(
        "---\ntype: meridian-config\nversion: 1\n---\n\n# Test roots\n\n\
         ```meridian-mount\nname: sessions\npath: {}\nvault: sessions\n```\n",
        sessions.display()
    );
    std::fs::write(&config, raw).expect("config");

    let cache_home = tmp.path().join("xdg-cache");
    Sandbox {
        tmp,
        cache_home,
        home,
        ws,
        sessions,
    }
}

impl Sandbox {
    fn base(&self) -> Command {
        let mut cmd = common::mrd_command(&self.home, &self.cache_home);
        cmd.env("MERIDIAN_CONFIG", self.home.join("MERIDIAN.md"))
            .env_remove("MERIDIAN_WORKSPACE");
        cmd
    }

    /// Run with the real daemon reachable (auto-spawn allowed).
    fn run_warm(&self, cwd: &Path, args: &[&str]) -> Output {
        self.base()
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("spawn mrd")
    }

    /// Run spawn-impossible, so the answer is deterministically the
    /// in-process degrade.
    fn run_degraded(&self, cwd: &Path, args: &[&str]) -> Output {
        self.base()
            .env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-daemon")
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("spawn mrd")
    }

    fn daemon_pidfile(&self) -> PathBuf {
        common::child_daemon_pidfile(&self.home, &self.cache_home)
    }

    fn wait_daemon_pid(&self, timeout: Duration) -> Option<i32> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Ok(text) = std::fs::read_to_string(self.daemon_pidfile())
                && let Ok(pid) = text.trim().parse::<i32>()
            {
                return Some(pid);
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        None
    }
}

/// Send `signal` to `pid` (a detached daemon we do not own as a child).
fn signal(pid: i32, signal: libc::c_int) {
    // SAFETY: a plain `kill(2)` on a pid the daemon wrote to its own pidfile.
    unsafe {
        libc::kill(pid, signal);
    }
}

/// Poll until `pid` is gone, so a failed assertion never leaks the daemon.
fn wait_dead(pid: i32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        // SAFETY: signal 0 probes existence without delivering a signal.
        if unsafe { libc::kill(pid, 0) } == -1 {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    false
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}
fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}
fn code(out: &Output) -> i32 {
    out.status.code().unwrap_or(-1)
}

/// The F3 acceptance, on one transport: the named root's bytes serve, the
/// ambient decoy never does, and the face echoes the caller's rooted
/// spelling.
fn assert_serves_the_named_root(out: &Output, plane: &str) {
    assert_eq!(
        code(out),
        0,
        "{plane}: a bound-root read serves: {}",
        stderr(out)
    );
    let body = stdout(out);
    assert!(
        body.contains(TARGET_PHRASE),
        "{plane}: the bytes come from the sessions root: {body:?}"
    );
    assert!(
        !body.contains(DECOY_PHRASE),
        "{plane}: the ambient decoy must never serve (§5.1 C-2): {body:?}"
    );
    assert!(
        body.contains("sessions:notes.md"),
        "{plane}: the face echoes the caller's rooted spelling: {body:?}"
    );
}

// ---------------------------------------------------------------------------
// The acceptance half — a real bound-root CLI read WORKS (quality gate 1).
// ---------------------------------------------------------------------------

#[test]
fn degraded_rooted_read_serves_the_named_roots_bytes_never_the_decoy() {
    let sb = sandbox();
    let out = sb.run_degraded(&sb.ws, &["read", "sessions:notes.md", "--section", "1"]);
    assert_serves_the_named_root(&out, "degrade");
}

#[test]
fn warm_rooted_read_serves_the_named_roots_bytes_never_the_decoy() {
    let sb = sandbox();

    // The warm control (g1's control law): prove the daemon arm really is
    // warm, or a spawn-broken environment turns this into a second degrade
    // test and quietly voids the transport half.
    let control = sb.run_warm(&sb.ws, &["read", "sessions:notes.md", "--json"]);
    let served = sb.run_warm(&sb.ws, &["read", "sessions:notes.md", "--section", "1"]);

    // Reap the auto-spawned daemon BEFORE asserting, so a failed assertion
    // never leaks it.
    let pid = sb.wait_daemon_pid(Duration::from_secs(5));
    if let Some(pid) = pid {
        signal(pid, libc::SIGTERM);
        wait_dead(pid, Duration::from_secs(5));
    }

    assert!(pid.is_some(), "the auto-spawned daemon wrote a pidfile");
    assert!(
        stdout(&control).contains("\"source\": \"daemon\""),
        "the control read must really be daemon-backed: {}",
        stdout(&control)
    );
    // The frame's workspace names the TARGET root — the rooted read bound to
    // it, not to the ambient workspace the caller stands in.
    let canonical_sessions = std::fs::canonicalize(&sb.sessions).expect("canonical sessions");
    assert!(
        stdout(&control).contains(&canonical_sessions.display().to_string()),
        "the --json frame's workspace is the named root's bound path: {}",
        stdout(&control)
    );
    assert_serves_the_named_root(&served, "warm");
}

/// The fragment rides the rooted spelling through the same selector door:
/// `sessions:notes.md#1.1` is a section read of the Design node inside the
/// NAMED root (a dewey fragment moves onto `sections`, Law A-2 routing).
#[test]
fn a_fragment_rides_a_rooted_spelling() {
    let sb = sandbox();
    let out = sb.run_degraded(&sb.ws, &["read", "sessions:notes.md#1.1"]);
    assert_eq!(code(&out), 0, "frag + rooted serves: {}", stderr(&out));
    assert!(
        stdout(&out).contains(TARGET_PHRASE),
        "the Design body comes from the sessions root: {}",
        stdout(&out)
    );
}

// ---------------------------------------------------------------------------
// §4.1 root-wins — no fallback to a literal reading, ever.
// ---------------------------------------------------------------------------

/// An unbound root refuses, NAMES the bound roots, and never diagnoses a
/// workspace-relative typo — even when a file literally named
/// `sessionz:notes.md` sits in the ambient root waiting to be misresolved.
#[test]
fn an_unbound_root_refuses_naming_the_bound_roots_never_a_literal_lookup() {
    let sb = sandbox();
    // The trap: the literal file EXISTS. §4.1 forbids the fallback that
    // would serve it — a wrong success of exactly the misresolve shape.
    std::fs::write(sb.ws.join("sessionz:notes.md"), DECOY).expect("literal trap");

    let out = sb.run_degraded(&sb.ws, &["read", "sessionz:notes.md"]);
    assert_eq!(code(&out), 1, "an unbound root is a refusal, not a serve");
    let err = stderr(&out);
    assert!(
        err.contains("does not bind") && err.contains("bound roots: sessions"),
        "the refusal names the miss and enumerates what DOES bind: {err:?}"
    );
    assert!(
        !stdout(&out).contains(DECOY_PHRASE),
        "the literal file must never serve through a rooted spelling"
    );
    for wrong in [AMBIENT_RESPELL_PHRASE, CWD_RESPELL_PHRASE] {
        assert!(
            !err.contains(wrong),
            "the refusal must not diagnose a workspace-relative typo ({wrong:?}) — \
             that is advice for a different mistake: {err:?}"
        );
    }
}

/// The malformed heads of §4.5 refuse with the grammar's own teaching.
#[test]
fn malformed_rooted_heads_refuse_with_the_grammar_teaching() {
    let sb = sandbox();
    for (spelling, phrase) in [
        ("Sessions:notes.md", "is not a canonical root name"),
        ("a:b:c.md", "more than one `:`"),
        (":notes.md", "names no root"),
        ("sessions:", "names no path"),
    ] {
        let out = sb.run_degraded(&sb.ws, &["read", spelling]);
        assert_eq!(code(&out), 1, "{spelling}: a malformed head refuses");
        assert!(
            stderr(&out).contains(phrase),
            "{spelling}: the refusal teaches its own arm ({phrase:?}): {:?}",
            stderr(&out)
        );
    }
}

/// The `--json` face keeps its `{workspace, error}` frame on the rooted
/// refusal leg — a machine consumer cannot tell an absent frame from success
/// with no output.
#[test]
fn a_rooted_refusal_emits_the_json_error_frame() {
    let sb = sandbox();
    let out = sb.run_degraded(&sb.ws, &["read", "sessionz:notes.md", "--json"]);
    assert_eq!(code(&out), 1);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("refusal frame parses ({e}): {}", stdout(&out)));
    assert!(v.get("workspace").is_some(), "frame carries workspace: {v}");
    assert!(
        v["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("does not bind")),
        "frame carries the teaching refusal: {v}"
    );
}

// ---------------------------------------------------------------------------
// The acceptance rows that keep the law from swallowing the ordinary corpus.
// ---------------------------------------------------------------------------

/// D3: a colon after the first `/` is an ordinary path byte — the literal
/// file serves in the ambient root.
#[test]
fn a_colon_after_the_first_slash_stays_a_literal_path() {
    let sb = sandbox();
    std::fs::create_dir_all(sb.ws.join("dir")).expect("dir");
    std::fs::write(
        sb.ws.join("dir").join("a:b.md"),
        "# X\n\nliteral colon file.\n",
    )
    .expect("literal member");
    let out = sb.run_degraded(&sb.ws, &["read", "dir/a:b.md", "--section", "1"]);
    assert_eq!(code(&out), 0, "D3 serves: {}", stderr(&out));
    assert!(
        stdout(&out).contains("literal colon file."),
        "the literal member serves by its own spelling: {}",
        stdout(&out)
    );
}

/// F4: a miss inside a BOUND root is scoped to that root — never the ambient
/// file, and never the ambient cwd respelling, even when the cwd holds the
/// exact rel path that would trigger it.
#[test]
fn a_miss_inside_a_bound_root_is_scoped_to_that_root() {
    let sb = sandbox();
    // The respell trap: `nope.md` exists relative to the caller's cwd, so
    // the AMBIENT lane's "Did you mean" would fire on this input.
    std::fs::write(sb.ws.join("nope.md"), DECOY).expect("cwd trap");

    let out = sb.run_degraded(&sb.ws, &["read", "sessions:nope.md"]);
    assert_eq!(code(&out), 1, "a bound-root miss refuses");
    let err = stderr(&out);
    assert!(
        err.contains("root `sessions`"),
        "the miss names WHICH root was searched (F4): {err:?}"
    );
    assert!(
        !err.contains(CWD_RESPELL_PHRASE),
        "the ambient cwd respelling never runs on the rooted lane: {err:?}"
    );
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        // Reap the daemon this sandbox auto-spawned (common::reap_daemon documents
        // the fixture daemon strategy). Runs before the TempDir fields drop, so
        // the pidfile is still on disk; never panics.
        let _ = common::reap_daemon(&self.home, &self.cache_home);
    }
}
