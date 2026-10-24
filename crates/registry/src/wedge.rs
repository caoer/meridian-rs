//! The timeout discipline for a client talking to the daemon — both halves.
//!
//! **A unix stream with no timeout parks the caller forever.** A daemon that
//! accepts a frame and never answers it — wedged, not gone — is invisible to
//! every check *above* the socket: the caller is blocked inside `read(2)` and
//! never reaches the line that would have given up. Only the socket bounds ONE
//! call, so only the socket can end that wait. The same is true of `write(2)`
//! against a daemon that has stopped DRAINING (§ The write half).
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
//! verdict. What ends the wait:
//!
//! | the connection shows | outcome |
//! |---|---|
//! | the answer | the line |
//! | EOF or a reset | the daemon exited or dropped this call — the transport says so at once |
//! | nothing, cap unspent | keep reading; a slow daemon is not a dead one |
//! | nothing, cap spent | [`io::ErrorKind::TimedOut`] — up, and wedged or stalled |
//!
//! # A held connection is the liveness evidence
//!
//! **A daemon that has exited cannot hold this connection open**: its exit
//! closes every descriptor it had, and the blocked read returns EOF (or the
//! write `EPIPE`) the same instant. So while the connection stays open and
//! silent, the peer is alive by construction, and the only question left is
//! how long to wait for it — which is [`WEDGE_CAP`].
//!
//! A second-connection `ping` cannot improve on that evidence, and as a
//! verdict it was wrong. It used to run on every tick, and a ping that missed
//! [`PROBE_TIMEOUT`] answered "it died mid-request". A daemon swapping or
//! saturated misses that 2 s bound while it is very much alive: measured on
//! devstation-nyc-1, 2026-10-24 02:58Z (ccc-statusd 1.12.173), `mrd sql`
//! refused `NO_VIEW` "died mid-request" after 4.09 s while the engine — same
//! pid before and after, 3.6 GB resident and 3.7 GB in swap — went on serving,
//! and the same call 20 s later succeeded. The probe now runs once, when the
//! cap is spent, and only to word the wedge message: a daemon that still
//! answers pings is wedged on THIS request; one that answers neither is stalled
//! whole (overloaded, swapping, or hung). Neither is absent.
//!
//! # The write half
//!
//! **A stalled DRAIN is the same daemon, seen from the other side**, and until
//! 2026-08-24 it was the one place this module still drew the verdict it exists
//! to abolish. [`bind`] put the whole cap on the write as a flat socket
//! timeout, so a daemon that stopped reading — busy, saturated, wedged mid-put —
//! surfaced as a bare `WouldBlock`: "Resource temporarily unavailable (os error
//! 35/11)", with no wedged-vs-absent separation, no remedy, and no statement of
//! what happened to the bytes. That is the exact message class card
//! `dial-eagain-under-pipeline-load` was opened over, arriving by the other
//! half.
//!
//! The justification for the flat cap was that "a timeout mid-frame leaves a
//! partial line on the wire, which is unrecoverable", so a write timeout could
//! only mean a dead peer. **Measured 2026-08-24 (macOS, `SO_SNDTIMEO` 2 s, an
//! 8 MiB body against a listener that accepts and never reads), that premise is
//! false**: the first `write(2)` returned `Ok(8192)` — a PARTIAL COUNT, the
//! bytes the kernel took — and every later one returned `WouldBlock` with the
//! transferred total unchanged. The kernel says exactly how far it got, so
//! resuming at that offset loses no byte and duplicates none. `write_all` is
//! what makes the partial unrecoverable: it discards the count on error.
//!
//! So [`write_all`] is the write-half twin of [`read_line`] — the socket
//! timeout is the TICK, the held connection is the liveness evidence (a dead
//! peer answers the next `write(2)` with `EPIPE`), and the cap ends a drain
//! that never resumes. It has one thing the read half does not: **the frame is not
//! whole on the wire, so nothing was committed** — an NDJSON request the daemon
//! never received to its newline cannot have been acted on. Every verdict below
//! says so, because "the write failed" and "the outcome is unknown" send a
//! caller to opposite acts.
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

/// How long a liveness probe may take before it counts as unanswered. Its own
/// connection and its own bound: probing through [`crate::Client`] would
/// re-enter this discipline and, on a wedged daemon, ask the wedged question
/// twice. An unanswered probe is never a death verdict (§ A held connection is
/// the liveness evidence).
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// How often a blocked read or write wakes to check the cap.
///
/// NOT a budget — an expired tick answers no question by itself, which is the
/// whole point (§ A tick is a question). A dead daemon is found by the
/// transport, not by the tick, so the tick costs a dead peer nothing.
pub const TICK: Duration = Duration::from_secs(2);

/// The backstop for the one case the held connection cannot settle: a daemon
/// that is alive and never answers THIS request — wedged on it, or stalled
/// whole. A bug or an overload worth naming rather than a wait worth extending.
///
/// Deliberately far above any answer a healthy daemon produces — and above
/// the multi-second ops a swapping one still completes — because a number is
/// the wrong instrument here and this one is only the floor under the right
/// one.
// `Duration::from_mins` not const-stable at MSRV 1.96.
#[allow(clippy::duration_suboptimal_units)]
pub const WEDGE_CAP: Duration = Duration::from_secs(60);

/// Put `stream` under the discipline: **both halves tick at [`TICK`]**.
///
/// No cap here on purpose. The cap is a property of the QUESTION, not of the
/// connection — a liveness probe that has waited two seconds has already
/// answered, while a write verb past the wire has no budget at all — so it is
/// passed per call, to [`read_line`] and [`write_all`]. Taking a `cap`
/// parameter it then put on only one half is what let the write half keep the
/// flat bound after the read half stopped drawing verdicts from a constant.
///
/// **A bare `write_all`/`read_line` on a stream bound here is now a bug**: it
/// gets a 2-second bound and a raw `WouldBlock`, which is the defect this
/// module exists to abolish. Reach the daemon through [`read_line`] and
/// [`write_all`].
///
/// Call this BEFORE [`UnixStream::try_clone`]: `SO_RCVTIMEO`/`SO_SNDTIMEO` are
/// socket-level, so the clone inherits them and one call covers both halves.
///
/// # Errors
///
/// The socket rejected the option — the connection is unusable.
pub fn bind(stream: &UnixStream) -> io::Result<()> {
    stream.set_read_timeout(Some(TICK))?;
    stream.set_write_timeout(Some(TICK))?;
    Ok(())
}

/// Does the daemon at `socket` answer a registry `ping` within
/// [`PROBE_TIMEOUT`]? A `true` proves the daemon is up; a `false` proves
/// nothing about a daemon that still holds a connection open — it may simply
/// be too loaded to answer in time — so [`read_line`] and [`write_all`] ask it
/// only to word their cap-spent message.
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

/// The cap-spent message's account of a daemon that still holds the
/// connection: one probe, asked only now, decides which kind of alive it is.
fn alive_state(socket: &Path) -> &'static str {
    if answers_ping(socket) {
        "is up (it is still answering liveness probes)"
    } else {
        "is alive (it still holds this connection open) but answers no liveness probe either \
         — stalled whole: overloaded, swapping, or hung,"
    }
}

/// Write `frame` whole under the discipline — the write-half twin of
/// [`read_line`]. Drop-in for [`io::Write::write_all`] followed by a flush,
/// with the tick loop around it.
///
/// # The offset is what makes the tick lossless
///
/// A tick lands wherever the daemon stopped draining, so it lands mid-frame
/// routinely on any body worth stalling over. [`io::Write::write_all`] cannot
/// be used across ticks: on error it discards how many bytes it had already
/// handed the kernel, so a caller cannot know where to resume — which is
/// precisely why the flat write cap looked unavoidable. A single
/// [`io::Write::write`] does report it: measured 2026-08-24, `Ok(8192)` when
/// the kernel took a partial slice at the timeout, then `WouldBlock` with the
/// total unchanged. So the bytes are tracked here, by offset, and a tick never
/// re-sends a byte the daemon already has (a duplicated slice is a corrupt
/// frame, not a retry).
///
/// The frame is sent ONCE. A tick resumes it; it never restarts it.
///
/// # Errors
///
/// On every error the frame is incomplete on the wire — no newline arrived, so
/// the daemon has not acted on it and **nothing was committed**. Each message
/// says so.
///
/// - [`io::ErrorKind::TimedOut`] — the connection stayed open for the whole
///   `cap` and the daemon never drained the rest: alive, and wedged or stalled.
/// - A dead daemon's `EPIPE`/`ECONNRESET`, verbatim and at once.
/// - [`io::ErrorKind::WriteZero`] — the socket accepted nothing and reported
///   success, which cannot make progress.
/// - Any other transport failure, verbatim.
pub fn write_all(
    writer: &mut impl Write,
    socket: &Path,
    cap: Duration,
    frame: &[u8],
) -> io::Result<()> {
    let started = Instant::now();
    let mut sent = 0usize;
    let total = frame.len();
    while sent < total {
        match writer.write(&frame[sent..]) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    format!(
                        "the socket accepted none of the remaining {} bytes of this \
                         {total}-byte request and reported success — the frame cannot be \
                         completed, so it never reached the daemon whole and nothing was \
                         committed",
                        total - sent
                    ),
                ));
            }
            // Whatever the kernel took is already on the wire; the offset is
            // what keeps the resumption lossless.
            Ok(n) => sent += n,
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                // The connection is still open (a dead peer answers `EPIPE`, the
                // arm below), so the daemon is alive: only the cap ends this wait.
                let waited = started.elapsed();
                if waited >= cap {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        format!(
                            "the daemon {} and has not drained this {total}-byte request in \
                             {waited:?} ({sent} bytes taken) — it is wedged, not absent, so \
                             restarting the client will not help; restart the daemon. The frame \
                             never reached it whole, so nothing was committed",
                            alive_state(socket)
                        ),
                    ));
                }
            }
            // `Write::write` does NOT absorb EINTR — unlike `read_until`, which
            // is why `read_line` above has no such arm. `write_all` handles it
            // internally and this loop replaces `write_all`, so the arm moves
            // here or a signal becomes a spurious transport failure.
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    // A no-op on a bare `UnixStream` (there is no user-space buffer), kept so
    // this is a drop-in for the `write_all` + `flush` pair it replaces.
    writer.flush()
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
/// - [`io::ErrorKind::TimedOut`] — the connection stayed open for the whole
///   `cap` and the daemon never answered this request: alive, and wedged or
///   stalled. A daemon that dies mid-request is not an error here: its exit
///   closes the connection, and that is `Ok(0)` (or a reset) at once.
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
                // `bytes`. That is what makes the resumption lossless. The
                // connection is still open, so the daemon is alive: only the cap
                // ends this wait (§ A held connection is the liveness evidence).
                let waited = started.elapsed();
                if waited >= cap {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        format!(
                            "the daemon {} and has not answered this request in {waited:?} — \
                             it is wedged, not absent, so restarting the client will not help; \
                             restart the daemon",
                            alive_state(socket)
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
        BufRead, BufReader, Duration, Instant, PROBE_TIMEOUT, TICK, UnixStream, Write, io,
        read_line,
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
                                // Read the ping, then pong — the daemon's
                                // order. Ponging first and dropping the stream
                                // races `answers_ping`: preempted between
                                // `connect` and `write` past an accept poll, it
                                // writes into a closed peer and takes `EPIPE`.
                                // Blocking, because the accepted stream
                                // inherits the listener's non-blocking flag on
                                // BSD/macOS.
                                let _ = stream.set_nonblocking(false);
                                let _ = stream.set_read_timeout(Some(PROBE_TIMEOUT));
                                let _ = BufReader::new(&stream).read_line(&mut String::new());
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
            super::bind(&stream).expect("bind the discipline");
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

    impl Mute {
        /// A daemon stalled whole, then back: the FIRST connection (the
        /// request) is held for `delay` and then gets `answer` — or, with
        /// `None`, is closed, which is what a daemon's exit does to it. Every
        /// other connection (a liveness probe) is held and never answered, so
        /// for the whole `delay` the daemon misses every probe it is sent.
        fn first_after(
            listener: UnixListener,
            delay: Duration,
            answer: Option<&'static [u8]>,
        ) -> Self {
            let done = Arc::new(AtomicBool::new(false));
            let flag = Arc::clone(&done);
            listener
                .set_nonblocking(true)
                .expect("poll instead of blocking, so the flag is checked");
            let thread = thread::spawn(move || {
                let mut first: Option<(UnixStream, Instant)> = None;
                let mut settled = false;
                let mut held = Vec::new();
                while !flag.load(Ordering::SeqCst) {
                    if let Ok((stream, _)) = listener.accept() {
                        if first.is_none() && !settled {
                            first = Some((stream, Instant::now()));
                        } else {
                            held.push(stream);
                        }
                        continue;
                    }
                    if let Some((stream, since)) = first.take() {
                        if since.elapsed() < delay {
                            first = Some((stream, since));
                        } else {
                            settled = true;
                            if let Some(bytes) = answer {
                                let mut w = stream;
                                let _ = w.write_all(bytes);
                                let _ = w.flush();
                                held.push(w);
                            }
                            // `None`: the stream drops here — the exit.
                        }
                    }
                    thread::sleep(Duration::from_millis(20));
                }
            });
            Mute {
                done,
                thread: Some(thread),
            }
        }
    }

    /// **A daemon that accepts, answers nothing, and cannot be pinged must not
    /// park the caller** — and must not be called dead either: it still holds
    /// the connection, which a daemon that exited cannot do. The cap ends the
    /// wait, and the message says the daemon is alive and stalled whole.
    ///
    /// Only a LOWER bound is asserted on the elapsed time: it says the cap is
    /// what fired rather than something cheaper, and load can lengthen a wait
    /// but never shorten it.
    #[test]
    fn a_mute_unpingable_daemon_spends_the_cap_and_is_called_stalled_not_dead() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sock = dir.path().join("daemon.sock");
        let _mute = Mute::holding(UnixListener::bind(&sock).expect("bind"));
        let cap = TICK + Duration::from_millis(200);

        let (outcome, elapsed) = read_off_thread(&sock, cap);
        let (kind, message) = outcome.expect_err("a mute daemon does not answer");
        assert_eq!(
            kind,
            io::ErrorKind::TimedOut,
            "a listener that holds the connection open has not exited, whatever its probes \
             say: {message}"
        );
        assert!(
            message.contains("answers no liveness probe") && message.contains("not absent"),
            "the message names the stall and rules out the absence: {message}"
        );
        assert!(
            elapsed >= cap,
            "the cap is what ended the wait, not something faster: {elapsed:?}"
        );
    }

    /// **A live daemon too loaded to answer its probes is waited for, not
    /// called dead.** Card `mrd-client-reads-slow-engine-as-dead`: on
    /// devstation-nyc-1 a swapping engine missed one 2 s ping and `mrd sql`
    /// refused "it died mid-request" after 4.09 s, while the engine served on.
    /// Here the request is answered after more than one tick plus one probe
    /// timeout, and every probe sent meanwhile goes unanswered — the old
    /// verdict's exact trigger.
    #[test]
    fn a_live_daemon_that_misses_its_probes_is_waited_for_not_called_dead() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sock = dir.path().join("daemon.sock");
        let slow = TICK + PROBE_TIMEOUT + Duration::from_secs(1);
        let _slow = Mute::first_after(
            UnixListener::bind(&sock).expect("bind"),
            slow,
            Some(b"{\"status\":\"pong\"}\n"),
        );

        let (outcome, elapsed) = read_off_thread(&sock, super::WEDGE_CAP);
        let read = outcome.unwrap_or_else(|(kind, message)| {
            panic!("a daemon that answers after {slow:?} was given up on ({kind:?}): {message}")
        });
        assert!(read > 0, "the answer is bytes, not EOF");
        assert!(
            elapsed >= slow,
            "the answer really did come after the probes went unanswered: {elapsed:?}"
        );
    }

    /// **A daemon that dies mid-request is still reported at once** — by the
    /// transport, not by a probe. Its exit closes the connection, the read
    /// returns EOF the same instant, and nothing waits for the cap.
    #[test]
    fn a_daemon_that_dies_mid_request_ends_the_read_at_eof_without_the_cap() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sock = dir.path().join("daemon.sock");
        let dies_after = TICK + Duration::from_millis(500);
        let _dies = Mute::first_after(UnixListener::bind(&sock).expect("bind"), dies_after, None);

        let (outcome, elapsed) = read_off_thread(&sock, super::WEDGE_CAP);
        let read = outcome.unwrap_or_else(|(kind, message)| {
            panic!("a closed connection is EOF for the caller to read, not {kind:?}: {message}")
        });
        assert_eq!(read, 0, "the exit reaches the caller as EOF");
        assert!(
            elapsed < super::WEDGE_CAP / 2,
            "the death was seen when it happened, not when the cap ran out: {elapsed:?}"
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
        super::bind(&stream).expect("bind the discipline");
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
