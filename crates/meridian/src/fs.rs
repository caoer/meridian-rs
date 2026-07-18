//! **Charter:** disk truth in — read, walk, watch; feeds the model and
//! nothing else. **Never does:** writing files it didn't splice, caching to
//! disk.
//!
//! Rung 1 (read) and rung 4 (watch; the daemon bin arrives with it). The one
//! write this module will ever own is splice *execution* — atomic
//! tmp+fsync+rename of a model-validated splice (rung 2); hook *dispatch* on
//! change stays Go's (Rust never executes agent work).

use std::path::Path;

/// Read failure at the disk boundary — maps to the wire's `not_found` /
/// `invalid_utf8` / `internal` at the sidecar seam.
#[derive(Debug)]
pub enum ReadError {
    NotFound,
    /// Not valid UTF-8: refused, never lossy-decoded — spans must denote
    /// exact disk bytes or splice corrupts files (contract §8 row 1).
    InvalidUtf8,
    Io(std::io::Error),
}

/// Read one workspace file's raw truth. Path validation (workspace-relative,
/// no escapes) happens at the wire boundary before this call; `full` is
/// already rooted.
pub fn read_file(full: &Path) -> Result<String, ReadError> {
    let _ = full;
    todo!("read + strict UTF-8 (rung 1)")
}
