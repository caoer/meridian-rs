//! **The tick must never land on a byte boundary that costs us the answer.**
//!
//! `wedge::read_line` bounds a read so a wedged daemon cannot park the caller.
//! That bound introduces a boundary the unbounded read never had: the tick
//! fires wherever the daemon happened to stop writing, which on any corpus that
//! is not pure ASCII is routinely *inside* one multi-byte character.
//!
//! The first cut of the module read straight into the caller's `String` across
//! ticks. std's [`std::io::BufRead::read_line`] validates the bytes appended by
//! that call and truncates all of them when the slice ends mid-character — and
//! they are already consumed from the `BufReader`, so they are gone. A live
//! daemon's complete, valid answer was destroyed and reported as
//! `InvalidData`: a wrong verdict of exactly the kind the module exists to
//! abolish, reachable only *because* the read is bounded. The same shape made
//! the returned count the last inner call's bytes rather than the line's,
//! falsifying the drop-in claim.
//!
//! Origin: review probe for PR 234 (seat ca62df43), which demonstrated both
//! reds at head `2d2aa05b`. Promoted here so the boundary stays covered — a
//! future rewrite of the tick loop that reintroduces a per-tick UTF-8 guard
//! turns these red again.

use std::io::{BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use registry::wedge;

/// Longer than two ticks, far under the cap: the split must straddle a tick,
/// and the read must still be alive when the rest arrives.
const STALL: Duration = Duration::from_secs(5);

/// How long a test waits before calling a bounded read unbounded. A BACKSTOP,
/// never a budget — it only decides whether a regression FAILS or hangs the
/// suite forever.
#[allow(clippy::duration_suboptimal_units)]
const NEVER: Duration = Duration::from_secs(120);

/// First connection: write `prefix`, stall past two ticks, write `rest`.
/// Every later connection gets a pong, so the liveness probe keeps the
/// disciplined read alive through the stall — this is a SLOW daemon, provably
/// up, not a dead one.
fn slow_split_server(
    listener: UnixListener,
    prefix: &'static [u8],
    rest: &'static [u8],
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        listener.set_nonblocking(true).expect("nonblocking accept");
        let started = Instant::now();
        let mut first: Option<UnixStream> = None;
        let mut rest_written = false;
        while started.elapsed() < Duration::from_secs(30) {
            match listener.accept() {
                Ok((mut s, _)) => {
                    s.set_nonblocking(false).expect("blocking write half");
                    if first.is_none() {
                        s.write_all(prefix).expect("prefix");
                        s.flush().expect("flush prefix");
                        first = Some(s);
                    } else {
                        let _ = s.write_all(b"{\"status\":\"pong\"}\n");
                        let _ = s.flush();
                    }
                }
                Err(_) => thread::sleep(Duration::from_millis(20)),
            }
            if !rest_written
                && started.elapsed() >= STALL
                && let Some(s) = first.as_mut()
            {
                s.write_all(rest).expect("rest");
                s.flush().expect("flush rest");
                rest_written = true;
            }
            if rest_written && started.elapsed() >= STALL + Duration::from_secs(2) {
                break;
            }
        }
    })
}

/// Run the disciplined read off-thread; answer what it returned plus the line
/// as it stood. The shape of the bug under test is a call that does not
/// return, and a test that shares its thread cannot report that.
fn read_off_thread(
    sock: &std::path::Path,
) -> (Result<usize, (std::io::ErrorKind, String)>, String) {
    let (tx, rx) = mpsc::channel();
    let dialled = sock.to_owned();
    thread::spawn(move || {
        let stream = UnixStream::connect(&dialled).expect("connect");
        wedge::bind(&stream).expect("bind");
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        let outcome = wedge::read_line(&mut reader, &dialled, wedge::WEDGE_CAP, &mut line);
        let _ = tx.send((outcome.map_err(|e| (e.kind(), e.to_string())), line));
    });
    rx.recv_timeout(NEVER)
        .expect("the disciplined read returned")
}

/// CONTROL, and the count contract. An ASCII line split across a tick resumes
/// and completes — and `Ok(n)` is the WHOLE line's bytes, not the last inner
/// read's. Reading straight into the `String` returned 4 here, so a caller
/// branching on the count (`client.rs` and `engine::call` both do) was told a
/// 7-byte answer was a 4-byte one.
#[test]
fn an_ascii_line_split_across_a_tick_is_resumed_and_served_whole() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sock = dir.path().join("daemon.sock");
    let server = slow_split_server(UnixListener::bind(&sock).expect("bind"), b"abc", b"def\n");
    let (outcome, line) = read_off_thread(&sock);
    let read = outcome.expect("the resumed read completes");
    assert_eq!(line, "abcdef\n", "the content resumes across the tick");
    assert_eq!(
        read, 7,
        "drop-in claim: the count is the line's bytes, as BufRead::read_line answers"
    );
    drop(server);
}

/// **THE REGRESSION.** The same split, landing inside one UTF-8 character.
/// The daemon is alive, answers every ping, and delivers a complete valid
/// line — it is merely slow. Anything but that line arriving verbatim means
/// the discipline destroyed a good answer.
#[test]
fn a_line_split_inside_one_utf8_char_across_a_tick_still_arrives() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sock = dir.path().join("daemon.sock");
    // {"body":"<E4>  +  <B8><AD>"}\n — the three bytes of U+4E2D split 1+2.
    let server = slow_split_server(
        UnixListener::bind(&sock).expect("bind"),
        b"{\"body\":\"\xE4",
        b"\xB8\xAD\"}\n",
    );
    let (outcome, line) = read_off_thread(&sock);
    match outcome {
        Ok(read) => {
            assert_eq!(
                line, "{\"body\":\"\u{4E2D}\"}\n",
                "the answer arrives verbatim, character intact"
            );
            assert_eq!(read, line.len(), "the count is the line's bytes");
        }
        Err((kind, message)) => panic!(
            "a live daemon's valid answer was destroyed by the tick — \
             kind={kind:?} message={message:?} line_as_left={line:?}"
        ),
    }
    drop(server);
}

/// The other half of the drop-in claim, and the one edge that is not about
/// ticks: a partial line then EOF answers `Ok(n)` with those bytes in `line`
/// and no trailing newline, exactly as the bare reader does. `client.rs` and
/// `engine::call` branch on `read == 0` to mean "closed without a response";
/// this is what keeps that branch honest.
#[test]
fn a_partial_line_then_eof_answers_the_partial_not_zero() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sock = dir.path().join("daemon.sock");
    let listener = UnixListener::bind(&sock).expect("bind");
    let serving = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        stream.write_all(b"abc").expect("partial");
        stream.flush().expect("flush");
        // Dropped here: the peer closes with no newline ever sent.
    });

    let stream = UnixStream::connect(&sock).expect("connect");
    wedge::bind(&stream).expect("bind");
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    let read = wedge::read_line(&mut reader, &sock, wedge::WEDGE_CAP, &mut line)
        .expect("EOF is not an error");

    assert_eq!(read, 3, "the partial's bytes, not zero");
    assert_eq!(line, "abc", "and they are handed to the caller");
    serving.join().expect("the serving listener finishes");
}
