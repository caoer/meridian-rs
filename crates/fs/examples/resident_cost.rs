//! The resident-tree cost probe (card `resident-tree`, merged plan §6
//! step 3): the lane numbers and the MEASURED memory that replaces the
//! plan's ~10 MB estimate, on a real root.
//!
//! Reads the corpus; writes nothing to disk (the overlay lanes mutate the
//! in-memory cache only). Lanes, named as the fable bench named them:
//!
//! - cold observe (first `DomainCache::root` — walk + read + fold)
//! - warm currency pass (stat sweep, served-root cache hit)
//! - build-from-leaves: the resident tree from already-known leaves, split
//!   into trie construction and the first full refold (the 12 ms lane class)
//! - lane D: one leaf update + ancestor-chain refold to a fresh law-2
//!   fingerprint (the 14 µs class), with the vertex-hash count beside it
//! - lane C interim serve: the law-1 flat fold `overlay_root` pays when the
//!   root advances (merged plan §6 step 3's stated interim cost)
//!
//! Byte identity is asserted, not printed: the served root must equal
//! `model::merkle_root_of_leaves` over the same leaves (the fable-bench
//! assertion, re-run against this implementation).
//!
//! ```text
//! cargo run --release -p fs --example resident_cost -- <root> [reps]
//! ```

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use fs::resident::ResidentTree;

/// Counting wrapper over the system allocator: live bytes and peak, so the
/// probe MEASURES resident structure memory instead of estimating it.
struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn count_up(size: usize) {
    let now = LIVE.fetch_add(size, Ordering::Relaxed) + size;
    PEAK.fetch_max(now, Ordering::Relaxed);
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count_up(layout.size());
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if new_size >= layout.size() {
            count_up(new_size - layout.size());
        } else {
            LIVE.fetch_sub(layout.size() - new_size, Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

fn live() -> usize {
    LIVE.load(Ordering::Relaxed)
}

fn med(mut xs: Vec<Duration>) -> (Duration, Duration, Duration) {
    xs.sort();
    (xs[0], xs[xs.len() / 2], xs[xs.len() - 1])
}

// A linear measurement script: each lane reads better in sequence than
// scattered across helpers whose only caller is the next line.
#[allow(clippy::too_many_lines)]
fn main() {
    let mut args = std::env::args().skip(1);
    let root_arg = args.next().expect("usage: resident_cost <root> [reps]");
    let reps: usize = args.next().map_or(200, |n| n.parse().expect("reps"));
    let root = fs::WorkspaceRoot(std::path::PathBuf::from(&root_arg));
    println!("root={root_arg} reps={reps}");

    // Cold observe: walk + read + first law-1 serve.
    let mut cache = fs::DomainCache::new();
    let before_cache = live();
    let t = Instant::now();
    let served = cache.root(&root).expect("cold observe");
    let cold = t.elapsed();
    let cache_bytes = live() - before_cache;
    let leaves = cache.leaf_digests();
    println!(
        "cold_observe   {cold:>10.1?}  members={} cache_live_bytes={cache_bytes}",
        leaves.len()
    );

    // Warm currency passes: stat sweep + served-root cache hit.
    for pass in 0..3 {
        let t = Instant::now();
        cache.root(&root).expect("warm pass");
        println!(
            "warm_currency  pass={pass} {:>10.1?}  flat_folds={}",
            t.elapsed(),
            cache.flat_folds()
        );
    }

    // The fable-bench assertion, re-run against this implementation: the
    // served root equals the engine's own fold over the same leaves.
    let version = fs::domain::Domain::load(&root).expect("domain").version();
    let refs: Vec<(&[u8], [u8; 32])> = leaves
        .iter()
        .map(|(rel, digest)| (fs::hash_name(rel), *digest))
        .collect();
    assert_eq!(
        served,
        model::merkle_root_of_leaves(&refs, version),
        "served root diverged from model::merkle_root_of_leaves"
    );
    println!("byte_identity  served == merkle_root_of_leaves  OK");

    // Build-from-leaves: trie construction, then the first full refold.
    let before_tree = live();
    let t = Instant::now();
    let mut tree = ResidentTree::new();
    for (rel, digest) in &leaves {
        tree.set_leaf(rel, *digest);
    }
    let construct = t.elapsed();
    let t = Instant::now();
    let fp = tree.fingerprint();
    let first_refold = t.elapsed();
    let tree_bytes = live() - before_tree;
    let stats = tree.stats();
    println!(
        "build_from_leaves  construct={construct:>9.1?} first_refold={first_refold:>9.1?} \
         total={:>9.1?}",
        construct + first_refold
    );
    println!(
        "tree_stats     dir_nodes={} vertex_hashes={} hashed_bytes={}",
        stats.dir_nodes, stats.vertex_hashes, stats.hashed_bytes
    );
    println!(
        "tree_memory    live_bytes={tree_bytes} bytes_per_member={} (MEASURED, counting allocator)",
        tree_bytes / leaves.len().max(1)
    );

    // Lane D: one leaf update + refold to a fresh fingerprint, at the
    // deepest member (worst ancestor chain).
    let deep = leaves
        .keys()
        .max_by_key(|rel| rel.components().count())
        .expect("nonempty corpus")
        .clone();
    let original = leaves[&deep];
    let depth = deep.components().count();
    let mut lane_d = Vec::with_capacity(reps);
    let mut fresh_fp = fp;
    let hashes_before = tree.stats().vertex_hashes;
    for i in 0..reps {
        let mut digest = [0u8; 32];
        digest[..8].copy_from_slice(&(u64::try_from(i).expect("reps fit")).to_le_bytes());
        let t = Instant::now();
        tree.set_leaf(&deep, digest);
        fresh_fp = tree.fingerprint();
        lane_d.push(t.elapsed());
    }
    let per_op = (tree.stats().vertex_hashes - hashes_before) / u64::try_from(reps).expect("fits");
    let (min, median, max) = med(lane_d);
    println!(
        "lane_d         leaf_update+refold at depth {depth}: min={min:.1?} med={median:.1?} \
         max={max:.1?} vertex_hashes/op={per_op}"
    );
    tree.set_leaf(&deep, original);
    assert_ne!(fresh_fp, fp, "the probe's updates moved the fingerprint");
    assert_eq!(
        tree.fingerprint(),
        fp,
        "restoring the leaf restores the fingerprint"
    );

    // Lane C interim serve: the law-1 flat fold overlay_root pays per
    // ADVANCE (merged plan §6 step 3), through the cache end to end.
    let mut lane_c = Vec::with_capacity(8);
    for i in 0..8u8 {
        let mut digest = [0u8; 32];
        digest[0] = i.wrapping_add(1);
        cache.overlay_leaf(&deep, digest).expect("overlay");
        let t = Instant::now();
        cache.overlay_root().expect("interim serve");
        lane_c.push(t.elapsed());
    }
    cache.overlay_leaf(&deep, original).expect("restore");
    assert_eq!(
        cache.overlay_root().expect("restored serve"),
        served,
        "restoring the leaf restores the served root"
    );
    let (min, median, max) = med(lane_c);
    println!(
        "lane_c_interim overlay_root per advance: min={min:.1?} med={median:.1?} max={max:.1?}"
    );

    // Law-2 fingerprint through the cache (already resident): one call.
    let t = Instant::now();
    let law2 = cache.law2_fingerprint();
    println!(
        "law2_serve     {:>10.1?}  fingerprint={}",
        t.elapsed(),
        blake3::Hash::from_bytes(law2).to_hex()
    );
    assert_eq!(law2, fp, "cache tree and probe tree agree");

    println!(
        "peak_live_bytes={} (process, counting allocator)",
        PEAK.load(Ordering::Relaxed)
    );
}
