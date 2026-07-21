//! The client library: one connection per call to the daemon's unix socket.
//!
//! Each method opens a fresh [`UnixStream`], writes one NDJSON request line,
//! reads one NDJSON response line, and closes. A connection failure (no
//! daemon listening) surfaces as an [`io::Error`] the caller turns into an
//! ephemeral degrade — the daemon is an optimization, never a hard dependency
//! (decision 0001 round 5).

use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use crate::protocol::{Request, Response, WorkspaceEntry};
use crate::server::default_socket_path;

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

    /// A client for the default per-user socket (`<cache-root>/registry/`).
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

    /// Send one request and read one response.
    ///
    /// # Errors
    ///
    /// Returns an error when the socket cannot be reached (no daemon), the
    /// write fails, or the response is empty or unparseable.
    pub fn request(&self, request: &Request) -> io::Result<Response> {
        let stream = UnixStream::connect(&self.socket_path)?;
        let mut writer = stream.try_clone()?;
        let mut line = serde_json::to_string(request).map_err(io::Error::other)?;
        line.push('\n');
        writer.write_all(line.as_bytes())?;
        writer.flush()?;

        let mut reader = BufReader::new(stream);
        let mut response_line = String::new();
        let read = reader.read_line(&mut response_line)?;
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
    /// # Errors
    ///
    /// Returns an error only when the request round-trip itself fails; a
    /// running daemon that answers anything other than [`Response::Pong`]
    /// yields `Ok(false)`.
    pub fn ping(&self) -> io::Result<bool> {
        Ok(matches!(self.request(&Request::Ping)?, Response::Pong))
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
