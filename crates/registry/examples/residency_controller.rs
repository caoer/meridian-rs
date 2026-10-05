//! Quiescent calibration of controller charges against requested live heap.
//! Run on a build host; no installed service or workspace is used.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Counting;
static LIVE: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        }
        pointer
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        }
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(pointer, layout) };
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let next = unsafe { System.realloc(pointer, layout, size) };
        if !next.is_null() {
            if size >= layout.size() {
                LIVE.fetch_add(size - layout.size(), Ordering::Relaxed);
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
    let pin = a.pin(Access::Read, 1_048_576).unwrap();
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
