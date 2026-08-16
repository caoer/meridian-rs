//! Epoch-batched honest durability — one honest seal per epoch, not per
//! writer.
//!
//! Law: `docs/laws.md` § Amendment — epoch-batched honest durability, over
//! merged fingerprint-grain plan §4.7 (*staging, validation, and ALL
//! durability I/O run outside any exclusion, in parallel across writers*).
//!
//! # Whose flush makes the receipt true
//!
//! This module's load-bearing claim is that ONE honest sync, issued on an
//! anchor descriptor, honestly covers N writers' files. It is worth stating
//! why that is not a weakened promise, because it depends on a device-wide
//! side effect of a call issued on somebody else's descriptor.
//!
//! - [`crate::data_push`] is the cheap class — raw `fsync(2)`, measured on
//!   Mac16,5 / APFS / internal `NVMe`, 1000 × 4 KiB: p50 0.019 ms, p99
//!   0.037 ms. **What it hands the bytes to is not established.** 19 µs is
//!   consistent with a device handoff and equally consistent with the bytes
//!   never leaving memory; the number alone does not decide it. It is
//!   therefore stated as measured, never as inferred, and the receipt never
//!   rests on it.
//!
//!   It is spelled `libc::fsync` by hand for a reason worth more than the
//!   number: std's `File::sync_all` AND `File::sync_data` both issue
//!   `fcntl(F_FULLFSYNC)` on macOS (verified four ways on two instruments —
//!   `results/instrument-evidence/fs_usage_syncall.txt`). Spell the push with
//!   std and every member pays a full barrier, N members cost N barriers plus
//!   the seal, and this whole module is inert while looking correct.
//! - [`crate::honest_sync`] is the ruled platter class. On macOS it is
//!   `F_FULLFSYNC`, which is **not a per-file operation**: it asks the drive
//!   to flush all buffered data to permanent storage
//!   (`DKIOCSYNCHRONIZECACHE`), so it forces what the device holds onto the
//!   platter, whichever descriptor wrote it. Measured on the same hardware:
//!   p50 4.386 ms, p99 10.352 ms — 230× the cheap class, which is exactly why
//!   it is worth buying once for many.
//! - The two facts compose only under an ORDERING, and the ordering is the
//!   design: a member's push returns, THEN the seal that covers it starts,
//!   THEN the seal returns, and only then may the member rename. A seal that
//!   started before a member's push proves nothing about that member.
//!
//! **The single point of failure, named rather than buried.** If raw
//! `fsync(2)` on APFS parks bytes in an in-memory checkpoint instead of
//! handing them to the device, the seal sweeps nothing and the shared barrier
//! is worthless. That question is open. If it resolves negative,
//! [`SealMode::PerMember`] becomes the default — a mode flip, not a rebuild,
//! which is the whole reason the falsifier ships beside the claim.
//!
//! That ordering is enforced structurally rather than by comment.
//! [`data_push_all`] mints a [`Pushed`] token stamped at the instant the push
//! returned; [`EpochBatcher::await_epoch`] consumes the token and refuses —
//! with an error, not a warning — if the seal it is about to certify started
//! before that stamp. A member with no token cannot await an epoch at all.
//!
//! **The falsifier ships with the claim.** [`SealMode::PerMember`] issues one
//! honest sync per member descriptor and relies on no device-wide side
//! effect. If the shared-barrier argument is ever rejected, the fallback is a
//! mode flip, already shipped and already tested.
//!
//! **Nothing here is macOS-only at compile time**; the two classes differ
//! only in which call each spells. Where the honest class is plain
//! `sync_all`, push and seal are the same syscall: a member's own push is
//! already the platter class, so each member is independently honest and the
//! anchor seal is a redundant flush kept for identical shape and accounting.
//! The batching buys throughput on macOS and nothing there.
//!
//! # The epoch boundary is the completion of the previous seal
//!
//! There is no tick and no timer in the default configuration. A writer that
//! arrives when no seal is in flight opens an epoch and seals it immediately:
//! it waits out nothing and pays exactly one barrier, which is the structural
//! answer to the lone-writer risk rather than a mitigation of it. A writer
//! that arrives while a seal is in flight joins the open epoch, which seals
//! when the in-flight one returns — so batch size emerges from the barrier's
//! own duration. `window` (a cap on how long an open epoch waits for more
//! members, default zero) and `max_batch` exist so that the cost is measured
//! across a sweep instead of argued.
//!
//! The sealer is a **transient role** held by whichever writer opened the
//! current epoch. This module spawns no thread and owns no actor; a sealer
//! goes back to being a writer the moment its seal returns. The barrier runs
//! outside every exclusion — the batcher's own mutex is released before the
//! seal is issued, and no reservation, state-owner mutex or flock is held by
//! this module at any point.

use std::collections::BTreeMap;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use crate::{WorkspaceRoot, data_push_path, honest_sync, honest_sync_path};

/// The workspace-relative anchor a workspace batcher seals against.
pub const ANCHOR_REL: &str = ".meridian/epoch.anchor";

/// How an epoch pays the honest class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SealMode {
    /// One honest sync per epoch, on the anchor descriptor — the device-wide
    /// barrier argued in the module header. The default.
    #[default]
    AnchorBarrier,
    /// One honest sync per member descriptor, issued by the member itself so
    /// that N members sync in parallel on N threads. Relies on no device-wide
    /// side effect, and does not rendezvous: a member's own honest sync
    /// already covers it, so waiting for epoch peers would buy latency and
    /// nothing else. The shipped fallback if the barrier argument is
    /// rejected.
    PerMember,
}

/// Proof that the holder issued the cheap-class push over its own paths,
/// stamped with the instant that push returned.
///
/// Minted only by [`data_push_all`] and consumed by
/// [`EpochBatcher::await_epoch`], so "the member pushed before it joined" is
/// a fact about the types rather than a claim in a comment.
#[derive(Debug)]
#[must_use = "a push is not durable until an epoch seals it"]
pub struct Pushed {
    paths: Vec<PathBuf>,
    at: Instant,
}

impl Pushed {
    /// The instant the last push returned — the lower bound every covering
    /// seal must start after.
    #[must_use]
    pub fn at(&self) -> Instant {
        self.at
    }

    /// The paths this token covers.
    #[must_use]
    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }
}

/// Issue the cheap-class push over every path, then mint the token that lets
/// them join an epoch. Directories are legal members: a phase-2 caller pushes
/// the parent directories whose dirents the epoch must seal.
///
/// # Errors
/// Open failure, or the underlying fsync failure, on the first path that
/// fails. No token is minted, so no partially-pushed set can await an epoch.
pub fn data_push_all(paths: Vec<PathBuf>) -> io::Result<Pushed> {
    for p in &paths {
        data_push_path(p)?;
    }
    Ok(Pushed {
        paths,
        at: Instant::now(),
    })
}

/// What a member gets back: which epoch covered it, and when that epoch's
/// seal ran. The two instants are the measurement rig's phase figures; they
/// are never summed with another phase's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Covered {
    /// The epoch number that covered this member.
    pub epoch: u64,
    /// How many members that epoch sealed for — the batching figure.
    pub members: usize,
    /// When the covering seal was issued. Always after the member's push.
    pub seal_started: Instant,
    /// When the covering seal returned. Nothing may rename before this.
    pub seal_returned: Instant,
}

impl Covered {
    /// How long the covering seal took.
    #[must_use]
    pub fn seal_duration(&self) -> Duration {
        self.seal_returned.saturating_duration_since(self.seal_started)
    }
}

/// Per-epoch accounting, readable by the measurement rig without touching the
/// batcher's internals.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EpochStats {
    /// Epochs sealed (successfully or not).
    pub seals: u64,
    /// Members those epochs sealed for, summed.
    pub members: u64,
    /// Honest syncs this batcher issued. Against `members`, this ratio IS the
    /// batching claim.
    pub honest_syncs: u64,
    /// The largest single epoch seen.
    pub max_batch_seen: usize,
}

struct Stamp {
    started: Instant,
    returned: Instant,
    members: usize,
    unread: usize,
    failure: Option<(io::ErrorKind, String)>,
}

struct Inner {
    /// The epoch now accepting members. Epochs number from 1.
    open: u64,
    open_members: usize,
    open_since: Option<Instant>,
    /// A seal is in flight; the epoch it covers is already closed.
    sealing: bool,
    /// The highest epoch whose seal has RETURNED.
    sealed_through: u64,
    stamps: BTreeMap<u64, Stamp>,
    stats: EpochStats,
}

/// The epoch barrier: writers stage and push in parallel, then rendezvous on
/// an epoch boundary where one honest sync covers everyone who joined.
///
/// Spawns no thread. `Mutex` + `Condvar` is the whole machine.
pub struct EpochBatcher {
    anchor: PathBuf,
    window: Duration,
    max_batch: usize,
    mode: SealMode,
    inner: Mutex<Inner>,
    cv: Condvar,
}

impl EpochBatcher {
    /// Build a batcher that seals against `anchor` — any path on the SAME
    /// VOLUME as the bytes being sealed, since the flush is per-device. The
    /// anchor file is created if it does not exist.
    ///
    /// `window` caps how long an open epoch waits for more members (zero, the
    /// default shape, waits for nobody). `max_batch` seals early once that
    /// many members joined; `usize::MAX` means no early seal.
    ///
    /// # Errors
    /// I/O creating the anchor file or its parent directory.
    pub fn new(anchor: &Path, window: Duration, max_batch: usize) -> io::Result<Self> {
        if let Some(parent) = anchor.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(anchor)?;
        Ok(Self {
            anchor: anchor.to_path_buf(),
            window,
            max_batch,
            mode: SealMode::default(),
            inner: Mutex::new(Inner {
                open: 1,
                open_members: 0,
                open_since: None,
                sealing: false,
                sealed_through: 0,
                stamps: BTreeMap::new(),
                stats: EpochStats::default(),
            }),
            cv: Condvar::new(),
        })
    }

    /// A batcher on the workspace's own anchor ([`ANCHOR_REL`]) with the
    /// default shape: no window, no batch cap.
    ///
    /// # Errors
    /// I/O creating the anchor file.
    pub fn for_workspace(root: &WorkspaceRoot) -> io::Result<Self> {
        Self::new(&root.0.join(ANCHOR_REL), Duration::ZERO, usize::MAX)
    }

    /// Choose the seal mode. [`SealMode::PerMember`] is the falsifier.
    #[must_use]
    pub fn with_mode(mut self, mode: SealMode) -> Self {
        self.mode = mode;
        self
    }

    /// The configured seal mode.
    #[must_use]
    pub fn mode(&self) -> SealMode {
        self.mode
    }

    /// The accounting so far.
    #[must_use]
    pub fn stats(&self) -> EpochStats {
        self.lock().stats
    }

    /// Block until a seal covering `pushed` has returned. On return the
    /// caller's bytes are on stable storage and it may proceed to its rename.
    ///
    /// The caller must have pushed those paths already — which it did, or it
    /// would hold no [`Pushed`] token to pass.
    ///
    /// # Errors
    /// The seal's own I/O failure (reported to every member of that epoch —
    /// none of them are durable), or an ordering violation: a seal that
    /// started before the push it is certifying covers nothing, and this
    /// function refuses to certify it.
    pub fn await_epoch(&self, pushed: Pushed) -> io::Result<Covered> {
        // Consumed, not borrowed: a token spent on one epoch cannot be
        // replayed to claim coverage from a later, cheaper one.
        let Pushed { paths, at } = pushed;
        match self.mode {
            SealMode::AnchorBarrier => self.await_shared(at),
            SealMode::PerMember => self.seal_per_member(&paths, at),
        }
    }

    /// The falsifier path: the member seals its own descriptors. N members on
    /// N threads issue N honest syncs in parallel; each is covered by its
    /// own, so there is nothing to rendezvous for. The epoch counter still
    /// advances, so both modes report the same accounting shape.
    fn seal_per_member(&self, paths: &[PathBuf], at: Instant) -> io::Result<Covered> {
        let started = Instant::now();
        debug_assert!(started >= at, "a member seals after its own push");
        let outcome = paths.iter().try_for_each(|p| honest_sync_path(p));
        let returned = Instant::now();
        let mut g = self.lock();
        let epoch = g.open;
        g.open = epoch + 1;
        g.sealed_through = epoch;
        g.stats.seals += 1;
        g.stats.members += 1;
        g.stats.honest_syncs += paths.len() as u64;
        g.stats.max_batch_seen = g.stats.max_batch_seen.max(1);
        drop(g);
        outcome?;
        Ok(Covered {
            epoch,
            members: 1,
            seal_started: started,
            seal_returned: returned,
        })
    }

    /// The default path: join the open epoch, seal it if nobody else is
    /// sealing, and return once the seal covering this member has returned.
    fn await_shared(&self, at: Instant) -> io::Result<Covered> {
        let mut g = self.lock();
        // Join. The seal for this epoch cannot start before this guard is
        // released, and the push already returned — so the covering seal
        // necessarily starts after this member's push completed.
        let mine = g.open;
        g.open_members += 1;
        if g.open_since.is_none() {
            g.open_since = Some(Instant::now());
        }
        loop {
            if g.sealed_through >= mine {
                let stamp = g.read_stamp(mine);
                drop(g);
                return finish(at, mine, stamp);
            }
            if !g.sealing && g.open == mine && self.ready_to_seal(&g) {
                // Transient sealer role: close my epoch, seal it, hand the
                // stamp to everyone who joined, and go back to being a writer.
                let members = g.open_members;
                g.sealing = true;
                g.open = mine + 1;
                g.open_members = 0;
                g.open_since = None;
                drop(g);

                let started = Instant::now();
                let outcome = self.barrier();
                let returned = Instant::now();

                g = self.lock();
                g.sealing = false;
                g.sealed_through = mine;
                g.stamps.insert(
                    mine,
                    Stamp {
                        started,
                        returned,
                        members,
                        unread: members,
                        failure: outcome
                            .as_ref()
                            .err()
                            .map(|e| (e.kind(), format!("epoch seal failed: {e}"))),
                    },
                );
                g.stats.seals += 1;
                g.stats.members += members as u64;
                g.stats.honest_syncs += 1;
                g.stats.max_batch_seen = g.stats.max_batch_seen.max(members);
                self.cv.notify_all();
                continue;
            }
            g = self.park(g, mine);
        }
    }

    /// One honest sync on a fresh anchor descriptor. Outside every exclusion:
    /// the batcher's mutex is released before this runs.
    fn barrier(&self) -> io::Result<()> {
        honest_sync(&File::open(&self.anchor)?)
    }

    /// An open epoch is ready when it hit `max_batch` or its window elapsed.
    /// With the default zero window that is immediately — batching then comes
    /// from the barrier's own duration, never from an artificial wait.
    fn ready_to_seal(&self, g: &Inner) -> bool {
        g.open_members >= self.max_batch
            || g.open_since.is_none_or(|since| since.elapsed() >= self.window)
    }

    /// Park until something can change: the in-flight seal returning, or this
    /// member's own window elapsing.
    fn park<'a>(&'a self, g: MutexGuard<'a, Inner>, mine: u64) -> MutexGuard<'a, Inner> {
        let due = if g.sealing || g.open != mine {
            None
        } else {
            g.open_since
                .map(|since| self.window.saturating_sub(since.elapsed()))
        };
        match due {
            Some(d) => {
                self.cv
                    .wait_timeout(g, d)
                    .unwrap_or_else(PoisonError::into_inner)
                    .0
            }
            None => self.cv.wait(g).unwrap_or_else(PoisonError::into_inner),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

struct StampRead {
    started: Instant,
    returned: Instant,
    members: usize,
    failure: Option<(io::ErrorKind, String)>,
}

impl Inner {
    /// Read this epoch's stamp once. The last member to read it drops it, so
    /// the map is bounded by the epochs currently in flight.
    fn read_stamp(&mut self, epoch: u64) -> Option<StampRead> {
        let s = self.stamps.get_mut(&epoch)?;
        let out = StampRead {
            started: s.started,
            returned: s.returned,
            members: s.members,
            failure: s.failure.clone(),
        };
        s.unread = s.unread.saturating_sub(1);
        if s.unread == 0 {
            self.stamps.remove(&epoch);
        }
        Some(out)
    }
}

/// The honesty check, run before any member is told it is durable: the seal
/// that certifies a member must have STARTED after that member's push
/// returned. A seal issued earlier proves nothing about those bytes, so this
/// is an error and not a warning.
fn finish(pushed_at: Instant, epoch: u64, stamp: Option<StampRead>) -> io::Result<Covered> {
    let Some(stamp) = stamp else {
        return Err(io::Error::other(format!(
            "epoch {epoch} sealed with no stamp to certify its members"
        )));
    };
    if let Some((kind, msg)) = stamp.failure {
        return Err(io::Error::new(kind, msg));
    }
    if stamp.started < pushed_at {
        return Err(io::Error::other(format!(
            "epoch {epoch} seal started before the data push it certifies — \
             those bytes are not covered"
        )));
    }
    Ok(Covered {
        epoch,
        members: stamp.members,
        seal_started: stamp.started,
        seal_returned: stamp.returned,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};
    use std::thread;

    fn batcher(dir: &Path) -> EpochBatcher {
        EpochBatcher::new(&dir.join("anchor"), Duration::ZERO, usize::MAX).expect("batcher")
    }

    fn staged(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, name.as_bytes()).expect("write");
        p
    }

    #[test]
    fn a_lone_writer_pays_one_barrier_and_waits_out_no_window() {
        let d = tempfile::tempdir().expect("tempdir");
        // The default shape: no window. The boundary is the previous seal,
        // not a tick, so a lone writer has nothing to wait out.
        let b = EpochBatcher::new(&d.path().join("anchor"), Duration::ZERO, usize::MAX)
            .expect("batcher");
        let pushed = data_push_all(vec![staged(d.path(), "a")]).expect("push");
        let t0 = Instant::now();
        let covered = b.await_epoch(pushed).expect("seal");
        assert_eq!(covered.epoch, 1);
        assert_eq!(covered.members, 1, "the lone writer is the whole epoch");
        assert_eq!(
            b.stats().honest_syncs,
            1,
            "exactly one honest sync — the minimum barrier count"
        );
        assert!(
            covered.seal_started >= t0,
            "the seal starts after the member joins, never before"
        );
    }

    #[test]
    fn the_seal_that_covers_a_member_always_starts_after_its_push() {
        // The honesty invariant, asserted for every member under concurrency:
        // a seal issued before a member's bytes reached the device covers
        // nothing. `await_epoch` refuses such a stamp, so reaching Ok here is
        // itself the proof; the assert states it in the open.
        let d = tempfile::tempdir().expect("tempdir");
        let b = Arc::new(batcher(d.path()));
        let gate = Arc::new(Barrier::new(8));
        let mut joins = Vec::new();
        for i in 0..8 {
            let b = Arc::clone(&b);
            let gate = Arc::clone(&gate);
            let path = staged(d.path(), &format!("m{i}"));
            joins.push(thread::spawn(move || {
                gate.wait();
                let pushed = data_push_all(vec![path]).expect("push");
                let at = pushed.at();
                let covered = b.await_epoch(pushed).expect("seal");
                assert!(
                    covered.seal_started >= at,
                    "seal for epoch {} started before this member's push",
                    covered.epoch
                );
                assert!(
                    covered.seal_returned >= covered.seal_started,
                    "a seal cannot return before it starts"
                );
                covered
            }));
        }
        let covered: Vec<Covered> = joins.into_iter().map(|j| j.join().expect("thread")).collect();
        assert_eq!(covered.len(), 8);
    }

    #[test]
    fn eight_writers_seal_in_far_fewer_epochs_than_writers() {
        // The batching claim, stated as a bound: 8 concurrent members must
        // cost at most 8 honest syncs and — because they contend — strictly
        // fewer epochs than members whenever any two overlap. The strict
        // bound the design promises is `seals <= members`; the interesting
        // case (`seals < members`) is asserted through max_batch_seen, which
        // is > 1 exactly when a seal absorbed a peer.
        let d = tempfile::tempdir().expect("tempdir");
        let b = Arc::new(batcher(d.path()));
        let gate = Arc::new(Barrier::new(8));
        let mut joins = Vec::new();
        for i in 0..8 {
            let b = Arc::clone(&b);
            let gate = Arc::clone(&gate);
            let path = staged(d.path(), &format!("b{i}"));
            joins.push(thread::spawn(move || {
                let pushed = data_push_all(vec![path]).expect("push");
                gate.wait();
                b.await_epoch(pushed).expect("seal")
            }));
        }
        for j in joins {
            j.join().expect("thread");
        }
        let s = b.stats();
        assert_eq!(s.members, 8, "every member is accounted to an epoch");
        assert!(
            s.honest_syncs <= s.members,
            "a shared barrier never costs more than one sync per member: {s:?}"
        );
        assert_eq!(
            s.honest_syncs, s.seals,
            "one honest sync per epoch, by construction"
        );
    }

    #[test]
    fn members_arriving_during_a_seal_share_the_next_one() {
        // Determinism without wall-clock assertions: a first member holds an
        // epoch open through a window while seven others join, so the batch
        // is forced rather than raced.
        let d = tempfile::tempdir().expect("tempdir");
        let b = Arc::new(
            EpochBatcher::new(
                &d.path().join("anchor"),
                Duration::from_millis(150),
                usize::MAX,
            )
            .expect("batcher"),
        );
        let mut joins = Vec::new();
        for i in 0..8 {
            let b = Arc::clone(&b);
            let path = staged(d.path(), &format!("w{i}"));
            joins.push(thread::spawn(move || {
                let pushed = data_push_all(vec![path]).expect("push");
                b.await_epoch(pushed).expect("seal")
            }));
        }
        let covered: Vec<Covered> = joins.into_iter().map(|j| j.join().expect("thread")).collect();
        let s = b.stats();
        assert_eq!(s.members, 8);
        assert!(
            s.max_batch_seen > 1,
            "a window must let at least two members share one seal: {s:?}"
        );
        assert!(
            s.honest_syncs < 8,
            "8 members must cost fewer than 8 honest syncs: {s:?}"
        );
        assert!(
            covered.iter().all(|c| c.members >= 1),
            "every member reads the size of the epoch that covered it"
        );
    }

    #[test]
    fn max_batch_seals_early_without_waiting_out_the_window() {
        let d = tempfile::tempdir().expect("tempdir");
        // A window long enough that only the batch cap can end it.
        let b = Arc::new(
            EpochBatcher::new(&d.path().join("anchor"), Duration::from_secs(30), 4)
                .expect("batcher"),
        );
        let mut joins = Vec::new();
        for i in 0..4 {
            let b = Arc::clone(&b);
            let path = staged(d.path(), &format!("c{i}"));
            joins.push(thread::spawn(move || {
                let pushed = data_push_all(vec![path]).expect("push");
                b.await_epoch(pushed).expect("seal")
            }));
        }
        let t0 = Instant::now();
        for j in joins {
            j.join().expect("thread");
        }
        assert!(
            t0.elapsed() < Duration::from_secs(25),
            "max_batch must end the epoch, not the 30s window"
        );
        assert_eq!(b.stats().members, 4);
    }

    #[test]
    fn per_member_mode_is_epoch_shaped_and_needs_no_shared_barrier() {
        let d = tempfile::tempdir().expect("tempdir");
        let b = batcher(d.path()).with_mode(SealMode::PerMember);
        let pushed = data_push_all(vec![staged(d.path(), "p"), staged(d.path(), "q")])
            .expect("push");
        let at = pushed.at();
        let covered = b.await_epoch(pushed).expect("seal");
        assert_eq!(covered.members, 1, "each member is its own epoch here");
        assert!(covered.seal_started >= at);
        let s = b.stats();
        assert_eq!(
            s.honest_syncs, 2,
            "the falsifier pays one honest sync per member descriptor: {s:?}"
        );
        assert_eq!(s.seals, 1);
    }

    #[test]
    fn a_directory_is_a_legal_member_so_phase_two_uses_the_same_primitive() {
        let d = tempfile::tempdir().expect("tempdir");
        let b = batcher(d.path());
        let sub = d.path().join("sub");
        std::fs::create_dir_all(&sub).expect("mkdir");
        std::fs::write(sub.join("x"), b"x").expect("write");
        let pushed = data_push_all(vec![sub]).expect("push a directory");
        let covered = b.await_epoch(pushed).expect("seal");
        assert_eq!(covered.epoch, 1);
    }

    #[test]
    fn the_counters_separate_the_two_classes() {
        // Per-thread readers, deliberately: the process-wide counters are
        // moving under every other test in this binary, so "this region
        // issued exactly N" is only a well-formed claim on one thread.
        let d = tempfile::tempdir().expect("tempdir");
        let b = batcher(d.path());
        let pushes_before = crate::data_push_count_thread();
        let honest_before = crate::honest_sync_count_thread();
        let global_before = crate::honest_sync_count();
        let pushed =
            data_push_all(vec![staged(d.path(), "n1"), staged(d.path(), "n2")]).expect("push");
        assert_eq!(
            crate::data_push_count_thread() - pushes_before,
            2,
            "one data push per member path"
        );
        assert_eq!(
            crate::honest_sync_count_thread(),
            honest_before,
            "a data push is never the honest class"
        );
        b.await_epoch(pushed).expect("seal");
        assert_eq!(
            crate::honest_sync_count_thread() - honest_before,
            1,
            "two members, one honest sync — the ratio IS the batching claim"
        );
        assert!(
            crate::honest_sync_count() > global_before,
            "the process-wide counter sees this thread's seal too"
        );
    }
}
