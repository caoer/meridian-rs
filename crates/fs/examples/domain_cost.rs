//! What a currency pass costs on a real root, byte fold vs leaf memo — and
//! what the fingerprint-only doors and the cold start cost through it.
//!
//! The daemon re-derives the corpus root far more often than the corpus
//! changes, so this measures the ways of doing it over the SAME tree:
//!
//! - **A** [`fs::domain_snapshot`], which reads and folds every domain byte;
//! - **B** [`fs::DomainCache::root`], which `stat`s every member and reads
//!   only what moved (the resident memo the daemon serves from);
//! - **C** [`fs::domain_fingerprint_memoized`], the same stat-keyed answer
//!   through a caller-held [`fs::digestmemo::DigestMemo`] — the drawer memo
//!   the CLI doors (`mrd sql`'s `live` sample) ride, cold then warm;
//! - **D** the daemon's cold start, both ways: the floor pass over an empty
//!   memo followed by the snapshot for the bytes (two full reads — the
//!   shape before merkle-spec §6.5's seed), against
//!   [`fs::DomainCache::cold_snapshot`] followed by the stat floor.
//!
//! ```text
//! cargo run --release -p fs --example domain_cost -- <root> [passes]
//! ```
use std::time::Instant;

fn main() {
    let mut args = std::env::args().skip(1);
    let root_arg = args.next().expect("usage: domain_cost <root> [passes]");
    let passes: usize = args.next().map_or(3, |n| n.parse().expect("passes"));
    let root = fs::WorkspaceRoot(std::path::PathBuf::from(&root_arg));

    let domain = fs::domain::Domain::load(&root).expect("domain");
    let rels = fs::hash_domain(&root, &domain).expect("walk");
    let bytes: u64 = rels
        .iter()
        .map(|rel| std::fs::symlink_metadata(root.0.join(rel)).map_or(0, |m| m.len()))
        .sum();
    println!("root={root_arg}\nmembers={} bytes={bytes}", rels.len());

    // Arm A — the byte fold: reads and hashes the whole domain every pass.
    for pass in 0..passes {
        let t = Instant::now();
        let (_files, folded) = fs::domain_snapshot(&root).expect("snapshot");
        println!(
            "byte_fold        pass={pass} {:>9.1?}  root={}",
            t.elapsed(),
            folded.0
        );
    }

    // Arm B — the leaf memo: pass 0 is cold (reads everything), the rest are
    // the steady state the daemon actually lives in.
    let mut cache = fs::DomainCache::new();
    for pass in 0..=passes {
        let t = Instant::now();
        let folded = cache.root(&root).expect("memo fold");
        println!(
            "memo_fold        pass={pass} {:>9.1?}  reads={:<6} root={}",
            t.elapsed(),
            cache.leaves_read(),
            folded.0
        );
    }

    // Arm C — the drawer memo's fingerprint-only door: pass 0 cold (every
    // member a miss), the rest warm (every member a hit, no byte read).
    let mut memo = fs::digestmemo::DigestMemo::new();
    for pass in 0..=passes {
        let t = Instant::now();
        let folded = fs::domain_fingerprint_memoized(&root, &mut memo).expect("memo fingerprint");
        println!(
            "memo_fingerprint pass={pass} {:>9.1?}  misses={:<6} hits={:<6} root={}",
            t.elapsed(),
            memo.misses(),
            memo.hits(),
            folded.0
        );
    }

    // Arm D — the cold start. Before: currency floor over an empty memo, then
    // the snapshot for the bytes. After: the seed, then the currency floor it
    // leaves behind.
    {
        let mut before = fs::DomainCache::new();
        let t = Instant::now();
        let floor = before.root(&root).expect("floor pass");
        let t_floor = t.elapsed();
        let (_files, _leaves, folded) =
            fs::domain_snapshot_with_leaves(&root).expect("snapshot with leaves");
        let t_total = t.elapsed();
        assert_eq!(floor, folded);
        println!(
            "cold_start_before  {t_total:>9.1?}  (floor {t_floor:.1?} + snapshot {:.1?})  reads={}  root={}",
            t_total.saturating_sub(t_floor),
            before.leaves_read(),
            folded.0
        );
    }
    {
        let mut after = fs::DomainCache::new();
        let t = Instant::now();
        let (_files, _leaves, seeded) = after.cold_snapshot(&root).expect("seed");
        let t_seed = t.elapsed();
        let floor = after.root(&root).expect("post-seed floor");
        let t_total = t.elapsed();
        assert_eq!(floor, seeded);
        println!(
            "cold_start_after   {t_total:>9.1?}  (seed {t_seed:.1?} + floor {:.1?})  reads={}  root={}",
            t_total.saturating_sub(t_seed),
            after.leaves_read(),
            seeded.0
        );
    }
}
