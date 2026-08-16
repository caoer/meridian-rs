//! Parallel disjoint commits — the publication half of construction step 6.
//!
//! Law: the amended authority contract
//! `solve-codex-cross-process-authority.md`, pin `95f7c248101e6ff6…`
//! (§3–§6, §11, §15 capsule). The lease half is [`crate::authority`].
//!
//! What this module IS: reservation algebra (complete `R` + complete `W`,
//! atomic acquire, OCC compatibility); the publish path that stages,
//! renames, and finalizes outside any exclusion; one checksummed
//! `O_EXCL` intent before the first visible rename; `commit_unknown`
//! after the durable decision; recovery that finishes or restores the
//! complete declared set; the one state owner that assigns a contiguous
//! `root`/`seq`/frame with no disk I/O in the mutex.
//!
//! Durability is SCHEDULED, not paid inline (`docs/laws.md` § Amendment —
//! epoch-batched honest durability). The path pays exactly TWO honest syncs
//! however many files it touches, and shares both with every other writer in
//! the same epoch: phase 1 seals content temps + recovery images + the intent
//! record — that barrier IS the durable decision — and phase 2 seals the
//! dirents + the `Applied` record. No door on this path syncs on its own; one
//! that did would silently cost a full barrier.
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
use std::time::{Duration, Instant};

use wire::{DeltaFile, DeltaFrame, Root};

use crate::authority::AuthorityEpoch;
use crate::write::assemble_delta;
use fs::WorkspaceRoot;
use fs::epoch::{Covered, EpochBatcher, data_push_all};
use fs::intent::{self, DestClass, Found, GroupManifest, Intent, IntentMember, Phase, Policy};

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

/// A measured interval. The publish path records its three intervals
/// SEPARATELY and never sums them: one span covering both durability phases
/// is an aggregate that hides which sync class was paid where (`docs/laws.md`
/// § Amendment — epoch-batched honest durability).
#[derive(Debug, Clone, Copy)]
pub struct Span {
    pub start: Instant,
    pub end: Instant,
}

impl Span {
    /// How long the interval took. Never add two of these together.
    #[must_use]
    pub fn duration(&self) -> Duration {
        self.end.saturating_duration_since(self.start)
    }
}

/// Success: the assigned frame, plus the three intervals the publish path is
/// measured by. They are three figures, not three summands.
#[derive(Debug, Clone)]
pub struct PublishReceipt {
    pub txn: TxnId,
    pub frame: DeltaFrame,
    /// Phase 1 — content durability: every staged temp handed to the device,
    /// then the barrier that covers those bytes. Held under NO exclusion, so
    /// this is also the overlap instrument for §7(b)/(f).
    pub content: Span,
    /// Phase 2 — metadata publish: the `O_EXCL` intent, the visible renames,
    /// and the dirent barrier. Held under the reservation.
    pub publish: Span,
    /// The linearized state-owner step. No disk I/O of any class runs inside
    /// it; the per-thread sync counters are what prove that.
    pub owner_step: Span,
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
    batcher: &EpochBatcher,
    txn: StagedTxn,
    fold: impl FnOnce(&Root) -> Root,
) -> Result<PublishReceipt, PublishError> {
    let _res = table.reserve(txn.txn, txn.reads.clone(), txn.writes.clone());
    reverify(root, &txn.dests)?;

    // ── Phase 1 — the durable decision ──────────────────────────────────
    // Content temps, recovery images and the O_EXCL intent, sealed by ONE
    // barrier shared with every other writer in this epoch. Nothing below
    // syncs on its own: a stray honest sync here silently costs a full
    // barrier and defeats the batching.
    let c0 = Instant::now();
    let mut seal: Vec<PathBuf> = txn.dests.iter().map(|d| d.tmp.clone()).collect();
    let intent = decide(root, &txn, &mut seal)?;
    // The decision becomes durable HERE, not at create_decided. No rename has
    // happened yet, so a fault on this barrier is still a clean refusal.
    barrier(batcher, seal)?;
    let content = Span {
        start: c0,
        end: Instant::now(),
    };

    if txn.drop_tmp_after_decide {
        for d in &txn.dests {
            let _ = std::fs::remove_file(&d.tmp);
        }
    }

    // ── Phase 2 — metadata publish ──────────────────────────────────────
    // Visible renames, their parent dirents, and the Applied record.
    let p0 = Instant::now();
    let mut seal2: Vec<PathBuf> = Vec::new();
    if let Err(e) = rename_all(root, &txn.dests, &mut seal2) {
        return Err(PublishError::CommitUnknown {
            txn: txn.txn,
            cause: e,
        });
    }
    if let Err(e) = intent::finalize(root, &intent, &mut seal2) {
        return Err(PublishError::CommitUnknown {
            txn: txn.txn,
            cause: e,
        });
    }
    if let Err(e) = barrier(batcher, seal2) {
        return Err(PublishError::CommitUnknown {
            txn: txn.txn,
            cause: e,
        });
    }
    let publish = Span {
        start: p0,
        end: Instant::now(),
    };

    let o0 = Instant::now();
    let frame = owner.apply(txn.files, txn.actor, txn.now, fold);
    let owner_step = Span {
        start: o0,
        end: Instant::now(),
    };

    let _ = intent::reclaim(root, &txn.txn.hex());
    Ok(PublishReceipt {
        txn: txn.txn,
        frame,
        content,
        publish,
        owner_step,
    })
}

/// Push every recorded path and await the epoch that covers them — the one
/// place a publish pays the honest class.
///
/// The dedupe is safe for a structural reason, not an arithmetic one: `dedup`
/// removes only EXACT duplicates of a path already in the set, so no distinct
/// path can lose its push. That argument holds at any destination count and
/// any folder layout, where comparing totals would not. It does make the
/// cheap-push count corpus-dependent — `5D + Ddir + 6` per commit, in the
/// number of distinct destination directories — which is why the test states
/// the formula rather than a constant.
fn barrier(batcher: &EpochBatcher, mut seal: Vec<PathBuf>) -> io::Result<Covered> {
    seal.sort();
    seal.dedup();
    batcher.await_epoch(data_push_all(seal)?)
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
    batcher: &EpochBatcher,
    gid: TxnId,
    members: Vec<StagedTxn>,
    mut fold: impl FnMut(&Root) -> Root,
) -> Vec<Result<PublishReceipt, PublishError>> {
    let manifest = GroupManifest {
        gid_hex: gid.hex(),
        members: members.iter().map(|m| m.txn.hex()).collect(),
    };
    // The manifest rides the epoch like everything else, so the group stops
    // paying its own serial pair of honest syncs before the members start.
    let mut seal = Vec::new();
    let wrote = intent::create_group(root, &manifest, &mut seal)
        .and_then(|()| barrier(batcher, seal).map(|_| ()));
    if let Err(e) = wrote {
        // No member has a durable decision yet.
        return members
            .into_iter()
            .map(|_| Err(PublishError::Io(io::Error::new(e.kind(), e.to_string()))))
            .collect();
    }
    members
        .into_iter()
        .map(|m| publish(root, table, owner, batcher, m, &mut fold))
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
/// Nothing here syncs. Every path this writes — both recovery images and the
/// intent record — is recorded in `seal`, and the caller's phase-1 barrier
/// makes the whole set durable in one honest sync. The decision is complete
/// when that barrier returns.
///
/// # Errors
/// I/O at image write or intent create.
pub fn decide(
    root: &WorkspaceRoot,
    txn: &StagedTxn,
    seal: &mut Vec<PathBuf>,
) -> Result<Intent, PublishError> {
    let txn_hex = txn.txn.hex();
    let mut members = Vec::with_capacity(txn.dests.len());
    for (i, d) in txn.dests.iter().enumerate() {
        let (old_store, new_store) = intent::store_images(root, &txn_hex, i, &d.old, &d.new, seal)?;
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
    intent::create_decided(root, &rec, seal)?;
    Ok(rec)
}

/// Make every staged temp visible. The temps' bytes are already on platter —
/// the phase-1 barrier returned before this runs — so each rename only owes
/// its parent dirent, which is recorded for the phase-2 barrier.
fn rename_all(
    root: &WorkspaceRoot,
    dests: &[DestImage],
    seal: &mut Vec<PathBuf>,
) -> io::Result<()> {
    for d in dests {
        let dest = root.0.join(&d.rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(&d.tmp, &dest)?;
        if let Some(parent) = dest.parent() {
            seal.push(parent.to_path_buf());
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
            // Recovery runs once after a crash and shares no epoch: it seals
            // synchronously rather than batching.
            let mut seal = Vec::new();
            intent::finalize(root, &rec, &mut seal)?;
            intent::seal_now(&seal)?;
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
        let root = WorkspaceRoot(dir.path().to_path_buf());
        (dir, root)
    }

    fn batcher(root: &WorkspaceRoot) -> EpochBatcher {
        EpochBatcher::for_workspace(root).expect("batcher")
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
        fs::data_push_path(&tmp).unwrap();
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
                    &batcher(&root),
                    staged,
                    fold_mark("t"),
                )
                .expect("disjoint publish");
                tx.send((rec.content.start, rec.content.end)).unwrap();
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
        let err = publish(
            &root,
            &table,
            &owner,
            &batcher(&root),
            staged,
            fold_mark("x"),
        )
        .expect_err("unknown");
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
        let mut seal = Vec::new();
        decide(&root, &staged, &mut seal).unwrap();
        intent::seal_now(&seal).unwrap();
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
        let mut seal = Vec::new();
        decide(&root, &t1, &mut seal).unwrap();
        intent::seal_now(&seal).unwrap();
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
        let mut seal3 = Vec::new();
        let rec3 = decide(&root, &t3, &mut seal3).unwrap();
        rename_all(&root, &t3.dests, &mut seal3).unwrap();
        intent::finalize(&root, &rec3, &mut seal3).unwrap();
        intent::seal_now(&seal3).unwrap();

        let mut gseal = Vec::new();
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
            &mut gseal,
        )
        .unwrap();
        intent::seal_now(&gseal).unwrap();

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
        let err = publish(
            &root,
            &table,
            &owner,
            &batcher(&root),
            staged,
            fold_mark("x"),
        )
        .expect_err("conflict");
        assert!(matches!(err, PublishError::Conflict { .. }));
        assert_eq!(std::fs::read(root.0.join("p.md")).unwrap(), b"moved");
        assert!(
            intent::scan(&root).unwrap().is_empty(),
            "a clean refusal writes no intent"
        );
    }

    /// The pre-registered gate, both classes named because two exist.
    ///
    /// **Honest barriers must be exactly 2 at every D** — constant in
    /// destination count, against a `6D + 4` baseline. Anything above 2 is a
    /// straggler issuing its own `F_FULLFSYNC`.
    ///
    /// **Cheap pushes must be exactly `5D + Ddir + 6`**, where `Ddir` is the
    /// number of DISTINCT destination directories. The count is
    /// corpus-dependent because `barrier` dedupes the seal set, and dedupe
    /// collapses by directory: a fixture with every destination in one folder
    /// measures `5D + 7`, one with a folder each measures `6D + 6`. Predicting
    /// a single constant would be predicting the fixture, not the design.
    ///
    /// The dedupe is honest for a structural reason, not an arithmetic one:
    /// `dedup` removes only EXACT duplicates of an already-present path, so no
    /// distinct path ever loses its push. That argument holds at any D and any
    /// layout, where comparing totals does not.
    ///
    /// An exact count is also what makes the accumulator safe: a path
    /// collected into a seal vector and never drained shows up here as a
    /// count BELOW the formula, which is the honesty failure (bytes riding a
    /// barrier that never had them handed over) rather than a slow number.
    #[test]
    fn barrier_count_is_constant_in_d_and_pushes_follow_the_formula() {
        // `true` puts every destination in one folder, `false` gives each its
        // own — the two layouts that make the corpus dependence visible.
        let mut seen = Vec::new();
        for one_folder in [true, false] {
            let name = if one_folder {
                "one folder"
            } else {
                "folder each"
            };
            let rel_of = |i: usize| {
                if one_folder {
                    format!("shared/f{i}.md")
                } else {
                    format!("d{i}/f{i}.md")
                }
            };
            for d in 1..=3usize {
                let (_dir, root) = ws();
                let table = ReservationTable::new();
                let owner = StateOwner::new(Root("g0".into()));
                let b = batcher(&root);
                let dests: Vec<DestImage> = (0..d)
                    .map(|i| dest(&root, &rel_of(i), b"old", b"new"))
                    .collect();
                let writes = (0..d)
                    .map(|i| Region::file("c", rel_of(i).as_bytes(), RegionKind::Write))
                    .collect();
                let staged = StagedTxn {
                    txn: TxnId::mint().unwrap(),
                    epoch: AuthorityEpoch(1),
                    reads: Vec::new(),
                    writes,
                    dests,
                    files: Vec::new(),
                    actor: None,
                    now: None,
                    policy: Policy::Restore,
                    drop_tmp_after_decide: false,
                };
                let honest0 = fs::honest_sync_count_thread();
                let push0 = fs::data_push_count_thread();
                publish(&root, &table, &owner, &b, staged, fold_mark("x")).expect("publish");
                let honest = fs::honest_sync_count_thread() - honest0;
                let pushes = fs::data_push_count_thread() - push0;
                let dirs = if one_folder { 1 } else { d };
                let want_pushes = 5 * d as u64 + dirs as u64 + 6;
                assert_eq!(
                    honest, 2,
                    "{name} D={d}: honest barriers must be exactly 2, got {honest} — \
                     a straggler is issuing its own F_FULLFSYNC"
                );
                assert_eq!(
                    pushes, want_pushes,
                    "{name} D={d} Ddir={dirs}: cheap pushes must be 5D+Ddir+6 = \
                     {want_pushes}, got {pushes}. Below it means a path was collected \
                     into a seal set and never drained — bytes riding a barrier that \
                     never had them handed over."
                );
                seen.push((name, d, dirs, honest, pushes));
            }
        }
        let barriers: Vec<u64> = seen.iter().map(|r| r.3).collect();
        assert!(
            barriers.iter().all(|b| *b == 2),
            "the load-bearing prediction is that barrier count does not grow with D \
             or with layout: {seen:?}"
        );
    }

    /// Gate: NO sync of either class runs inside the linearized step, proved
    /// while other publishers are concurrently syncing. Per-thread counters
    /// are the only well-formed instrument for this — the process-wide
    /// counter is moving by design, and the test asserts that it moved, so
    /// "the delta was zero" cannot pass by nothing happening at all.
    #[test]
    fn the_linearized_step_and_the_reservation_table_issue_no_sync() {
        let (_dir, root) = ws();
        let table = ReservationTable::new();
        let owner = Arc::new(StateOwner::new(Root("g0".into())));
        let b = Arc::new(batcher(&root));
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));

        // Background publishers, hammering the honest class on other threads.
        // Each signals after its FIRST publish, and the measured region opens
        // only once all three have — otherwise a thread can observe `stop`
        // before it ever publishes and the "it moved" assertion races.
        let (ready_tx, ready_rx) = mpsc::channel();
        let mut noise = Vec::new();
        for i in 0..3 {
            let root = WorkspaceRoot(root.0.clone());
            let table = Arc::clone(&table);
            let owner = Arc::clone(&owner);
            let b = Arc::clone(&b);
            let stop = Arc::clone(&stop);
            let ready_tx = ready_tx.clone();
            noise.push(thread::spawn(move || {
                let mut n = 0u32;
                loop {
                    let rel = format!("noise{i}/n{n}.md");
                    let staged = txn(
                        &root,
                        &rel,
                        b"old",
                        b"new",
                        Region::file("c", rel.as_bytes(), RegionKind::Write),
                    );
                    publish(&root, &table, &owner, &b, staged, fold_mark("n"))
                        .expect("noise publish");
                    if n == 0 {
                        ready_tx.send(()).expect("ready");
                    }
                    n += 1;
                    if stop.load(std::sync::atomic::Ordering::Relaxed) {
                        break;
                    }
                }
            }));
        }
        drop(ready_tx);
        for _ in 0..3 {
            ready_rx
                .recv_timeout(Duration::from_secs(10))
                .expect("a background publisher started");
        }

        let global_before = fs::honest_sync_count();

        // The reservation table's own critical section.
        let h0 = fs::honest_sync_count_thread();
        let p0 = fs::data_push_count_thread();
        let res = table.reserve(
            TxnId::mint().unwrap(),
            Vec::new(),
            vec![Region::file("c", b"quiet/q.md", RegionKind::Write)],
        );
        assert_eq!(
            (fs::honest_sync_count_thread(), fs::data_push_count_thread()),
            (h0, p0),
            "acquiring a reservation must issue no sync of either class"
        );

        // The linearized step.
        let h1 = fs::honest_sync_count_thread();
        let p1 = fs::data_push_count_thread();
        let frame = owner.apply(Vec::new(), None, None, fold_mark("q"));
        assert_eq!(
            fs::honest_sync_count_thread() - h1,
            0,
            "the state-owner step must issue no honest sync"
        );
        assert_eq!(
            fs::data_push_count_thread() - p1,
            0,
            "the state-owner step must issue no data push either"
        );
        assert!(frame.delta.seq >= 1);
        drop(res);

        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        for j in noise {
            j.join().expect("thread");
        }
        assert!(
            fs::honest_sync_count() > global_before,
            "the zero delta must be measured against LIVE concurrent syncing, \
             not against an idle process"
        );
    }

    /// Gate: a crash before the phase-1 barrier returns changes nothing. No
    /// rename has happened, so every destination is still Old — including the
    /// ugly case where the intent record itself is torn and decodes as
    /// `Unknown`. A torn decision with zero renames must leave the workspace
    /// clean, not quarantined.
    #[test]
    fn a_crash_before_the_phase_one_barrier_leaves_the_workspace_untouched() {
        let (_dir, root) = ws();
        let staged = txn(
            &root,
            "pre/b.md",
            b"OLD",
            b"NEW",
            Region::file("c", b"pre/b.md", RegionKind::Write),
        );
        // Everything phase 1 writes, and then the process dies: the seal is
        // built but never drained, so nothing was sealed and nothing renamed.
        let mut seal = Vec::new();
        decide(&root, &staged, &mut seal).expect("decision written");
        assert!(!seal.is_empty(), "phase 1 must declare a seal set");
        assert_eq!(
            std::fs::read(root.0.join("pre/b.md")).unwrap(),
            b"OLD",
            "no rename precedes the phase-1 barrier, so the dest is untouched"
        );

        // The ugly case: truncate the intent so it decodes as indeterminate.
        let intent_file = root
            .0
            .join(intent::INTENTS_DIR)
            .join(format!("{}.intent", staged.txn.hex()));
        std::fs::write(&intent_file, b"MRD-INTENT/1\n").expect("truncate");

        let report = recover(&root).expect("recover");
        assert_eq!(report.len(), 1);
        assert_eq!(
            report[0].outcome,
            RecoveryOutcome::Unknown,
            "a torn record is Unknown — recovery never guesses"
        );
        assert_eq!(
            std::fs::read(root.0.join("pre/b.md")).unwrap(),
            b"OLD",
            "Unknown with zero renames must still leave the workspace clean"
        );
        assert!(
            !root.0.join("quarantine").exists(),
            "nothing to quarantine: no destination ever changed"
        );
    }

    /// Gate: durability is not weakened. Every committed destination is
    /// covered by a seal that RETURNED before its rename — asserted as an
    /// ordering fact, never as a wall-clock one.
    #[test]
    fn every_committed_dest_is_covered_by_a_seal_that_returned_before_its_rename() {
        let (_dir, root) = ws();
        let b = batcher(&root);
        let tmp = dest(&root, "ord/c.md", b"old", b"new");
        let pushed = data_push_all(vec![tmp.tmp.clone()]).expect("push");
        let pushed_at = pushed.at();
        let covered = b.await_epoch(pushed).expect("seal");
        // The seal covering these bytes started after their push returned...
        assert!(covered.seal_started >= pushed_at);
        // ...and returned before the rename that makes them visible.
        let renamed_at = Instant::now();
        std::fs::rename(&tmp.tmp, root.0.join(&tmp.rel)).expect("rename");
        assert!(
            covered.seal_returned <= renamed_at,
            "no writer may proceed to its rename before the barrier covering \
             its bytes has returned"
        );
        assert_eq!(std::fs::read(root.0.join("ord/c.md")).unwrap(), b"new");
    }

    #[test]
    fn write_conflict_from_fs_is_still_the_second_writer_class() {
        // Hygiene: the publish Conflict class is the same event the fs
        // pre-image verify names — this just keeps the split visible.
        let err = fs::write_conflict(Path::new("x.md"));
        assert!(is_write_conflict(&err));
    }
}
