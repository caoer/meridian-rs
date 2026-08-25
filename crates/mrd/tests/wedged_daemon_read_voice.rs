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
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

mod common;

const DOC: &str = "# Alpha\n\nsee [[beta]]\n";

/// This client's own baked build token. The fake greets with it verbatim so the
/// 0025 skew check passes and a greeting run genuinely reaches the request
/// exchange — a skew is a refusal, and would stop the run one site early.
const OWN_BUILD: &str = env!("MRD_BUILD_SHA");

/// How long the fake waits on one connection's next frame before giving up on
/// it. Only bounds a client that connects and says nothing; every client here
/// writes its frame immediately.
const REQUEST_SLICE: Duration = Duration::from_secs(2);

/// The fake's accept-loop idle poll — it runs non-blocking so the stop flag is
/// checked between connections.
const ACCEPT_POLL: Duration = Duration::from_millis(20);

/// A daemon that is demonstrably UP, and then is not.
///
/// It answers `{"op":"ping"}` with `{"status":"pong"}` always, and answers the
/// v3 hello `ok:true` for its first `dark_after` greetings. When that quota is
/// spent it goes dark: it accepts every connection, **holds it open**, and
/// answers nothing at all — including the liveness probes.
///
/// That sequence is what puts the client in `wedge::read_line`'s
/// died-mid-request arm rather than in a spawn failure: the pong satisfies
/// `engine::ensure_daemon`, so no daemon is ever spawned and no spawn quote is
/// ever recorded, and the going-dark is what the tick's `answers_ping` finds.
/// Holding the stream matters — dropping it would send an EOF, and
/// `wedge::read_line` answers EOF with `Ok(0)`, never with a verdict.
///
/// # Why the quota exists: it CHOOSES which guarded site the wedge lands on
///
/// `read_cmd::daemon_read` guards TWO exchanges — the hello and the request —
/// and a fake that goes dark on the first non-ping frame can only ever drive
/// the first. `dark_after: 0` wedges the HELLO site; `dark_after: 1` greets and
/// wedges the REQUEST site. Without that choice the second site is gated by
/// nothing and a revert of it survives every test in this file.
///
/// # The latch fires on a frame READ, and on the greeting quota — never on silence
///
/// A read that merely EXPIRES latches nothing: it is held open and abandoned.
/// Latching on an empty line would let a ping slow enough to miss its slice
/// take the fake dark, `ensure_daemon` would then fail to spawn (this fake owns
/// the socket) and the run would degrade with a SPAWN quote — reddening the
/// negative assertions below and pointing the reader at the wrong arm entirely.
/// Symmetrically the quota latches the instant the last greeting is WRITTEN,
/// not when the next frame arrives, so no arm of this fixture depends on a
/// frame landing inside `REQUEST_SLICE`.
struct FakeDaemon {
    done: Arc<AtomicBool>,
    greetings: Arc<AtomicUsize>,
    thread: Option<JoinHandle<()>>,
}

impl FakeDaemon {
    /// Dark on the first non-ping frame — the HELLO exchange.
    fn pings_until_it_goes_dark(listener: UnixListener) -> Self {
        Self::goes_dark_after(listener, 0)
    }

    fn goes_dark_after(listener: UnixListener, dark_after: usize) -> Self {
        let done = Arc::new(AtomicBool::new(false));
        let greetings = Arc::new(AtomicUsize::new(0));
        let flag = Arc::clone(&done);
        let counted = Arc::clone(&greetings);
        listener
            .set_nonblocking(true)
            .expect("poll instead of blocking, so the stop flag is checked");
        let thread = thread::spawn(move || {
            let mut held: Vec<UnixStream> = Vec::new();
            let mut dark = false;
            let mut greeted = 0usize;
            while !flag.load(Ordering::SeqCst) {
                let Ok((stream, _)) = listener.accept() else {
                    thread::sleep(ACCEPT_POLL);
                    continue;
                };
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(REQUEST_SLICE));
                if dark {
                    // Every connection after the fake went dark is a liveness
                    // probe that must also go unanswered.
                    held.push(stream);
                    continue;
                }
                let Ok(clone) = stream.try_clone() else {
                    continue;
                };
                let mut reader = BufReader::new(clone);
                let mut writer = stream;
                // One connection carries MANY frames — the read path sends its
                // hello and its request down the same stream — so serve frames
                // until this connection ends or the fake goes dark.
                loop {
                    let mut line = String::new();
                    match reader.read_line(&mut line) {
                        Ok(n) if n > 0 => {}
                        // A clean EOF: the peer is done with us.
                        Ok(_) => break,
                        // The slice expired and NOTHING was said. Hold rather
                        // than drop — dropping would send an EOF the client
                        // would read as a short answer — and latch nothing.
                        Err(_) => {
                            held.push(writer);
                            break;
                        }
                    }
                    if line.contains("\"ping\"") {
                        let _ = writer.write_all(b"{\"status\":\"pong\"}\n");
                        let _ = writer.flush();
                        continue;
                    }
                    if greeted < dark_after {
                        greeted += 1;
                        counted.store(greeted, Ordering::SeqCst);
                        let _ = writer.write_all(hello_frame().as_bytes());
                        let _ = writer.flush();
                        if greeted < dark_after {
                            continue;
                        }
                    }
                    // Either the greeting quota is spent (this connection's
                    // REQUEST goes unanswered) or there was no quota at all
                    // (this frame IS the hello). Both are the exchange this
                    // fake exists to wedge.
                    dark = true;
                    held.push(writer);
                    break;
                }
            }
        });
        Self {
            done,
            greetings,
            thread: Some(thread),
        }
    }

    /// How many v3 hellos this fake actually answered. The positive control
    /// that a wedge landed on the REQUEST site and not, by accident, on the
    /// hello site one exchange earlier.
    fn greetings_served(&self) -> usize {
        self.greetings.load(Ordering::SeqCst)
    }
}

/// A v3 hello answer carrying THIS build's identity, so `hello_identity_skew`
/// passes and the run proceeds to the request exchange.
fn hello_frame() -> String {
    format!(
        "{{\"ok\":true,\"body\":{{\"proto\":1,\"server\":\"fake/0\",\"contract\":\"v3\",\
         \"caps\":[],\"identity\":{{\"build\":\"{OWN_BUILD}\"}}}}}}\n"
    )
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
        FakeDaemon::pings_until_it_goes_dark(self.bind_socket())
    }

    /// The same fake, standing on the same socket, but greeting `dark_after`
    /// times first — so the wedge lands on a CHOSEN exchange.
    fn fake_daemon_greeting(&self, dark_after: usize) -> FakeDaemon {
        FakeDaemon::goes_dark_after(self.bind_socket(), dark_after)
    }

    fn bind_socket(&self) -> UnixListener {
        let socket = common::child_socket_path(&self.home, &self.cache_home);
        if let Some(parent) = socket.parent() {
            std::fs::create_dir_all(parent).expect("socket parent");
        }
        UnixListener::bind(&socket).expect("bind the fake")
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

/// The `read` verb's FIRST guarded exchange — the HELLO. `read_cmd::daemon_read`
/// discarded the same verdict at its own `.ok()?` sites, and the read path is
/// the higher-traffic half, so leaving it silent would have left the card's own
/// symptom in place on the verb most likely to meet it.
///
/// The fake wedges on the first non-ping frame, so this drives the hello site
/// (`read_cmd.rs`, the `hello` `served_or_recorded`) and nothing past it. The
/// request site is a SEPARATE guard and has its own test below — one test that
/// reds for both is detection without attribution.
#[test]
fn read_states_the_transport_verdict_instead_of_degrading_mute() {
    let sb = Sandbox::new();
    let ws = sb.workspace();
    let fake = sb.fake_daemon();

    let (out, elapsed) = sb.run(&ws, &["read", "a.md"]);
    let said = stderr_of(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Alpha"),
        "the degrade must still serve the answer — a run that fails proves nothing about \
         a SILENT degrade: {stdout}\nstderr: {said}"
    );
    // The site control, stated as a COUNT rather than as a hope: this run wedged
    // BEFORE any hello was answered, so the verdict below is the hello site's.
    assert_eq!(
        fake.greetings_served(),
        0,
        "this fake greeted, so the run reached the REQUEST exchange and this test is no \
         longer about the hello site: {said}"
    );
    assert_the_verdict_reached_the_user(&said, &out, elapsed);
}

/// The `read` verb's SECOND guarded exchange — the REQUEST, reached only after a
/// hello that SUCCEEDED. This is the site a real wedge actually hits: the hello
/// is cheap, and the request is where an engine stalls on a big corpus.
///
/// Without this test the request site is gated by nothing — reverting its
/// `served_or_recorded` to `.ok()?` leaves every other test in this file green,
/// because they all wedge one exchange earlier and never reach it. A verdict
/// computed and thrown away there is indistinguishable, from outside the
/// process, from one never computed — this file's whole thesis, applied to the
/// exchange that carries it.
#[test]
fn read_states_the_transport_verdict_when_the_daemon_wedges_on_the_request() {
    let sb = Sandbox::new();
    let ws = sb.workspace();
    let fake = sb.fake_daemon_greeting(1);

    let (out, elapsed) = sb.run(&ws, &["read", "a.md"]);
    let said = stderr_of(&out);

    let stdout = String::from_utf8_lossy(&out.stdout);
    // POSITIVE CONTROLS, both as expected COUNTS or exact absences, and both
    // BEFORE the verdict assertion: a run that never got past the hello would
    // satisfy every assertion below while proving only what the test above
    // already proves.
    assert_eq!(
        fake.greetings_served(),
        1,
        "the fake answered {} hellos, not the one this test needs — the run did not reach \
         the request exchange, so this is the hello site's verdict wearing this test's \
         name: {said}",
        fake.greetings_served()
    );
    assert!(
        !said.contains("SKEW"),
        "the greeting's identity did not match this build, so the run refused at the skew \
         check and never sent its request: {said}"
    );
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
