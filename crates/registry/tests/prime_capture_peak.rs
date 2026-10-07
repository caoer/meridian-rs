//! A watcher primed under a host retention provider captures the hash
//! domain one member at a time (merkle-spec §6.7, the watch plane), never the
//! whole corpus's bytes. Measured with a counting allocator; this binary
//! holds one test, so nothing else allocates while it measures.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use registry::raw_store::RawStore;
use registry::residency::controller::Controller;
use registry::residency::retention::Retention;
use wire_serve::ring::RootRing;
use wire_serve::watch::{PrimeSnapshot, WatchState, reconcile};

struct Counting;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn grew(size: usize) {
    let live = LIVE.fetch_add(size, Ordering::Relaxed) + size;
    PEAK.fetch_max(live, Ordering::Relaxed);
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            grew(layout.size());
        }
        pointer
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            grew(layout.size());
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
            if size >= layout.size() {
                grew(size - layout.size());
            } else {
                LIVE.fetch_sub(layout.size() - size, Ordering::Relaxed);
            }
        }
        next
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// The peak live heap `work` adds above what was live when it started.
fn peak_of<T>(work: impl FnOnce() -> T) -> (T, usize) {
    let base = LIVE.load(Ordering::Relaxed);
    PEAK.store(base, Ordering::Relaxed);
    let out = work();
    (out, PEAK.load(Ordering::Relaxed) - base)
}

#[test]
fn a_retained_prime_capture_holds_one_member_at_a_time() {
    const MEMBERS: usize = 64;
    let body = "a line with [[a link]] and **strong** text\n".repeat(256);
    let member = |n: usize| format!("# Member {n}\n\n{body}");
    let dir = tempfile::tempdir().unwrap();
    for n in 0..MEMBERS {
        std::fs::write(dir.path().join(format!("m{n:03}.md")), member(n)).unwrap();
    }
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    let (_, disk_root) = wire_serve::domain_snapshot(&root).unwrap();
    let backing = tempfile::tempdir().unwrap();
    let store = Arc::new(RawStore::new(backing.path()).unwrap());
    let retention: Arc<dyn model::bytes::ByteRetention> =
        Arc::new(Retention::new(Controller::new(0), store));
    let raw: usize = (0..MEMBERS).map(|n| member(n).len()).sum();
    // What one member costs to read: its bytes, read whole.
    let single = member(0).len();
    let bound = 2 * single + raw / 4;

    // The unlocked half of priming.
    let (snapshot, captured) =
        peak_of(|| PrimeSnapshot::take_with_retention(&root, Arc::clone(&retention)));
    drop(snapshot.unwrap());
    // The full snapshot a racing re-read falls back to, under the flock.
    let mut watch = WatchState::with_retention(&root, Arc::clone(&retention));
    let mut ring = RootRing::new();
    let (primed, fallback) = peak_of(|| reconcile(&root, &mut ring, &mut watch));
    assert!(primed.unwrap().is_none(), "priming tells nothing");
    assert_eq!(
        watch.root(),
        Some(&disk_root),
        "the streamed fold is the snapshot's"
    );

    // A capture that kept each member's bytes until the walk ended would add
    // at least their sum.
    for (step, peak) in [("capture", captured), ("fallback snapshot", fallback)] {
        assert!(
            peak < bound,
            "the {step} of {MEMBERS} members ({raw} bytes of text) peaked at {peak} bytes; one \
             member is {single} bytes, so it kept members it had retained"
        );
    }
}
