//! The ledger's charge for a retained tree (`model::allocation::document`) is
//! the heap its drop frees, byte for byte, including the boxes a callout and a
//! qualified link hold. The residency controller charges that figure against
//! the host budget, so an undercount lets resident trees exceed the budget
//! with no signal. Measured with a counting allocator; this binary holds one
//! test, so nothing else allocates while it measures.

use std::alloc::{GlobalAlloc, Layout, System};
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
        unsafe { System.dealloc(pointer, layout) };
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
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

#[test]
fn tree_charge_equals_the_heap_its_drop_frees() {
    // Every node kind that owns heap: frontmatter, a section and its heading,
    // a callout, a task, a qualified wikilink, a qualified and an unqualified
    // embed, an unqualified wikilink, a link, a tag, a fence, an anchor, a table.
    let raw = "---\naliases: [first, second]\n---\n# Heading\n\n> [!note]- Callout\n\n\
               - [ ] task\n\n[[target#Heading|alias]] ![[embed#part]] ![[image]] [[plain]] \
               [link](dest) #tag\n\n```rust\ncode\n```\n\npara ^anchor\n\n| a | b |\n\
               | - | - |\n| c | d |\n<!-- comment --> `inline`\n";
    let doc = model::build(raw.to_owned(), syntax::parse(raw));
    let raw_bytes = std::sync::Arc::clone(&doc.raw);
    let charged = model::allocation::document(&doc).tree;
    let before = LIVE.load(Ordering::Relaxed);
    drop(doc);
    let freed = before - LIVE.load(Ordering::Relaxed);
    drop(raw_bytes);
    assert_eq!(charged, freed, "tree census must match deallocation");
}
