//! A `run` mode row keeps only what it runs of its page (merkle-spec §6.8):
//! the fences' bytes and a few short strings, never the page's parse tree.
//! Measured with a counting allocator; this binary holds one test, so nothing
//! else allocates while it measures.

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

/// The heap `make` leaves live in what it returns.
fn kept_by<T>(make: impl FnOnce() -> T) -> (T, usize) {
    let base = LIVE.load(Ordering::Relaxed);
    let value = make();
    (value, LIVE.load(Ordering::Relaxed).saturating_sub(base))
}

#[test]
fn a_mode_rows_page_copy_keeps_its_blocks_and_not_the_parse_tree() {
    let prose = "a line with [[a link]] and **strong** text\n".repeat(24_000);
    let raw = format!(
        "# Hooks\n\n{prose}\n```starlark\ndef run(event):\n    return 1\n\n\
         declare(on = \"Stop\")\n```\n^h\n\n```bash\necho ran\n```\n^p\n"
    );
    let doc = model::build(raw.clone(), syntax::parse(&raw));
    let (page, kept) = kept_by(|| run::modes::Page::of(&doc));
    assert!(
        kept < 16 * 1024,
        "a row's copy of a {} byte page keeps {kept} bytes",
        raw.len()
    );
    drop(page);
}
