//! Measure default priming's actual flock-held work on a synthetic corpus.
//! Run only on a build host. Arguments: members, bytes/member, rounds, optional
//! maximum allocations per prime and number of members changed after capture. This is not the real-root RSS acceptance run.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

struct Counting;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static CALLS: AtomicUsize = AtomicUsize::new(0);
static REQUESTED: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            CALLS.fetch_add(1, Ordering::Relaxed);
            REQUESTED.fetch_add(layout.size(), Ordering::Relaxed);
            let live = LIVE.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(live, Ordering::Relaxed);
        }
        pointer
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            CALLS.fetch_add(1, Ordering::Relaxed);
            REQUESTED.fetch_add(layout.size(), Ordering::Relaxed);
            let live = LIVE.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(live, Ordering::Relaxed);
        }
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let next = unsafe { System.realloc(pointer, layout, size) };
        if !next.is_null() {
            CALLS.fetch_add(1, Ordering::Relaxed);
            REQUESTED.fetch_add(size, Ordering::Relaxed);
            if size >= layout.size() {
                let live =
                    LIVE.fetch_add(size - layout.size(), Ordering::Relaxed) + size - layout.size();
                PEAK.fetch_max(live, Ordering::Relaxed);
            } else {
                LIVE.fetch_sub(layout.size() - size, Ordering::Relaxed);
            }
        }
        next
    }
}
#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn argument(args: &[String], index: usize, default: usize) -> usize {
    args.get(index).map_or(default, |value| {
        value.parse().expect("numeric probe argument")
    })
}
/// Wait until this thread is the process's only task. The capture's scoped
/// readers are joined when it returns, but the kernel releases a joined
/// thread's task a moment after the join; a measurement that starts before
/// then counts nothing of theirs, yet cannot prove it.
#[cfg(target_os = "linux")]
fn await_quiescence() {
    let deadline = Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let tasks = std::fs::read_dir("/proc/self/task").unwrap().count();
        if tasks == 1 {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "allocation measurement requires quiescence: {tasks} tasks after 10 s"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

fn main() {
    println!(
        "watcher priming: synthetic corpus; requested Rust allocations; actual write flock and state mutex held"
    );
    let args: Vec<_> = std::env::args().skip(1).collect();
    let members = argument(&args, 0, 20_000);
    let bytes = argument(&args, 1, 4096);
    let rounds = argument(&args, 2, 5);
    let max_allocations = argument(&args, 3, usize::MAX);
    let movers = argument(&args, 4, 0);
    assert!(movers <= members);
    assert!(members > 0 && bytes >= 16 && rounds > 0);
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::WorkspaceRoot(tmp.path().to_path_buf());
    let mut raw = vec![b'x'; bytes];
    raw[..9].copy_from_slice(b"# Header\n");
    for i in 0..members {
        std::fs::write(root.0.join(format!("member-{i:08}.md")), &raw).unwrap();
    }
    drop(raw);
    let (_, leaves, folded) = fs::domain_snapshot_with_leaves(&root).unwrap();
    let mut leaves: std::collections::BTreeMap<String, [u8; 32]> = leaves
        .into_iter()
        .map(|(path, digest)| (path.to_str().unwrap().to_owned(), digest))
        .collect();
    assert_eq!(leaves.len(), members);
    let mut observed = wire::Root(folded.0);
    let mut failed = false;
    for round in 0..rounds {
        let start = Instant::now();
        let snapshot = wire_serve::watch::PrimeSnapshot::take(&root).unwrap();
        let snapshot_us = start.elapsed().as_micros();
        if movers != 0 {
            let mut replacement = vec![b'x'; bytes];
            let label = format!("# Round {round}\n");
            replacement[..label.len()].copy_from_slice(label.as_bytes());
            for i in 0..movers {
                std::fs::write(root.0.join(format!("member-{i:08}.md")), &replacement).unwrap();
            }
            let (_, fresh, folded) = fs::domain_snapshot_with_leaves(&root).unwrap();
            leaves = fresh
                .into_iter()
                .map(|(path, digest)| (path.to_str().unwrap().to_owned(), digest))
                .collect();
            observed = wire::Root(folded.0);
        }
        #[cfg(target_os = "linux")]
        await_quiescence();
        let _flock = fs::WriteLock::acquire(&root).unwrap();
        let state = Mutex::new(wire_serve::watch::WatchState::new(&root));
        let mut watch = state.lock().unwrap();
        let calls = CALLS.load(Ordering::Relaxed);
        let requested = REQUESTED.load(Ordering::Relaxed);
        let live = LIVE.load(Ordering::Relaxed);
        PEAK.store(live, Ordering::Relaxed);
        let start = Instant::now();
        let primed =
            wire_serve::watch::prime(&root, &mut watch, snapshot, &leaves, &observed).unwrap();
        let prime_us = start.elapsed().as_micros();
        let prime_calls = CALLS.load(Ordering::Relaxed) - calls;
        let prime_requested = REQUESTED.load(Ordering::Relaxed) - requested;
        let peak_delta = PEAK.load(Ordering::Relaxed).saturating_sub(live);
        assert!(primed && watch.root() == Some(&observed));
        println!(
            "prime-cost round={round} members={members} bytes_per_member={bytes} movers={movers} snapshot_us={snapshot_us} prime_us={prime_us} prime_allocs={prime_calls} prime_requested_bytes={prime_requested} prime_peak_delta={peak_delta}"
        );
        failed |= prime_calls > max_allocations;
    }
    drop(tmp);
    if failed {
        eprintln!(
            "prime-budget exceeded: more than {max_allocations} allocations under the write flock"
        );
        std::process::exit(3);
    }
}
