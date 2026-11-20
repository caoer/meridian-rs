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
    // Cap initial capacity at 64 KiB so normal documents avoid reallocations
    // while a corrupt/truncated header declaring a huge length cannot OOM.
    let mut bytes = if retain {
        Vec::with_capacity(len.min(64 * 1024))
    } else {
        Vec::new()
    };
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
        emit(&mut out, &mut stream, tag, &digest, &payload)?;
    }
    out.write_all(&[END])?;
    out.write_all(stream.finalize().as_bytes())?;
    Ok(encoded)
}

/// Write one record and fold its frame into the footer's stream hash.
fn emit(
    out: &mut impl Write,
    stream: &mut blake3::Hasher,
    tag: u8,
    digest: &[u8; 32],
    payload: &[u8],
) -> io::Result<()> {
    let sum = checksum(tag, digest, payload);
    emit_with_sum(out, stream, tag, digest, &sum, payload)
}

/// Write one record whose BLAKE3 checksum `sum` was already verified by
/// [`records`], avoiding a second full-payload BLAKE3 pass during [`rewrite`].
fn emit_with_sum(
    out: &mut impl Write,
    stream: &mut blake3::Hasher,
    tag: u8,
    digest: &[u8; 32],
    sum: &[u8; 32],
    payload: &[u8],
) -> io::Result<()> {
    let len = (payload.len() as u64).to_le_bytes();
    out.write_all(&[tag])?;
    out.write_all(digest)?;
    out.write_all(&len)?;
    out.write_all(sum)?;
    out.write_all(payload)?;
    stream.update(&[tag]);
    stream.update(digest);
    stream.update(&len);
    stream.update(sum);
    Ok(())
}

/// What [`rewrite`] wrote: document records copied from the snapshot it
/// replaces, and documents it encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rewritten {
    pub carried: usize,
    pub encoded: usize,
}

/// Stream a deduplicated snapshot of a generation that holds no document
/// map (merkle-spec §6.9, saving under a host). Every UTF-8-named leaf is a
/// document unless `unserved` names it. A document record of `prior` (the
/// snapshot this one replaces) whose digest `leaves` still names is copied,
/// checksum verified and not decoded; `encode` is asked for the payload
/// ([`crate::encode`]) of each remaining document by one of its paths and
/// its digest, and `None` writes an abstention. At most one record's payload
/// is held.
///
/// # Errors
/// Propagates `out`'s stream errors, under [`write`]'s publication rule. A
/// `prior` that is unreadable, incompatible or damaged only carries less.
pub fn rewrite(
    mut out: impl Write,
    workspace: &Path,
    fingerprint: &str,
    unserved: &BTreeMap<String, String>,
    leaves: &BTreeMap<PathBuf, [u8; 32]>,
    prior: Option<impl Read>,
    mut encode: impl FnMut(&str, &[u8; 32]) -> Option<Vec<u8>>,
) -> io::Result<Rewritten> {
    let mut documents = BTreeMap::new();
    let mut conditions = BTreeMap::new();
    for (path, digest) in leaves {
        let Some(path) = path.to_str() else {
            continue;
        };
        match unserved.get(path) {
            Some(why) => {
                conditions.entry(*digest).or_insert(why.as_str());
            }
            None => {
                documents.entry(*digest).or_insert(path);
            }
        }
    }
    let header = header(workspace, fingerprint)?;
    out.write_all(&header)?;
    out.write_all(blake3::hash(&header).as_bytes())?;
    let mut stream = blake3::Hasher::new();
    stream.update(&header);
    let mut written = Rewritten {
        carried: 0,
        encoded: 0,
    };
    if let Some(mut prior) = prior
        && let Ok((_, prior_header)) = read_header(&mut prior, workspace)
    {
        let wanted: BTreeSet<_> = documents.keys().copied().collect();
        records(
            &mut prior,
            &prior_header,
            &wanted,
            |tag, digest, sum, payload| {
                if tag == 0 && documents.remove(digest).is_some() {
                    emit_with_sum(&mut out, &mut stream, 0, digest, sum, &payload)?;
                    written.carried += 1;
                }
                Ok(true)
            },
        )?;
    }
    for (digest, path) in documents {
        match encode(path, &digest) {
            Some(payload) => {
                written.encoded += 1;
                emit(&mut out, &mut stream, 0, &digest, &payload)?;
            }
            None => emit(&mut out, &mut stream, 2, &digest, &[])?,
        }
    }
    for (digest, why) in conditions {
        emit(&mut out, &mut stream, 1, &digest, why.as_bytes())?;
    }
    out.write_all(&[END])?;
    out.write_all(stream.finalize().as_bytes())?;
    Ok(written)
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

/// Walk the records after a verified `header`, handing `visit` each one whose
/// digest `wanted` holds and whose checksum verifies: its tag, digest and
/// payload. `visit` answers whether the record was sound. Returns whether the
/// stream ended at a valid footer with every record sound; a damaged record is
/// skipped, a broken frame ends the walk.
///
/// # Errors
/// Only what `visit` returns.
fn records(
    input: &mut impl Read,
    header: &[u8],
    wanted: &BTreeSet<[u8; 32]>,
    mut visit: impl FnMut(u8, &[u8; 32], &[u8; 32], Vec<u8>) -> io::Result<bool>,
) -> io::Result<bool> {
    let mut stream = blake3::Hasher::new();
    stream.update(header);
    let mut sound = true;
    loop {
        let mut tag = [0];
        if input.read_exact(&mut tag).is_err() {
            return Ok(false);
        }
        if tag[0] == END {
            let mut sum = [0; 32];
            let mut extra = [0];
            return Ok(input.read_exact(&mut sum).is_ok()
                && sum == *stream.finalize().as_bytes()
                && input.read(&mut extra).is_ok_and(|n| n == 0)
                && sound);
        }
        if tag[0] > 2 {
            return Ok(false);
        }
        let mut digest = [0; 32];
        let mut length = [0; 8];
        let mut sum = [0; 32];
        if input.read_exact(&mut digest).is_err()
            || input.read_exact(&mut length).is_err()
            || input.read_exact(&mut sum).is_err()
        {
            return Ok(false);
        }
        let Ok(len) = usize::try_from(u64::from_le_bytes(length)) else {
            return Ok(false);
        };
        if len > crate::MAX_ENTRY_BYTES {
            return Ok(false);
        }
        let retain = wanted.contains(&digest);
        let Ok((payload, actual_sum)) = read_payload(input, len, tag[0], &digest, retain) else {
            return Ok(false);
        };
        stream.update(&tag);
        stream.update(&digest);
        stream.update(&length);
        stream.update(&sum);
        if actual_sum != sum {
            sound = false;
            continue;
        }
        if retain {
            sound &= visit(tag[0], &digest, &sum, payload)?;
        }
    }
}

/// One verified object of a snapshot, as [`scan`] hands it over.
#[derive(Debug)]
pub enum Object {
    /// The document decoded from a record ([`crate::decode`]).
    Doc(model::Document),
    /// The condition of a member whose bytes are not UTF-8.
    Unserved(String),
}

/// What [`scan`] read beside the objects it handed over.
#[derive(Debug)]
pub struct Scanned {
    /// The saved engine stamp, never a statement about current disk.
    pub fingerprint: String,
    /// False after any damaged record or missing/invalid footer.
    pub complete: bool,
}

/// Stream a snapshot's usable prefix: each verified object whose digest
/// `wanted` holds is handed to `adopt` with that digest, once, and dropped
/// before the next record is read (merkle-spec §6.9).
///
/// # Errors
/// A missing/incompatible/corrupt header, or what `adopt` returns. Record
/// corruption and interrupted tails end as `complete: false`.
pub fn scan(
    mut input: impl Read,
    workspace: &Path,
    wanted: &BTreeSet<[u8; 32]>,
    mut adopt: impl FnMut(&[u8; 32], Object) -> io::Result<()>,
) -> io::Result<Scanned> {
    let (fingerprint, header) = read_header(&mut input, workspace)?;
    let mut adopted = BTreeSet::new();
    let complete = records(&mut input, &header, wanted, |tag, digest, _sum, payload| {
        let object = match tag {
            0 => crate::decode(&payload, digest).map(Object::Doc),
            1 => String::from_utf8(payload)
                .ok()
                .filter(|why| why.starts_with("is not UTF-8 ("))
                .map(Object::Unserved),
            _ => return Ok(payload.is_empty()),
        };
        let Some(object) = object else {
            return Ok(false);
        };
        if adopted.insert(*digest) {
            adopt(digest, object)?;
        }
        Ok(true)
    })?;
    Ok(Scanned {
        fingerprint,
        complete,
    })
}

/// Decode a batch of raw verified records in parallel across worker threads,
/// keeping memory bounded per batch while utilizing multiple cores for
/// [`crate::decode`] + per-node revision verification.
fn decode_batch(batch: Vec<(u8, [u8; 32], Vec<u8>)>) -> (Vec<([u8; 32], Object)>, bool) {
    const PARALLEL_DECODE_FLOOR: usize = 16;
    if batch.len() < PARALLEL_DECODE_FLOOR {
        let mut out = Vec::with_capacity(batch.len());
        let mut all_ok = true;
        for (tag, digest, payload) in batch {
            match decode_record(tag, &digest, payload) {
                RecordDecoded::Object(obj) => out.push((digest, obj)),
                RecordDecoded::Abstain => {}
                RecordDecoded::Corrupt => all_ok = false,
            }
        }
        return (out, all_ok);
    }
    let workers = std::thread::available_parallelism()
        .map_or(2, |n| n.get().clamp(2, 8))
        .min(batch.len());
    let queue = std::sync::Mutex::new(batch.into_iter());
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                scope.spawn(|| {
                    let mut local = Vec::new();
                    let mut local_ok = true;
                    loop {
                        let next = queue
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .next();
                        let Some((tag, digest, payload)) = next else {
                            break;
                        };
                        match decode_record(tag, &digest, payload) {
                            RecordDecoded::Object(obj) => local.push((digest, obj)),
                            RecordDecoded::Abstain => {}
                            RecordDecoded::Corrupt => local_ok = false,
                        }
                    }
                    (local, local_ok)
                })
            })
            .collect();
        let mut out = Vec::new();
        let mut all_ok = true;
        for handle in handles {
            match handle.join() {
                Ok((local, ok)) => {
                    out.extend(local);
                    all_ok &= ok;
                }
                Err(panic) => std::panic::resume_unwind(panic),
            }
        }
        (out, all_ok)
    })
}

enum RecordDecoded {
    Object(Object),
    Abstain,
    Corrupt,
}

fn decode_record(tag: u8, digest: &[u8; 32], payload: Vec<u8>) -> RecordDecoded {
    match tag {
        0 => match crate::decode(&payload, digest) {
            Some(doc) => RecordDecoded::Object(Object::Doc(doc)),
            None => RecordDecoded::Corrupt,
        },
        1 => match String::from_utf8(payload)
            .ok()
            .filter(|why| why.starts_with("is not UTF-8 ("))
        {
            Some(why) => RecordDecoded::Object(Object::Unserved(why)),
            None => RecordDecoded::Corrupt,
        },
        _ => {
            if payload.is_empty() {
                RecordDecoded::Abstain
            } else {
                RecordDecoded::Corrupt
            }
        }
    }
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
    const DECODE_BATCH: usize = 128;
    let wanted: BTreeSet<_> = fresh.values().copied().collect();
    let (fingerprint, header) = read_header(&mut input, workspace)?;
    let mut objects = BTreeMap::new();
    let mut invalid_utf8 = BTreeMap::new();
    let mut seen_digests = BTreeSet::new();
    let mut batch: Vec<(u8, [u8; 32], Vec<u8>)> = Vec::with_capacity(DECODE_BATCH);
    let mut decode_sound = true;
    let flush = |batch: &mut Vec<(u8, [u8; 32], Vec<u8>)>,
                 objects: &mut BTreeMap<[u8; 32], Arc<model::Document>>,
                 invalid_utf8: &mut BTreeMap<[u8; 32], String>,
                 decode_sound: &mut bool| {
        if batch.is_empty() {
            return;
        }
        let (decoded, ok) = decode_batch(std::mem::take(batch));
        *decode_sound &= ok;
        for (digest, object) in decoded {
            match object {
                Object::Doc(doc) => {
                    objects.entry(digest).or_insert_with(|| Arc::new(doc));
                }
                Object::Unserved(why) => {
                    invalid_utf8.entry(digest).or_insert(why);
                }
            }
        }
    };
    let stream_sound = records(
        &mut input,
        &header,
        &wanted,
        |tag, digest, _sum, payload| {
            if tag == 2 {
                return Ok(payload.is_empty());
            }
            if seen_digests.insert(*digest) {
                batch.push((tag, *digest, payload));
                if batch.len() >= DECODE_BATCH {
                    flush(
                        &mut batch,
                        &mut objects,
                        &mut invalid_utf8,
                        &mut decode_sound,
                    );
                }
            }
            Ok(true)
        },
    )?;
    flush(
        &mut batch,
        &mut objects,
        &mut invalid_utf8,
        &mut decode_sound,
    );
    let complete = stream_sound && decode_sound;
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
