//! The CLI write transport: every routine write verb dials the running daemon
//! over authenticated IPC (hello + the socket-law identity check). There is no
//! in-process publication fallback — a down daemon is a taught refusal, never a
//! local `splice` / `remove`.
//!
//! Law: merged-plan §4.7 / §4.11 CLI row (pre-merge ruling 4, "No Daemon Route,
//! B"); authority contract §15. The daemon write door is already the authority;
//! this module is the client half.

use std::path::Path;

use registry::Client;
use serde_json::Value;
use wire::ErrorBody;

use crate::script::wire_host::{Frame, SocketDoor, WriteHalt};
use crate::{Fail, engine};

/// The one teaching every write verb prints when the authority cannot be
/// reached. Face and docs share this spelling so a test can match it.
pub(crate) const DAEMON_DOWN: &str = "\
mrd write verbs talk to the running daemon over authenticated IPC; there is no \
direct-publication fallback. The daemon must come up (`mrd daemon`, or the next \
call auto-spawns it). Scripts that used to write with no daemon now need the \
daemon up.";

/// Dial the workspace authority for a write. Auto-spawns the resident daemon
/// the same way the read path does, then refuses — never degrades — if it
/// does not answer.
pub(crate) fn connect(workspace: &Path) -> Result<SocketDoor, Fail> {
    let client = Client::from_default().map_err(|e| {
        Fail::tool(format!(
            "{DAEMON_DOWN} (cannot resolve the daemon socket: {e})"
        ))
    })?;
    engine::ensure_daemon(&client).map_err(|e| {
        Fail::tool(format!(
            "{DAEMON_DOWN} ({e}). {}",
            engine::degrade_reason().unwrap_or_else(|| {
                "Start the daemon, or fix the socket path (XDG_RUNTIME_DIR / HOME / sun_path)."
                    .to_owned()
            })
        ))
    })?;
    // A write verb carries no budget, so the handshake gets the general
    // backstop, never the script entry's wall clock: a busy daemon's slow greet
    // is not this verb's deadline (card `dial-eagain-under-pipeline-load`).
    SocketDoor::connect(
        client.socket_path(),
        workspace,
        crate::script::wire_host::GREET_CAP,
    )
    .map_err(|e| Fail::tool(e.teach(DAEMON_DOWN)))
}

/// Printed to stderr once when the daemon has held a write past the socket's
/// read timeout (the script host's 7 s wall clock, which this door shares).
/// The wait goes on: a write on the wire is a write the daemon is committing,
/// and the only honest result is its answer (card
/// `cli-face-put-false-negative`: before this, the tick returned `os error
/// 35` and exit 1 while the bytes landed).
pub(crate) const STILL_WAITING: &str = "\
mrd: the daemon has not answered this write yet — it is in flight (an armed \
middleware may be running) and may already be committed; still waiting for \
the daemon's answer. Do not interrupt and re-send: a re-send of landed bytes \
is a second write. If you must stop, read the file back before any retry.";

/// One write op on an open door. Success is the projected body; a daemon
/// refusal is the engine's [`ErrorBody`] so the verb's `--json` face stays
/// one envelope.
///
/// The answer is waited for — never abandoned at the wall clock. A transport
/// loss after the frame went out (the daemon closed the connection, the
/// stream tore) is reported as what it is: the outcome is UNKNOWN, the write
/// may have committed, read before any re-send. It is never rendered as a
/// failed write — that was the false negative.
///
/// **A frame that never finished going out is the other half of that honesty**
/// ([`WriteHalt`]). It used to take the UNKNOWN wording too, which is the false
/// negative inverted: an operator was sent to distrust a file the daemon had
/// not been asked to touch. A stalled drain leaves no newline on the wire, so
/// no request was ever parsed — that is a KNOWN nothing, and it says so.
pub(crate) fn call(door: &mut SocketDoor, request: &Value) -> Result<Value, Box<ErrorBody>> {
    let line = door
        .call_until_answered(request, STILL_WAITING)
        .map_err(|halt| {
            let mut err = ErrorBody::new(wire::ErrorCode::IoError);
            let target = request
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or("the target");
            err.message = Some(match &halt {
                WriteHalt::AnswerLost(e) => format!(
                    "the answer to the write on {target} was lost ({e}) — the frame had reached \
                     the daemon, so the write may have committed before the loss: whether the \
                     file carries it is UNKNOWN. Read the file back and check for your edit \
                     BEFORE any re-send; a re-send of landed bytes is a second write, and a lost \
                     answer is not a failed write."
                ),
                WriteHalt::NotSent(e) => format!(
                    "the write on {target} was never delivered ({e}). The request frame did not \
                     reach the daemon whole — no newline arrived, so the daemon never parsed a \
                     request and NOTHING was committed: {target} is untouched. This is not the \
                     lost-answer case; no read-back is owed, and a retry sends the write for the \
                     first time, not a second."
                ),
            });
            err.cause = Some(halt.error().to_string());
            Box::new(err)
        })?;
    let frame = Frame::parse(&line).map_err(|e| {
        let mut err = ErrorBody::new(wire::ErrorCode::IoError);
        err.message = Some(format!("the daemon's write answer would not parse: {e}"));
        err.cause = Some(e.to_string());
        Box::new(err)
    })?;
    if frame.ok {
        let body = frame.body_value("write").map_err(|e| {
            let mut err = ErrorBody::new(wire::ErrorCode::IoError);
            err.message = Some(e.to_string());
            Box::new(err)
        })?;
        // A write reply names what it armed or removed. Hello/identity
        // bodies are ok:true too — treating those as a commit was the
        // silent-success class (pin printed "?" and exited 0).
        let armed_empty = body
            .pointer("/armed/edits")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty);
        if (body.get("armed").is_none()
            && body.get("file_rev_before").is_none()
            && body.get("file_rev_after").is_none())
            || (armed_empty && request.get("pin").is_none())
        {
            let mut err = ErrorBody::new(wire::ErrorCode::IoError);
            err.message = Some(format!(
                "the daemon answered ok but the body is not a write (request={request} body={body})"
            ));
            return Err(Box::new(err));
        }
        return Ok(body);
    }
    if let Some(raw) = frame.error {
        // The daemon's v3 session projects error codes (root_mismatch →
        // fingerprint_mismatch). ErrorBody is the v2 enum; unproject before
        // decode so a world-guard refusal is not an io_error parse miss.
        let mut raw_val: Value = serde_json::from_str(raw.get()).map_err(|e| {
            let mut err = ErrorBody::new(wire::ErrorCode::IoError);
            err.message = Some(format!(
                "the daemon refused the write but the error frame would not parse: {e}"
            ));
            Box::new(err)
        })?;
        if let Some(obj) = raw_val.as_object_mut()
            && let Some(Value::String(code)) = obj.get("code")
        {
            let v2 = match code.as_str() {
                "fingerprint_mismatch" => Some("root_mismatch"),
                "fingerprint_unknown" => Some("root_unknown"),
                _ => None,
            };
            if let Some(v2) = v2 {
                obj.insert("code".into(), Value::String(v2.into()));
            }
        }
        let error: ErrorBody = serde_json::from_value(raw_val).map_err(|e| {
            let mut err = ErrorBody::new(wire::ErrorCode::IoError);
            err.message = Some(format!(
                "the daemon refused the write but the error frame would not parse: {e}"
            ));
            Box::new(err)
        })?;
        Err(Box::new(error))
    } else {
        let mut err = ErrorBody::new(wire::ErrorCode::IoError);
        err.message = Some(format!(
            "the daemon refused the write with no error body: {}",
            line.trim()
        ));
        Err(Box::new(err))
    }
}

/// Lift a v2-shaped splice/remove body into the v3 vocabulary the CLI speaks.
/// Idempotent on a body the daemon already projected.
pub(crate) fn project_body(body: &Value) -> Value {
    let mut frame = serde_json::json!({ "body": body });
    wire_serve::rev::project_response(&mut frame);
    frame
        .as_object_mut()
        .and_then(|obj| obj.remove("body"))
        .unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    //! **The two halts have opposite commit facts, and nothing held them
    //! apart.** PR 236 split [`WriteHalt`] into `NotSent` and `AnswerLost`
    //! precisely because one voice for both sent an operator to distrust a file
    //! the daemon was never asked to touch. Nothing then tested the split:
    //! measured at `77a95b70`, `NotSent`/`AnswerLost` appear in no `.rs` file in
    //! this repository outside `script/wire_host.rs` and this module's parent —
    //! not in `crates/mrd/tests/` (98 integration files), not in any of the 25
    //! `#[cfg(test)]` modules under `crates/mrd/src/`.
    //!
    //! `tests/put_waits_for_the_daemon.rs` does drive a lost answer end to end
    //! (`Splice::Vanish`) and asserts the UNKNOWN wording, so a one-sided SWAP
    //! of the render arms already reds there. What it cannot see is the
    //! dangerous direction: **a collapse onto the lost-answer voice** leaves
    //! that test green while restoring the exact false negative 236 closed. It
    //! also never exercises the send half at all — no test in the tree makes a
    //! frame fail to go out.
    //!
    //! So these four bite in two layers, and a swap or a collapse reds at least
    //! one of them whichever way it is done:
    //!
    //! - **Classification** ([`SocketDoor::call_until_answered`]) — which arm a
    //!   given transport failure becomes.
    //! - **Rendering** ([`super::call`]) — what each arm then TELLS the
    //!   operator. Each test asserts its own arm's commit facts AND the absence
    //!   of the other's, so one message cannot satisfy both.
    //!
    //! Harness: a hand-rolled listener that completes the identity-matched v3
    //! hello (the shape `script::wire_host`'s own tests drive) and then fails
    //! the round trip in one of the two ways. Ordering is handshake-then-fail
    //! by construction — the test signals the listener only after `connect`
    //! has returned, and joins it before writing — so neither arm is reached
    //! by a race.

    use std::io::{BufRead as _, BufReader, Write as _};
    use std::net::Shutdown;
    use std::os::unix::net::UnixListener;
    use std::path::Path;
    use std::sync::mpsc::{self, Sender};
    use std::thread::{self, JoinHandle};

    use serde_json::{Value, json};

    use super::call;
    use crate::script::wire_host::{GREET_CAP, SocketDoor, WriteHalt};

    /// A hello answer carrying THIS build's identity — the 0025 socket law
    /// refuses an identity-less local hello, and these tests measure the halt
    /// arms, not that law.
    fn hello_answer() -> String {
        format!(
            "{{\"ok\":true,\"body\":{{\"proto\":1,\"server\":\"fake\",\"caps\":[],\
             \"identity\":{{\"build\":\"{}\"}}}}}}\n",
            env!("MRD_BUILD_SHA")
        )
    }

    /// The request under test. The body is a mebibyte so the send cannot be
    /// swallowed whole by a socket buffer on any platform: with the peer gone,
    /// `wedge::write_all` must reach a transport error rather than return `Ok`
    /// and let the failure surface on the READ half — which would be the other
    /// arm, and would make the send-half test prove nothing.
    fn big_request() -> Value {
        json!({"op": "splice", "path": "doc.md", "body": "x".repeat(1024 * 1024)})
    }

    /// A listener that completes the hello and then **closes the connection**
    /// on the test's signal. Send on the returned channel once `connect` has
    /// returned, then `join` the handle: after that the peer is provably gone,
    /// so the next frame cannot leave whole.
    fn daemon_that_closes_after_the_hello(sock: &Path) -> (JoinHandle<()>, Sender<()>) {
        let listener = UnixListener::bind(sock).expect("bind");
        let (tx, rx) = mpsc::channel::<()>();
        let handle = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));
            let mut line = String::new();
            reader.read_line(&mut line).expect("read the hello");
            let mut w = stream.try_clone().expect("clone");
            w.write_all(hello_answer().as_bytes()).expect("answer");
            w.flush().expect("flush");
            rx.recv()
                .expect("the test signals once the handshake is done");
            stream.shutdown(Shutdown::Both).expect("shutdown");
            drop(reader);
            drop(w);
            drop(stream);
            drop(listener);
        });
        (handle, tx)
    }

    /// A listener that completes the hello, **reads the write frame**, and then
    /// vanishes without answering — the frame reached the daemon whole and the
    /// answer never came back.
    fn daemon_that_vanishes_after_reading_the_frame(sock: &Path) -> JoinHandle<()> {
        let listener = UnixListener::bind(sock).expect("bind");
        thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));
            let mut line = String::new();
            reader.read_line(&mut line).expect("read the hello");
            let mut w = stream.try_clone().expect("clone");
            w.write_all(hello_answer().as_bytes()).expect("answer");
            w.flush().expect("flush");
            // The whole request line, newline included: reading it to
            // completion is what makes this the DELIVERED case.
            let mut frame = String::new();
            reader.read_line(&mut frame).expect("read the write frame");
            assert!(
                frame.ends_with('\n'),
                "the fixture must consume a WHOLE frame or it is testing the other arm"
            );
        })
    }

    /// **A frame that never went out is `NotSent`.** The peer is provably gone
    /// before the send starts, so no newline can have arrived and the daemon
    /// cannot have parsed a request.
    ///
    /// Red if `call_until_answered`'s send half is mapped to `AnswerLost`
    /// (`script/wire_host.rs`, the `map_err` on `wedge::write_all`), and red if
    /// the arms are collapsed to whichever one `AnswerLost` is.
    #[test]
    fn a_frame_that_never_leaves_is_classified_not_sent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sock = dir.path().join("daemon.sock");
        let (served, go) = daemon_that_closes_after_the_hello(&sock);

        let mut door = SocketDoor::connect(&sock, dir.path(), GREET_CAP)
            .expect("the hello completes before the connection is closed");
        go.send(()).expect("release the listener");
        served.join().expect("the listener closes the connection");

        let Err(halt) = door.call_until_answered(&big_request(), "notice") else {
            panic!("a write into a closed connection cannot succeed")
        };
        assert!(
            matches!(halt, WriteHalt::NotSent(_)),
            "a frame that could not go out is NotSent — nothing was committed. \
             Classified instead as {halt:?}, which claims the daemon received it"
        );
    }

    /// **A delivered frame whose answer is lost is `AnswerLost`.** The fixture
    /// reads the whole request line and only then vanishes, so the daemon
    /// demonstrably had the frame: what landed is genuinely unknown.
    ///
    /// Red if the read half's EOF is mapped to `NotSent`, and red if the arms
    /// are collapsed to whichever one `NotSent` is. Together with its
    /// neighbour, no single arm can satisfy both.
    #[test]
    fn a_delivered_frame_whose_answer_is_lost_is_classified_answer_lost() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sock = dir.path().join("daemon.sock");
        let served = daemon_that_vanishes_after_reading_the_frame(&sock);

        let mut door = SocketDoor::connect(&sock, dir.path(), GREET_CAP).expect("the hello lands");

        let Err(halt) = door.call_until_answered(&big_request(), "notice") else {
            panic!("a daemon that never answers cannot yield a response line")
        };
        served.join().expect("the listener finishes");
        assert!(
            matches!(halt, WriteHalt::AnswerLost(_)),
            "a frame the daemon read and never answered is AnswerLost — the outcome is unknown. \
             Classified instead as {halt:?}, which claims nothing was committed"
        );
    }

    /// **The `NotSent` face reports a KNOWN nothing.** The operator is told the
    /// file is untouched and owed no read-back — the opposite of the
    /// lost-answer remedy, and the reason the arms exist.
    ///
    /// The forbidden list is the bite: it is exactly the lost-answer voice, so
    /// this reds if the render arms in `write_ipc::call` are swapped AND if
    /// they are collapsed onto the `AnswerLost` wording — the collapse
    /// `put_waits_for_the_daemon.rs` cannot see, and the precise regression
    /// PR 236 closed.
    #[test]
    fn the_not_sent_face_says_nothing_was_committed_and_never_the_unknown_voice() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sock = dir.path().join("daemon.sock");
        let (served, go) = daemon_that_closes_after_the_hello(&sock);

        let mut door = SocketDoor::connect(&sock, dir.path(), GREET_CAP).expect("the hello lands");
        go.send(()).expect("release the listener");
        served.join().expect("the listener closes the connection");

        let Err(err) = call(&mut door, &big_request()) else {
            panic!("a write into a closed connection cannot succeed")
        };
        let message = err.message.unwrap_or_default();
        for want in [
            "was never delivered",
            "NOTHING was committed",
            "is untouched",
            "no read-back is owed",
            "not the lost-answer case",
        ] {
            assert!(
                message.contains(want),
                "{want:?} missing — an undelivered write must report a KNOWN nothing:\n{message}"
            );
        }
        for forbidden in ["UNKNOWN", "Read the file back", "may have committed"] {
            assert!(
                !message.contains(forbidden),
                "{forbidden:?} is the LOST-ANSWER voice: it sends an operator to re-read a file \
                 the daemon was never asked to touch — the false negative inverted:\n{message}"
            );
        }
    }

    /// **The `AnswerLost` face reports an UNKNOWN outcome.** The write may have
    /// landed, so the remedy is a read-back before any re-send.
    ///
    /// The forbidden list is this test's bite in the other direction: a
    /// collapse onto the `NotSent` wording would tell an operator that a write
    /// which may be on disk is not, and reds here.
    #[test]
    fn the_answer_lost_face_says_unknown_and_never_claims_the_file_is_untouched() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sock = dir.path().join("daemon.sock");
        let served = daemon_that_vanishes_after_reading_the_frame(&sock);

        let mut door = SocketDoor::connect(&sock, dir.path(), GREET_CAP).expect("the hello lands");

        let Err(err) = call(&mut door, &big_request()) else {
            panic!("a daemon that never answers cannot yield a response line")
        };
        served.join().expect("the listener finishes");
        let message = err.message.unwrap_or_default();
        for want in [
            "was lost",
            "may have committed",
            "UNKNOWN",
            "Read the file back",
        ] {
            assert!(
                message.contains(want),
                "{want:?} missing — a lost answer leaves the outcome unknown:\n{message}"
            );
        }
        for forbidden in [
            "was never delivered",
            "NOTHING was committed",
            "is untouched",
            "no read-back is owed",
        ] {
            assert!(
                !message.contains(forbidden),
                "{forbidden:?} claims a commit fact nobody has after a lost answer — the write \
                 may be on disk:\n{message}"
            );
        }
    }
}
