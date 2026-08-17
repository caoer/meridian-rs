//! The mint door's rooted lane — `[root:]path` on `mrd fingerprint`, resolved
//! to the named root's bound workspace (address-grammar §4.1, the colon law).
//!
//! The measured false ACCEPT this closes (card
//! cli-fingerprint-rooted-ref-absent): after the read door grew its rooted
//! lane (#124), `mrd read meridian-rs:docs/laws.md` served while
//! `mrd fingerprint` on the SAME ref sent the spelling to the engine as a
//! literal scope and minted `absent`, exit 0 — a legal §5.6 premise asserting
//! nothing exists at the literal string in the ambient workspace. That
//! assertion is permanently true: the guard built on it can never fire. Two
//! doors held two opinions of one ref, and the natural workflow — read, then
//! mint — walked straight into the bogus premise.
//!
//! The law under gate, by section of `docs/address-grammar.md`:
//! - §4.1 root-wins: a head colon is an address, never a literal scope — a
//!   bound root's node mints the token that root's OWN workspace mints, and
//!   a file literally named `sessionz:notes.md` in the ambient root must NOT
//!   mint by that spelling (no fallback);
//! - §5.1 C-2: the mint binds the NAMED root, never the ambient
//!   same-basename decoy;
//! - §4.5: the malformed heads refuse with the grammar's own teaching, and an
//!   unbound root refuses as a ROOT problem — exit 1, never `absent`;
//! - §5.6: a genuinely missing path inside a BOUND root still mints `absent`
//!   — lawful absence stays a value, not an error;
//! - D3: a colon after the first `/` stays an ordinary path byte.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

fn mrd_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mrd")
}

/// The TRUE target, in the `sessions` root.
const TARGET: &str = "# Notes\n\n## Design\n\nthe sessions root's real design note.\n";
/// The decoy: same basename, ambient root, different bytes — the node a
/// defective lane mints instead of the named root's.
const DECOY: &str = "# Notes\n\n## Design\n\nAMBIENT ROOT FILE, the wrong document.\n";

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
        let mut cmd = Command::new(mrd_bin());
        cmd.env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
            .env("MERIDIAN_CONFIG", self.home.join("MERIDIAN.md"))
            .env_remove("MERIDIAN_WORKSPACE");
        cmd
    }

    /// Run with the real daemon reachable (auto-spawn allowed) — the mint has
    /// no in-process degrade, so every served leg below is daemon-backed by
    /// construction.
    fn run_warm(&self, cwd: &Path, args: &[&str]) -> Output {
        self.base()
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("spawn mrd")
    }

    /// Run spawn-impossible. The mint door cannot degrade, so anything that
    /// answers here answered BEFORE any dial — the refusal-precedes-daemon
    /// half of the lane.
    fn run_undaemoned(&self, cwd: &Path, args: &[&str]) -> Output {
        self.base()
            .env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-daemon")
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("spawn mrd")
    }

    fn daemon_pidfile(&self) -> PathBuf {
        self.cache_home
            .join("meridian")
            .join("registry")
            .join("daemon.pid")
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

    /// Reap the auto-spawned daemon; call BEFORE asserting so a failed
    /// assertion never leaks it.
    fn reap_daemon(&self) -> Option<i32> {
        let pid = self.wait_daemon_pid(Duration::from_secs(5));
        if let Some(pid) = pid {
            signal(pid, libc::SIGTERM);
            wait_dead(pid, Duration::from_secs(5));
        }
        pid
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

/// The minted token off the human face (`fingerprint: <token>`), asserting
/// the mint served at all.
fn token(out: &Output, what: &str) -> String {
    assert_eq!(code(out), 0, "{what}: the mint serves: {}", stderr(out));
    stdout(out)
        .lines()
        .find_map(|l| l.strip_prefix("fingerprint: ").map(str::to_owned))
        .unwrap_or_else(|| panic!("{what}: no token line in {:?}", stdout(out)))
}

// ---------------------------------------------------------------------------
// The acceptance half — a real bound-root mint serves the NAMED root's token
// (quality gate 1: never `absent` for a node that exists).
// ---------------------------------------------------------------------------

/// One warm daemon serves three mints: the rooted ref from the ambient
/// workspace, the same node from inside its own root (the card's control),
/// and the ambient decoy. The rooted token equals the inside token and
/// differs from the decoy's — the mint bound the NAMED root, not the literal
/// string in the ambient workspace.
#[test]
fn a_rooted_mint_serves_the_named_roots_token_never_absent() {
    let sb = sandbox();
    let rooted = sb.run_warm(&sb.ws, &["fingerprint", "sessions:notes.md"]);
    let inside = sb.run_warm(&sb.sessions, &["fingerprint", "notes.md"]);
    let decoy = sb.run_warm(&sb.ws, &["fingerprint", "notes.md"]);
    let framed = sb.run_warm(&sb.ws, &["fingerprint", "sessions:notes.md", "--json"]);
    let pid = sb.reap_daemon();

    assert!(pid.is_some(), "the auto-spawned daemon wrote a pidfile");
    let rooted_token = token(&rooted, "rooted");
    let inside_token = token(&inside, "inside the root");
    let decoy_token = token(&decoy, "ambient decoy");
    assert_ne!(
        rooted_token, "absent",
        "the card's defect: a bound root's existing node must never mint `absent`"
    );
    assert_eq!(
        rooted_token, inside_token,
        "the rooted mint serves the token the node's OWN workspace serves"
    );
    assert_ne!(
        rooted_token, decoy_token,
        "the rooted mint never binds the ambient same-basename decoy (§5.1 C-2)"
    );
    assert!(
        stdout(&rooted).contains("scope: sessions:notes.md"),
        "the echo is the caller's rooted spelling (§4.7 desync guard): {:?}",
        stdout(&rooted)
    );

    // The `--json` frame: workspace names the root's bound path, and the mint
    // body's scope echo carries the caller's rooted spelling.
    assert_eq!(code(&framed), 0, "json mint serves: {}", stderr(&framed));
    let v: serde_json::Value = serde_json::from_slice(&framed.stdout)
        .unwrap_or_else(|e| panic!("mint frame parses ({e}): {}", stdout(&framed)));
    let canonical_sessions = std::fs::canonicalize(&sb.sessions).expect("canonical sessions");
    assert_eq!(
        v["workspace"].as_str().unwrap_or_default(),
        canonical_sessions.display().to_string(),
        "the frame's workspace is the named root's bound path: {v}"
    );
    assert_eq!(
        v["mint"]["scope"].as_str().unwrap_or_default(),
        "sessions:notes.md",
        "the json echo keeps the rooted spelling: {v}"
    );
    assert_eq!(
        v["mint"]["fingerprint"].as_str().unwrap_or_default(),
        rooted_token,
        "both faces mint one token: {v}"
    );
}

/// §5.6 stands inside the lane: a genuinely missing path in a BOUND root is
/// lawful absence — `absent`, exit 0, the rooted spelling echoed (quality
/// gate 3).
#[test]
fn a_missing_path_inside_a_bound_root_still_mints_absent() {
    let sb = sandbox();
    let out = sb.run_warm(&sb.ws, &["fingerprint", "sessions:nope.md"]);
    let pid = sb.reap_daemon();

    assert!(pid.is_some(), "the auto-spawned daemon wrote a pidfile");
    assert_eq!(
        token(&out, "bound-root miss"),
        "absent",
        "lawful absence stays a value: {}",
        stdout(&out)
    );
    assert!(
        stdout(&out).contains("scope: sessions:nope.md"),
        "the absent echo still carries the rooted spelling: {:?}",
        stdout(&out)
    );
}

/// D3: a colon after the first `/` is an ordinary path byte — the literal
/// member mints in the ambient root, no rooted lane entered.
#[test]
fn a_colon_after_the_first_slash_stays_a_literal_scope() {
    let sb = sandbox();
    std::fs::create_dir_all(sb.ws.join("dir")).expect("dir");
    std::fs::write(
        sb.ws.join("dir").join("a:b.md"),
        "# X\n\nliteral colon file.\n",
    )
    .expect("literal member");
    let out = sb.run_warm(&sb.ws, &["fingerprint", "dir/a:b.md"]);
    let pid = sb.reap_daemon();

    assert!(pid.is_some(), "the auto-spawned daemon wrote a pidfile");
    assert_ne!(
        token(&out, "D3 literal"),
        "absent",
        "the literal member mints by its own spelling: {}",
        stdout(&out)
    );
}

// ---------------------------------------------------------------------------
// §4.1 root-wins — a root problem refuses as a root problem, never `absent`,
// and the address answer precedes any dial (quality gate 2).
// ---------------------------------------------------------------------------

/// An unbound root refuses, NAMES the bound roots, and never mints — even
/// when a file literally named `sessionz:notes.md` sits in the ambient root
/// waiting to be misresolved, and even with no daemon reachable: the address
/// plane answers before the mint authority is ever dialed.
#[test]
fn an_unbound_root_refuses_as_a_root_problem_never_absent() {
    let sb = sandbox();
    // The trap: the literal file EXISTS. §4.1 forbids the fallback that
    // would mint it — a wrong success of exactly the misresolve shape.
    std::fs::write(sb.ws.join("sessionz:notes.md"), DECOY).expect("literal trap");

    let out = sb.run_undaemoned(&sb.ws, &["fingerprint", "sessionz:notes.md"]);
    assert_eq!(
        code(&out),
        1,
        "an unbound root is an address refusal, not a mint and not a daemon error: {}",
        stderr(&out)
    );
    let err = stderr(&out);
    assert!(
        err.contains("does not bind") && err.contains("bound roots: sessions"),
        "the refusal names the miss and enumerates what DOES bind: {err:?}"
    );
    assert!(
        !stdout(&out).contains("fingerprint:"),
        "no token is minted on the refusal leg: {:?}",
        stdout(&out)
    );
    assert!(
        !stdout(&out).contains("absent") && !err.contains("absent"),
        "a root problem never answers in absence vocabulary: {:?} / {err:?}",
        stdout(&out)
    );
}

/// The malformed heads of §4.5 refuse with the grammar's own teaching —
/// before any dial, never `absent`.
#[test]
fn malformed_rooted_heads_refuse_with_the_grammar_teaching() {
    let sb = sandbox();
    for (spelling, phrase) in [
        ("Sessions:notes.md", "is not a canonical root name"),
        ("a:b:c.md", "more than one `:`"),
        (":notes.md", "names no root"),
        ("sessions:", "names no path"),
    ] {
        let out = sb.run_undaemoned(&sb.ws, &["fingerprint", spelling]);
        assert_eq!(code(&out), 1, "{spelling}: a malformed head refuses");
        assert!(
            stderr(&out).contains(phrase),
            "{spelling}: the refusal teaches its own arm ({phrase:?}): {:?}",
            stderr(&out)
        );
        assert!(
            !stdout(&out).contains("absent"),
            "{spelling}: a grammar refusal never mints `absent`: {:?}",
            stdout(&out)
        );
    }
}

/// The `--json` face keeps its `{workspace, error}` frame on the rooted
/// refusal leg — a machine consumer cannot tell an absent frame from success
/// with no output.
#[test]
fn a_rooted_refusal_emits_the_json_error_frame() {
    let sb = sandbox();
    let out = sb.run_undaemoned(&sb.ws, &["fingerprint", "sessionz:notes.md", "--json"]);
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
