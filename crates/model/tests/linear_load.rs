//! A document's parse staging and governed tree grow linearly with its bytes
//! for every shape (merkle-spec §6.4), the precondition for a load-peak
//! declaration that is a function of the raw length. The shapes here are the
//! two amplifying families: a long heading governing many small leaves (each
//! leaf's `hpath` must share the heading text, not copy it) and a wide table
//! header over one-cell rows (the parser's row padding must not stage one
//! item per missing cell). Measured with a counting allocator; this binary
//! holds one test, so nothing else allocates while it measures. The ledger's
//! charge for the retained tree (`model::allocation`) is held to the same
//! bound.

use std::alloc::{GlobalAlloc, Layout, System};
use std::fmt::Write as _;
use std::sync::atomic::{AtomicUsize, Ordering};

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

/// A heading of half the page, then small leaves under it up to `size`.
fn long_heading(size: usize, leaf: &str) -> String {
    let mut raw = String::from("# ");
    raw.push_str(&"h".repeat(size / 2));
    raw.push_str("\n\n");
    while raw.len() < size {
        raw.push_str(leaf);
    }
    raw
}

/// Tables with a `cols`-wide header, each followed by `rows` one-cell rows.
fn wide_tables(size: usize, cols: usize, rows: usize) -> String {
    let mut table = String::new();
    for _ in 0..cols {
        table.push_str("|a");
    }
    table.push_str("|\n");
    for _ in 0..cols {
        table.push_str("|-");
    }
    table.push_str("|\n");
    for _ in 0..rows {
        table.push_str("a\n");
    }
    table.push('\n');
    let mut raw = String::new();
    while raw.len() < size {
        raw.push_str(&table);
    }
    raw
}

/// Staging plus tree per input byte that every shape stays under; the
/// linear shapes measure well below it, and the amplifying ones measured
/// 3 000–11 000 per byte at these lengths before their representations
/// changed.
const PER_BYTE: usize = 512;
const FIXED: usize = 128 * 1024;
/// Off the doubling points, where growth buffers are not exactly full.
const SIZE: usize = 48 * 1024 + 3;

#[test]
fn amplifying_shapes_parse_and_build_within_a_linear_bound() {
    let shapes: [(&str, String); 5] = [
        ("long heading over `##` sections", {
            let mut raw = String::from("# ");
            raw.push_str(&"h".repeat(SIZE / 2));
            raw.push_str("\n\n");
            while raw.len() < SIZE {
                raw.push_str("##\n");
            }
            raw
        }),
        ("long heading over code spans", long_heading(SIZE, "`a` ")),
        ("long heading over wikilinks", long_heading(SIZE, "[[a]]")),
        ("64-wide tables, one-cell rows", wide_tables(SIZE, 64, 64)),
        (
            "362-wide tables, one-cell rows",
            wide_tables(SIZE, 362, 724),
        ),
    ];
    let mut over = String::new();
    for (name, raw) in shapes {
        let len = raw.len();
        let (doc, peak) = peak_of(|| {
            let nodes = syntax::parse(&raw);
            model::build(raw, nodes)
        });
        // The ledger's charge for the retained tree is linear as well: a
        // shared chain is charged once, where it was allocated.
        let charged = model::allocation::document(&doc).tree;
        drop(doc);
        let bound = FIXED + PER_BYTE * len;
        if peak > bound {
            let _ = writeln!(
                over,
                "{name} ({len} B): peak {peak} B ({} per byte) over {bound} B",
                peak / len
            );
        }
        if charged > bound {
            let _ = writeln!(
                over,
                "{name} ({len} B): tree charged {charged} B ({} per byte) over {bound} B",
                charged / len
            );
        }
    }
    assert!(
        over.is_empty(),
        "parse + build is not linear in the input:\n{over}"
    );
}
