//! Sequential snapshot framing. Generic byte streams only; no filesystem I/O.
//! Entries are independent checksummed objects. A torn tail preserves earlier
//! verified entries; the footer distinguishes a complete save from a prefix.
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use model::Docs;

const MAGIC: &[u8] = b"mrd-parsed-v1\n";
const END: u8 = 255;
const HEADER_LIMIT: usize = 1024;

/// A transient prior for `fs::update_corpus`, selected by CURRENT verified leaves.
pub struct Restored {
    pub docs: Docs,
    pub unserved: BTreeMap<String, String>,
    pub leaves: BTreeMap<PathBuf, [u8; 32]>,
    /// The saved engine stamp, never a statement about current disk.
    pub fingerprint: String,
    /// False after any damaged record or missing/invalid footer.
    pub complete: bool,
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid parsed snapshot")
}

fn header(workspace: &Path, fingerprint: &str) -> io::Result<Vec<u8>> {
    if fingerprint.len() > HEADER_LIMIT {
        return Err(invalid());
    }
    let mut bytes = Vec::new();
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(crate::GENERATION.as_bytes());
    bytes.extend_from_slice(blake3::hash(workspace.as_os_str().as_encoded_bytes()).as_bytes());
    bytes.extend_from_slice(&(fingerprint.len() as u64).to_le_bytes());
    bytes.extend_from_slice(fingerprint.as_bytes());
    Ok(bytes)
}

fn record_hasher(tag: u8, digest: &[u8; 32]) -> blake3::Hasher {
    let mut hash = blake3::Hasher::new();
    hash.update(crate::GENERATION.as_bytes());
    hash.update(&[tag]);
    hash.update(digest);
    hash
}

fn checksum(tag: u8, digest: &[u8; 32], bytes: &[u8]) -> [u8; 32] {
    let mut hash = record_hasher(tag, digest);
    hash.update(bytes);
    *hash.finalize().as_bytes()
}

/// Stream unwanted objects without allocating their declared size. Wanted
/// payloads grow only as bytes actually arrive, including on a truncated file.
fn read_payload(
    input: &mut impl Read,
    len: usize,
    tag: u8,
    digest: &[u8; 32],
    retain: bool,
) -> io::Result<(Vec<u8>, [u8; 32])> {
    let mut hash = record_hasher(tag, digest);
    let mut bytes = Vec::new();
    let mut buffer = [0; 16 * 1024];
    let mut remaining = len;
    while remaining > 0 {
        let n = remaining.min(buffer.len());
        input.read_exact(&mut buffer[..n])?;
        hash.update(&buffer[..n]);
        if retain {
            bytes.extend_from_slice(&buffer[..n]);
        }
        remaining -= n;
    }
    Ok((bytes, *hash.finalize().as_bytes()))
}

/// Stream a deduplicated snapshot, retaining at most one encoded document.
///
/// # Errors
/// Propagates stream errors. The caller must write a temporary file and only
/// publish it after this function and the storage flush have succeeded.
pub fn write(
    mut out: impl Write,
    workspace: &Path,
    fingerprint: &str,
    docs: &Docs,
    unserved: &BTreeMap<String, String>,
    leaves: &BTreeMap<PathBuf, [u8; 32]>,
) -> io::Result<usize> {
    enum Entry<'a> {
        Doc(&'a model::Document),
        Unserved(&'a str),
    }
    let mut unique = BTreeMap::new();
    for (path, digest) in leaves {
        let Some(path) = path.to_str() else {
            continue;
        };
        if let Some(doc) = docs.get(path) {
            unique.entry(*digest).or_insert(Entry::Doc(doc));
        } else if let Some(why) = unserved.get(path) {
            unique.entry(*digest).or_insert(Entry::Unserved(why));
        }
    }
    let header = header(workspace, fingerprint)?;
    out.write_all(&header)?;
    out.write_all(blake3::hash(&header).as_bytes())?;
    let mut stream = blake3::Hasher::new();
    stream.update(&header);
    let mut encoded = 0;
    for (digest, entry) in unique {
        let (tag, payload) = match entry {
            Entry::Doc(doc) if model::leaf_digest(doc.raw.as_bytes()) == digest => {
                match crate::encode(doc) {
                    Some(bytes) => {
                        encoded += 1;
                        (0, bytes)
                    }
                    None => (2, Vec::new()), // intentional cache abstention
                }
            }
            Entry::Unserved(why) => (1, why.as_bytes().to_vec()),
            Entry::Doc(_) => (2, Vec::new()),
        };
        let sum = checksum(tag, &digest, &payload);
        let len = (payload.len() as u64).to_le_bytes();
        out.write_all(&[tag])?;
        out.write_all(&digest)?;
        out.write_all(&len)?;
        out.write_all(&sum)?;
        out.write_all(&payload)?;
        stream.update(&[tag]);
        stream.update(&digest);
        stream.update(&len);
        stream.update(&sum);
    }
    out.write_all(&[END])?;
    out.write_all(stream.finalize().as_bytes())?;
    Ok(encoded)
}

fn read_header(input: &mut impl Read, workspace: &Path) -> io::Result<(String, Vec<u8>)> {
    let mut magic = [0; MAGIC.len()];
    let mut generation = [0; 64];
    let mut identity = [0; 32];
    let mut len = [0; 8];
    input.read_exact(&mut magic)?;
    input.read_exact(&mut generation)?;
    input.read_exact(&mut identity)?;
    input.read_exact(&mut len)?;
    let len = usize::try_from(u64::from_le_bytes(len)).map_err(|_| invalid())?;
    if magic != MAGIC
        || generation != crate::GENERATION.as_bytes()
        || identity != *blake3::hash(workspace.as_os_str().as_encoded_bytes()).as_bytes()
        || len > HEADER_LIMIT
    {
        return Err(invalid());
    }
    let mut fingerprint = vec![0; len];
    input.read_exact(&mut fingerprint)?;
    let fingerprint = String::from_utf8(fingerprint).map_err(|_| invalid())?;
    let expected = header(workspace, &fingerprint)?;
    let mut sum = [0; 32];
    input.read_exact(&mut sum)?;
    if sum != *blake3::hash(&expected).as_bytes() {
        return Err(invalid());
    }
    Ok((fingerprint, expected))
}

/// Restore a usable prefix; current verified digests are the sole selection key.
///
/// # Errors
/// A missing/incompatible/corrupt header is an error (the caller cold-builds).
/// Record corruption and interrupted tails return a partial, safe prior.
pub fn read(
    mut input: impl Read,
    workspace: &Path,
    fresh: &BTreeMap<PathBuf, [u8; 32]>,
) -> io::Result<Restored> {
    let (fingerprint, header) = read_header(&mut input, workspace)?;
    let wanted: BTreeSet<_> = fresh.values().copied().collect();
    let mut objects = BTreeMap::new();
    let mut invalid_utf8 = BTreeMap::new();
    let mut stream = blake3::Hasher::new();
    stream.update(&header);
    let mut sound = true;
    let complete = loop {
        let mut tag = [0];
        if input.read_exact(&mut tag).is_err() {
            break false;
        }
        if tag[0] == END {
            let mut sum = [0; 32];
            let mut extra = [0];
            break input.read_exact(&mut sum).is_ok()
                && sum == *stream.finalize().as_bytes()
                && input.read(&mut extra).is_ok_and(|n| n == 0)
                && sound;
        }
        if tag[0] > 2 {
            break false;
        }
        let mut digest = [0; 32];
        let mut length = [0; 8];
        let mut sum = [0; 32];
        if input.read_exact(&mut digest).is_err()
            || input.read_exact(&mut length).is_err()
            || input.read_exact(&mut sum).is_err()
        {
            break false;
        }
        let Ok(len) = usize::try_from(u64::from_le_bytes(length)) else {
            break false;
        };
        if len > crate::MAX_ENTRY_BYTES {
            break false;
        }
        let retain = wanted.contains(&digest);
        let Ok((payload, actual_sum)) = read_payload(&mut input, len, tag[0], &digest, retain)
        else {
            break false;
        };
        stream.update(&tag);
        stream.update(&digest);
        stream.update(&length);
        stream.update(&sum);
        if actual_sum != sum {
            sound = false;
            continue;
        }
        if !retain {
            continue;
        }
        match tag[0] {
            0 => match crate::decode(&payload, &digest) {
                Some(doc) => {
                    objects.insert(digest, Arc::new(doc));
                }
                None => sound = false,
            },
            1 => match String::from_utf8(payload) {
                Ok(why) if why.starts_with("is not UTF-8 (") => {
                    invalid_utf8.insert(digest, why);
                }
                _ => sound = false,
            },
            2 => {
                if !payload.is_empty() {
                    sound = false;
                }
            }
            _ => unreachable!(),
        }
    };
    let mut restored = Restored {
        docs: Docs::new(),
        unserved: BTreeMap::new(),
        leaves: BTreeMap::new(),
        fingerprint,
        complete,
    };
    for (path, digest) in fresh {
        let Some(name) = path.to_str() else {
            // Non-UTF8 names are integrity-covered but never parsed/servable.
            restored.leaves.insert(path.clone(), *digest);
            continue;
        };
        if let Some(doc) = objects.get(digest) {
            restored.docs.insert(name.to_owned(), Arc::clone(doc));
            restored.leaves.insert(path.clone(), *digest);
        } else if let Some(why) = invalid_utf8.get(digest) {
            restored.unserved.insert(name.to_owned(), why.clone());
            restored.leaves.insert(path.clone(), *digest);
        }
    }
    Ok(restored)
}
