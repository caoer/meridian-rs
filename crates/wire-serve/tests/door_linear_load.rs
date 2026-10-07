//! A door that resolves or plans one section over a page costs memory in
//! proportion to the page and its answer, not to rows times the heading
//! chain (merkle-spec §6.4, "The doors keep that sharing"). The shape is the
//! amplifying one: a heading of half the page governing many empty `##`
//! sections, so every section's address repeats the long heading. Three
//! doors run over it — the toc facts table every door resolves against, a
//! one-section read, and a one-section append planned and validated in
//! memory — each measured above the loaded document with a counting
//! allocator. This binary holds one test, so nothing else allocates while it
//! measures.

use std::alloc::{GlobalAlloc, Layout, System};
use std::fmt::Write as _;
use std::sync::atomic::{AtomicUsize, Ordering};

use wire::{HpathSeg, Path as WPath, PlanEdit, ReadSel};
use wire_serve::read::{NO_DECORATIONS, ReadParams, composed_read};

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

/// Per input byte, above the loaded page, that a door may stage. The facts
/// table and a read stay well under it; a write also reparses the page,
/// which `model/tests/linear_load.rs` bounds at 512 per byte on its own.
const PER_BYTE: usize = 1024;
const FIXED: usize = 128 * 1024;
/// Off the doubling points, where growth buffers are not exactly full.
const SIZE: usize = 48 * 1024 + 3;
/// The section both doors address: one of the empty `##` sections, which
/// share their (empty) text, so the address pins the occurrence.
const OCCURRENCE: u32 = 777;

#[test]
fn one_section_doors_stay_linear_under_a_long_heading() {
    let heading = "h".repeat(SIZE / 2);
    let mut raw = format!("# {heading}\n\n");
    while raw.len() < SIZE {
        raw.push_str("##\n");
    }
    let len = raw.len();
    let doc = model::build(raw.clone(), syntax::parse(&raw));
    let address = vec![
        HpathSeg {
            h: heading.clone().into(),
            n: None,
        },
        HpathSeg {
            h: String::new().into(),
            n: Some(OCCURRENCE),
        },
    ];
    let bound = |answer: usize| FIXED + PER_BYTE * len + answer;
    let mut over = String::new();

    let (rows, peak) = peak_of(|| {
        wire_map::facts::read_facts(&wire_map::project_toc(&doc), doc.raw.as_bytes()).len()
    });
    assert!(rows > 8_000, "the page has its sections ({rows} facts)");
    if peak > bound(0) {
        let _ = writeln!(
            over,
            "toc facts: peak {peak} B ({} per byte) over {} B",
            peak / len,
            bound(0)
        );
    }

    let params = ReadParams {
        sections: Some(vec![ReadSel::Hpath {
            hpath: address.clone(),
        }]),
        display_path: Some("page.md".into()),
        ..ReadParams::default()
    };
    let (answer, peak) = peak_of(|| {
        composed_read(
            &doc,
            &WPath("page.md".into()),
            &wire::Root("r".into()),
            &params,
            &NO_DECORATIONS,
        )
        .map(|body| {
            serde_json::to_vec(&body)
                .expect("an answer serializes")
                .len()
        })
    });
    let answer = answer.expect("the one section reads");
    if peak > bound(answer) {
        let _ = writeln!(
            over,
            "one-section read ({answer} B answer): peak {peak} B ({} per byte) over {} B",
            peak / len,
            bound(answer)
        );
    }

    let edit = [PlanEdit::Append {
        hpath: address,
        body: "appended\n".into(),
        rev: None,
    }];
    let (candidate, peak) = peak_of(|| {
        wire_serve::write::overlay_candidate(&doc, &WPath("page.md".into()), &edit)
            .map(|next| next.raw.len())
    });
    let candidate = candidate.expect("the append plans and validates");
    assert!(candidate > len, "the append landed in the candidate");
    if peak > bound(0) {
        let _ = writeln!(
            over,
            "one-section append: peak {peak} B ({} per byte) over {} B",
            peak / len,
            bound(0)
        );
    }

    assert!(
        over.is_empty(),
        "a one-section door is not linear in the page ({len} B):\n{over}"
    );
}
