//! **`mrd script` needs a daemon, and `--help` now promises exactly how** (card
//! `script-door-commit-premise-world-grain-vs-touch-set`).
//!
//! The sentence the `--help` block carries:
//!
//! > NEEDS A DAEMON: this door writes AS you through the one socket, so there is
//! > no daemonless leg. With none running it AUTO-SPAWNS one and waits for it to
//! > bind; if that never happens it refuses by name and nothing is evaluated.
//!
//! It is a promise about behavior a caller cannot see any other way, and it went
//! into `--help` in the PR that made this door single-lane. A promise in a help
//! text with no test is a claim about a running system that nobody checked, so
//! both halves are driven here through the real binary, over its process
//! boundary — the only place an auto-spawn is observable at all.
//!
//! **Why it belongs to this card.** The precondition the advisor and the leader
//! both demanded before the local transaction could be deleted was: *name the
//! state the local lane existed for, and show it survives*. The answer was that
//! it never served a daemonless case — `engine::ensure_daemon` and the socket
//! dial both sit ABOVE the lane split (`script/cmd.rs`), so every lane this verb
//! ever had already required a daemon. These tests are the evidence for that
//! answer, not a new feature's coverage.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

mod common;

/// The binary every drive here goes through. `MRD_BIN` names another artifact —
/// the fixv convention (`crates/mrd/tests/s2fix_cross_surface.rs`), reused here
/// so the SAME asserts can run against a pre-change build.
fn mrd_bin() -> PathBuf {
    std::env::var_os("MRD_BIN")
        .map_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_mrd")), PathBuf::from)
}

/// A read-class program: it arms nothing, so an auto-spawned daemon can serve it
/// without the test caring what landed. What is under test is whether the door
/// reached a daemon at all.
const READ_ONLY: &str = "n = len(files)\n";

struct Sandbox {
    tmp: tempfile::TempDir,
    cache_home: PathBuf,
    home: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cache_home = tmp.path().join("xdg-cache");
        let home = tmp.path().join("home");
        std::fs::create_dir_all(&home).expect("home");
        Self {
            tmp,
            cache_home,
            home,
        }
    }

    fn command(&self, cwd: &Path, args: &[&str]) -> Command {
        let mut command = Command::new(mrd_bin());
        command
            .args(args)
            .current_dir(cwd)
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
            .env_remove("MERIDIAN_WORKSPACE");
        command
    }

    fn run(&self, cwd: &Path, args: &[&str]) -> Output {
        self.command(cwd, args).output().expect("spawn mrd")
    }

    /// `mrd script`, with `source` fed on stdin the way the verb takes it.
    fn script(&self, cwd: &Path, flags: &[&str], source: &str, daemon_bin: Option<&str>) -> Output {
        let mut invocation = vec!["script"];
        invocation.extend_from_slice(flags);
        let mut command = self.command(cwd, &invocation);
        if let Some(bin) = daemon_bin {
            command.env("MERIDIAN_DAEMON_BIN", bin);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn mrd script");
        common::feed_stdin(&mut child, source.as_bytes());
        child.wait_with_output().expect("mrd script answers")
    }

    /// A workspace with one page, declared a root by `mrd init`.
    fn workspace(&self) -> PathBuf {
        let ws = self.tmp.path().join("project");
        std::fs::create_dir_all(&ws).expect("mkdir");
        std::fs::write(ws.join("a.md"), "# A\n\nalpha\n").expect("a");
        let init = self.run(&ws, &["init"]);
        assert!(
            init.status.success(),
            "init: {}",
            String::from_utf8_lossy(&init.stderr)
        );
        ws
    }
}

/// The listing with every whitespace run collapsed, so an assertion about a
/// SENTENCE is not an assertion about where the column wrapped it.
fn help_prose() -> String {
    let out = Command::new(mrd_bin())
        .arg("--help")
        .output()
        .expect("run mrd --help");
    assert!(out.status.success(), "mrd --help is a success");
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// **The promise itself, and the sentence it replaced.**
///
/// A help text is a claim about a running system, and this one changed in both
/// directions with this card: it gained the daemon promise the two tests below
/// measure, and it LOST the world-grain law — *"Entry pins one fingerprint;
/// commit guards on it — world moved ⇒ refuse, nothing lands"* — which was the
/// false sentence the card exists to correct. Asserting the absence matters as
/// much as asserting the presence: a reader who acts on the deleted sentence
/// builds retry loops around corpus churn that no longer refuses them.
#[test]
fn the_help_promises_the_touch_set_and_the_auto_spawn_and_no_longer_the_moved_world() {
    let help = help_prose();

    assert!(
        help.contains("THE COMMIT'S AUTHORITY IS THE TOUCH SET"),
        "the law the engine now honours is the law the help states: {help}"
    );
    assert!(
        help.contains("A foreign write OUTSIDE that set does NOT refuse"),
        "and it states the half that changed behaviour: {help}"
    );
    assert!(
        !help.contains("Entry pins one fingerprint"),
        "the deleted world-grain sentence must not survive its own law: {help}"
    );

    assert!(
        help.contains("NEEDS A DAEMON"),
        "the promise the two tests below measure: {help}"
    );
    assert!(
        help.contains("with none running it auto-spawns one and waits for it to bind"),
        "half one, in the words the help uses: {help}"
    );
    assert!(
        help.contains("if that never happens it refuses by name and nothing is evaluated"),
        "half two, in the words the help uses: {help}"
    );
}

/// **Half one: with no daemon running, the door AUTO-SPAWNS one and serves.**
///
/// The sandbox is a fresh `XDG_CACHE_HOME`/`HOME`, so the derived socket has
/// nothing behind it when the verb starts. Success is measured three ways, none
/// of which a lucky exit code alone would satisfy: the run exits clean, it emits
/// a trace carrying the DAEMON's entry fingerprint (a value this process cannot
/// mint — its presence is the proof a daemon answered), and a pidfile now sits
/// beside the socket naming the process that was spawned to answer it.
#[test]
fn with_no_daemon_running_the_door_spawns_one_and_serves() {
    let sandbox = Sandbox::new();
    let ws = sandbox.workspace();
    // The sandbox owns the daemon it causes to exist (card
    // e2e-daemon-leak-fixture): reaped on drop, never left parked for the
    // 15-minute idle exit.
    let _reaper = common::DaemonReaper {
        home: sandbox.home.clone(),
        cache_home: sandbox.cache_home.clone(),
    };

    let pidfile = common::child_daemon_pidfile(&sandbox.home, &sandbox.cache_home);
    assert!(
        !pidfile.exists(),
        "the premise: nothing is running yet ({})",
        pidfile.display()
    );

    let out = sandbox.script(&ws, &["--json"], READ_ONLY, None);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        out.status.success(),
        "a run that auto-spawned its daemon exits clean.\nstdout: {stdout}\nstderr: {stderr}"
    );
    let trace: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("a trace on stdout ({e}): {stdout}"));
    assert_eq!(
        trace["outcome"], "no_effect",
        "a read-class program lands nothing: {trace}"
    );
    let entry = trace["entry_fingerprint"]
        .as_str()
        .unwrap_or_else(|| panic!("every trace opens with the entry premise: {trace}"));
    assert!(
        !entry.is_empty(),
        "the entry fingerprint is the DAEMON's — this lane mints none, so a \
         non-empty one is the proof a daemon answered: {trace}"
    );
    assert!(
        pidfile.exists(),
        "and the daemon it spawned wrote its pidfile beside the socket ({}). \
         stderr: {stderr}",
        pidfile.display()
    );
}

/// **Half two: when the spawn cannot happen, it refuses BY NAME and evaluates
/// nothing.**
///
/// `MERIDIAN_DAEMON_BIN` points the auto-spawn at a path that does not exist, so
/// `daemon::spawn_detached` fails at `Command::spawn` and `ensure_daemon` errors
/// immediately — the same terminal the 5-second bind timeout reaches, without
/// spending five seconds to get there.
///
/// The assertions are on the TEXT, because the text is what `--help` promises: a
/// refusal that names the daemon and states that nothing was evaluated. A silent
/// nonzero exit would satisfy "it did not write" while telling an operator
/// nothing, and a daemonless in-process write would satisfy neither — it would
/// land bytes actor-absent, which is the reason this door has no degrade leg at
/// all (`run-plane.md` § the seam table, "wire-client mode").
#[test]
fn when_the_spawn_cannot_happen_the_door_refuses_by_name_and_writes_nothing() {
    let sandbox = Sandbox::new();
    let ws = sandbox.workspace();
    let missing = sandbox.tmp.path().join("no-such-daemon-binary");
    assert!(!missing.exists(), "the premise: there is nothing to spawn");

    let out = sandbox.script(
        &ws,
        &["--json"],
        READ_ONLY,
        Some(missing.to_str().expect("utf-8 path")),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        !out.status.success(),
        "a door that could not reach a daemon must not exit clean.\nstdout: \
         {stdout}\nstderr: {stderr}"
    );
    assert!(
        stderr.contains("no daemon answered"),
        "it refuses BY NAME — an operator reads which door failed: {stderr}"
    );
    assert!(
        stderr.contains("no daemonless leg"),
        "and it states the design rather than looking like a transient: this \
         door writes AS you through the one socket, so there is nothing to \
         fall back to: {stderr}"
    );
    assert!(
        stdout.trim().is_empty(),
        "nothing was evaluated, so there is no trace to print: {stdout}"
    );
    // And the workspace is untouched — the refusal happened before any lane.
    let page = std::fs::read_to_string(ws.join("a.md")).expect("the page reads back");
    assert_eq!(page, "# A\n\nalpha\n", "nothing was written: {page}");
}
