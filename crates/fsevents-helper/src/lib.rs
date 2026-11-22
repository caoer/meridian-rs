//! The macOS `/dev/fsevents` root helper's shared half.
//!
//! `fseventsd` reorders events, so an engine cannot use an `FSEvents` sentinel as
//! an exact barrier. The raw device keeps kernel order and needs root: the
//! `mrd-fsevents-helper` binary reads it and serves each engine the events
//! under its workspace root, in kernel order, over a unix socket. This library
//! holds what both sides share: the handshake constants, the kernel event
//! types ([`fse`]), the helper-to-engine record format ([`wire`]), the pure
//! device-buffer parser ([`device`]) and router ([`route`]), and the engine's
//! [`client::subscribe`].
//!
//! Handshake: the engine opens its root directory, connects to
//! [`SOCKET_PATH`] and sends [`HELLO`] in one `sendmsg` carrying that
//! descriptor. The helper answers [`ACK`] or `no <reason>\n` and closes. After
//! the ack the stream carries [`wire`] records. The engine asks for a barrier
//! by writing a serial, 8 bytes little-endian; the helper raises an event of
//! its own, reads the device up to it, and answers with a [`fse::FENCE`]
//! record carrying the highest serial that fence covers.

pub mod device;
pub mod route;
pub mod wire;

#[cfg(unix)]
pub mod client;

/// Where the helper listens.
pub const SOCKET_PATH: &str = "/var/run/mrd-fsevents.sock";

/// The engine's first 8 bytes, sent with the root descriptor.
pub const HELLO: [u8; 8] = *b"MRDFSE1\n";

/// The helper's own directory, `nobody`'s and closed to everyone else: the
/// files whose creation is the helper's event in the kernel queue.
pub const MARK_DIR: &str = "/var/run/mrd-fsevents";

/// The helper's acceptance line. A refusal is `no <reason>\n`, then close.
pub const ACK: &[u8] = b"ok\n";

/// Kernel event types (`FSE_*` in `sys/fsevents.h`), as the record `kind` byte.
pub mod fse {
    pub const CREATE_FILE: u8 = 0;
    pub const DELETE: u8 = 1;
    pub const STAT_CHANGED: u8 = 2;
    pub const RENAME: u8 = 3;
    pub const CONTENT_MODIFIED: u8 = 4;
    pub const EXCHANGE: u8 = 5;
    pub const FINDER_INFO_CHANGED: u8 = 6;
    pub const CREATE_DIR: u8 = 7;
    pub const CHOWN: u8 = 8;
    pub const XATTR_MODIFIED: u8 = 9;
    pub const XATTR_REMOVED: u8 = 10;
    pub const CLONE: u8 = 14;
    /// Not a kernel type: the helper's "events were lost" record. Everything
    /// the subscriber knows about its tree is stale up to this record.
    pub const DROPPED: u8 = 0xFF;
    /// Not a kernel type: the helper's answer to a barrier request. Its path
    /// field is a serial, 8 bytes little-endian. Every event raised before
    /// the helper received that serial's request precedes this record in the
    /// stream, or a [`DROPPED`] record does.
    pub const FENCE: u8 = 0xFE;

    /// Whether `kind` is a kernel type the helper reports.
    #[must_use]
    pub fn reported(kind: u8) -> bool {
        kind <= XATTR_REMOVED || kind == CLONE
    }
}
