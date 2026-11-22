//! `mrd-fsevents-helper [--socket <path>]`: the root helper. It clones a
//! `/dev/fsevents` watcher and serves each subscriber the events under the
//! directory it passed at handshake, in kernel order, as relative-path
//! [`fsevents_helper::wire`] records. One thread, one `poll` loop.
//!
//! A barrier: a subscriber writes a serial; the helper creates a file in its
//! own directory, reads the device until that creation comes back, and sends
//! the subscriber a fence. The creation is raised after the request arrived,
//! and the read that returns it began after it was raised, so that read
//! reports any loss the kernel flagged before it. Every event raised before
//! the request is therefore ahead of the fence in the subscriber's stream,
//! or a loss record is.

#[cfg(not(target_os = "macos"))]
fn main() -> std::process::ExitCode {
    eprintln!("mrd-fsevents-helper: macOS only");
    std::process::ExitCode::from(2)
}

#[cfg(target_os = "macos")]
fn main() -> std::process::ExitCode {
    macos::main()
}

#[cfg(target_os = "macos")]
mod macos {
    use std::fs::{File, Permissions};
    use std::io::{self, ErrorKind, Read, Write};
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::process::ExitCode;
    use std::time::{Duration, Instant};

    use fsevents_helper::client::recv_with_fd;
    use fsevents_helper::route::{self, Outcome};
    use fsevents_helper::wire::{self, Node};
    use fsevents_helper::{ACK, HELLO, MARK_DIR, SOCKET_PATH, device, fse};

    /// `_IOW('s', 1, fsevent_clone_args)` with the 24-byte 64-bit args.
    const FSEVENTS_CLONE: libc::c_ulong = 0x8018_7301;
    /// `_IO('s', 102)`: put the dropped/combined flags into each event's type.
    const FSEVENTS_WANT_EXTENDED_INFO: libc::c_ulong = 0x2000_7366;
    const FSE_REPORT: i8 = 1;
    /// Kernel queue slots for this watcher. The event pool is shared by every
    /// watcher on the machine (4096 by default) and a non-Apple watcher is
    /// flushed past three quarters of its queue, so a stalled helper pins at
    /// most 768 events.
    const QUEUE_DEPTH: i32 = 1024;
    /// The kernel path form of a descriptor, the one event paths use.
    const F_GETPATH_NOFIRMLINK: libc::c_int = 102;
    const MAXPATHLEN: usize = 1024;
    const READ_BUF: usize = 256 * 1024;
    const OUT_CAP: usize = 1 << 20;
    const MAX_CONNS: usize = 64;
    /// Bound on one device read; see [`read_device`].
    const ALARM_USEC: libc::suseconds_t = 200;
    /// The user and group the helper runs as once it holds the device and its
    /// socket: macOS's `nobody`.
    const NOBODY: u32 = 4_294_967_294;
    const HANDSHAKE: Duration = Duration::from_secs(5);
    /// How long a fence waits for its mark before the helper raises another:
    /// a mark the kernel dropped is never read.
    const MARK_AGAIN: Duration = Duration::from_millis(2);

    #[repr(C, packed(4))]
    struct CloneArgs {
        event_list: *mut i8,
        num_events: i32,
        event_queue_depth: i32,
        fd: *mut i32,
    }

    struct Sub {
        prefix: Vec<u8>,
        /// Whole records; `out[..sent]` is already on the socket. Starts on a
        /// record boundary.
        out: Vec<u8>,
        sent: usize,
        lost: bool,
    }

    struct Conn {
        stream: UnixStream,
        since: Instant,
        sub: Option<Sub>,
        dead: bool,
        /// Bytes of a serial not yet complete.
        partial: Vec<u8>,
        /// The highest serial asked for and not yet given a mark.
        asked: Option<u64>,
        /// The mark this subscriber's fence waits for, and the serial it
        /// answers: any mark numbered at least this was raised after the
        /// request arrived.
        fence: Option<(u64, u64)>,
    }

    /// The helper's own events: file `m<n>` created and removed in
    /// [`MARK_DIR`].
    struct Marks {
        /// `MARK_DIR` in the kernel's path form, with `/m` appended.
        stem: Vec<u8>,
        /// This process, as the kernel names it on the events it raises.
        pid: i32,
        raised: u64,
        at: Instant,
    }

    impl Marks {
        /// Raise mark `raised + 1`. A creation that fails raises nothing, and
        /// the fences waiting on it time out with their engines.
        fn raise(&mut self) {
            self.raised += 1;
            self.at = Instant::now();
            let path = format!("{MARK_DIR}/m{}", self.raised);
            if File::create_new(&path).is_ok() {
                let _ = std::fs::remove_file(&path);
            }
        }

        /// The number of the mark whose creation `ev` is. Only a creation
        /// this process raised counts: a file another process puts in the
        /// directory releases no fence.
        fn number(&self, ev: &device::Event<'_>) -> Option<u64> {
            if ev.kind != fse::CREATE_FILE || ev.pid != self.pid {
                return None;
            }
            let (path, _) = ev.paths[0]?;
            std::str::from_utf8(path.strip_prefix(self.stem.as_slice())?)
                .ok()?
                .parse()
                .ok()
        }
    }

    pub(crate) fn main() -> ExitCode {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let socket = match args.as_slice() {
            [] => SOCKET_PATH.to_string(),
            [flag, path] if flag == "--socket" => path.clone(),
            _ => {
                eprintln!("usage: mrd-fsevents-helper [--socket <path>]");
                return ExitCode::from(2);
            }
        };
        match run(&socket) {
            Ok(()) => ExitCode::SUCCESS,
            Err(msg) => {
                eprintln!("mrd-fsevents-helper: {msg}");
                ExitCode::from(1)
            }
        }
    }

    fn run(socket: &str) -> Result<(), String> {
        // SAFETY: geteuid has no preconditions.
        if unsafe { libc::geteuid() } != 0 {
            return Err("must run as root".into());
        }
        let mut device = File::from(open_clone().map_err(|e| format!("/dev/fsevents clone: {e}"))?);
        catch_alarm().map_err(|e| format!("sigaction: {e}"))?;
        let mut marks = mark_dir().map_err(|e| format!("{MARK_DIR}: {e}"))?;
        let _ = std::fs::remove_file(socket);
        let listener = UnixListener::bind(socket).map_err(|e| format!("bind {socket}: {e}"))?;
        // Open on purpose: authorization is the handshake.
        std::fs::set_permissions(socket, Permissions::from_mode(0o666))
            .and_then(|()| listener.set_nonblocking(true))
            .map_err(|e| format!("{socket}: {e}"))?;
        drop_root().map_err(|e| format!("drop root: {e}"))?;
        // Rust's runtime ignores SIGPIPE, so a write to a closed subscriber is EPIPE.
        let mut conns: Vec<Conn> = Vec::new();
        let mut fds: Vec<libc::pollfd> = Vec::with_capacity(2 + MAX_CONNS);
        let mut buf = vec![0u8; READ_BUF];
        loop {
            fds.clear();
            let poll_in = |fd| libc::pollfd {
                fd,
                events: libc::POLLIN,
                revents: 0,
            };
            fds.push(poll_in(device.as_raw_fd()));
            fds.push(poll_in(listener.as_raw_fd()));
            for c in &conns {
                let mut p = poll_in(c.stream.as_raw_fd());
                if c.sub.as_ref().is_some_and(|s| s.sent < s.out.len()) {
                    p.events |= libc::POLLOUT;
                }
                fds.push(p);
            }
            let fencing = conns.iter().any(|c| c.fence.is_some());
            let timeout = conns
                .iter()
                .filter(|c| c.sub.is_none())
                .map(|c| HANDSHAKE.saturating_sub(c.since.elapsed()).as_millis() + 1)
                .chain(fencing.then_some(MARK_AGAIN.as_millis()))
                .min()
                .map_or(-1, |ms| i32::try_from(ms).unwrap_or(i32::MAX));
            let nfds = libc::nfds_t::try_from(fds.len()).unwrap_or(0);
            // SAFETY: `fds` is a live pollfd array of at least `nfds` entries.
            let n = unsafe { libc::poll(fds.as_mut_ptr(), nfds, timeout) };
            if n < 0 {
                let e = io::Error::last_os_error();
                if e.kind() == ErrorKind::Interrupted {
                    continue;
                }
                return Err(format!("poll: {e}"));
            }
            for (c, p) in conns.iter_mut().zip(&fds[2..]) {
                if p.revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0 {
                    readable(c);
                }
                if c.sub.is_none() && c.since.elapsed() >= HANDSHAKE {
                    c.dead = true;
                }
            }
            turn(
                &mut device,
                &mut buf,
                &mut marks,
                &mut conns,
                fds[0].revents != 0,
                fencing,
            )?;
            for c in &mut conns {
                if let Some(s) = &mut c.sub {
                    c.dead |= !flush(&c.stream, s);
                }
            }
            if fds[1].revents != 0 {
                while let Ok((stream, _)) = listener.accept() {
                    if conns.len() < MAX_CONNS && stream.set_nonblocking(true).is_ok() {
                        conns.push(Conn {
                            stream,
                            since: Instant::now(),
                            sub: None,
                            dead: false,
                            partial: Vec::new(),
                            asked: None,
                            fence: None,
                        });
                    }
                }
            }
            conns.retain(|c| !c.dead);
        }
    }

    /// One pass over the device: raise a mark when serials arrived or a
    /// fence's mark is overdue, read once, and queue each event's records and
    /// the fences its marks release.
    fn turn(
        device: &mut File,
        buf: &mut [u8],
        marks: &mut Marks,
        conns: &mut [Conn],
        device_ready: bool,
        fencing: bool,
    ) -> Result<(), String> {
        // A mark for the serials that just arrived, and another for
        // fences whose mark is overdue. Raised here, between reads: the
        // read that returns a mark began after the mark was raised.
        let asked = conns.iter().any(|c| c.asked.is_some());
        if asked || (fencing && marks.at.elapsed() >= MARK_AGAIN) {
            marks.raise();
            for c in conns.iter_mut() {
                if let Some(serial) = c.asked.take() {
                    let covered = c.fence.map_or(serial, |(_, earlier)| earlier.max(serial));
                    c.fence = Some((marks.raised, covered));
                }
            }
        }
        if device_ready || asked {
            let len = read_device(device, buf).map_err(|e| format!("read /dev/fsevents: {e}"))?;
            for ev in device::parse(&buf[..len]) {
                let ev = ev.map_err(|_| "malformed /dev/fsevents buffer")?;
                let mark = marks.number(&ev);
                for c in conns.iter_mut() {
                    let Some(s) = &mut c.sub else { continue };
                    c.dead |= !queue(s, |prefix, out| route::records_for(prefix, &ev, out));
                    if let (Some(mark), Some((waits_for, serial))) = (mark, c.fence)
                        && mark >= waits_for
                    {
                        c.fence = None;
                        queue(s, |_, out| {
                            wire::encode(fse::FENCE, Node::Unknown, &serial.to_le_bytes(), out);
                            Outcome::Sent
                        });
                    }
                }
            }
        }
        Ok(())
    }

    fn open_clone() -> io::Result<OwnedFd> {
        let dev = File::open("/dev/fsevents")?;
        let mut list = [0i8; 15];
        for kind in (0..15u8).filter(|&k| fse::reported(k)) {
            list[usize::from(kind)] = FSE_REPORT;
        }
        let mut fd: i32 = -1;
        let mut args = CloneArgs {
            event_list: list.as_mut_ptr(),
            num_events: 15,
            event_queue_depth: QUEUE_DEPTH,
            fd: &raw mut fd,
        };
        // SAFETY: `args` matches the kernel's packed fsevent_clone_args and
        // its pointers outlive the call.
        if unsafe { libc::ioctl(dev.as_raw_fd(), FSEVENTS_CLONE, &raw mut args) } < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the clone ioctl returned a fresh descriptor nothing else owns.
        let clone = unsafe { OwnedFd::from_raw_fd(fd) };
        // SAFETY: an argument-less ioctl on a descriptor we own.
        if unsafe { libc::ioctl(clone.as_raw_fd(), FSEVENTS_WANT_EXTENDED_INFO) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(clone)
    }

    /// Become `nobody` for good. Root was needed to open the device and to
    /// bind under `/var/run`; reading the clone, accepting, and inspecting a
    /// passed descriptor need no privilege, so nothing after this line holds
    /// any.
    fn drop_root() -> io::Result<()> {
        let groups = [NOBODY];
        // SAFETY: each call takes plain integers or a pointer to `groups`.
        let dropped = unsafe {
            libc::setgroups(1, groups.as_ptr()) == 0
                && libc::setgid(NOBODY) == 0
                && libc::setuid(NOBODY) == 0
                && libc::geteuid() == NOBODY
                && libc::setuid(0) != 0
        };
        if dropped {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    extern "C" fn on_alarm(_: libc::c_int) {}

    /// Install a no-op SIGALRM handler without `SA_RESTART`, so the alarm
    /// [`read_device`] arms interrupts a blocked read with EINTR.
    fn catch_alarm() -> io::Result<()> {
        // SAFETY: a zeroed sigaction with an empty mask and a handler that
        // touches nothing is valid; the old action is not requested.
        let rc = unsafe {
            let mut sa: libc::sigaction = std::mem::zeroed();
            sa.sa_sigaction = on_alarm as extern "C" fn(libc::c_int) as libc::sighandler_t;
            libc::sigaction(libc::SIGALRM, &raw const sa, std::ptr::null_mut())
        };
        if rc == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    /// Arm SIGALRM every `usec` microseconds; zero disarms it. Repeating, so
    /// an alarm that fires before the read is entered is followed by one that
    /// lands inside it.
    fn alarm(usec: libc::suseconds_t) {
        let every = libc::timeval {
            tv_sec: 0,
            tv_usec: usec,
        };
        let it = libc::itimerval {
            it_interval: every,
            it_value: every,
        };
        // SAFETY: setitimer reads one itimerval from a local.
        unsafe { libc::setitimer(libc::ITIMER_REAL, &raw const it, std::ptr::null_mut()) };
    }

    /// One read of the clone descriptor, bounded to [`ALARM_USEC`].
    ///
    /// Poll reports the device readable whenever its queue is non-empty, but
    /// the kernel's read skips events under the directories it withholds
    /// (`route::ignored`; Spotlight writes there all the time) and then sleeps
    /// for the next event, even after it copied others. That sleep checks the
    /// queue outside the lock its wakeup takes, so it can miss the wakeup and
    /// hold an event until the one after. The alarm turns that sleep into
    /// EINTR (with any bytes already copied) and the loop polls again.
    fn read_device(device: &mut File, buf: &mut [u8]) -> io::Result<usize> {
        alarm(ALARM_USEC);
        let r = device.read(buf);
        alarm(0);
        match r {
            Err(e) if e.kind() == ErrorKind::Interrupted => Ok(0),
            r => r,
        }
    }

    /// A connection's socket is readable: the handshake, barrier serials, or
    /// the end.
    fn readable(c: &mut Conn) {
        let mut scratch = [0u8; 256];
        if c.sub.is_some() {
            match (&c.stream).read(&mut scratch) {
                Ok(0) => c.dead = true,
                Ok(n) => {
                    c.partial.extend_from_slice(&scratch[..n]);
                    let (serials, _) = c.partial.as_chunks::<8>();
                    let whole = serials.len() * 8;
                    for serial in serials {
                        let serial = u64::from_le_bytes(*serial);
                        c.asked = Some(c.asked.map_or(serial, |a| a.max(serial)));
                    }
                    c.partial.drain(..whole);
                }
                Err(e) => {
                    c.dead = !matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted);
                }
            }
            return;
        }
        let got = match recv_with_fd(&c.stream, &mut scratch) {
            Ok(got) if got.len > 0 => got,
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) => return,
            _ => {
                c.dead = true;
                return;
            }
        };
        let verdict = match got.fd {
            Some(fd) if !got.extra && scratch[..got.len] == HELLO => {
                authorize(&c.stream, &File::from(fd))
            }
            _ => Err("bad-hello"),
        };
        match verdict {
            Ok(prefix) => {
                if (&c.stream).write_all(ACK).is_ok() {
                    c.sub = Some(Sub {
                        prefix,
                        out: Vec::new(),
                        sent: 0,
                        lost: false,
                    });
                } else {
                    c.dead = true;
                }
            }
            Err(reason) => {
                let _ = (&c.stream).write_all(format!("no {reason}\n").as_bytes());
                c.dead = true;
            }
        }
    }

    /// The subscription prefix for `dir`, or the refusal reason. The caller's
    /// temporary `dir` closes the passed descriptor in every case.
    fn authorize(stream: &UnixStream, dir: &File) -> Result<Vec<u8>, &'static str> {
        let (mut uid, mut gid) = (0, 0);
        // SAFETY: getpeereid writes two ids into locals.
        if unsafe { libc::getpeereid(stream.as_raw_fd(), &raw mut uid, &raw mut gid) } != 0 {
            return Err("bad-hello");
        }
        let meta = dir.metadata().map_err(|_| "not-a-directory")?;
        if !meta.is_dir() {
            return Err("not-a-directory");
        }
        if uid != 0 && meta.uid() != uid {
            return Err("not-owner");
        }
        let mut prefix = kernel_path(dir).ok_or("no-path")?;
        prefix.push(b'/');
        if route::ignored(&prefix) {
            return Err("ignored-directory");
        }
        prefix.pop();
        eprintln!(
            "mrd-fsevents-helper: uid {uid} subscribed {}",
            String::from_utf8_lossy(&prefix)
        );
        Ok(prefix)
    }

    /// The path the kernel's events use for the directory `dir` is open on.
    fn kernel_path(dir: &File) -> Option<Vec<u8>> {
        let mut path = [0u8; MAXPATHLEN];
        // SAFETY: F_GETPATH_NOFIRMLINK writes at most MAXPATHLEN bytes.
        if unsafe { libc::fcntl(dir.as_raw_fd(), F_GETPATH_NOFIRMLINK, path.as_mut_ptr()) } != 0 {
            return None;
        }
        let end = path.iter().position(|&b| b == 0)?;
        Some(path[..end].to_vec())
    }

    /// Make [`MARK_DIR`] afresh as `nobody`'s own, mode 0700. Run as root.
    fn mark_dir() -> io::Result<Marks> {
        match std::fs::remove_dir_all(MARK_DIR) {
            Err(e) if e.kind() != ErrorKind::NotFound => return Err(e),
            _ => {}
        }
        std::fs::create_dir(MARK_DIR)?;
        std::fs::set_permissions(MARK_DIR, Permissions::from_mode(0o700))?;
        std::os::unix::fs::chown(MARK_DIR, Some(NOBODY), Some(NOBODY))?;
        let mut stem = kernel_path(&File::open(MARK_DIR)?).ok_or(ErrorKind::NotFound)?;
        stem.extend_from_slice(b"/m");
        Ok(Marks {
            stem,
            pid: i32::try_from(std::process::id()).map_err(|_| ErrorKind::InvalidData)?,
            raised: 0,
            at: Instant::now(),
        })
    }

    /// Queue records for `s` through `add`, a pending loss record first;
    /// `false` drops the subscriber.
    fn queue(s: &mut Sub, add: impl FnOnce(&[u8], &mut Vec<u8>) -> Outcome) -> bool {
        if s.lost {
            wire::encode(fse::DROPPED, Node::Unknown, b"", &mut s.out);
            s.lost = false;
        }
        if add(&s.prefix, &mut s.out) == Outcome::RootGone {
            return false;
        }
        if s.out.len() - s.sent > OUT_CAP {
            // Keep only the record already partly on the socket.
            let keep = boundary(&s.out, s.sent).1;
            s.out.truncate(keep);
            s.lost = true;
        }
        true
    }

    /// The record boundaries at or before, and at or after, `at`.
    fn boundary(out: &[u8], at: usize) -> (usize, usize) {
        let mut pos = 0;
        while pos < at {
            let next = pos + wire::record_len(&out[pos..]).unwrap_or(out.len() - pos);
            if next > at {
                return (pos, next);
            }
            pos = next;
        }
        (pos, pos)
    }

    /// Send what the socket takes now; `false` drops the subscriber.
    fn flush(stream: &UnixStream, s: &mut Sub) -> bool {
        while s.sent < s.out.len() {
            match (&*stream).write(&s.out[s.sent..]) {
                Ok(0) => return false,
                Ok(n) => s.sent += n,
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(_) => return false,
            }
        }
        if s.sent == s.out.len() {
            s.out.clear();
            s.sent = 0;
        } else if s.sent > OUT_CAP {
            let cut = boundary(&s.out, s.sent).0;
            s.out.drain(..cut);
            s.sent -= cut;
        }
        true
    }
}
