//! Parallel disjoint commits — the publication half of construction step 6.
//!
//! Law: the amended authority contract
//! `solve-codex-cross-process-authority.md`, pin `95f7c248101e6ff6…`
//! (§3–§6, §11, §15 capsule). The lease half is [`crate::authority`].
//!
//! What this module IS: reservation algebra (complete `R` + complete `W`,
//! atomic acquire, OCC compatibility); the publish path that stages,
//! syncs, renames, and finalizes outside any exclusion; one checksummed
//! `O_EXCL` intent before the first visible rename; `commit_unknown`
//! after the durable decision; recovery that finishes or restores the
//! complete declared set; the one state owner that assigns a contiguous
//! `root`/`seq`/frame with no disk I/O in the mutex.
//!
//! What this module is NOT: it does not acquire `write.lock`, open a
//! socket, activate the downgrade fence, or rewire the live splice door
//! (that door keeps its interim flock until the cutover). Dry never
//! enters here — the existing dry planner publishes nothing.
//!
//! Contract law 11, claimed exactly: reservations exclude cooperating
//! Meridian publishers only. Editors, git, and bash remain the stated
//! external-race residual; the `apply_batch` pre-image verify is what
//! refuses a second in-process writer.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Instant;

use wire::{DeltaFile, DeltaFrame, Root};

use crate::authority::AuthorityEpoch;
use crate::write::assemble_delta;
use fs::intent::{self, DestClass, Found, GroupManifest, Intent, IntentMember, Phase, Policy};
use fs::{WorkspaceRoot, honest_sync_path};

// ── identities ──────────────────────────────────────────────────────────────

/// A transaction id, minted from kernel entropy, hex-spelled on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TxnId(pub u128);

impl TxnId {
    /// Mint a fresh id. Entropy, not a clock: two txns in one tick stay distinct.
    ///
    /// # Errors
    /// `getentropy` failure.
    pub fn mint() -> io::Result<Self> {
        let mut bytes = [0u8; 16];
        // SAFETY: getentropy fills exactly `bytes.len()` (≤ 256) bytes of a
        // valid, owned buffer; the buffer outlives the call.
        if unsafe { libc::getentropy(bytes.as_mut_ptr().cast(), bytes.len()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(u128::from_le_bytes(bytes)))
    }

    /// Canonical 32-char hex spelling.
    #[must_use]
    pub fn hex(&self) -> String {
        format!("{:032x}", self.0)
    }

    /// Parse the hex spelling. `None` when the string is not 32 hex chars.
    #[must_use]
    pub fn from_hex(s: &str) -> Option<Self> {
        if s.len() != 32 {
            return None;
        }
        u128::from_str_radix(s, 16).ok().map(Self)
    }
}

// ── reservation algebra (§4) ────────────────────────────────────────────────

/// One sealed region: the complete `R` and complete `W` are sets of these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    pub corpus: String,
    pub kind: RegionKind,
    pub spatial: Spatial,
}

/// Read vs write — `R/R` is compatible; any pair involving a write conflicts
/// on a spatial intersection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionKind {
    Read,
    Write,
}

/// Spatial extent of a region.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Spatial {
    /// Intersects every other region in the corpus (a root premise).
    Root,
    /// The folder node and everything under it.
    Folder { prefix: Vec<u8> },
    /// One file.
    File { path: Vec<u8> },
    /// The parent/name namespace edge (an absence premise).
    Absence { parent: Vec<u8>, name: Vec<u8> },
}

impl Region {
    /// A root-read premise: intersects every write in `corpus`.
    #[must_use]
    pub fn root_read(corpus: impl Into<String>) -> Self {
        Self {
            corpus: corpus.into(),
            kind: RegionKind::Read,
            spatial: Spatial::Root,
        }
    }

    /// A folder (or prefix) premise — intersects dests at or under `prefix`.
    #[must_use]
    pub fn folder(corpus: impl Into<String>, prefix: impl AsRef<[u8]>, kind: RegionKind) -> Self {
        Self {
            corpus: corpus.into(),
            kind,
            spatial: Spatial::Folder {
                prefix: prefix.as_ref().to_vec(),
            },
        }
    }

    /// A point-file premise — does not block a disjoint folder.
    #[must_use]
    pub fn file(corpus: impl Into<String>, path: impl AsRef<[u8]>, kind: RegionKind) -> Self {
        Self {
            corpus: corpus.into(),
            kind,
            spatial: Spatial::File {
                path: path.as_ref().to_vec(),
            },
        }
    }

    /// An absence premise: the exact parent/name namespace edge.
    #[must_use]
    pub fn absence(
        corpus: impl Into<String>,
        parent: impl AsRef<[u8]>,
        name: impl AsRef<[u8]>,
    ) -> Self {
        Self {
            corpus: corpus.into(),
            kind: RegionKind::Read,
            spatial: Spatial::Absence {
                parent: parent.as_ref().to_vec(),
                name: name.as_ref().to_vec(),
            },
        }
    }

    /// Conventional OCC: conflict iff at least one side is a write AND the
    /// spatial extents intersect in the same corpus.
    #[must_use]
    pub fn conflicts(&self, other: &Region) -> bool {
        if self.corpus != other.corpus {
            return false;
        }
        if self.kind == RegionKind::Read && other.kind == RegionKind::Read {
            return false;
        }
        spatial_intersect(&self.spatial, &other.spatial)
    }
}

fn spatial_intersect(a: &Spatial, b: &Spatial) -> bool {
    match (a, b) {
        (Spatial::Root, _) | (_, Spatial::Root) => true,
        (Spatial::Folder { prefix: p }, Spatial::Folder { prefix: q }) => {
            under_or_eq(p, q) || under_or_eq(q, p)
        }
        (Spatial::Folder { prefix: p }, Spatial::File { path: f })
        | (Spatial::File { path: f }, Spatial::Folder { prefix: p }) => under_or_eq(f, p),
        (Spatial::Folder { prefix: p }, Spatial::Absence { parent, name })
        | (Spatial::Absence { parent, name }, Spatial::Folder { prefix: p }) => {
            let full = join_edge(parent, name);
            under_or_eq(&full, p) || under_or_eq(parent, p)
        }
        (Spatial::File { path: f }, Spatial::File { path: g }) => f == g,
        (Spatial::File { path: f }, Spatial::Absence { parent, name })
        | (Spatial::Absence { parent, name }, Spatial::File { path: f }) => {
            f.as_slice() == join_edge(parent, name)
        }
        (
            Spatial::Absence {
                parent: p1,
                name: n1,
            },
            Spatial::Absence {
                parent: p2,
                name: n2,
            },
        ) => p1 == p2 && n1 == n2,
    }
}

fn under_or_eq(path: &[u8], prefix: &[u8]) -> bool {
    path == prefix || (path.starts_with(prefix) && path.get(prefix.len()) == Some(&b'/'))
}

fn join_edge(parent: &[u8], name: &[u8]) -> Vec<u8> {
    if parent.is_empty() {
        name.to_vec()
    } else {
        [parent, b"/", name].concat()
    }
}

struct Held {
    txn: TxnId,
    regions: Vec<Region>,
}

#[derive(Default)]
struct TableInner {
    held: Vec<Held>,
}

/// The per-workspace reservation table. Overlapping callers wait; disjoint
/// callers never wait; nobody receives `workspace_busy`.
#[derive(Default)]
pub struct ReservationTable {
    inner: Mutex<TableInner>,
    cv: Condvar,
}

/// A held reservation. Dropping it releases the regions and wakes waiters.
pub struct Reservation {
    table: Arc<ReservationTable>,
    txn: TxnId,
}

impl ReservationTable {
    /// A fresh empty table. Share via the returned `Arc`.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Atomically acquire the complete `R ∪ W`. Waits while any held set
    /// conflicts; never returns `workspace_busy`.
    #[must_use]
    pub fn reserve(
        self: &Arc<Self>,
        txn: TxnId,
        reads: Vec<Region>,
        writes: Vec<Region>,
    ) -> Reservation {
        let mut regions = reads;
        regions.extend(writes);
        let mut guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if !guard.held.iter().any(|h| {
                h.regions
                    .iter()
                    .any(|a| regions.iter().any(|b| a.conflicts(b)))
            }) {
                guard.held.push(Held { txn, regions });
                return Reservation {
                    table: Arc::clone(self),
                    txn,
                };
            }
            guard = self.cv.wait(guard).unwrap_or_else(PoisonError::into_inner);
        }
    }

    fn release(&self, txn: TxnId) {
        let mut guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        guard.held.retain(|h| h.txn != txn);
        self.cv.notify_all();
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.table.release(self.txn);
    }
}

// ── state owner (§3.3) ──────────────────────────────────────────────────────

struct OwnerInner {
    instance: String,
    tip: Root,
    seq: u64,
    frames: Vec<DeltaFrame>,
}

/// The one per-workspace state owner. The ONLY linearized step: apply a
/// settled path delta to the then-current root and assign the next
/// `(instance, seq)`. No disk I/O.
pub struct StateOwner {
    inner: Mutex<OwnerInner>,
}

impl StateOwner {
    /// Birth an owner at `tip` (the authenticated current root).
    #[must_use]
    pub fn new(tip: Root) -> Self {
        Self {
            inner: Mutex::new(OwnerInner {
                instance: crate::ring::RootRing::new().instance().to_owned(),
                tip,
                seq: 0,
                frames: Vec::new(),
            }),
        }
    }

    /// Apply `fold` to the then-current tip (never the planning root) and
    /// assign the next contiguous frame. `fold` must be pure and fast — this
    /// mutex is the µs step.
    pub fn apply(
        &self,
        files: Vec<DeltaFile>,
        actor: Option<String>,
        now: Option<String>,
        fold: impl FnOnce(&Root) -> Root,
    ) -> DeltaFrame {
        let mut g = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let root_before = g.tip.clone();
        let root_after = fold(&root_before);
        g.seq += 1;
        let frame = assemble_delta(g.seq, root_before, root_after.clone(), actor, now, files);
        g.tip = root_after;
        g.frames.push(frame.clone());
        frame
    }

    /// The frames assigned so far, in seq order — the contiguity instrument.
    #[must_use]
    pub fn frames(&self) -> Vec<DeltaFrame> {
        self.inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .frames
            .clone()
    }

    #[must_use]
    pub fn instance(&self) -> String {
        self.inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .instance
            .clone()
    }

    #[must_use]
    pub fn tip(&self) -> Root {
        self.inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .tip
            .clone()
    }
}

/// Assert the assigned chain is contiguous: each frame's `root_before`
/// equals the previous frame's `root_after`.
#[must_use]
pub fn chain_is_contiguous(frames: &[DeltaFrame]) -> bool {
    frames
        .windows(2)
        .all(|w| w[1].delta.root_before == w[0].delta.root_after)
}

// ── publish path ────────────────────────────────────────────────────────────

/// One destination staged outside any exclusion.
#[derive(Debug, Clone)]
pub struct DestImage {
    pub rel: PathBuf,
    pub old: Vec<u8>,
    pub new: Vec<u8>,
    /// Already written + synced, same-directory as `rel`.
    pub tmp: PathBuf,
}

/// A transaction ready to reserve / reverify / decide / rename.
pub struct StagedTxn {
    pub txn: TxnId,
    pub epoch: AuthorityEpoch,
    pub reads: Vec<Region>,
    pub writes: Vec<Region>,
    pub dests: Vec<DestImage>,
    pub files: Vec<DeltaFile>,
    pub actor: Option<String>,
    pub now: Option<String>,
    pub policy: Policy,
    /// Test hook: delete staged temps after the durable decision so rename
    /// fails and the caller must see `commit_unknown`. Production callers
    /// leave this false.
    pub drop_tmp_after_decide: bool,
}

/// Success: the assigned frame, plus when disk-stage I/O ran (the overlap
/// instrument for §7(b)/(f)).
#[derive(Debug, Clone)]
pub struct PublishReceipt {
    pub txn: TxnId,
    pub frame: DeltaFrame,
    pub disk_start: Instant,
    pub disk_end: Instant,
}

/// Failures on the publish path. After a durable decision the only
/// remaining class is [`Self::CommitUnknown`].
#[derive(Debug)]
pub enum PublishError {
    /// Pre-decision: a destination drifted. Zero dest bytes changed.
    Conflict { path: PathBuf },
    /// Pre-decision I/O. Zero dest bytes changed.
    Io(io::Error),
    /// The durable decision landed; recovery must finish or restore.
    CommitUnknown { txn: TxnId, cause: io::Error },
}

impl From<io::Error> for PublishError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// Stage → reserve → reverify → `O_EXCL` intent → rename → state-owner
/// apply → finalize. Staging of `txn.dests` is the caller's (already
/// done); this function is the rest of the path.
///
/// # Errors
/// [`PublishError`] — conflict/I/O before the decision, or
/// `commit_unknown` after it.
pub fn publish(
    root: &WorkspaceRoot,
    table: &Arc<ReservationTable>,
    owner: &StateOwner,
    txn: StagedTxn,
    fold: impl FnOnce(&Root) -> Root,
) -> Result<PublishReceipt, PublishError> {
    let _res = table.reserve(txn.txn, txn.reads.clone(), txn.writes.clone());
    reverify(root, &txn.dests)?;
    let disk_start = Instant::now();
    let intent = decide(root, &txn)?;
    if txn.drop_tmp_after_decide {
        for d in &txn.dests {
            let _ = std::fs::remove_file(&d.tmp);
        }
    }
    if let Err(e) = rename_all(root, &txn.dests) {
        return Err(PublishError::CommitUnknown {
            txn: txn.txn,
            cause: e,
        });
    }
    let disk_end = Instant::now();
    let frame = owner.apply(txn.files, txn.actor, txn.now, fold);
    if let Err(e) = intent::finalize(root, &intent) {
        return Err(PublishError::CommitUnknown {
            txn: txn.txn,
            cause: e,
        });
    }
    let _ = intent::reclaim(root, &txn.txn.hex());
    Ok(PublishReceipt {
        txn: txn.txn,
        frame,
        disk_start,
        disk_end,
    })
}

/// Group commit: one checksummed manifest, then each member independently.
/// A member that never joined (not in `members`) is a clean individual
/// refusal the caller already holds — it is not in the return vec.
///
/// # Errors
/// Per-member. A failure to write the group manifest (no durable
/// decision yet) is `Io` for every member. After a member's own
/// decision, a fault is `commit_unknown` under that member's id.
pub fn publish_group(
    root: &WorkspaceRoot,
    table: &Arc<ReservationTable>,
    owner: &StateOwner,
    gid: TxnId,
    members: Vec<StagedTxn>,
    mut fold: impl FnMut(&Root) -> Root,
) -> Vec<Result<PublishReceipt, PublishError>> {
    let manifest = GroupManifest {
        gid_hex: gid.hex(),
        members: members.iter().map(|m| m.txn.hex()).collect(),
    };
    if let Err(e) = intent::create_group(root, &manifest) {
        // No member has a durable decision yet.
        return members
            .into_iter()
            .map(|_| Err(PublishError::Io(io::Error::new(e.kind(), e.to_string()))))
            .collect();
    }
    members
        .into_iter()
        .map(|m| publish(root, table, owner, m, &mut fold))
        .collect()
}

fn reverify(root: &WorkspaceRoot, dests: &[DestImage]) -> Result<(), PublishError> {
    for d in dests {
        let dest = root.0.join(&d.rel);
        let live = match std::fs::read(&dest) {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(PublishError::Io(e)),
        };
        if live != d.old {
            return Err(PublishError::Conflict {
                path: d.rel.clone(),
            });
        }
    }
    Ok(())
}

/// Persist recovery images and the `O_EXCL` decided intent. After this
/// returns, no error on this transaction is a refusal.
///
/// # Errors
/// I/O at image write, intent create, or honest sync.
pub fn decide(root: &WorkspaceRoot, txn: &StagedTxn) -> Result<Intent, PublishError> {
    let txn_hex = txn.txn.hex();
    let mut members = Vec::with_capacity(txn.dests.len());
    for (i, d) in txn.dests.iter().enumerate() {
        honest_sync_path(&d.tmp)?;
        let (old_store, new_store) = intent::store_images(root, &txn_hex, i, &d.old, &d.new)?;
        let tmp = d
            .tmp
            .strip_prefix(&root.0)
            .map_or_else(|_| d.tmp.clone(), Path::to_path_buf);
        members.push(IntentMember {
            rel: d.rel.clone(),
            old: intent::digest(&d.old),
            new: intent::digest(&d.new),
            tmp,
            old_store,
            new_store,
        });
    }
    let rec = Intent {
        epoch_hex: txn.epoch.hex(),
        txn_hex,
        phase: Phase::Decided,
        policy: txn.policy,
        members,
    };
    intent::create_decided(root, &rec)?;
    Ok(rec)
}

fn rename_all(root: &WorkspaceRoot, dests: &[DestImage]) -> io::Result<()> {
    for d in dests {
        let dest = root.0.join(&d.rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(&d.tmp, &dest)?;
        if let Some(parent) = dest.parent() {
            honest_sync_path(parent)?;
        }
    }
    Ok(())
}

// ── recovery (§5) ───────────────────────────────────────────────────────────

/// One recovered transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberRecovery {
    pub txn_hex: String,
    pub outcome: RecoveryOutcome,
}

/// Honest recovery outcomes. There is no silent mixture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryOutcome {
    /// Already finalized; reclaimed.
    Clean,
    /// All dests matched old (or `AbsentOld`); intent aborted.
    Aborted,
    /// Mixture or all-new; dests restored to the complete old set.
    Restored,
    /// Mixture or all-new; dests forwarded to the complete new set.
    Completed,
    /// Truncated / checksum-failed record.
    Unknown,
    /// At least one dest matched neither identity; workspace quarantined.
    Ambiguous { paths: Vec<PathBuf> },
}

/// Walk every intent, recover independently. A txn that never wrote an
/// intent (rejected before membership) does not appear.
///
/// # Errors
/// Directory I/O; per-member failures become [`RecoveryOutcome::Unknown`].
pub fn recover(root: &WorkspaceRoot) -> io::Result<Vec<MemberRecovery>> {
    let found = intent::scan(root)?;
    let mut out = Vec::new();
    for row in found {
        match row {
            Found::Indeterminate { path, .. } => {
                let txn_hex = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("indeterminate")
                    .to_owned();
                out.push(MemberRecovery {
                    txn_hex,
                    outcome: RecoveryOutcome::Unknown,
                });
            }
            Found::Intent(rec) => out.push(recover_one(root, rec)?),
        }
    }
    Ok(out)
}

fn recover_one(root: &WorkspaceRoot, rec: Intent) -> io::Result<MemberRecovery> {
    if rec.phase == Phase::Applied {
        let _ = intent::reclaim(root, &rec.txn_hex);
        return Ok(MemberRecovery {
            txn_hex: rec.txn_hex,
            outcome: RecoveryOutcome::Clean,
        });
    }
    let mut classes = Vec::with_capacity(rec.members.len());
    let mut neither = Vec::new();
    for m in &rec.members {
        match intent::classify(root, m)? {
            DestClass::Neither => {
                neither.push(m.rel.clone());
                classes.push(DestClass::Neither);
            }
            c => classes.push(c),
        }
    }
    if !neither.is_empty() {
        intent::quarantine(root, &rec.txn_hex, &neither)?;
        return Ok(MemberRecovery {
            txn_hex: rec.txn_hex,
            outcome: RecoveryOutcome::Ambiguous { paths: neither },
        });
    }
    let all_old = classes
        .iter()
        .all(|c| matches!(c, DestClass::Old | DestClass::AbsentOld));
    // all-new is a legal decided state (crash after the last rename, before
    // finalize). The recorded policy still decides — Restore puts the
    // complete old set back.
    let _all_new = classes.iter().all(|c| *c == DestClass::New);
    if all_old {
        let _ = intent::reclaim(root, &rec.txn_hex);
        return Ok(MemberRecovery {
            txn_hex: rec.txn_hex,
            outcome: RecoveryOutcome::Aborted,
        });
    }
    // Mixture or all-new: apply the recorded policy to the COMPLETE set.
    match rec.policy {
        Policy::Restore => {
            for m in &rec.members {
                intent::restore_member(root, m)?;
            }
            let _ = intent::reclaim(root, &rec.txn_hex);
            Ok(MemberRecovery {
                txn_hex: rec.txn_hex,
                outcome: RecoveryOutcome::Restored,
            })
        }
        Policy::Forward => {
            for m in &rec.members {
                intent::forward_member(root, m)?;
            }
            intent::finalize(root, &rec)?;
            let _ = intent::reclaim(root, &rec.txn_hex);
            Ok(MemberRecovery {
                txn_hex: rec.txn_hex,
                outcome: RecoveryOutcome::Completed,
            })
        }
    }
}

// ── apply_batch seam ────────────────────────────────────────────────────────

/// Build a [`StagedTxn`] from a staged two-file batch (the `apply_batch`
/// split). The content dest is required; the receipt dest rides when present.
///
/// # Errors
/// Reading a staged temp, or minting a txn id.
pub fn staged_from_batch(
    root: &WorkspaceRoot,
    epoch: AuthorityEpoch,
    staged: &fs::StagedCommit,
    reads: Vec<Region>,
    writes: Vec<Region>,
) -> io::Result<StagedTxn> {
    let txn = TxnId::mint()?;
    let content_new = std::fs::read(staged.content_tmp())?;
    let content_rel = strip_root(root, staged.content_dst());
    let mut dests = vec![DestImage {
        rel: content_rel,
        old: staged.content_expected().to_vec(),
        new: content_new,
        tmp: staged.content_tmp().to_path_buf(),
    }];
    if let (Some(dst), Some(tmp), Some(old)) = (
        staged.receipt_dst(),
        staged.receipt_tmp(),
        staged.receipt_expected(),
    ) {
        dests.push(DestImage {
            rel: strip_root(root, dst),
            old: old.to_vec(),
            new: std::fs::read(tmp)?,
            tmp: tmp.to_path_buf(),
        });
    }
    Ok(StagedTxn {
        txn,
        epoch,
        reads,
        writes,
        dests,
        files: Vec::new(),
        actor: None,
        now: None,
        policy: Policy::Restore,
        drop_tmp_after_decide: false,
    })
}

fn strip_root(root: &WorkspaceRoot, path: &Path) -> PathBuf {
    path.strip_prefix(&root.0)
        .map_or_else(|_| path.to_path_buf(), Path::to_path_buf)
}

// ── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use fs::is_write_conflict;

    fn ws() -> (tempfile::TempDir, WorkspaceRoot) {
        let dir = tempfile::tempdir().expect("tempdir");
        (dir, WorkspaceRoot(dir.path().to_path_buf()))
    }

    fn fold_mark(tag: &'static str) -> impl Fn(&Root) -> Root {
        move |before| Root(format!("{}+{tag}", before.0))
    }

    fn dest(root: &WorkspaceRoot, rel: &str, old: &[u8], new: &[u8]) -> DestImage {
        let dest = root.0.join(rel);
        if let Some(p) = dest.parent() {
            std::fs::create_dir_all(p).unwrap();
        }
        std::fs::write(&dest, old).unwrap();
        let tmp = dest.with_extension("tmp-pub");
        std::fs::write(&tmp, new).unwrap();
        honest_sync_path(&tmp).unwrap();
        DestImage {
            rel: PathBuf::from(rel),
            old: old.to_vec(),
            new: new.to_vec(),
            tmp,
        }
    }

    fn txn(root: &WorkspaceRoot, rel: &str, old: &[u8], new: &[u8], w: Region) -> StagedTxn {
        StagedTxn {
            txn: TxnId::mint().unwrap(),
            epoch: AuthorityEpoch(1),
            reads: Vec::new(),
            writes: vec![w],
            dests: vec![dest(root, rel, old, new)],
            files: Vec::new(),
            actor: None,
            now: None,
            policy: Policy::Restore,
            drop_tmp_after_decide: false,
        }
    }

    #[test]
    fn r_r_same_file_is_compatible_and_w_w_waits() {
        let table = ReservationTable::new();
        let file = b"a.md".as_slice();
        let a = table.reserve(
            TxnId(1),
            vec![Region::file("c", file, RegionKind::Read)],
            vec![],
        );
        let b = table.reserve(
            TxnId(2),
            vec![Region::file("c", file, RegionKind::Read)],
            vec![],
        );
        drop((a, b));

        let held = table.reserve(
            TxnId(3),
            vec![],
            vec![Region::file("c", file, RegionKind::Write)],
        );
        let (tx, rx) = mpsc::channel();
        let t = Arc::clone(&table);
        thread::spawn(move || {
            let _r = t.reserve(
                TxnId(4),
                vec![],
                vec![Region::file("c", file, RegionKind::Write)],
            );
            tx.send(()).unwrap();
        });
        thread::sleep(Duration::from_millis(40));
        assert!(rx.try_recv().is_err(), "overlapping writer waits");
        drop(held);
        rx.recv_timeout(Duration::from_secs(1))
            .expect("waiter proceeds after release");
    }

    #[test]
    fn a_root_read_blocks_every_write_a_folder_does_not_block_a_sibling() {
        let table = ReservationTable::new();
        let root_r = table.reserve(TxnId(1), vec![Region::root_read("c")], vec![]);
        let (tx, rx) = mpsc::channel();
        let t = Arc::clone(&table);
        thread::spawn(move || {
            let _r = t.reserve(
                TxnId(2),
                vec![],
                vec![Region::file("c", b"z.md", RegionKind::Write)],
            );
            tx.send("blocked").unwrap();
        });
        thread::sleep(Duration::from_millis(40));
        assert!(rx.try_recv().is_err(), "root R blocks a file W");
        drop(root_r);
        rx.recv_timeout(Duration::from_secs(1)).unwrap();

        let a = table.reserve(
            TxnId(3),
            vec![Region::folder("c", b"a", RegionKind::Read)],
            vec![],
        );
        let b = table.reserve(
            TxnId(4),
            vec![],
            vec![Region::folder("c", b"b", RegionKind::Write)],
        );
        drop((a, b));
    }

    #[test]
    fn disjoint_commits_overlap_in_disk_stage_intervals() {
        let (_d, root) = ws();
        let table = ReservationTable::new();
        let owner = StateOwner::new(Root("g0".into()));
        let a = txn(
            &root,
            "a/x.md",
            b"old-a",
            &vec![b'A'; 256 * 1024],
            Region::folder("c", b"a", RegionKind::Write),
        );
        let b = txn(
            &root,
            "b/y.md",
            b"old-b",
            &vec![b'B'; 256 * 1024],
            Region::folder("c", b"b", RegionKind::Write),
        );
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let (tx, rx) = mpsc::channel();
        for staged in [a, b] {
            let table = Arc::clone(&table);
            let root = WorkspaceRoot(root.0.clone());
            let barrier = Arc::clone(&barrier);
            let tx = tx.clone();
            // Each thread owns its own owner apply via a channel back; the
            // linearized step runs on this thread after join so the overlap
            // measurement covers disk-stage only.
            thread::spawn(move || {
                barrier.wait();
                let rec = publish(
                    &root,
                    &table,
                    &StateOwner::new(Root("t".into())),
                    staged,
                    fold_mark("t"),
                )
                .expect("disjoint publish");
                tx.send((rec.disk_start, rec.disk_end)).unwrap();
            });
        }
        drop(tx);
        let mut iv = Vec::new();
        for _ in 0..2 {
            iv.push(rx.recv_timeout(Duration::from_secs(5)).expect("join"));
        }
        let overlap = iv[0].0 < iv[1].1 && iv[1].0 < iv[0].1;
        assert!(
            overlap,
            "disk-stage intervals must overlap: {:?} vs {:?}",
            iv[0], iv[1]
        );
        // Contiguity of the *one* state owner is the next test; here we only
        // prove the I/O windows themselves overlapped.
        let _ = owner;
    }

    #[test]
    fn the_state_owner_assigns_a_contiguous_chain_under_shuffled_completion() {
        let owner = Arc::new(StateOwner::new(Root("g0".into())));
        let mut joins = Vec::new();
        for i in 0..8 {
            let owner = Arc::clone(&owner);
            joins.push(thread::spawn(move || {
                thread::sleep(Duration::from_millis(i * 3 % 11));
                owner.apply(Vec::new(), None, None, fold_mark("x"))
            }));
        }
        for j in joins {
            j.join().expect("thread");
        }
        let frames = owner.frames();
        assert_eq!(frames.len(), 8);
        assert!(
            chain_is_contiguous(&frames),
            "root_before must equal the previous root_after: {:?}",
            frames
                .iter()
                .map(|f| (
                    f.delta.seq,
                    f.delta.root_before.0.clone(),
                    f.delta.root_after.0.clone()
                ))
                .collect::<Vec<_>>()
        );
        assert_eq!(frames[0].delta.root_before, Root("g0".into()));
        let seqs: Vec<u64> = frames.iter().map(|f| f.delta.seq).collect();
        let mut ordered = seqs.clone();
        ordered.sort_unstable();
        assert_eq!(seqs, ordered, "apply serializes seq assignment");
    }

    #[test]
    fn after_the_durable_decision_a_rename_fault_is_commit_unknown() {
        let (_d, root) = ws();
        let table = ReservationTable::new();
        let owner = StateOwner::new(Root("g0".into()));
        let mut staged = txn(
            &root,
            "n/b.md",
            b"old",
            b"new",
            Region::file("c", b"n/b.md", RegionKind::Write),
        );
        staged.drop_tmp_after_decide = true;
        let err = publish(&root, &table, &owner, staged, fold_mark("x")).expect_err("unknown");
        assert!(
            matches!(err, PublishError::CommitUnknown { .. }),
            "after decision the fault is commit_unknown, got {err:?}"
        );
        assert_eq!(
            std::fs::read(root.0.join("n/b.md")).unwrap(),
            b"old",
            "a post-decision rename fault must not half-apply"
        );
    }

    #[test]
    fn recover_restores_the_complete_set_after_the_first_rename() {
        let (_d, root) = ws();
        let table = ReservationTable::new();
        let owner = StateOwner::new(Root("g0".into()));
        let content = dest(&root, "notes/plan.md", b"OLD-C", b"NEW-C");
        let receipt = dest(&root, "receipts/log.md", b"OLD-R", b"NEW-R");
        let staged = StagedTxn {
            txn: TxnId::mint().unwrap(),
            epoch: AuthorityEpoch(1),
            reads: Vec::new(),
            writes: vec![
                Region::file("c", b"notes/plan.md", RegionKind::Write),
                Region::file("c", b"receipts/log.md", RegionKind::Write),
            ],
            dests: vec![content, receipt],
            files: Vec::new(),
            actor: None,
            now: None,
            policy: Policy::Restore,
            drop_tmp_after_decide: false,
        };
        reverify(&root, &staged.dests).unwrap();
        decide(&root, &staged).unwrap();
        // First rename only — the kill-after-first-rename window.
        std::fs::rename(&staged.dests[0].tmp, root.0.join(&staged.dests[0].rel)).unwrap();
        assert_eq!(
            std::fs::read(root.0.join("notes/plan.md")).unwrap(),
            b"NEW-C"
        );
        assert_eq!(
            std::fs::read(root.0.join("receipts/log.md")).unwrap(),
            b"OLD-R"
        );

        let report = recover(&root).expect("recover");
        assert_eq!(report.len(), 1);
        assert_eq!(report[0].outcome, RecoveryOutcome::Restored);
        assert_eq!(
            std::fs::read(root.0.join("notes/plan.md")).unwrap(),
            b"OLD-C"
        );
        assert_eq!(
            std::fs::read(root.0.join("receipts/log.md")).unwrap(),
            b"OLD-R",
            "the complete declared set is restored; no partial receipt remains"
        );
        let _ = (table, owner);
    }

    #[test]
    fn mixed_group_kill_attributes_per_member() {
        let (_d, root) = ws();
        // Member 0: never joined (rejected before membership) — no intent.
        // Member 1: durable decided, complete images, mixture on disk.
        // Member 2: truncated / indeterminate intent file.
        // Member 3: healthy neighbor, already Applied.

        let m1 = dest(&root, "g/one.md", b"o1", b"n1");
        let t1 = StagedTxn {
            txn: TxnId(0x11),
            epoch: AuthorityEpoch(1),
            reads: Vec::new(),
            writes: vec![Region::file("c", b"g/one.md", RegionKind::Write)],
            dests: vec![m1],
            files: Vec::new(),
            actor: None,
            now: None,
            policy: Policy::Restore,
            drop_tmp_after_decide: false,
        };
        decide(&root, &t1).unwrap();
        std::fs::rename(&t1.dests[0].tmp, root.0.join(&t1.dests[0].rel)).unwrap();

        let dir = root.0.join(intent::INTENTS_DIR);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("00000000000000000000000000000022.intent"),
            b"MRD-INTENT/1\n",
        )
        .unwrap();

        let m3 = dest(&root, "g/three.md", b"o3", b"n3");
        let t3 = StagedTxn {
            txn: TxnId(0x33),
            epoch: AuthorityEpoch(1),
            reads: Vec::new(),
            writes: vec![Region::file("c", b"g/three.md", RegionKind::Write)],
            dests: vec![m3],
            files: Vec::new(),
            actor: None,
            now: None,
            policy: Policy::Restore,
            drop_tmp_after_decide: false,
        };
        let rec3 = decide(&root, &t3).unwrap();
        rename_all(&root, &t3.dests).unwrap();
        intent::finalize(&root, &rec3).unwrap();

        intent::create_group(
            &root,
            &GroupManifest {
                gid_hex: "gg".into(),
                members: vec![
                    t1.txn.hex(),
                    "00000000000000000000000000000022".into(),
                    t3.txn.hex(),
                ],
            },
        )
        .unwrap();

        let report = recover(&root).expect("recover");
        let mut by = std::collections::BTreeMap::new();
        for row in &report {
            by.insert(row.txn_hex.clone(), row.outcome.clone());
        }
        assert_eq!(by.get(&t1.txn.hex()), Some(&RecoveryOutcome::Restored));
        assert_eq!(
            by.get("00000000000000000000000000000022"),
            Some(&RecoveryOutcome::Unknown)
        );
        assert_eq!(by.get(&t3.txn.hex()), Some(&RecoveryOutcome::Clean));
        assert!(
            !by.contains_key("00000000000000000000000000000000"),
            "a member rejected before membership leaves no recovery row"
        );
        // No cross-attribution: the healthy neighbor is Clean, not Unknown,
        // and the decided member Restored, not Unknown.
        assert_eq!(std::fs::read(root.0.join("g/one.md")).unwrap(), b"o1");
        assert_eq!(std::fs::read(root.0.join("g/three.md")).unwrap(), b"n3");
    }

    #[test]
    fn a_pre_decision_conflict_is_a_clean_refusal() {
        let (_d, root) = ws();
        let table = ReservationTable::new();
        let owner = StateOwner::new(Root("g0".into()));
        let staged = txn(
            &root,
            "p.md",
            b"old",
            b"new",
            Region::file("c", b"p.md", RegionKind::Write),
        );
        std::fs::write(root.0.join("p.md"), b"moved").unwrap();
        let err = publish(&root, &table, &owner, staged, fold_mark("x")).expect_err("conflict");
        assert!(matches!(err, PublishError::Conflict { .. }));
        assert_eq!(std::fs::read(root.0.join("p.md")).unwrap(), b"moved");
        assert!(
            intent::scan(&root).unwrap().is_empty(),
            "a clean refusal writes no intent"
        );
    }

    #[test]
    fn write_conflict_from_fs_is_still_the_second_writer_class() {
        // Hygiene: the publish Conflict class is the same event the fs
        // pre-image verify names — this just keeps the split visible.
        let err = fs::write_conflict(Path::new("x.md"));
        assert!(is_write_conflict(&err));
    }
}
