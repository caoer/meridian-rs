//! Quiescent calibration of controller charges against requested live heap.
//! Run on a build host; no installed service or workspace is used.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Counting;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            let live = LIVE.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(live, Ordering::Relaxed);
        }
        pointer
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
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

use registry::raw_store::RawStore;
use registry::residency::{Access, controller::Controller};

fn check(label: &str, controller: &Controller, baseline: usize) {
    let report = controller.report();
    let allocated = LIVE.load(Ordering::Relaxed) - baseline;
    assert_eq!(
        report.allocated, allocated,
        "{label}: controller census differs from live requested heap"
    );
    println!(
        "{label}: charged={} allocated={allocated}; reserved={}",
        report.allocated, report.reserved
    );
}

fn main() {
    // Prime stdout, hashing and parser process state before measuring ownership.
    println!("controller allocation calibration");
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let raw = b"---\naliases: [old, shared]\n---\n# Heading\n\n> [!note]- Callout\n\n- [ ] task\n\n[[target#Heading|alias]]\n";
    let source = Arc::new(store.retain(raw).unwrap());
    {
        let budget = Controller::new(0);
        let member = budget.retain(source.clone());
        drop(member.pin(Access::Read, 1_048_576).unwrap());
    }
    // The source Arc and backing store are externally owned and already exist
    // at baseline; their separate floor contract is exercised below by a Vec.
    let baseline = LIVE.load(Ordering::Relaxed);
    let budget = Controller::new(0);
    let a = budget.retain(source.clone());
    let b = budget.retain(source.clone());
    check("shared cold descriptor", &budget, baseline);
    let load_base = LIVE.load(Ordering::Relaxed);
    PEAK.store(load_base, Ordering::Relaxed);
    let declared_peak = 131_072;
    let pin = a.pin(Access::Read, declared_peak).unwrap();
    let load_peak = PEAK.load(Ordering::Relaxed) - load_base;
    assert!(
        load_peak <= declared_peak,
        "fixture reservation must cover its measured load peak"
    );
    println!("fixture load: declared={declared_peak} requested-heap peak delta={load_peak}");
    let other = b.pin(Access::Read, 1_048_576).unwrap();
    check("shared hot body and pins", &budget, baseline);
    let bytes = pin.raw();
    let twin = bytes.clone();
    drop((pin, other));
    budget.set_limit(1);
    check("shed tree with retained raw", &budget, baseline);
    drop((a, b));
    check("raw after descriptor destruction", &budget, baseline);
    drop((bytes, twin));
    check("last raw owner released", &budget, baseline);

    budget.set_limit(0);
    let reservation = budget.reserve(123_456).unwrap();
    let same = reservation.clone();
    check("shared reservation metadata", &budget, baseline);
    drop((reservation, same));
    let mut external = Vec::<u8>::with_capacity(4096);
    external.push(1);
    let floor = budget.charge_floor(external.capacity()).unwrap();
    check("external measured floor", &budget, baseline);
    external.reserve_exact(8192);
    floor.set(external.capacity());
    check("grown external floor", &budget, baseline);
    drop(external);
    drop(floor);
    check("external floor released", &budget, baseline);
    drop(budget);
    let remaining = LIVE.load(Ordering::Relaxed) - baseline;
    assert_eq!(remaining, 0);
    println!("all controller owners released: net retained bytes={remaining}");
}
