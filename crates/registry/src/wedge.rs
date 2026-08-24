//! The read-timeout discipline for a client waiting on the daemon.
//!
//! **A unix stream with no read timeout parks the caller forever.** A daemon
//! that accepts a frame and never answers it — wedged, not gone — is invisible
//! to every check *above* the socket: the caller is blocked inside `read(2)`
//! and never reaches the line that would have given up. Only the socket bounds
//! ONE call, so only the socket can end that wait.
//!
//! # A tick is a question, not a verdict
//!
//! The naive bound — one flat read timeout, and its expiry means "the daemon is
//! down" — is a defect this repo has already paid for. Card
//! `dial-eagain-under-pipeline-load`, measured 2026-08-24 on pipeline 1456:
//! four `mrd fingerprint` calls reported an absent daemon (`os error 11`,
//! EAGAIN — a read timeout on an ESTABLISHED connection) two lines after
//! `ensure_daemon` had proved the same socket answers a ping. A slow daemon was
//! rendered as a missing one, whose remedy is the opposite.
//!
//! So the timeout here is the TICK INTERVAL of [`read_line`], never the
//! verdict. On each tick the daemon is asked — over its own second connection,
//! itself bounded — whether it is still alive:
//!
//! | the tick finds | outcome |
//! |---|---|
//! | ping answers, cap unspent | keep reading; a slow daemon is not a dead one |
//! | ping does not answer | [`io::ErrorKind::ConnectionAborted`] — it really went away |
//! | ping answers, cap spent | [`io::ErrorKind::TimedOut`] — up, and wedged |
//!
//! The wait is therefore bounded AND evidenced: what normally ends it is the
//! daemon's own liveness, not a constant. [`WEDGE_CAP`] is only the floor under
//! that instrument.
//!
//! # What a caller observes
//!
//! An [`io::Error`], on a path that already had one. Every dialer of this
//! daemon treats a transport failure as *degrade to in-process* — the daemon is
//! an optimization, never a hard dependency (decision 0001 round 5). Before
//! this module those callers had one unreachable failure mode: park forever.
//! Now the wedged daemon lands where the absent one always did, and the two
//! carry different messages so a reader can tell them apart.
//!
//! # Sibling spelling
//!
//! `crates/mrd/src/script/wire_host.rs` holds the same discipline for the
//! script/write door — `PROBE_TIMEOUT`, `daemon_answers_ping`,
//! `SocketDoor::greet`. That door carries a script entry's own wall
//! clock and its own [`DialFailure`](../../mrd/script/wire_host) arms, so it is
//! not expressed in terms of this module; the two are kept equal BY HAND, like
//! the `WALL_CLOCK` / `DEFAULT_WALL_CLOCK` pair. Change one bound, grep for the
//! other.

use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

/// How long a liveness probe may take before the daemon is called unreachable.
/// Its own connection and its own bound: probing through [`crate::Client`]
/// would re-enter this discipline and, on a wedged daemon, ask the wedged
/// question twice.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// How often a blocked read pauses to ask whether the daemon is alive.
///
/// NOT a budget — an expired tick answers no question by itself, which is the
/// whole point (§ A tick is a question). Small enough that a dead daemon is
/// found promptly, large enough that a healthy round trip never probes at all.
pub const TICK: Duration = Duration::from_secs(2);

/// The backstop for the one case liveness cannot settle: a daemon that answers
/// pings forever and never answers THIS request. Up, and wedged — a bug worth
/// naming rather than a load condition worth waiting out.
///
/// Deliberately far above any answer a healthy daemon produces, because a
/// number is the wrong instrument here and this one is only the floor under the
/// right one.
// `Duration::from_mins` not const-stable at MSRV 1.96.
#[allow(clippy::duration_suboptimal_units)]
pub const WEDGE_CAP: Duration = Duration::from_secs(60);

/// Put `stream` under the discipline: reads tick at [`TICK`], writes are bound
/// by `cap`.
///
/// The write bound is the whole cap rather than a tick because a write is not a
/// tick loop — a timeout mid-frame leaves a partial line on the wire, which is
/// unrecoverable. A write that cannot drain within the cap is a dead peer, not
/// a slow one.
///
/// Call this BEFORE [`UnixStream::try_clone`]: `SO_RCVTIMEO`/`SO_SNDTIMEO` are
/// socket-level, so the clone inherits them and one call covers both halves.
///
/// # Errors
///
/// The socket rejected the option — the connection is unusable.
pub fn bind(stream: &UnixStream, cap: Duration) -> io::Result<()> {
    stream.set_read_timeout(Some(TICK))?;
    stream.set_write_timeout(Some(cap))?;
    Ok(())
}

/// Does the daemon at `socket` answer a registry `ping` within
/// [`PROBE_TIMEOUT`]? This is the EVIDENCE that replaces the clock.
///
/// Hand-rolled rather than [`crate::Client::ping`]: that call goes through
/// [`read_line`], so probing with it would recurse into the discipline it is
/// meant to inform.
#[must_use]
pub fn answers_ping(socket: &Path) -> bool {
    let Ok(stream) = UnixStream::connect(socket) else {
        return false;
    };
    if stream.set_read_timeout(Some(PROBE_TIMEOUT)).is_err()
        || stream.set_write_timeout(Some(PROBE_TIMEOUT)).is_err()
    {
        return false;
    }
    let Ok(mut writer) = stream.try_clone() else {
        return false;
    };
    if writer.write_all(b"{\"op\":\"ping\"}\n").is_err() || writer.flush().is_err() {
        return false;
    }
    let mut line = String::new();
    if BufReader::new(stream).read_line(&mut line).is_err() {
        return false;
    }
    serde_json::from_str::<serde_json::Value>(&line)
        .ok()
        .and_then(|v| {
            v.get("status")
                .and_then(serde_json::Value::as_str)
                .map(|s| s == "pong")
        })
        .unwrap_or(false)
}

/// Read one NDJSON line under the discipline. Drop-in for
/// [`BufRead::read_line`] — same append semantics, same `Ok(n)` byte count,
/// same `Ok(0)` at EOF — with the tick loop around it.
///
/// The request is NOT re-sent on a tick — a second frame on the same connection
/// is a different request, not a retry.
///
/// # Bytes accumulate raw; UTF-8 is validated once, when the line is whole
///
/// The tick lands wherever the daemon happened to stop writing, so it lands
/// mid-character routinely on any corpus that is not pure ASCII. Reading
/// straight into `line` across ticks therefore cannot work: std's
/// [`BufRead::read_line`] validates the bytes it appended *during that call*
/// and **truncates all of them** when that slice ends mid-character — and they
/// have already been consumed from the [`BufReader`], so they are gone. A tick
/// inside one multi-byte character would destroy a live daemon's complete,
/// valid answer and report it as [`io::ErrorKind::InvalidData`]: exactly the
/// class of wrong verdict this module exists to abolish, and reachable only
/// because the module bounds the read. It also made the count a lie — `Ok(n)`
/// was the last inner call's bytes, not the line's.
///
/// So the bytes land in a `Vec<u8>` via [`BufRead::read_until`], which has no
/// UTF-8 guard and leaves what it appended in place when the tick fires; the
/// completed line is validated once and pushed into `line`.
/// (Regression: `crates/registry/tests/wedge_tick_partial.rs`.)
///
/// `Ok(0)` is EOF and stays the caller's to interpret, exactly as it is on the
/// bare reader. A partial line then EOF answers `Ok(n)` with those bytes in
/// `line` and no trailing newline — again exactly as the bare reader does.
///
/// # Errors
///
/// On every error `line` is left untouched: a partial answer is not an answer.
///
/// - [`io::ErrorKind::ConnectionAborted`] — the daemon stopped answering
///   liveness probes: it died mid-request.
/// - [`io::ErrorKind::TimedOut`] — it answered probes for the whole `cap` and
///   never answered this request: up, and wedged.
/// - [`io::ErrorKind::InvalidData`] — the completed line is not valid UTF-8.
/// - Any other transport failure, verbatim.
pub fn read_line(
    reader: &mut BufReader<UnixStream>,
    socket: &Path,
    cap: Duration,
    line: &mut String,
) -> io::Result<usize> {
    let started = Instant::now();
    let mut bytes = Vec::new();
    loop {
        match reader.read_until(b'\n', &mut bytes) {
            // The delimiter arrived, or the stream ended. Either way nothing
            // further is coming for this line, so stop ticking.
            Ok(_) => break,
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                // Whatever `read_until` appended before the tick fired stays in
                // `bytes`. That is what makes the resumption lossless.
                let waited = started.elapsed();
                if !answers_ping(socket) {
                    return Err(io::Error::new(
                        io::ErrorKind::ConnectionAborted,
                        format!(
                            "the daemon accepted this request and then stopped answering \
                             liveness probes after {waited:?} — it died mid-request; nothing \
                             answered, so the outcome of this call is unknown"
                        ),
                    ));
                }
                if waited >= cap {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        format!(
                            "the daemon is up (it is still answering liveness probes) and has \
                             not answered this request in {waited:?} — it is wedged, not absent, \
                             so restarting the client will not help; restart the daemon"
                        ),
                    ));
                }
            }
            // No `Interrupted` arm: `read_until` absorbs EINTR itself, so it
            // cannot surface here.
            Err(e) => return Err(e),
        }
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "stream did not contain valid UTF-8",
        )
    })?;
    line.push_str(text);
    Ok(bytes.len())
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixListener;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, mpsc};
    use std::thread;

    use super::{
        BufReader, Duration, Instant, PROBE_TIMEOUT, TICK, UnixStream, Write, io, read_line,
    };

    /// How long a test waits before calling a bounded read unbounded.
    ///
    /// A BACKSTOP, never a budget: every arm below is expected to finish in a
    /// few ticks, and the only thing this number decides is whether a
    /// regression FAILS or hangs the suite forever. Set far above any load
    /// this suite meets, because a flaky red is worth less than a slow one.
    // `Duration::from_mins` not const-stable at MSRV 1.96 (same reason as
    // `WEDGE_CAP` above).
    #[allow(clippy::duration_suboptimal_units)]
    const NEVER_RETURNED: Duration = Duration::from_secs(120);

    /// A listener whose accepted connections are held open and mute until the
    /// test says otherwise.
    ///
    /// Holding is a FLAG, not a sleep: a sleeping holder must outlive the
    /// slowest plausible read (or it closes the socket and the reader sees EOF
    /// instead of the timeout under test) AND be joined at the end (so the test
    /// costs that sleep every green run). Both cannot be satisfied by one
    /// constant. The flag decouples them — the holder lives exactly as long as
    /// the measurement, on a fast machine and a loaded one alike.
    struct Mute {
        done: Arc<AtomicBool>,
        thread: Option<thread::JoinHandle<()>>,
    }

    impl Mute {
        /// Accept forever, answer nothing, hold every connection open.
        fn holding(listener: UnixListener) -> Self {
            let done = Arc::new(AtomicBool::new(false));
            let flag = Arc::clone(&done);
            listener
                .set_nonblocking(true)
                .expect("poll instead of blocking, so the flag is checked");
            let thread = thread::spawn(move || {
                let mut held = Vec::new();
                while !flag.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => held.push(stream),
                        Err(_) => thread::sleep(Duration::from_millis(20)),
                    }
                }
            });
            Mute {
                done,
                thread: Some(thread),
            }
        }

        /// Accept forever; answer a `pong` on every connection but the first,
        /// which is held mute. That is a daemon that is demonstrably UP and
        /// still will not answer the request in flight.
        fn ponging_after_the_first(listener: UnixListener) -> Self {
            let done = Arc::new(AtomicBool::new(false));
            let flag = Arc::clone(&done);
            listener
                .set_nonblocking(true)
                .expect("poll instead of blocking, so the flag is checked");
            let thread = thread::spawn(move || {
                let mut held = Vec::new();
                while !flag.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            if held.is_empty() {
                                held.push(stream);
                            } else {
                                let _ = stream.write_all(b"{\"status\":\"pong\"}\n");
                                let _ = stream.flush();
                            }
                        }
                        Err(_) => thread::sleep(Duration::from_millis(20)),
                    }
                }
            });
            Mute {
                done,
                thread: Some(thread),
            }
        }
    }

    impl Drop for Mute {
        fn drop(&mut self) {
            self.done.store(true, Ordering::SeqCst);
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }

    /// Run `read_line` off-thread and answer what it returned, or fail loudly
    /// if it never did — the shape of the bug under test is a call that does
    /// not return, and a test that shares its thread cannot report that.
    fn read_off_thread(
        sock: &std::path::Path,
        cap: Duration,
    ) -> (Result<usize, (io::ErrorKind, String)>, Duration) {
        let (tx, rx) = mpsc::channel();
        let dialled = sock.to_owned();
        thread::spawn(move || {
            let stream = UnixStream::connect(&dialled).expect("connect");
            super::bind(&stream, cap).expect("bind the discipline");
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            let started = Instant::now();
            let outcome = read_line(&mut reader, &dialled, cap, &mut line);
            let _ = tx.send((
                outcome.map_err(|e| (e.kind(), e.to_string())),
                started.elapsed(),
            ));
        });
        rx.recv_timeout(NEVER_RETURNED)
            .expect("the read returned — a socket with no read timeout never would")
    }

    /// **A daemon that accepts, answers nothing, and cannot be pinged must not
    /// park the caller.** The read is the only thing that can end this wait —
    /// every check above the socket is unreachable while the caller sits in
    /// `read(2)`.
    ///
    /// The mute listener also fails the liveness probe, so this is the
    /// died-mid-request arm. Only a LOWER bound is asserted on the elapsed
    /// time: it says the socket's own tick is what fired rather than something
    /// cheaper, and load can lengthen a wait but never shorten it.
    #[test]
    fn a_mute_unpingable_daemon_aborts_the_read_instead_of_parking() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sock = dir.path().join("daemon.sock");
        let _mute = Mute::holding(UnixListener::bind(&sock).expect("bind"));

        let (outcome, elapsed) = read_off_thread(&sock, super::WEDGE_CAP);
        let (kind, message) = outcome.expect_err("a mute daemon does not answer");
        assert_eq!(
            kind,
            io::ErrorKind::ConnectionAborted,
            "a mute listener that also fails the liveness probe is gone, not merely slow: {message}"
        );
        assert!(
            elapsed >= TICK,
            "the socket's own tick is what fired, not something faster: {elapsed:?}"
        );
    }

    /// **The other arm, which a flat timeout conflates with the first:** the
    /// daemon answers every ping and never answers the held request. That is a
    /// wedge, and the cap — not the tick — is what ends it. The error must say
    /// so, because the two remedies are opposite (bring the daemon up vs
    /// restart the daemon that is already up).
    #[test]
    fn a_pinging_daemon_that_never_answers_spends_the_cap_and_says_it_is_wedged() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sock = dir.path().join("daemon.sock");
        let _alive = Mute::ponging_after_the_first(UnixListener::bind(&sock).expect("bind"));
        let cap = TICK + Duration::from_millis(200);

        let (outcome, elapsed) = read_off_thread(&sock, cap);
        let (kind, message) = outcome.expect_err("a wedged daemon does not answer");
        assert_eq!(
            kind,
            io::ErrorKind::TimedOut,
            "an answering-but-wedged daemon is TimedOut, never ConnectionAborted: {message}"
        );
        assert!(
            message.contains("wedged, not absent"),
            "the message must separate the wedge from the absence — the remedies differ: {message}"
        );
        assert!(
            elapsed >= cap,
            "the CAP is what ended this arm, not the first tick: {elapsed:?}"
        );
    }

    /// The probe is itself bounded: a socket nobody answers must not park the
    /// prober, or the instrument inherits the disease it diagnoses.
    #[test]
    fn the_liveness_probe_is_itself_bounded() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sock = dir.path().join("daemon.sock");
        let _mute = Mute::holding(UnixListener::bind(&sock).expect("bind"));

        let started = Instant::now();
        assert!(
            !super::answers_ping(&sock),
            "a socket that answers nothing does not answer a ping"
        );
        let elapsed = started.elapsed();
        assert!(
            elapsed >= PROBE_TIMEOUT,
            "the probe's own timeout is what fired: {elapsed:?}"
        );
    }

    /// An absent socket answers instantly and negatively — the probe must not
    /// spend its timeout on a `connect(2)` that already failed.
    #[test]
    fn an_absent_socket_fails_the_probe_without_waiting() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sock = dir.path().join("nothing-here.sock");
        let started = Instant::now();
        assert!(!super::answers_ping(&sock));
        assert!(
            started.elapsed() < PROBE_TIMEOUT,
            "a refused connect is a verdict, not a wait"
        );
    }

    /// A daemon that ANSWERS is untouched by the discipline: the round trip
    /// returns the line, at once, with no probe in between. The bound must cost
    /// the healthy path nothing — that is what makes it safe to put on every
    /// door.
    #[test]
    fn an_answering_daemon_is_served_immediately_and_never_probed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sock = dir.path().join("daemon.sock");
        let listener = UnixListener::bind(&sock).expect("bind");
        let serving = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            stream
                .write_all(b"{\"status\":\"pong\"}\n")
                .expect("answer at once");
            stream.flush().expect("flush");
            // Held open until the reader has the line, so the assertion cannot
            // be satisfied by an EOF instead of by the answer.
            thread::sleep(Duration::from_millis(50));
        });

        let stream = UnixStream::connect(&sock).expect("connect");
        super::bind(&stream, super::WEDGE_CAP).expect("bind the discipline");
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        let started = Instant::now();
        let read = read_line(&mut reader, &sock, super::WEDGE_CAP, &mut line).expect("served");
        let elapsed = started.elapsed();

        assert!(read > 0, "the answer is bytes, not EOF");
        assert!(line.contains("pong"), "the line arrives verbatim: {line:?}");
        assert!(
            elapsed < TICK,
            "a healthy round trip never reaches the first tick, so it never probes: {elapsed:?}"
        );
        serving.join().expect("the serving listener finishes");
    }
}
