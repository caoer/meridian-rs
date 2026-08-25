//! **A daemon that is REACHED and then fails the exchange says so on the read
//! path** (card `mrd-read-path-discards-the-wedged-verdict`).
//!
//! The defect these gates close: `registry::wedge` computes a verdict on every
//! failed exchange — up-and-wedged (`TimedOut`, "restart the daemon") or dead
//! mid-request (`ConnectionAborted`, "the outcome of this call is unknown") —
//! and the read lanes threw it away. `try_daemon_links` collapsed it into
//! `Ok(DialedLinks::Unusable) | Err(_) => Ok(None)`, and `read_cmd::daemon_read`
//! into `.ok()?`. The user paid up to `registry::wedge::WEDGE_CAP` (60 s), got a
//! correct in-process answer, and was never told a daemon was wedged — so the
//! one remedy that ends it was never suggested and the wait recurred per
//! invocation. From outside the process, a computed verdict thrown away is
//! indistinguishable from a verdict never computed.
//!
//! The WRITE half already surfaced this (`script::wire_host::DialFailure`), so
//! the whole card is one-sidedness. Nothing here mints new prose: the sentence
//! asserted below is `registry::wedge`'s own, relayed verbatim through
//! `engine::degrade_reason` — the one seam all five daemon-path lanes consult.
//!
//! # Why the injected failure is died-mid-request, not the 60 s wedge
//!
//! `wedge::read_line` picks between its two arms by ONE test (`answers_ping`),
//! and which sentence each arm mints is gated directly, cheaply, and already:
//! `crates/registry/src/wedge.rs`
//! § `a_pinging_daemon_that_never_answers_spends_the_cap_and_says_it_is_wedged`
//! (asserts `TimedOut` + "wedged, not absent") and
//! § `a_mute_unpingable_daemon_aborts_the_read_instead_of_parking`. What was NOT
//! gated anywhere is the half these tests own: that the sentence survives
//! `engine::call` → `dial_links` → the degrade and reaches a user's stderr.
//! Both arms leave that code path by the same `Err`, so driving the cheap one
//! proves the relay; driving the expensive one would prove the same relay and
//! cost 60 s of suite time per verb. (The write half's own wedge gate makes the
//! identical trade — `wire_host.rs`
//! § `a_daemon_that_pings_but_never_greets_refuses_as_silent_not_as_absent`
//! drives the script entry's cap "so the assertion is cheap".)
//!
//! Fixture law (card `e2e-daemon-leak-fixture`): each sandbox reaps its own
//! daemon on drop. These never let one start — the fake owns the socket path —
//! so the reap is a no-op, and it stays because the fixture would own the
//! teardown either way.

use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

mod common;

const DOC: &str = "# Alpha\n\nsee [[beta]]\n";

/// How long the fake waits on one connection's request line before giving up on
/// it. Only bounds a client that connects and says nothing; every client here
/// writes its frame immediately.
const REQUEST_SLICE: Duration = Duration::from_millis(500);

/// The fake's accept-loop idle poll — it runs non-blocking so the stop flag is
/// checked between connections.
const ACCEPT_POLL: Duration = Duration::from_millis(20);

/// A daemon that is demonstrably UP, and then is not.
///
/// It answers `{"op":"ping"}` with `{"status":"pong"}` until the first non-ping
/// frame arrives; from then on it accepts every connection, **holds it open**,
/// and answers nothing at all — including the liveness probes.
///
/// That sequence is what puts the client in `wedge::read_line`'s
/// died-mid-request arm rather than in a spawn failure: the pong satisfies
/// `engine::ensure_daemon`, so no daemon is ever spawned and no spawn quote is
/// ever recorded, and the going-dark is what the tick's `answers_ping` finds.
/// Holding the stream matters — dropping it would send an EOF, which is a
/// different error entirely.
struct FakeDaemon {
    done: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl FakeDaemon {
    fn pings_until_it_goes_dark(listener: UnixListener) -> Self {
        let done = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&done);
        listener
            .set_nonblocking(true)
            .expect("poll instead of blocking, so the stop flag is checked");
        let thread = thread::spawn(move || {
            let mut held: Vec<UnixStream> = Vec::new();
            let mut dark = false;
            while !flag.load(Ordering::SeqCst) {
                let Ok((stream, _)) = listener.accept() else {
                    thread::sleep(ACCEPT_POLL);
                    continue;
                };
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(REQUEST_SLICE));
                let mut line = String::new();
                if let Ok(clone) = stream.try_clone() {
                    let _ = BufReader::new(clone).read_line(&mut line);
                }
                if dark || !line.contains("\"ping\"") {
                    // The first non-ping frame is the handshake this fake will
                    // never answer, and every connection after it is a liveness
                    // probe that must also go unanswered.
                    dark = true;
                    held.push(stream);
                    continue;
                }
                let mut answer = stream;
                let _ = answer.write_all(b"{\"status\":\"pong\"}\n");
                let _ = answer.flush();
            }
        });
        Self {
            done,
            thread: Some(thread),
        }
    }
}

impl Drop for FakeDaemon {
    fn drop(&mut self) {
        self.done.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

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

    /// An anchored workspace with one doc, so a degraded run is visibly a
    /// COMPLETE run and not an empty one.
    fn workspace(&self) -> PathBuf {
        let ws = self.tmp.path().join("project");
        std::fs::create_dir_all(ws.join(".git")).expect("git anchor");
        std::fs::write(ws.join("a.md"), DOC).expect("doc");
        std::fs::canonicalize(&ws).expect("canonical ws")
    }

    /// Stand the fake on the exact socket path this sandbox's `mrd` child will
    /// derive. Occupying it is also what keeps the run from auto-spawning a
    /// real daemon behind the test's back.
    fn fake_daemon(&self) -> FakeDaemon {
        let socket = common::child_socket_path(&self.home, &self.cache_home);
        if let Some(parent) = socket.parent() {
            std::fs::create_dir_all(parent).expect("socket parent");
        }
        FakeDaemon::pings_until_it_goes_dark(UnixListener::bind(&socket).expect("bind the fake"))
    }

    /// Run `mrd` with the timing mode explicitly OFF — the lane under test is
    /// the daemon's own, not the instrument's — and answer the wall time beside
    /// the output.
    fn run(&self, ws: &Path, args: &[&str]) -> (Output, Duration) {
        let mut cmd = common::mrd_command(&self.home, &self.cache_home);
        cmd.args(args)
            .env_remove("MRD_TIMING")
            .env_remove("MERIDIAN_WORKSPACE")
            .current_dir(ws);
        let started = Instant::now();
        let out = cmd.output().expect("mrd runs");
        (out, started.elapsed())
    }
}

fn stderr_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Every assertion both verbs share: the run still succeeds, it degraded, and
/// the degrade STATES the transport verdict instead of discarding it.
///
/// The two negative assertions are the load-bearing half of the card's "decide
/// whether both deserve the same voice": a reached-then-failed daemon must not
/// be reported with the spawn arms' words, because their remedy ("start one")
/// is the wrong act when one is already running.
fn assert_the_verdict_reached_the_user(said: &str, out: &Output, elapsed: Duration) {
    assert_eq!(
        out.status.code(),
        Some(0),
        "the degrade must never fail the run: {said}"
    );
    assert!(
        said.contains("no daemon answered"),
        "the degrade did not fire at all, so this run never reached the arm under test: {said}"
    );
    assert!(
        said.contains("The daemon was reached and the exchange failed"),
        "the read path degraded WITHOUT naming the verdict it had just computed — this is \
         exactly the silence the card is about: {said}"
    );
    // The wedge discipline's own sentence, relayed verbatim. Asserting the text
    // rather than merely "some reason appeared" is what proves the relay carries
    // the ERROR KIND: died-mid-request and up-and-wedged have opposite remedies
    // and would both satisfy a bare is-non-empty check.
    assert!(
        said.contains("it died mid-request"),
        "the relayed sentence is not `registry::wedge`'s verdict, so some other failure \
         produced this degrade and the test proves nothing about the wedge discipline: {said}"
    );
    assert!(
        said.contains("the outcome of this call is unknown"),
        "the verdict was relayed without its consequence clause, which is the half an \
         operator acts on: {said}"
    );
    // A daemon that was REACHED is not a daemon that failed to start. The two
    // recorders share one cell and one voice; they must not share one message.
    assert!(
        !said.contains("could not be launched at all"),
        "a daemon that answered a ping was reported as one that never launched: {said}"
    );
    assert!(
        !said.contains("never bound its socket"),
        "a daemon that answered a ping was reported as one that never bound: {said}"
    );
    // Corroboration by a second instrument, free with the run: the wedge tick is
    // what ended this, not something cheaper. A lower bound only — load can
    // lengthen a wait and never shorten it.
    assert!(
        elapsed >= registry::wedge::TICK,
        "the exchange ended faster than one wedge tick, so the tick is not what produced this \
         verdict: {elapsed:?}"
    );
}

/// The `links` verb — `engine::try_daemon_links`, the arm the card pins.
#[test]
fn links_states_the_transport_verdict_instead_of_degrading_mute() {
    let sb = Sandbox::new();
    let ws = sb.workspace();
    let _fake = sb.fake_daemon();

    let (out, elapsed) = sb.run(&ws, &["links", "--json"]);
    let said = stderr_of(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    let answer: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("`links --json` did not serve JSON ({e}): {stdout}"));
    assert_eq!(
        answer["source"].as_str(),
        Some("ephemeral"),
        "a daemon that never answered must be degraded past: {stdout}"
    );
    assert_the_verdict_reached_the_user(&said, &out, elapsed);
}

/// The `read` verb — `read_cmd::daemon_read`, which discarded the same verdict
/// at its own `.ok()?` sites. It is the higher-traffic half of the read path,
/// so leaving it silent would have left the card's own symptom in place on the
/// verb most likely to meet it.
#[test]
fn read_states_the_transport_verdict_instead_of_degrading_mute() {
    let sb = Sandbox::new();
    let ws = sb.workspace();
    let _fake = sb.fake_daemon();

    let (out, elapsed) = sb.run(&ws, &["read", "a.md"]);
    let said = stderr_of(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Alpha"),
        "the degrade must still serve the answer — a run that fails proves nothing about \
         a SILENT degrade: {stdout}\nstderr: {said}"
    );
    assert_the_verdict_reached_the_user(&said, &out, elapsed);
}

/// The other half of the split, and the reason the two arms are no longer one
/// pattern: a daemon that ANSWERS — with an op error — has produced no
/// transport verdict, so the degrade must not invent one.
///
/// The fake here greets normally and refuses the request with `ok:false`, which
/// is `DialedLinks::Unusable`. The run degrades exactly as before and the
/// degrade's first line stands alone.
#[test]
fn a_daemon_that_answers_an_op_error_gains_no_transport_verdict() {
    let sb = Sandbox::new();
    let ws = sb.workspace();
    let socket = common::child_socket_path(&sb.home, &sb.cache_home);
    if let Some(parent) = socket.parent() {
        std::fs::create_dir_all(parent).expect("socket parent");
    }
    let _refuser = Refuser::on(UnixListener::bind(&socket).expect("bind the fake"));

    let (out, _elapsed) = sb.run(&ws, &["links", "--json"]);
    let said = stderr_of(&out);

    assert_eq!(
        out.status.code(),
        Some(0),
        "the degrade must never fail the run: {said}"
    );
    assert!(
        said.contains("no daemon answered"),
        "an op error must still degrade: {said}"
    );
    assert!(
        !said.contains("The daemon was reached and the exchange failed"),
        "a daemon that ANSWERED was reported as a failed exchange — the two events are not \
         the same and their arms must stay apart: {said}"
    );
}

/// A daemon that answers every frame with `ok:false`: pongs pings, refuses the
/// v3 hello. Up, talking, and useless — `DialedLinks::Unusable`.
struct Refuser {
    done: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Refuser {
    fn on(listener: UnixListener) -> Self {
        let done = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&done);
        listener.set_nonblocking(true).expect("poll, not block");
        let thread = thread::spawn(move || {
            while !flag.load(Ordering::SeqCst) {
                let Ok((stream, _)) = listener.accept() else {
                    thread::sleep(ACCEPT_POLL);
                    continue;
                };
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(REQUEST_SLICE));
                let mut line = String::new();
                if let Ok(clone) = stream.try_clone() {
                    let _ = BufReader::new(clone).read_line(&mut line);
                }
                let mut answer = stream;
                let reply: &[u8] = if line.contains("\"ping\"") {
                    b"{\"status\":\"pong\"}\n"
                } else {
                    b"{\"ok\":false,\"error\":{\"code\":\"internal\"}}\n"
                };
                let _ = answer.write_all(reply);
                let _ = answer.flush();
            }
        });
        Self {
            done,
            thread: Some(thread),
        }
    }
}

impl Drop for Refuser {
    fn drop(&mut self) {
        self.done.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
