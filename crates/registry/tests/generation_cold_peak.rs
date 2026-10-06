//! A cold build under the residency controller holds one member's bytes and
//! tree at a time (merkle-spec §6.8), never the whole corpus. Measured with a
//! counting allocator; this binary holds one test, so nothing else allocates
//! while it measures.

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use registry::raw_store::RawStore;
use registry::residency::catalog::{Slots, update};
use registry::residency::controller::Controller;

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
fn a_cold_build_holds_one_member_at_a_time() {
    const MEMBERS: usize = 64;
    let body = "a line with [[a link]] and **strong** text\n".repeat(256);
    let member = |n: usize| format!("# Member {n}\n\n{body}");
    let dir = tempfile::tempdir().unwrap();
    for n in 0..MEMBERS {
        std::fs::write(dir.path().join(format!("m{n:03}.md")), member(n)).unwrap();
    }
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    let (files, leaves, fingerprint) = fs::domain_snapshot_with_leaves(&root).unwrap();
    drop(files);
    let backing = tempfile::tempdir().unwrap();
    let store = RawStore::new(backing.path()).unwrap();
    let controller = Controller::new(0);
    let none = BTreeMap::new();
    let unserved = BTreeMap::new();
    let index = Arc::new(model::CorpusIndex::new());
    let empty = Slots::default();
    let prior = fs::CorpusPrior {
        docs: &empty,
        unserved: &unserved,
        leaves: &none,
        index: Some(&index),
    };
    let moves = fs::leaf_delta(&none, leaves.iter().map(|(rel, digest)| (rel, *digest)));

    // What one member costs the eager build: its bytes, its tree, its rows.
    let (one, single) =
        peak_of(|| fs::build_corpus(vec![("m.md".to_owned(), member(0).into_bytes())]));
    drop(one);
    let (built, peak) = peak_of(|| {
        update(
            &root,
            &prior,
            &moves,
            Some(&fingerprint),
            &controller,
            &store,
        )
        .unwrap()
    });

    assert_eq!(built.parsed, MEMBERS);
    assert_eq!(built.root, fingerprint);
    assert!(
        peak < 4 * single,
        "a cold build of {MEMBERS} members peaked at {peak} bytes; one member's eager build \
         costs {single}, so the build held more than a few members at once"
    );
}
