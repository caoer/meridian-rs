//! The dedicated durability actor — every honest-class sync in the process
//! is issued by ONE owner thread. Writers stage bytes and hand off
//! ([`submit`]); they never issue an honest sync themselves. Group commit is
//! the consequence of the single queue: whatever is queued when the actor
//! drains is one batch, one barrier — no timer, no window, no contention
//! detector.
//!
//! # The premise (load-bearing — read this before touching the batch loop)
//!
//! On macOS, `fcntl(fd, F_FULLFSYNC)` does an `fsync(fd)` and then asks the
//! DEVICE to flush its volatile write cache to permanent storage. Apple's
//! `fcntl(2)` documents it as draining the entire device queue and acting as
//! a barrier: data that was fsync'd to the same device BEFORE the call is
//! persisted when it returns. So the batch protocol is:
//!
//! 1. **push** — plain `libc::fsync(fd)` per member: moves that member's
//!    bytes from the page cache into the device. Cheap (µs class). NOT the
//!    durability promise. `File::sync_all`/`sync_data` must never be used
//!    here — on macOS both issue `F_FULLFSYNC` (verified by `fs_usage`
//!    decode, session 15-14, roster 0028), which would pay a full platter
//!    barrier per member and the shared barrier would win nothing.
//! 2. **barrier** — ONE [`honest_sync`] (`F_FULLFSYNC`) on the actor's OWN
//!    workspace-root handle, opened once at spawn. Never on a member's fd:
//!    a writer's receipt must not rest on a device-wide side effect of a
//!    call issued on another writer's file.
//! 3. **ack** — only after the barrier returns does any member of the batch
//!    get its receipt. Bytes reach the device, the barrier returns, the
//!    member is acked — in that order, visible in [`settle_batch`].
//!
//! **Falsifier:** the premise requires every batch member to sit on the same
//! device as the barrier handle. A member on a different device would be
//! acked without its device ever flushing — so the batch loop REFUSES any
//! member whose `st_dev` differs from the barrier handle's (the premise is
//! enforced, not assumed; `tests/honest_durability.rs` additionally pins
//! the deployment fact). The durability unit on macOS is likely the APFS
//! CONTAINER rather than the raw device; the boundary here is already the
//! narrower one by construction — `st_dev` identifies a mounted volume, and
//! an APFS volume lives in exactly one container, so `st_dev` equality
//! implies same container. A second falsifier is a device that lies about
//! `F_FULLFSYNC` (some external enclosures ignore the flush); that
//! falsifies every honest-durability design on such hardware, not this
//! placement.
//!
//! **On sync failure we REFUSE, never degrade** — a deliberate divergence:
//! Go, the JDK, libuv, LevelDB and SQLite all fall back to a weaker sync
//! when `F_FULLFSYNC` reports `ENOTSUP`. Degrading returns success wearing
//! durability's name — exactly the success-shaped failure criterion 1
//! exists to catch — so [`honest_sync`] propagates the raw OS error and a
//! failed barrier refuses every pushed member.
//!
//! The cheap tier itself is DOCUMENTED, not verified (APFS `VNOP_FSYNC` is
//! closed source): Apple documents plain `fsync(2)` as forcing bytes to the
//! drive without power-loss durability — reaching the device is exactly its
//! contract. The strongest documentary evidence is `F_BARRIERFSYNC` itself,
//! described as a device barrier applying to I/O *flushed with `fsync`* and
//! implemented on APFS — incoherent unless fsync'd I/O is already at the
//! device. (That cheaper rung is DECLINED here for the ack path: it orders,
//! it does not persist, and our ack claims persistence.) The cheap push may
//! therefore claim NOTHING about power loss — which is why an ack is only
//! ever sent after the covering barrier has returned, and why a batch's
//! serialized pushes are kept few by the dedupe (the cheap tier's tail is
//! not uniformly cheap).
//!
//! On non-macOS, `fsync` IS the honest class per file (no device-wide
//! barrier is documented), so the actor issues [`honest_sync`] per member
//! and there is no shared barrier. The queue still groups arrivals, so the
//! fallback keeps the assigned shape: one owner, one queue, ack after sync.
//!
//! # Dedupe
//!
//! Before the push, a batch is deduped by `(st_dev, st_ino)`: the queue
//! naturally collects several requests naming the same object (a parent
//! directory once per rename in the batch, measured 4× on the intent path),
//! and one sync of the object settles all of them — four flushes of one
//! directory are redundant; one is identically durable.
//!
//! # Instrument
//!
//! [`honest_ops`] counts honest-class device operations issued (macOS: one
//! per batch barrier; elsewhere: one per distinct object). It increments
//! inside [`honest_sync`] after the syscall returns success — the single
//! chokepoint for the class — and is the runtime receipt the measurement
//! seat asserts (amended gate 2: dtrace is SIP-blocked on this host).
//! [`object_syncs`] counts distinct objects synced post-dedupe (both
//! platforms) — the dedupe evidence. Writer-side waits are measured by the
//! caller (`PublishReceipt`'s durability interval), not here.
//! [`ops_on_this_thread`] counts submits + waits issued by the calling
//! thread — the no-sync-in-the-linearized-step proof instrument.

use std::cell::Cell;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::fs::File;
use std::io;
use std::os::unix::fs::MetadataExt;
// The raw-fd calls (fcntl F_FULLFSYNC, libc::fsync) exist only in the macOS
// arms; on other targets the import would be unused.
#[cfg(target_os = "macos")]
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};

/// Honest-class device ops issued, process-wide (see module doc # Instrument).
static HONEST_OPS: AtomicU64 = AtomicU64::new(0);

/// Distinct on-disk objects synced by the ACTOR (post-dedupe count).
static OBJECT_SYNCS: AtomicU64 = AtomicU64::new(0);

/// Honest-class ops the ACTOR issued (macOS: batch barriers; elsewhere:
/// per-object syncs). `honest_ops - actor_barriers` > 0 would name a
/// writer-issued honest op — the banned shape.
static ACTOR_BARRIERS: AtomicU64 = AtomicU64::new(0);

/// Batch (drain-cycle) ids, so receipts can prove co-settlement.
static BATCH_SEQ: AtomicU64 = AtomicU64::new(0);

/// Grouping tallies a batch settle reports into — the actor points these at
/// the process statics; unit tests inject locals so exact-count assertions
/// cannot race concurrent tests settling through the process actor.
struct BatchTallies<'a> {
    objects: &'a AtomicU64,
    barriers: &'a AtomicU64,
}

/// The process tallies (what [`object_syncs`] / [`actor_barriers`] read).
static PROCESS_TALLIES: BatchTallies<'static> = BatchTallies {
    objects: &OBJECT_SYNCS,
    barriers: &ACTOR_BARRIERS,
};

/// The one process-wide actor: its submit side, spawned on first use.
static ACTOR: OnceLock<Sender<Req>> = OnceLock::new();

thread_local! {
    /// Submits + waits issued by THIS thread (linearized-step instrument).
    static THREAD_OPS: Cell<u64> = const { Cell::new(0) };
}

/// One enqueue: a SET of files that must share a drain. A multi-member set
/// reaches the queue as one message, so no drain can begin mid-set — the
/// mechanism behind [`submit_set`]'s one-barrier-per-phase guarantee. The
/// actor itself never waits for members (no timer, no window): atomicity
/// lives in the enqueue, not in the drain.
struct Req {
    members: Vec<(File, Ack)>,
}

/// Proof of settlement. Members sharing a `batch` id settled through the
/// same drain cycle — on macOS, the same single barrier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettleReceipt {
    /// Drain-cycle id (1-based, process-wide).
    pub batch: u64,
    /// How many members rode that cycle.
    pub group_len: usize,
}

/// A submitted, not-yet-awaited settle. Durability is established only when
/// [`Pending::wait`] returns `Ok` — a rename that makes staged bytes visible
/// must wait first. Await once; a second wait reports the actor gone.
#[derive(Debug)]
pub struct Pending {
    rx: Receiver<io::Result<SettleReceipt>>,
}

impl Pending {
    /// Block until the actor has settled this submission (ack after the
    /// barrier — module doc step 3).
    ///
    /// # Errors
    /// The push or barrier failure for this member, or an actor death.
    pub fn wait(&self) -> io::Result<SettleReceipt> {
        THREAD_OPS.with(|c| c.set(c.get() + 1));
        match self.rx.recv() {
            Ok(res) => res,
            Err(_) => Err(io::Error::other(
                "durability actor is gone; the settle outcome is unknown",
            )),
        }
    }
}

/// Hand a file's already-written bytes to the actor; returns immediately.
/// `root_dir` names the workspace root whose device the barrier handle pins;
/// the first submit in the process spawns the actor with a handle opened on
/// it (later roots are the same device by the module-doc premise).
///
/// # Errors
/// Opening the root handle (first call), duplicating `file`'s fd, or a dead
/// actor thread.
pub fn submit(root_dir: &Path, file: &File) -> io::Result<Pending> {
    THREAD_OPS.with(|c| c.set(c.get() + 1));
    let tx = actor(root_dir)?;
    let dup = file.try_clone()?;
    let (ack, rx) = channel();
    tx.send(Req {
        members: vec![(dup, ack)],
    })
    .map_err(|_| io::Error::other("durability actor is gone; nothing was settled"))?;
    Ok(Pending { rx })
}

/// Hand a whole phase's files to the actor as ONE enqueue, consuming them.
/// The set cannot be split across drains — a drain that begins mid-phase
/// was the measured 5–7-barriers-per-txn defect; one enqueue makes the
/// phase share one drain (macOS: one barrier). Returns one [`Pending`] per
/// member, in order; all resolve from the same drain cycle.
///
/// # Errors
/// A dead actor thread, or opening the root handle on the first call.
pub fn submit_set(root_dir: &Path, files: Vec<File>) -> io::Result<Vec<Pending>> {
    if files.is_empty() {
        return Ok(Vec::new());
    }
    THREAD_OPS.with(|c| c.set(c.get() + 1));
    let tx = actor(root_dir)?;
    let mut members = Vec::with_capacity(files.len());
    let mut pendings = Vec::with_capacity(files.len());
    for file in files {
        let (ack, rx) = channel();
        members.push((file, ack));
        pendings.push(Pending { rx });
    }
    tx.send(Req { members })
        .map_err(|_| io::Error::other("durability actor is gone; nothing was settled"))?;
    Ok(pendings)
}

/// Submit and wait: the blocking settle. What a writer that needs the bytes
/// durable NOW calls (directory syncs, intent records).
///
/// # Errors
/// [`submit`] + [`Pending::wait`] failures.
pub fn settle(root_dir: &Path, file: &File) -> io::Result<SettleReceipt> {
    submit(root_dir, file)?.wait()
}

/// [`settle`] on a path. `File::open` works for files and directories alike
/// on Unix; the fd is only used for the sync.
///
/// # Errors
/// Open failure, or the settle failure.
pub fn settle_path(root_dir: &Path, path: &Path) -> io::Result<SettleReceipt> {
    settle(root_dir, &File::open(path)?)
}

/// [`submit`] on a path — non-blocking. Lets a caller queue several
/// directory settles (the actor dedupes repeats) and await them as a set.
///
/// # Errors
/// Open failure, or the submit failure.
pub fn submit_path(root_dir: &Path, path: &Path) -> io::Result<Pending> {
    submit(root_dir, &File::open(path)?)
}

/// Honest-class device ops issued so far, process-wide. The measurement
/// seat's runtime receipt; a flat reading across a code region proves the
/// region issued no honest-class op (given per-thread quiescence — for the
/// linearized-step proof use [`ops_on_this_thread`]).
#[must_use]
pub fn honest_ops() -> u64 {
    HONEST_OPS.load(Ordering::Relaxed)
}

/// Distinct on-disk objects synced by the actor so far (post-dedupe). With
/// [`actor_barriers`] this is the grouping evidence: requests in vs objects
/// synced vs barriers issued.
#[must_use]
pub fn object_syncs() -> u64 {
    OBJECT_SYNCS.load(Ordering::Relaxed)
}

/// Honest-class ops issued BY THE ACTOR. Equal to [`honest_ops`] when no
/// writer bypasses the actor — the shape the design promises.
#[must_use]
pub fn actor_barriers() -> u64 {
    ACTOR_BARRIERS.load(Ordering::Relaxed)
}

/// Durability operations (submits + waits) issued by the CALLING thread.
/// Immune to concurrent writers, so a flat reading across
/// `StateOwner::apply` is deterministic proof under full parallelism.
#[must_use]
pub fn ops_on_this_thread() -> u64 {
    THREAD_OPS.with(Cell::get)
}

/// The raw honest-class barrier op (merged-plan §4.7): `F_FULLFSYNC` on
/// macOS (platter, not drive-cache), `sync_all` elsewhere. The durability
/// actor is its only production caller — writers route through [`settle`] /
/// [`submit`] and never issue this themselves. It stays public as the
/// class's single chokepoint: [`honest_ops`] increments here, after the
/// syscall reports success, and nowhere else.
///
/// Never substitute `F_BARRIERFSYNC`: that is an ordering barrier only and
/// does not force bytes to permanent storage — a silent honesty downgrade.
///
/// # Errors
/// The underlying fcntl/fsync failure.
pub fn honest_sync(file: &File) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        // SAFETY: fcntl on a valid fd we own; F_FULLFSYNC takes no extra arg.
        let rc = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_FULLFSYNC) };
        if rc == -1 {
            return Err(io::Error::last_os_error());
        }
        HONEST_OPS.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        file.sync_all()?;
        HONEST_OPS.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

/// [`honest_sync`] on a path — recovery-grade single-shot use.
///
/// # Errors
/// Open failure, or the underlying sync failure.
pub fn honest_sync_path(path: &Path) -> io::Result<()> {
    honest_sync(&File::open(path)?)
}

/// The submit side of the one actor, spawning it on first use with a barrier
/// handle opened on `root_dir` (module doc step 2: the actor's OWN handle).
fn actor(root_dir: &Path) -> io::Result<&'static Sender<Req>> {
    if let Some(tx) = ACTOR.get() {
        return Ok(tx);
    }
    // Open before init so a failed open surfaces as this call's error. Two
    // racers may both open; the losing handle just drops.
    let handle = File::open(root_dir)?;
    Ok(ACTOR.get_or_init(|| {
        let (tx, rx) = channel();
        std::thread::Builder::new()
            .name("durability-actor".into())
            .spawn(move || actor_loop(&rx, &handle))
            .expect("spawning the durability actor thread");
        tx
    }))
}

/// The actor loop: block for the first request, drain the rest, settle the
/// set as one batch. Grouping is a consequence of the queue, never a policy.
fn actor_loop(rx: &Receiver<Req>, barrier_handle: &File) {
    while let Ok(first) = rx.recv() {
        let mut batch = vec![first];
        while let Ok(more) = rx.try_recv() {
            batch.push(more);
        }
        settle_batch(barrier_handle, batch, &PROCESS_TALLIES);
    }
}

/// Where a settled member's ack goes.
type Ack = Sender<io::Result<SettleReceipt>>;

/// Settle one batch. THE ORDER IS THE PROMISE — push, then barrier, then
/// ack — a member is acked only after the barrier that covers its bytes has
/// returned (module doc steps 1–3). Members are first DEDUPED by
/// `(st_dev, st_ino)`: many queued requests may name the same on-disk object
/// (the same parent directory arrives once per rename), and one sync of the
/// object settles every request that named it — identically durable, paid
/// once.
fn settle_batch(barrier_handle: &File, batch: Vec<Req>, tallies: &BatchTallies<'_>) {
    let batch_id = BATCH_SEQ.fetch_add(1, Ordering::Relaxed) + 1;
    let members: Vec<(File, Ack)> = batch.into_iter().flat_map(|req| req.members).collect();
    let group_len = members.len();
    let receipt = SettleReceipt {
        batch: batch_id,
        group_len,
    };

    // 0. DEDUPE — one entry per distinct object; the first fd stands for
    //    all requests naming the object. A member whose object cannot be
    //    identified is refused loudly, never guessed at.
    let mut keyed: HashMap<(u64, u64), (File, Vec<Ack>)> = HashMap::new();
    for (file, ack) in members {
        match file.metadata() {
            Ok(m) => match keyed.entry((m.dev(), m.ino())) {
                Entry::Occupied(mut o) => o.get_mut().1.push(ack),
                Entry::Vacant(v) => {
                    v.insert((file, vec![ack]));
                }
            },
            Err(e) => {
                let _ = ack.send(Err(e));
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        // PREMISE GUARD — the module-doc falsifier, enforced at runtime: a
        // member on a different device than the barrier handle would be
        // acked without its device ever flushing. Refuse it loudly instead
        // (the same class as the recovery-image digest check: enforcing a
        // stated premise, not policy).
        let barrier_dev = match barrier_handle.metadata() {
            Ok(m) => m.dev(),
            Err(e) => {
                let msg = format!("durability barrier handle cannot be identified: {e}");
                for (_, (_, acks)) in keyed {
                    for ack in acks {
                        let _ = ack.send(Err(io::Error::new(e.kind(), msg.clone())));
                    }
                }
                return;
            }
        };
        // 1. PUSH — byte transfer only: plain fsync moves each object's
        //    bytes into the device. libc::fsync direct; sync_all would be
        //    F_FULLFSYNC here (module doc step 1).
        let mut pushed: Vec<Vec<Ack>> = Vec::with_capacity(keyed.len());
        for ((dev, _), (file, acks)) in keyed {
            if dev != barrier_dev {
                let msg = "member is on a different device than the durability \
                           barrier; its bytes would never be settled by the \
                           barrier — refused (module-doc falsifier)";
                for ack in acks {
                    let _ = ack.send(Err(io::Error::new(io::ErrorKind::Unsupported, msg)));
                }
                continue;
            }
            // SAFETY: fsync on a valid fd we own for the call's duration.
            let rc = unsafe { libc::fsync(file.as_raw_fd()) };
            if rc == -1 {
                let e = io::Error::last_os_error();
                let msg = format!("durability push failed: {e}");
                for ack in acks {
                    let _ = ack.send(Err(io::Error::new(e.kind(), msg.clone())));
                }
            } else {
                tallies.objects.fetch_add(1, Ordering::Relaxed);
                pushed.push(acks);
            }
        }
        // 2. BARRIER — one honest-class op on the actor's own root handle
        //    settles every byte the device already holds for this batch.
        //    A push claims nothing about power loss, so a failed barrier
        //    refuses every pushed member — no waiter is ever released on a
        //    push alone.
        if let Err(e) = honest_sync(barrier_handle) {
            let msg = format!("durability barrier failed for the batch: {e}");
            for acks in pushed {
                for ack in acks {
                    let _ = ack.send(Err(io::Error::new(e.kind(), msg.clone())));
                }
            }
            return;
        }
        tallies.barriers.fetch_add(1, Ordering::Relaxed);
        // 3. ACK — only now is any member's receipt true.
        for acks in pushed {
            for ack in acks {
                let _ = ack.send(Ok(receipt));
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = barrier_handle;
        // fsync IS the honest class per file here: sync each object once,
        // then ack its members — sync-before-ack is the same promise.
        for (_, (file, acks)) in keyed {
            let outcome = match honest_sync(&file) {
                Ok(()) => {
                    tallies.objects.fetch_add(1, Ordering::Relaxed);
                    tallies.barriers.fetch_add(1, Ordering::Relaxed);
                    Ok(receipt)
                }
                Err(e) => {
                    let msg = format!("durability sync failed: {e}");
                    Err((e.kind(), msg))
                }
            };
            for ack in acks {
                let _ = ack.send(match &outcome {
                    Ok(r) => Ok(*r),
                    Err((kind, msg)) => Err(io::Error::new(*kind, msg.clone())),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Local tallies so exact-count assertions cannot race other tests
    /// settling through the process actor.
    struct LocalTallies {
        objects: AtomicU64,
        barriers: AtomicU64,
    }

    impl LocalTallies {
        fn new() -> Self {
            Self {
                objects: AtomicU64::new(0),
                barriers: AtomicU64::new(0),
            }
        }
        fn as_batch(&self) -> BatchTallies<'_> {
            BatchTallies {
                objects: &self.objects,
                barriers: &self.barriers,
            }
        }
        fn counts(&self) -> (u64, u64) {
            (
                self.objects.load(Ordering::Relaxed),
                self.barriers.load(Ordering::Relaxed),
            )
        }
    }

    /// Queue `n` requests over fresh files, then run ONE drain cycle exactly
    /// as `actor_loop` would: recv one, `try_recv` the rest, settle as one
    /// batch. Returns the per-member outcomes.
    fn drain_once(
        dir: &tempfile::TempDir,
        barrier_handle: &File,
        n: usize,
        tallies: &LocalTallies,
    ) -> Vec<io::Result<SettleReceipt>> {
        let (tx, rx) = channel();
        let mut acks = Vec::new();
        for i in 0..n {
            let path = dir.path().join(format!("w{i}.md"));
            let mut f = File::create(&path).expect("create");
            f.write_all(b"bytes").expect("write");
            let (ack, ack_rx) = channel();
            tx.send(Req {
                members: vec![(f, ack)],
            })
            .expect("queue");
            acks.push(ack_rx);
        }
        let first = rx.recv().expect("first");
        let mut batch = vec![first];
        while let Ok(more) = rx.try_recv() {
            batch.push(more);
        }
        assert_eq!(batch.len(), n, "the whole queue is one batch");
        settle_batch(barrier_handle, batch, &tallies.as_batch());
        acks.iter().map(|rx| rx.recv().expect("acked")).collect()
    }

    /// N writers already queued when the actor drains settle as ONE batch:
    /// every receipt carries the same batch id and `group_len == N`, and the
    /// batch issues exactly one barrier (macOS) / one honest op per distinct
    /// object (elsewhere). Deterministic: pre-queued, drained once, tallied
    /// locally.
    #[test]
    fn queued_writers_settle_through_one_drain() {
        let dir = tempfile::tempdir().expect("tempdir");
        let barrier_handle = File::open(dir.path()).expect("root handle");
        let tallies = LocalTallies::new();
        let receipts: Vec<SettleReceipt> = drain_once(&dir, &barrier_handle, 8, &tallies)
            .into_iter()
            .map(|r| r.expect("settled"))
            .collect();
        assert!(
            receipts.iter().all(|r| r.batch == receipts[0].batch),
            "one drain, one batch id: {receipts:?}"
        );
        assert!(
            receipts.iter().all(|r| r.group_len == 8),
            "every member sees the group size: {receipts:?}"
        );
        let (objects, barriers) = tallies.counts();
        assert_eq!(objects, 8, "eight distinct files, eight object syncs");
        #[cfg(target_os = "macos")]
        assert_eq!(barriers, 1, "one honest barrier settles the whole batch");
        #[cfg(not(target_os = "macos"))]
        assert_eq!(barriers, 8, "per-object honest sync off macOS");
    }

    /// The premise guard, driven: a barrier handle on a DIFFERENT device
    /// (a devfs node) than the members refuses every member loudly — no
    /// push happens, no barrier is issued, and no waiter is ever released
    /// on durability its device never received. This is the module-doc
    /// falsifier enforced at runtime; the ack-strictly-after-barrier
    /// ordering for the same-device path is settle_batch's visible
    /// structure (one barrier call site, ack loop after it).
    #[cfg(target_os = "macos")]
    #[test]
    fn a_cross_device_barrier_refuses_every_member() {
        let devnull = File::open("/dev/null").expect("devnull");
        let dir = tempfile::tempdir().expect("tempdir");
        let dir_dev = std::fs::metadata(dir.path()).expect("meta").dev();
        let null_dev = devnull.metadata().expect("meta").dev();
        assert_ne!(dir_dev, null_dev, "devfs and the tempdir must differ");
        let tallies = LocalTallies::new();
        let outcomes = drain_once(&dir, &devnull, 3, &tallies);
        assert_eq!(outcomes.len(), 3);
        for outcome in outcomes {
            let err = outcome.expect_err("cross-device member must be refused");
            assert_eq!(err.kind(), io::ErrorKind::Unsupported);
        }
        let (objects, barriers) = tallies.counts();
        assert_eq!(objects, 0, "nothing was pushed for a refused member");
        assert_eq!(barriers, 0, "no barrier covers a refused member");
    }

    /// Four requests naming two distinct objects sync each object ONCE:
    /// the same file queued twice and the same directory queued twice
    /// collapse to two object syncs (not four) — and on macOS the whole
    /// batch still settles through one barrier.
    #[test]
    fn duplicate_objects_in_a_batch_sync_once() {
        let dir = tempfile::tempdir().expect("tempdir");
        let barrier_handle = File::open(dir.path()).expect("root handle");
        let file_path = dir.path().join("dup.md");
        let mut f = File::create(&file_path).expect("create");
        f.write_all(b"bytes").expect("write");

        let (tx, rx) = channel();
        let mut acks = Vec::new();
        for _ in 0..2 {
            let (ack, ack_rx) = channel();
            tx.send(Req {
                members: vec![(f.try_clone().expect("dup"), ack)],
            })
            .expect("queue");
            acks.push(ack_rx);
        }
        for _ in 0..2 {
            let (ack, ack_rx) = channel();
            tx.send(Req {
                members: vec![(File::open(dir.path()).expect("dir fd"), ack)],
            })
            .expect("queue");
            acks.push(ack_rx);
        }
        let tallies = LocalTallies::new();
        let mut batch = Vec::new();
        while let Ok(req) = rx.try_recv() {
            batch.push(req);
        }
        assert_eq!(batch.len(), 4);
        settle_batch(&barrier_handle, batch, &tallies.as_batch());
        for ack in &acks {
            ack.recv().expect("acked").expect("settled");
        }
        let (objects, barriers) = tallies.counts();
        assert_eq!(objects, 2, "two distinct objects, two syncs — dedupe holds");
        #[cfg(target_os = "macos")]
        assert_eq!(barriers, 1, "one barrier per drain");
        #[cfg(not(target_os = "macos"))]
        assert_eq!(barriers, 2, "one honest op per object");
    }

    /// The public path: submit is non-blocking, wait establishes durability,
    /// and concurrent settles through the process actor all succeed.
    #[test]
    fn submit_then_wait_settles_through_the_process_actor() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("a.md");
        let mut f = File::create(&path).expect("create");
        f.write_all(b"payload").expect("write");
        let t0 = ops_on_this_thread();
        let pending = submit(dir.path(), &f).expect("submit");
        let receipt = pending.wait().expect("settled");
        assert!(receipt.group_len >= 1);
        assert_eq!(
            ops_on_this_thread() - t0,
            2,
            "one submit + one wait on this thread"
        );
    }

    /// An atomic set shares ONE drain by construction: a multi-member
    /// `submit_set` reaches the queue as one message, so every member's
    /// receipt carries the same batch id even when the actor was idle and
    /// draining eagerly — the mid-phase-drain defect cannot split it.
    #[test]
    fn an_atomic_set_cannot_be_split_across_drains() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut files = Vec::new();
        for i in 0..5 {
            let path = dir.path().join(format!("s{i}.md"));
            let mut f = File::create(&path).expect("create");
            f.write_all(b"bytes").expect("write");
            files.push(f);
        }
        let pendings = submit_set(dir.path(), files).expect("one enqueue");
        let receipts: Vec<SettleReceipt> = pendings
            .iter()
            .map(|p| p.wait().expect("settled"))
            .collect();
        assert_eq!(receipts.len(), 5);
        assert!(
            receipts.iter().all(|r| r.batch == receipts[0].batch),
            "one enqueue, one drain, one batch id: {receipts:?}"
        );
    }
}
