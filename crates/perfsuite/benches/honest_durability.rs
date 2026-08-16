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
    // Destination LAYOUT changes the count: each distinct parent directory
    // costs its own barrier after the renames. Separate directories is the
    // worst case and the default here; HD_SAME_DIR puts every destination in
    // one directory, which is what a multi-file edit in one folder looks like.
    let same_dir = std::env::var("HD_SAME_DIR").is_ok();
    let rels: Vec<String> = (0..dests)
        .map(|i| {
            if same_dir {
                format!("d0/f{i}.md")
            } else {
                format!("d{i}/f{i}.md")
            }
        })
        .collect();

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
    content_fsync: Duration,
    metadata_publish: Duration,
    group_wait: Duration,
    cohort_size: usize,
    wall: Duration,
}

/// One writer's transaction. The four durability figures are read straight
/// off the receipt — never re-derived here, and `group_wait` is never folded
/// into either durability figure (a writer that waited for a cohort did not
/// spend that time on durability).
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
    Sample {
        content_fsync: rec.content_fsync,
        metadata_publish: rec.metadata_publish,
        group_wait: rec.group_wait,
        cohort_size: rec.cohort_size,
        wall,
    }
}

/// One arm's measured sample set. Percentile vectors are already sorted.
struct ArmStats {
    fsync_ms: Vec<f64>,
    meta_ms: Vec<f64>,
    wait_ms: Vec<f64>,
    wall_ms: Vec<f64>,
    burst_ms: f64,
    peak_cohort: usize,
}

impl ArmStats {
    fn txn_per_s(&self) -> f64 {
        self.wall_ms.len() as f64 / (self.burst_ms / 1e3)
    }
}

/// N concurrent writers over disjoint destinations, released together.
/// Whichever arm `fs::set_durability_coordination` has in force is the arm
/// measured — this function never sets it, so a caller pairing two arms
/// controls the order.
fn run_arm(n: usize, iters: usize) -> ArmStats {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = WorkspaceRoot(dir.path().to_path_buf());
    let table = ReservationTable::new();
    let owner = Arc::new(StateOwner::new(Root("tip".to_owned())));
    let payload = vec![b'x'; 4096];

    let arrived = Arc::new(AtomicUsize::new(0));
    let go = Arc::new(std::sync::atomic::AtomicBool::new(false));

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
    let started = Instant::now();
    go.store(true, Ordering::SeqCst);

    let mut st = ArmStats {
        fsync_ms: Vec::new(),
        meta_ms: Vec::new(),
        wait_ms: Vec::new(),
        wall_ms: Vec::new(),
        burst_ms: 0.0,
        peak_cohort: 0,
    };
    for h in handles {
        for s in h.join().expect("writer thread") {
            st.fsync_ms.push(s.content_fsync.as_secs_f64() * 1e3);
            st.meta_ms.push(s.metadata_publish.as_secs_f64() * 1e3);
            st.wait_ms.push(s.group_wait.as_secs_f64() * 1e3);
            st.wall_ms.push(s.wall.as_secs_f64() * 1e3);
            st.peak_cohort = st.peak_cohort.max(s.cohort_size);
        }
    }
    st.burst_ms = started.elapsed().as_secs_f64() * 1e3;
    sort_ms(&mut st.fsync_ms);
    sort_ms(&mut st.meta_ms);
    sort_ms(&mut st.wait_ms);
    sort_ms(&mut st.wall_ms);
    st
}

fn mode_sweep(ns: &[usize], iters: usize) {
    // HD_COORD=0 runs the control arm, so a count sweep can be taken on either
    // arm and the difference attributed to grouping rather than assumed.
    if std::env::var("HD_COORD").as_deref() == Ok("0") {
        fs::set_durability_coordination(false);
    }
    let arm = if fs::durability_coordination() {
        "ON"
    } else {
        "OFF"
    };
    println!(
        "sweep: arm={arm} iters_per_writer={iters} payload_bytes=4096 \
         volume=APFS-internal-NVMe instrument=receipt-fields"
    );
    for &n in ns {
        let st = run_arm(n, iters);
        println!(
            "arm={arm} N={n} txns={} burst_wall_ms={:.2} txn_per_s={:.1} | \
             fsync_p50={:.3} fsync_p99={:.3} | meta_p50={:.3} meta_p99={:.3} | \
             wait_p50={:.3} wait_p99={:.3} | txn_p50={:.3} txn_p99={:.3} | \
             cohort_peak={}",
            st.wall_ms.len(),
            st.burst_ms,
            st.txn_per_s(),
            pct(&st.fsync_ms, 500),
            pct(&st.fsync_ms, 990),
            pct(&st.meta_ms, 500),
            pct(&st.meta_ms, 990),
            pct(&st.wait_ms, 500),
            pct(&st.wait_ms, 990),
            pct(&st.wall_ms, 500),
            pct(&st.wall_ms, 990),
            st.peak_cohort,
        );
    }
}

/// Config L2 — BOUND COST, the A/B control, PAIRED.
///
/// Both arms run inside one repetition under shared machine conditions, and
/// the order alternates per repetition so neither arm holds a favoured
/// position. This host is not quiescent (load average 46–110 observed), so
/// running one arm now and the other later would measure sampling luck and
/// could report it as an architecture win — a false positive in our own
/// favour, which is worse than a disproof.
///
/// The reported figure is **per-writer end-to-end** p50/p99 and the per-pair
/// ratio. Aggregate throughput is printed for context only: it would hide
/// exactly the late-arriving writer this config exists to find.
fn mode_ab(ns: &[usize], iters: usize, reps: usize) {
    println!("L2 bound cost — paired A/B, iters_per_writer={iters} reps={reps} payload_bytes=4096");
    println!(
        "rep\tN\torder\ton_p50\ton_p99\toff_p50\toff_p99\tratio_p50\tratio_p99\t\
         on_wait_p50\ton_cohort\ton_txn_s\toff_txn_s"
    );
    for &n in ns {
        for rep in 1..=reps {
            let on_first = rep % 2 == 1;
            let (on, off) = if on_first {
                fs::set_durability_coordination(true);
                let a = run_arm(n, iters);
                fs::set_durability_coordination(false);
                let b = run_arm(n, iters);
                (a, b)
            } else {
                fs::set_durability_coordination(false);
                let b = run_arm(n, iters);
                fs::set_durability_coordination(true);
                let a = run_arm(n, iters);
                (a, b)
            };
            let (on50, on99) = (pct(&on.wall_ms, 500), pct(&on.wall_ms, 990));
            let (off50, off99) = (pct(&off.wall_ms, 500), pct(&off.wall_ms, 990));
            println!(
                "{rep}\t{n}\t{}\t{on50:.2}\t{on99:.2}\t{off50:.2}\t{off99:.2}\t\
                 {:.3}\t{:.3}\t{:.2}\t{}\t{:.1}\t{:.1}",
                if on_first { "on-first" } else { "off-first" },
                on50 / off50,
                on99 / off99,
                pct(&on.wait_ms, 500),
                on.peak_cohort,
                on.txn_per_s(),
                off.txn_per_s(),
            );
        }
    }
    fs::set_durability_coordination(true);
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
        Some("ab") => {
            let ns: Vec<usize> = args.get(1).map_or_else(
                || vec![60],
                |s| s.split(',').filter_map(|x| x.trim().parse().ok()).collect(),
            );
            let iters = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(10);
            let reps = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(6);
            mode_ab(&ns, iters, reps);
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
