//! The client library: one connection per call to the daemon's unix socket.
//!
//! Each method opens a fresh [`UnixStream`], writes one NDJSON request line,
//! reads one NDJSON response line, and closes. A connection failure (no
//! daemon listening) surfaces as an [`io::Error`] the caller turns into an
//! ephemeral degrade — the daemon is an optimization, never a hard dependency
//! (decision 0001 round 5).
//!
//! **The read is bounded** ([`crate::wedge`]). A daemon that accepts a frame
//! and never answers it used to park the caller forever — a failure with no
//! symptom and no remedy, and the reason `mrd`'s script/write door hand-rolled
//! its own dial rather than use [`Client`] (`script::wire_host`). That reason
//! is spent: the park now lands as an `io::Error` on the path that already
//! handled one. What still keeps that door separate is its own wall clock, its
//! own failure arms, and the recursion it would cause by probing liveness
//! through a client whose reads are themselves probe-driven — see
//! `wire_host::PROBE_TIMEOUT`.

use std::io::{self, BufReader};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::protocol::{Request, Response, WorkspaceEntry};
use crate::server::default_socket_path;
use crate::wedge;

/// A handle to a daemon at a known socket path. Cheap to clone; holds no open
/// connection between calls.
#[derive(Debug, Clone)]
pub struct Client {
    socket_path: PathBuf,
}

impl Client {
    /// A client for the daemon at `socket_path`.
    #[must_use]
    pub fn new(socket_path: PathBuf) -> Self {
        Client { socket_path }
    }

    /// A client for the default per-user socket — the short hash-keyed path
    /// derived from the env-resolved cache root
    /// ([`crate::socket_path_for_cache_root`]).
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::NotFound`] when no cache root resolves.
    pub fn from_default() -> io::Result<Self> {
        Ok(Client::new(default_socket_path()?))
    }

    /// The socket path this client dials.
    #[must_use]
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Send one request and read one response, under the wedge discipline
    /// ([`crate::wedge`]): the read ticks, each tick asks the daemon whether it
    /// is still alive, and [`wedge::WEDGE_CAP`] only catches a daemon that
    /// answers pings forever and never answers this frame.
    ///
    /// # Errors
    ///
    /// Returns an error when the socket cannot be reached (no daemon), the
    /// write fails, the response is empty or unparseable, or the daemon died
    /// mid-request / is wedged (see [`wedge::read_line`] for those two kinds
    /// and why they are kept apart).
    pub fn request(&self, request: &Request) -> io::Result<Response> {
        self.request_capped(request, wedge::WEDGE_CAP)
    }

    /// [`Self::request`] with an explicit wedge cap. Private because the cap is
    /// a property of the QUESTION, not of the caller: a liveness probe that has
    /// not been answered in a couple of seconds has already answered.
    fn request_capped(&self, request: &Request, cap: Duration) -> io::Result<Response> {
        let stream = UnixStream::connect(&self.socket_path)?;
        // Before the clone: the timeouts are socket-level, so both halves
        // inherit them from this one call.
        wedge::bind(&stream)?;
        let mut writer = stream.try_clone()?;
        let mut line = serde_json::to_string(request).map_err(io::Error::other)?;
        line.push('\n');
        // Both halves under the discipline. Registry frames are small — a
        // `Register` carries one path — so a stalled drain is unlikely here;
        // it is not impossible (a daemon that has stopped reading altogether
        // leaves a full socket buffer), and the raw `WouldBlock` it used to
        // produce is the very message class this module abolishes.
        wedge::write_all(&mut writer, &self.socket_path, cap, line.as_bytes())?;

        let mut reader = BufReader::new(stream);
        let mut response_line = String::new();
        let read = wedge::read_line(&mut reader, &self.socket_path, cap, &mut response_line)?;
        if read == 0 || response_line.trim().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "daemon closed the connection without a response",
            ));
        }
        serde_json::from_str(&response_line).map_err(io::Error::other)
    }

    /// Whether the daemon answers a ping.
    ///
    /// Bounded by [`wedge::PROBE_TIMEOUT`], not the wedge cap: this is the
    /// liveness question itself, and [`crate::server`]'s spawn-readiness poll
    /// (`engine::ensure_daemon`, 5 s deadline) depends on a fast negative. A
    /// ping that spent the full cap would blow that deadline twelve times over.
    ///
    /// # Errors
    ///
    /// Returns an error only when the request round-trip itself fails; a
    /// running daemon that answers anything other than [`Response::Pong`]
    /// yields `Ok(false)`.
    pub fn ping(&self) -> io::Result<bool> {
        Ok(matches!(
            self.request_capped(&Request::Ping, wedge::PROBE_TIMEOUT)?,
            Response::Pong
        ))
    }

    /// Resolve `cwd`: `Some(entry)` when it is inside a registered workspace,
    /// `None` on a miss.
    ///
    /// # Errors
    ///
    /// Propagates a transport failure, or an unexpected response variant.
    pub fn resolve(&self, cwd: &Path) -> io::Result<Option<WorkspaceEntry>> {
        match self.request(&Request::Resolve {
            cwd: cwd.to_path_buf(),
        })? {
            Response::Resolved { entry } => Ok(Some(entry)),
            Response::Miss => Ok(None),
            other => Err(unexpected("resolve", &other)),
        }
    }

    /// Register `path`. Returns the raw [`Response`] so the caller can
    /// distinguish [`Response::Registered`] from [`Response::Denied`] and
    /// [`Response::Error`].
    ///
    /// # Errors
    ///
    /// Propagates a transport failure.
    pub fn register(&self, path: &Path) -> io::Result<Response> {
        self.request(&Request::Register {
            path: path.to_path_buf(),
        })
    }

    /// Unregister `path`. `true` when an entry was removed.
    ///
    /// # Errors
    ///
    /// Propagates a transport failure, or an unexpected response variant.
    pub fn unregister(&self, path: &Path) -> io::Result<bool> {
        match self.request(&Request::Unregister {
            path: path.to_path_buf(),
        })? {
            Response::Unregistered { removed } => Ok(removed),
            other => Err(unexpected("unregister", &other)),
        }
    }

    /// List every registered workspace.
    ///
    /// # Errors
    ///
    /// Propagates a transport failure, or an unexpected response variant.
    pub fn list(&self) -> io::Result<Vec<WorkspaceEntry>> {
        match self.request(&Request::List)? {
            Response::Listed { entries } => Ok(entries),
            other => Err(unexpected("list", &other)),
        }
    }
}

/// Build the error for a response variant a typed call did not expect.
fn unexpected(op: &str, response: &Response) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("unexpected response to {op}: {response:?}"),
    )
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixListener;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, mpsc};
    use std::thread;
    use std::time::{Duration, Instant};

    use super::{Client, Request, wedge};

    /// A BACKSTOP, never a budget — see `wedge::tests::NEVER_RETURNED`. It only
    /// decides whether a regression FAILS or hangs the suite forever.
    // `Duration::from_mins` not const-stable at MSRV 1.96 (same reason as
    // `wedge::WEDGE_CAP`).
    #[allow(clippy::duration_suboptimal_units)]
    const NEVER_RETURNED: Duration = Duration::from_secs(120);

    /// A listener that accepts and answers nothing, held open by a flag rather
    /// than a sleep, so it outlives the slowest read without costing a green
    /// run that sleep. (Same reasoning as `wedge::tests::Mute`; kept local
    /// because a test helper shared across modules is a dependency between
    /// tests, and these two measure different doors.)
    struct Mute {
        done: Arc<AtomicBool>,
        thread: Option<thread::JoinHandle<()>>,
    }

    impl Mute {
        fn holding(listener: UnixListener) -> Self {
            let done = Arc::new(AtomicBool::new(false));
            let flag = Arc::clone(&done);
            listener.set_nonblocking(true).expect("poll for the flag");
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
    }

    impl Drop for Mute {
        fn drop(&mut self) {
            self.done.store(true, Ordering::SeqCst);
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }

    /// **The whole point of the card.** A daemon that accepts the connection
    /// and answers nothing used to park every caller of this client forever —
    /// `mrd`, the MCP face, the daemon's own spawn-readiness poll. The failure
    /// had no symptom: no error, no exit, no log line, just a process that
    /// never returned.
    ///
    /// Measured through the PUBLIC door, because that is what callers hold. The
    /// elapsed lower bound proves the socket's own tick ended the wait rather
    /// than something cheaper upstream.
    #[test]
    fn a_daemon_that_accepts_and_never_answers_does_not_park_the_client() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sock = dir.path().join("daemon.sock");
        let _mute = Mute::holding(UnixListener::bind(&sock).expect("bind"));

        let (tx, rx) = mpsc::channel();
        let client = Client::new(sock.clone());
        thread::spawn(move || {
            let started = Instant::now();
            let outcome = client.request(&Request::List);
            let _ = tx.send((
                outcome.map(|_| ()).map_err(|e| e.to_string()),
                started.elapsed(),
            ));
        });

        let (outcome, elapsed) = rx
            .recv_timeout(NEVER_RETURNED)
            .expect("request returned — a socket with no read timeout never would");
        let message = outcome.expect_err("a daemon that answers nothing fails the round trip");
        assert!(
            elapsed >= wedge::TICK,
            "the socket's own tick is what fired, not something faster: {elapsed:?} ({message})"
        );
    }

    /// `ping` carries the probe bound, not the wedge cap: it IS the liveness
    /// question, and `engine::ensure_daemon` polls it against a 5 s deadline.
    /// A ping inheriting `WEDGE_CAP` would blow that deadline twelvefold, so
    /// the upper bound here is a real requirement rather than a timing guess —
    /// it is asserted against the CAP, which is 30x the value under test.
    #[test]
    fn ping_is_bounded_by_the_probe_timeout_not_the_wedge_cap() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sock = dir.path().join("daemon.sock");
        let _mute = Mute::holding(UnixListener::bind(&sock).expect("bind"));

        let (tx, rx) = mpsc::channel();
        let client = Client::new(sock.clone());
        thread::spawn(move || {
            let started = Instant::now();
            let _ = tx.send((client.ping().is_err(), started.elapsed()));
        });

        let (refused, elapsed) = rx
            .recv_timeout(NEVER_RETURNED)
            .expect("ping returned — an unbounded ping never would");
        assert!(refused, "an unanswering daemon does not pong");
        assert!(
            elapsed < wedge::WEDGE_CAP,
            "a ping must not inherit the wedge cap — ensure_daemon polls it against 5 s: {elapsed:?}"
        );
    }

    /// An absent socket still fails at `connect(2)` — instantly, and with the
    /// kind callers already branch on. The discipline must not turn a cheap
    /// negative into a wait.
    #[test]
    fn an_absent_daemon_still_fails_at_connect() {
        let dir = tempfile::tempdir().expect("tempdir");
        let client = Client::new(dir.path().join("nothing-here.sock"));
        let started = Instant::now();
        assert!(client.request(&Request::List).is_err());
        assert!(
            started.elapsed() < wedge::TICK,
            "no daemon is a refused connect, not a timed-out read"
        );
    }
}
