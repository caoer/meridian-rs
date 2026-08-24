//! **An auto-spawned daemon that dies at startup says why** (card
//! `auto-spawned-daemon-dies-silently`).
//!
//! The defect these gates close, measured on main `40fad579b`: the client
//! launched the daemon with `stderr(Stdio::null())`, so anything it printed or
//! panicked with went to `/dev/null`; the client then polled for
//! `SPAWN_READY_TIMEOUT` (5 s) and degraded to the in-process ephemeral engine,
//! which never refuses. Net: a panic, a layout that will not resolve, an
//! unbindable socket and a poisoned state file ALL presented as "5 seconds
//! slower, then the ephemeral answer", to the user and to the test suite alike.
//!
//! The degrade is right as a policy and is untouched here — every gate below
//! asserts the run still succeeds. What changed is that the degrade can now SAY
//! why: the daemon's stderr is a file beside its own socket (`daemon::voice`),
//! and the client reads back exactly the region that child appended
//! (`daemon::voice_since`, bounded by a mark taken before the spawn) and quotes
//! it through `engine::degrade_reason` — the one seam all five daemon-path
//! lanes already consult.
//!
//! **Why the injected failure is an occupied socket path.** It kills the REAL
//! `mrd daemon` binary inside `RunningServer::start`, which is the shape the
//! card names, and it is invisible to every pre-flight the client runs: the
//! client's own `Config::resolve` succeeds, so nothing here is proved by the
//! client-side `debug_assert` in `engine::ensure_daemon`. A fake binary in
//! `MERIDIAN_DAEMON_BIN` would prove only that a pipe carries bytes.
//!
//! Fixture law (card `e2e-daemon-leak-fixture`): each sandbox reaps its own
//! daemon on drop. These two never get one up, so the reap is a no-op — it
//! stays because a fixture that CAN auto-spawn owns the teardown either way.

use std::path::{Path, PathBuf};
use std::process::Output;

mod common;

const DOC: &str = "# Alpha\n\nsee [[beta]]\n";

struct Sandbox {
    tmp: tempfile::TempDir,
    home: PathBuf,
    cache_home: PathBuf,
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        common::reap_daemon(&self.home, &self.cache_home);
    }
}

impl Sandbox {
    fn new() -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let home = tmp.path().join("home");
        let cache_home = tmp.path().join("xdg-cache");
        std::fs::create_dir_all(&home).expect("home");
        Self {
            tmp,
            home,
            cache_home,
        }
    }

    /// An anchored workspace with one doc — enough for `links` to have an
    /// answer, so a degraded run is visibly a COMPLETE run.
    fn workspace(&self) -> PathBuf {
        let ws = self.tmp.path().join("project");
        std::fs::create_dir_all(ws.join(".git")).expect("git anchor");
        std::fs::write(ws.join("a.md"), DOC).expect("doc");
        std::fs::canonicalize(&ws).expect("canonical ws")
    }

    /// Where the daemon this sandbox spawns speaks: `<socket-stem>.log`, the
    /// stem the socket and the pidfile already key off.
    fn lane(&self) -> PathBuf {
        common::child_socket_path(&self.home, &self.cache_home).with_extension("log")
    }

    /// `mrd links --json` in `ws`, with the timing mode explicitly OFF — the
    /// lane under test is the daemon's own, not the instrument's.
    fn links(&self, ws: &Path, daemon_bin: Option<&Path>) -> Output {
        let mut cmd = common::mrd_command(&self.home, &self.cache_home);
        cmd.args(["links", "--json"])
            .env_remove("MRD_TIMING")
            .env_remove("MERIDIAN_WORKSPACE")
            .current_dir(ws);
        if let Some(bin) = daemon_bin {
            cmd.env("MERIDIAN_DAEMON_BIN", bin);
        }
        cmd.output().expect("mrd links runs")
    }
}

fn stderr_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// The whole point: a daemon that dies inside `RunningServer::start` leaves a
/// readable trace naming the cause, and the degraded run QUOTES it.
///
/// A directory at the socket path is the injection. The daemon cannot bind
/// there, so `RunningServer::start` returns and `mrd` prints
/// `mrd: cannot start the registry daemon: …` — to what used to be `/dev/null`
/// and is now its lane. The client waits out the 5 s, degrades, and says what
/// the child said.
///
/// Note what is NOT asserted: the exact OS error. That text is the platform's
/// (`EADDRINUSE` reads differently across libcs) and pinning it would make this
/// a test of the C library. The assertion is that the daemon's OWN refusal —
/// the sentence `mrd` itself mints — survives the trip from a detached child's
/// stderr to the client's.
#[test]
fn a_daemon_that_dies_at_startup_is_quoted_by_the_degrade() {
    let sb = Sandbox::new();
    let ws = sb.workspace();

    // Occupy the socket path with something that cannot be bound. The lane is
    // `<stem>.log`, a sibling, so it is untouched by this.
    let socket = common::child_socket_path(&sb.home, &sb.cache_home);
    std::fs::create_dir_all(&socket).expect("a directory where the socket wants a socket");

    let out = sb.links(&ws, None);
    let said = stderr_of(&out);

    // 1. The degrade policy is intact: a run never fails for want of a daemon.
    assert_eq!(
        out.status.code(),
        Some(0),
        "the degrade must never fail the run: {said}"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let answer: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("`links --json` did not serve JSON ({e}): {stdout}"));
    assert_eq!(
        answer["source"].as_str(),
        Some("ephemeral"),
        "a daemon that never bound must be degraded past: {stdout}"
    );

    // 2. The daemon's dying words are ON DISK, on the lane beside its socket.
    let lane = sb.lane();
    let trace = std::fs::read_to_string(&lane).unwrap_or_default();
    assert!(
        trace.contains("cannot start the registry daemon"),
        "the daemon's lane {} does not name the cause.\nlane: {trace}\nclient stderr: {said}",
        lane.display()
    );

    // 3. And the client QUOTED them — the half that makes the failure visible
    //    to somebody who only ever sees the client subprocess.
    assert!(
        said.contains("never bound its socket"),
        "the degrade did not say the daemon failed to come up: {said}"
    );
    assert!(
        said.contains("cannot start the registry daemon"),
        "the degrade named no cause — this is the silence the card is about, \
         with an extra sentence in front of it.\nlane: {trace}\nclient stderr: {said}"
    );
    assert!(
        said.contains(&lane.display().to_string()),
        "the degrade quoted the lane without naming it, so a reader cannot go \
         and read the rest: {said}"
    );

    // 4. One line. The quote carries an `io::Error` whose text is the OS's, and
    //    a raw newline in it would split the diagnostic and leave the tail
    //    unprefixed (`daemon::one_line` folds it).
    let quotes: Vec<&str> = said
        .lines()
        .filter(|line| line.contains("carries its last words:"))
        .collect();
    assert_eq!(
        quotes.len(),
        1,
        "expected exactly one quote line, got {quotes:?}\nclient stderr: {said}"
    );
}

/// The sibling failure, which is NOT the same one: the daemon was never
/// launched at all. There are no dying words to quote — the child does not
/// exist — so the degrade names that instead of quoting an empty lane.
///
/// This is the pre-existing `MERIDIAN_DAEMON_BIN` degrade
/// (`e2e_links_spawn_impossible_degrades_and_answers_correctly`); what is new
/// is that it, too, now says why rather than degrading mute.
#[test]
fn a_daemon_that_could_not_be_launched_says_that_instead() {
    let sb = Sandbox::new();
    let ws = sb.workspace();
    let missing = sb.tmp.path().join("no-such-daemon-binary");
    assert!(!missing.exists(), "the premise: there is nothing to spawn");

    let out = sb.links(&ws, Some(&missing));
    let said = stderr_of(&out);

    assert_eq!(
        out.status.code(),
        Some(0),
        "the degrade must never fail the run: {said}"
    );
    assert!(
        said.contains("could not be launched at all"),
        "a spawn that never happened must not report itself as a daemon that \
         went quiet: {said}"
    );
    assert!(
        !said.contains("carries its last words"),
        "there was no child, so there is nothing it could have said: {said}"
    );
}
