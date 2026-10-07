//! Holds the load-peak rule to measurement (merkle-spec §6.4, load-peak
//! declarations): for each document shape, one cold load through a
//! controller member is measured with a counting allocator — the peak of
//! live heap above the moment the pin begins — and
//! `residency::load_peak(len)` must cover it. The printed rows are the
//! spec's per-shape table. One test, so no other test thread allocates
//! while it measures.

use std::alloc::{GlobalAlloc, Layout, System};
use std::fmt::Write as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use registry::raw_store::RawStore;
use registry::residency::Access;
use registry::residency::controller::Controller;

struct Counting;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn up(size: usize) {
    let now = LIVE.fetch_add(size, Ordering::SeqCst) + size;
    PEAK.fetch_max(now, Ordering::SeqCst);
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            up(layout.size());
        }
        pointer
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            up(layout.size());
        }
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::SeqCst);
        unsafe { System.dealloc(pointer, layout) };
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let next = unsafe { System.realloc(pointer, layout, size) };
        if !next.is_null() {
            if size >= layout.size() {
                up(size - layout.size());
            } else {
                LIVE.fetch_sub(layout.size() - size, Ordering::SeqCst);
            }
        }
        next
    }
}
#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Lengths on and off the doubling points: at `2^k` every growth buffer
/// (the parser's tree, the dialect list, the model's children) can be
/// exactly full; one unit past it, or at `1.5·2^k`, they have just doubled.
/// Every `k` from 10 to 16; `LOAD_PEAK_SWEEP=full` extends it to 20 (up to
/// 1.5 MiB), the run the spec's table and the per-byte part are taken from.
fn sizes() -> Vec<usize> {
    let full = std::env::var_os("LOAD_PEAK_SWEEP").is_some_and(|sweep| sweep == "full");
    let mut sizes = vec![64];
    for k in 10..=if full { 20 } else { 16 } {
        let at = 1usize << k;
        sizes.extend([at, at + 1, at + at / 2]);
    }
    sizes
}

/// A heading of half the page, then small leaves under it.
fn long_heading(size: usize, leaf: &str) -> String {
    let mut raw = String::from("# ");
    raw.push_str(&"h".repeat(size / 2));
    raw.push_str("\n\n");
    while raw.len() < size {
        raw.push_str(leaf);
    }
    raw
}

/// Tables whose header is `cols` wide, each followed by `rows` one-cell rows.
fn wide_tables(size: usize, cols: usize, rows: usize) -> String {
    let mut table = "|a".repeat(cols);
    table.push_str("|\n");
    table.push_str(&"|-".repeat(cols));
    table.push_str("|\n");
    table.push_str(&"a\n".repeat(rows));
    table.push('\n');
    let mut raw = String::new();
    while raw.len() < size {
        raw.push_str(&table);
    }
    raw
}

/// The shapes at `size` bytes or just over: each repeats one unit.
#[allow(clippy::too_many_lines)]
fn shapes(size: usize) -> Vec<(&'static str, String)> {
    let fill = |unit: &dyn Fn(usize) -> String| {
        let mut raw = String::new();
        let mut n = 0;
        while raw.len() < size {
            raw.push_str(&unit(n));
            n += 1;
        }
        raw
    };
    let frontmatter = {
        let mut raw = String::from("---\naliases: [");
        let mut n = 0;
        while raw.len() < size / 2 {
            let _ = write!(raw, "alias-{n}, ");
            n += 1;
        }
        raw.push_str("last]\n");
        while raw.len() < size {
            let _ = writeln!(raw, "key-{n}: value {n}");
            n += 1;
        }
        raw.push_str("---\n# Title\n");
        raw
    };
    vec![
        (
            "prose",
            fill(&|n| {
                format!(
                    "Sentence {n} of the prose shape carries ordinary words and punctuation. {}",
                    if n % 6 == 5 { "\n\n" } else { "" }
                )
            }),
        ),
        ("list", fill(&|n| format!("- item {n}\n"))),
        ("bare-list", fill(&|_| "-\n".to_owned())),
        (
            "nested-list",
            fill(&|n| format!("- a{n}\n  - b\n    - c\n")),
        ),
        ("tasks", fill(&|n| format!("- [ ] task {n}\n"))),
        (
            "wikilinks",
            fill(&|n| format!("[[target-{n}#Heading|alias {n}]] ")),
        ),
        ("bare-links", fill(&|_| "[[a]]".to_owned())),
        ("embeds", fill(&|n| format!("![[image-{n}.png]]\n"))),
        ("headings", fill(&|n| format!("# Heading {n}\n\n"))),
        ("bare-headings", fill(&|_| "#\n".to_owned())),
        ("block-ids", fill(&|n| format!("line {n} ^b{n}\n\n"))),
        (
            "table",
            format!(
                "| a | b | c |\n|---|---|---|\n{}",
                fill(&|n| format!("| {n} | x | y |\n"))
            ),
        ),
        (
            "cjk",
            fill(&|_| "中文段落的测试内容，包含标点。\n\n".to_owned()),
        ),
        ("frontmatter", frontmatter),
        (
            "callouts",
            fill(&|n| format!("> [!note] Title {n}\n> body\n\n")),
        ),
        (
            "code",
            fill(&|n| format!("```rust\nfn f{n}() {{}}\n```\n\n")),
        ),
        (
            "inline",
            fill(&|n| format!("*a{n}* **b** `c` ~~d~~ ==e== ")),
        ),
        ("tags", fill(&|n| format!("#tag-{n} "))),
        // Adversarial: the least text per construct.
        ("bare-tags", fill(&|_| "#a ".to_owned())),
        ("bare-embeds", fill(&|_| "![[a]]".to_owned())),
        ("bare-tasks", fill(&|_| "- [ ]\n".to_owned())),
        ("bare-block-ids", fill(&|_| "a ^b\n\n".to_owned())),
        (
            "deep-list",
            fill(&|n| format!("{}- a\n", "  ".repeat(n % 32))),
        ),
        ("bare-quotes", fill(&|_| ">\n\n".to_owned())),
        (
            "deep-quotes",
            fill(&|n| format!("{}a\n\n", "> ".repeat(n % 32 + 1))),
        ),
        (
            "cell-table",
            format!(
                "|a|a|a|a|a|a|a|a|\n|-|-|-|-|-|-|-|-|\n{}",
                fill(&|_| "|a|a|a|a|a|a|a|a|\n".to_owned())
            ),
        ),
        ("one-row-tables", fill(&|_| "|a|\n|-|\n\n".to_owned())),
        ("ref-defs", fill(&|n| format!("[r{n}]: /u\n"))),
        ("ref-links", fill(&|_| "[a][b] ".to_owned())),
        ("inline-links", fill(&|_| "[a](b)".to_owned())),
        ("autolinks", fill(&|_| "<a:b> ".to_owned())),
        ("html-blocks", fill(&|_| "<div>\n\n".to_owned())),
        ("inline-html", fill(&|_| "<b>a</b> ".to_owned())),
        ("emphasis-runs", fill(&|_| "*a*".to_owned())),
        ("open-emphasis", fill(&|_| "*a ".to_owned())),
        ("strong-runs", fill(&|_| "**a**".to_owned())),
        ("code-spans", fill(&|_| "`a` ".to_owned())),
        ("hard-breaks", fill(&|_| "a\\\n".to_owned())),
        ("rules", fill(&|_| "---\n\n".to_owned())),
        ("bare-fences", fill(&|_| "```\n```\n".to_owned())),
        ("footnotes", fill(&|n| format!("[^{n}]: a\n\n"))),
        ("one-char-paragraphs", fill(&|_| "a\n\n".to_owned())),
        // A long heading governs every leaf below it (rv-360 P1-2a).
        ("long-heading-sections", long_heading(size, "##\n")),
        ("long-heading-code-spans", long_heading(size, "`a` ")),
        ("long-heading-links", long_heading(size, "[[a]]")),
        // A wide header over one-cell rows: padded cells (rv-360 P1-2b).
        ("wide-tables-k64", wide_tables(size, 64, 64)),
        ("wide-tables-k362", wide_tables(size, 362, 724)),
        ("heading-code", fill(&|_| "# `a`\n".to_owned())),
    ]
}

/// The peak heap above its start of one cold load of `raw`.
fn cold_load_peak(store: &RawStore, raw: &str) -> usize {
    let source = Arc::new(store.retain(raw.as_bytes()).unwrap());
    let controller = Controller::new(0);
    let member = controller.retain(source);
    let start = LIVE.load(Ordering::SeqCst);
    PEAK.store(start, Ordering::SeqCst);
    let pin = member.pin(Access::Read, usize::MAX / 4).unwrap();
    let peak = PEAK.load(Ordering::SeqCst).saturating_sub(start);
    drop(pin);
    peak
}

/// The peak heap above its start of parsing `raw` alone.
fn parse_peak(raw: &str) -> usize {
    let start = LIVE.load(Ordering::SeqCst);
    PEAK.store(start, Ordering::SeqCst);
    let nodes = syntax::parse(raw);
    let peak = PEAK.load(Ordering::SeqCst).saturating_sub(start);
    drop(nodes);
    peak
}

#[test]
fn every_shapes_cold_load_peaks_within_its_declaration() {
    let tmp = tempfile::tempdir().unwrap();
    let store = RawStore::new(&tmp.path().join("backing")).unwrap();
    // Process-wide first-use allocations are not a load's.
    cold_load_peak(&store, "# prime\n\n[[a]] text ^b\n");
    let mut rows = Vec::new();
    for size in sizes() {
        for (shape, raw) in shapes(size) {
            let doc = model::build(raw.clone(), syntax::parse(&raw));
            let tree = model::allocation::document(&doc).tree;
            drop(doc);
            let nodes = syntax::parse(&raw).len();
            let parse = parse_peak(&raw);
            let measured = cold_load_peak(&store, &raw);
            let declared = registry::residency::load_peak(raw.len() as u64);
            println!(
                "load-peak shape={shape} len={} tree={tree} nodes={nodes} parse={parse} \
                 measured={measured} declared={declared} declared%={}",
                raw.len(),
                declared.saturating_mul(100) / measured.max(1),
            );
            rows.push((shape, raw.len(), measured, declared));
        }
    }
    for (shape, len, measured, declared) in rows {
        assert!(
            declared >= measured,
            "{shape} ({len} bytes): declared {declared} is below the measured peak {measured}"
        );
    }
}
