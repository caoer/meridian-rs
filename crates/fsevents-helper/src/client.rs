//! The engine's side of the handshake, and the descriptor passing both sides
//! use.

use std::fs::OpenOptions;
use std::io::{self, Read};
use std::mem;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::ptr;
use std::time::Duration;

use crate::{ACK, HELLO};

/// How long [`subscribe`] waits for the helper's reply line.
pub const REPLY_TIMEOUT: Duration = Duration::from_secs(2);

/// Subscribe to the events under `root` from the helper at `socket`.
///
/// After `Ok` the stream carries [`crate::wire`] records; writing a serial
/// to it, 8 bytes little-endian, asks for a [`crate::fse::FENCE`].
///
/// # Errors
///
/// Opening `root` or connecting fails, or the helper does not answer `ok`
/// within [`REPLY_TIMEOUT`]; a refusal's message is the helper's reply line.
pub fn subscribe(socket: &Path, root: &Path) -> io::Result<UnixStream> {
    let dir = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY)
        .open(root)?;
    let mut stream = UnixStream::connect(socket)?;
    // Set before sending: macOS refuses socket options with EINVAL once the
    // peer has closed, and a refusing helper closes right after its reply.
    stream.set_read_timeout(Some(REPLY_TIMEOUT))?;
    send_with_fd(&stream, &HELLO, dir.as_raw_fd())?;
    drop(dir);
    // One byte at a time: records may follow the ack in the same segment.
    let mut reply = Vec::new();
    let mut byte = [0u8];
    while reply.len() < 64 && reply.last() != Some(&b'\n') {
        if stream.read(&mut byte)? == 0 {
            break;
        }
        reply.push(byte[0]);
    }
    if reply == ACK {
        // Clearing fails only on a stream the helper already closed, which the
        // caller's first read reports as end of stream.
        let _ = stream.set_read_timeout(None);
        return Ok(stream);
    }
    let text = String::from_utf8_lossy(&reply);
    Err(io::Error::other(if text.is_empty() {
        "helper closed without a reply"
    } else {
        text.trim_end()
    }))
}

/// Control buffer with room and alignment for one `SCM_RIGHTS` descriptor.
#[repr(C, align(8))]
struct Cmsg([u8; 32]);

/// Send `bytes` in one `sendmsg` carrying `fd` as `SCM_RIGHTS`.
///
/// # Errors
///
/// The `sendmsg` call fails or sends only part of `bytes`.
pub fn send_with_fd(stream: &UnixStream, bytes: &[u8], fd: RawFd) -> io::Result<()> {
    let mut iov = libc::iovec {
        iov_base: bytes.as_ptr().cast_mut().cast(),
        iov_len: bytes.len(),
    };
    let mut ctl = Cmsg([0; 32]);
    // SAFETY: zeroed msghdr is valid; every pointer set below outlives the
    // sendmsg call; CMSG_SPACE(4) fits in `ctl` and CMSG_FIRSTHDR is non-null
    // for a control buffer of at least one header.
    let sent = unsafe {
        let mut msg: libc::msghdr = mem::zeroed();
        msg.msg_iov = &raw mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = ctl.0.as_mut_ptr().cast();
        msg.msg_controllen = libc::CMSG_SPACE(4) as _;
        let c = libc::CMSG_FIRSTHDR(&raw const msg);
        (*c).cmsg_level = libc::SOL_SOCKET;
        (*c).cmsg_type = libc::SCM_RIGHTS;
        (*c).cmsg_len = libc::CMSG_LEN(4) as _;
        ptr::write_unaligned(libc::CMSG_DATA(c).cast::<RawFd>(), fd);
        libc::sendmsg(stream.as_raw_fd(), &raw const msg, 0)
    };
    match usize::try_from(sent) {
        Ok(n) if n == bytes.len() => Ok(()),
        Ok(_) => Err(io::Error::new(io::ErrorKind::WriteZero, "short sendmsg")),
        Err(_) => Err(io::Error::last_os_error()),
    }
}

/// What one [`recv_with_fd`] got.
#[derive(Debug)]
pub struct Received {
    /// Data bytes read into the caller's buffer; 0 is end of stream.
    pub len: usize,
    /// The first descriptor passed, owned (closed on drop).
    pub fd: Option<OwnedFd>,
    /// More than one descriptor came, or the control data was truncated.
    pub extra: bool,
}

/// One `recvmsg` with room for one descriptor. Any further descriptors are
/// closed and reported as `extra`.
///
/// # Errors
///
/// The `recvmsg` call fails (including `WouldBlock` on a non-blocking socket).
pub fn recv_with_fd(stream: &UnixStream, buf: &mut [u8]) -> io::Result<Received> {
    let mut iov = libc::iovec {
        iov_base: buf.as_mut_ptr().cast(),
        iov_len: buf.len(),
    };
    let mut ctl = Cmsg([0; 32]);
    // SAFETY: zeroed msghdr is valid; `iov` and `ctl` outlive the call, and
    // CMSG_SPACE(4) fits in `ctl`.
    let (n, msg) = unsafe {
        let mut msg: libc::msghdr = mem::zeroed();
        msg.msg_iov = &raw mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = ctl.0.as_mut_ptr().cast();
        msg.msg_controllen = libc::CMSG_SPACE(4) as _;
        (libc::recvmsg(stream.as_raw_fd(), &raw mut msg, 0), msg)
    };
    let len = usize::try_from(n).map_err(|_| io::Error::last_os_error())?;
    let mut fds = Vec::new();
    // SAFETY: the kernel filled `ctl` and set msg_controllen; CMSG_FIRSTHDR /
    // CMSG_NXTHDR walk only within it, descriptors are read only below that
    // end, and each SCM_RIGHTS descriptor is a fresh one this process owns.
    unsafe {
        let mut c = libc::CMSG_FIRSTHDR(&raw const msg);
        while !c.is_null() {
            if (*c).cmsg_level == libc::SOL_SOCKET && (*c).cmsg_type == libc::SCM_RIGHTS {
                let data = libc::CMSG_DATA(c);
                // `cmsg_len` and `msg_controllen` are u32 on macOS, usize on
                // Linux. A truncated message may claim more than was written.
                #[allow(clippy::unnecessary_cast)]
                let end = (c as usize + (*c).cmsg_len as usize)
                    .min(ctl.0.as_ptr() as usize + msg.msg_controllen as usize);
                let bytes = end.saturating_sub(data as usize);
                for i in 0..bytes / size_of::<RawFd>() {
                    let fd = ptr::read_unaligned(data.cast::<RawFd>().add(i));
                    fds.push(OwnedFd::from_raw_fd(fd));
                }
            }
            c = libc::CMSG_NXTHDR(&raw const msg, c);
        }
    }
    let extra = fds.len() > 1 || msg.msg_flags & libc::MSG_CTRUNC != 0;
    let fd = fds.into_iter().next();
    Ok(Received { len, fd, extra })
}

#[cfg(test)]
mod tests {
    use super::{recv_with_fd, subscribe};
    use crate::HELLO;
    use std::io::Write;
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::net::UnixListener;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("fseh-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn helper_answers(reply: &'static [u8]) -> Result<(), String> {
        let dir = scratch(std::str::from_utf8(&reply[..2]).unwrap());
        let sock = dir.join("s");
        let listener = UnixListener::bind(&sock).unwrap();
        let root = dir.clone();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = [0u8; 16];
            let got = recv_with_fd(&s, &mut buf).unwrap();
            assert_eq!(&buf[..got.len], &HELLO);
            assert!(!got.extra);
            let passed = std::fs::File::from(got.fd.unwrap()).metadata().unwrap();
            let want = std::fs::metadata(&root).unwrap();
            assert_eq!((passed.dev(), passed.ino()), (want.dev(), want.ino()));
            s.write_all(reply).unwrap();
        });
        let r = subscribe(&sock, &dir).map(drop).map_err(|e| e.to_string());
        server.join().unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        r
    }

    #[test]
    fn subscribe_passes_the_root_and_reads_the_verdict() {
        assert_eq!(helper_answers(b"ok\n"), Ok(()));
        assert_eq!(
            helper_answers(b"no not-owner\n"),
            Err("no not-owner".to_string())
        );
    }
}
