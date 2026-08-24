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
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::value::RawValue;
use serde_json::{Value, json};

/// The backstop on the v3 handshake — NOT the meter. The wait normally ends
/// one of two ways: the daemon greets, or it stops answering liveness probes
/// ([`daemon_answers_ping`]). This cap only catches the third case, a daemon
/// that answers pings forever and never greets — a wedged daemon, which is a
/// bug worth naming rather than a load condition worth waiting out. It is
/// deliberately far above any greet a healthy daemon produces, because a
/// number is the wrong instrument here and this one is only the floor under
/// the right one.
pub const GREET_CAP: Duration = Duration::from_mins(1);

/// How long a liveness probe may take before we call the daemon unreachable.
///
/// Its own bound rather than [`registry::Client`]'s. `Client` is no longer
/// unbounded — every read it does now carries the wedge discipline
/// ([`registry::wedge`]) — but that discipline is built ON a liveness probe, so
/// probing through `Client` would re-enter it and ask the wedged question
/// twice. This door also owns a script entry's own wall clock and its own
/// [`DialFailure`] arms, which `Client` knows nothing about.
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// Printed once when the daemon has held the handshake past the socket's read
/// timeout while still answering liveness probes.
const GREET_NOTICE: &str = "\
mrd: the daemon has not answered the handshake yet — it is up (it is still \
answering liveness probes) and busy, so this is a slow greet, not an absent \
daemon; still waiting. Nothing has been written.";

/// The neutral absent-daemon teaching, for a caller with no voice of its own
/// (`Display`, and the `io::Error` lift). A caller that HAS one passes it to
/// [`DialFailure::teach`] instead.
const ABSENT: &str = "the daemon could not be reached";

/// Which half of the dial failed.
///
/// The halves have OPPOSITE remedies and were rendered with one voice: every
/// failure below reached the caller as "cannot dial the daemon", whose
/// teaching is "the daemon must come up". Measured 2026-08-24 on pipeline
/// 1456: four `mrd fingerprint` calls reported an absent daemon (`Resource
/// temporarily unavailable (os error 11)`) two lines after `ensure_daemon`
/// had proved the same socket answers a ping. `os error 11` on Linux (35 on
/// macOS) is EAGAIN — a READ TIMEOUT on an established connection, never a
/// `connect(2)` refusal, which for a blocking Unix stream cannot return it at
/// all (measured: 600 blocking connects against a never-accepting listener all
/// succeeded). So the message named the one condition that was ruled out.
#[derive(Debug)]
pub enum DialFailure {
    /// `connect(2)`, the write, or the stream itself failed: nothing is
    /// listening, or the connection died before a greet could arrive.
    Unreachable(io::Error),
    /// Connected and greeted, then the daemon stopped answering liveness
    /// probes: it died mid-handshake.
    Died {
        /// How long the handshake had been waiting when the probe failed.
        waited: Duration,
    },
    /// Connected and greeted; the daemon kept answering liveness probes for
    /// the whole cap and never answered the handshake. Up, and wedged.
    Silent {
        /// How long the handshake waited before the cap was spent.
        waited: Duration,
    },
    /// The daemon answered and refused the handshake (contract mismatch,
    /// identity skew). Its own words, already verbatim.
    Refused(io::Error),
}

impl DialFailure {
    /// Render for a caller whose absent-daemon teaching is `absent`. Only the
    /// arms that MEAN an absent daemon get that teaching; a slow or wedged
    /// daemon gets its own, because "start the daemon" is the wrong act when
    /// one is already running and answering.
    pub(crate) fn teach(&self, absent: &str) -> String {
        match self {
            Self::Unreachable(e) => format!("{absent} (cannot dial the daemon: {e})"),
            Self::Died { waited } => format!(
                "{absent} (the daemon accepted this connection and then stopped answering \
                 liveness probes {:.1}s into the v3 handshake — it died mid-dial. Nothing was \
                 sent past the handshake, so nothing was written.)",
                waited.as_secs_f32()
            ),
            Self::Silent { waited } => format!(
                "the daemon is UP and did not greet: it answered liveness probes for the whole \
                 {:.0}s this handshake waited, and never answered the v3 hello. This is a wedged \
                 or saturated daemon, NOT an absent one — starting another will not help, and \
                 nothing was sent past the handshake, so nothing was written.\n\
                 Why this is not a transport error: the handshake's read timeout expiring is \
                 EAGAIN (`os error 11` on Linux, 35 on macOS), which reads exactly like a failed \
                 dial and is not one.\n\
                 Fixes — run whichever fits your case:\n\
                 \x20 - retry: a saturated daemon greets again once its load drops.\n\
                 \x20 - read the daemon's stderr: one that pings but never greets is stalled \
                 under the registry write lock; restart it and report the stall.",
                waited.as_secs_f32()
            ),
            Self::Refused(e) => e.to_string(),
        }
    }
}

/// Where a write verb's round trip stopped — and therefore **what is known
/// about the file**.
///
/// The two arms have opposite remedies, exactly like [`DialFailure`]'s, and
/// for the same reason: they were rendered with one voice. Every failure of
/// [`SocketDoor::call_until_answered`] reached `write_ipc::call` as one
/// `io::Error` and was reported as a landed-or-not UNKNOWN — true of a lost
/// answer, false of a frame that never finished going out. A caller told
/// "UNKNOWN" reads the file back and distrusts it; a caller told "nothing was
/// committed" retries. Getting that backwards is the same class of defect as
/// calling a wedged daemon an absent one.
#[derive(Debug)]
pub(crate) enum WriteHalt {
    /// The frame did not reach the daemon whole — a stalled or dead drain, or
    /// a request that would not even serialize. No newline arrived, so the
    /// daemon never parsed a request: **nothing was committed**, and a retry is
    /// safe.
    NotSent(io::Error),
    /// The frame WAS delivered and the answer was lost (EOF, a torn stream).
    /// The daemon may have committed before the loss: the outcome is UNKNOWN,
    /// and the file must be read back before any re-send.
    AnswerLost(io::Error),
}

impl WriteHalt {
    /// The transport error underneath, whichever half it came from.
    pub(crate) fn error(&self) -> &io::Error {
        match self {
            Self::NotSent(e) | Self::AnswerLost(e) => e,
        }
    }
}

impl std::fmt::Display for WriteHalt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.error())
    }
}

impl std::fmt::Display for DialFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.teach(ABSENT))
    }
}

impl std::error::Error for DialFailure {}

impl From<DialFailure> for io::Error {
    fn from(f: DialFailure) -> Self {
        match f {
            DialFailure::Unreachable(e) | DialFailure::Refused(e) => e,
            other => io::Error::other(other.teach(ABSENT)),
        }
    }
}

/// Does the daemon at `socket` answer a registry `ping` within
/// [`PROBE_TIMEOUT`]? This is the EVIDENCE that replaces the clock: a
/// handshake tick asks the daemon whether it is alive instead of assuming from
/// a constant that it is not.
///
/// Deliberately NOT [`registry::Client::ping`]. That call is bounded now (by
/// `registry::wedge::PROBE_TIMEOUT`), so it would no longer park — but it
/// reaches the daemon through `registry::wedge::read_line`, whose every tick
/// runs a liveness probe of its own. Probing with it would recurse into the
/// discipline this probe exists to inform. `registry::wedge::answers_ping` is
/// hand-rolled for the same reason; the two are kept equal BY HAND.
fn daemon_answers_ping(socket: &Path) -> bool {
    use std::io::{BufRead as _, Write as _};

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
    serde_json::from_str::<Value>(&line)
        .ok()
        .and_then(|v| v.get("status").and_then(Value::as_str).map(|s| s == "pong"))
        .unwrap_or(false)
}

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
    /// The daemon's own socket path, retained at connect. The write half's
    /// tick probes it on a SECOND connection — that is what lets a daemon too
    /// busy to drain be told from one that died mid-send
    /// ([`registry::wedge::write_all`]).
    socket: PathBuf,
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
    /// a notice rather than a verdict.
    ///
    /// **The handshake used to draw that verdict anyway, and that was the
    /// defect** (card `dial-eagain-under-pipeline-load`). This doc used to read
    /// "a daemon that will not greet is down; nothing was sent" — the first
    /// clause is false under load, and it cost four CI reds that read as an
    /// absent daemon while the daemon was answering pings. The timeouts set
    /// above are now the TICK INTERVAL of [`Self::greet`], not the verdict: a
    /// tick asks the daemon whether it is alive ([`daemon_answers_ping`])
    /// instead of concluding from a constant that it is not.
    ///
    /// `greet_cap` is only the backstop for a daemon that pings forever and
    /// never greets. The script entry passes its own wall clock — its budget is
    /// real — and the budgetless write and mint verbs pass [`GREET_CAP`].
    ///
    /// # Errors
    /// [`DialFailure`], whose arms are kept apart on purpose: three of them
    /// used to render as "cannot dial the daemon", whose remedy is the wrong
    /// one for two of them.
    pub fn connect(
        socket: &Path,
        workspace: &Path,
        greet_cap: Duration,
    ) -> Result<Self, DialFailure> {
        let stream = UnixStream::connect(socket).map_err(DialFailure::Unreachable)?;
        stream
            .set_read_timeout(Some(super::cmd::WALL_CLOCK))
            .map_err(DialFailure::Unreachable)?;
        stream
            .set_write_timeout(Some(super::cmd::WALL_CLOCK))
            .map_err(DialFailure::Unreachable)?;
        let writer = stream.try_clone().map_err(DialFailure::Unreachable)?;
        let reader = BufReader::new(stream);
        let mut door = Self {
            writer,
            reader,
            socket: socket.to_path_buf(),
            caps: Vec::new(),
        };
        let hello = json!({
            "op": "hello",
            "proto": 1,
            "contract": "v3",
            "workspace": workspace.to_string_lossy(),
        });
        let line = door.greet(&hello, socket, greet_cap)?;
        let frame = Frame::parse(&line).map_err(DialFailure::Refused)?;
        if !frame.ok {
            // The daemon names WHAT refused (code, path, cause) in its error
            // frame; collapsing it to a static string cost a dogfood session a
            // fleet-wide outage nobody could attribute. Carry the frame's error
            // verbatim — RawValue bytes, never re-serialized.
            return Err(DialFailure::Refused(io::Error::other(format!(
                "the daemon refused the v3 handshake for this workspace: {}",
                frame
                    .error
                    .map_or_else(|| "(no error body)".to_owned(), |e| e.get().to_owned()),
            ))));
        }
        // 0025 socket law: identity equality on the hello frame already in
        // hand (one voice with the read/links lanes). Parsed once, at connect.
        let body: Option<Value> = match frame.body.as_deref() {
            Some(raw) => Some(
                serde_json::from_str(raw.get())
                    .map_err(|e| DialFailure::Refused(io::Error::other(e)))?,
            ),
            None => None,
        };
        if let Err(message) = crate::engine::hello_identity_skew(body.as_ref(), socket) {
            return Err(DialFailure::Refused(io::Error::other(message)));
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

    /// The handshake round trip, where a timeout tick is a QUESTION rather
    /// than a verdict.
    ///
    /// The hello goes out ONCE — a tick resumes the read, it never re-sends,
    /// because a second hello on the same connection is a different frame, not
    /// a retry. On each tick the daemon is asked, over its own second
    /// connection, whether it is alive:
    ///
    /// | the tick finds | outcome |
    /// |---|---|
    /// | ping answers, cap unspent | notice once, keep reading |
    /// | ping does not answer | [`DialFailure::Died`] — it really went away |
    /// | ping answers, cap spent | [`DialFailure::Silent`] — up and wedged |
    ///
    /// So the wait is bounded AND evidenced: what normally ends it is the
    /// daemon's own liveness, not a constant. The constant is only the floor.
    fn greet(
        &mut self,
        request: &Value,
        socket: &Path,
        cap: Duration,
    ) -> Result<String, DialFailure> {
        use std::io::{BufRead as _, Write as _};

        let mut frame = serde_json::to_string(request)
            .map_err(|e| DialFailure::Unreachable(io::Error::other(e)))?;
        frame.push('\n');
        // DOCUMENTED-FLAT, and the one write half in this repo that stays so.
        // The hello is ~120 bytes on a connection opened microseconds ago, so
        // its send buffer is empty and the whole frame fits in one `write(2)`
        // — a stalled DRAIN, which needs a FULL buffer, is unreachable here.
        // What can still fail is the connection itself, and `Unreachable` is
        // the right arm for that. The bytes that could stall the drain are a
        // write verb's BODY, and they go through `call_until_answered` below.
        self.writer
            .write_all(frame.as_bytes())
            .map_err(DialFailure::Unreachable)?;
        self.writer.flush().map_err(DialFailure::Unreachable)?;

        let started = Instant::now();
        let mut line = String::new();
        let mut noticed = false;
        loop {
            match self.reader.read_line(&mut line) {
                Ok(0) => {
                    return Err(DialFailure::Unreachable(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "the daemon closed the connection without greeting",
                    )));
                }
                Ok(_) => return Ok(line),
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    if !daemon_answers_ping(socket) {
                        return Err(DialFailure::Died {
                            waited: started.elapsed(),
                        });
                    }
                    if started.elapsed() >= cap {
                        return Err(DialFailure::Silent {
                            waited: started.elapsed(),
                        });
                    }
                    if !noticed {
                        eprintln!("{GREET_NOTICE}");
                        noticed = true;
                    }
                }
                // No `Interrupted` arm: `read_until` (which `read_line` calls)
                // absorbs EINTR itself, so it cannot surface here.
                Err(e) => return Err(DialFailure::Unreachable(e)),
            }
        }
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
    /// # The two halves have OPPOSITE commit facts, so they are separate arms
    ///
    /// This function's own read half is where "the outcome is unknown" is true:
    /// the frame is whole on the wire, the daemon is committing, and only its
    /// answer can settle what landed. Its WRITE half is the exact opposite —
    /// the frame never arrived whole, so the daemon cannot have acted on it —
    /// and until 2026-08-24 both reached the caller as one `io::Result`, which
    /// `write_ipc::call` rendered with the read half's words: *"the frame had
    /// reached the daemon, so the write may have committed … whether the file
    /// carries it is UNKNOWN."* On a stalled drain every clause of that is
    /// false, and it sends an operator to re-read a file that was never
    /// touched — the mirror image of the false-negative this door was built to
    /// fix. [`WriteHalt`] keeps them apart.
    ///
    /// The write half is also no longer flat-bounded: it carries the wedge
    /// discipline ([`registry::wedge::write_all`]), so a daemon too busy to
    /// drain a large body is not abandoned at the socket's 7 s tick, and a
    /// daemon that IS wedged says so instead of emitting a bare `os error
    /// 35`/`11`.
    ///
    /// # Errors
    /// [`WriteHalt::NotSent`] — the frame never reached the daemon whole;
    /// nothing was committed. [`WriteHalt::AnswerLost`] — it did, and the
    /// answer did not come back; the outcome is unknown.
    pub(crate) fn call_until_answered(
        &mut self,
        request: &Value,
        notice: &str,
    ) -> Result<String, WriteHalt> {
        use std::io::BufRead as _;

        let mut frame =
            serde_json::to_string(request).map_err(|e| WriteHalt::NotSent(io::Error::other(e)))?;
        frame.push('\n');
        // A write verb carries no budget, so the send gets the general
        // backstop rather than the socket's 7 s script wall clock — the same
        // reasoning that hands the handshake `GREET_CAP` (`write_ipc::connect`).
        registry::wedge::write_all(&mut self.writer, &self.socket, GREET_CAP, frame.as_bytes())
            .map_err(WriteHalt::NotSent)?;

        let mut line = String::new();
        let mut noticed = false;
        loop {
            match self.reader.read_line(&mut line) {
                Ok(0) => {
                    return Err(WriteHalt::AnswerLost(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "daemon closed the connection without a response",
                    )));
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
                Err(e) => return Err(WriteHalt::AnswerLost(e)),
            }
        }
    }
}

impl Door for SocketDoor {
    fn call(&mut self, request: &Value) -> io::Result<String> {
        crate::engine::call_line(&mut self.writer, &mut self.reader, &self.socket, request)
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
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, mpsc};
    use std::thread;
    use std::time::{Duration, Instant};

    use super::{DialFailure, GREET_CAP, SocketDoor, io};

    /// **A daemon that accepts, answers nothing, and cannot be pinged must not
    /// park the process.** The check above the socket bounds the NUMBER of
    /// calls; only the socket bounds one call, and a call that never returns is
    /// a call the clock never gets to check again — the shape that hung the tool
    /// with nothing bounding it.
    ///
    /// The mute listener here also fails the liveness probe (it accepts ONE
    /// connection and holds it), so this is the [`DialFailure::Died`] arm: gone,
    /// and correctly called gone. The neighbouring test covers the arm that used
    /// to be conflated with it — alive, and slow.
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
        SocketDoor::connect(&sock, dir.path(), GREET_CAP)
            .expect("an answering daemon binds the connection");
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
            let outcome = SocketDoor::connect(&dialled, &workspace, GREET_CAP);
            let died = matches!(outcome, Err(DialFailure::Died { .. }));
            let _ = tx.send((outcome.is_err(), died, started.elapsed()));
        });
        let (refused, died, elapsed) = rx
            .recv_timeout(super::super::cmd::WALL_CLOCK * 3)
            .expect("connect returned — a socket with no deadline never would");
        assert!(
            refused,
            "a daemon that answers nothing fails the round trip"
        );
        assert!(
            died,
            "and it fails as Died — a mute listener that also fails the liveness probe is gone, \
             not merely slow"
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

        let Err(refusal) = SocketDoor::connect(&sock, dir.path(), GREET_CAP) else {
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

    /// Serve a socket where liveness probes answer instantly on their own
    /// connections and the FIRST connection (the hello) is handled by `on_hello`
    /// — the CI shape made deterministic. Returns the stop flag and the thread.
    fn serve_pinging(
        listener: UnixListener,
        on_hello: impl Fn(std::os::unix::net::UnixStream) + Send + Sync + 'static,
    ) -> (Arc<AtomicBool>, thread::JoinHandle<()>) {
        listener.set_nonblocking(true).expect("nonblocking");
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            let on_hello = Arc::new(on_hello);
            let mut first = true;
            let mut workers = Vec::new();
            while !flag.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        stream.set_nonblocking(false).expect("blocking conn");
                        let is_hello = first;
                        first = false;
                        let on_hello = Arc::clone(&on_hello);
                        workers.push(thread::spawn(move || {
                            let mut reader = io::BufReader::new(stream.try_clone().expect("clone"));
                            let mut line = String::new();
                            let _ = reader.read_line(&mut line);
                            if is_hello {
                                on_hello(stream);
                            } else {
                                let mut w = stream;
                                let _ = w.write_all(b"{\"status\":\"pong\"}\n");
                                let _ = w.flush();
                            }
                        }));
                    }
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(20));
                    }
                    Err(_) => break,
                }
            }
            for w in workers {
                let _ = w.join();
            }
        });
        (stop, handle)
    }

    /// **A daemon that is UP and slow to greet must be DIALLED, not declared
    /// absent** — the defect of card `dial-eagain-under-pipeline-load`.
    ///
    /// Before the fix the handshake's read timeout was the verdict: a greet one
    /// tick late surfaced as EAGAIN (`os error 11` on Linux, 35 on macOS) and
    /// every write and mint verb rendered it "cannot dial the daemon. The daemon
    /// must come up." — pointing the operator at the one condition that was
    /// false. Measured on CI pipeline 1456: four of four daemon-dialling tests
    /// failed that way while `ensure_daemon` had just proved the same socket
    /// answers a ping.
    ///
    /// This test returns `Err` against the pre-fix `connect`.
    #[test]
    fn a_daemon_that_pings_but_greets_late_is_dialled_not_declared_absent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sock = dir.path().join("daemon.sock");
        let listener = UnixListener::bind(&sock).expect("bind");

        // One tick late: past the point the pre-fix code gave up, and no
        // further, so the suite does not pay for the whole cap.
        let late = super::super::cmd::WALL_CLOCK + Duration::from_secs(1);
        let (stop, serving) = serve_pinging(listener, move |stream| {
            thread::sleep(late);
            let frame = format!(
                "{{\"ok\":true,\"body\":{{\"proto\":1,\"server\":\"fake\",\"caps\":[],\
                 \"identity\":{{\"build\":\"{}\"}}}}}}\n",
                env!("MRD_BUILD_SHA")
            );
            let mut w = stream;
            let _ = w.write_all(frame.as_bytes());
            let _ = w.flush();
        });

        let started = Instant::now();
        let dialled = SocketDoor::connect(&sock, dir.path(), GREET_CAP);
        let elapsed = started.elapsed();
        stop.store(true, Ordering::SeqCst);
        serving.join().expect("the listener finishes");

        assert!(
            dialled.is_ok(),
            "a daemon answering liveness probes throughout is UP, and a late greet must be waited \
             for, not rendered as an absent daemon: {}",
            dialled.err().map_or_else(String::new, |e| e.to_string())
        );
        assert!(
            elapsed >= super::super::cmd::WALL_CLOCK,
            "the wait really did outlive one tick — otherwise this test proves nothing about the \
             timeout it exists to cover: {elapsed:?}"
        );
    }

    /// **A wedged daemon — pings forever, never greets — still refuses, and says
    /// which.** The evidenced wait needs a floor, or the fix for a false failure
    /// becomes an infinite hang. The refusal must name the real condition and
    /// must NOT tell the operator to start a daemon that is already running.
    ///
    /// Driven at the script entry's cap rather than [`GREET_CAP`] so the
    /// assertion is cheap; both ride the same code path.
    #[test]
    fn a_daemon_that_pings_but_never_greets_refuses_as_silent_not_as_absent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sock = dir.path().join("daemon.sock");
        let listener = UnixListener::bind(&sock).expect("bind");

        let held: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
        let release = Arc::clone(&held);
        let (stop, serving) = serve_pinging(listener, move |_stream| {
            // Accepted, read, never answered: the wedge. `_stream` stays alive
            // for the hold, so the client sees silence and not an EOF.
            while !release.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(20));
            }
        });

        let outcome = SocketDoor::connect(&sock, dir.path(), super::super::cmd::WALL_CLOCK);
        held.store(true, Ordering::SeqCst);
        stop.store(true, Ordering::SeqCst);
        serving.join().expect("the listener finishes");

        let Err(failure) = outcome else {
            panic!("a daemon that never greets cannot bind the connection")
        };
        assert!(
            matches!(failure, DialFailure::Silent { .. }),
            "a pingable daemon that never greets is Silent, never Died and never Unreachable: \
             {failure:?}"
        );
        let rendered = failure.to_string();
        assert!(
            rendered.contains("UP") && !rendered.contains("must come up"),
            "the refusal must not send the operator to start a daemon that is already running: \
             {rendered}"
        );
    }
}
