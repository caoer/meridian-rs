//! What one bash-step observation cycle costs, drawer lane vs resident lane
//! (card run-observation-unification; engine-warm-cost design § 5).
//!
//! One cycle is the run plane's observation trio around a bash exec window:
//! the pre-flock leaves fold, the bracket open, the bracket close. The drawer
//! lane pays it the way the CLI (and, before the unification, the daemon)
//! does — three fresh walks, byte reads amortised by a per-workspace drawer
//! memo loaded and saved per dispatch. The resident lane pays it the way the
//! daemon door now does — listings from the resident dir memo, digests from
//! the resident leaf memo, no drawer I/O.
//!
//! Hermetic by construction: a synthetic corpus in a fresh tempdir, the
//! drawer file beside it, no daemon, no socket, no cache-home resolution.
//!
//! ```text
//! cargo run --release -p fs --example run_observation_cost -- [docs] [cycles] [churn]
//! ```
use std::io::Write as _;
use std::path::Path;
use std::time::{Duration, Instant};

use fs::digestmemo::DigestMemo;
use fs::guard::StepGuard;
use fs::{DomainCache, WorkspaceRoot};

fn main() {
    let mut args = std::env::args().skip(1);
    let docs: usize = args.next().map_or(29_500, |n| n.parse().expect("docs"));
    let cycles: usize = args.next().map_or(10, |n| n.parse().expect("cycles"));
    let churn: usize = args.next().map_or(5, |n| n.parse().expect("churn"));

    let tmp = tempfile::tempdir().expect("tempdir");
    let ws = tmp.path().join("ws");
    let root = build_corpus(&ws, docs);
    let drawer = tmp.path().join("run-digests.v1");
    println!("docs={docs} cycles={cycles} churn={churn}/cycle");

    // ---- Drawer lane (the pre-unification daemon cost, still the CLI's) ----
    let mut drawer_walls = Vec::new();
    for cycle in 0..=cycles {
        mutate(&root, cycle, churn);
        let t = Instant::now();
        let mut memo = match std::fs::read(&drawer) {
            Ok(bytes) => DigestMemo::from_bytes(&bytes),
            Err(_) => DigestMemo::new(),
        };
        let leaves = fs::domain_leaves_memoized(&root, &mut memo).expect("leaves");
        let _ = leaves.root();
        let open = StepGuard::open_memoized(&root, &mut memo).expect("open");
        let _ = open.pre_root();
        let _ = open.close_memoized(&[], &mut memo).expect("close");
        std::fs::write(&drawer, memo.to_bytes()).expect("drawer save");
        let wall = t.elapsed();
        let label = if cycle == 0 { "cold" } else { "warm" };
        println!("drawer   cycle={cycle:<2} {label} {wall:>9.1?}");
        if cycle > 0 {
            drawer_walls.push(wall);
        }
    }

    // ---- Resident lane (the daemon door after the unification) ----
    let mut cache = DomainCache::new();
    let mut resident_walls = Vec::new();
    for cycle in 0..=cycles {
        mutate(&root, 1000 + cycle, churn);
        let (l0, r0) = (cache.listings(), cache.leaves_read());
        let t = Instant::now();
        let leaves = cache.domain_leaves(&root).expect("leaves");
        let _ = leaves.root();
        let open = StepGuard::open_cached(&root, &mut cache).expect("open");
        let _ = open.pre_root();
        let _ = open.close_cached(&[], &mut cache).expect("close");
        let wall = t.elapsed();
        let label = if cycle == 0 { "cold" } else { "warm" };
        println!(
            "resident cycle={cycle:<2} {label} {wall:>9.1?}  listings+={} reads+={}",
            cache.listings() - l0,
            cache.leaves_read() - r0,
        );
        if cycle > 0 {
            resident_walls.push(wall);
        }
    }

    println!(
        "median warm cycle: drawer={:?} resident={:?}",
        median(&mut drawer_walls),
        median(&mut resident_walls),
    );
}

/// A nested synthetic corpus: `docs` markdown members spread over ~docs/100
/// directories, ~2 KB each — the shape that prices listings and stats, which
/// is what the lanes differ on (steady-state byte reads are movers-only in
/// both).
fn build_corpus(ws: &Path, docs: usize) -> WorkspaceRoot {
    let body = "lorem ipsum dolor sit amet, consectetur adipiscing elit\n".repeat(35);
    for i in 0..docs {
        let dir = ws.join(format!("d{:03}", i % (docs / 100).max(1)));
        std::fs::create_dir_all(&dir).expect("dir");
        let mut f = std::fs::File::create(dir.join(format!("n{i:05}.md"))).expect("file");
        writeln!(f, "# Doc {i}\n\n{body}").expect("write");
    }
    WorkspaceRoot(std::fs::canonicalize(ws).expect("canonicalize"))
}

/// The churn between observations: rewrite `churn` members with new content —
/// the "typically a handful of files" premise the warm design measures under.
fn mutate(root: &WorkspaceRoot, cycle: usize, churn: usize) {
    for k in 0..churn {
        let dir = root.0.join(format!("d{:03}", (cycle * 7 + k) % 100));
        if !dir.exists() {
            continue;
        }
        let target = dir.join(format!("n{:05}.md", cycle % 10));
        let _ = std::fs::write(target, format!("# churned {cycle}-{k}\n"));
    }
}

fn median(walls: &mut [Duration]) -> Duration {
    walls.sort();
    walls[walls.len() / 2]
}
