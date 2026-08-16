//! `honest-durability` — the measurement half of the Team A durability card.
//!
//! Answers two questions no source reading can answer:
//!
//! - **How many honest syncs does one real `publish()` issue, and on what?**
//!   `count` mode drives one 2-destination transaction with markers written
//!   into the external trace log, so the syncs belonging to `publish()` are
//!   delimited from the ones belonging to staging.
//! - **What do they cost, at one writer and under a burst?** `sweep` mode runs
//!   N concurrent writers over disjoint destinations and reports fsync latency
//!   and metadata publish latency as SEPARATE figures (gate 12 forbids any
//!   aggregate being called the tree cost).
//!
//! The external instrument is `.scratch/fffspy.dylib`, a DYLD interposer over
//! `fcntl`/`fsync` — SIP blocks dtrace's syscall provider on this host. The
//! interposer is qualified against `.scratch/fsprobe` before it is trusted.
//! This bench does not require it: without `FFFSPY_LOG` set, the markers are
//! skipped and the timing figures are unaffected.
//!
//! Hand-written `main`, `harness = false`, following `cold-ingest`: this is a
//! single timed pass over a real disk transaction, not a criterion micro-loop.
//! `F_FULLFSYNC` is macOS-only and the ruling baseline is a mac number, so
//! these figures come from a mac; CI (Linux) is the compile gate only.
//!
//!   cargo bench -p perfsuite --bench honest-durability -- count
//!   cargo bench -p perfsuite --bench honest-durability -- sweep 1,2,4,8,16,60

#![allow(
    clippy::cast_precision_loss,
    reason = "nanosecond counts to millisecond floats; perf reporting, not arithmetic"
)]

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use fs::WorkspaceRoot;
use fs::intent::Policy;
use wire::Root;
use wire_serve::publish::{
    DestImage, PublishReceipt, Region, RegionKind, ReservationTable, StagedTxn, StateOwner, TxnId,
    publish,
};

/// Append a delimiter into the external trace log so the syncs issued by
/// `publish()` can be told apart from the ones issued while staging. A no-op
/// when the interposer is not loaded.
fn mark(tag: &str) {
    let Ok(path) = std::env::var("FFFSPY_LOG") else {
        return;
    };
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "MARK {tag}");
    }
}

/// Stage one destination: write the tmp beside its target and sync it, which
/// is what a real caller has already done before it reaches `publish()`.
fn stage_dest(root: &WorkspaceRoot, rel: &str, old: &[u8], new: &[u8]) -> DestImage {
    let dest = root.0.join(rel);
    if let Some(p) = dest.parent() {
        std::fs::create_dir_all(p).expect("dest parent");
    }
    std::fs::write(&dest, old).expect("seed old");
    let tmp = dest.with_extension("tmp-pub");
    std::fs::write(&tmp, new).expect("stage tmp");
    DestImage {
        rel: PathBuf::from(rel),
        old: old.to_vec(),
        new: new.to_vec(),
        tmp,
    }
}

fn staged_txn(root: &WorkspaceRoot, rels: &[String], payload: &[u8]) -> StagedTxn {
    let dests = rels
        .iter()
        .map(|rel| stage_dest(root, rel, b"old", payload))
        .collect();
    let writes = rels
        .iter()
        .map(|rel| Region::file("c", rel.as_bytes(), RegionKind::Write))
        .collect();
    StagedTxn {
        txn: TxnId::mint().expect("txn id"),
        epoch: wire_serve::authority::AuthorityEpoch(1),
        reads: Vec::new(),
        writes,
        dests,
        files: Vec::new(),
        actor: None,
        now: None,
        policy: Policy::Restore,
        drop_tmp_after_decide: false,
    }
}

/// Percentile by integer permille (500 = p50, 990 = p99). Integer math on
/// purpose: a float index would need a truncating cast back to `usize`.
fn pct(sorted: &[f64], permille: usize) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let n = sorted.len();
    let idx = (permille * (n - 1) + 500) / 1000;
    sorted[idx.min(n - 1)]
}

fn sort_ms(v: &mut [f64]) {
    v.sort_by(|a, b| a.partial_cmp(b).expect("no NaN latencies"));
}

/// The per-transaction honest-sync COUNT — the headline figure of this card.
/// One 2-destination `publish()`, delimited in the trace by markers.
fn mode_count(dests: usize) {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = WorkspaceRoot(dir.path().to_path_buf());
    let table = ReservationTable::new();
    let owner = StateOwner::new(Root("tip".to_owned()));
    let rels: Vec<String> = (0..dests).map(|i| format!("d{i}/f{i}.md")).collect();

    mark("stage-begin");
    let txn = staged_txn(&root, &rels, b"new payload");
    mark("stage-end");

    mark("publish-begin");
    let t0 = Instant::now();
    let rec = publish(&root, &table, &owner, txn, |before| {
        Root(format!("{}+x", before.0))
    })
    .expect("publish");
    let elapsed = t0.elapsed();
    mark("publish-end");

    println!(
        "count-mode dests={dests} publish_wall_ms={:.3} disk_stage_ms={:.3} txn={}",
        elapsed.as_secs_f64() * 1e3,
        rec.disk_end.duration_since(rec.disk_start).as_secs_f64() * 1e3,
        rec.txn.hex()
    );
    println!("count-mode: honest syncs are counted from the trace log between the publish markers");
}

struct Sample {
    fsync: Duration,
    metadata: Duration,
    wall: Duration,
}

/// One writer's transaction, timed. `content_fsync` / `metadata_publish` are
/// read from the receipt when the implementation exposes them; until then the
/// receipt's disk-stage pair bounds the durable half and the remainder is
/// metadata publish.
fn one_txn(
    root: &WorkspaceRoot,
    table: &Arc<ReservationTable>,
    owner: &StateOwner,
    rel: &str,
    payload: &[u8],
) -> Sample {
    let rels = [rel.to_owned()];
    let txn = staged_txn(root, &rels, payload);
    let t0 = Instant::now();
    let rec: PublishReceipt = publish(root, table, owner, txn, |before| {
        Root(format!("{}+x", before.0))
    })
    .expect("publish");
    let wall = t0.elapsed();
    let disk = rec.disk_end.duration_since(rec.disk_start);
    Sample {
        fsync: disk,
        metadata: wall.saturating_sub(disk),
        wall,
    }
}

/// N concurrent writers over disjoint destinations, released together.
fn mode_sweep(ns: &[usize], iters: usize) {
    println!(
        "sweep: iters_per_writer={iters} payload_bytes=4096 volume=APFS-internal-NVMe \
         instrument=receipt-timers"
    );
    for &n in ns {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = WorkspaceRoot(dir.path().to_path_buf());
        let table = ReservationTable::new();
        let owner = Arc::new(StateOwner::new(Root("tip".to_owned())));
        let payload = vec![b'x'; 4096];

        let arrived = Arc::new(AtomicUsize::new(0));
        let go = Arc::new(std::sync::atomic::AtomicBool::new(false));

        let wall0 = Arc::new(std::sync::Mutex::new(None::<Instant>));
        let mut handles = Vec::with_capacity(n);
        for w in 0..n {
            let root = WorkspaceRoot(root.0.clone());
            let table = Arc::clone(&table);
            let owner = Arc::clone(&owner);
            let payload = payload.clone();
            let arrived = Arc::clone(&arrived);
            let go = Arc::clone(&go);
            handles.push(std::thread::spawn(move || {
                arrived.fetch_add(1, Ordering::SeqCst);
                while !go.load(Ordering::SeqCst) {
                    std::hint::spin_loop();
                }
                let mut out = Vec::with_capacity(iters);
                for i in 0..iters {
                    let rel = format!("w{w}/f{i}.md");
                    out.push(one_txn(&root, &table, &owner, &rel, &payload));
                }
                out
            }));
        }
        while arrived.load(Ordering::SeqCst) < n {
            std::hint::spin_loop();
        }
        *wall0.lock().expect("wall0") = Some(Instant::now());
        go.store(true, Ordering::SeqCst);

        let mut fsync_ms = Vec::new();
        let mut meta_ms = Vec::new();
        let mut wall_ms = Vec::new();
        for h in handles {
            for s in h.join().expect("writer thread") {
                fsync_ms.push(s.fsync.as_secs_f64() * 1e3);
                meta_ms.push(s.metadata.as_secs_f64() * 1e3);
                wall_ms.push(s.wall.as_secs_f64() * 1e3);
            }
        }
        let burst_ms = wall0
            .lock()
            .expect("wall0")
            .expect("start stamped")
            .elapsed()
            .as_secs_f64()
            * 1e3;

        sort_ms(&mut fsync_ms);
        sort_ms(&mut meta_ms);
        sort_ms(&mut wall_ms);
        let total = wall_ms.len();
        println!(
            "N={n} txns={total} burst_wall_ms={burst_ms:.2} txn_per_s={:.1} | \
             fsync_p50={:.3} fsync_p99={:.3} | meta_p50={:.3} meta_p99={:.3} | \
             txn_p50={:.3} txn_p99={:.3}",
            total as f64 / (burst_ms / 1e3),
            pct(&fsync_ms, 500),
            pct(&fsync_ms, 990),
            pct(&meta_ms, 500),
            pct(&meta_ms, 990),
            pct(&wall_ms, 500),
            pct(&wall_ms, 990),
        );
    }
}

fn main() {
    let args: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| a != "--bench")
        .collect();

    // An external tracer (fs_usage) cannot attach to a short-lived process it
    // did not see start — the fsprobe positive control produced zero lines
    // under that shape. This holds the process open long enough to be attached
    // to before any measured work runs. Unset in normal runs.
    if let Some(secs) = std::env::var("HD_ATTACH_DELAY_S")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
    {
        std::thread::sleep(Duration::from_secs(secs));
    }

    // Smoke lane (`cargo bench -q -p perfsuite --locked -- --test`), same
    // detection as `cold-ingest`. It runs on Linux, where the honest class is
    // `sync_all` and F_FULLFSYNC does not exist, so this path only proves the
    // lane links and drives one real transaction. It asserts nothing: no
    // macOS-only behaviour, no wall-time threshold. The card's numbers come
    // from an explicit mac invocation, never from CI.
    if args.iter().any(|a| a == "--test") {
        mode_count(1);
        mode_sweep(&[2], 1);
        return;
    }

    match args.first().map(String::as_str) {
        Some("count") => {
            let dests = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(2);
            mode_count(dests);
        }
        Some("sweep") => {
            let ns: Vec<usize> = args.get(1).map_or_else(
                || vec![1, 2, 4, 8, 16, 60],
                |s| s.split(',').filter_map(|x| x.trim().parse().ok()).collect(),
            );
            let iters = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(20);
            mode_sweep(&ns, iters);
        }
        _ => {
            eprintln!("usage: honest-durability count [dests] | sweep [n,n,...] [iters]");
            mode_count(1);
        }
    }
}
