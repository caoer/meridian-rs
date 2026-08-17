//! The write door's rooted-scope lane — `--scope [root:]path` on `mrd put`
//! (card put-scope-rejects-rooted-mint-echo).
//!
//! The measured misclassification this closes: `mrd fingerprint
//! mrd-experiments:target.md` echoes the ROOTED spelling beside its token
//! (#126's §4.7 desync guard), but `mrd put --scope` sent that spelling to
//! the engine as a literal node name — a premise at a node that does not
//! exist — and the refusal surfaced as the §5.5 "no premise covers" coverage
//! answer, teaching the stripped spelling only inside a parenthetical. An
//! unbound root degraded into the SAME coverage refusal, so every root name,
//! real or typo'd, misclassified as a coverage fault (measured 2026-08-17 at
//! `fcb32381`, dry and commit legs alike).
//!
//! The law under gate (ruled Option 1, session 15-14-fingerprint-grain):
//! - the token and its own echoed scope, copied verbatim from one
//!   `mrd fingerprint` call into one `mrd put`, never refuse on the pair —
//!   measured on BOTH the `--dry` and the commit leg;
//! - a rooted `--scope` is accepted exactly when the named root binds the
//!   workspace the put writes; the rel half rides the wire, so the §5.4
//!   `scope` field stays workspace-relative and rooted and stripped
//!   spellings are ONE law, not two;
//! - a bound root binding a DIFFERENT workspace refuses naming both
//!   workspaces; an unbound root refuses as a root problem with the bound
//!   names enumerated (the `mrd resolve` posture); a `#` fragment refuses at
//!   path grain — all address answers (exit 1, `{workspace, error}` under
//!   `--json`), before any dial, never the coverage refusal;
//! - the fingerprint echo is UNCHANGED (the rooted spelling stays the §4.7
//!   desync guard's echo — this lane makes it pasteable instead of moving
//!   it);
//! - the blind-strip trap stays closed (safety constraint, dogfood
//!   f483c7da): a leaf token is CONTENT-only — byte-identical files mint one
//!   token — so `--scope` is the only thing binding a premise to a node. A
//!   foreign bound root over a basename that ALSO EXISTS LOCALLY, guarded by
//!   the LOCAL file's own leaf token, is the exact pair a
//!   strip-the-root-and-proceed implementation would accept; it must refuse
//!   on both legs with the disk untouched.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

fn mrd_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mrd")
}

/// The TRUE target, in the `sessions` root.
const TARGET: &str = "# Notes\n\n## Design\n\nthe sessions root's real design note.\n";
/// A batch that edits [`TARGET`]'s Design section — the §4.4 grammar.
const BATCH: &str = r#"[{"target":{"hpath":[{"h":"Notes"},{"h":"Design"}]},"edit":{"match":{"old":"real design note","new":"revised design note"}}}]"#;

struct Sandbox {
    /// Held for its Drop — the sandbox tree dies with it.
    #[allow(dead_code)]
    tmp: tempfile::TempDir,
    cache_home: PathBuf,
    home: PathBuf,
    /// The ambient workspace (git-anchored) — the workspace the cross-root
    /// and unbound-root puts write.
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
    std::fs::write(ws.join("doc.md"), "# Alpha\n\none two three\n").expect("ambient doc");

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

    /// Run with the real daemon reachable (auto-spawn allowed), feeding
    /// `stdin` — the write door has no in-process degrade, so every served
    /// leg below is daemon-backed by construction.
    fn run_warm(&self, cwd: &Path, args: &[&str], stdin: &str) -> Output {
        spawn_with_stdin(self.base().args(args).current_dir(cwd), stdin)
    }

    /// Run spawn-impossible. The rooted-scope walls answer BEFORE any dial,
    /// so anything that refuses here refused on the address plane — the
    /// refusal-precedes-daemon half of the lane.
    fn run_undaemoned(&self, cwd: &Path, args: &[&str], stdin: &str) -> Output {
        spawn_with_stdin(
            self.base()
                .env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-daemon")
                .args(args)
                .current_dir(cwd),
            stdin,
        )
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

/// One `mrd` run with `stdin` piped in whole — the write door reads its §4.4
/// batch there, and the mint door ignores it.
fn spawn_with_stdin(cmd: &mut Command, stdin: &str) -> Output {
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn mrd");
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(stdin.as_bytes())
        .expect("write stdin");
    child.wait_with_output().expect("wait mrd")
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

/// The mint's two lines off the human face — the token AND the echoed scope,
/// taken verbatim so the pair test pastes exactly what the mint answered.
fn mint_pair(out: &Output, what: &str) -> (String, String) {
    assert_eq!(code(out), 0, "{what}: the mint serves: {}", stderr(out));
    let text = stdout(out);
    let token = text
        .lines()
        .find_map(|l| l.strip_prefix("fingerprint: ").map(str::to_owned))
        .unwrap_or_else(|| panic!("{what}: no token line in {text:?}"));
    let scope = text
        .lines()
        .find_map(|l| l.trim_start().strip_prefix("scope: ").map(str::to_owned))
        .unwrap_or_else(|| panic!("{what}: no scope echo in {text:?}"));
    (token, scope)
}

/// The misclassification this card kills: no address-plane refusal may speak
/// in coverage vocabulary.
fn assert_never_coverage(out: &Output, what: &str) {
    assert!(
        !stderr(out).contains("no premise covers") && !stdout(out).contains("no premise covers"),
        "{what}: an address fault must never surface as premise coverage: {} / {}",
        stdout(out),
        stderr(out)
    );
}

// ---------------------------------------------------------------------------
// The acceptance half — the mint echo pastes into the write door verbatim
// (quality gate 1: dry AND commit), and rooted/stripped stay one law.
// ---------------------------------------------------------------------------

/// THE pair gate: mint rooted, then pass the token and its own echoed scope —
/// both parsed off the mint's answer, not retyped — into `put`. The `--dry`
/// leg rehearses exit 0, the commit leg lands the edit on disk. The dry leg
/// alone is a false green (the card measured both legs refusing).
#[test]
fn a_rooted_mints_echo_pastes_verbatim_into_put_dry_and_commit() {
    let sb = sandbox();
    let mint = sb.run_warm(&sb.sessions, &["fingerprint", "sessions:notes.md"], "");
    let (token, scope) = mint_pair(&mint, "rooted mint");
    let dry = sb.run_warm(
        &sb.sessions,
        &[
            "put",
            "notes.md",
            "--if-fingerprint",
            &token,
            "--scope",
            &scope,
            "--dry",
        ],
        BATCH,
    );
    let commit = sb.run_warm(
        &sb.sessions,
        &[
            "put",
            "notes.md",
            "--if-fingerprint",
            &token,
            "--scope",
            &scope,
        ],
        BATCH,
    );
    let pid = sb.reap_daemon();

    assert!(pid.is_some(), "the auto-spawned daemon wrote a pidfile");
    assert_eq!(
        scope, "sessions:notes.md",
        "the mint echo is the caller's rooted spelling (§4.7 desync guard, unchanged)"
    );
    assert_eq!(
        code(&dry),
        0,
        "the echoed pair rehearses without refusing: {}",
        stderr(&dry)
    );
    assert_eq!(
        code(&commit),
        0,
        "the echoed pair commits without refusing: {}",
        stderr(&commit)
    );
    let on_disk = std::fs::read_to_string(sb.sessions.join("notes.md")).expect("read target");
    assert!(
        on_disk.contains("revised design note"),
        "the commit leg landed the edit: {on_disk:?}"
    );
}

/// One node, one law: the rooted and stripped spellings mint one token, and
/// the stripped `--scope` keeps working exactly as before (the #110 lane, no
/// regression).
#[test]
fn rooted_and_stripped_spellings_are_one_law() {
    let sb = sandbox();
    let rooted = sb.run_warm(&sb.sessions, &["fingerprint", "sessions:notes.md"], "");
    let stripped = sb.run_warm(&sb.sessions, &["fingerprint", "notes.md"], "");
    let (rooted_token, _) = mint_pair(&rooted, "rooted mint");
    let (stripped_token, stripped_scope) = mint_pair(&stripped, "stripped mint");
    let dry = sb.run_warm(
        &sb.sessions,
        &[
            "put",
            "notes.md",
            "--if-fingerprint",
            &stripped_token,
            "--scope",
            &stripped_scope,
            "--dry",
        ],
        BATCH,
    );
    let pid = sb.reap_daemon();

    assert!(pid.is_some(), "the auto-spawned daemon wrote a pidfile");
    assert_eq!(
        rooted_token, stripped_token,
        "one node mints one token whichever spelling asked"
    );
    assert_eq!(
        stripped_scope, "notes.md",
        "the stripped mint echoes the stripped spelling"
    );
    assert_eq!(
        code(&dry),
        0,
        "the stripped lane keeps working: {}",
        stderr(&dry)
    );
}

// ---------------------------------------------------------------------------
// The refusal half — every rooted-scope fault is an address answer, before
// any dial, never the §5.5 coverage refusal (quality gates 2 and 3).
// ---------------------------------------------------------------------------

/// A bound root whose workspace is NOT the one this put writes refuses
/// naming both workspaces — never "no premise covers", and before any dial.
#[test]
fn a_cross_root_scope_refuses_naming_both_workspaces_never_coverage() {
    let sb = sandbox();
    let out = sb.run_undaemoned(
        &sb.ws,
        &[
            "put",
            "doc.md",
            "--if-fingerprint",
            "b3a:0000000000000000000000000000000000000000000000000000000000000000",
            "--scope",
            "sessions:notes.md",
        ],
        BATCH,
    );
    assert_eq!(
        code(&out),
        1,
        "a cross-root scope is an address refusal, before any dial: {}",
        stderr(&out)
    );
    let err = stderr(&out);
    assert!(
        err.contains("names root `sessions`") && err.contains("the workspace being written"),
        "the refusal names the root and the workspace law: {err:?}"
    );
    let canonical_sessions = std::fs::canonicalize(&sb.sessions).expect("canonical sessions");
    let canonical_ws = std::fs::canonicalize(&sb.ws).expect("canonical ws");
    assert!(
        err.contains(&canonical_sessions.display().to_string())
            && err.contains(&canonical_ws.display().to_string()),
        "the refusal names BOTH workspaces: {err:?}"
    );
    assert_never_coverage(&out, "cross-root scope");
}

/// The blind-strip trap (safety constraint, dogfood f483c7da): a foreign
/// bound root over a basename that ALSO EXISTS LOCALLY, guarded by the LOCAL
/// file's own leaf token — the exact pair a strip-the-root-and-proceed
/// implementation would accept, because a leaf token is content-only and the
/// stripped spelling plus the local token is a satisfied premise. Both legs
/// must refuse naming the root mismatch, and the commit leg must leave the
/// disk untouched. A basename with no local twin cannot fail this test: the
/// coverage check would refuse it for the wrong reason.
#[test]
fn a_foreign_root_scope_with_a_local_decoy_refuses_never_strips() {
    let sb = sandbox();
    // The local decoy: same basename as the sessions root's file, different
    // bytes, sitting in the AMBIENT workspace this put writes.
    let decoy = "# Notes\n\n## Design\n\nambient decoy body.\n";
    std::fs::write(sb.ws.join("notes.md"), decoy).expect("local decoy");
    let batch = r#"[{"target":{"hpath":[{"h":"Notes"},{"h":"Design"}]},"edit":{"match":{"old":"ambient decoy body","new":"counterfeit edit"}}}]"#;
    // The LOCAL file's own leaf token, minted ambient inside ws — the token
    // that makes the stripped premise TRUE.
    let mint = sb.run_warm(&sb.ws, &["fingerprint", "notes.md"], "");
    let (token, local_scope) = mint_pair(&mint, "ambient decoy mint");
    let dry = sb.run_warm(
        &sb.ws,
        &[
            "put",
            "notes.md",
            "--if-fingerprint",
            &token,
            "--scope",
            "sessions:notes.md",
            "--dry",
        ],
        batch,
    );
    let commit = sb.run_warm(
        &sb.ws,
        &[
            "put",
            "notes.md",
            "--if-fingerprint",
            &token,
            "--scope",
            "sessions:notes.md",
        ],
        batch,
    );
    let pid = sb.reap_daemon();

    assert!(pid.is_some(), "the auto-spawned daemon wrote a pidfile");
    assert_eq!(
        local_scope, "notes.md",
        "the ambient mint echoes the stripped spelling the trap would ride"
    );
    for (leg, out) in [("dry", &dry), ("commit", &commit)] {
        assert_eq!(
            code(out),
            1,
            "{leg}: a foreign-root scope refuses even with the local leaf token: {} / {}",
            stdout(out),
            stderr(out)
        );
        assert!(
            stderr(out).contains("names root `sessions`"),
            "{leg}: the refusal names the root mismatch, never silent stripping: {:?}",
            stderr(out)
        );
        assert_never_coverage(out, leg);
    }
    let on_disk = std::fs::read_to_string(sb.ws.join("notes.md")).expect("read decoy");
    assert_eq!(
        on_disk, decoy,
        "the commit leg wrote NOTHING through the trap"
    );
}

/// An unbound root refuses as a root problem with the bound names enumerated
/// (the `mrd resolve` posture) — never "no premise covers" — even when a
/// file literally named `nosuchroot:notes.md` sits in the workspace waiting
/// to be misresolved (§4.1: the root reading wins, no fallback).
#[test]
fn an_unbound_root_scope_refuses_as_a_root_problem_never_coverage() {
    let sb = sandbox();
    // The trap: the literal file EXISTS and would cover the target's file as
    // its own premise node. §4.1 forbids the literal reading that would send
    // it — the pre-lane defect answered coverage here.
    std::fs::write(sb.ws.join("nosuchroot:notes.md"), "# T\n\nx\n").expect("literal trap");
    let out = sb.run_undaemoned(
        &sb.ws,
        &[
            "put",
            "doc.md",
            "--if-fingerprint",
            "b3a:0000000000000000000000000000000000000000000000000000000000000000",
            "--scope",
            "nosuchroot:notes.md",
        ],
        BATCH,
    );
    assert_eq!(
        code(&out),
        1,
        "an unbound root is an address refusal, before any dial: {}",
        stderr(&out)
    );
    let err = stderr(&out);
    assert!(
        err.contains("does not bind") && err.contains("bound roots: sessions"),
        "the refusal names the miss and enumerates what DOES bind: {err:?}"
    );
    assert_never_coverage(&out, "unbound-root scope");
}

/// A `#` fragment on a rooted scope refuses at path grain (the resolve
/// door's posture): a §5.4 premise binds a node, and silently stripping the
/// fragment would bind a premise the caller did not spell.
#[test]
fn a_fragment_bearing_rooted_scope_refuses_at_path_grain() {
    let sb = sandbox();
    let out = sb.run_undaemoned(
        &sb.ws,
        &[
            "put",
            "doc.md",
            "--if-fingerprint",
            "b3a:0000000000000000000000000000000000000000000000000000000000000000",
            "--scope",
            "sessions:notes.md#Design",
        ],
        BATCH,
    );
    assert_eq!(code(&out), 1, "a fragment refuses: {}", stderr(&out));
    let err = stderr(&out);
    assert!(
        err.contains("carries a `#` fragment") && err.contains("path grain"),
        "the refusal teaches the grain law: {err:?}"
    );
    assert_never_coverage(&out, "fragment-bearing scope");
}

/// The `--json` face keeps its `{workspace, error}` frame on every
/// rooted-scope refusal leg (the card's error-shape gate).
#[test]
fn a_rooted_scope_refusal_emits_the_json_error_frame() {
    let sb = sandbox();
    let out = sb.run_undaemoned(
        &sb.ws,
        &[
            "put",
            "doc.md",
            "--if-fingerprint",
            "b3a:0000000000000000000000000000000000000000000000000000000000000000",
            "--scope",
            "nosuchroot:notes.md",
            "--json",
        ],
        BATCH,
    );
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
