//! The DAEMON lane of the `MRD_TIMING` mode (`docs/status.md` § The timing
//! mode), driven through a real auto-spawned daemon — the only process that can
//! prove it, because the defect was in how that process is spawned.
//!
//! Two claims, both from PR #176's round-2 review:
//!
//! - **A refused sink is audible.** The file sink exists for one caller, the
//!   resident daemon, and that caller was spawned with `Stdio::null()` — so the
//!   corpus-extension refusal and the failed-open degrade, which both speak on
//!   stderr, reached nobody. An operator who pointed a daemon at `ws/x.md` got
//!   no complaint AND no measurements: the "the code never ran there" answer
//!   the instrument must never fake. While the mode is on the daemon now speaks
//!   into `<socket-stem>.log` beside its own socket and pidfile.
//! - **Concurrent work is attributable.** Every line carries `who=p<pid>.t<n>`,
//!   so daemon lines that used to be byte-identical now name their emitter.
//!   What that emitter IS on this lane is narrower than it first looks: the
//!   cold gate runs the fold on a per-workspace `drawer-rebuild` thread, not on
//!   the connection thread, so `t` separates concurrent COLD FOLDS — one per
//!   workspace — and two ops on one workspace share its single fold
//!   (`docs/status.md` § What `t` names here, and what it does not).
//!
//! Harness note: these tests do NOT set `MERIDIAN_DAEMON_BIN` — the point is the
//! real auto-spawn, which launches `current_exe()`, i.e. the `mrd` under test.
//! Each sandbox reaps its own daemon on drop (`common::reap_daemon`), the
//! fixture law from card `e2e-daemon-leak-fixture`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

mod common;

fn mrd_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mrd")
}

/// A one-read starlark program: enough to reach the daemon and be served.
const PROGRAM: &str = "t = read(\"doc.md\")\n";

const DOC: &str = "# Alpha\n\none two three\n";

/// How long [`wait_for`] gives the daemon to write what a test is waiting for.
/// The daemon binds within `SPAWN_READY_TIMEOUT` (5 s) or the client degrades;
/// this only bounds a machine slower than that, and a failure prints what the
/// file did hold.
const SETTLE: Duration = Duration::from_secs(10);

struct Sandbox {
    tmp: tempfile::TempDir,
    home: PathBuf,
    cache_home: PathBuf,
}

impl Drop for Sandbox {
    /// The daemon this sandbox auto-spawned is detached (`setsid`) and would
    /// otherwise park for `registry::DEFAULT_IDLE_EXIT`.
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

    /// An anchored workspace holding the fixture doc.
    fn workspace(&self) -> PathBuf {
        self.workspace_named("project")
    }

    /// A SECOND (third, …) workspace under the same home and cache root, so the
    /// one resident daemon has to bind and fold each of them separately.
    ///
    /// This is what makes the concurrency gate deterministic: a repeat call on
    /// an already-warm workspace can be served with no fold at all, and so with
    /// no phase to emit — one workspace per caller guarantees each caller's
    /// workspace gets its own fold, so there is a `who=` to attribute.
    fn workspace_named(&self, name: &str) -> PathBuf {
        let ws = self.tmp.path().join(name);
        std::fs::create_dir_all(ws.join(".git")).expect("git anchor");
        std::fs::write(ws.join("doc.md"), DOC).expect("doc");
        std::fs::canonicalize(&ws).expect("canonical ws")
    }

    /// Where the daemon this sandbox spawns speaks while the mode is on —
    /// `<socket-stem>.log`, the same stem the socket and the pidfile key off.
    fn voice(&self) -> PathBuf {
        common::child_socket_path(&self.home, &self.cache_home).with_extension("log")
    }

    /// `mrd script --json` against `ws`, with `MRD_TIMING` set to `timing`.
    /// Spawned rather than `output()`ed so two of them can be in flight at once.
    fn script(&self, ws: &Path, timing: &str) -> Child {
        let mut child = Command::new(mrd_bin())
            .args(["script", "--json"])
            .env("HOME", &self.home)
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("MRD_TIMING", timing)
            .env_remove("MERIDIAN_WORKSPACE")
            .current_dir(ws)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn mrd script");
        common::feed_stdin(&mut child, PROGRAM.as_bytes());
        child
    }

    fn script_now(&self, ws: &Path, timing: &str) -> Output {
        self.script(ws, timing)
            .wait_with_output()
            .expect("mrd script exits")
    }

    /// `mrd links --json` against `ws` — the op these gates drive, because a
    /// fresh workspace makes the daemon FOLD it, which is the only thing the
    /// daemon emits phases for.
    ///
    /// Measured on workstation-nyc-2, 2026-08-23: three `links` calls on three
    /// fresh workspaces produced three complete folds (`who=…t1/t2/t3`), while
    /// three `script` calls on three fresh workspaces produced exactly ONE.
    /// The `script` asymmetry is UNEXPLAINED, and this note keeps only the
    /// measurement.
    ///
    /// **Its stated cause is void.** The sentence here read "a pattern-less
    /// `mrd script` evaluates CLIENT-SIDE (`script/cmd.rs`), and the wire
    /// `read` its program issues takes `cold_gate_wire` exactly as `links`
    /// does". Card `script-door-commit-premise-world-grain-vs-touch-set`
    /// deleted that lane: EVERY `mrd script` attempt is now the wire `script`
    /// op, which takes `cold_gate_wire` once, in
    /// `registry::script_op::serve`, and issues no client-side `read` at all.
    /// So the reasoning that made the asymmetry surprising no longer describes
    /// the code, and whether the asymmetry itself survives is UNMEASURED — the
    /// measurement above predates the change. These gates drive `links`, so
    /// nothing here depends on the answer.
    fn links(&self, ws: &Path, timing: &str) -> Child {
        Command::new(mrd_bin())
            .args(["links", "--json"])
            .env("HOME", &self.home)
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("MRD_TIMING", timing)
            .env_remove("MERIDIAN_WORKSPACE")
            .current_dir(ws)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn mrd links")
    }
}

/// One parsed MEASUREMENT. This lane's question is ATTRIBUTION, so `us=` is
/// parsed — the grammar's fourth field must be an integer or the line is
/// malformed — and then dropped: no assertion here is about a duration.
///
/// A line that opens `mrd-timing ` and does not parse is a failure, not a skip:
/// the grammar is the contract.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Measurement {
    cmd: String,
    who: String,
    phase: String,
}

fn measurements(raw: &str) -> Vec<Measurement> {
    raw.lines()
        .filter(|line| line.starts_with("mrd-timing "))
        .map(|line| {
            let mut fields = line.split(' ');
            assert_eq!(fields.next(), Some("mrd-timing"), "{line}");
            let mut field = |key: &str| {
                fields
                    .next()
                    .and_then(|f| f.strip_prefix(key))
                    .unwrap_or_else(|| panic!("no {key} field: {line}"))
                    .to_owned()
            };
            let (cmd, who, phase) = (field("cmd="), field("who="), field("phase="));
            field("us=")
                .parse::<u128>()
                .unwrap_or_else(|e| panic!("us= is not an integer ({e}): {line}"));
            assert!(fields.next().is_none(), "extra field: {line}");
            Measurement { cmd, who, phase }
        })
        .collect()
}

/// The mode's own diagnostics — the COLON shape, which is what a refusal takes.
fn diagnostics(raw: &str) -> Vec<String> {
    raw.lines()
        .filter(|line| line.starts_with("mrd-timing:"))
        .map(str::to_owned)
        .collect()
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

/// Poll `path` until `done` accepts its contents, or [`SETTLE`] elapses.
/// Returns the last contents read either way — a failing assertion then prints
/// what the daemon DID say, which is the whole diagnostic value of this file.
fn wait_for(path: &Path, done: impl Fn(&str) -> bool) -> String {
    let deadline = Instant::now() + SETTLE;
    loop {
        let text = read(path);
        if done(&text) || Instant::now() >= deadline {
            return text;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The `p` half of a `who=` — the process. Lines from one daemon share it.
fn pid_of(who: &str) -> &str {
    who.split_once(".t")
        .unwrap_or_else(|| panic!("who is not p<pid>.t<n>: {who}"))
        .0
}

/// A sink the daemon cannot use is REFUSED — and the refusal is now on a lane
/// somebody reads. `ws/timing.md` is the exact hazard: a corpus extension,
/// inside the workspace, so the fold would chase its own tail.
///
/// The measurements land there too. A degrade that kept the complaint and lost
/// the numbers would be a different silence, not a fix.
#[test]
fn a_refused_daemon_sink_is_audible_in_the_daemons_own_lane() {
    let sb = Sandbox::new();
    let ws = sb.workspace();
    let refused = ws.join("timing.md");

    let out = sb.script_now(&ws, refused.to_str().expect("utf-8 sink"));
    assert!(
        !refused.exists(),
        "the corpus-extension sink was opened after all"
    );

    let voice = sb.voice();
    let said = wait_for(&voice, |text| {
        !diagnostics(text).is_empty() && measurements(text).iter().any(|m| m.cmd == "daemon")
    });

    let complaints = diagnostics(&said);
    assert!(
        complaints
            .iter()
            .any(|line| line.contains("corpus extension")),
        "the daemon's lane {} carries no refusal.\nlane: {said}\nclient stderr: {}",
        voice.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    let lines = measurements(&said);
    assert!(
        lines.iter().any(|m| m.cmd == "daemon"),
        "the refusal kept the complaint and lost the measurements: {said}"
    );

    // The client is a second process with the same bad sink: it complains on
    // its OWN stderr, which it has. Two lanes, one refusal each.
    let client = String::from_utf8_lossy(&out.stderr);
    assert!(
        diagnostics(&client)
            .iter()
            .any(|line| line.contains("corpus extension")),
        "the client swallowed its own refusal: {client}"
    );
}

/// `MRD_TIMING=1` — the stderr word — measured nothing at all on a detached
/// daemon, because its stderr was `/dev/null`. It is the same silence as a
/// refused sink and it closes the same way.
#[test]
fn the_stderr_word_reaches_the_daemons_lane_too() {
    let sb = Sandbox::new();
    let ws = sb.workspace();

    let out = sb.script_now(&ws, "1");
    let said = wait_for(&sb.voice(), |text| {
        measurements(text).iter().any(|m| m.cmd == "daemon")
    });

    assert!(
        measurements(&said).iter().any(|m| m.cmd == "daemon"),
        "the daemon measured into the void under MRD_TIMING=1.\nlane: {said}\nclient stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The OTHER half of "a lane that cannot be opened is heard": the sink the
/// operator named is fine, but the daemon's own lane will not open. Nothing
/// server-side can report that — the daemon has no stderr yet, which is the
/// whole problem — so it is said on the SPAWNING CLIENT's stderr, a lane
/// somebody hears, and the daemon starts mute rather than pretending.
///
/// A DIRECTORY at `<socket-stem>.log` is the cleanest way to fail exactly that
/// open: `EISDIR` on the log, and the socket bind beside it is untouched, so
/// the daemon still comes up and answers. A degrade that also broke the daemon
/// would be a worse bug than the silence it replaced.
#[test]
fn an_unopenable_voice_is_said_on_the_spawning_clients_stderr() {
    let sb = Sandbox::new();
    let ws = sb.workspace();

    // Occupy the lane's path with something that cannot be opened for append.
    let voice = sb.voice();
    std::fs::create_dir_all(&voice).expect("a directory where the lane wants a file");

    let out = sb
        .links(&ws, "1")
        .wait_with_output()
        .expect("mrd links exits");
    let client = String::from_utf8_lossy(&out.stderr).into_owned();

    let said = diagnostics(&client);
    assert!(
        said.iter()
            .any(|line| line.contains("cannot open the daemon's lane")),
        "the client swallowed the unopenable lane: {client}"
    );
    // The `io::Error` text is the OS's, not ours, and it rides this line: it
    // must not have split it. `diagnostics()` splits on newlines already, so
    // "no `\n` in the element" is vacuous — the real assertion is that the ONE
    // matching line still carries the END of the message.
    let complaint: Vec<&String> = said
        .iter()
        .filter(|line| line.contains("cannot open the daemon's lane"))
        .collect();
    assert_eq!(complaint.len(), 1, "expected exactly one: {said:?}");
    assert!(
        complaint[0].trim_end().ends_with("are LOST."),
        "the complaint line does not end with `are LOST.` — split by the error text, or the tail \
         in daemon.rs changed: {}",
        complaint[0]
    );

    // And the daemon still came up. Exit 0 alone would NOT show that: `links`
    // degrades to the in-process engine on any daemon-path failure
    // (`engine.rs` § `try_daemon_links`), so a dead daemon also exits 0. The
    // pidfile is written after the bind.
    assert_eq!(
        out.status.code(),
        Some(0),
        "the verb must not fail: {client}"
    );
    assert!(
        common::child_daemon_pidfile(&sb.home, &sb.cache_home).exists(),
        "no daemon bound, so the degrade cost the server and not just the voice: {client}"
    );

    // The decisive one: who ANSWERED. Asked on a SECOND call, deliberately —
    // the first is the cold one, and a cold workspace can be refused
    // `corpus_warming` while the drawer rebuilds, which degrades the client to
    // its own engine and would report `ephemeral` for a perfectly healthy
    // daemon. By now the daemon is resident and the workspace warm, so
    // `source` answers the question this test is actually asking.
    let second = sb
        .links(&ws, "1")
        .wait_with_output()
        .expect("mrd links exits");
    // Parsed, not substring-matched: `source` is a TOP-LEVEL key of the
    // `links --json` frame (`engine.rs` § `Format::Json`), so reading it is
    // independent of the serializer's spacing and of anything the fixture's own
    // link bodies might happen to spell.
    let stdout = String::from_utf8_lossy(&second.stdout);
    let answer: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!(
            "`links --json` did not serve JSON ({e}): {stdout}\nstderr: {}",
            String::from_utf8_lossy(&second.stderr)
        )
    });
    assert_eq!(
        answer["source"].as_str(),
        Some("daemon"),
        "a mute daemon must still ANSWER, not be degraded past: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&second.stderr)
    );
}

/// Off, nothing changes: no voice file is created, and the daemon keeps the
/// null stderr it was always spawned with. The gate is the operator's own
/// switch, so a run that did not ask for the mode pays nothing.
#[test]
fn the_mode_off_creates_no_lane() {
    let sb = Sandbox::new();
    let ws = sb.workspace();

    let out = Command::new(mrd_bin())
        .args(["script", "--json"])
        .env("HOME", &sb.home)
        .env("XDG_CACHE_HOME", &sb.cache_home)
        .env_remove("MRD_TIMING")
        .env_remove("MERIDIAN_WORKSPACE")
        .current_dir(&ws)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            common::feed_stdin(&mut child, PROGRAM.as_bytes());
            child.wait_with_output()
        })
        .expect("spawn mrd script");
    assert_eq!(
        out.status.code(),
        Some(0),
        "the read-only script failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // The daemon it auto-spawned answered — so the spawn path ran — and left
    // no file behind.
    assert!(
        common::child_daemon_pidfile(&sb.home, &sb.cache_home).exists(),
        "no daemon was spawned, so this proves nothing about the off path"
    );
    assert!(
        !sb.voice().exists(),
        "the mode is off and a lane was created anyway: {}",
        sb.voice().display()
    );
}

/// The N4 receipt: concurrent work served by ONE daemon is separable afterwards.
/// Same `p` (one process), different `t` — which is exactly what "byte-identical
/// `phase=snapshot` lines" lacked.
///
/// Three clients and the daemon all append to ONE sink file here, which is the
/// arrangement `status.md` used to say was not demultiplexable.
///
/// **One workspace per caller, and that is load-bearing, not tidiness.** The
/// cold gate folds on a per-workspace `drawer-rebuild` thread, single-flight
/// (`registry.rs` § `cold_gate`), so what `t` separates is folds. Three fresh
/// workspaces is what makes three of them exist; the same three calls against
/// ONE workspace would correctly yield ONE `who=`, which is
/// [`two_ops_on_one_cold_workspace_share_one_fold`].
///
/// **Why `links` and not `script`.** The card's receipt named two concurrent
/// `script` calls. Measured on workstation-nyc-2 2026-08-23, three `script`
/// calls on three fresh workspaces produced ONE daemon fold, so there was no
/// second daemon-side line to attribute and this gate failed on that. **Why is
/// not established.** A pattern-less `mrd script` evaluates client-side
/// (`script/cmd.rs`), and the wire `read` its program issues takes
/// `cold_gate_wire` exactly as `links` does, so by code it should fold per fresh
/// workspace. Two explanations have already been wrong here: the `run-plane.md`
/// lazy-fold law (it binds `runner::run`/`rehearse` callers, not this), and
/// `script_op::serve` (a pattern-less script never reaches it). The observation
/// stands, the cause does not; `links` folds deterministically per fresh
/// workspace, which is all this gate needs.
///
/// What this does NOT claim: that `who=` is a request id, or a connection. A
/// thread reused for later work keeps its ordinal — the field separates
/// concurrent work, never successive work on one thread (`crates/timing` § who).
#[test]
fn concurrent_ops_on_one_daemon_are_attributable() {
    let sb = Sandbox::new();
    let sink_dir = tempfile::tempdir().expect("sink dir");
    let sink = sink_dir.path().join("timing.log");
    let arg = sink.to_str().expect("utf-8 sink").to_owned();

    // Warm the daemon first, so the two calls below are served by one resident
    // process rather than racing to spawn it.
    let warm = sb
        .links(&sb.workspace_named("first"), &arg)
        .wait_with_output()
        .expect("mrd links exits");
    assert_eq!(
        warm.status.code(),
        Some(0),
        "the warm-up links failed: {}",
        String::from_utf8_lossy(&warm.stderr)
    );

    let (second, third) = (sb.workspace_named("second"), sb.workspace_named("third"));
    let (left, right) = (sb.links(&second, &arg), sb.links(&third, &arg));
    for child in [left, right] {
        let out = child.wait_with_output().expect("mrd links exits");
        assert_eq!(
            out.status.code(),
            Some(0),
            "a concurrent links failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    let text = wait_for(&sink, |text| {
        measurements(text)
            .iter()
            .filter(|m| m.cmd == "daemon")
            .map(|m| m.who.clone())
            .collect::<BTreeSet<_>>()
            .len()
            >= 3
    });
    let all = measurements(&text);
    let served: Vec<&Measurement> = all.iter().filter(|m| m.cmd == "daemon").collect();
    assert!(
        !served.is_empty(),
        "the daemon wrote no measurement to the shared sink: {text}"
    );

    let daemon_pids: BTreeSet<&str> = served.iter().map(|m| pid_of(&m.who)).collect();
    assert_eq!(
        daemon_pids.len(),
        1,
        "one resident daemon should have served all three calls: {daemon_pids:?}\n{text}"
    );
    let mut by_who: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for m in &served {
        by_who
            .entry(m.who.as_str())
            .or_default()
            .push(m.phase.as_str());
    }
    assert_eq!(
        by_who.len(),
        3,
        "three workspaces, three folds, three emitters — a shared `who=` means \
         those lines are still indistinguishable: {by_who:?}\n{text}"
    );
    // Each fold's group is a WHOLE fold, not a fragment of one shuffled in with
    // another's: attribution that split a fold across two `who=` would be worse
    // than none.
    for (who, seen) in &by_who {
        assert!(
            seen.contains(&"snapshot") && seen.contains(&"corpus.build"),
            "{who} carries a partial fold: {seen:?}\n{text}"
        );
    }

    // And the clients that appended to the SAME file are separable from the
    // daemon and from each other — the `p` half doing the job the doc used to
    // say no field did.
    let client_pids: BTreeSet<&str> = all
        .iter()
        .filter(|m| m.cmd == "links")
        .map(|m| pid_of(&m.who))
        .collect();
    assert_eq!(
        client_pids.len(),
        3,
        "the three clients sharing this file are not separable: {client_pids:?}\n{text}"
    );
    assert!(
        client_pids.is_disjoint(&daemon_pids),
        "a client and the daemon share a pid: {client_pids:?} vs {daemon_pids:?}"
    );
}

/// The other side of the same coin, and the one a reader is most likely to get
/// wrong: two ops racing for ONE cold workspace produce **one** `who=`, not two.
///
/// That is not attribution failing. The cold gate is single-flight per
/// workspace (`registry.rs` § `cold_gate`): the first caller kicks a
/// `drawer-rebuild` thread, the second waits on that same rebuild or is refused
/// `corpus_warming`, and there is exactly one fold for both to be attributed
/// to. An operator who reads the missing `t2` as "my second op never ran" is
/// being misled by the very field that was added to stop that — hence a gate,
/// so `docs/status.md` § What `t` names here cannot drift away from it.
///
/// Robust to the race either way: if the second call arrives after the fold
/// finished, its workspace is warm and it emits nothing server-side. One fold
/// either way, which is the claim.
#[test]
fn two_ops_on_one_cold_workspace_share_one_fold() {
    let sb = Sandbox::new();
    let sink_dir = tempfile::tempdir().expect("sink dir");
    let sink = sink_dir.path().join("timing.log");
    let arg = sink.to_str().expect("utf-8 sink").to_owned();

    // Bring the daemon up on a DIFFERENT workspace, so the workspace under test
    // is still cold when the two racing calls arrive.
    let warm = sb
        .links(&sb.workspace_named("elsewhere"), &arg)
        .wait_with_output()
        .expect("mrd links exits");
    assert_eq!(
        warm.status.code(),
        Some(0),
        "the warm-up links failed: {}",
        String::from_utf8_lossy(&warm.stderr)
    );
    let before: BTreeSet<String> = measurements(&read(&sink))
        .into_iter()
        .filter(|m| m.cmd == "daemon")
        .map(|m| m.who)
        .collect();

    let contested = sb.workspace_named("contested");
    let (left, right) = (sb.links(&contested, &arg), sb.links(&contested, &arg));
    for child in [left, right] {
        let out = child.wait_with_output().expect("mrd links exits");
        assert_eq!(
            out.status.code(),
            Some(0),
            "a racing links failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    let text = wait_for(&sink, |text| {
        measurements(text)
            .iter()
            .filter(|m| m.cmd == "daemon")
            .any(|m| !before.contains(&m.who))
    });
    let added: BTreeSet<String> = measurements(&text)
        .into_iter()
        .filter(|m| m.cmd == "daemon" && !before.contains(&m.who))
        .map(|m| m.who)
        .collect();
    assert_eq!(
        added.len(),
        1,
        "one contested workspace is one fold, so one new emitter — got {added:?}\n{text}"
    );
}
