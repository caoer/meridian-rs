//! honest-durability measurement harness (Team B, card `hdb-measure`).
//!
//! Measures the two costs of a staged-publish commit SEPARATELY (gate 12):
//! the content-durability wait (fsync latency) and the metadata publish
//! (rename + parent-directory sync). Three modes, same commit code, only the
//! sync placement differs:
//!
//! - `today` — `File::sync_all` inline, the live `write_fsync`/`fsync_dir`
//!   placement. On macOS std issues `fcntl(F_FULLFSYNC)` here already.
//! - `naive` — `fs::honest_sync` inline per writer, the serialization
//!   reference. On macOS this is syscall-identical to `today` (control cell).
//! - `actor` — every honest sync issued by the dedicated durability actor
//!   (Team B's shape). Wired to `crates/fs` when the actor lands.
//!
//! The `real` family drives the ENGINE's publish path (`wire_serve::publish`)
//! rather than a synthesized commit: `bench-real` measures one (destinations,
//! writers) cell with the caller's staging sync, the receipt's durability
//! interval, and its metadata interval reported as SEPARATE figures;
//! `receipt-real` provides the `fs_usage` attach window so honest syncs per
//! transaction are counted at the syscall boundary (the 6D+4 cross-check).
//!
//! Subcommands:
//!   baseline <dir> [iters]            three sync classes, one 4 KiB file
//!   bench <dir> <mode> <writers> <iters>   one synthesized-commit cell
//!   receipt <dir> <mode> <iters>      pid + attach window for `fs_usage`
//!   bench-real <dir> <dests> <writers> <iters>   one engine-path cell
//!   receipt-real <dir> <dests> <iters>           engine-path count window
//!
//! Percentiles ride `perfsuite::measure::LatencyRun` — the one instrument.

use std::fs::File;
use std::io::Write as _;
use std::os::fd::AsRawFd as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

use ::fs::WorkspaceRoot;
use ::fs::intent::Policy;
use perfsuite::measure::LatencyRun;
use wire::Root;
use wire_serve::authority::AuthorityEpoch;
use wire_serve::publish::{
    DestImage, Region, RegionKind, ReservationTable, StagedTxn, StateOwner, TxnId, publish,
};

const WRITE_SIZE: usize = 4096;
const WARMUP_ITERS: u32 = 20;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let strs: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match strs.as_slice() {
        ["baseline", dir] => baseline(Path::new(dir), 1000),
        ["baseline", dir, iters] => baseline(Path::new(dir), parse(iters)),
        ["premise", dir, pairs] => premise(Path::new(dir), parse(pairs), WRITE_SIZE),
        ["premise", dir, pairs, payload] => {
            premise(Path::new(dir), parse(pairs), parse(payload) as usize)
        }
        ["bench", dir, mode, writers, iters] => {
            bench(Path::new(dir), mode, parse(writers), parse(iters))
        }
        ["receipt", dir, mode, iters] => receipt(Path::new(dir), mode, parse(iters)),
        ["bench-real", dir, dests, writers, iters] => bench_real(
            Path::new(dir),
            parse(dests),
            parse(writers),
            parse(iters),
            Staging::Submitted,
        ),
        ["bench-real", dir, dests, writers, iters, staging] => bench_real(
            Path::new(dir),
            parse(dests),
            parse(writers),
            parse(iters),
            parse_staging(staging),
        ),
        ["receipt-real", dir, dests, iters] => receipt_real(
            Path::new(dir),
            parse(dests),
            parse(iters),
            Staging::Submitted,
        ),
        ["receipt-real", dir, dests, iters, staging] => receipt_real(
            Path::new(dir),
            parse(dests),
            parse(iters),
            parse_staging(staging),
        ),
        ["sweep-real", dir, dests, rounds, iters, levels] => sweep_real(
            Path::new(dir),
            parse(dests),
            parse(rounds),
            parse(iters),
            levels,
            Staging::Submitted,
        ),
        ["sweep-real", dir, dests, rounds, iters, levels, staging] => sweep_real(
            Path::new(dir),
            parse(dests),
            parse(rounds),
            parse(iters),
            levels,
            parse_staging(staging),
        ),
        ["loss-a", dir, max_offset_us, step_us, samples] => loss_a(
            Path::new(dir),
            parse(max_offset_us),
            parse(step_us),
            parse(samples),
        ),
        ["loss-b", dir, big_d, small_writers, iters] => loss_b(
            Path::new(dir),
            parse(big_d),
            parse(small_writers),
            parse(iters),
        ),
        _ => {
            eprintln!(
                "usage: hdb-durability baseline <dir> [iters]\n       \
                 hdb-durability bench <dir> today|naive|actor <writers> <iters>\n       \
                 hdb-durability receipt <dir> today|naive|actor <iters>\n       \
                 hdb-durability bench-real <dir> <dests> <writers> <iters> [settled|submitted]\n       \
                 hdb-durability receipt-real <dir> <dests> <iters> [settled|submitted]\n       \
                 hdb-durability sweep-real <dir> <dests> <rounds> <iters_per_visit> <n1,n2,..> [staging]\n       \
                 hdb-durability loss-a <dir> <max_offset_us> <step_us> <samples>\n       \
                 hdb-durability loss-b <dir> <bigD> <small_writers> <iters>"
            );
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("hdb-durability: {e}");
            ExitCode::FAILURE
        }
    }
}

fn parse(s: &str) -> u32 {
    s.parse().unwrap_or_else(|_| {
        eprintln!("not a number: {s}");
        std::process::exit(2);
    })
}

/// The sync placement under test. `Today` and `Naive` sync inline in the
/// writer; `Actor` is Team B's dedicated durability actor.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Today,
    Naive,
    Actor,
}

impl Mode {
    fn from_str(s: &str) -> std::io::Result<Self> {
        match s {
            "today" => Ok(Self::Today),
            "naive" => Ok(Self::Naive),
            "actor" => Ok(Self::Actor),
            other => Err(std::io::Error::other(format!("unknown mode: {other}"))),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Today => "today",
            Self::Naive => "naive",
            Self::Actor => "actor",
        }
    }
}

// ---------------------------------------------------------------------------
// baseline — the sync classes, single file, single thread
// ---------------------------------------------------------------------------

/// Re-measure the pinned probe on THIS host, PAIRED (gate item 5 as amended):
/// all three classes sampled inside ONE iteration, each on its own fd with a
/// fresh 4 KiB write immediately before the measured call, order rotating per
/// iteration so no class always goes first. Pairs share machine conditions,
/// so the per-pair honest/cheap ratio survives host noise the absolute level
/// does not (reference: results/instrument-evidence/paired_push.rs).
fn baseline(dir: &Path, iters: u32) -> std::io::Result<()> {
    let classes: [(&str, ContentSync); 3] = [
        ("fsync(2) direct", plain_fsync),
        ("fs::honest_sync", ::fs::honest_sync),
        ("File::sync_all", sync_all_file),
    ];
    let mut files: Vec<File> = Vec::new();
    let mut paths = Vec::new();
    for i in 0..classes.len() {
        let path = dir.join(format!("hdb-baseline-{}-{i}.dat", std::process::id()));
        files.push(File::create(&path)?);
        paths.push(path);
    }
    let buf = vec![b'x'; WRITE_SIZE];
    let load_before = loadavg();

    let mut runs: Vec<LatencyRun> = classes.iter().map(|_| LatencyRun::new()).collect();
    // Per-pair honest/cheap ratios (x1000), computed inside the iteration —
    // percentile by sorted index, mirroring the advisor's reference
    // instrument (paired_push.rs), not a second latency instrument.
    let mut ratios: Vec<u128> = Vec::with_capacity(iters as usize);
    for iter in 0..(WARMUP_ITERS + iters) {
        let mut elapsed = [0u128; 3];
        for k in 0..classes.len() {
            let c = (iter as usize + k) % classes.len(); // rotate order per iteration
            files[c].write_all(&buf)?;
            let t = Instant::now();
            classes[c].1(&files[c])?;
            elapsed[c] = t.elapsed().as_nanos();
        }
        if iter >= WARMUP_ITERS {
            for (c, run) in runs.iter_mut().enumerate() {
                run.record(Duration::from_nanos(
                    u64::try_from(elapsed[c]).unwrap_or(u64::MAX),
                ));
            }
            ratios.push(elapsed[1] * 1000 / elapsed[0].max(1)); // honest_sync / fsync(2)
        }
    }
    let load_after = loadavg();

    println!(
        "baseline PAIRED: {iters} iters x {WRITE_SIZE} B fresh write + sync per class per iteration, dir {}",
        dir.display()
    );
    println!("load_avg before {load_before} after {load_after}");
    println!(
        "{:<16} {:>10} {:>10} {:>10} {:>10} {:>8}",
        "class", "p50_ms", "p95_ms", "p99_ms", "max_ms", "samples"
    );
    for (i, (name, _)) in classes.iter().enumerate() {
        let d = runs[i].dist();
        println!(
            "{name:<16} {:>10.3} {:>10.3} {:>10.3} {:>10.3} {:>8}",
            d.p50_us / 1000.0,
            runs[i].percentile_us(0.95) / 1000.0,
            d.p99_us / 1000.0,
            d.max_us / 1000.0,
            runs[i].samples()
        );
    }
    ratios.sort_unstable();
    println!(
        "per_pair_ratio honest/fsync x1000: p50={} p90={} p99={} min={} max={}",
        pct(&ratios, 50),
        pct(&ratios, 90),
        pct(&ratios, 99),
        ratios.first().copied().unwrap_or(0),
        ratios.last().copied().unwrap_or(0)
    );
    for p in paths {
        std::fs::remove_file(p)?;
    }
    Ok(())
}

/// Sorted-index percentile for the per-pair ratio vectors — the advisor's
/// reference method (`paired_push.rs` `nr`), kept for ratios only; latency
/// percentiles all ride [`LatencyRun`].
fn pct(v: &[u128], q: usize) -> u128 {
    if v.is_empty() {
        return 0;
    }
    v[((v.len() * q).div_ceil(100)).clamp(1, v.len()) - 1]
}

/// The 1/5/15-minute load averages, printed beside every measured table
/// (gate item 5 as amended: report the load observed during the run).
fn loadavg() -> String {
    let mut avg = [0f64; 3];
    // SAFETY: getloadavg fills up to 3 doubles of a valid, owned buffer.
    let n = unsafe { libc::getloadavg(avg.as_mut_ptr(), 3) };
    if n <= 0 {
        return "unavailable".to_owned();
    }
    format!("{:.2} {:.2} {:.2}", avg[0], avg[1], avg[2])
}

/// The criterion-1 premise experiment: does a cheap `fsync(2)` push actually
/// move bytes to the DEVICE, so a later barrier need not carry the writeout?
/// Per pair, alternating order: arm P writes `payload`, cheap-pushes it, then
/// times `F_FULLFSYNC`; arm D writes the same payload to its own fd and times
/// `F_FULLFSYNC` directly (the barrier carries the writeout). Indistinguishable
/// barriers would mean the cheap push buys nothing and the actor's
/// group-commit premise fails — reported either way.
fn premise(dir: &Path, pairs: u32, payload: usize) -> std::io::Result<()> {
    let path_p = dir.join(format!("hdb-premise-p-{}.dat", std::process::id()));
    let path_d = dir.join(format!("hdb-premise-d-{}.dat", std::process::id()));
    let mut fp = File::create(&path_p)?;
    let mut fd = File::create(&path_d)?;
    let buf = vec![b'x'; payload];
    let load_before = loadavg();

    let mut push_run = LatencyRun::new();
    let mut barrier_pushed_run = LatencyRun::new();
    let mut settle_run = LatencyRun::new();
    let mut barrier_dirty_run = LatencyRun::new();
    let mut ratios: Vec<u128> = Vec::with_capacity(pairs as usize);

    for iter in 0..(WARMUP_ITERS + pairs) {
        let arm_p = |fp: &mut File| -> std::io::Result<(Duration, Duration)> {
            fp.write_all(&buf)?;
            let t0 = Instant::now();
            plain_fsync(fp)?;
            let push = t0.elapsed();
            let t1 = Instant::now();
            ::fs::honest_sync(fp)?;
            Ok((push, t1.elapsed()))
        };
        let arm_d = |fd: &mut File| -> std::io::Result<Duration> {
            fd.write_all(&buf)?;
            let t = Instant::now();
            ::fs::honest_sync(fd)?;
            Ok(t.elapsed())
        };
        let ((push, barrier_pushed), barrier_dirty) = if iter % 2 == 0 {
            let p = arm_p(&mut fp)?;
            (p, arm_d(&mut fd)?)
        } else {
            let d = arm_d(&mut fd)?;
            (arm_p(&mut fp)?, d)
        };
        if iter >= WARMUP_ITERS {
            push_run.record(push);
            barrier_pushed_run.record(barrier_pushed);
            settle_run.record(push + barrier_pushed);
            barrier_dirty_run.record(barrier_dirty);
            ratios.push(barrier_dirty.as_nanos() * 1000 / barrier_pushed.as_nanos().max(1));
        }
    }
    let load_after = loadavg();

    println!(
        "premise PAIRED: {pairs} pairs x {payload} B payload per arm per iteration, dir {}",
        dir.display()
    );
    println!("load_avg before {load_before} after {load_after}");
    println!(
        "{:<24} {:>10} {:>10} {:>10} {:>10} {:>8}",
        "figure", "p50_ms", "p95_ms", "p99_ms", "max_ms", "samples"
    );
    for (name, run) in [
        ("cheap_push fsync(2)", &push_run),
        ("barrier_after_push", &barrier_pushed_run),
        ("batch_settle push+bar", &settle_run),
        ("barrier_with_writeout", &barrier_dirty_run),
    ] {
        let d = run.dist();
        println!(
            "{name:<24} {:>10.3} {:>10.3} {:>10.3} {:>10.3} {:>8}",
            d.p50_us / 1000.0,
            run.percentile_us(0.95) / 1000.0,
            d.p99_us / 1000.0,
            d.max_us / 1000.0,
            run.samples()
        );
    }
    ratios.sort_unstable();
    println!(
        "per_pair_ratio dirty/pushed x1000: p50={} p90={} p99={} min={} max={}",
        pct(&ratios, 50),
        pct(&ratios, 90),
        pct(&ratios, 99),
        ratios.first().copied().unwrap_or(0),
        ratios.last().copied().unwrap_or(0)
    );
    std::fs::remove_file(&path_p)?;
    std::fs::remove_file(&path_d)?;
    Ok(())
}

/// `fsync(2)` with no `F_FULLFSYNC` upgrade — the drive-cache class. No std
/// API issues this on macOS (`sync_all`/`sync_data` both upgrade), so libc.
fn plain_fsync(f: &File) -> std::io::Result<()> {
    // SAFETY: fsync on a valid fd we own.
    let rc = unsafe { libc::fsync(f.as_raw_fd()) };
    if rc == -1 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// the commit under measurement — identical across modes but for the syncs
// ---------------------------------------------------------------------------

/// One writer's per-iteration timings, recorded separately (gate 12).
struct CommitLatency {
    fsync: Duration,
    publish: Duration,
}

/// Stage-and-publish one commit the way `stage_file` + `commit_rename` do:
/// write a fresh temp beside `dst`, make its bytes durable (mode's class),
/// rename onto `dst`, make the rename durable (mode's class). Returns the
/// two waits separately.
type ContentSync = fn(&File) -> std::io::Result<()>;

fn commit_once(
    dst: &Path,
    tmp: &Path,
    buf: &[u8],
    mode: Mode,
    root: &Path,
) -> std::io::Result<CommitLatency> {
    let mut f = File::create(tmp)?;
    f.write_all(buf)?;
    let t0 = Instant::now();
    mode_content_sync(mode, root, &f)?;
    let fsync = t0.elapsed();
    drop(f);

    let parent = dst.parent().unwrap_or_else(|| Path::new("."));
    let t1 = Instant::now();
    std::fs::rename(tmp, dst)?;
    mode_dir_sync(mode, root, parent)?;
    let publish = t1.elapsed();
    Ok(CommitLatency { fsync, publish })
}

fn sync_all_file(f: &File) -> std::io::Result<()> {
    f.sync_all()
}

fn sync_all_path(p: &Path) -> std::io::Result<()> {
    File::open(p)?.sync_all()
}

// ---------------------------------------------------------------------------
// bench — one (mode, writers) cell
// ---------------------------------------------------------------------------

/// Run `writers` concurrent writer threads, each committing `iters` times to
/// its own destination in the shared `dir`, and print the cell's fsync and
/// publish percentiles separately.
fn bench(dir: &Path, mode: &str, writers: u32, iters: u32) -> std::io::Result<()> {
    let mode = Mode::from_str(mode)?;
    let start = Barrier::new(writers as usize);
    let mut cells: Vec<Vec<CommitLatency>> = Vec::new();
    std::thread::scope(|scope| -> std::io::Result<()> {
        let mut handles = Vec::new();
        for w in 0..writers {
            let start = &start;
            handles.push(scope.spawn(move || -> std::io::Result<Vec<CommitLatency>> {
                let dst = dir.join(format!("w{w}.dat"));
                let tmp = dir.join(format!(".w{w}.tmp"));
                let buf = vec![b'x'; WRITE_SIZE];
                for _ in 0..WARMUP_ITERS {
                    commit_once(&dst, &tmp, &buf, mode, dir)?;
                }
                start.wait();
                let mut out = Vec::with_capacity(iters as usize);
                for _ in 0..iters {
                    out.push(commit_once(&dst, &tmp, &buf, mode, dir)?);
                }
                Ok(out)
            }));
        }
        for h in handles {
            cells.push(h.join().expect("writer thread panicked")?);
        }
        Ok(())
    })?;

    let mut fsync_run = LatencyRun::new();
    let mut publish_run = LatencyRun::new();
    for lat in cells.iter().flatten() {
        fsync_run.record(lat.fsync);
        publish_run.record(lat.publish);
    }
    let f = fsync_run.dist();
    let p = publish_run.dist();
    println!(
        "cell mode={} writers={writers} iters={iters} samples={} | \
         fsync_p50_ms={:.3} fsync_p99_ms={:.3} | publish_p50_ms={:.3} publish_p99_ms={:.3}",
        mode.name(),
        fsync_run.samples(),
        f.p50_us / 1000.0,
        f.p99_us / 1000.0,
        p.p50_us / 1000.0,
        p.p99_us / 1000.0,
    );
    cleanup(dir, writers);
    Ok(())
}

/// The content-sync placement per mode. `root` names the workspace whose
/// device the actor's barrier handle pins; inline modes ignore it.
fn mode_content_sync(mode: Mode, root: &Path, f: &File) -> std::io::Result<()> {
    match mode {
        Mode::Today => f.sync_all(),
        Mode::Naive => ::fs::honest_sync(f),
        Mode::Actor => ::fs::durability::settle(root, f).map(|_| ()),
    }
}

/// The dir-sync placement per mode (rename durability).
fn mode_dir_sync(mode: Mode, root: &Path, p: &Path) -> std::io::Result<()> {
    match mode {
        Mode::Today => sync_all_path(p),
        Mode::Naive => ::fs::honest_sync_path(p),
        Mode::Actor => ::fs::durability::settle_path(root, p).map(|_| ()),
    }
}

fn cleanup(dir: &Path, writers: u32) {
    for w in 0..writers {
        let _ = std::fs::remove_file(dir.join(format!("w{w}.dat")));
        let _ = std::fs::remove_file(dir.join(format!(".w{w}.tmp")));
    }
}

// ---------------------------------------------------------------------------
// receipt — the fs_usage attach window
// ---------------------------------------------------------------------------

/// Print our pid, hold 3 s for the tracer to attach, then run `iters`
/// single-writer commits in `mode` and print the expected barrier count so
/// the trace can be checked against it.
fn receipt(dir: &Path, mode: &str, iters: u32) -> std::io::Result<()> {
    let mode = Mode::from_str(mode)?;
    println!(
        "pid={} mode={} iters={iters}",
        std::process::id(),
        mode.name()
    );
    std::io::stdout().flush()?;
    std::thread::sleep(Duration::from_secs(3));

    let dst = dir.join("receipt.dat");
    let tmp = dir.join(".receipt.tmp");
    let buf = vec![b'x'; WRITE_SIZE];
    let c0 = counters();
    for _ in 0..iters {
        commit_once(&dst, &tmp, &buf, mode, dir)?;
    }
    let c1 = counters();
    println!(
        "done: {iters} commits, expected honest-class syncs on macOS = {iters} (content) + {iters} (dir)"
    );
    print_counter_deltas(c0, c1, u64::from(iters));
    let _ = std::fs::remove_file(&dst);
    Ok(())
}

// ---------------------------------------------------------------------------
// in-process counters — the eager-drain discriminator
// ---------------------------------------------------------------------------

/// A snapshot of the actor's three counters (module `fs::durability`):
/// process-wide honest ops, post-dedupe object pushes, actor barriers.
#[derive(Clone, Copy)]
struct Counters {
    honest_ops: u64,
    object_syncs: u64,
    actor_barriers: u64,
}

fn counters() -> Counters {
    Counters {
        honest_ops: ::fs::durability::honest_ops(),
        object_syncs: ::fs::durability::object_syncs(),
        actor_barriers: ::fs::durability::actor_barriers(),
    }
}

/// Per-txn counter deltas — printed x1000 so grouping shows without floats.
fn print_counter_deltas(c0: Counters, c1: Counters, txns: u64) {
    let t = txns.max(1);
    println!(
        "counters/txn x1000: honest_ops={} object_syncs={} actor_barriers={} (raw deltas {} {} {})",
        (c1.honest_ops - c0.honest_ops) * 1000 / t,
        (c1.object_syncs - c0.object_syncs) * 1000 / t,
        (c1.actor_barriers - c0.actor_barriers) * 1000 / t,
        c1.honest_ops - c0.honest_ops,
        c1.object_syncs - c0.object_syncs,
        c1.actor_barriers - c0.actor_barriers,
    );
}

// ---------------------------------------------------------------------------
// the real engine path — wire_serve::publish, D destinations per transaction
// ---------------------------------------------------------------------------

/// One transaction's timings on the real path, all reported separately —
/// ruling 18: three DISJOINT receipt intervals (content durability, metadata
/// publish, state-owner apply), none derived by subtraction, plus the
/// caller's staging syncs and the whole-transaction wall.
struct RealLatency {
    stage: Vec<Duration>,
    durability: Duration,
    metadata: Duration,
    apply: Duration,
    wall: Duration,
}

/// A writer's private lane in the shared workspace: its destination files
/// and the last bytes each is known to hold (`reverify` compares against
/// these, so tracking them in memory keeps reads out of the timed window).
struct Lane {
    rels: Vec<String>,
    cur: Vec<Vec<u8>>,
}

fn lane_setup(ws: &Path, writer: u32, dests: u32) -> std::io::Result<Lane> {
    let sub = ws.join(format!("w{writer}"));
    std::fs::create_dir_all(&sub)?;
    let mut rels = Vec::new();
    let mut cur = Vec::new();
    for d in 0..dests {
        let rel = format!("w{writer}/d{d}.md");
        let initial = vec![b'0'; WRITE_SIZE];
        std::fs::write(ws.join(&rel), &initial)?;
        rels.push(rel);
        cur.push(initial);
    }
    Ok(Lane { rels, cur })
}

/// How the harness stages dest temps: `Settled` syncs each inline before
/// publish (`pending` stays empty); `Submitted` hands each temp to the
/// durability actor at stage time and rides the [`Pending`]s into the txn —
/// the shape the actor-wired engine stages in.
#[derive(Clone, Copy, PartialEq)]
enum Staging {
    Settled,
    Submitted,
}

fn parse_staging(s: &str) -> Staging {
    match s {
        "settled" => Staging::Settled,
        "submitted" => Staging::Submitted,
        other => {
            eprintln!("unknown staging: {other}");
            std::process::exit(2);
        }
    }
}

/// Stage this iteration's D temps (per `staging`), publish through the REAL
/// `wire_serve::publish`, and return the separated timings.
fn real_txn_once(
    root: &WorkspaceRoot,
    table: &Arc<ReservationTable>,
    owner: &StateOwner,
    lane: &mut Lane,
    iter: u32,
    staging: Staging,
) -> std::io::Result<RealLatency> {
    let t_wall = Instant::now();
    let mut stage = Vec::with_capacity(lane.rels.len());
    let mut dests = Vec::with_capacity(lane.rels.len());
    let mut writes = Vec::with_capacity(lane.rels.len());
    let mut pending = Vec::new();
    let new = vec![b'a' + u8::try_from(iter % 26).expect("mod 26 fits"); WRITE_SIZE];
    for (d, rel) in lane.rels.iter().enumerate() {
        let dst = root.0.join(rel);
        let tmp = dst.with_extension("tmp-pub");
        std::fs::write(&tmp, &new)?;
        let t = Instant::now();
        match staging {
            Staging::Settled => ::fs::honest_sync_path(&tmp)?,
            Staging::Submitted => pending.push(::fs::durability::submit_path(&root.0, &tmp)?),
        }
        stage.push(t.elapsed());
        dests.push(DestImage {
            rel: PathBuf::from(rel),
            old: lane.cur[d].clone(),
            new: new.clone(),
            tmp,
        });
        writes.push(Region::file("c", rel.as_bytes(), RegionKind::Write));
    }
    let txn = StagedTxn {
        txn: TxnId::mint()?,
        epoch: AuthorityEpoch(1),
        reads: Vec::new(),
        writes,
        dests,
        pending,
        files: Vec::new(),
        actor: None,
        now: None,
        policy: Policy::Restore,
        drop_tmp_after_decide: false,
    };
    let rec = publish(root, table, owner, txn, |r| Root(r.0.clone()))
        .map_err(|e| std::io::Error::other(format!("publish: {e:?}")))?;
    let wall = t_wall.elapsed();
    for cur in &mut lane.cur {
        cur.clone_from(&new);
    }
    Ok(RealLatency {
        stage,
        durability: rec.durability_end.duration_since(rec.durability_start),
        metadata: rec.metadata_end.duration_since(rec.metadata_start),
        apply: rec.apply_end.duration_since(rec.apply_start),
        wall,
    })
}

/// One engine-path cell: `writers` concurrent threads, each publishing
/// `iters` transactions of `dests` destinations through the shared
/// reservation table and state owner. Staging sync, durability interval,
/// metadata interval, and transaction wall are printed as separate figures.
fn bench_real(
    dir: &Path,
    dests: u32,
    writers: u32,
    iters: u32,
    staging: Staging,
) -> std::io::Result<()> {
    let ws = dir.join(format!("hdb-real-{}", std::process::id()));
    std::fs::create_dir_all(&ws)?;
    let root = WorkspaceRoot(ws.clone());
    let table = ReservationTable::new();
    let owner = StateOwner::new(Root("hdb".into()));
    let load_before = loadavg();
    let c0 = counters();

    let start = Barrier::new(writers as usize + 1);
    let mut per_writer: Vec<Vec<RealLatency>> = Vec::new();
    let mut c_measured = c0;
    std::thread::scope(|scope| -> std::io::Result<()> {
        let mut handles = Vec::new();
        for w in 0..writers {
            let (root, table, owner, start) = (&root, &table, &owner, &start);
            handles.push(scope.spawn(move || -> std::io::Result<Vec<RealLatency>> {
                let mut lane = lane_setup(&root.0, w, dests)?;
                for i in 0..WARMUP_ITERS {
                    real_txn_once(root, table, owner, &mut lane, i, staging)?;
                }
                start.wait();
                let mut out = Vec::with_capacity(iters as usize);
                for i in 0..iters {
                    out.push(real_txn_once(root, table, owner, &mut lane, i, staging)?);
                }
                Ok(out)
            }));
        }
        start.wait(); // warmups done on every lane: snapshot the counters
        c_measured = counters();
        for h in handles {
            per_writer.push(h.join().expect("writer thread panicked")?);
        }
        Ok(())
    })?;
    let c1 = counters();
    let load_after = loadavg();

    let mut stage_run = LatencyRun::new();
    let mut durability_run = LatencyRun::new();
    let mut metadata_run = LatencyRun::new();
    let mut apply_run = LatencyRun::new();
    let mut wall_run = LatencyRun::new();
    for txn in per_writer.iter().flatten() {
        for s in &txn.stage {
            stage_run.record(*s);
        }
        durability_run.record(txn.durability);
        metadata_run.record(txn.metadata);
        apply_run.record(txn.apply);
        wall_run.record(txn.wall);
    }
    println!(
        "cell-real dests={dests} writers={writers} iters={iters} staging={} txns={} load_before=[{load_before}] load_after=[{load_after}]",
        if staging == Staging::Submitted {
            "submitted"
        } else {
            "settled"
        },
        wall_run.samples()
    );
    print_counter_deltas(c_measured, c1, u64::from(writers) * u64::from(iters));
    for (name, run) in [
        ("stage_sync (per barrier)", &stage_run),
        ("durability (decide)", &durability_run),
        ("metadata (rename_all)", &metadata_run),
        ("owner_apply", &apply_run),
        ("txn_wall", &wall_run),
    ] {
        let d = run.dist();
        println!(
            "  {name:<26} p50_ms={:.3} p99_ms={:.3} max_ms={:.3} samples={}",
            d.p50_us / 1000.0,
            d.p99_us / 1000.0,
            d.max_us / 1000.0,
            run.samples()
        );
    }
    std::fs::remove_dir_all(&ws)?;
    Ok(())
}

/// The engine-path count window: pid + 3 s attach for `fs_usage`, then
/// `iters` single-writer transactions at `dests` destinations, so honest
/// syncs per transaction are counted at the syscall boundary and
/// cross-checked against the published 6D+4.
fn receipt_real(dir: &Path, dests: u32, iters: u32, staging: Staging) -> std::io::Result<()> {
    let ws = dir.join(format!("hdb-real-receipt-{}", std::process::id()));
    std::fs::create_dir_all(&ws)?;
    let root = WorkspaceRoot(ws.clone());
    let table = ReservationTable::new();
    let owner = StateOwner::new(Root("hdb".into()));
    // Warmups run BEFORE the attach window so one-time setup (dirs, actor
    // spawn, intent scaffolding) never rides the counted region.
    let mut lane = lane_setup(&ws, 0, dests)?;
    for i in 0..WARMUP_ITERS {
        real_txn_once(&root, &table, &owner, &mut lane, i, staging)?;
    }
    println!("pid={} dests={dests} iters={iters}", std::process::id());
    std::io::stdout().flush()?;
    std::thread::sleep(Duration::from_secs(3));

    let c0 = counters();
    for i in 0..iters {
        real_txn_once(&root, &table, &owner, &mut lane, i, staging)?;
    }
    let c1 = counters();
    println!(
        "done: {iters} transactions x {dests} dests (staging {}), 6D+4 cross-check target = {} syncs/txn",
        if staging == Staging::Submitted {
            "submitted"
        } else {
            "settled"
        },
        6 * dests + 4
    );
    print_counter_deltas(c0, c1, u64::from(iters));
    std::fs::remove_dir_all(&ws)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// the pre-registered loss regimes (hdb-team-b-prediction.md §4b)
// ---------------------------------------------------------------------------

/// Loss A — adversarial arrival: writer 2 submits at a controlled offset
/// after writer 1's drain begins, the offset sweeps the drain window, and the
/// figure reported is the ARRIVING writer's wait, not batch throughput.
/// Pre-registered: Team B loses this cell at roughly two barriers.
fn loss_a(dir: &Path, max_offset_us: u32, step_us: u32, samples: u32) -> std::io::Result<()> {
    let root = dir.to_path_buf();
    let p1 = dir.join("loss-a-1.dat");
    let p2 = dir.join("loss-a-2.dat");
    let buf = vec![b'x'; WRITE_SIZE];
    let load_before = loadavg();
    println!(
        "loss-a: offsets 0..={max_offset_us}us step {step_us}us, {samples} samples each, load [{load_before}]"
    );
    println!(
        "{:>10} {:>12} {:>12} {:>12} {:>8}",
        "offset_us", "wait_p50_ms", "wait_p99_ms", "wait_max_ms", "samples"
    );
    let mut offset = 0u32;
    while offset <= max_offset_us {
        let mut run = LatencyRun::new();
        for _ in 0..samples {
            let mut f1 = File::create(&p1)?;
            f1.write_all(&buf)?;
            let mut f2 = File::create(&p2)?;
            f2.write_all(&buf)?;
            let pend = ::fs::durability::submit(&root, &f1)?; // drain begins
            std::thread::sleep(Duration::from_micros(u64::from(offset)));
            let t = Instant::now();
            ::fs::durability::settle(&root, &f2)?; // the arriving writer
            run.record(t.elapsed());
            pend.wait()
                .map_err(|e| std::io::Error::other(format!("w1 settle: {e}")))?;
        }
        let d = run.dist();
        println!(
            "{offset:>10} {:>12.3} {:>12.3} {:>12.3} {:>8}",
            d.p50_us / 1000.0,
            d.p99_us / 1000.0,
            d.max_us / 1000.0,
            run.samples()
        );
        offset += step_us.max(1);
    }
    println!("load_after [{}]", loadavg());
    let _ = std::fs::remove_file(&p1);
    let _ = std::fs::remove_file(&p2);
    Ok(())
}

/// Loss B — head-of-line: several D=1 writers, first ALONE, then MIXED into
/// batches with one writer settling `big_d` files per round. The reported
/// delta between the small writers' waits in the two phases is the
/// head-of-line cost. Group sizes ride the settle receipts as evidence that
/// batching actually happened.
fn loss_b(dir: &Path, big_d: u32, small_writers: u32, iters: u32) -> std::io::Result<()> {
    let root = dir.to_path_buf();
    let load_before = loadavg();
    println!(
        "loss-b: {small_writers} small writers x {iters} iters, big writer D={big_d}, load [{load_before}]"
    );
    for mixed in [false, true] {
        let stop = std::sync::atomic::AtomicBool::new(false);
        let start = Barrier::new(small_writers as usize + usize::from(mixed));
        let mut runs: Vec<(LatencyRun, u64, u64)> = Vec::new();
        std::thread::scope(|scope| -> std::io::Result<()> {
            let mut handles = Vec::new();
            for w in 0..small_writers {
                let (root, start) = (&root, &start);
                handles.push(
                    scope.spawn(move || -> std::io::Result<(LatencyRun, u64, u64)> {
                        let path = dir.join(format!("loss-b-small-{w}.dat"));
                        let buf = vec![b'x'; WRITE_SIZE];
                        let mut run = LatencyRun::new();
                        let (mut group_sum, mut group_n) = (0u64, 0u64);
                        for _ in 0..WARMUP_ITERS {
                            let mut f = File::create(&path)?;
                            f.write_all(&buf)?;
                            ::fs::durability::settle(root, &f)?;
                        }
                        start.wait();
                        for _ in 0..iters {
                            let mut f = File::create(&path)?;
                            f.write_all(&buf)?;
                            let t = Instant::now();
                            let rec = ::fs::durability::settle(root, &f)?;
                            run.record(t.elapsed());
                            group_sum += u64::try_from(rec.group_len).unwrap_or(u64::MAX);
                            group_n += 1;
                        }
                        Ok((run, group_sum, group_n))
                    }),
                );
            }
            if mixed {
                let (root, start, stop) = (&root, &start, &stop);
                scope.spawn(move || -> std::io::Result<()> {
                    let buf = vec![b'x'; WRITE_SIZE];
                    let paths: Vec<_> = (0..big_d)
                        .map(|d| dir.join(format!("loss-b-big-{d}.dat")))
                        .collect();
                    start.wait();
                    while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                        let mut pend = Vec::with_capacity(paths.len());
                        for p in &paths {
                            let mut f = File::create(p)?;
                            f.write_all(&buf)?;
                            pend.push(::fs::durability::submit(root, &f)?);
                        }
                        for pd in pend {
                            pd.wait()?;
                        }
                    }
                    Ok(())
                });
            }
            for h in handles {
                runs.push(h.join().expect("small writer panicked")?);
            }
            stop.store(true, std::sync::atomic::Ordering::Relaxed);
            Ok(())
        })?;
        // LatencyRun carries no merge, so per-writer runs stay separate:
        // report the median writer's p50 and the WORST writer's p99 — the
        // head-of-line question is about the worst-treated small writer.
        let (mut gsum, mut gn) = (0u64, 0u64);
        for (_, gs, n) in &runs {
            gsum += gs;
            gn += n;
        }
        let mut p50s: Vec<f64> = runs.iter().map(|(r, _, _)| r.percentile_us(0.50)).collect();
        let mut p99s: Vec<f64> = runs.iter().map(|(r, _, _)| r.percentile_us(0.99)).collect();
        p50s.sort_by(f64::total_cmp);
        p99s.sort_by(f64::total_cmp);
        println!(
            "  phase={} writer-median p50_ms={:.3} worst-writer p99_ms={:.3} mean_group_len_x1000={}",
            if mixed { "mixed" } else { "alone" },
            p50s.get(p50s.len() / 2).copied().unwrap_or(0.0) / 1000.0,
            p99s.last().copied().unwrap_or(0.0) / 1000.0,
            (gsum * 1000).checked_div(gn).unwrap_or(0),
        );
    }
    println!("load_after [{}]", loadavg());
    Ok(())
}

// ---------------------------------------------------------------------------
// sweep-real — round-robin interleaved writer-count sweep (ruling 17)
// ---------------------------------------------------------------------------

/// Per-level accumulator for the interleaved sweep: the level's lanes stay
/// warm across rounds and every round appends into the same runs, so device
/// drift over the sweep spreads across ALL levels instead of being reported
/// as a concurrency effect.
struct Level {
    writers: u32,
    lanes: Vec<Lane>,
    stage: LatencyRun,
    durability: LatencyRun,
    metadata: LatencyRun,
    apply: LatencyRun,
    wall: LatencyRun,
    honest_ops: u64,
    object_syncs: u64,
    actor_barriers: u64,
    txns: u64,
}

/// One chunk: this level's writers each run `iters` transactions.
fn sweep_chunk(
    root: &WorkspaceRoot,
    table: &Arc<ReservationTable>,
    owner: &StateOwner,
    level: &mut Level,
    iters: u32,
    staging: Staging,
    round: u32,
) -> std::io::Result<()> {
    let c0 = counters();
    let start = Barrier::new(level.writers as usize);
    let mut per_writer: Vec<Vec<RealLatency>> = Vec::new();
    std::thread::scope(|scope| -> std::io::Result<()> {
        let mut handles = Vec::new();
        for lane in &mut level.lanes {
            let (root, table, owner, start) = (root, table, owner, &start);
            handles.push(scope.spawn(move || -> std::io::Result<Vec<RealLatency>> {
                start.wait();
                let mut out = Vec::with_capacity(iters as usize);
                for i in 0..iters {
                    out.push(real_txn_once(
                        root,
                        table,
                        owner,
                        lane,
                        round * iters + i,
                        staging,
                    )?);
                }
                Ok(out)
            }));
        }
        for h in handles {
            per_writer.push(h.join().expect("sweep writer panicked")?);
        }
        Ok(())
    })?;
    let c1 = counters();
    for txn in per_writer.iter().flatten() {
        for st in &txn.stage {
            level.stage.record(*st);
        }
        level.durability.record(txn.durability);
        level.metadata.record(txn.metadata);
        level.apply.record(txn.apply);
        level.wall.record(txn.wall);
    }
    level.honest_ops += c1.honest_ops - c0.honest_ops;
    level.object_syncs += c1.object_syncs - c0.object_syncs;
    level.actor_barriers += c1.actor_barriers - c0.actor_barriers;
    level.txns += u64::from(level.writers) * u64::from(iters);
    Ok(())
}

/// The writer-count sweep of record: `rounds` passes, each visiting every
/// level once (round-robin), `iters_per_visit` txns per writer per visit.
/// Levels are fixed and pre-declared; nothing is re-run after results are
/// seen (rulings 17 and the conduct standard).
fn sweep_real(
    dir: &Path,
    dests: u32,
    rounds: u32,
    iters_per_visit: u32,
    levels_arg: &str,
    staging: Staging,
) -> std::io::Result<()> {
    let ws = dir.join(format!("hdb-sweep-{}", std::process::id()));
    std::fs::create_dir_all(&ws)?;
    let root = WorkspaceRoot(ws.clone());
    let table = ReservationTable::new();
    let owner = StateOwner::new(Root("hdb".into()));
    let load_before = loadavg();

    let mut levels: Vec<Level> = Vec::new();
    for part in levels_arg.split(',') {
        let writers: u32 = part
            .parse()
            .map_err(|_| std::io::Error::other(format!("bad level: {part}")))?;
        let mut lanes = Vec::new();
        for w in 0..writers {
            // Disjoint namespaces per (level, writer): level prefix in the id.
            lanes.push(lane_setup(&ws, writers * 1000 + w, dests)?);
        }
        levels.push(Level {
            writers,
            lanes,
            stage: LatencyRun::new(),
            durability: LatencyRun::new(),
            metadata: LatencyRun::new(),
            apply: LatencyRun::new(),
            wall: LatencyRun::new(),
            honest_ops: 0,
            object_syncs: 0,
            actor_barriers: 0,
            txns: 0,
        });
    }
    // Warm every lane once, before any counted chunk.
    for level in &mut levels {
        for lane in &mut level.lanes {
            for i in 0..WARMUP_ITERS {
                real_txn_once(&root, &table, &owner, lane, i, staging)?;
            }
        }
    }
    for round in 0..rounds {
        for level in &mut levels {
            sweep_chunk(
                &root,
                &table,
                &owner,
                level,
                iters_per_visit,
                staging,
                round,
            )?;
        }
    }
    let load_after = loadavg();

    println!(
        "sweep-real dests={dests} rounds={rounds} iters_per_visit={iters_per_visit} staging={} interleaved=round-robin load_before=[{load_before}] load_after=[{load_after}]",
        if staging == Staging::Submitted {
            "submitted"
        } else {
            "settled"
        },
    );
    for level in &levels {
        println!("level writers={} txns={}", level.writers, level.txns);
        for (name, run) in [
            ("stage_sync (per barrier)", &level.stage),
            ("durability (decide)", &level.durability),
            ("metadata (rename_all)", &level.metadata),
            ("owner_apply", &level.apply),
            ("txn_wall", &level.wall),
        ] {
            let d = run.dist();
            println!(
                "  {name:<26} p50_ms={:.3} p99_ms={:.3} max_ms={:.3} samples={}",
                d.p50_us / 1000.0,
                d.p99_us / 1000.0,
                d.max_us / 1000.0,
                run.samples()
            );
        }
        let t = level.txns.max(1);
        println!(
            "  counters/txn x1000: honest_ops={} object_syncs={} actor_barriers={}",
            level.honest_ops * 1000 / t,
            level.object_syncs * 1000 / t,
            level.actor_barriers * 1000 / t,
        );
    }
    std::fs::remove_dir_all(&ws)?;
    Ok(())
}
