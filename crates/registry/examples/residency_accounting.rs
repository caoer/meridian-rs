//! Quiescent calibration of residency heap censuses against real deallocation.
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

fn freed<T>(value: T) -> usize {
    let before = LIVE.load(Ordering::Relaxed);
    drop(value);
    before - LIVE.load(Ordering::Relaxed)
}

fn documents() {
    #[repr(align(64))]
    struct Aligned(u8);
    let aligned = Arc::new(Aligned(1));
    assert_eq!(aligned.0, 1);
    let charged = model::allocation::arc_bytes::<Aligned>();
    let observed = freed(aligned);
    println!("aligned Arc charged={charged} freed={observed}");
    assert_eq!(charged, observed);

    let raw = "---\naliases: [first, second]\n---\n# Heading\n\n> [!note]- Callout\n\n- [ ] task\n\n[[target#Heading|alias]] ![[embed#part]] [link](dest) #tag\n\n```rust\ncode\n```\n\npara ^anchor\n\n| a | b |\n| - | - |\n| c | d |\n<!-- comment --> `inline`\n";
    let mut owned = String::with_capacity(8192);
    owned.push_str(raw);
    let mut doc = model::build(owned, syntax::parse(raw));
    doc.root.children.reserve_exact(16);
    let charge = model::allocation::document(&doc);
    let bytes = model::bytes::SharedBytes::from(Arc::clone(&doc.raw));
    let byte_charge = bytes.heap_bytes();
    let tree_freed = freed(doc);
    let raw_freed = freed(bytes);
    println!(
        "document tree charged={} freed={tree_freed}; shared raw charged={} freed={raw_freed}",
        charge.tree, charge.raw
    );
    assert_eq!(
        charge.tree, tree_freed,
        "tree census must match deallocation"
    );
    assert_eq!(
        charge.raw, raw_freed,
        "shared raw census must match deallocation"
    );
    assert_eq!(byte_charge, raw_freed);

    let binary = model::bytes::SharedBytes::from(vec![0xff; 8193]);
    let charged = binary.heap_bytes();
    let observed = freed(binary);
    println!("binary charged={charged} freed={observed}");
    assert_eq!(charged, observed);
}

fn backing() {
    let home = tempfile::tempdir().unwrap();
    let empty = registry::raw_store::RawStore::new(home.path()).unwrap();
    let charged = empty.heap_bytes().unwrap();
    let observed = freed(empty);
    println!("empty backing charged={charged} freed={observed}");
    assert_eq!(charged, observed);

    let store = registry::raw_store::RawStore::new(home.path()).unwrap();
    let first = store.retain(b"first").unwrap();
    let same = first.clone();
    let retired = store.retain(b"retired but still weak-indexed").unwrap();
    drop(retired);
    let charged = store.heap_bytes().unwrap();
    let observed = freed((first, same, store));
    println!("backing with dead weak record charged={charged} freed={observed}");
    assert_eq!(
        charged, observed,
        "backing census must include weak-held storage"
    );

    let compacted = registry::raw_store::RawStore::new(home.path()).unwrap();
    drop(compacted.retain(b"removed before compaction").unwrap());
    compacted.compact().unwrap();
    let charged = compacted.heap_bytes().unwrap();
    let observed = freed(compacted);
    println!("empty compacted backing charged={charged} freed={observed}");
    assert_eq!(
        charged, observed,
        "retained index/vector capacity remains charged"
    );
}

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("documents") => documents(),
        Some("backing") => backing(),
        None => {
            documents();
            backing();
        }
        Some(other) => panic!("unknown calibration case: {other}"),
    }
}
