//! Counting-allocator probe for merkle §6.4 document residency.
//!
//! Run only on a PRIVATE COPY of a root. Creates a private cache and, when
//! `edits` is nonzero, one temporary member in that copy. No installed daemon
//! is contacted. Uses a real in-process registry, kernel feed, primed watcher
//! and live `SubGuard`; it does not measure socket transport or SQL allocations.
//!
//! ```text
//! cargo run --release -p registry --example residency_breakdown -- ROOT CACHE [edits]
//! ```
//!
//! Requested live heap bytes exclude allocator metadata/retained pages and
//! native allocations outside Rust's global allocator. Linux RSS+swap is
//! printed separately. Ownership is measured by destruction after stopping
//! the registry/feed, with engine, memo and subscription still pinned.

use std::alloc::{GlobalAlloc, Layout, System};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

struct Counting;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn count_up(size: usize) {
    let now = LIVE.fetch_add(size, Ordering::Relaxed) + size;
    PEAK.fetch_max(now, Ordering::Relaxed);
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            count_up(layout.size());
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            count_up(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let next = unsafe { System.realloc(ptr, layout, size) };
        if !next.is_null() {
            if size >= layout.size() {
                count_up(size - layout.size());
            } else {
                LIVE.fetch_sub(layout.size() - size, Ordering::Relaxed);
            }
        }
        next
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn live() -> usize {
    LIVE.load(Ordering::Relaxed)
}

fn sample(phase: &str, started: Instant) {
    let heap = live();
    let peak = PEAK.load(Ordering::Relaxed);
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let kib = |key: &str| -> usize {
        status
            .lines()
            .find_map(|line| line.strip_prefix(key))
            .and_then(|v| v.split_whitespace().next())
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    };
    let rss = kib("VmRSS:");
    let swap = kib("VmSwap:");
    println!(
        "phase={phase} elapsed_ms={} live_bytes={heap} peak_bytes={peak} \
         rss_kib={rss} swap_kib={swap} rss_swap_kib={}",
        started.elapsed().as_millis(),
        rss + swap
    );
}

fn freed<T>(value: T) -> usize {
    let before = live();
    drop(value);
    before.checked_sub(live()).expect("drop must free bytes")
}

fn await_worker_shutdown() {
    // notify's Drop requests shutdown but does not join its worker. A quiet
    // allocation counter does not prove it exited: delayed thread cleanup
    // can otherwise be misattributed to whichever component we drop next.
    for _ in 0..400 {
        let threads = std::fs::read_dir("/proc/self/task")
            .expect("Linux thread census")
            .count();
        if threads == 1 {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("workers remain after feed shutdown; component deltas are not trustworthy");
}

// Sequential ownership accounting is intentionally kept in one place.
#[allow(clippy::too_many_lines)]
fn main() {
    let started = Instant::now();
    let mut args = std::env::args().skip(1);
    let root =
        std::fs::canonicalize(args.next().expect("ROOT (private copy)")).expect("canonical root");
    let cache = PathBuf::from(args.next().expect("CACHE (private, new directory)"));
    let edits: usize = args.next().map_or(0, |n| n.parse().expect("edit count"));
    assert!(!cache.exists(), "use a new cache directory for each run");
    assert!(!cache.starts_with(&root), "cache must be outside the root");
    let edit_rel = PathBuf::from("residency-breakdown-probe.md");
    let edit_path = root.join(&edit_rel);
    if edits > 0 {
        assert!(!edit_path.exists(), "probe member already exists");
        std::fs::write(&edit_path, "# Residency probe\n\n0\n").expect("seed probe member");
    }
    println!(
        "probe=residency_breakdown root={} edits={edits}",
        root.display()
    );
    sample("start", started);
    let mut config = registry::Config::for_cache_root(cache.clone());
    config.socket_path = cache.join("unused.sock");
    config.state_path = cache.join("registry/state.json");
    let reg = registry::in_process_registry(&config).expect("private registry");
    reg.register(&root);
    let ws = fs::WorkspaceRoot(root.clone());
    let memo = reg.domain_cache(&root);
    reg.currency_refresh(&root, Duration::from_secs(2))
        .expect("memo prime");
    assert!(
        reg.feed_stats(&root).is_some(),
        "a live kernel feed is required for this probe"
    );
    sample("memo_and_feed", started);
    let ring = reg.ring(&root);
    let sub = reg.subscribe(&root);
    let (_, first_seq) = ring.prime(&ws, &reg).expect("subscription prime");
    assert!(ring.has_subscribers());
    sample("live_subscription", started);
    let outcome = reg.warm_or_build(&root).expect("warm corpus");
    println!("warm={outcome:?} seq={first_seq}");
    sample("warm_with_subscription", started);
    for i in 1..=edits {
        std::fs::write(&edit_path, format!("# Residency probe\n\n{i}\n"))
            .expect("edit private member");
        reg.note_dirty(&root, std::slice::from_ref(&edit_rel));
        let (_, seq) = ring.prime(&ws, &reg).expect("reconcile change");
        assert_eq!(seq, first_seq + u64::try_from(i).expect("count fits"));
        reg.warm_or_build(&root).expect("rebuild mover");
        sample(&format!("edit_{i}"), started);
    }
    let engine = reg.engine_snapshot(&root).expect("engine retained");
    println!(
        "members={} raw_len_bytes={} unserved={} leaves={} seq={} live_sub={} fingerprint={}",
        engine.docs.len(),
        engine.docs.values().map(|d| d.raw.len()).sum::<usize>(),
        engine.unserved.len(),
        engine.leaves.len(),
        ring.seq(),
        ring.has_subscribers(),
        engine.at_fingerprint.0
    );
    sample("before_teardown", started);
    // Stop the feed before component deltas: background allocation churn
    // would otherwise contaminate deallocation measurements.
    let before_registry_drop = live();
    drop(reg);
    await_worker_shutdown();
    let registry_feed_bytes = before_registry_drop - live();
    let engine = Arc::try_unwrap(engine).unwrap_or_else(|_| panic!("sole engine owner"));
    let index_bytes = freed(engine.index);
    let leaves_bytes = freed(engine.leaves);
    let unserved_bytes = freed(engine.unserved);
    let docs_before = live();
    let mut raw_bytes = 0;
    let mut tree_bytes = 0;
    for (_, doc) in engine.docs {
        let model::Document { raw, root } =
            Arc::try_unwrap(doc).unwrap_or_else(|_| panic!("sole document owner"));
        raw_bytes += freed(raw);
        tree_bytes += freed(root);
    }
    let docs_total = docs_before - live();
    let docs_structure = docs_total - raw_bytes - tree_bytes;
    let memo_bytes = freed(memo);
    drop(sub);
    let watcher_ring_bytes = freed(ring);
    println!(
        "component_bytes raw={raw_bytes} parse_tree={tree_bytes} docs_structure={docs_structure} \
         name_index={index_bytes} engine_leaves={leaves_bytes} unserved={unserved_bytes} \
         memo={memo_bytes} watcher_and_ring={watcher_ring_bytes} registry_and_feed={registry_feed_bytes}"
    );
    sample("after_teardown", started);
    if edits > 0 {
        std::fs::remove_file(edit_path).expect("remove private probe member");
    }
}
