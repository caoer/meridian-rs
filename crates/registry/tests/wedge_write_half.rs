//! **A stalled DRAIN is a question, not a verdict** — the write-half twin of
//! `wedge_tick_partial.rs`.
//!
//! `wedge::bind` used to put the whole cap on the write as a flat socket
//! timeout, on the premise that "a timeout mid-frame leaves a partial line on
//! the wire, which is unrecoverable" — so a write that could not drain was
//! called a dead peer. Two things were wrong with that. The premise is false:
//! `write(2)` under `SO_SNDTIMEO` reports the partial COUNT, so an offset
//! resumes losslessly (`an_oversized_frame_stalled_past_ticks_arrives_whole`
//! below is that fact, executable). And the verdict it produced was the very
//! message class card `dial-eagain-under-pipeline-load` was opened over,
//! arriving by the other half: a bare `WouldBlock` — "Resource temporarily
//! unavailable (os error 35/11)" — with no wedged-vs-absent separation and no
//! statement of what happened to the bytes.
//!
//! The write half also knows something the read half never can: the frame is
//! not whole on the wire, so the daemon never parsed a request and **nothing
//! was committed**. Every verdict here must say so — a caller told "unknown"
//! reads a file back and distrusts it; a caller told "nothing landed" retries.
//!
//! Origin: card `wedge-write-half-no-discipline` (seat d6afbb91), carded from
//! the PR-234 review note at main `e8705b9b`.

use std::io::Read;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use registry::wedge;

/// Far past any unix-socket send buffer (measured: macOS hands over ~8 KiB
/// before blocking, Linux ~208 KiB), so the write is GUARANTEED to stall on a
/// peer that is not reading. Small enough to stay a fast test.
const OVERSIZED: usize = 2 * 1024 * 1024;

/// Longer than one tick, far under the cap: the stall must straddle a tick so
/// the resume is exercised, and the write must still be alive when the peer
/// finally drains.
const STALL: Duration = Duration::from_secs(5);

/// How long a test waits before calling a bounded write unbounded. A BACKSTOP,
/// never a budget — it only decides whether a regression FAILS or hangs the
/// suite forever.
#[allow(clippy::duration_suboptimal_units)]
const NEVER: Duration = Duration::from_secs(120);

/// A listener that answers `pong` on every connection after the first, holding
/// the first open and — depending on `drain_after` — either never reading it
/// (a stalled drain) or reading it dry once the stall has elapsed (a SLOW
/// drain, provably up).
///
/// Holding is a FLAG, not a sleep, for the reason `wedge::tests::Mute` gives:
/// one constant cannot both outlive the slowest plausible write and stay cheap
/// on a green run.
struct Peer {
    done: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
    /// The bytes the peer read off the first connection, once it drained.
    drained: Option<mpsc::Receiver<Vec<u8>>>,
}

impl Peer {
    /// Accept forever; pong on every connection but the first. `drain_after`
    /// `Some(d)` reads the first connection dry starting `d` after the peer
    /// came up — a busy daemon that catches up. `None` never reads it at all.
    /// `pong` false makes even the probe connections mute — a daemon that is
    /// gone, not merely busy.
    fn new(listener: UnixListener, drain_after: Option<Duration>, pong: bool) -> Self {
        use std::io::Write as _;

        let done = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&done);
        let (tx, rx) = mpsc::channel();
        listener
            .set_nonblocking(true)
            .expect("poll instead of blocking, so the flag is checked");
        let thread = thread::spawn(move || {
            let started = Instant::now();
            let mut first: Option<UnixStream> = None;
            let mut held: Vec<UnixStream> = Vec::new();
            let mut draining = false;
            while !flag.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        if first.is_none() {
                            first = Some(stream);
                        } else if pong {
                            let _ = stream.write_all(b"{\"status\":\"pong\"}\n");
                            let _ = stream.flush();
                        } else {
                            held.push(stream);
                        }
                    }
                    Err(_) => thread::sleep(Duration::from_millis(20)),
                }
                if let Some(after) = drain_after
                    && !draining
                    && started.elapsed() >= after
                    && let Some(stream) = first.take()
                {
                    draining = true;
                    // Off-thread so the accept loop keeps answering probes
                    // THROUGH the drain — this is a busy daemon, not a paused
                    // one, and a tick landing mid-drain must still see it up.
                    let sink = tx.clone();
                    thread::spawn(move || {
                        let mut stream = stream;
                        let mut bytes = Vec::new();
                        // The accepted stream inherits the LISTENER's
                        // non-blocking flag on BSD/macOS — the same fact the
                        // daemon's own accept loop states and handles
                        // (`server.rs`, `spawn_accept`'s dispatch arm). Without
                        // this reset the drain is non-blocking, `SO_RCVTIMEO`
                        // below never applies, and the first `read` past the
                        // socket buffer returns `WouldBlock` — which the `Err(_)`
                        // arm treats as end-of-frame. The drain then RETURNS and
                        // drops the stream, so the writer under test gets a
                        // truthful `BrokenPipe` from a peer this harness killed.
                        // Measured on Darwin: 32 KiB drained of 2 MiB, then EPIPE.
                        // A no-op on Linux, where accept(2) does not pass file
                        // status flags to the new socket — which is why CI stayed
                        // green while every mac run failed 3/3.
                        stream
                            .set_nonblocking(false)
                            .expect("the drain must BLOCK — see above");
                        stream
                            .set_read_timeout(Some(Duration::from_secs(30)))
                            .expect("bounded drain");
                        let mut buf = vec![0u8; 64 * 1024];
                        while bytes.len() < OVERSIZED {
                            match stream.read(&mut buf) {
                                // EOF, or the drain's own bound: either way no
                                // more of this frame is coming.
                                Ok(0) | Err(_) => break,
                                Ok(n) => bytes.extend_from_slice(&buf[..n]),
                            }
                        }
                        let _ = sink.send(bytes);
                    });
                }
            }
            drop(first);
            drop(held);
        });
        Peer {
            done,
            thread: Some(thread),
            drained: Some(rx),
        }
    }
}

impl Drop for Peer {
    fn drop(&mut self) {
        self.done.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Run the disciplined write off-thread and answer what it returned. The shape
/// of the bug under test is a call that does not return, and a test that shares
/// its thread cannot report that.
fn write_off_thread(
    sock: &std::path::Path,
    cap: Duration,
    frame: Vec<u8>,
) -> (Result<(), (std::io::ErrorKind, String)>, Duration) {
    let (tx, rx) = mpsc::channel();
    let dialled = sock.to_owned();
    thread::spawn(move || {
        let stream = UnixStream::connect(&dialled).expect("connect");
        wedge::bind(&stream).expect("bind the discipline");
        let mut writer = &stream;
        let started = Instant::now();
        let outcome = wedge::write_all(&mut writer, &dialled, cap, &frame);
        let _ = tx.send((
            outcome.map_err(|e| (e.kind(), e.to_string())),
            started.elapsed(),
        ));
        // Held open until the peer has the bytes, so a drained frame cannot be
        // truncated by this side hanging up.
        thread::sleep(Duration::from_secs(2));
    });
    rx.recv_timeout(NEVER)
        .expect("the disciplined write returned — a socket with no write timeout never would")
}

/// **THE REGRESSION, and the premise the flat cap rested on.** A body far past
/// the socket buffer, against a peer that does not read for five seconds and
/// then drains: the write must survive the stall and the peer must receive
/// every byte, once, in order.
///
/// A lost byte, a duplicated slice, or a verdict here means the tick resumed
/// wrongly — and the flat bound this replaces would have failed the whole write
/// at the first tick, on a daemon that was alive the entire time.
#[test]
fn an_oversized_frame_stalled_past_ticks_arrives_whole() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sock = dir.path().join("daemon.sock");
    let mut peer = Peer::new(UnixListener::bind(&sock).expect("bind"), Some(STALL), true);

    // A recognisable, non-uniform body: a duplicated or dropped slice moves
    // every later byte, so an exact compare catches an off-by-one resume.
    let frame: Vec<u8> = (0..OVERSIZED)
        .map(|i| u8::try_from(i % 251).unwrap())
        .collect();
    let (outcome, elapsed) = write_off_thread(&sock, wedge::WEDGE_CAP, frame.clone());

    match outcome {
        Ok(()) => {}
        Err((kind, message)) => panic!(
            "a live peer that was merely slow to drain was called dead — kind={kind:?} \
             message={message:?}"
        ),
    }
    assert!(
        elapsed >= wedge::TICK,
        "the socket's own tick is what the write rode through, not something faster: {elapsed:?}"
    );
    let drained = peer
        .drained
        .take()
        .expect("the receiver")
        .recv_timeout(NEVER)
        .expect("the peer drained the frame");
    assert_eq!(
        drained.len(),
        frame.len(),
        "every byte arrives exactly once — no loss, no duplicated slice across the tick"
    );
    assert!(drained == frame, "and in order, byte for byte");
}

/// **The died-mid-send arm.** A peer that stopped draining AND stopped
/// answering probes is gone, not busy. The verdict is `ConnectionAborted`, and
/// it must state the commit fact: the frame never arrived whole.
#[test]
fn a_peer_that_stops_draining_and_stops_answering_is_reported_as_died_mid_send() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sock = dir.path().join("daemon.sock");
    let _peer = Peer::new(UnixListener::bind(&sock).expect("bind"), None, false);

    let (outcome, elapsed) = write_off_thread(&sock, wedge::WEDGE_CAP, vec![b'x'; OVERSIZED]);
    let (kind, message) = outcome.expect_err("a peer that never drains does not take the frame");
    assert_eq!(
        kind,
        std::io::ErrorKind::ConnectionAborted,
        "a peer that fails the liveness probe is gone, not merely slow: {message}"
    );
    assert!(
        message.contains("nothing was committed"),
        "the write half KNOWS the frame never arrived whole, and owes the caller that fact — \
         'unknown' is the read half's answer, not this one: {message}"
    );
    assert!(
        elapsed >= wedge::TICK,
        "the socket's own tick fired, not something faster: {elapsed:?}"
    );
}

/// **The wedged arm, which the flat bound conflated with the first.** The peer
/// answers every probe and never drains: up, and wedged. The cap — not the
/// tick — ends it, and the message must separate the wedge from the absence,
/// because the remedies are opposite.
///
/// This is the arm that used to surface as a bare `Resource temporarily
/// unavailable`.
#[test]
fn a_pinging_peer_that_never_drains_spends_the_cap_and_says_it_is_wedged() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sock = dir.path().join("daemon.sock");
    let _peer = Peer::new(UnixListener::bind(&sock).expect("bind"), None, true);
    let cap = wedge::TICK + Duration::from_millis(200);

    let (outcome, elapsed) = write_off_thread(&sock, cap, vec![b'x'; OVERSIZED]);
    let (kind, message) = outcome.expect_err("a wedged peer does not take the frame");
    assert_eq!(
        kind,
        std::io::ErrorKind::TimedOut,
        "a draining-nothing-but-answering peer is TimedOut, never ConnectionAborted: {message}"
    );
    assert!(
        message.contains("wedged, not absent"),
        "the message must separate the wedge from the absence — the remedies differ: {message}"
    );
    assert!(
        message.contains("nothing was committed"),
        "and must say what happened to the bytes: {message}"
    );
    assert!(
        elapsed >= cap,
        "the CAP is what ended this arm, not the first tick: {elapsed:?}"
    );
}

/// A peer that DRAINS is untouched by the discipline: the frame goes out at
/// once, with no probe in between. The bound must cost the healthy path
/// nothing — that is what makes it safe to put on every door.
#[test]
fn a_draining_peer_is_served_immediately_and_never_probed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sock = dir.path().join("daemon.sock");
    let listener = UnixListener::bind(&sock).expect("bind");
    let serving = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut sink = Vec::new();
        let mut buf = vec![0u8; 64 * 1024];
        while sink.len() < OVERSIZED {
            match stream.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => sink.extend_from_slice(&buf[..n]),
            }
        }
        sink.len()
    });

    let stream = UnixStream::connect(&sock).expect("connect");
    wedge::bind(&stream).expect("bind the discipline");
    let mut writer = &stream;
    let started = Instant::now();
    wedge::write_all(&mut writer, &sock, wedge::WEDGE_CAP, &vec![b'x'; OVERSIZED])
        .expect("a draining peer takes the frame");
    let elapsed = started.elapsed();

    assert!(
        elapsed < wedge::TICK,
        "a healthy send never reaches the first tick, so it never probes: {elapsed:?}"
    );
    assert_eq!(
        serving.join().expect("the serving peer finishes"),
        OVERSIZED,
        "and the peer has the whole frame"
    );
}
