//! B-04 durable cutover authority state — the per-workspace record that
//! decides which hash LAW may serve (merkle-spec §4.2.6).
//!
//! The record lives at `.meridian/cutover` — a dot-segment path, outside the
//! hash domain by the §12.1 structural floor, beside `write.lock` and
//! `intents/`. It is checksummed as one unit (BLAKE3, the [`intent`]
//! convention: checksum is the last line over every preceding byte) and paid
//! in the ruled plain-fsync class ([`crate::honest_sync`] — never
//! `F_FULLFSYNC`).
//!
//! Two durable states only. An ABSENT record is the pre-cutover world: law 1
//! serves and retirement is unreachable by construction
//! (`model::ServingVersion::law1`). `NEW_BUILDING` means the no-return
//! boundary was crossed: the engine never serves or computes the retired
//! tree again — it resumes or restarts the new build. `NEW_COMMITTED` means
//! the new law serves. DRAINING is deliberately NOT a durable state: a crash
//! during the drain leaves no record, and the old law still serves — exactly
//! the pre-boundary crash row of the priced protocol (§10).
//!
//! **The no-return boundary is ONE durable act**: [`begin_new_building`]'s
//! rename reaching disk with its parent directory synced. The write is
//! tmp + sync + rename so a crash mid-write leaves only a tmp leftover and
//! an ABSENT record — the old law serves, which is the correct side of the
//! boundary. No fence tombstone rides this act (spec §4.2.5 amendment
//! 2026-08-16: the B-03 fence never activates).
//!
//! Every unrecognized, truncated, or foreign state is a loud typed
//! [`CutoverStateError`] — the same never-guess posture as [`crate::fence`]:
//! the engine must not decide old or new from whatever cache happens to
//! exist (`fingerprint_cutover_recovery_required` at the wire).

use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{WorkspaceRoot, honest_sync, honest_sync_path};

/// Version tag — line 1 of the record.
pub const FORMAT: &str = "MRD-CUTOVER/1";

/// The record's path relative to the workspace root.
fn record_path(root: &WorkspaceRoot) -> PathBuf {
    root.0.join(".meridian").join("cutover")
}

/// The two durable states (merkle-spec §4.2.6). `Ord`: a record only ever
/// advances.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CutoverState {
    /// Past the no-return boundary; the new tree is being built. The engine
    /// never serves or computes the retired tree again.
    NewBuilding,
    /// The new law serves.
    NewCommitted,
}

/// The decoded per-workspace cutover record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CutoverRecord {
    /// Durable state.
    pub state: CutoverState,
    /// Opaque stable identifier of the one cutover event — the id every
    /// refusal and terminal control teaches (`cutover_id` on the wire).
    pub cutover_id: String,
    /// The retiring identity: `(hash_law, domain)` at the boundary.
    pub from: model::RootVersion,
    /// The serving identity minted by the cutover: `(hash_law, domain)` at
    /// the boundary. Later domain bumps advance the domain and leave the
    /// boundary put ([`model::ServingVersion`]).
    pub to: model::RootVersion,
    /// The retired ring/tree instance, when the boundary OBSERVED one (a
    /// live drain). An at-open cutover cannot know a prior daemon's RAM-only
    /// instance and records none (wire-contract §4.7, the epoch bullet).
    pub old_instance: Option<String>,
    /// Unix milliseconds at [`begin_new_building`].
    pub started_unix_ms: u64,
    /// Unix milliseconds at [`commit`]; absent while `NEW_BUILDING`.
    pub committed_unix_ms: Option<u64>,
}

/// [`status`]'s answer: no record, or one recognized record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CutoverStatus {
    /// Pre-cutover: law 1 serves, retirement unreachable.
    Absent,
    /// A recognized record in one of the two durable states.
    Present(CutoverRecord),
}

/// The typed unrecognized-record refusal: the record exists but this engine
/// cannot certify which LAW may serve. Carried inside
/// `io::ErrorKind::InvalidData`; the wire spelling is
/// `fingerprint_cutover_recovery_required` (env class).
#[derive(Debug)]
pub struct CutoverStateError {
    found: String,
}

impl std::fmt::Display for CutoverStateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "unrecognized cutover record: {} — which hash law may serve is unknown; \
             serving is blocked (recover or remove .meridian/cutover from a known-good \
             state; the engine never guesses a law from surviving caches)",
            self.found
        )
    }
}

impl std::error::Error for CutoverStateError {}

/// Is this I/O error the typed unrecognized-cutover-record refusal?
#[must_use]
pub fn is_cutover_state_error(e: &io::Error) -> bool {
    e.get_ref()
        .is_some_and(|inner| inner.downcast_ref::<CutoverStateError>().is_some())
}

fn state_error(found: impl Into<String>) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        CutoverStateError {
            found: found.into(),
        },
    )
}

/// The typed serving-blocked-mid-build refusal: the record says
/// `NEW_BUILDING`, so the workspace is inside its cutover and nothing may
/// serve until the build completes. The daemon RESUMES the build at
/// workspace entry; any other door refuses loudly with this.
#[derive(Debug)]
pub struct CutoverInProgress {
    cutover_id: String,
}

impl std::fmt::Display for CutoverInProgress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "hash-law cutover {} is past its no-return boundary and still building — \
             the retired law never serves again; the daemon resumes the build at \
             workspace entry",
            self.cutover_id
        )
    }
}

impl std::error::Error for CutoverInProgress {}

/// Is this I/O error the typed mid-build refusal?
#[must_use]
pub fn is_cutover_in_progress(e: &io::Error) -> bool {
    e.get_ref()
        .is_some_and(|inner| inner.downcast_ref::<CutoverInProgress>().is_some())
}

/// Certify the cutover state of `root`. [`CutoverStatus::Absent`] for the
/// legal pre-cutover world (no `.meridian`, no record — a tmp leftover from
/// a crash before the rename is still Absent); a decoded record for the two
/// durable states. Anything else — truncation, foreign bytes, a checksum
/// mismatch, an unknown format or state word — errors loud
/// ([`CutoverStateError`]): a record check that guesses is worse than none.
///
/// # Errors
/// [`CutoverStateError`] (via `InvalidData`) on an unrecognized record; any
/// I/O failure reading it.
pub fn status(root: &WorkspaceRoot) -> io::Result<CutoverStatus> {
    let bytes = match fs::read(record_path(root)) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(CutoverStatus::Absent),
        Err(e) => return Err(e),
    };
    decode(&bytes).map(CutoverStatus::Present)
}

/// The serving hash-law identity `root`'s record dictates, law dimension
/// only — the caller supplies the live domain version
/// ([`ServingLaw::version`]). Absent ⇒ law 1, boundary 0 (retirement
/// unreachable by construction). `NEW_COMMITTED` ⇒ the record's `to` law at
/// its boundary. `NEW_BUILDING` ⇒ the typed [`CutoverInProgress`] refusal —
/// nothing serves mid-build.
///
/// # Errors
/// [`CutoverInProgress`] while `NEW_BUILDING`; [`CutoverStateError`] on an
/// unrecognized record; I/O reading it.
pub fn serving(root: &WorkspaceRoot) -> io::Result<ServingLaw> {
    match status(root)? {
        CutoverStatus::Absent => Ok(ServingLaw {
            hash_law: model::HASH_LAW_FLAT,
            law_boundary: 0,
        }),
        CutoverStatus::Present(rec) => {
            // Never-guess extends to the LAW dimension: a record naming a
            // law this binary cannot mint blocks serving loudly — minting
            // under a guessed encoding would be worse than not serving.
            if rec.to.hash_law != model::HASH_LAW_RADIX {
                return Err(state_error(format!(
                    "record names serving hash law {} — this engine mints law {} only",
                    rec.to.hash_law,
                    model::HASH_LAW_RADIX
                )));
            }
            match rec.state {
                CutoverState::NewCommitted => Ok(ServingLaw {
                    hash_law: rec.to.hash_law,
                    law_boundary: rec.to.ladder_position(),
                }),
                CutoverState::NewBuilding => Err(io::Error::other(CutoverInProgress {
                    cutover_id: rec.cutover_id,
                })),
            }
        }
    }
}

/// A workspace's serving law dimension — [`serving`]'s answer. The domain
/// dimension rides `meridian/domain.md` and moves independently; combine at
/// the point of use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServingLaw {
    /// The hash law that mints ([`model::HASH_LAW_FLAT`] or
    /// [`model::HASH_LAW_RADIX`]).
    pub hash_law: u32,
    /// The §12.3 ladder position where this law began to serve — 0 before
    /// any cutover.
    pub law_boundary: u32,
}

impl ServingLaw {
    /// The pre-cutover serving law: law 1 at boundary 0 — what an absent
    /// record dictates.
    #[must_use]
    pub const fn pre_cutover() -> Self {
        ServingLaw {
            hash_law: model::HASH_LAW_FLAT,
            law_boundary: 0,
        }
    }

    /// The full two-dimensional serving identity at the live `domain`
    /// version.
    #[must_use]
    pub const fn version(self, domain: u32) -> model::ServingVersion {
        model::ServingVersion {
            current: model::RootVersion {
                hash_law: self.hash_law,
                domain,
            },
            law_boundary: self.law_boundary,
        }
    }
}

/// Mint one opaque, stable cutover id: unix-millis, pid, and an in-process
/// counter, hex — unique per workspace history (one cutover ever per law
/// bump), never parsed.
#[must_use]
pub fn mint_cutover_id() -> String {
    static NTH: AtomicU64 = AtomicU64::new(0);
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
    format!(
        "cut-{ms:x}.{:x}.{:x}",
        std::process::id(),
        NTH.fetch_add(1, Ordering::Relaxed)
    )
}

/// Cross the no-return boundary: durably birth the record at
/// `NEW_BUILDING`. Refuses `AlreadyExists` when any record is already
/// present (one cutover per workspace per law bump — the caller reads
/// [`status`] first and holds the write exclusion across drain → here).
///
/// tmp + [`honest_sync`] + rename + parent [`honest_sync_path`]: the rename
/// reaching disk IS the boundary. A crash before it leaves an ABSENT record
/// and the old law serving; after it, [`CutoverStatus::Present`] at
/// `NEW_BUILDING` and the old law never serves again.
///
/// # Errors
/// `AlreadyExists` when a record exists; I/O/sync failure at any step.
pub fn begin_new_building(root: &WorkspaceRoot, record: &CutoverRecord) -> io::Result<()> {
    if record.state != CutoverState::NewBuilding {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "begin_new_building takes a NEW_BUILDING record",
        ));
    }
    let path = record_path(root);
    if path.symlink_metadata().is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "a cutover record already exists at this workspace",
        ));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    write_record(&path, record)
}

/// Advance the record to `NEW_COMMITTED` (tmp + sync + rename, the
/// [`intent`](crate::intent) update convention). Requires the durable state
/// to be `NEW_BUILDING`.
///
/// # Errors
/// `InvalidInput` when no `NEW_BUILDING` record is present or the handed
/// record is not `NEW_COMMITTED`; [`CutoverStateError`] on an unrecognized
/// record; I/O/sync failure.
pub fn commit(root: &WorkspaceRoot, record: &CutoverRecord) -> io::Result<()> {
    if record.state != CutoverState::NewCommitted {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "commit takes a NEW_COMMITTED record",
        ));
    }
    match status(root)? {
        CutoverStatus::Present(prior) if prior.state == CutoverState::NewBuilding => {}
        CutoverStatus::Present(_) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the record is already NEW_COMMITTED",
            ));
        }
        CutoverStatus::Absent => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "no NEW_BUILDING record to commit — the boundary was never crossed",
            ));
        }
    }
    write_record(&record_path(root), record)
}

/// tmp + `honest_sync` + rename + parent sync — the one write shape both
/// state advances share.
fn write_record(path: &std::path::Path, record: &CutoverRecord) -> io::Result<()> {
    let tmp = path.with_extension("tmp-cutover");
    {
        let mut f = fs::File::create(&tmp)?;
        io::Write::write_all(&mut f, encode(record).as_bytes())?;
        honest_sync(&f)?;
    }
    fs::rename(&tmp, path)?;
    if let Some(parent) = path.parent() {
        honest_sync_path(parent)?;
    }
    Ok(())
}

fn encode(record: &CutoverRecord) -> String {
    let mut body = String::new();
    let _ = writeln!(body, "{FORMAT}");
    let _ = writeln!(
        body,
        "state={}",
        match record.state {
            CutoverState::NewBuilding => "new_building",
            CutoverState::NewCommitted => "new_committed",
        }
    );
    let _ = writeln!(body, "cutover={}", record.cutover_id);
    let _ = writeln!(body, "from={} {}", record.from.hash_law, record.from.domain);
    let _ = writeln!(body, "to={} {}", record.to.hash_law, record.to.domain);
    if let Some(instance) = &record.old_instance {
        let _ = writeln!(body, "old_instance={instance}");
    }
    let _ = writeln!(body, "started_unix_ms={}", record.started_unix_ms);
    if let Some(ms) = record.committed_unix_ms {
        let _ = writeln!(body, "committed_unix_ms={ms}");
    }
    let sum = blake3::hash(body.as_bytes()).to_hex();
    let _ = writeln!(body, "checksum={sum}");
    body
}

fn decode(bytes: &[u8]) -> io::Result<CutoverRecord> {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return Err(state_error("non-UTF-8 bytes"));
    };
    let Some((body, checksum_line)) = text.rsplit_once("checksum=") else {
        return Err(state_error("no checksum line (truncated?)"));
    };
    if blake3::hash(body.as_bytes()).to_hex().as_str() != checksum_line.trim() {
        return Err(state_error(
            "checksum mismatch (truncated or foreign bytes)",
        ));
    }
    let mut lines = body.lines();
    match lines.next() {
        Some(FORMAT) => {}
        Some(other) => {
            return Err(state_error(format!(
                "format {other:?} — this engine knows {FORMAT} only"
            )));
        }
        None => return Err(state_error("empty record")),
    }
    let mut state = None;
    let mut cutover_id = None;
    let mut from = None;
    let mut to = None;
    let mut old_instance = None;
    let mut started_unix_ms = None;
    let mut committed_unix_ms = None;
    for line in lines {
        let Some((k, v)) = line.split_once('=') else {
            return Err(state_error(format!("line without '=': {line:?}")));
        };
        match k {
            "state" => {
                state = Some(match v {
                    "new_building" => CutoverState::NewBuilding,
                    "new_committed" => CutoverState::NewCommitted,
                    other => return Err(state_error(format!("unknown state {other:?}"))),
                });
            }
            "cutover" => cutover_id = Some(v.to_owned()),
            "from" => from = Some(parse_version(v)?),
            "to" => to = Some(parse_version(v)?),
            "old_instance" => old_instance = Some(v.to_owned()),
            "started_unix_ms" => {
                started_unix_ms = Some(
                    v.parse()
                        .map_err(|_| state_error(format!("unparseable started_unix_ms {v:?}")))?,
                );
            }
            "committed_unix_ms" => {
                committed_unix_ms =
                    Some(v.parse().map_err(|_| {
                        state_error(format!("unparseable committed_unix_ms {v:?}"))
                    })?);
            }
            other => return Err(state_error(format!("unknown field {other:?}"))),
        }
    }
    let (Some(state), Some(cutover_id), Some(from), Some(to), Some(started_unix_ms)) =
        (state, cutover_id, from, to, started_unix_ms)
    else {
        return Err(state_error("missing required field"));
    };
    Ok(CutoverRecord {
        state,
        cutover_id,
        from,
        to,
        old_instance,
        started_unix_ms,
        committed_unix_ms,
    })
}

fn parse_version(v: &str) -> io::Result<model::RootVersion> {
    let Some((law, domain)) = v.split_once(' ') else {
        return Err(state_error(format!(
            "version {v:?} is not \"<law> <domain>\""
        )));
    };
    let hash_law = law
        .parse()
        .map_err(|_| state_error(format!("unparseable hash law {law:?}")))?;
    let domain = domain
        .parse()
        .map_err(|_| state_error(format!("unparseable domain {domain:?}")))?;
    Ok(model::RootVersion { hash_law, domain })
}
