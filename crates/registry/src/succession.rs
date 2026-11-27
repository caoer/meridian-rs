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

/// How long the successor waits before it asks again after the first `busy`;
/// the wait doubles with each further one, up to [`BUSY_RETRY_MAX`].
pub(crate) const BUSY_RETRY: Duration = Duration::from_secs(2);

/// The longest wait between two asks of a resident that answers `busy`.
// `Duration::from_mins` is not const-stable at MSRV 1.96.
#[allow(clippy::duration_suboptimal_units)]
pub(crate) const BUSY_RETRY_MAX: Duration = Duration::from_secs(60);

/// How many `busy` answers the successor takes before it gives up. Each one
/// held every caller of the resident for up to [`DRAIN_BOUND`].
pub(crate) const BUSY_TRIES: u32 = 12;

/// How long the successor waits before it asks again when the resident
/// answered `warm` and named nothing new: it is building a drawer.
pub(crate) const WARM_RETRY: Duration = Duration::from_millis(200);

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
    /// With `reason:"busy"`: what was still running when the drain ended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    busy: Option<String>,
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

/// Write all of `bytes` on a shared stream.
fn put(mut stream: &UnixStream, bytes: &[u8]) -> io::Result<()> {
    stream.write_all(bytes)
}

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
        return put(stream, bytes);
    }
    let payload = size_of_val(fds);
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
    put(stream, &bytes[sent..])
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
                    let fd = std::ptr::read_unaligned(
                        data.add(slot * size_of::<RawFd>()).cast::<RawFd>(),
                    );
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
    /// Gives both back when the handover was abandoned meanwhile: the caller
    /// reads on. The flag is read under the lock [`resume_all`] clears it
    /// under, so a connection is never parked after the parked were resumed.
    pub(crate) fn park(&self, reading: Reading, parked: Parked) -> Option<(Reading, Parked)> {
        let mut live = self.live();
        if !self.quiescing() {
            return Some((reading, parked));
        }
        live.parked.push(parked);
        // `Reading` takes this lock to leave.
        drop(live);
        drop(reading);
        None
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

/// Why a `handover` request was not granted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Undrained {
    /// The successor's warm set does not cover this daemon's.
    Warm,
    /// Another handover is draining this daemon, or it is stopping.
    Taken,
    /// This many connections had not reached a request boundary when the
    /// drain bound passed: in dispatch, mid-line, or writing an answer.
    Requests(usize),
    /// A background pass (reaper, pre-warm, saver) outlasted the drain bound.
    Background,
    /// A query or a compaction holds a sql cache open.
    SqlCache,
}

impl Undrained {
    /// The wire `reason`.
    fn reason(self) -> &'static str {
        match self {
            Undrained::Warm => "warm",
            Undrained::Taken
            | Undrained::Requests(_)
            | Undrained::Background
            | Undrained::SqlCache => "busy",
        }
    }

    /// The wire `busy`: what was still running, when a drain found it.
    fn busy(self) -> Option<String> {
        match self {
            Undrained::Warm | Undrained::Taken => None,
            Undrained::Requests(n) => Some(format!("{n} request(s) in flight")),
            Undrained::Background => Some("background work in a pass".to_owned()),
            Undrained::SqlCache => Some("a sql cache in use".to_owned()),
        }
    }
}

impl std::fmt::Display for Undrained {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.busy() {
            Some(what) => write!(f, "{}: {what}", self.reason()),
            None => f.write_str(self.reason()),
        }
    }
}

/// Answer `grant:false` with why and this daemon's warm set.
fn refuse(stream: &UnixStream, registry: &Registry, why: Undrained) -> io::Result<Granted> {
    let answer = Answer {
        status: "handover".to_owned(),
        grant: false,
        reason: Some(why.reason().to_owned()),
        busy: why.busy(),
        warm: registry.warm_workspaces(),
        conns: Vec::new(),
    };
    let mut out = serde_json::to_string(&answer).map_err(io::Error::other)?;
    out.push('\n');
    put(stream, out.as_bytes())?;
    Ok(Granted::Kept)
}

/// Abandon a handover: read requests again and restart every parked
/// connection on this daemon.
fn resume_all(succession: &Succession, resume: &dyn Fn(Parked), why: &str) {
    let parked = {
        let mut live = succession.live();
        succession.quiesce.store(false, Ordering::SeqCst);
        std::mem::take(&mut live.parked)
    };
    timing::say!(
        "registry: handover abandoned ({why}); {} connection(s) resumed",
        parked.len()
    );
    for conn in parked {
        resume(conn);
    }
}

/// Resumes this daemon when the handover thread unwinds before the commit: a
/// panic there must not leave it holding the socket and reading nothing.
struct ResumeOnPanic<'a> {
    succession: &'a Succession,
    resume: &'a dyn Fn(Parked),
}

impl Drop for ResumeOnPanic<'_> {
    fn drop(&mut self) {
        if std::thread::panicking() && !self.succession.handed() {
            // The drain's exclusive hold was dropped by the same unwind.
            self.succession.background.clear_poison();
            resume_all(self.succession, self.resume, "the handover panicked");
        }
    }
}

/// Does the successor's claimed warm set cover everything this daemon holds
/// warm whose directory still exists on disk, with no drawer rebuild in flight?
fn covers(registry: &Registry, claimed: &[PathBuf]) -> bool {
    registry.cold_builds_idle()
        && registry
            .warm_workspaces()
            .iter()
            .all(|workspace| !workspace.is_dir() || claimed.contains(workspace))
}

/// The drain: every connection thread parks or ends, background work ends
/// its pass and stays out, the sql caches close and the state file is
/// flushed. `Err` names why the handover is abandoned, and for a `busy` one
/// what was still running.
fn drain<'a>(
    succession: &'a Succession,
    registry: &Registry,
    claimed: &[PathBuf],
) -> Result<std::sync::RwLockWriteGuard<'a, ()>, Undrained> {
    let deadline = Instant::now() + DRAIN_BOUND;
    let reading = {
        let readers = succession.live();
        let (readers, _) = succession
            .moved
            .wait_timeout_while(readers, DRAIN_BOUND, |readers| readers.reading > 0)
            .unwrap_or_else(PoisonError::into_inner);
        readers.reading
    };
    if reading > 0 {
        return Err(Undrained::Requests(reading));
    }
    let background = loop {
        match succession.background.try_write() {
            Ok(guard) => break guard,
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            Err(_) => return Err(Undrained::Background),
        }
    };
    // A request served during the drain may have warmed something new.
    if !covers(registry, claimed) {
        return Err(Undrained::Warm);
    }
    if !registry.close_sql_stores() {
        return Err(Undrained::SqlCache);
    }
    registry.flush();
    Ok(background)
}

/// Send the grant line with every descriptor and wait for `adopted`.
/// `Ok(false)`: the successor did not adopt inside the bound.
fn pass(stream: &UnixStream, conns: Vec<ConnState>, mut fds: Vec<RawFd>) -> io::Result<bool> {
    let answer = Answer {
        status: "handover".to_owned(),
        grant: true,
        reason: None,
        busy: None,
        warm: Vec::new(),
        conns,
    };
    let mut out = serde_json::to_string(&answer).map_err(io::Error::other)?;
    out.push('\n');
    let first: Vec<RawFd> = fds.drain(..fds.len().min(FDS_PER_MESSAGE)).collect();
    send_with_fds(stream, out.as_bytes(), &first)?;
    while !fds.is_empty() {
        let chunk: Vec<RawFd> = fds.drain(..fds.len().min(FDS_PER_MESSAGE)).collect();
        send_with_fds(stream, b" ", &chunk)?;
    }
    // The commit: `adopted` inside the bound, or this daemon serves on.
    stream.set_read_timeout(Some(ADOPT_BOUND))?;
    let mut word = String::new();
    Ok(BufReader::new(stream).read_line(&mut word).is_ok() && word.contains("\"adopted\""))
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
    request: &str,
    resume: &dyn Fn(Parked),
) -> io::Result<Granted> {
    let claimed: Ask = serde_json::from_str(request).map_err(io::Error::other)?;
    if !covers(registry, &claimed.warm) {
        return refuse(stream, registry, Undrained::Warm);
    }
    // One handover at a time.
    if succession.quiesce.swap(true, Ordering::SeqCst) {
        return refuse(stream, registry, Undrained::Taken);
    }
    let _resume_on_panic = ResumeOnPanic { succession, resume };
    let background = match drain(succession, registry, &claimed.warm) {
        Ok(background) => background,
        Err(why) => {
            resume_all(succession, resume, &why.to_string());
            return refuse(stream, registry, why);
        }
    };

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
        resume_all(succession, resume, "this daemon is stopping");
        return refuse(stream, registry, Undrained::Taken);
    };
    let (conns, fds): (Vec<ConnState>, Vec<RawFd>) = {
        let readers = succession.live();
        let parked = &readers.parked;
        let mut fds = vec![listening.as_raw_fd(), lock.held().as_raw_fd()];
        fds.extend(parked.iter().map(|conn| conn.stream.as_raw_fd()));
        let conns = parked
            .iter()
            .map(|conn| ConnState {
                workspace: conn.workspace.clone(),
                rev: rev_word(conn.rev).to_owned(),
            })
            .collect();
        (conns, fds)
    };
    let passed = conns.len();
    let adopted = pass(stream, conns, fds);
    if !matches!(adopted, Ok(true)) {
        let _ = put(stream, b"{\"aborted\":true}\n");
        drop((listener, singleton, background));
        resume_all(succession, resume, "the successor did not adopt");
        return adopted.map(|_| Granted::Kept);
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
    put(stream, b"{\"released\":true}\n")?;
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
    /// The resident could not drain; ask again later. Carries what it named
    /// as still running, when it named anything.
    Busy(Option<String>),
    /// Committed: these are this process's now.
    Granted {
        listener: UnixListener,
        singleton: DrawerLock,
        conns: Vec<Parked>,
    },
}

/// The resident ended the exchange before the commit: `Absent` when nothing
/// accepts on the socket any more, else `Busy`. The caller has closed every
/// descriptor it received, the listening socket's copy among them.
fn after_loss(socket: &Path) -> Offer {
    if UnixStream::connect(socket).is_ok() {
        Offer::Busy(None)
    } else {
        Offer::Absent
    }
}

/// Ask the daemon on `socket` to hand over to this process, which holds
/// `warm` warm.
///
/// Until the commit every received descriptor is a plain descriptor: closing
/// one unlocks nothing and ends no connection the resident still serves.
///
/// # Errors
/// The resident does not speak `handover`, or descriptors were lost in
/// transit: this process must not serve, and the resident serves on.
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
    // One read; `false` when the resident closed or died instead.
    let mut read = |fds: &mut Vec<OwnedFd>, bytes: &mut Vec<u8>| -> io::Result<bool> {
        match recv_with_fds(&stream, &mut buf, fds) {
            Ok(0) => Ok(false),
            Ok(got) => {
                bytes.extend_from_slice(&buf[..got]);
                Ok(true)
            }
            Err(e) if e.kind() == io::ErrorKind::InvalidData => Err(e),
            Err(_) => Ok(false),
        }
    };
    let line_end = loop {
        if let Some(end) = bytes.iter().position(|b| *b == b'\n') {
            break end;
        }
        if !read(&mut fds, &mut bytes)? {
            // The resident died mid-drain; nothing was passed.
            return Ok(after_loss(socket));
        }
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
            _ => Offer::Busy(answer.busy),
        });
    }
    // Continuation messages carry the descriptors past the first message's.
    let expected = 2 + answer.conns.len();
    while fds.len() < expected {
        if !read(&mut fds, &mut bytes)? {
            // The resident died mid-grant.
            drop(fds);
            return Ok(after_loss(socket));
        }
    }

    let adopted = (&stream).write_all(b"{\"adopted\":true}\n");
    let mut word = Vec::new();
    // `released`, `aborted`, or a close with neither: the resident died
    // holding the grant, and these descriptors are the only live ones.
    let _ = (&stream).read_to_end(&mut word);
    if adopted.is_err() || String::from_utf8_lossy(&word).contains("\"aborted\"") {
        // The resident gave the try up and serves on with its own copies, or
        // died before it could commit.
        drop(fds);
        return Ok(after_loss(socket));
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

    /// Accept the successor's connection and read its request line.
    fn asked(listener: &UnixListener) -> UnixStream {
        let (stream, _) = listener.accept().unwrap();
        let mut request = String::new();
        BufReader::new(&stream).read_line(&mut request).unwrap();
        assert!(is_handover(&request), "{request}");
        stream
    }

    #[test]
    fn a_grant_the_resident_gave_up_on_unlocks_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let socket = tmp.path().join("daemon.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let lock = DrawerLock::try_acquire(tmp.path()).unwrap().unwrap();

        let offer = std::thread::scope(|scope| {
            let asking = scope.spawn(|| ask(&socket, &[]));
            let stream = asked(&listener);
            // The resident stopped waiting for `adopted` before the successor
            // wrote it: the successor's write fails with the grant in hand.
            stream.shutdown(std::net::Shutdown::Read).unwrap();
            send_with_fds(
                &stream,
                b"{\"status\":\"handover\",\"grant\":true}\n",
                &[listener.as_raw_fd(), lock.held().as_raw_fd()],
            )
            .unwrap();
            drop(stream);
            asking.join().unwrap().unwrap()
        });

        assert!(matches!(offer, Offer::Busy(None)), "{offer:?}");
        assert!(
            DrawerLock::try_acquire(tmp.path()).unwrap().is_none(),
            "the resident still holds its lock"
        );
    }

    #[test]
    fn a_resident_that_dies_before_it_answers_is_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let socket = tmp.path().join("daemon.sock");
        let listener = UnixListener::bind(&socket).unwrap();

        let offer = std::thread::scope(|scope| {
            let asking = scope.spawn(|| ask(&socket, &[]));
            let stream = asked(&listener);
            drop(listener);
            drop(stream);
            asking.join().unwrap().unwrap()
        });

        assert!(matches!(offer, Offer::Absent), "{offer:?}");
    }

    #[test]
    fn only_a_handover_request_is_a_handover() {
        assert!(is_handover(r#"{"op":"handover","warm":[]}"#));
        assert!(!is_handover(r#"{"op":"toc","path":"handover.md"}"#));
        assert!(!is_handover(r#"{"op":"toc","path":"\"handover\""}"#));
    }
}
