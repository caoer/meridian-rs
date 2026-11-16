//! Succession (`docs/wire-contract.md` §A.3): a successor daemon warms beside
//! the resident and takes its listening socket, its singleton lock and its
//! idle client connections in one exchange.
//!
//! Three parts:
//! - descriptor passing ([`send_with_fds`], [`recv_with_fds`]): `SCM_RIGHTS`
//!   over the handover connection;
//! - the resident's side ([`Succession`], [`grant`]): stop reading, drain,
//!   pass, commit or resume;
//! - the successor's side ([`ask`]): one request, one [`Offer`].
//!
//! At every instant exactly one daemon reads requests. The resident parks
//! every connection before it passes anything, and the successor accepts only
//! after the resident said `released` or died.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError, RwLock};
use std::time::{Duration, Instant};

use cache::DrawerLock;
use serde::{Deserialize, Serialize};
use wire_serve::rev::Rev;

use crate::Registry;

/// How often a connection waiting for its next request line looks at the
/// quiesce flag. Bounds how long an idle connection takes to park.
pub(crate) const PARK_TICK: Duration = Duration::from_millis(100);

/// How long the resident waits for requests in flight, background work and
/// cache compactions to end before it abandons a handover and serves on.
pub(crate) const DRAIN_BOUND: Duration = Duration::from_secs(5);

/// How long the resident waits for `adopted` after it passed its descriptors.
const ADOPT_BOUND: Duration = Duration::from_secs(5);

/// How long the successor waits before it asks again after `busy`.
pub(crate) const BUSY_RETRY: Duration = Duration::from_secs(2);

/// How long the successor keeps asking a resident that answers `busy`.
// `Duration::from_mins` is not const-stable at MSRV 1.96.
#[allow(clippy::duration_suboptimal_units)]
pub(crate) const BUSY_BOUND: Duration = Duration::from_secs(600);

/// Descriptors per message: under every platform's `SCM_RIGHTS` ceiling.
const FDS_PER_MESSAGE: usize = 64;

/// One client connection between two requests, with the two words its
/// `hello` negotiated.
#[derive(Debug)]
pub(crate) struct Parked {
    pub(crate) stream: UnixStream,
    pub(crate) workspace: Option<PathBuf>,
    pub(crate) rev: Rev,
}

/// The wire form of a [`Parked`] connection's state.
#[derive(Debug, Serialize, Deserialize)]
struct ConnState {
    workspace: Option<PathBuf>,
    rev: String,
}

/// The successor's request line.
#[derive(Debug, Serialize, Deserialize)]
struct Ask {
    op: String,
    warm: Vec<PathBuf>,
}

/// The resident's answer line.
#[derive(Debug, Serialize, Deserialize)]
struct Answer {
    status: String,
    grant: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    #[serde(default)]
    warm: Vec<PathBuf>,
    #[serde(default)]
    conns: Vec<ConnState>,
}

fn rev_word(rev: Rev) -> &'static str {
    match rev {
        Rev::V2 => "v2",
        Rev::V3 => "v3",
    }
}

/// Is `line` a `handover` request? A substring test first, so an ordinary
/// request line is not parsed twice.
pub(crate) fn is_handover(line: &str) -> bool {
    line.contains("\"handover\"")
        && serde_json::from_str::<Ask>(line).is_ok_and(|ask| ask.op == "handover")
}

// ── descriptor passing ────────────────────────────────────────────────────

/// Write `bytes` on `stream` with `fds` attached as `SCM_RIGHTS`.
///
/// # Errors
/// The `sendmsg` failed, or `fds` is longer than one message carries.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    reason = "libc's msghdr and cmsghdr field widths differ per platform; `as _` is the portable spelling"
)]
pub(crate) fn send_with_fds(stream: &UnixStream, bytes: &[u8], fds: &[RawFd]) -> io::Result<()> {
    if fds.is_empty() {
        return (&*stream).write_all(bytes);
    }
    let payload = std::mem::size_of_val(fds);
    let payload_len = libc::c_uint::try_from(payload)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "too many descriptors"))?;
    // SAFETY: CMSG_SPACE is arithmetic on its argument.
    let space = unsafe { libc::CMSG_SPACE(payload_len) } as usize;
    // u64 words keep the control buffer aligned for `cmsghdr`.
    let mut control = vec![0u64; space.div_ceil(8)];
    let mut iov = libc::iovec {
        iov_base: bytes.as_ptr().cast_mut().cast(),
        iov_len: bytes.len(),
    };
    // SAFETY: an all-zero `msghdr` is a valid empty header.
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &raw mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = control.as_mut_ptr().cast();
    msg.msg_controllen = space as _;
    // SAFETY: `msg` names one live iovec and a zeroed control buffer of
    // CMSG_SPACE(payload) bytes, so the first header and its data area lie
    // inside it; `fds` is `payload` bytes long.
    let sent = unsafe {
        let header = libc::CMSG_FIRSTHDR(&raw const msg);
        (*header).cmsg_level = libc::SOL_SOCKET;
        (*header).cmsg_type = libc::SCM_RIGHTS;
        (*header).cmsg_len = libc::CMSG_LEN(payload_len) as _;
        std::ptr::copy_nonoverlapping(fds.as_ptr().cast::<u8>(), libc::CMSG_DATA(header), payload);
        libc::sendmsg(stream.as_raw_fd(), &raw const msg, 0)
    };
    let sent = usize::try_from(sent).map_err(|_| io::Error::last_os_error())?;
    // The descriptors ride the first byte; the rest is plain data.
    (&*stream).write_all(&bytes[sent..])
}

/// Read into `buf`, appending any descriptors that arrived with the bytes to
/// `fds`. `Ok(0)` is end of stream.
///
/// # Errors
/// The `recvmsg` failed, or the control data was truncated (a descriptor was
/// lost in the kernel, so the handover cannot be trusted).
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    reason = "libc's msghdr and cmsghdr field widths differ per platform; `as _` is the portable spelling"
)]
pub(crate) fn recv_with_fds(
    stream: &UnixStream,
    buf: &mut [u8],
    fds: &mut Vec<OwnedFd>,
) -> io::Result<usize> {
    let payload = FDS_PER_MESSAGE * size_of::<RawFd>();
    // SAFETY: CMSG_SPACE is arithmetic on its argument.
    let space = unsafe { libc::CMSG_SPACE(payload as libc::c_uint) } as usize;
    let mut control = vec![0u64; space.div_ceil(8)];
    let mut iov = libc::iovec {
        iov_base: buf.as_mut_ptr().cast(),
        iov_len: buf.len(),
    };
    // SAFETY: an all-zero `msghdr` is a valid empty header.
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &raw mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = control.as_mut_ptr().cast();
    msg.msg_controllen = space as _;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    let flags = libc::MSG_CMSG_CLOEXEC;
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    let flags = 0;
    let got = loop {
        // SAFETY: `msg` names one live iovec over `buf` and a control buffer
        // of `space` bytes, both outliving the call.
        let got = unsafe { libc::recvmsg(stream.as_raw_fd(), &raw mut msg, flags) };
        if got >= 0 {
            break got;
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    };
    // SAFETY: the kernel filled `msg_controllen` bytes of `control`; the CMSG
    // walk stays inside them, and every SCM_RIGHTS payload is whole `RawFd`s
    // this process now owns exactly once.
    unsafe {
        let mut header = libc::CMSG_FIRSTHDR(&raw const msg);
        while !header.is_null() {
            if (*header).cmsg_level == libc::SOL_SOCKET && (*header).cmsg_type == libc::SCM_RIGHTS {
                let bytes = (*header).cmsg_len as usize - libc::CMSG_LEN(0) as usize;
                let data = libc::CMSG_DATA(header);
                for slot in 0..bytes / size_of::<RawFd>() {
                    let fd = std::ptr::read_unaligned(data.add(slot * size_of::<RawFd>()).cast::<RawFd>());
                    #[cfg(not(any(target_os = "linux", target_os = "android")))]
                    libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
                    fds.push(OwnedFd::from_raw_fd(fd));
                }
            }
            header = libc::CMSG_NXTHDR(&raw const msg, header);
        }
    }
    if msg.msg_flags & libc::MSG_CTRUNC != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "handover: descriptors were truncated in transit",
        ));
    }
    Ok(usize::try_from(got).unwrap_or(0))
}

// ── the resident's side ───────────────────────────────────────────────────

#[derive(Debug, Default)]
struct Live {
    /// Connection threads in request mode, the accept loop's own pass included.
    reading: usize,
    /// Connections that left request mode at a request boundary.
    parked: Vec<Parked>,
}

/// One daemon's succession state, shared by its accept loop, its connection
/// threads, its background threads and the handover driver.
#[derive(Debug)]
pub(crate) struct Succession {
    /// Raised by a handover in progress: nothing accepts, and a connection
    /// parks at its next request boundary.
    quiesce: AtomicBool,
    /// Raised at the commit: this daemon no longer serves.
    handed: AtomicBool,
    live: Mutex<Live>,
    moved: Condvar,
    /// Background work (reaper, pre-warm, saver) takes this shared for one
    /// pass and skips the pass when it cannot; a grant holds it exclusively.
    background: RwLock<()>,
    /// The listening socket, a copy of the accept loop's; `None` once this
    /// daemon stopped.
    listener: Mutex<Option<UnixListener>>,
    /// The singleton lock, here so the commit can give it away; `None` once
    /// this daemon stopped or handed over.
    singleton: Mutex<Option<DrawerLock>>,
}

/// A connection thread's place in the request-mode count; dropping it leaves.
#[derive(Debug)]
pub(crate) struct Reading(Arc<Succession>);

impl Drop for Reading {
    fn drop(&mut self) {
        self.0.live().reading -= 1;
        self.0.moved.notify_all();
    }
}

impl Succession {
    pub(crate) fn new(listener: UnixListener, singleton: DrawerLock) -> Self {
        Succession {
            quiesce: AtomicBool::new(false),
            handed: AtomicBool::new(false),
            live: Mutex::new(Live::default()),
            moved: Condvar::new(),
            background: RwLock::new(()),
            listener: Mutex::new(Some(listener)),
            singleton: Mutex::new(Some(singleton)),
        }
    }

    fn live(&self) -> std::sync::MutexGuard<'_, Live> {
        self.live.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Enter request mode. `None` while a handover is draining: the caller
    /// must not read (the accept loop skips its pass).
    ///
    /// The count is raised before the flag is read and the driver raises the
    /// flag before it reads the count, so one of the two always sees the
    /// other.
    pub(crate) fn enter(self: &Arc<Self>) -> Option<Reading> {
        let reading = self.enter_held();
        (!self.quiescing()).then_some(reading)
    }

    /// Enter request mode for a connection that already exists (adopted, or
    /// resumed after an abandoned handover): it parks by itself if it must.
    pub(crate) fn enter_held(self: &Arc<Self>) -> Reading {
        self.live().reading += 1;
        Reading(Arc::clone(self))
    }

    /// Is a handover draining this daemon?
    pub(crate) fn quiescing(&self) -> bool {
        self.quiesce.load(Ordering::SeqCst)
    }

    /// Has this daemon handed over? It serves nothing from then on.
    pub(crate) fn handed(&self) -> bool {
        self.handed.load(Ordering::SeqCst)
    }

    /// Leave request mode at a request boundary, giving the connection up.
    pub(crate) fn park(&self, reading: Reading, parked: Parked) {
        self.live().parked.push(parked);
        drop(reading);
    }

    /// This daemon is stopping: close its copy of the listening socket, so
    /// the socket refuses once the accept loop ends.
    pub(crate) fn close_listener(&self) {
        drop(
            self.listener
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take(),
        );
    }

    /// This daemon stopped: release the singleton lock. After a handover it
    /// is the successor's and nothing is released here.
    pub(crate) fn release_singleton(&self) {
        drop(
            self.singleton
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take(),
        );
    }

    /// One background pass's shared hold, or `None` while a handover holds
    /// the work still (the pass is skipped, never queued).
    pub(crate) fn background_pass(&self) -> Option<std::sync::RwLockReadGuard<'_, ()>> {
        if self.quiescing() || self.handed() {
            return None;
        }
        self.background.try_read().ok()
    }
}

/// How a `handover` request ended on the resident.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Granted {
    /// Committed: this daemon no longer serves.
    Handed,
    /// Refused or abandoned: this daemon serves on.
    Kept,
}

/// Serve one `handover` request line on `stream` (the driver's own
/// connection, already out of request mode). `resume` restarts a parked
/// connection on this daemon when the handover is abandoned.
///
/// # Errors
/// Stream I/O on the handover connection. The daemon serves on.
pub(crate) fn grant(
    succession: &Arc<Succession>,
    registry: &Registry,
    stream: &UnixStream,
    line: &str,
    resume: &dyn Fn(Parked),
) -> io::Result<Granted> {
    let claimed: Ask = serde_json::from_str(line).map_err(io::Error::other)?;
    let refuse = |reason: &str| -> io::Result<Granted> {
        let answer = Answer {
            status: "handover".to_owned(),
            grant: false,
            reason: Some(reason.to_owned()),
            warm: registry.warm_workspaces(),
            conns: Vec::new(),
        };
        let mut out = serde_json::to_string(&answer).map_err(io::Error::other)?;
        out.push('\n');
        (&*stream).write_all(out.as_bytes())?;
        Ok(Granted::Kept)
    };
    let covers = |registry: &Registry| {
        registry.cold_builds_idle()
            && registry
                .warm_workspaces()
                .iter()
                .all(|ws| claimed.warm.contains(ws))
    };
    if !covers(registry) {
        return refuse("warm");
    }
    // One handover at a time.
    if succession.quiesce.swap(true, Ordering::SeqCst) {
        return refuse("busy");
    }
    let abandon = |reason: &str| -> io::Result<Granted> {
        succession.quiesce.store(false, Ordering::SeqCst);
        let parked = std::mem::take(&mut succession.live().parked);
        timing::say!(
            "registry: handover abandoned ({reason}); {} connection(s) resumed",
            parked.len()
        );
        for conn in parked {
            resume(conn);
        }
        refuse(reason)
    };

    // Drain: every connection thread parks or ends.
    let deadline = Instant::now() + DRAIN_BOUND;
    let drained = {
        let live = succession.live();
        let (live, _) = succession
            .moved
            .wait_timeout_while(live, DRAIN_BOUND, |live| live.reading > 0)
            .unwrap_or_else(PoisonError::into_inner);
        live.reading == 0
    };
    if !drained {
        return abandon("busy");
    }
    // Background work ends its pass and stays out.
    let background = loop {
        match succession.background.try_write() {
            Ok(guard) => break Some(guard),
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            Err(_) => break None,
        }
    };
    let Some(background) = background else {
        return abandon("busy");
    };
    // A request served during the drain may have warmed something new.
    if !covers(registry) {
        drop(background);
        return abandon("warm");
    }
    if !registry.close_sql_stores() {
        drop(background);
        return abandon("busy");
    }
    registry.flush();

    // Both stay locked until the descriptors are sent: a concurrent stop
    // must not close a descriptor whose number is about to be passed.
    let listener = succession
        .listener
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let mut singleton = succession
        .singleton
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let (Some(listening), Some(lock)) = (listener.as_ref(), singleton.as_ref()) else {
        // This daemon is stopping.
        drop((listener, singleton, background));
        return abandon("busy");
    };
    let (conns, mut fds): (Vec<ConnState>, Vec<RawFd>) = {
        let live = succession.live();
        let mut fds = vec![listening.as_raw_fd(), lock.held().as_raw_fd()];
        fds.extend(live.parked.iter().map(|conn| conn.stream.as_raw_fd()));
        let conns = live
            .parked
            .iter()
            .map(|conn| ConnState {
                workspace: conn.workspace.clone(),
                rev: rev_word(conn.rev).to_owned(),
            })
            .collect();
        (conns, fds)
    };
    let answer = Answer {
        status: "handover".to_owned(),
        grant: true,
        reason: None,
        warm: Vec::new(),
        conns,
    };
    let mut out = serde_json::to_string(&answer).map_err(io::Error::other)?;
    out.push('\n');
    let passed = fds.len() - 2;
    let sent = (|| -> io::Result<bool> {
        let first: Vec<RawFd> = fds.drain(..fds.len().min(FDS_PER_MESSAGE)).collect();
        send_with_fds(stream, out.as_bytes(), &first)?;
        while !fds.is_empty() {
            let chunk: Vec<RawFd> = fds.drain(..fds.len().min(FDS_PER_MESSAGE)).collect();
            send_with_fds(stream, b" ", &chunk)?;
        }
        // The commit: `adopted` inside the bound, or this daemon serves on.
        stream.set_read_timeout(Some(ADOPT_BOUND))?;
        let mut word = String::new();
        let adopted = BufReader::new(stream).read_line(&mut word).is_ok()
            && word.contains("\"adopted\"");
        Ok(adopted)
    })();
    if !matches!(sent, Ok(true)) {
        let _ = (&*stream).write_all(b"{\"aborted\":true}\n");
        drop((listener, singleton, background));
        succession.quiesce.store(false, Ordering::SeqCst);
        let parked = std::mem::take(&mut succession.live().parked);
        timing::say!(
            "registry: handover abandoned (the successor did not adopt); {} connection(s) resumed",
            parked.len()
        );
        for conn in parked {
            resume(conn);
        }
        return sent.map(|_| Granted::Kept);
    }
    succession.handed.store(true, Ordering::SeqCst);
    // The lock leaves with its descriptor: closing this copy unlocks nothing.
    if let Some(lock) = singleton.take() {
        drop(lock.into_held());
    }
    drop((listener, singleton));
    drop(std::mem::take(&mut succession.live().parked));
    // Held for the rest of this process: no background pass runs again.
    std::mem::forget(background);
    (&*stream).write_all(b"{\"released\":true}\n")?;
    timing::say!("registry: handed over to a successor ({passed} connection(s) passed)");
    Ok(Granted::Handed)
}

// ── the successor's side ──────────────────────────────────────────────────

/// What the resident answered a `handover` request.
#[derive(Debug)]
pub(crate) enum Offer {
    /// No daemon accepts on the socket.
    Absent,
    /// The resident holds these warm; warm them and ask again.
    Warm(Vec<PathBuf>),
    /// The resident could not drain; ask again later.
    Busy,
    /// Committed: these are this process's now.
    Granted {
        listener: UnixListener,
        singleton: DrawerLock,
        conns: Vec<Parked>,
    },
}

/// Ask the daemon on `socket` to hand over to this process, which holds
/// `warm` warm.
///
/// # Errors
/// The exchange failed in a way that leaves the resident's state unknown to
/// this process: it must not serve.
pub(crate) fn ask(socket: &Path, warm: &[PathBuf]) -> io::Result<Offer> {
    let Ok(stream) = UnixStream::connect(socket) else {
        return Ok(Offer::Absent);
    };
    let mut request = serde_json::to_string(&Ask {
        op: "handover".to_owned(),
        warm: warm.to_vec(),
    })
    .map_err(io::Error::other)?;
    request.push('\n');
    (&stream).write_all(request.as_bytes())?;

    // The answer line, with whatever descriptors rode it.
    let mut fds: Vec<OwnedFd> = Vec::new();
    let mut bytes: Vec<u8> = Vec::new();
    let mut buf = [0u8; 4096];
    let line_end = loop {
        if let Some(end) = bytes.iter().position(|b| *b == b'\n') {
            break end;
        }
        let got = recv_with_fds(&stream, &mut buf, &mut fds)?;
        if got == 0 {
            // A resident that predates `handover` answers and closes, or one
            // that died: either way nothing was passed.
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the serving daemon closed the handover connection unanswered",
            ));
        }
        bytes.extend_from_slice(&buf[..got]);
    };
    let answer: Answer = serde_json::from_slice(&bytes[..line_end]).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "the serving daemon does not speak handover ({e}): {}",
                String::from_utf8_lossy(&bytes[..line_end])
            ),
        )
    })?;
    if !answer.grant {
        return Ok(match answer.reason.as_deref() {
            Some("warm") => Offer::Warm(answer.warm),
            _ => Offer::Busy,
        });
    }
    // Continuation messages carry the descriptors past the first message's.
    let expected = 2 + answer.conns.len();
    while fds.len() < expected {
        if recv_with_fds(&stream, &mut buf, &mut fds)? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the serving daemon closed the handover connection mid-grant",
            ));
        }
    }
    let mut fds = fds.into_iter();
    let listener = UnixListener::from(fds.next().expect("counted"));
    let singleton = DrawerLock::from_held(std::fs::File::from(fds.next().expect("counted")));
    let conns: Vec<Parked> = answer
        .conns
        .into_iter()
        .zip(fds)
        .map(|(state, fd)| Parked {
            stream: UnixStream::from(fd),
            workspace: state.workspace,
            rev: Rev::from_contract(Some(&state.rev)),
        })
        .collect();

    (&stream).write_all(b"{\"adopted\":true}\n")?;
    let mut word = Vec::new();
    // `released`, `aborted`, or a close with neither: the resident died
    // holding the grant, and these descriptors are the only live ones.
    let _ = (&stream).read_to_end(&mut word);
    if String::from_utf8_lossy(&word).contains("\"aborted\"") {
        // The resident serves on with its own copies. Dropping the lock here
        // must close, never unlock: the description is the resident's.
        drop(singleton.into_held());
        return Ok(Offer::Busy);
    }
    Ok(Offer::Granted {
        listener,
        singleton,
        conns,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptors_ride_the_bytes_they_were_sent_with() {
        let (a, b) = UnixStream::pair().unwrap();
        let (passed, kept) = UnixStream::pair().unwrap();
        send_with_fds(&a, b"take\n", &[passed.as_raw_fd()]).unwrap();
        drop(passed);

        let mut fds = Vec::new();
        let mut buf = [0u8; 16];
        let got = recv_with_fds(&b, &mut buf, &mut fds).unwrap();
        assert_eq!(&buf[..got], b"take\n");
        assert_eq!(fds.len(), 1, "one descriptor arrives with the line");

        // The received descriptor is the same open socket: its peer hears it.
        let received = UnixStream::from(fds.pop().unwrap());
        (&received).write_all(b"x").unwrap();
        let mut byte = [0u8; 1];
        (&kept).read_exact(&mut byte).unwrap();
        assert_eq!(&byte, b"x");
    }

    #[test]
    fn only_a_handover_request_is_a_handover() {
        assert!(is_handover(r#"{"op":"handover","warm":[]}"#));
        assert!(!is_handover(r#"{"op":"toc","path":"handover.md"}"#));
        assert!(!is_handover(r#"{"op":"toc","path":"\"handover\""}"#));
    }
}
