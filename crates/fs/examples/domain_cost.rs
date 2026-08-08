//! What a currency pass costs on a real root, byte fold vs leaf memo.
//!
//! The daemon re-derives the corpus root far more often than the corpus
//! changes, so this measures the two ways of doing it over the SAME tree:
//! [`fs::domain_snapshot`], which reads and folds every domain byte, and
//! [`fs::DomainCache`], which `stat`s every member and reads only what moved.
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
            "byte_fold  pass={pass} {:>9.1?}  root={}",
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
            "memo_fold  pass={pass} {:>9.1?}  reads={:<6} root={}",
            t.elapsed(),
            cache.leaves_read(),
            folded.0
        );
    }
}
