//! The §4.6 family gates for the doors converted by rooted-refs-everywhere:
//! every page-taking door RESOLVES a `root:page` ref instead of
//! refusing or misreading it. Two gates per converted door, modeled on
//! `read_rooted_ref.rs`:
//!
//! 1. a rooted ref resolves and serves/acts on the NAMED root's tree;
//! 2. an unbound root refuses (exit 1; exit 2 on the pre-corpus doors' own
//!    refusal leg) naming the bound roots, and never falls back to a
//!    literal-path reading — even when a file literally named `root:page.md`
//!    sits in the ambient workspace waiting to be misresolved.
//!
//! Plus the authority gate (the ratified ruling: the page tree governs): a
//! rooted `mrd run` lands its receipt in the PAGE's workspace, never the
//! standing one. The preset lane (`unfold`/`reconcile`/`new`) gates its
//! refuse-with-teaching stance, and `sql --root` gates the bare-name selector.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

mod common;

/// The TRUE target, in the `sessions` root.
const TARGET: &str = "# Notes\n\n## Design\n\nthe sessions root's real design note.\n";
/// The decoy: same basename, ambient root, different bytes.
const DECOY: &str = "# Notes\n\n## Design\n\nAMBIENT ROOT FILE, the wrong document.\n";
const DECOY_PHRASE: &str = "AMBIENT ROOT FILE";
/// The ambient cwd-respell teaching a rooted refusal must never carry.
const CWD_RESPELL_PHRASE: &str = "Did you mean";

/// A task page for the run door: one bash task that leaves a receipt.
const JOB: &str = "\
---
task.say: \"[[#^say-1]]\"
---

# Job

```bash
echo rooted-run-ok
```
^say-1
";

/// A realising page whose claim is converged as written.
const CLAIM: &str = "\
---
status: green
realise.field: status
realise.expected: green
---

# Claim
";

struct Sandbox {
    #[allow(dead_code)]
    tmp: tempfile::TempDir,
    cache_home: PathBuf,
    home: PathBuf,
    /// The ambient workspace (git-anchored), holding the decoys.
    ws: PathBuf,
    /// The `sessions` root, holding the true targets.
    sessions: PathBuf,
}

fn sandbox() -> Sandbox {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().join("home");
    let ws = tmp.path().join("project");
    let sessions = tmp.path().join("sessions");
    let assets = tmp.path().join("assets");
    std::fs::create_dir_all(ws.join(".git")).expect("git anchor");
    for d in [&home, &sessions, &assets] {
        std::fs::create_dir_all(d).expect("mkdir");
    }

    // Each mounted root declares its own canonical name (INV-5).
    std::fs::write(
        sessions.join("MERIDIAN.md"),
        "---\ntype: meridian-root\nversion: 1\nname: sessions\n---\n\n# Sessions root\n",
    )
    .expect("sessions declaration");
    std::fs::write(
        assets.join("MERIDIAN.md"),
        "---\ntype: meridian-root\nversion: 1\nname: assets\n---\n\n# Assets root\n",
    )
    .expect("assets declaration");
    std::fs::write(sessions.join("notes.md"), TARGET).expect("target");
    std::fs::write(sessions.join("only-there.md"), "# Only\n\nhere.\n").expect("only-there");
    std::fs::write(sessions.join("job.md"), JOB).expect("job");
    std::fs::write(sessions.join("claim.md"), CLAIM).expect("claim");
    std::fs::write(assets.join("notes.md"), "# Assets\n\nasset notes.\n").expect("assets notes");
    std::fs::write(ws.join("notes.md"), DECOY).expect("decoy");

    let config = home.join("MERIDIAN.md");
    let raw = format!(
        "---\ntype: meridian-config\nversion: 1\n---\n\n# Test roots\n\n\
         ```meridian-mount\nname: sessions\npath: {}\nvault: sessions\n```\n\n\
         ```meridian-mount\nname: assets\npath: {}\nvault: assets\n```\n",
        sessions.display(),
        assets.display()
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

    /// Run spawn-impossible, so no resident daemon outlives the test.
    fn run_degraded(&self, cwd: &Path, args: &[&str]) -> Output {
        self.base()
            .env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-daemon")
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("spawn mrd")
    }

    /// Run with the real daemon reachable (auto-spawn allowed) — the write
    /// doors have no degrade leg.
    fn run_warm(&self, cwd: &Path, args: &[&str]) -> Output {
        self.base()
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("spawn mrd")
    }

    /// Warm run with bytes on stdin (the put door's edits).
    fn run_warm_stdin(&self, cwd: &Path, args: &[&str], stdin: &str) -> Output {
        let mut child = self
            .base()
            .args(args)
            .current_dir(cwd)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn mrd");
        common::feed_stdin(&mut child, stdin.as_bytes());
        child.wait_with_output().expect("wait mrd")
    }

    fn daemon_pidfile(&self) -> PathBuf {
        common::child_daemon_pidfile(&self.home, &self.cache_home)
    }

    /// Reap the auto-spawned daemon so a failed assertion never leaks it.
    fn reap_daemon(&self) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Ok(text) = std::fs::read_to_string(self.daemon_pidfile())
                && let Ok(pid) = text.trim().parse::<i32>()
            {
                // SAFETY: plain kill(2) on the pid the daemon wrote itself.
                unsafe {
                    libc::kill(pid, libc::SIGTERM);
                }
                let dead = Instant::now() + Duration::from_secs(5);
                while Instant::now() < dead {
                    // SAFETY: signal 0 probes existence only.
                    if unsafe { libc::kill(pid, 0) } == -1 {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
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

/// The unbound-root refusal every converted door owes: names the miss,
/// enumerates what DOES bind, and never diagnoses a workspace-relative typo.
fn assert_unbound_refusal(out: &Output, door: &str) {
    let err = stderr(out);
    assert!(
        err.contains("does not bind") && err.contains("bound roots:"),
        "{door}: the refusal names the miss and enumerates what binds: {err:?}"
    );
    assert!(
        !err.contains(CWD_RESPELL_PHRASE),
        "{door}: the refusal must not diagnose a workspace-relative typo: {err:?}"
    );
    assert!(
        !stdout(out).contains(DECOY_PHRASE),
        "{door}: the literal/ambient file must never serve through a rooted spelling"
    );
}

// ── walk ─────────────────────────────────────────────────────────────────────

#[test]
fn walk_serves_the_named_root_and_echoes_the_spelling() {
    let sb = sandbox();
    let out = sb.run_degraded(&sb.ws, &["walk", "sessions:notes.md"]);
    assert_eq!(code(&out), 0, "a bound-root walk serves: {}", stderr(&out));
    let body = stdout(&out);
    assert!(
        body.contains("walk up sessions:notes.md"),
        "the header echoes the caller's rooted spelling: {body:?}"
    );
    assert!(
        body.contains("notes.md @"),
        "the rev citation reads the named root's page: {body:?}"
    );
}

#[test]
fn walk_unbound_root_refuses_never_a_literal_walk() {
    let sb = sandbox();
    std::fs::write(sb.ws.join("sessionz:notes.md"), DECOY).expect("literal trap");
    let out = sb.run_degraded(&sb.ws, &["walk", "sessionz:notes.md"]);
    assert_eq!(code(&out), 1, "an unbound root is an address refusal");
    assert_unbound_refusal(&out, "walk");
}

// ── repair ───────────────────────────────────────────────────────────────────

#[test]
fn repair_scans_the_named_root() {
    let sb = sandbox();
    // `only-there.md` exists ONLY in the sessions root: a served scan proves
    // the corpus read bound there, not to the ambient workspace.
    let out = sb.run_degraded(&sb.ws, &["repair", "sessions:only-there.md"]);
    assert_eq!(
        code(&out),
        0,
        "a rooted repair scans the named root's corpus: {}",
        stderr(&out)
    );
}

#[test]
fn repair_rooted_miss_is_scoped_to_the_named_root() {
    let sb = sandbox();
    // The respell trap: `nope.md` exists relative to the caller's cwd.
    std::fs::write(sb.ws.join("nope.md"), DECOY).expect("cwd trap");
    let out = sb.run_degraded(&sb.ws, &["repair", "sessions:nope.md"]);
    assert_eq!(code(&out), 2, "a bound-root miss is the door's own refusal");
    let err = stderr(&out);
    assert!(
        err.contains("root `sessions`"),
        "the miss names WHICH root was searched: {err:?}"
    );
    assert!(
        !err.contains(CWD_RESPELL_PHRASE),
        "no ambient cwd respelling on the rooted lane: {err:?}"
    );
}

#[test]
fn repair_unbound_root_refuses() {
    let sb = sandbox();
    let out = sb.run_degraded(&sb.ws, &["repair", "sessionz:notes.md"]);
    assert_eq!(code(&out), 1);
    assert_unbound_refusal(&out, "repair");
}

// ── run (incl. the authority gate) ───────────────────────────────────────────

#[test]
fn run_lists_tasks_from_the_named_root() {
    let sb = sandbox();
    let out = sb.run_degraded(&sb.ws, &["run", "sessions:job.md", "--list"]);
    assert_eq!(code(&out), 0, "rooted --list serves: {}", stderr(&out));
    assert!(
        stdout(&out).contains("say"),
        "the listing reads the named root's page: {}",
        stdout(&out)
    );
}

/// The authority ruling's receipt half: a rooted run's receipt lands in the
/// PAGE's workspace, never the standing one.
#[test]
fn run_receipt_lands_in_the_page_tree_never_the_standing_one() {
    let sb = sandbox();
    let out = sb.run_degraded(&sb.ws, &["run", "sessions:job.md", "say"]);
    assert_eq!(code(&out), 0, "the rooted run executes: {}", stderr(&out));
    assert!(
        sb.sessions.join("receipts/run.md").is_file(),
        "the receipt lands in the page's workspace (authority: the page tree governs)"
    );
    assert!(
        !sb.ws.join("receipts").exists(),
        "the standing workspace gains no receipt"
    );
}

#[test]
fn run_unbound_root_refuses_and_executes_nothing() {
    let sb = sandbox();
    // The trap: a literal file with a runnable task sits in the ambient root.
    std::fs::write(sb.ws.join("sessionz:job.md"), JOB).expect("literal trap");
    let out = sb.run_degraded(&sb.ws, &["run", "sessionz:job.md", "say"]);
    assert_eq!(code(&out), 1, "an unbound root is an address refusal");
    assert_unbound_refusal(&out, "run");
    assert!(
        !sb.ws.join("receipts").exists(),
        "nothing was executed and no receipt was written"
    );
}

/// A page in the `guarded` root whose task explicitly grants `md.edit` —
/// which that root's own convention ceiling narrows to nothing.
const GUARDED_TASK: &str = "\
---
status: open
task.fix-status: \"[[#^fx-1]]\"
task.fix-status.caps: md.edit
---

# Guarded

```starlark
def run(ctx):
    set_field(field = \"status\", value = \"done\")
```
^fx-1
";

/// Grow the sandbox a `guarded` root that declares a read-only convention
/// (`run.caps.fix-*: \"\"` — an explicit empty ceiling) over [`GUARDED_TASK`],
/// mounted beside `sessions`/`assets`.
fn add_guarded_root(sb: &Sandbox) -> PathBuf {
    let guarded = sb.home.parent().expect("sandbox layout").join("guarded");
    std::fs::create_dir_all(guarded.join(".git")).expect("guarded anchor");
    std::fs::write(
        guarded.join("MERIDIAN.md"),
        "---\ntype: meridian-root\nversion: 1\nname: guarded\n\"run.caps.fix-*\": \"\"\n---\n\n\
         # Guarded root\n",
    )
    .expect("guarded declaration");
    std::fs::write(guarded.join("task.md"), GUARDED_TASK).expect("guarded task");
    let config = sb.home.join("MERIDIAN.md");
    let mut raw = std::fs::read_to_string(&config).expect("config");
    {
        use std::fmt::Write as _;
        let _ = write!(
            raw,
            "\n```meridian-mount\nname: guarded\npath: {}\nvault: guarded\n```\n",
            guarded.display()
        );
    }
    std::fs::write(&config, raw).expect("config grows guarded");
    guarded
}

/// The ceiling half of the ratified authority ruling (the permission-bypass
/// hazard that motivated it): the PAGE tree's read-only convention governs a
/// rooted run no matter where the caller stands. The standing tree declares
/// NOTHING, so a regression of `declaring_root` back to the standing tree
/// would lift the ceiling and let the task run — this gate fails then.
#[test]
fn a_read_only_page_tree_is_not_bypassable_by_standing_elsewhere() {
    let sb = sandbox();
    let guarded = add_guarded_root(&sb);
    let out = sb.run_degraded(&sb.ws, &["run", "guarded:task.md", "fix-status"]);
    assert_eq!(
        code(&out),
        1,
        "the PAGE tree's ceiling refuses the effect: {} {}",
        stdout(&out),
        stderr(&out)
    );
    let err = stderr(&out);
    assert!(
        err.contains("capability denied"),
        "the refusal is the caps fault, not an address or page miss: {err:?}"
    );
    let page = std::fs::read_to_string(guarded.join("task.md")).expect("task page");
    assert!(
        page.contains("status: open"),
        "nothing was applied under the ceiling: {page:?}"
    );
}

/// The mirror: the same task invoked from INSIDE the page tree refuses
/// identically — the refusal is that tree's ceiling, not an artifact of the
/// rooted lane.
#[test]
fn the_ceiling_refuses_identically_from_inside_the_page_tree() {
    let sb = sandbox();
    let guarded = add_guarded_root(&sb);
    let out = sb.run_degraded(&guarded, &["run", "task.md", "fix-status"]);
    assert_eq!(
        code(&out),
        1,
        "the same ceiling binds ambient too: {} {}",
        stdout(&out),
        stderr(&out)
    );
    assert!(
        stderr(&out).contains("capability denied"),
        "the identical caps fault: {:?}",
        stderr(&out)
    );
    let page = std::fs::read_to_string(guarded.join("task.md")).expect("task page");
    assert!(
        page.contains("status: open"),
        "nothing was applied: {page:?}"
    );
}

// ── realise ──────────────────────────────────────────────────────────────────

#[test]
fn realise_checks_the_claim_in_the_named_root() {
    let sb = sandbox();
    let out = sb.run_degraded(&sb.ws, &["realise", "sessions:claim.md"]);
    assert_eq!(
        code(&out),
        0,
        "the rooted claim converges in the named root: {} {}",
        stdout(&out),
        stderr(&out)
    );
}

#[test]
fn realise_unbound_root_refuses() {
    let sb = sandbox();
    let out = sb.run_degraded(&sb.ws, &["realise", "sessionz:claim.md"]);
    assert_eq!(code(&out), 1);
    assert_unbound_refusal(&out, "realise");
}

// ── links ────────────────────────────────────────────────────────────────────

#[test]
fn links_serves_the_named_roots_edge_map() {
    let sb = sandbox();
    let out = sb.run_degraded(&sb.ws, &["links", "sessions:notes.md", "--json"]);
    assert_eq!(code(&out), 0, "rooted links serves: {}", stderr(&out));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("frame parses");
    let canonical = std::fs::canonicalize(&sb.sessions).expect("canonical sessions");
    assert_eq!(
        v["workspace"],
        serde_json::json!(canonical.display().to_string()),
        "the frame's workspace is the named root's bound path: {v}"
    );
}

#[test]
fn links_unbound_root_refuses() {
    let sb = sandbox();
    let out = sb.run_degraded(&sb.ws, &["links", "sessionz:notes.md"]);
    assert_eq!(code(&out), 1);
    assert_unbound_refusal(&out, "links");
}

// ── rules ────────────────────────────────────────────────────────────────────

#[test]
fn rules_answers_at_the_named_root() {
    let sb = sandbox();
    let out = sb.run_degraded(&sb.ws, &["rules", "sessions:notes.md"]);
    assert_eq!(
        code(&out),
        0,
        "a rooted rules query answers (empty set is clean): {}",
        stderr(&out)
    );
}

#[test]
fn rules_unbound_root_refuses() {
    let sb = sandbox();
    let out = sb.run_degraded(&sb.ws, &["rules", "sessionz:notes.md"]);
    assert_eq!(code(&out), 1);
    assert_unbound_refusal(&out, "rules");
}

// ── the preset lane: refuse-with-teaching (not converted YET) ────────────────

#[test]
fn preset_doors_refuse_rooted_refs_with_the_armed_gate_teaching() {
    let sb = sandbox();
    for argv in [
        vec!["unfold", "sessions:presets/x.md"],
        vec!["reconcile", "sessions:presets/x.md"],
        vec!["new", "sessions:defs/x.md", "id-1"],
    ] {
        let out = sb.run_degraded(&sb.ws, &argv);
        assert_eq!(
            code(&out),
            2,
            "{}: the preset lane refuses a rooted ref pre-read",
            argv[0]
        );
        let err = stderr(&out);
        assert!(
            err.contains("does not serve the rooted lane yet") && err.contains("armed gates"),
            "{}: the refusal teaches the reason and the future route: {err:?}",
            argv[0]
        );
    }
}

// ── sql --root ───────────────────────────────────────────────────────────────

#[test]
fn sql_root_selects_the_named_projection_workspace() {
    let sb = sandbox();
    let out = sb.run_degraded(&sb.ws, &["sql", "select 1 as one", "--root", "sessions"]);
    assert_eq!(
        code(&out),
        0,
        "--root sessions serves the projection: {}",
        stderr(&out)
    );
}

#[test]
fn sql_unbound_root_refuses_naming_the_bound_roots() {
    let sb = sandbox();
    let out = sb.run_degraded(&sb.ws, &["sql", "select 1", "--root", "sessionz"]);
    assert_eq!(code(&out), 2, "an unbound --root is the door's own refusal");
    let err = stderr(&out);
    assert!(
        err.contains("does not bind") && err.contains("bound roots:"),
        "the refusal enumerates what binds: {err:?}"
    );
}

#[test]
fn sql_root_and_cwd_are_mutually_exclusive() {
    let sb = sandbox();
    let out = sb.run_degraded(
        &sb.ws,
        &["sql", "select 1", "--root", "sessions", "--cwd", "."],
    );
    assert_eq!(code(&out), 2);
    assert!(
        stderr(&out).contains("both select the projection workspace"),
        "the wall teaches: {}",
        stderr(&out)
    );
}

// ── script --files: the one-root law (pre-dial refusals) ────────────────────

#[test]
fn script_files_across_two_roots_refuse_the_one_root_law() {
    let sb = sandbox();
    let out = sb.run_warm_stdin(
        &sb.ws,
        &[
            "script",
            "--files",
            "sessions:notes.md",
            "--files",
            "assets:notes.md",
        ],
        "x = 1\n",
    );
    assert_eq!(code(&out), 1, "mixed roots refuse: {}", stderr(&out));
    assert!(
        stderr(&out).contains("one root"),
        "the refusal cites the one-root law: {}",
        stderr(&out)
    );
}

#[test]
fn script_bare_member_beside_a_foreign_rooted_one_refuses() {
    let sb = sandbox();
    let out = sb.run_warm_stdin(
        &sb.ws,
        &[
            "script",
            "--files",
            "notes.md",
            "--files",
            "sessions:notes.md",
        ],
        "x = 1\n",
    );
    assert_eq!(code(&out), 1, "mixed trees refuse: {}", stderr(&out));
    assert!(
        stderr(&out).contains("one root"),
        "the refusal cites the one-root law: {}",
        stderr(&out)
    );
}

// ── the write doors (daemon-backed) ──────────────────────────────────────────

#[test]
fn put_writes_into_the_named_root_never_the_decoy() {
    let sb = sandbox();
    let edits =
        r#"[{"target":{"fm_key":"verdict"},"edit":{"put":{"at":"upsert","text":"approve"}}}]"#;
    let out = sb.run_warm_stdin(&sb.ws, &["put", "sessions:notes.md", "--force"], edits);
    sb.reap_daemon();
    assert_eq!(code(&out), 0, "the rooted put commits: {}", stderr(&out));
    assert!(
        stdout(&out).contains("committed sessions:notes.md"),
        "the face echoes the caller's rooted spelling: {}",
        stdout(&out)
    );
    let target = std::fs::read_to_string(sb.sessions.join("notes.md")).expect("target");
    assert!(
        target.contains("verdict: approve"),
        "the write landed in the NAMED root: {target:?}"
    );
    let decoy = std::fs::read_to_string(sb.ws.join("notes.md")).expect("decoy");
    assert_eq!(decoy, DECOY, "the ambient decoy is byte-untouched");
}

#[test]
fn put_unbound_root_refuses_and_writes_nothing() {
    let sb = sandbox();
    std::fs::write(sb.ws.join("sessionz:notes.md"), DECOY).expect("literal trap");
    let edits =
        r#"[{"target":{"fm_key":"verdict"},"edit":{"put":{"at":"upsert","text":"approve"}}}]"#;
    let out = sb.run_warm_stdin(&sb.ws, &["put", "sessionz:notes.md", "--force"], edits);
    sb.reap_daemon();
    assert_eq!(code(&out), 1, "an unbound root is an address refusal");
    assert_unbound_refusal(&out, "put");
    let trap = std::fs::read_to_string(sb.ws.join("sessionz:notes.md")).expect("trap");
    assert_eq!(trap, DECOY, "the literal file is byte-untouched");
}

#[test]
fn rm_consults_the_named_root_not_the_ambient_twin() {
    let sb = sandbox();
    // `ghost.md` exists ONLY ambient: a rooted rm must answer for the NAMED
    // root (file_not_found there), never remove the ambient twin.
    std::fs::write(sb.ws.join("ghost.md"), DECOY).expect("ambient twin");
    let out = sb.run_warm(
        &sb.ws,
        &["rm", "sessions:ghost.md", "--rev", "0000000000000000"],
    );
    sb.reap_daemon();
    assert_eq!(code(&out), 1, "the engine answers for the named root");
    let err = stderr(&out);
    assert!(
        err.contains("file_not_found") || err.contains("not found") || err.contains("no file"),
        "the refusal is the TARGET tree's absence, not an address fault: {err:?}"
    );
    assert!(
        sb.ws.join("ghost.md").is_file(),
        "the ambient twin survives"
    );
}

#[test]
fn pin_writes_the_lock_into_the_named_roots_page() {
    let sb = sandbox();
    std::fs::write(sb.sessions.join("holder.md"), "# Holder\n\nbody.\n").expect("holder");
    // An R4 pin needs a git blob oid for its target, so the NAMED root must
    // be a git work tree with the target committed.
    for argv in [
        vec!["init", "-q"],
        vec!["add", "."],
        vec![
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-qm",
            "seed",
        ],
    ] {
        let ok = Command::new("git")
            .args(&argv)
            .current_dir(&sb.sessions)
            .status()
            .expect("git")
            .success();
        assert!(ok, "git {argv:?} seeds the sessions repo");
    }
    let out = sb.run_warm(
        &sb.ws,
        &["pin", "sessions:holder.md", "notes.md#Notes/Design"],
    );
    sb.reap_daemon();
    assert_eq!(code(&out), 0, "the rooted pin commits: {}", stderr(&out));
    let holder = std::fs::read_to_string(sb.sessions.join("holder.md")).expect("holder");
    assert!(
        holder.contains("meridian-lock"),
        "the lock landed in the NAMED root's page: {holder:?}"
    );
    assert!(
        !std::fs::read_to_string(sb.ws.join("notes.md"))
            .expect("decoy")
            .contains("meridian-lock"),
        "the ambient decoy gains no lock"
    );
}
