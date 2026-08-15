//! Persistable leaf-digest memo: `path → (StatKey, blake3 leaf digest)`.
//!
//! The run plane's answer to the F8 fold bill (card run-preexec-severity):
//! every `mrd run` used to read the whole corpus four times per invocation —
//! two `domain_snapshot` folds plus the exec-window guard's two serial
//! full-byte snapshots — and cached none of it, so run never amortised while
//! sql (drawer) did. This memo is the digest half of a fold made persistent:
//! a member whose [`StatKey`] identity is unmoved serves its recorded
//! [`model::leaf_digest`] without a byte read; a moved or unknown member is
//! read by the CALLER (each observation site keeps its own read discipline —
//! the guard reads `O_NOFOLLOW`, the domain snapshot reads plain) and
//! recorded back.
//!
//! # Evidence, not proof
//! Same standing as [`crate::DomainCache`]'s in-process memo, which already
//! mints the daemon's detect-cycle roots off stat currency: a mutation that
//! preserves size, mtime, ctime, dev and inode is invisible. The memo only
//! ever fails toward READING — a missing file, a torn write, an unknown
//! version, or an unparseable row all degrade to cold entries, never to a
//! wrong digest served.
//!
//! # Storage
//! The caller owns placement (the run plane keeps it in the per-workspace
//! cache drawer, beside `sql.duckdb`) and writes it atomically
//! (temp + rename). [`DigestMemo::from_bytes`] never fails; rows are one
//! line each with the path LAST, so any path without a newline round-trips.
//! Paths carrying a newline are skipped at serialization — those members are
//! simply re-read next run.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::StatKey;

/// The format tag serialized rows ride under. Bump on any row-shape change:
/// an unknown tag deserializes as an empty memo (cold, correct).
const VERSION_LINE: &str = "mrd-digest-memo v1";

/// A persistable `path → (identity, leaf digest)` memo. See the module docs
/// for trust standing and storage contract.
#[derive(Debug, Default)]
pub struct DigestMemo {
    leaves: BTreeMap<PathBuf, (StatKey, [u8; 32])>,
    /// Members whose bytes the CALLER had to read this run (memo misses).
    misses: u64,
    /// Members served from the memo without a byte read.
    hits: u64,
}

impl DigestMemo {
    /// An empty memo: every lookup misses, every observation is recorded.
    #[must_use]
    pub fn new() -> DigestMemo {
        DigestMemo::default()
    }

    /// Deserialize a stored memo. NEVER fails: an unknown version, a torn
    /// tail, or an unparseable row degrades to fewer (or zero) entries —
    /// the memo fails toward reading, not toward a wrong digest.
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> DigestMemo {
        let Ok(text) = std::str::from_utf8(bytes) else {
            return DigestMemo::new();
        };
        let mut lines = text.lines();
        if lines.next() != Some(VERSION_LINE) {
            return DigestMemo::new();
        }
        let mut leaves = BTreeMap::new();
        for line in lines {
            let Some((key, digest, rel)) = parse_row(line) else {
                continue;
            };
            leaves.insert(rel, (key, digest));
        }
        DigestMemo {
            leaves,
            misses: 0,
            hits: 0,
        }
    }

    /// Serialize for storage. Entries whose path contains a newline are
    /// skipped (they cannot ride a line format; skipping costs one re-read).
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = String::with_capacity(64 + self.leaves.len() * 96);
        out.push_str(VERSION_LINE);
        out.push('\n');
        for (rel, (key, digest)) in &self.leaves {
            let Some(rel_str) = rel.to_str() else {
                continue;
            };
            if rel_str.contains('\n') {
                continue;
            }
            let (dev, ino, size, mtime, ctime) = key.raw_parts();
            let mut hex = String::with_capacity(64);
            for byte in digest {
                let _ = write!(hex, "{byte:02x}");
            }
            let _ = writeln!(
                out,
                "{dev} {ino} {size} {}.{} {}.{} {hex} {rel_str}",
                mtime.0, mtime.1, ctime.0, ctime.1
            );
        }
        out.into_bytes()
    }

    /// The recorded digest for `rel` iff its identity is exactly `key`.
    #[must_use]
    pub fn lookup(&mut self, rel: &Path, key: &StatKey) -> Option<[u8; 32]> {
        match self.leaves.get(rel) {
            Some((seen, digest)) if seen == key => {
                self.hits += 1;
                Some(*digest)
            }
            _ => {
                self.misses += 1;
                None
            }
        }
    }

    /// Record the digest observed for `rel` at identity `key` (the caller
    /// read the bytes; later lookups at the same identity serve this).
    pub fn record(&mut self, rel: PathBuf, key: StatKey, digest: [u8; 32]) {
        self.leaves.insert(rel, (key, digest));
    }

    /// Members served without a byte read since construction.
    #[must_use]
    pub fn hits(&self) -> u64 {
        self.hits
    }

    /// Lookups that required the caller to read bytes since construction.
    #[must_use]
    pub fn misses(&self) -> u64 {
        self.misses
    }
}

/// One stored row: `dev ino size mtime_s.mtime_ns ctime_s.ctime_ns hex64 path`.
/// The path is the tail (may contain spaces); any malformed field drops the row.
fn parse_row(line: &str) -> Option<(StatKey, [u8; 32], PathBuf)> {
    let mut parts = line.splitn(7, ' ');
    let dev = parts.next()?.parse::<u64>().ok()?;
    let ino = parts.next()?.parse::<u64>().ok()?;
    let size = parts.next()?.parse::<u64>().ok()?;
    let mtime = parse_secs_nanos(parts.next()?)?;
    let ctime = parse_secs_nanos(parts.next()?)?;
    let hex = parts.next()?;
    let rel = parts.next()?;
    if hex.len() != 64 || rel.is_empty() {
        return None;
    }
    let mut digest = [0u8; 32];
    for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
        let hi = (chunk[0] as char).to_digit(16)?;
        let lo = (chunk[1] as char).to_digit(16)?;
        digest[i] = u8::try_from(hi * 16 + lo).ok()?;
    }
    Some((
        StatKey::from_raw_parts(dev, ino, size, mtime, ctime),
        digest,
        PathBuf::from(rel),
    ))
}

/// `<i64>.<i64>` (seconds.nanos, both signed as `stat` reports them).
fn parse_secs_nanos(field: &str) -> Option<(i64, i64)> {
    let (secs, nanos) = field.split_once('.')?;
    Some((secs.parse().ok()?, nanos.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(size: u64) -> StatKey {
        StatKey::from_raw_parts(1, 2, size, (3, 4), (5, 6))
    }

    #[test]
    fn round_trips_entries_and_serves_only_exact_identities() {
        let mut memo = DigestMemo::new();
        memo.record(PathBuf::from("a/b c.md"), key(10), [7u8; 32]);
        let mut back = DigestMemo::from_bytes(&memo.to_bytes());
        assert_eq!(
            back.lookup(Path::new("a/b c.md"), &key(10)),
            Some([7u8; 32])
        );
        assert_eq!(back.lookup(Path::new("a/b c.md"), &key(11)), None);
        assert_eq!(back.lookup(Path::new("other.md"), &key(10)), None);
        assert_eq!(back.hits(), 1);
        assert_eq!(back.misses(), 2);
    }

    #[test]
    fn corrupt_or_alien_bytes_degrade_to_a_cold_memo() {
        for bytes in [
            &b"\xff\xfe"[..],
            b"mrd-digest-memo v999\n1 2 3 4.5 6.7 00 x.md\n",
            b"mrd-digest-memo v1\nnot a row\n",
            b"",
        ] {
            let mut memo = DigestMemo::from_bytes(bytes);
            assert_eq!(memo.lookup(Path::new("x.md"), &key(1)), None);
        }
    }

    #[test]
    fn a_torn_tail_drops_only_the_torn_row() {
        let mut memo = DigestMemo::new();
        memo.record(PathBuf::from("a.md"), key(1), [1u8; 32]);
        memo.record(PathBuf::from("b.md"), key(2), [2u8; 32]);
        let mut bytes = memo.to_bytes();
        bytes.truncate(bytes.len() - 20); // tear the second row
        let mut back = DigestMemo::from_bytes(&bytes);
        assert_eq!(back.lookup(Path::new("a.md"), &key(1)), Some([1u8; 32]));
        assert_eq!(back.lookup(Path::new("b.md"), &key(2)), None);
    }
}
