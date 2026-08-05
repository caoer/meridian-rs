//! Run stdout record (U8): live-stream tee + content-addressed durable log.
//! Stdout is teed to the caller while hashing; seal fsyncs and mints the
//! [`StdoutRecord`] (S8 ordering hook). Address is the invocation id; content
//! pinned by full sha256. Env is recorded as KEYS ONLY (S7) — values never
//! enter the receipt type.
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Where run logs live under the workspace root, out-of-tree by
/// construction (`.meridian/` is outside the md domain).
const RUNS_DIR: &str = ".meridian/runs";

/// Tee read size. Chunks are written through unbuffered so the log is
/// live-tailable and the live sink sees bytes as the child emits them.
const CHUNK: usize = 8192;

/// Why the record path refused. Log-side failures ([`RecordError::Write`],
/// [`RecordError::Seal`]) are fatal to the record even when the live stream
/// succeeded — a run whose stdout cannot be made durable must not receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordError {
    /// The caller-supplied invocation id is not a safe single path
    /// component (empty, over-long, dot-only, or outside `[A-Za-z0-9._-]`).
    BadInvocationId {
        /// The refused id.
        id: String,
    },
    /// A log for this invocation id already exists — a reused id would
    /// silently interleave two runs' stdout, so creation refuses loudly.
    Collision {
        /// The workspace-relative path that already exists.
        path: String,
    },
    /// Creating `.meridian/runs/` or the log file failed.
    Create {
        /// The workspace-relative path being created.
        path: String,
        /// The underlying I/O error.
        reason: String,
    },
    /// Reading the streamed source (the child's stdout pipe) failed.
    Read {
        /// The underlying I/O error.
        reason: String,
    },
    /// Writing the log file failed (disk full, `.meridian` removed, …).
    Write {
        /// The underlying I/O error.
        reason: String,
    },
    /// The live sink refused bytes (e.g. broken pipe on the caller's
    /// stdout). Distinct from [`RecordError::Write`] so the runner can tell
    /// a lost record from a lost audience.
    Live {
        /// The underlying I/O error.
        reason: String,
    },
    /// Making the log durable (fsync of the file or its directory) failed —
    /// the S8 ordering hook refused, so no receipt facts exist.
    Seal {
        /// The underlying I/O error.
        reason: String,
    },
}

impl std::fmt::Display for RecordError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RecordError::BadInvocationId { id } => {
                write!(f, "invocation id '{id}' is not a safe path component")
            }
            RecordError::Collision { path } => {
                write!(f, "run log {path} already exists (reused invocation id)")
            }
            RecordError::Create { path, reason } => write!(f, "create {path}: {reason}"),
            RecordError::Read { reason } => write!(f, "stdout read: {reason}"),
            RecordError::Write { reason } => write!(f, "run log write: {reason}"),
            RecordError::Live { reason } => write!(f, "live stream: {reason}"),
            RecordError::Seal { reason } => write!(f, "run log fsync: {reason}"),
        }
    }
}

impl std::error::Error for RecordError {}

/// An open, append-only run log: every byte fed in goes to the log file and
/// the running content hash. Sealing (fsync) turns it into the receipt-side
/// facts — there is no other way to obtain a [`StdoutRecord`].
#[derive(Debug)]
pub struct RunLog {
    file: File,
    /// Absolute `.meridian/runs` dir — fsynced at seal so the log's
    /// directory entry is durable, not just its bytes.
    dir: PathBuf,
    /// Workspace-relative log path (`.meridian/runs/<id>.log`).
    rel_path: String,
    invocation: String,
    hasher: Sha256,
    bytes: u64,
}

impl RunLog {
    /// Open the log for `invocation_id` at `.meridian/runs/<id>.log` under
    /// `workspace_root`, creating the directory on first use. Refuses a
    /// pre-existing log (id reuse) and any id that is not a safe single
    /// path component (§9: the id is caller-supplied, so it is validated
    /// before it names a file).
    ///
    /// # Errors
    /// [`RecordError::BadInvocationId`], [`RecordError::Collision`], or
    /// [`RecordError::Create`] on I/O failure.
    pub fn create(workspace_root: &Path, invocation_id: &str) -> Result<Self, RecordError> {
        if !is_safe_invocation_id(invocation_id) {
            return Err(RecordError::BadInvocationId {
                id: invocation_id.to_owned(),
            });
        }
        let rel_path = format!("{RUNS_DIR}/{invocation_id}.log");
        let dir = workspace_root.join(RUNS_DIR);
        std::fs::create_dir_all(&dir).map_err(|e| RecordError::Create {
            path: RUNS_DIR.to_owned(),
            reason: e.to_string(),
        })?;
        let file = match OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(dir.join(format!("{invocation_id}.log")))
        {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                return Err(RecordError::Collision { path: rel_path });
            }
            Err(e) => {
                return Err(RecordError::Create {
                    path: rel_path,
                    reason: e.to_string(),
                });
            }
        };
        Ok(Self {
            file,
            dir,
            rel_path,
            invocation: invocation_id.to_owned(),
            hasher: Sha256::new(),
            bytes: 0,
        })
    }

    /// Append one chunk: log file + content hash + size, in that order.
    /// Unbuffered — the on-disk log trails the child by at most one chunk.
    ///
    /// # Errors
    /// [`RecordError::Write`] — the chunk did not land; the record is dead.
    pub fn append(&mut self, chunk: &[u8]) -> Result<(), RecordError> {
        self.file.write_all(chunk).map_err(|e| RecordError::Write {
            reason: e.to_string(),
        })?;
        self.hasher.update(chunk);
        self.bytes += chunk.len() as u64; // usize→u64 is lossless on every supported target
        Ok(())
    }

    /// The S8 ordering hook: make the log durable, THEN mint the receipt
    /// facts. fsyncs the log file and its directory (so the dirent survives
    /// a crash, not just the bytes). The returned [`StdoutRecord`] is the
    /// only source of the stdout hash/size/address — receipt filling cannot
    /// precede this call.
    ///
    /// # Errors
    /// [`RecordError::Seal`] — the log is not durable; no facts are minted
    /// and the run must not receipt.
    pub fn seal(self) -> Result<StdoutRecord, RecordError> {
        let seal_err = |e: io::Error| RecordError::Seal {
            reason: e.to_string(),
        };
        self.file.sync_all().map_err(seal_err)?;
        File::open(&self.dir)
            .and_then(|d| d.sync_all())
            .map_err(seal_err)?;
        Ok(StdoutRecord {
            invocation: self.invocation,
            log: self.rel_path,
            sha256: to_hex(&self.hasher.finalize()),
            bytes: self.bytes,
        })
    }

    /// The workspace-relative log path (`.meridian/runs/<id>.log`).
    #[must_use]
    pub fn rel_path(&self) -> &str {
        &self.rel_path
    }
}

/// Tee `reader` (the child's stdout pipe) to completion: every chunk lands
/// in the log FIRST (the record outranks the audience), then flushes
/// through the live sink. Returns the total byte count.
///
/// # Errors
/// [`RecordError::Read`] / [`RecordError::Write`] / [`RecordError::Live`]
/// per failing side.
pub fn stream<R, W>(reader: &mut R, log: &mut RunLog, live: &mut W) -> Result<u64, RecordError>
where
    R: Read + ?Sized,
    W: Write + ?Sized,
{
    let mut buf = [0u8; CHUNK];
    let mut total = 0u64;
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => return Ok(total),
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => {
                return Err(RecordError::Read {
                    reason: e.to_string(),
                });
            }
        };
        log.append(&buf[..n])?;
        live.write_all(&buf[..n])
            .and_then(|()| live.flush())
            .map_err(|e| RecordError::Live {
                reason: e.to_string(),
            })?;
        total += n as u64; // n ≤ CHUNK, lossless
    }
}

/// The durable stdout facts of one run — minted exclusively by
/// [`RunLog::seal`] (the S8 ordering hook), consumed by the receipt.
/// Content-addressed: the log's address is the invocation id, its content
/// is pinned by the full sha256 (the cache-drawer hash discipline).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StdoutRecord {
    /// The caller-supplied invocation id (§9).
    pub invocation: String,
    /// Workspace-relative log address (`.meridian/runs/<id>.log`).
    pub log: String,
    /// Full lowercase-hex sha256 of the streamed stdout bytes.
    pub sha256: String,
    /// Total stdout size in bytes.
    pub bytes: u64,
}

/// One bash step's exec facts for the receipt (task-card gate: invocation
/// id + exit code + stdout hash + size + log address). Environment is
/// recorded as KEYS ONLY (S7): the constructor swallows the full env and
/// discards every value, so this type cannot carry a secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecRecord {
    /// The child's exit code (the timeout/signal report states are the
    /// runner's, decision #21 — a record exists only for a child that
    /// exited).
    pub exit_code: i32,
    /// The sealed stdout facts — presence proves the S8 fsync ran.
    pub stdout: StdoutRecord,
    /// The child environment's key names, sorted, deduped — never values.
    pub env_keys: Vec<String>,
}

impl ExecRecord {
    /// Build the record from the child's exit code, its sealed stdout
    /// facts, and its FULL environment — values are discarded here (S7),
    /// keys are sorted and deduped.
    pub fn new<K, V>(
        exit_code: i32,
        stdout: StdoutRecord,
        env: impl IntoIterator<Item = (K, V)>,
    ) -> Self
    where
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let mut env_keys: Vec<String> = env
            .into_iter()
            .map(|(k, _value_discarded)| k.as_ref().to_owned())
            .collect();
        env_keys.sort();
        env_keys.dedup();
        Self {
            exit_code,
            stdout,
            env_keys,
        }
    }
}

/// The U8→U4 record edge (review S10): the receipt owner adopts exec facts
/// by implementing this on its receipt struct. Defined here so the bash
/// path can fill records before the receipt grows its exec field, without
/// either unit editing the other's files.
pub trait ExecRecordSink {
    /// Adopt one exec record into the receipt-side facts.
    fn fill_exec(&mut self, exec: ExecRecord);
}

/// A caller-supplied invocation id is used as a file name; admit only a
/// bounded single path component: `[A-Za-z0-9._-]`, non-empty, ≤128 bytes,
/// not dot-only (kills `.`/`..` traversal and separators outright).
fn is_safe_invocation_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && !id.bytes().all(|b| b == b'.')
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// Lowercase-hex encode (no `hex` crate in the dep graph — the
/// `crates/cache` pattern).
fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}
