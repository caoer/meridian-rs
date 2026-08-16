//! Honest-durability wall-time measurement over the REAL publish path — PERF
//! lane only, run by hand on macOS. Never the PR lane, and never CI.
//!
//! # Why this target is macOS-only and feature-gated
//!
//! The honest durability class ruled in merged-plan §4.7 is `F_FULLFSYNC`,
//! which exists only on macOS; everywhere else [`fs::honest_sync`] falls back
//! to `sync_all`. A Linux run would report a *different syscall's* latency
//! under the same heading — the exact defect this card corrects, reintroduced
//! by the measurement. `required-features` makes the separation mechanical:
//! `cargo test --workspace` skips the target, `.woodpecker.yaml`'s discovery
//! loop compiles and lints it so it cannot bit-rot, and nothing runs it but a
//! person on the hardware being reported.
//!
//! # What is measured
//!
//! [`wire_serve::publish::publish`] itself — the shipped function, not a model
//! of it. It reports three intervals and this file reports them as three
//! figures:
//!
//! | figure           | `PublishReceipt` field | what it covers                    |
//! |------------------|------------------------|-----------------------------------|
//! | content durable  | `content`              | staged bytes to the device, under NO exclusion |
//! | metadata publish | `publish`              | `O_EXCL` intent, renames, dirent barrier |
//! | owner step       | `owner_step`           | the linearized state-owner apply  |
//!
//! **The three are never summed, here or in `RESULTS.md`.** One interval
//! spanning both durability phases is the aggregate gate 12 forbids: it hides
//! which sync class was paid where, which is the whole question. The
//! `publish_call` figure below is measured independently around the call and
//! is NOT their sum — it also contains reserve, reverify and finalize, none of
//! which appear in any span.
//!
//! # Barrier count, which is the clearer statement of what this card changes
//!
//! Latency answers "how long"; the count answers "how many barriers did one
//! commit buy". Every transaction reports both, from two instruments that
//! cannot both be wrong in the same direction:
//!
//! - **inside** — [`fs::honest_sync_count_thread`] / [`fs::data_push_count_thread`]
//!   deltas across the call. Per-thread, so the figure stays exact while other
//!   writers publish concurrently.
//! - **outside** — the `DYLD_INSERT_LIBRARIES` interposer in
//!   `.scratch/instrument/`, which counts `fcntl(F_FULLFSYNC)`,
//!   `fcntl(F_BARRIERFSYNC)` and `fsync(2)` from outside the process.
//!
//! A disagreement between them is a finding, not a nuisance — report it.
//!
//! # Reading the output
//!
//! One `RESULT` line per (class, writer count) on stdout, all times in
//! nanoseconds, every field named. Percentiles are **exact nearest-rank** over
//! the pooled per-operation samples: sample `ceil(q*n)` of the sorted vector,
//! no interpolation and no histogram bucketing. Raw stdout is the evidence;
//! `RESULTS.md` may not carry a number this file did not print.
//!
//! # Running it
//!
//! ```text
//! cargo test -p wire-serve --features perf-walltime --test durability_walltime \
//!   -- --nocapture --test-threads=1
//! ```
//!
//! `MRD_DURABILITY_SAMPLES` (default 512) sets the pooled sample target per
//! configuration; `MRD_DURABILITY_ROOT` relocates the fixture tree, which must
//! stay on the filesystem whose durability is being reported.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use fs::epoch::{ANCHOR_REL, EpochBatcher, SealMode};
use fs::intent::Policy;
use fs::{WorkspaceRoot, honest_sync};
use wire::Root;
use wire_serve::authority::AuthorityEpoch;
use wire_serve::publish::{
    DestImage, Region, RegionKind, ReservationTable, StagedTxn, StateOwner, TxnId, publish,
};

/// Payload size, matching the ruling probe's 4 KiB writes so these numbers are
/// comparable to its baseline rows.
const PAYLOAD: usize = 4096;

/// Discarded leading transactions per writer — first-touch directory creation
/// and allocator warmup are not what is being measured.
const WARMUP: usize = 4;

// Two libc symbols, declared here rather than pulled in as a `libc`
// dev-dependency because they are two symbols used in two places.
//
// `fsync` is the raw cheap-class control. It exists because on macOS Rust's
// `File::sync_all` AND `File::sync_data` both issue `fcntl(F_FULLFSYNC)` —
// verified on this host by the interposer and independently by `fs_usage`. So
// `sync_all` is NOT the cheap class here, and a cheap row measured with it
// would report the honest class twice under two names.
//
// `getloadavg` is how every table carries the conditions it was taken under.
// This host runs a multi-agent fleet and is never quiescent; a number without
// its load average is not reproducible, and eyeballing `uptime` afterwards
// records the wrong moment.
unsafe extern "C" {
    fn fsync(fd: i32) -> i32;
    fn getloadavg(loadavg: *mut f64, nelem: i32) -> i32;
}

/// The 1/5/15-minute load averages, or zeros if the kernel refuses them.
fn loadavg() -> [f64; 3] {
    let mut out = [0.0f64; 3];
    // SAFETY: `out` is a live array of exactly 3 f64, which is what we declare.
    let rc = unsafe { getloadavg(out.as_mut_ptr(), 3) };
    if rc == 3 { out } else { [0.0; 3] }
}

/// Emit the machine conditions under `tag`. Called at the start and end of
/// every configuration so a table's drift is visible rather than inferred.
fn report_load(tag: &str) {
    let l = loadavg();
    println!(
        "LOAD at={tag} load1={:.2} load5={:.2} load15={:.2}",
        l[0], l[1], l[2]
    );
}

/// Elapsed nanoseconds since `t`, saturating. A run long enough to overflow
/// u64 nanoseconds (584 years) is not a measurement.
fn ns_since(t: Instant) -> u64 {
    u64::try_from(t.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

fn ns_of(d: Duration) -> u64 {
    u64::try_from(d.as_nanos()).unwrap_or(u64::MAX)
}

/// Exact nearest-rank percentile: sample `ceil(q * n)` of a sorted slice,
/// 1-indexed, no interpolation. `sorted` must be non-empty.
fn nearest_rank(sorted: &[u64], numerator: usize, denominator: usize) -> u64 {
    let n = sorted.len();
    let rank = (n * numerator).div_ceil(denominator).clamp(1, n);
    sorted[rank - 1]
}

/// Print one machine-readable line. Times stay integers all the way to stdout —
/// nanosecond counts converted to float lose exactness for no gain, and every
/// downstream number must trace back to what this printed.
fn report(class: &str, writers: usize, iters: usize, samples: &mut [u64]) {
    assert!(!samples.is_empty(), "{class}: no samples");
    samples.sort_unstable();
    let n = samples.len();
    let total: u128 = samples.iter().map(|&v| u128::from(v)).sum();
    let mean = total / n as u128;
    println!(
        "RESULT class={class} writers={writers} iters_per_writer={iters} samples={n} \
         p50_ns={} p90_ns={} p99_ns={} min_ns={} max_ns={} mean_ns={mean}",
        nearest_rank(samples, 50, 100),
        nearest_rank(samples, 90, 100),
        nearest_rank(samples, 99, 100),
        samples[0],
        samples[n - 1],
    );
}

/// The fixture tree. Kept under the worktree so it lands on the same
/// filesystem as the corpus whose durability is reported, and so the raw
/// artifacts survive the run.
fn fixture_root() -> PathBuf {
    if let Ok(p) = std::env::var("MRD_DURABILITY_ROOT") {
        return PathBuf::from(p);
    }
    // CARGO_MANIFEST_DIR is crates/wire-serve; the worktree root is two up.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(".scratch/durability-fixtures")
}

fn pooled_target() -> usize {
    std::env::var("MRD_DURABILITY_SAMPLES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(512)
}

/// What one publish call cost, in the four figures that are never summed plus
/// the two barrier counts.
struct Measured {
    content_ns: u64,
    publish_ns: u64,
    owner_ns: u64,
    call_ns: u64,
    honest_syncs: u64,
    data_pushes: u64,
}

/// Stage `dests` destinations and publish them through the real path in one
/// transaction.
///
/// Staging is deliberately outside the timed region and issues no sync of its
/// own: `publish` phase 1 is what makes the staged bytes durable, and syncing
/// at staging would warm the device and understate the phase this card is
/// about.
/// The live engine pieces one publish call needs, bundled so the measurement
/// helpers pass a plane rather than a parameter list.
struct Plane<'a> {
    root: &'a WorkspaceRoot,
    table: &'a Arc<ReservationTable>,
    owner: &'a StateOwner,
    batcher: &'a EpochBatcher,
}

fn one_txn(plane: &Plane<'_>, writer: usize, seq: usize, dests: usize, payload: &[u8]) -> Measured {
    one_txn_layout(plane, writer, seq, dests, false, payload)
}

/// As [`one_txn`], with the corpus layout as a knob. `spread` puts every
/// destination in its own directory instead of sharing one.
///
/// The layout is a knob because the cheap-push count depends on it: phase 2
/// seals destination PARENT DIRECTORIES, and `publish::barrier` dedupes exact
/// duplicates, so D destinations sharing one directory contribute one push
/// where D destinations in D directories contribute D. A count reported without
/// its layout is not reproducible.
fn one_txn_layout(
    plane: &Plane<'_>,
    writer: usize,
    seq: usize,
    dests: usize,
    spread: bool,
    payload: &[u8],
) -> Measured {
    let root = plane.root;
    let mut images = Vec::with_capacity(dests);
    let mut regions = Vec::with_capacity(dests);
    for d in 0..dests {
        let rel = PathBuf::from(if spread {
            format!("w{writer}/d{d}/dest-{seq}-{d}.md")
        } else {
            format!("w{writer}/dest-{seq}-{d}.md")
        });
        let dest = root.0.join(&rel);
        let parent = dest.parent().expect("dest has a parent");
        std::fs::create_dir_all(parent).expect("create dest parent");

        let tmp = dest.with_extension("md.tmp-pub");
        {
            let mut f = File::create(&tmp).expect("create staged temp");
            f.write_all(payload).expect("write staged temp");
        }

        // Disjoint write region per destination: the reservation table must not
        // serialize writers that touch different files, and measuring it as if
        // it did would report a queue this design does not have.
        regions.push(Region::file(
            "c",
            rel.to_string_lossy().as_bytes(),
            RegionKind::Write,
        ));
        images.push(DestImage {
            rel,
            old: Vec::new(),
            new: payload.to_vec(),
            tmp,
        });
    }

    let txn = StagedTxn {
        txn: TxnId::mint().expect("mint txn id"),
        epoch: AuthorityEpoch(1),
        reads: Vec::new(),
        writes: regions,
        dests: images,
        files: Vec::new(),
        actor: None,
        now: None,
        policy: Policy::Restore,
        drop_tmp_after_decide: false,
    };

    let h0 = fs::honest_sync_count_thread();
    let d0 = fs::data_push_count_thread();
    let t = Instant::now();
    let rec = publish(
        plane.root,
        plane.table,
        plane.owner,
        plane.batcher,
        txn,
        |before| Root(format!("{}+{writer}.{seq}", before.0)),
    )
    .expect("publish");
    let call_ns = ns_since(t);

    Measured {
        content_ns: ns_of(rec.content.duration()),
        publish_ns: ns_of(rec.publish.duration()),
        owner_ns: ns_of(rec.owner_step.duration()),
        call_ns,
        honest_syncs: fs::honest_sync_count_thread() - h0,
        data_pushes: fs::data_push_count_thread() - d0,
    }
}

/// How the epoch batcher is shaped for a round. `window` is the risk this team
/// was assigned to prove or disprove: a lone writer must wait it out.
#[derive(Clone, Copy)]
struct EpochCfg {
    window: Duration,
    mode: SealMode,
    max_batch: usize,
}

impl Default for EpochCfg {
    fn default() -> Self {
        Self {
            window: Duration::ZERO,
            mode: SealMode::AnchorBarrier,
            max_batch: usize::MAX,
        }
    }
}

/// One round of one configuration: `writers` threads against a fresh table and
/// owner, each doing `WARMUP + iters` transactions. Returns the kept samples
/// and the round's wall time.
fn run_round(
    root: &WorkspaceRoot,
    writers: usize,
    iters: usize,
    dests: usize,
    cfg: EpochCfg,
) -> (Vec<Measured>, u64) {
    let payload = vec![b'm'; PAYLOAD];
    let table = ReservationTable::new();
    let owner = StateOwner::new(Root("r0".to_owned()));
    let batcher = EpochBatcher::new(&root.0.join(ANCHOR_REL), cfg.window, cfg.max_batch)
        .expect("epoch batcher")
        .with_mode(cfg.mode);

    let wall = Instant::now();
    let collected: Vec<Vec<Measured>> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..writers)
            .map(|w| {
                let payload = &payload;
                let table = &table;
                let owner = &owner;
                let batcher = &batcher;
                s.spawn(move || {
                    let mut out = Vec::with_capacity(iters);
                    for i in 0..(WARMUP + iters) {
                        let plane = Plane {
                            root,
                            table,
                            owner,
                            batcher,
                        };
                        let m = one_txn(&plane, w, i, dests, payload);
                        if i >= WARMUP {
                            out.push(m);
                        }
                    }
                    out
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("writer thread"))
            .collect()
    });
    (collected.into_iter().flatten().collect(), ns_since(wall))
}

/// Report every figure for one configuration over its pooled samples.
fn report_config(writers: usize, iters: usize, samples: &[Measured], wall_ns: u64) {
    for (class, pick) in [
        ("content_durable", 0usize),
        ("metadata_publish", 1),
        ("owner_step", 2),
        ("publish_call", 3),
    ] {
        let mut v: Vec<u64> = samples
            .iter()
            .map(|m| match pick {
                0 => m.content_ns,
                1 => m.publish_ns,
                2 => m.owner_ns,
                _ => m.call_ns,
            })
            .collect();
        report(class, writers, iters, &mut v);
    }

    // Barrier count per commit — the cleanest statement of what this card
    // changes. Reported as min/max as well as total, because a mean would hide
    // a commit that paid a different number than its neighbours.
    let honest: Vec<u64> = samples.iter().map(|m| m.honest_syncs).collect();
    let pushes: Vec<u64> = samples.iter().map(|m| m.data_pushes).collect();
    let txns = samples.len();
    println!(
        "COUNT writers={writers} txns={txns} \
         honest_syncs_total={} honest_per_commit_min={} honest_per_commit_max={} \
         data_pushes_total={} pushes_per_commit_min={} pushes_per_commit_max={}",
        honest.iter().sum::<u64>(),
        honest.iter().min().copied().unwrap_or(0),
        honest.iter().max().copied().unwrap_or(0),
        pushes.iter().sum::<u64>(),
        pushes.iter().min().copied().unwrap_or(0),
        pushes.iter().max().copied().unwrap_or(0),
    );
    println!(
        "WALL writers={writers} iters_per_writer={iters} txns={txns} \
         wall_ns={wall_ns} ns_per_txn={}",
        wall_ns / txns as u64,
    );
}

/// The two sync classes, compared by **paired interleaved sampling**: both
/// measured inside one iteration on their own fds, order alternating so
/// neither always goes first.
///
/// Pairing is not fussiness — it is what makes the comparison mean anything on
/// this host. Measured unpaired, raw `fsync` p50 came back at 4.78 ms, 4.83 ms
/// and 0.074 ms from three runs of the same binary seconds apart — a 65x spread
/// — while `F_FULLFSYNC` drifted 5.0 → 15.5 ms in the same window and the load
/// average went *down*. Sampling class A at one moment and class B at another
/// therefore measures when you sampled, not which syscall you issued. Paired,
/// every pair shares machine conditions and the per-pair ratio is meaningful
/// even when the absolute level is not: measured stable to within 20% across
/// four runs (`.scratch/instrument/sync-class-paired.txt`).
///
/// The cheap side uses raw `fsync(2)`, NOT `File::sync_all`: on macOS
/// `sync_all` and `sync_data` both issue `fcntl(F_FULLFSYNC)` — verified here
/// by the interposer and independently by `fs_usage`. A cheap row measured with
/// `sync_all` would report the honest class twice under two names, and the
/// assertion below is the guard against that reappearing.
///
/// Dirty-page discipline: one fresh `PAYLOAD`-byte write to that class's own fd
/// immediately before every measured call, never two syncs on a clean fd. A
/// sync over no dirty pages returns having done nothing (~700 ns measured here)
/// and would turn the cheap row into a comparison against a return statement.
#[test]
fn the_two_sync_classes_are_measured_apart() {
    let dir = fixture_root().join("classes");
    std::fs::create_dir_all(&dir).expect("fixture dir");
    let iters = pooled_target();
    let payload = vec![b'm'; PAYLOAD];

    report_load("sync_class.start");
    let mut honest_f = File::create(dir.join("paired-honest.dat")).expect("honest fixture");
    let mut cheap_f = File::create(dir.join("paired-cheap.dat")).expect("cheap fixture");

    let mut honest_ns = Vec::with_capacity(iters);
    let mut cheap_ns = Vec::with_capacity(iters);
    // Per-pair ratio in thousandths, captured before any sorting so each entry
    // is one moment rather than two order statistics divided by each other.
    let mut ratio_milli = Vec::with_capacity(iters);

    for i in 0..(WARMUP + iters) {
        let mut take_honest = || {
            honest_f.write_all(&payload).expect("write honest payload");
            let t = Instant::now();
            honest_sync(&honest_f).expect("honest sync");
            ns_since(t)
        };
        let mut take_cheap = || {
            cheap_f.write_all(&payload).expect("write cheap payload");
            let t = Instant::now();
            // SAFETY: an open file we own; fsync takes only the fd.
            let rc = unsafe { fsync(std::os::unix::io::AsRawFd::as_raw_fd(&cheap_f)) };
            assert_eq!(rc, 0, "fsync failed");
            ns_since(t)
        };
        let (h, c) = if i % 2 == 0 {
            let h = take_honest();
            (h, take_cheap())
        } else {
            let c = take_cheap();
            (take_honest(), c)
        };
        if i >= WARMUP {
            ratio_milli.push(h.saturating_mul(1000) / c.max(1));
            honest_ns.push(h);
            cheap_ns.push(c);
        }
    }

    let honest_p50 = {
        let mut v = honest_ns.clone();
        v.sort_unstable();
        nearest_rank(&v, 50, 100)
    };
    let cheap_p50 = {
        let mut v = cheap_ns.clone();
        v.sort_unstable();
        nearest_rank(&v, 50, 100)
    };

    report("sync_class_honest_fullfsync", 1, iters, &mut honest_ns);
    report("sync_class_cheap_fsync", 1, iters, &mut cheap_ns);
    // The ratio DISTRIBUTION, not just its middle. The minimum is the figure a
    // tail claim lives or dies on: measured min has been ~1.0x, meaning that
    // under load a plain push can cost as much as a full barrier. "N cheap
    // pushes" is cheap at p50 and is not uniformly cheap at the tail.
    report("sync_class_ratio_milli", 1, iters, &mut ratio_milli);
    report_load("sync_class.end");

    assert!(
        honest_p50 > cheap_p50 * 5,
        "honest sync p50 {honest_p50}ns is not materially above raw fsync p50 \
         {cheap_p50}ns — the honest class is not reaching the device, so every \
         durability figure here would be measuring the wrong syscall"
    );
}

/// The syscall COUNT per commit, swept over destination count D — the
/// load-independent half of this card's evidence, and the half that leads
/// `RESULTS.md`.
///
/// A latency table on this host can be argued away by load, and correctly so:
/// the same class measured minutes apart has differed 65x here. A syscall count
/// cannot be argued away. It is deterministic, it reproduces, and it does not
/// move with the fleet — which is why the baseline was settled by three teams
/// on three methods with no dissent.
///
/// This test states no expected number. It reports what the counters say, at
/// D ∈ {1,2,3}, one writer, so the shape in D is visible: a count that grows
/// with D and a count that is constant in D are different architectures, and
/// which one is on disk is a measurement, not an argument. The pre-registered
/// prediction lives in `RESULTS.md` and in the session inbox, written before
/// this ran; keeping it out of the assertion is deliberate, so a failed
/// prediction surfaces as a number rather than as a red test somebody is
/// tempted to adjust.
///
/// Run it under the interposer for the outside witness:
/// `DYLD_INSERT_LIBRARIES=.scratch/instrument/libsyncprobe.dylib` on the test
/// binary, filtered to this test name.
#[test]
fn barrier_count_per_commit_across_destination_counts() {
    let base = fixture_root().join("counts");
    let _ = std::fs::remove_dir_all(&base);
    report_load("counts.start");

    for dests in [1usize, 2, 3] {
        let dir = base.join(format!("d{dests}"));
        std::fs::create_dir_all(&dir).expect("count fixture dir");
        let root = WorkspaceRoot(dir);
        // 8 commits is plenty: the count is deterministic, so the only thing
        // more repetitions buy is confirmation that it does not vary.
        let (samples, _) = run_round(&root, 1, 8, dests, EpochCfg::default());
        let honest: Vec<u64> = samples.iter().map(|m| m.honest_syncs).collect();
        let pushes: Vec<u64> = samples.iter().map(|m| m.data_pushes).collect();
        println!(
            "GATECOUNT dests={dests} commits={} \
             honest_min={} honest_max={} pushes_min={} pushes_max={} \
             honest_observed={:?} pushes_observed={:?}",
            samples.len(),
            honest.iter().min().copied().unwrap_or(0),
            honest.iter().max().copied().unwrap_or(0),
            pushes.iter().min().copied().unwrap_or(0),
            pushes.iter().max().copied().unwrap_or(0),
            honest,
            pushes,
        );
    }
    report_load("counts.end");
}

/// How many times the whole writer-count sweep is repeated. The configurations
/// are **interleaved across rounds**, not run one after another to completion.
const ROUNDS: usize = 4;

/// The baseline: the real `publish()` at one writer and at N concurrent, with
/// content durability, metadata publish and the owner step as three separate
/// figures, plus the barrier count per commit.
///
/// The sweep is interleaved for the same reason the class comparison is paired.
/// Run to completion one configuration at a time, N=64 would be sampled minutes
/// after N=1, and on this host device latency drifts several-fold inside such a
/// window — the sweep would then report drift as a concurrency effect, which is
/// exactly the conclusion it exists to test. Round-robin instead, and pool each
/// configuration's samples across rounds, so every configuration is spread over
/// the same wall-clock window and any drift hits all of them alike.
#[test]
fn publish_path_durability_across_writer_counts() {
    let base = fixture_root().join("baseline");
    let _ = std::fs::remove_dir_all(&base);
    let pooled = pooled_target();
    let configs = [1usize, 4, 16, 64];

    let mut samples: Vec<Vec<Measured>> = configs.iter().map(|_| Vec::new()).collect();
    let mut wall: Vec<u64> = configs.iter().map(|_| 0).collect();

    for round in 0..ROUNDS {
        for (slot, &writers) in configs.iter().enumerate() {
            let dir = base.join(format!("n{writers}/r{round}"));
            std::fs::create_dir_all(&dir).expect("config fixture dir");
            let root = WorkspaceRoot(dir);
            let iters = (pooled / (writers * ROUNDS)).max(4);
            report_load(&format!("round{round}.n{writers}.start"));
            let (got, ns) = run_round(&root, writers, iters, 1, EpochCfg::default());
            report_load(&format!("round{round}.n{writers}.end"));
            samples[slot].extend(got);
            wall[slot] += ns;
        }
    }

    for (slot, &writers) in configs.iter().enumerate() {
        let iters = (pooled / (writers * ROUNDS)).max(4);
        report_config(writers, iters, &samples[slot], wall[slot]);
    }
}

/// Barrier count under CONCURRENCY — the claim that N writers sharing an epoch
/// pay barriers per *epoch*, not per commit.
///
/// The per-thread counter cannot answer this: the seal is issued by whichever
/// thread is sealing, so a per-thread delta attributes the whole epoch's barrier
/// to one writer and zero to its peers. The process-wide counter is the right
/// instrument, and its delta across the whole round divided by the commits in
/// that round is the figure — a batching ratio measured, not predicted.
#[test]
fn barrier_count_under_concurrency() {
    let base = fixture_root().join("counts-concurrent");
    let _ = std::fs::remove_dir_all(&base);
    report_load("concurrent_counts.start");

    for writers in [1usize, 4, 16, 64] {
        let dir = base.join(format!("n{writers}"));
        std::fs::create_dir_all(&dir).expect("fixture dir");
        let root = WorkspaceRoot(dir);
        let iters = 8;
        let h0 = fs::honest_sync_count();
        let d0 = fs::data_push_count();
        let (samples, wall_ns) = run_round(&root, writers, iters, 1, EpochCfg::default());
        let honest = fs::honest_sync_count() - h0;
        let pushes = fs::data_push_count() - d0;
        // Includes the warmup commits, which issue barriers too — the process
        // counter cannot see the warmup flag, so state the divisor honestly.
        let commits = (writers * (WARMUP + iters)) as u64;
        println!(
            "EPOCHRATIO writers={writers} commits_incl_warmup={commits} \
             honest_barriers_total={honest} pushes_total={pushes} \
             honest_per_commit_milli={} pushes_per_commit_milli={} \
             measured_samples={} wall_ns={wall_ns}",
            honest * 1000 / commits,
            pushes * 1000 / commits,
            samples.len(),
        );
    }
    report_load("concurrent_counts.end");
}

/// The WINDOW SWEEP, crossed with CONCURRENCY — the knob measured where it can
/// help as well as where it can hurt.
///
/// Team C's assigned risk is added latency for a lone writer that must wait out
/// a window with nobody to batch with, and at N=1 the window can only cost: the
/// writer pays it once per barrier, twice per commit. But the same knob points
/// the other way at high N. With `window = 0` an epoch seals immediately with
/// whoever is present, so batch density is whatever arrival timing happens to
/// give — measured barriers per commit was 0.82 at N=16, meaning writers spread
/// across many epochs rather than sharing few. A nonzero window should raise
/// density and can therefore *lower* per-commit latency under contention.
///
/// Sweeping only N=1 would have shown the window as a pure cost, which is the
/// same defect as a matrix containing only the regimes that favour the design,
/// pointed at ourselves. Both directions are reported, including if the honest
/// finding is that the right window is contention-dependent and our story is
/// more complicated than "no window".
#[test]
fn window_sweep_across_concurrency() {
    let base = fixture_root().join("window");
    let _ = std::fs::remove_dir_all(&base);
    report_load("window.start");

    for writers in [1usize, 16, 64] {
        for micros in [0u64, 1000, 5000] {
            let dir = base.join(format!("n{writers}-w{micros}"));
            std::fs::create_dir_all(&dir).expect("fixture dir");
            let root = WorkspaceRoot(dir);
            let cfg = EpochCfg {
                window: Duration::from_micros(micros),
                ..EpochCfg::default()
            };
            let iters = if writers == 1 { 64 } else { 8 };
            let h0 = fs::honest_sync_count();
            let (samples, wall_ns) = run_round(&root, writers, iters, 1, cfg);
            let honest = fs::honest_sync_count() - h0;
            let commits = (writers * (WARMUP + iters)) as u64;
            let mut content: Vec<u64> = samples.iter().map(|m| m.content_ns).collect();
            let mut call: Vec<u64> = samples.iter().map(|m| m.call_ns).collect();
            report(
                &format!("window{micros}us_n{writers}_content_durable"),
                writers,
                iters,
                &mut content,
            );
            report(
                &format!("window{micros}us_n{writers}_publish_call"),
                writers,
                iters,
                &mut call,
            );
            println!(
                "WINDOWCELL window_us={micros} writers={writers} \
                 commits_incl_warmup={commits} honest_barriers_total={honest} \
                 honest_per_commit_milli={} wall_ns={wall_ns} ns_per_txn={}",
                honest * 1000 / commits,
                wall_ns / samples.len() as u64,
            );
        }
    }
    report_load("window.end");
}

/// SEAL MODE comparison — `AnchorBarrier` (one honest sync per epoch, on the
/// anchor descriptor) against `PerMember` (one per member fd, in parallel).
///
/// `PerMember` is the shipped fallback if a reviewer rejects the shared-barrier
/// argument, so its cost is the price of the conservative answer and is reported
/// as such. Both modes are measured in the same run at the same writer counts so
/// the comparison is paired rather than taken minutes apart.
#[test]
fn seal_mode_comparison() {
    let base = fixture_root().join("sealmode");
    let _ = std::fs::remove_dir_all(&base);
    report_load("sealmode.start");

    for writers in [1usize, 16] {
        for (label, mode) in [
            ("anchor", SealMode::AnchorBarrier),
            ("permember", SealMode::PerMember),
        ] {
            let dir = base.join(format!("n{writers}-{label}"));
            std::fs::create_dir_all(&dir).expect("fixture dir");
            let root = WorkspaceRoot(dir);
            let cfg = EpochCfg {
                mode,
                ..EpochCfg::default()
            };
            let iters = 16;
            let h0 = fs::honest_sync_count();
            let (samples, wall_ns) = run_round(&root, writers, iters, 1, cfg);
            let honest = fs::honest_sync_count() - h0;
            let commits = (writers * (WARMUP + iters)) as u64;
            let mut content: Vec<u64> = samples.iter().map(|m| m.content_ns).collect();
            report(
                &format!("seal_{label}_content_durable"),
                writers,
                iters,
                &mut content,
            );
            println!(
                "SEALCOUNT mode={label} writers={writers} commits_incl_warmup={commits} \
                 honest_barriers_total={honest} honest_per_commit_milli={} wall_ns={wall_ns}",
                honest * 1000 / commits,
            );
        }
    }
    report_load("sealmode.end");
}

/// ITEM 11 — ADVERSARIAL ARRIVAL TIMING, the cell this design is predicted to
/// lose, measured rather than conceded.
///
/// A writer that arrives just after a seal begins must wait out that in-flight
/// seal before its own epoch can seal — a worst case of roughly two barriers
/// where a per-writer design pays one. The question this test answers is not
/// whether that happens; it is whether the wait is **bounded** at about 2x a
/// barrier or whether a writer can **starve** by repeatedly missing epochs.
///
/// Construction: one victim thread publishes alongside a continuous stream of
/// noise writers, so the victim is arriving at arbitrary phases of other
/// writers' seals for its whole run. Its phase-1 (`content`) distribution is the
/// answer. Two things are reported and they answer different questions:
///
/// - **max, in barrier units** — bounded near 2x is the predicted loss;
///   materially above it is worse than predicted.
/// - **first half against second half** — a victim that is starving gets worse
///   as the run proceeds. A flat comparison means unlucky, not starving. This is
///   the falsifier, and an unbounded or growing result is reported as a
///   falsification rather than softened.
#[test]
fn adversarial_arrival_timing() {
    let base = fixture_root().join("arrival");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).expect("fixture dir");
    report_load("arrival.start");

    let root = WorkspaceRoot(base);
    let noise = 7usize;
    let iters = 48usize;
    // Every thread publishes; thread 0 is the victim whose samples we read.
    let (samples, wall_ns) = run_round(&root, noise + 1, iters, 1, EpochCfg::default());

    // run_round pools by thread in spawn order, so the victim's samples are the
    // first `iters` entries.
    let victim: Vec<u64> = samples.iter().take(iters).map(|m| m.content_ns).collect();
    // The peers, pooled. This is the control the halves test lacks: a victim
    // that is merely paying the contention its peers also pay has no
    // arrival-specific penalty, however fat the absolute tail is. Comparing the
    // victim against its own median cannot tell those apart; comparing it
    // against the peer distribution can.
    let peers: Vec<u64> = samples.iter().skip(iters).map(|m| m.content_ns).collect();
    let half = victim.len() / 2;
    let mut first: Vec<u64> = victim[..half].to_vec();
    let mut second: Vec<u64> = victim[half..].to_vec();
    let mut all = victim.clone();
    let mut peer_sorted = peers.clone();

    report("arrival_victim_content_all", 1, iters, &mut all);
    report("arrival_victim_content_firsthalf", 1, half, &mut first);
    report("arrival_victim_content_secondhalf", 1, half, &mut second);
    report("arrival_peers_content_all", noise, iters, &mut peer_sorted);

    // Where the victim's median falls inside the peer distribution. ~500 per
    // mille means the victim is an ordinary writer; a value near 1000 means it
    // is being singled out by its arrival timing.
    let v_p50 = nearest_rank(&all, 50, 100);
    let below = peer_sorted.iter().filter(|&&p| p < v_p50).count();
    let pct_milli = if peer_sorted.is_empty() {
        0
    } else {
        (below * 1000 / peer_sorted.len()) as u64
    };
    println!(
        "ARRIVAL noise_writers={noise} victim_iters={iters} wall_ns={wall_ns} \
         victim_p50_ns={v_p50} victim_max_ns={} peer_p50_ns={} peer_max_ns={} \
         victim_rank_in_peers_milli={pct_milli} \
         firsthalf_p50_ns={} secondhalf_p50_ns={}",
        all.iter().max().copied().unwrap_or(0),
        nearest_rank(&peer_sorted, 50, 100),
        peer_sorted.iter().max().copied().unwrap_or(0),
        nearest_rank(&first, 50, 100),
        nearest_rank(&second, 50, 100),
    );
    report_load("arrival.end");
}

/// The CORPUS LAYOUT sweep — the cheap-push count is a function of D **and** of
/// the number of distinct destination directories, and this measures both axes
/// against a formula derived from the code beforehand.
///
/// Derivation (`fs::intent::write_staged` pushes inline then seals the file and
/// its parent; `store_images` runs it twice per destination; `create_decided`
/// seals the intent file and its parent; `rename_all` seals destination parent
/// directories only; `publish::barrier` sorts and dedupes before pushing):
///
/// ```text
/// phase 1 = 2D inline + |{D tmps, 2D images, 1 image dir, 1 intent file,
///                         1 intents dir}|                      = 5D + 3
/// phase 2 = 1  inline + |{Ddir dest parents, 1 intent file,
///                         1 intents dir}|                      = Ddir + 3
/// cheap pushes per commit = 5D + Ddir + 6
/// honest barriers per commit = 2, independent of D and Ddir
/// ```
///
/// `dedup` after `sort` removes only exact duplicates, so no distinct path ever
/// loses its push — that is why collapsing repeated parent directories is not a
/// skipped push. The property is provable from the operation and holds at any D,
/// which the earlier total-count comparison did not: a slope of 5 against a
/// baseline slope of 6 crosses at D=3 and inverts after it.
///
/// This test prints; it does not assert the formula. The prediction is
/// pre-registered with the leader, so a miss must surface as a number rather
/// than as a red test somebody is tempted to adjust.
#[test]
fn barrier_count_corpus_layout_sweep() {
    let base = fixture_root().join("layout");
    let _ = std::fs::remove_dir_all(&base);
    report_load("layout.start");

    for dests in [1usize, 2, 3, 8] {
        for spread in [false, true] {
            let dir_count = if spread { dests } else { 1 };
            let tag = if spread { "perdest" } else { "shared" };
            let dir = base.join(format!("d{dests}-{tag}"));
            std::fs::create_dir_all(&dir).expect("fixture dir");
            let root = WorkspaceRoot(dir);
            let table = ReservationTable::new();
            let owner = StateOwner::new(Root("r0".to_owned()));
            let batcher = EpochBatcher::new(&root.0.join(ANCHOR_REL), Duration::ZERO, usize::MAX)
                .expect("epoch batcher");
            let plane = Plane {
                root: &root,
                table: &table,
                owner: &owner,
                batcher: &batcher,
            };
            let payload = vec![b'm'; PAYLOAD];

            let mut honest = Vec::new();
            let mut pushes = Vec::new();
            for i in 0..(WARMUP + 8) {
                let m = one_txn_layout(&plane, 0, i, dests, spread, &payload);
                if i >= WARMUP {
                    honest.push(m.honest_syncs);
                    pushes.push(m.data_pushes);
                }
            }
            println!(
                "LAYOUTCOUNT dests={dests} ddir={dir_count} layout={tag} commits={} \
                 predicted_cheap={} honest_min={} honest_max={} \
                 cheap_min={} cheap_max={}",
                honest.len(),
                5 * dests + dir_count + 6,
                honest.iter().min().copied().unwrap_or(0),
                honest.iter().max().copied().unwrap_or(0),
                pushes.iter().min().copied().unwrap_or(0),
                pushes.iter().max().copied().unwrap_or(0),
            );
        }
    }
    report_load("layout.end");
}
