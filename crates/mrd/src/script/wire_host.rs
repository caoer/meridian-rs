//! The script entry in **wire-client mode**: the one door, dialled as an
//! ordinary client (`docs/run-plane.md` § The script entry, "Wire-client mode").
//!
//! Two things live here. [`Door`] is one NDJSON round trip, so the wire client
//! is testable without a daemon and the ops it puts on the socket are
//! observable. [`SocketDoor`] is the production door — a connected daemon
//! socket, bound to a workspace by a v3 `hello`, and the write verbs' door too.
//!
//! *(A third thing lived here until this card's PR 2: `WireHost`, the
//! `effects::ScriptHost` whose `read()` lowered to `toc`/`cat` for the local
//! transaction. Reads are the daemon's now — it evaluates against ONE pinned
//! entry world — so the lowering was deleted with the lane it served.)*
//!
//! **Zero wire delta.** Every op sent through this door is an op the contract
//! already declares: `hello` (§3.2) from the dial itself, `script` (§ A.7) from
//! [`super::cmd`], and the write verbs' own ops (`splice` §4.4, `fingerprint`
//! §4.7) from the modules that share the door. Nothing here invents a request
//! shape, and no response is re-serialized on its way into the trace.
//!
//! **A response line is carried as BYTES.** [`Door::call`] answers the raw
//! response line, not a parsed value, because the commit leg is embedded in
//! `ScriptTrace` verbatim: `serde_json::Value` sorts object keys and normalizes
//! whitespace, which would mint a second commit-fact shape (U3's law).

use std::io::{self, BufReader};
use std::os::unix::net::UnixStream;
use std::path::Path;

use serde::Deserialize;
use serde_json::value::RawValue;
use serde_json::{Value, json};

/// One NDJSON round trip on the one door: the request goes out, the whole
/// response line comes back as its own bytes.
///
/// `Send` because the kernel evaluates on its large-stack thread and the host
/// (holding a door) is moved onto it.
///
/// # Errors
/// Any transport failure — the connection closed, the write failed, the daemon
/// answered nothing.
pub trait Door: Send {
    /// Send `request`, return the response line verbatim.
    ///
    /// # Errors
    /// The transport failed; the script aborts rather than guessing.
    fn call(&mut self, request: &Value) -> io::Result<String>;
}

/// The production door: a connected daemon socket, already bound to a workspace
/// by a v3 `hello`.
pub struct SocketDoor {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
    /// The caps the daemon's hello advertised, retained at connect — the
    /// client half of §3.2 discovery honesty. A write verb consults these
    /// before sending a field the daemon never negotiated (the strict wall
    /// would refuse it; the client refuses first, with a teaching).
    caps: Vec<String>,
}

impl SocketDoor {
    /// Dial `socket` and bind the connection to `workspace` with the §3.2 hello
    /// frame — proto 1, contract v3, copied from the one the read verbs send so
    /// the two clients cannot negotiate differently.
    ///
    /// **The connection carries the wall clock** (§ Where the budgets bind,
    /// layer 2). A socket with no timeout parks the process forever on a daemon
    /// that accepts a frame and never answers, and a parked process never
    /// reaches the deadline check that would have refused: the check above the
    /// socket bounds the number of calls, and only the socket bounds one call.
    /// One round trip may therefore not exceed the entry's WHOLE wall clock,
    /// which is the loosest bound that is still a bound.
    ///
    /// **That bound is the SCRIPT entry's, and the write verbs share only the
    /// hello.** They dial this door too and carry no budget, so their round
    /// trips go through [`Self::call_until_answered`], where a timeout tick is
    /// a notice rather than a verdict — the timeouts set here still bound the
    /// handshake below (a daemon that will not greet is down; nothing was
    /// sent).
    ///
    /// # Errors
    /// The socket refuses the connection, the transport fails, or the daemon
    /// refuses the handshake.
    pub fn connect(socket: &Path, workspace: &Path) -> io::Result<Self> {
        let stream = UnixStream::connect(socket)?;
        stream.set_read_timeout(Some(super::cmd::WALL_CLOCK))?;
        stream.set_write_timeout(Some(super::cmd::WALL_CLOCK))?;
        let writer = stream.try_clone()?;
        let reader = BufReader::new(stream);
        let mut door = Self {
            writer,
            reader,
            caps: Vec::new(),
        };
        let hello = json!({
            "op": "hello",
            "proto": 1,
            "contract": "v3",
            "workspace": workspace.to_string_lossy(),
        });
        let line = door.call(&hello)?;
        let frame = Frame::parse(&line)?;
        if !frame.ok {
            // The daemon names WHAT refused (code, path, cause) in its error
            // frame; collapsing it to a static string cost a dogfood session a
            // fleet-wide outage nobody could attribute. Carry the frame's error
            // verbatim — RawValue bytes, never re-serialized.
            return Err(io::Error::other(format!(
                "the daemon refused the v3 handshake for this workspace: {}",
                frame
                    .error
                    .map_or_else(|| "(no error body)".to_owned(), |e| e.get().to_owned()),
            )));
        }
        // 0025 socket law: identity equality on the hello frame already in
        // hand (one voice with the read/links lanes). Parsed once, at connect.
        let body: Option<Value> = match frame.body.as_deref() {
            Some(raw) => Some(serde_json::from_str(raw.get()).map_err(io::Error::other)?),
            None => None,
        };
        if let Err(message) = crate::engine::hello_identity_skew(body.as_ref(), socket) {
            return Err(io::Error::other(message));
        }
        door.caps = body
            .as_ref()
            .and_then(|b| b.get("caps"))
            .and_then(Value::as_array)
            .map(|caps| {
                caps.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        Ok(door)
    }

    /// Did the connect-time hello advertise `cap` (§3.2 discovery honesty)?
    #[must_use]
    pub fn has_cap(&self, cap: &str) -> bool {
        self.caps.iter().any(|c| c == cap)
    }
}

impl SocketDoor {
    /// One round trip whose answer is WAITED FOR: the write-verb door (card
    /// `cli-face-put-false-negative`).
    ///
    /// The socket's read timeout is the script host's wall clock
    /// ([`super::cmd::WALL_CLOCK`]), and for a script that is the right bound —
    /// the entry has a budget, a round trip that outlives it is refused. A
    /// write verb has no budget: once its frame is on the wire the daemon is
    /// committing, and the only honest result is the daemon's own answer.
    /// Measured 2026-08-21 (seat 547853b4, sessions root): a splice behind a
    /// slow middleware returned `os error 35` (`EAGAIN` — the read timeout) at
    /// 7 s and exited 1 while the bytes landed seconds later. A caller that
    /// believed it re-sent bytes the engine already held, or reported a landed
    /// write as failed.
    ///
    /// So the timeout ticks are not a verdict here. The first tick prints
    /// `notice` — the caller watching a terminal learns the write is in
    /// flight, not dead — and the read resumes. Bytes of a partial line read
    /// before a tick stay in `line`; `read_line` appends. The wait ends only
    /// with the daemon's answer or a transport loss (EOF, a torn stream), and
    /// THAT is the caller's to render as an unknown outcome.
    ///
    /// # Errors
    /// The write failed, or the connection died before an answer arrived.
    pub(crate) fn call_until_answered(
        &mut self,
        request: &Value,
        notice: &str,
    ) -> io::Result<String> {
        use std::io::{BufRead as _, Write as _};

        let mut frame = serde_json::to_string(request).map_err(io::Error::other)?;
        frame.push('\n');
        self.writer.write_all(frame.as_bytes())?;
        self.writer.flush()?;

        let mut line = String::new();
        let mut noticed = false;
        loop {
            match self.reader.read_line(&mut line) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "daemon closed the connection without a response",
                    ));
                }
                Ok(_) => return Ok(line),
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    if !noticed {
                        eprintln!("{notice}");
                        noticed = true;
                    }
                }
                // No `Interrupted` arm: `read_until` (which `read_line` calls)
                // absorbs EINTR itself, so it cannot surface here.
                Err(e) => return Err(e),
            }
        }
    }
}

impl Door for SocketDoor {
    fn call(&mut self, request: &Value) -> io::Result<String> {
        crate::engine::call_line(&mut self.writer, &mut self.reader, request)
    }
}

/// One response frame, split without disturbing its bytes: `body` and `error`
/// stay [`RawValue`], so whichever one reaches the trace reaches it verbatim.
#[derive(Debug, Deserialize)]
pub(crate) struct Frame {
    #[serde(default)]
    pub(crate) ok: bool,
    #[serde(default)]
    pub(crate) body: Option<Box<RawValue>>,
    #[serde(default)]
    pub(crate) error: Option<Box<RawValue>>,
}

impl Frame {
    /// Split one response line.
    ///
    /// # Errors
    /// The line is not a JSON object — a transport-grade failure.
    pub(crate) fn parse(line: &str) -> io::Result<Self> {
        serde_json::from_str(line).map_err(io::Error::other)
    }

    /// The success body as a parsed value, or a transport error naming what the
    /// daemon answered instead.
    pub(crate) fn body_value(self, op: &str) -> io::Result<Value> {
        if !self.ok {
            // `self.error`, NOT `self.body`: the original match bound its
            // refusal arm to the body slot — always absent on a refusal — so
            // every daemon refusal rendered "(no error body)" and the error
            // frame the daemon actually sent (code, path, cause) was dropped
            // on the floor. Same incident class as the connect-time static
            // string: the daemon names the poison, the operator never sees it.
            return Err(io::Error::other(format!(
                "{op} refused: {}",
                self.error
                    .map_or_else(|| "(no error body)".to_owned(), |e| e.get().to_owned())
            )));
        }
        match self.body {
            Some(body) => serde_json::from_str(body.get()).map_err(io::Error::other),
            None => Err(io::Error::other(format!("{op}: ok frame with no body"))),
        }
    }
}

#[cfg(test)]
mod tests {
    //! The two laws the DIAL carries, each proved against a control run of the
    //! same listener: a socket that accepts and never answers must not park the
    //! process, and a foreign-build daemon is refused at connect.
    //!
    //! They sit here rather than in `tests/` because both drive
    //! [`SocketDoor::connect`] against a hand-rolled listener — the only seam
    //! that can serve a mute hello or a foreign identity. No wall-clock BUDGET
    //! is asserted below; only that a bound fired.
    //!
    //! *(The read lowering's own bounds — the clock checked per ROUND TRIP, the
    //! `file_rev` bracket across a composed read — were measured here too, and
    //! went with `WireHost` in this card's PR 2. The daemon binds the clock at
    //! the read builtin now, and serves every read of one attempt from ONE
    //! pinned entry world, so a face composed from two revisions cannot arise.)*

    use std::io::{BufRead as _, Write as _};
    use std::os::unix::net::UnixListener;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Instant;

    use super::{SocketDoor, io};

    /// **A daemon that accepts and never answers must not park the process.**
    /// The check above the socket bounds the NUMBER of calls; only the socket
    /// bounds one call, and a call that never returns is a call the clock never
    /// gets to check again — the shape that hung the tool with nothing bounding
    /// it.
    ///
    /// No wall-clock BUDGET is asserted: the lower bound only says the deadline
    /// was the thing that fired, and load can lengthen it but never shorten it.
    /// The outer channel timeout exists so a regression FAILS instead of hanging
    /// the suite forever.
    #[test]
    fn a_socket_that_never_answers_fails_the_round_trip_instead_of_parking() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sock = dir.path().join("daemon.sock");

        // Control: a listener that answers the hello. The dial completes.
        let answering = UnixListener::bind(&sock).expect("bind");
        let served = thread::spawn(move || {
            let (stream, _) = answering.accept().expect("accept");
            let mut reader = io::BufReader::new(stream.try_clone().expect("clone"));
            let mut line = String::new();
            reader.read_line(&mut line).expect("read the hello");
            let mut w = stream;
            // The control publishes THIS build's identity: the 0025 socket law
            // refuses an identity-less local hello, and this test measures the
            // deadline, not the law.
            let frame = format!(
                "{{\"ok\":true,\"body\":{{\"proto\":1,\"server\":\"fake\",\"caps\":[],\
                 \"identity\":{{\"build\":\"{}\"}}}}}}\n",
                env!("MRD_BUILD_SHA")
            );
            w.write_all(frame.as_bytes()).expect("answer");
            w.flush().expect("flush");
        });
        SocketDoor::connect(&sock, dir.path()).expect("an answering daemon binds the connection");
        served.join().expect("the control listener finishes");
        std::fs::remove_file(&sock).expect("unlink");

        // Measured: a listener that accepts the connection and answers nothing.
        let mute = UnixListener::bind(&sock).expect("bind");
        let held = thread::spawn(move || {
            let (stream, _) = mute.accept().expect("accept");
            thread::sleep(super::super::cmd::WALL_CLOCK * 3);
            drop(stream);
        });
        let (tx, rx) = mpsc::channel();
        let dialled = sock.clone();
        let workspace = dir.path().to_owned();
        thread::spawn(move || {
            let started = Instant::now();
            let outcome = SocketDoor::connect(&dialled, &workspace);
            let _ = tx.send((outcome.is_err(), started.elapsed()));
        });
        let (refused, elapsed) = rx
            .recv_timeout(super::super::cmd::WALL_CLOCK * 2)
            .expect("connect returned — a socket with no deadline never would");
        assert!(
            refused,
            "a daemon that answers nothing fails the round trip"
        );
        assert!(
            elapsed >= super::super::cmd::WALL_CLOCK,
            "the socket's own deadline is what fired, not something faster: {elapsed:?}"
        );
        held.join().expect("the mute listener finishes");
    }

    /// **The script host refuses a foreign-build daemon at connect** (0025
    /// socket law): the handshake succeeds, the identity does not match, and
    /// the door never opens. One voice with the read/links lanes — both
    /// builds named, the verdict, the remedy.
    #[test]
    fn a_foreign_build_daemon_is_refused_at_connect_and_both_builds_are_named() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sock = dir.path().join("daemon.sock");
        let foreign = "feedfacefeedfacefeedfacefeedfacefeedface";

        let listener = UnixListener::bind(&sock).expect("bind");
        let served = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let mut reader = io::BufReader::new(stream.try_clone().expect("clone"));
            let mut line = String::new();
            reader.read_line(&mut line).expect("read the hello");
            let mut w = stream;
            let frame = format!(
                "{{\"ok\":true,\"body\":{{\"proto\":1,\"server\":\"fake\",\"caps\":[],\
                 \"identity\":{{\"build\":\"{foreign}\"}}}}}}\n"
            );
            w.write_all(frame.as_bytes()).expect("answer");
            w.flush().expect("flush");
        });

        let Err(refusal) = SocketDoor::connect(&sock, dir.path()) else {
            panic!("a foreign-build daemon must not bind the connection")
        };
        served.join().expect("the listener finishes");

        let message = refusal.to_string();
        assert!(
            message.contains(foreign) && message.contains(env!("MRD_BUILD_SHA")),
            "the refusal names BOTH builds: {message}"
        );
        assert!(message.contains("SKEW"), "the one skew voice: {message}");
    }
}
