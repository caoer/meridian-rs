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
//! Subcommands:
//!   baseline <dir> [iters]            three sync classes, one 4 KiB file
//!   bench <dir> <mode> <writers> <iters>   one matrix cell
//!   receipt <dir> <mode> <iters>      pid + attach window for `fs_usage`
//!
//! Percentiles ride `perfsuite::measure::LatencyRun` — the one instrument.

use std::fs::File;
use std::io::Write as _;
use std::os::fd::AsRawFd as _;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Barrier;
use std::time::{Duration, Instant};

use perfsuite::measure::LatencyRun;

const WRITE_SIZE: usize = 4096;
const WARMUP_ITERS: u32 = 20;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let strs: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match strs.as_slice() {
        ["baseline", dir] => baseline(Path::new(dir), 1000),
        ["baseline", dir, iters] => baseline(Path::new(dir), parse(iters)),
        ["bench", dir, mode, writers, iters] => {
            bench(Path::new(dir), mode, parse(writers), parse(iters))
        }
        ["receipt", dir, mode, iters] => receipt(Path::new(dir), mode, parse(iters)),
        _ => {
            eprintln!(
                "usage: hdb-durability baseline <dir> [iters]\n       \
                 hdb-durability bench <dir> today|naive|actor <writers> <iters>\n       \
                 hdb-durability receipt <dir> today|naive|actor <iters>"
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

/// Re-measure the pinned probe on THIS host: 4 KiB overwrite then one sync,
/// three classes — plain `fsync(2)` (drive-cache), `fs::honest_sync`
/// (`F_FULLFSYNC` on macOS), and std `File::sync_all` (empirically the honest
/// class on macOS too — the receipt run proves it at the kernel).
fn baseline(dir: &Path, iters: u32) -> std::io::Result<()> {
    let path = dir.join(format!("hdb-baseline-{}.dat", std::process::id()));
    let mut file = File::create(&path)?;
    let buf = vec![b'x'; WRITE_SIZE];

    let classes: [(&str, ContentSync); 3] = [
        ("fsync(2) direct", plain_fsync),
        ("fs::honest_sync", ::fs::honest_sync),
        ("File::sync_all", sync_all_file),
    ];

    println!(
        "baseline: {iters} iters x {WRITE_SIZE} B overwrite+sync, file {}",
        path.display()
    );
    println!(
        "{:<16} {:>10} {:>10} {:>10} {:>10} {:>8}",
        "class", "p50_ms", "p95_ms", "p99_ms", "max_ms", "samples"
    );
    for (name, sync) in classes {
        let mut run = LatencyRun::new();
        for _ in 0..WARMUP_ITERS {
            file.write_all(&buf)?;
            sync(&file)?;
        }
        for _ in 0..iters {
            file.write_all(&buf)?;
            let t = Instant::now();
            sync(&file)?;
            run.record(t.elapsed());
        }
        let d = run.dist();
        println!(
            "{name:<16} {:>10.3} {:>10.3} {:>10.3} {:>10.3} {:>8}",
            d.p50_us / 1000.0,
            run.percentile_us(0.95) / 1000.0,
            d.p99_us / 1000.0,
            d.max_us / 1000.0,
            run.samples()
        );
    }
    std::fs::remove_file(&path)?;
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
type DirSync = fn(&Path) -> std::io::Result<()>;

fn commit_once(
    dst: &Path,
    tmp: &Path,
    buf: &[u8],
    content_sync: ContentSync,
    dir_sync: DirSync,
) -> std::io::Result<CommitLatency> {
    let mut f = File::create(tmp)?;
    f.write_all(buf)?;
    let t0 = Instant::now();
    content_sync(&f)?;
    let fsync = t0.elapsed();
    drop(f);

    let parent = dst.parent().unwrap_or_else(|| Path::new("."));
    let t1 = Instant::now();
    std::fs::rename(tmp, dst)?;
    dir_sync(parent)?;
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
    if mode == Mode::Actor {
        return Err(std::io::Error::other(
            "actor mode: the crates/fs durability actor has not landed on hd-team-b yet; \
             this cell is wired when it does",
        ));
    }

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
                let (content, dirsync) = sync_fns(mode);
                for _ in 0..WARMUP_ITERS {
                    commit_once(&dst, &tmp, &buf, content, dirsync)?;
                }
                start.wait();
                let mut out = Vec::with_capacity(iters as usize);
                for _ in 0..iters {
                    out.push(commit_once(&dst, &tmp, &buf, content, dirsync)?);
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

/// The two sync placements for an inline mode. `Actor` never reaches here.
fn sync_fns(mode: Mode) -> (ContentSync, DirSync) {
    match mode {
        Mode::Today => (sync_all_file, sync_all_path),
        Mode::Naive => (::fs::honest_sync, ::fs::honest_sync_path),
        Mode::Actor => unreachable!("actor cells are wired when the actor lands"),
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
    if mode == Mode::Actor {
        return Err(std::io::Error::other(
            "actor mode: the crates/fs durability actor has not landed on hd-team-b yet",
        ));
    }
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
    let (content, dirsync) = sync_fns(mode);
    for _ in 0..iters {
        commit_once(&dst, &tmp, &buf, content, dirsync)?;
    }
    println!(
        "done: {iters} commits, expected honest-class syncs on macOS = {iters} (content) + {iters} (dir)"
    );
    let _ = std::fs::remove_file(&dst);
    Ok(())
}
