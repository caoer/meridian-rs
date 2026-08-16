//! The §6.5 disposable checkpoint — FORMAT ONLY (merkle-spec §6.5; ruled by
//! `decisions/2026-08-15-restart-index-allowed.md`, superseding the
//! memory-only direction).
//!
//! Serializes the resident memo ([`crate::DomainCache`]'s leaf rows — path,
//! [`crate::StatKey`], §6.2 watermark, §12.2 leaf digest) under the identity
//! tuple, and restores it whole-or-not-at-all. This module touches no disk:
//! placement, atomic writes, journal replay, and the labeled re-baseline are
//! the registry's (the crate charter's "no snapshot files" stands here —
//! bytes in, bytes out; the §0 scoping note governs the object itself).
//!
//! # Two questions, two instruments — the ratified identity law
//! **A checkpoint is an observation record from a dead epoch: a claim about
//! the past, never about the present** (`results/checkpoint-design-ruling.md`
//! § I). Its trust splits into two independent questions, and conflating them
//! is the exact hazard the §6.5 caution names.
//!
//! - **Lawfulness — may these rows enter the memo as HYPOTHESES?** Decided
//!   once, here, at restore, by the SOUNDNESS fields: format and checksum,
//!   `workspace`, `domain_version`, `hash_law`, parse-cache generation, and
//!   the `tree_root` binding over the restored rows. A mismatch means the rows
//!   are not lawful statements about this world under today's law — a hash-law
//!   change re-spells every interior value, a domain bump re-cuts membership, a
//!   failed binding is tampering. Any mismatch discards the object WHOLE,
//!   loudly, exactly once, labeled per field ([`Discard`]). Partial adoption
//!   does not exist. This is the **COLD** re-baseline.
//! - **Currency — may a row SERVE?** Never answered here, and never answered
//!   wholesale. Every restored row serves only through the §6.2 watermark
//!   trust close — the SAME live protocol that governs a RAM-resident row. A
//!   restored row has exactly the trust class of a resident row whose stat
//!   evidence is stale: none, until the live instrument confirms it. That is
//!   "cannot serve stale by construction" said mechanically, and it is why
//!   [`restore`] marks the memo unobserved (`DomainCache::guard_currency`
//!   answers UNTRUSTED until a completed observation).
//!
//! **The journal pair is neither — it is a CURSOR.** The landed cursor law
//! governs it (§6.3, from plan §4.9): "hash tokens are epoch-free; cursors are
//! not… instance mismatch degrades to the content-fold compare". A cursor that
//! anchors buys REPLAY; a cursor that cannot anchor — certain across process
//! death, since §7.1 persists no epoch fact — forfeits replay ONLY, and the
//! rows still enter as hypotheses. Trust never rides the cursor in either
//! direction, so a cursor mismatch is the **WARM** re-baseline: re-establishing
//! currency from zero TRUST, not from zero BYTES. [`restore`] returns the
//! cursor unjudged; the caller adjudicates it against the live journal.
//!
//! The vacuity trap the name caused, recorded so it is not re-derived:
//! reading the pair as an identity FIELD makes the object discard on every
//! ordinary restart, so it never fires. The plan never does that — the
//! git-index class it named discards nothing at a gap, it re-verifies by stat.
//!
//! # Trust standing of what restores
//! Restored rows carry the same evidence grade a live memo holds: digests
//! byte-derived under a recorded [`crate::StatKey`] identity and §6.2
//! watermark. The next observation re-verifies every member's identity
//! against disk exactly as it always has — nothing is SERVED from the
//! checkpoint without the normal currency law running, which is why the
//! object cannot serve stale by construction. The journal cursor names the
//! replay point for changes made while down; replay itself is the caller's
//! ([`crate::DomainCache::overlay_leaf`] / `overlay_remove` — conservative,
//! spoiled identities).
//!
//! # Format (version-tagged lines; paths are raw bytes, `\n`-free)
//! ```text
//! mrd-checkpoint v1
//! sum <blake3 hex of every byte after this line>
//! workspace <canonical root path bytes>
//! version <domain_version> <hash_law>
//! parse-cache <generation>
//! journal <seq> <instance>
//! tree-root <hex64 law-2 fingerprint>
//! leaves <count>
//! <dev> <ino> <size> <mt_s>.<mt_ns> <ct_s>.<ct_ns> <seen_s>.<seen_ns>|- <hex64> <path bytes>
//! ```
//! Unlike [`crate::digestmemo`] (per-row evidence cache, degrades row-wise),
//! this object discards WHOLE: any header, checksum, count, row, or
//! tree-root defect is a loud [`Discard`], never a partial adoption.

use std::fmt;
use std::os::unix::ffi::OsStrExt as _;
use std::path::{Path, PathBuf};

use crate::domain::Domain;
use crate::stable::{self, FsStamp};
use crate::{DomainCache, LeafSeen, StatKey};

/// The format tag. Bump on any shape change: an alien tag is a
/// [`Discard::Format`] — the caller re-baselines, never guesses.
const VERSION_LINE: &[u8] = b"mrd-checkpoint v1";

/// The save-side identity the CALLER supplies. The tuple's remaining fields
/// come from the memo itself: `domain_version` from the observed generation,
/// `hash_law` from the resident tree's encoding ([`model::HASH_LAW_RADIX`]),
/// `tree_root` from the law-2 fold.
#[derive(Debug, Clone)]
pub struct SaveIdentity {
    /// The canonical workspace root — the engine's workspace identity (the
    /// registry key; no separate uuid exists in this engine). Compared as
    /// raw path bytes.
    pub workspace: PathBuf,
    /// The parse/derived-cache schema generation this engine persists derived
    /// state under (`cache::SCHEMA_SALT` at the daemon). A schema bump
    /// discards the checkpoint with it.
    pub parse_cache: String,
    /// The journal (delta-ring) instance the cursor was numbered under.
    pub journal_instance: String,
    /// The journal cursor: everything at or before this seq is inside the
    /// serialized tree. Capture it BEFORE snapshotting the memo — an
    /// under-claimed cursor only re-replays (idempotent overlay); an
    /// over-claimed one would skip a change.
    pub journal_seq: u64,
}

/// A restored checkpoint: the rebuilt memo plus the CURSOR the caller must
/// adjudicate (module docs — lawfulness is settled here, replay is not).
#[derive(Debug)]
pub struct Restored {
    /// The rebuilt resident memo: leaf rows, radix tree re-composed and
    /// verified against the stored `tree_root`, observed domain installed,
    /// marked unobserved so nothing serves before the §6.2 floor runs.
    /// Calibration, feed generation, and counters start fresh.
    pub cache: DomainCache,
    /// The epoch the cursor was numbered under. The caller compares it
    /// against the LIVE journal: equal ⇒ the cursor may anchor and replay is
    /// legal; different ⇒ the numbering died (restart, reap) and the caller
    /// logs the WARM re-baseline — replay forfeited, rows retained.
    pub journal_instance: String,
    /// Replay from here: the caller re-derives every journaled change after
    /// this seq. A cursor outside retained history cannot anchor — the
    /// caller's journal owns that verdict.
    pub journal_seq: u64,
    /// Leaf rows restored (receipt surface).
    pub leaves: usize,
}

/// Why a checkpoint was refused, whole. Every arm is a labeled re-baseline
/// cause; [`fmt::Display`] is the label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Discard {
    /// Structural defect: alien tag, checksum mismatch, torn or unparseable
    /// row, row-count mismatch.
    Format(&'static str),
    /// The stored workspace is not this workspace.
    Workspace {
        /// Stored path, display-spelled.
        stored: String,
    },
    /// The stored `(domain_version, hash_law)` is not the serving pair —
    /// the checkpoint must never outlive either dimension (b02: the pair,
    /// never the flattened ladder position, so one bump of each can never
    /// compare equal).
    Version {
        stored: (u32, u32),
        current: (u32, u32),
    },
    /// The stored parse/derived-cache generation is not this engine's.
    ParseCache { stored: String, current: String },
    /// The rebuilt tree's law-2 fingerprint is not the stored `tree_root`.
    TreeRoot,
}

impl fmt::Display for Discard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Discard::Format(what) => write!(f, "format defect ({what})"),
            Discard::Workspace { stored } => {
                write!(f, "workspace identity mismatch (stored '{stored}')")
            }
            Discard::Version { stored, current } => write!(
                f,
                "version identity mismatch (stored domain {} hash-law {}, serving domain {} \
                 hash-law {})",
                stored.0, stored.1, current.0, current.1
            ),
            Discard::ParseCache { stored, current } => write!(
                f,
                "parse-cache generation mismatch (stored '{stored}', current '{current}')"
            ),
            Discard::TreeRoot => write!(f, "tree-root binding failed (rebuilt fold differs)"),
        }
    }
}

/// Serialize the memo under `identity`. `None` when there is nothing sound to
/// persist: no observation baseline yet, or a member path carrying a newline
/// (unencodable in a line format — skipping it would break the tree-root
/// binding, so the whole save abstains).
#[must_use]
pub fn serialize(cache: &mut DomainCache, identity: &SaveIdentity) -> Option<Vec<u8>> {
    let domain_version = cache.domain_seen.as_ref()?.version();
    // The §6.2 smudge (`decisions/2026-08-16-gate11-stat-floor.md` Repair 2,
    // format consequence). `StatKey` alone can miss a same-instant, same-size,
    // in-place write, so a row that is RACILY CLEAN at save — its recorded
    // stamps inside one calibrated granularity unit of its own observation
    // watermark — must never be trusted on a stat-match after restore.
    //
    // Of the two mechanisms the law allows, this format takes the SMUDGE
    // (git's): such rows serialize with their watermark spoiled, so the
    // restored memo re-reads them whatever the restoring process measures.
    // The alternative — persisting the watermark plus the granularity unit —
    // would leave the decision to a granule measured by a different process;
    // spoiling decides it here, with the evidence that produced the row.
    //
    // No calibration means no granule to judge racy-ness with, so every row
    // spoils: the §6.2 floor is never silent trust.
    let granule = match &cache.calibration {
        Some(stable::Calibration::Measured { granule_ns }) => Some(*granule_ns),
        _ => None,
    };
    let tree_root = cache.law2_fingerprint();
    let mut payload: Vec<u8> = Vec::with_capacity(256 + cache.leaves.len() * 128);
    payload.extend_from_slice(b"workspace ");
    payload.extend_from_slice(identity.workspace.as_os_str().as_bytes());
    payload.push(b'\n');
    payload.extend_from_slice(
        format!("version {domain_version} {}\n", model::HASH_LAW_RADIX).as_bytes(),
    );
    payload.extend_from_slice(format!("parse-cache {}\n", identity.parse_cache).as_bytes());
    payload.extend_from_slice(
        format!(
            "journal {} {}\n",
            identity.journal_seq, identity.journal_instance
        )
        .as_bytes(),
    );
    payload.extend_from_slice(format!("tree-root {}\n", hex(&tree_root)).as_bytes());
    payload.extend_from_slice(format!("leaves {}\n", cache.leaves.len()).as_bytes());
    for (rel, entry) in &cache.leaves {
        let bytes = rel.as_os_str().as_bytes();
        if bytes.contains(&b'\n') {
            return None;
        }
        let (dev, ino, size, mtime, ctime) = entry.key.raw_parts();
        // Spoil on save when the row is racily clean, or when nothing
        // calibrated says otherwise (see the smudge note above).
        let seen = match entry.seen {
            Some(stamp) if granule.is_some_and(|g| !stable::racy(&entry.key, stamp, g)) => {
                format!("{}.{}", stamp.0, stamp.1)
            }
            _ => "-".to_owned(),
        };
        payload.extend_from_slice(
            format!(
                "{dev} {ino} {size} {}.{} {}.{} {seen} {} ",
                mtime.0,
                mtime.1,
                ctime.0,
                ctime.1,
                hex(&entry.digest)
            )
            .as_bytes(),
        );
        payload.extend_from_slice(bytes);
        payload.push(b'\n');
    }
    let mut out = Vec::with_capacity(payload.len() + 80);
    out.extend_from_slice(VERSION_LINE);
    out.push(b'\n');
    out.extend_from_slice(format!("sum {}\n", blake3::hash(&payload).to_hex()).as_bytes());
    out.extend_from_slice(&payload);
    Some(out)
}

/// Restore a serialized checkpoint against the CURRENT soundness identity:
/// `workspace` and `parse_cache` as the caller serves them now, `domain`
/// freshly loaded (its `version()` is the current domain dimension; the
/// current hash law is the resident encoding's constant). On success the
/// rebuilt tree is verified against the stored `tree_root` and `domain` is
/// installed as the observed generation, so the overlay doors are open for
/// the caller's journal replay.
///
/// The stored journal pair rides out on [`Restored`] unjudged — replay
/// authority is the caller's adjudication, never this module's (module docs,
/// the tuple split).
///
/// # Errors
/// [`Discard`] naming the refusing fact. The caller re-baselines loudly,
/// exactly once — partial adoption does not exist.
pub fn restore(
    bytes: &[u8],
    workspace: &Path,
    parse_cache: &str,
    domain: &Domain,
) -> Result<Restored, Discard> {
    let rest = bytes
        .strip_prefix(VERSION_LINE)
        .and_then(|r| r.strip_prefix(b"\n"))
        .ok_or(Discard::Format("unknown version tag"))?;
    let (sum_line, payload) = split_line(rest).ok_or(Discard::Format("missing checksum"))?;
    let sum = sum_line
        .strip_prefix(b"sum ")
        .ok_or(Discard::Format("missing checksum"))?;
    if blake3::hash(payload).to_hex().as_bytes() != sum {
        return Err(Discard::Format("checksum mismatch"));
    }

    let mut lines = payload;
    let stored_ws = header_line(&mut lines, b"workspace ", "bad workspace line")?;
    let version_line = header_line(&mut lines, b"version ", "bad version line")?;
    let stored_parse = header_line(&mut lines, b"parse-cache ", "bad parse-cache line")?;
    let journal_line = header_line(&mut lines, b"journal ", "bad journal line")?;
    let tree_root_hex = header_line(&mut lines, b"tree-root ", "bad tree-root line")?;
    let count: usize = ascii(header_line(&mut lines, b"leaves ", "bad leaves line")?)
        .ok_or(Discard::Format("bad leaves count"))?;

    // Identity gates, each its own labeled fact — cheap fields first, the
    // tree rebuild last.
    if stored_ws != workspace.as_os_str().as_bytes() {
        return Err(Discard::Workspace {
            stored: String::from_utf8_lossy(stored_ws).into_owned(),
        });
    }
    let (stored_domain, stored_law) =
        parse_version(version_line).ok_or(Discard::Format("bad version pair"))?;
    let current = (domain.version(), model::HASH_LAW_RADIX);
    if (stored_domain, stored_law) != current {
        return Err(Discard::Version {
            stored: (stored_domain, stored_law),
            current,
        });
    }
    if stored_parse != parse_cache.as_bytes() {
        return Err(Discard::ParseCache {
            stored: String::from_utf8_lossy(stored_parse).into_owned(),
            current: parse_cache.to_owned(),
        });
    }
    let (journal_seq, journal_instance) =
        parse_journal(journal_line).ok_or(Discard::Format("bad journal line"))?;
    let tree_root = parse_hex32(tree_root_hex).ok_or(Discard::Format("bad tree-root value"))?;

    let mut cache = DomainCache::default();
    let mut rows = 0usize;
    while !lines.is_empty() {
        let (line, rest) = split_line(lines).ok_or(Discard::Format("unterminated row"))?;
        lines = rest;
        let (rel, entry) = parse_row(line).ok_or(Discard::Format("unparseable row"))?;
        cache.tree.set_leaf(&rel, entry.digest);
        cache.leaves.insert(rel, entry);
        rows += 1;
    }
    if rows != count {
        return Err(Discard::Format("row count mismatch"));
    }
    if cache.law2_fingerprint() != tree_root {
        return Err(Discard::TreeRoot);
    }
    cache.domain_seen = Some(domain.clone());
    // The rows are a previous process's evidence: guard-grade currency stays
    // UNTRUSTED until a live observation re-verifies every member (§6.5's
    // "cannot serve stale by construction"). `domain_seen` is set because the
    // overlay doors compose against the observed generation — it must not be
    // mistaken for an observation having landed.
    cache.restored_unobserved = true;
    Ok(Restored {
        cache,
        journal_instance,
        journal_seq,
        leaves: rows,
    })
}

/// Take one `<prefix><value>` header line off `lines`, advancing past it.
fn header_line<'a>(
    lines: &mut &'a [u8],
    prefix: &[u8],
    defect: &'static str,
) -> Result<&'a [u8], Discard> {
    let (line, rest) = split_line(lines).ok_or(Discard::Format("truncated header"))?;
    *lines = rest;
    line.strip_prefix(prefix).ok_or(Discard::Format(defect))
}

/// One row: the digestmemo shape plus the §6.2 watermark field, path as the
/// raw-byte tail (spaces legal, `\n` impossible by construction).
fn parse_row(line: &[u8]) -> Option<(PathBuf, LeafSeen)> {
    let mut fields = line.splitn(8, |b| *b == b' ');
    let dev: u64 = ascii(fields.next()?)?;
    let ino: u64 = ascii(fields.next()?)?;
    let size: u64 = ascii(fields.next()?)?;
    let mtime = parse_secs_nanos(fields.next()?)?;
    let ctime = parse_secs_nanos(fields.next()?)?;
    let seen_field = fields.next()?;
    let seen: Option<FsStamp> = if seen_field == b"-" {
        None
    } else {
        Some(parse_secs_nanos(seen_field)?)
    };
    let digest = parse_hex32(fields.next()?)?;
    let rel = fields.next()?;
    if rel.is_empty() {
        return None;
    }
    Some((
        PathBuf::from(std::ffi::OsStr::from_bytes(rel)),
        LeafSeen {
            key: StatKey::from_raw_parts(dev, ino, size, mtime, ctime),
            digest,
            seen,
        },
    ))
}

/// `version <domain> <law>`'s pair.
fn parse_version(line: &[u8]) -> Option<(u32, u32)> {
    let mut fields = line.splitn(2, |b| *b == b' ');
    Some((ascii(fields.next()?)?, ascii(fields.next()?)?))
}

/// `journal <seq> <instance>` — instance is the tail (opaque, echoed).
fn parse_journal(line: &[u8]) -> Option<(u64, String)> {
    let mut fields = line.splitn(2, |b| *b == b' ');
    let seq: u64 = ascii(fields.next()?)?;
    let instance = std::str::from_utf8(fields.next()?).ok()?;
    Some((seq, instance.to_owned()))
}

/// `<i64>.<i64>` seconds.nanos, signed as `stat` reports them.
fn parse_secs_nanos(field: &[u8]) -> Option<(i64, i64)> {
    let text = std::str::from_utf8(field).ok()?;
    let (secs, nanos) = text.split_once('.')?;
    Some((secs.parse().ok()?, nanos.parse().ok()?))
}

/// Parse an ASCII integer field.
fn ascii<T: std::str::FromStr>(field: &[u8]) -> Option<T> {
    std::str::from_utf8(field).ok()?.parse().ok()
}

/// 64 lowercase hex chars → 32 bytes.
fn parse_hex32(field: &[u8]) -> Option<[u8; 32]> {
    if field.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, chunk) in field.chunks(2).enumerate() {
        let hi = (chunk[0] as char).to_digit(16)?;
        let lo = (chunk[1] as char).to_digit(16)?;
        out[i] = u8::try_from(hi * 16 + lo).ok()?;
    }
    Some(out)
}

/// 32 bytes → 64 lowercase hex chars.
fn hex(bytes: &[u8; 32]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(64);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Split at the first `\n`: `(line without the newline, everything after)`.
fn split_line(bytes: &[u8]) -> Option<(&[u8], &[u8])> {
    let at = bytes.iter().position(|b| *b == b'\n')?;
    Some((&bytes[..at], &bytes[at + 1..]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WorkspaceRoot;

    fn identity(dir: &Path) -> SaveIdentity {
        SaveIdentity {
            workspace: dir.to_path_buf(),
            parse_cache: "s7".to_owned(),
            journal_instance: "aa.bb.0".to_owned(),
            journal_seq: 41,
        }
    }

    /// A three-member corpus observed into a memo, plus its serialized
    /// checkpoint.
    fn observed(dir: &Path) -> (DomainCache, Vec<u8>) {
        std::fs::write(dir.join("a.md"), "# A\n").unwrap();
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/b c.md"), "# B\n").unwrap();
        std::fs::write(dir.join("sub/d.md"), "# D\n").unwrap();
        let mut cache = DomainCache::new();
        cache.root(&WorkspaceRoot(dir.to_path_buf())).unwrap();
        let bytes = serialize(&mut cache, &identity(dir)).expect("baselined memo serializes");
        (cache, bytes)
    }

    /// Recompute the `sum` line after a deliberate payload edit, so tamper
    /// tests reach the arm BEHIND the checksum.
    fn resum(bytes: &[u8]) -> Vec<u8> {
        let rest = bytes.strip_prefix(b"mrd-checkpoint v1\n").unwrap();
        let (_, payload) = split_line(rest).unwrap();
        let mut out = b"mrd-checkpoint v1\n".to_vec();
        out.extend_from_slice(format!("sum {}\n", blake3::hash(payload).to_hex()).as_bytes());
        out.extend_from_slice(payload);
        out
    }

    fn restore_here(bytes: &[u8], dir: &Path) -> Result<Restored, Discard> {
        let domain = Domain::load(&WorkspaceRoot(dir.to_path_buf())).unwrap();
        restore(bytes, dir, "s7", &domain)
    }

    /// Round trip: the restored memo carries the same leaves, binds the same
    /// law-2 fingerprint, serves the same law-1 root without I/O, and its
    /// journal cursor is the saved one.
    #[test]
    fn round_trips_whole() {
        let tmp = tempfile::tempdir().unwrap();
        let (mut cache, bytes) = observed(tmp.path());
        let mut back = restore_here(&bytes, tmp.path()).expect("identity matches");
        assert_eq!(back.journal_seq, 41);
        assert_eq!(
            back.journal_instance, "aa.bb.0",
            "the cursor rides out unjudged for the caller to adjudicate"
        );
        assert_eq!(back.leaves, 3);
        assert_eq!(back.cache.leaf_digests(), cache.leaf_digests());
        assert_eq!(back.cache.law2_fingerprint(), cache.law2_fingerprint());
        // The overlay door is open (observed domain installed) and the
        // served law-1 root folds from restored leaves with zero reads.
        assert_eq!(
            back.cache.overlay_root().unwrap(),
            cache.overlay_root().unwrap()
        );
        assert_eq!(back.cache.leaves_read(), 0, "restore reads no bytes");
    }

    /// Restored `StatKey` + watermark evidence serves the next observation
    /// exactly as the live memo's would — pass-for-pass read parity, the
    /// O(changes-while-down) point without asserting the backend's stamp
    /// granularity (racy records re-read identically on both sides).
    #[test]
    fn restored_evidence_matches_a_live_memo_pass_for_pass() {
        let tmp = tempfile::tempdir().unwrap();
        let root = WorkspaceRoot(tmp.path().to_path_buf());
        let (mut live, _) = observed(tmp.path());
        // Let the records settle past a ms-class stamp quantum, then
        // re-observe so the serialized watermarks are trust-clean.
        std::thread::sleep(std::time::Duration::from_millis(30));
        live.root(&root).unwrap();
        let bytes = serialize(&mut live, &identity(tmp.path())).expect("baselined");
        let mut back = restore_here(&bytes, tmp.path()).unwrap().cache;
        let live_before = live.leaves_read();
        let live_root = live.root(&root).unwrap();
        let live_pass = live.leaves_read() - live_before;
        assert_eq!(back.root(&root).unwrap(), live_root);
        assert_eq!(
            back.leaves_read(),
            live_pass,
            "restored evidence serves exactly as the live memo's"
        );
    }

    /// Every SOUNDNESS field refuses with its own labeled arm — the fields
    /// that can make a restored row wrong.
    #[test]
    fn each_soundness_field_discards() {
        let tmp = tempfile::tempdir().unwrap();
        let (_, bytes) = observed(tmp.path());
        let domain = Domain::load(&WorkspaceRoot(tmp.path().to_path_buf())).unwrap();
        assert!(matches!(
            restore(&bytes, Path::new("/else/where"), "s7", &domain),
            Err(Discard::Workspace { .. })
        ));
        assert!(matches!(
            restore(&bytes, tmp.path(), "s8", &domain),
            Err(Discard::ParseCache { .. })
        ));
        // Domain-version bump: the config declares a new version — the
        // checkpoint must not outlive it (§12.1 membership is re-cut).
        std::fs::create_dir_all(tmp.path().join("meridian")).unwrap();
        std::fs::write(
            tmp.path().join("meridian/domain.md"),
            "---\nversion: 2\n---\n",
        )
        .unwrap();
        let bumped = Domain::load(&WorkspaceRoot(tmp.path().to_path_buf())).unwrap();
        assert_eq!(bumped.version(), 2);
        assert!(matches!(
            restore(&bytes, tmp.path(), "s7", &bumped),
            Err(Discard::Version {
                stored: (0, model::HASH_LAW_RADIX),
                current: (2, model::HASH_LAW_RADIX),
            })
        ));
    }

    /// A hash-law bump discards loudly: the checkpoint can never outlive the
    /// interior-encoding law it was folded under (the card's own gate).
    #[test]
    fn a_hash_law_bump_discards_loudly() {
        let tmp = tempfile::tempdir().unwrap();
        let (_, bytes) = observed(tmp.path());
        let text = String::from_utf8(bytes).unwrap();
        let old = format!("version 0 {}", model::HASH_LAW_RADIX);
        let forged = resum(text.replace(&old, "version 0 3").as_bytes());
        let got = restore_here(&forged, tmp.path());
        assert_eq!(
            got.unwrap_err(),
            Discard::Version {
                stored: (0, 3),
                current: (0, model::HASH_LAW_RADIX),
            }
        );
    }

    /// Structural defects discard whole: alien tag, flipped payload byte,
    /// torn tail, row-count lie. Never a partial adoption.
    #[test]
    fn structural_defects_discard_whole() {
        let tmp = tempfile::tempdir().unwrap();
        let (_, bytes) = observed(tmp.path());
        for (case, mutation) in [
            ("alien tag", b"mrd-checkpoint v9\n".to_vec()),
            ("empty", Vec::new()),
        ] {
            let got = restore_here(&mutation, tmp.path());
            assert!(matches!(got, Err(Discard::Format(_))), "{case}");
        }
        let mut flipped = bytes.clone();
        let last = flipped.len() - 2;
        flipped[last] ^= 0x01;
        assert!(matches!(
            restore_here(&flipped, tmp.path()),
            Err(Discard::Format("checksum mismatch"))
        ));
        let torn = resum(&bytes[..bytes.len() - 10]);
        assert!(matches!(
            restore_here(&torn, tmp.path()),
            Err(Discard::Format(_))
        ));
    }

    /// The tree-root binding: a tampered digest row (checksum recomputed)
    /// fails the law-2 rebuild — the fold audits the rows.
    #[test]
    fn tree_root_binding_catches_a_tampered_row() {
        let tmp = tempfile::tempdir().unwrap();
        let (cache, bytes) = observed(tmp.path());
        let digest = hex(&cache.leaf_digests()[Path::new("a.md")]);
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains(&digest));
        let forged = resum(
            text.replace(&digest, &format!("{}{}", &digest[2..], &digest[..2]))
                .as_bytes(),
        );
        assert_eq!(
            restore_here(&forged, tmp.path()).unwrap_err(),
            Discard::TreeRoot
        );
    }

    /// **It cannot serve stale by construction.** A restored memo answers
    /// guard-grade currency UNTRUSTED until a live observation re-verifies
    /// every member — the rows are a previous process's evidence, and the
    /// §6.4 watcher started after the gap, so nothing can vouch for what
    /// changed while the daemon was down. Without this the first
    /// `currency_refresh` after a restart would fold restored rows straight
    /// into a guard answer.
    #[test]
    fn a_restored_memo_refuses_guard_currency_until_it_observes() {
        let tmp = tempfile::tempdir().unwrap();
        let (_, bytes) = observed(tmp.path());
        let mut back = restore_here(&bytes, tmp.path()).unwrap().cache;

        let stable::GuardCurrency::Untrusted { reason } = back.guard_currency() else {
            panic!("a restored memo must not vouch for a gap it did not watch");
        };
        assert!(reason.contains("checkpoint"), "the reason names the cause");

        // A live pass re-verifies every member — and only then may the memo
        // answer a guard-grade question.
        back.root(&WorkspaceRoot(tmp.path().to_path_buf())).unwrap();
        assert_eq!(back.guard_currency(), stable::GuardCurrency::Trusted);
    }

    /// An unbaselined memo has nothing sound to persist.
    #[test]
    fn an_unbaselined_memo_abstains() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cold = DomainCache::new();
        assert!(serialize(&mut cold, &identity(tmp.path())).is_none());
    }
}
