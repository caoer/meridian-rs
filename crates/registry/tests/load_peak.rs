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

/// The sweep's reach in bytes: 64 KiB by default (CI), 4 MiB under
/// `LOAD_PEAK_SWEEP=full`, the run the spec's table and the per-byte part
/// are taken from.
fn reach() -> usize {
    if std::env::var_os("LOAD_PEAK_SWEEP").is_some_and(|sweep| sweep == "full") {
        1 << 22
    } else {
        1 << 16
    }
}

/// The shapes whose load the default sweep also takes near 1 MiB, so a
/// peak that grows faster than the length goes red in CI.
const PINNED: [&str; 3] = [
    "bare-headings",
    "heading-code-over-headings",
    "headings-then-code-span",
];

/// Unit counts just past each doubling point. The parser's tree, the
/// dialect list and the model's children grow by doubling on counts of
/// constructs, and one of them holds `n − 1` entries, so a page of `n`
/// repeated units peaks highest per byte at `2^j + 1` or `2^j + 2` units,
/// whatever the unit's length. Every `j` while `2^j` units fit in `reach`;
/// a pinned shape also at the first `j` whose `2^j` units reach 1 MiB.
fn unit_counts(unit_len: usize, pinned: bool) -> Vec<usize> {
    let doublings = |reach: usize| (0..).take_while(move |j| (1usize << j) * unit_len <= reach);
    let mut js: Vec<u32> = doublings(reach()).collect();
    if pinned {
        js.push(doublings((1 << 20) - 1).count().try_into().unwrap());
    }
    let mut counts: Vec<usize> = js
        .into_iter()
        .flat_map(|j| [(1usize << j) + 1, (1 << j) + 2])
        .collect();
    counts.sort_unstable();
    counts.dedup();
    counts
}

/// Byte lengths for the shapes not built of repeated units: `2^k`, one
/// byte past it and `1.5·2^k`, every `k` from 10 while `2^k` is in `reach`.
fn byte_lengths() -> Vec<usize> {
    let mut lengths = vec![64];
    for k in 10..=reach().ilog2() {
        let at = 1usize << k;
        lengths.extend([at, at + 1, at + at / 2]);
    }
    lengths
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

/// How a shape's page is built.
enum Page {
    /// `head`, then `unit(i)` for each of `n` units.
    Units(&'static str, fn(usize) -> String),
    /// At least `n` bytes.
    Bytes(fn(usize) -> String),
}

impl Page {
    fn build(&self, n: usize) -> String {
        match self {
            Self::Units(head, unit) => {
                let mut raw = (*head).to_owned();
                for i in 0..n {
                    raw.push_str(&unit(i));
                }
                raw
            }
            Self::Bytes(build) => build(n),
        }
    }

    /// The `n` of each sampled page.
    fn samples(&self, shape: &str) -> Vec<usize> {
        match self {
            Self::Units(head, _) => {
                let unit_len = (self.build(1024).len() - head.len()) / 1024;
                unit_counts(unit_len, PINNED.contains(&shape))
            }
            Self::Bytes(_) => byte_lengths(),
        }
    }
}

/// The shapes: most repeat one unit.
#[allow(clippy::too_many_lines)]
fn shapes() -> Vec<(&'static str, Page)> {
    use Page::{Bytes, Units};
    vec![
        (
            "prose",
            Units("", |n| {
                format!(
                    "Sentence {n} of the prose shape carries ordinary words and punctuation. {}",
                    if n % 6 == 5 { "\n\n" } else { "" }
                )
            }),
        ),
        ("list", Units("", |n| format!("- item {n}\n"))),
        ("bare-list", Units("", |_| "-\n".to_owned())),
        (
            "nested-list",
            Units("", |n| format!("- a{n}\n  - b\n    - c\n")),
        ),
        ("tasks", Units("", |n| format!("- [ ] task {n}\n"))),
        (
            "wikilinks",
            Units("", |n| format!("[[target-{n}#Heading|alias {n}]] ")),
        ),
        ("bare-links", Units("", |_| "[[a]]".to_owned())),
        ("embeds", Units("", |n| format!("![[image-{n}.png]]\n"))),
        ("headings", Units("", |n| format!("# Heading {n}\n\n"))),
        ("bare-headings", Units("", |_| "#\n".to_owned())),
        ("block-ids", Units("", |n| format!("line {n} ^b{n}\n\n"))),
        (
            "table",
            Units("| a | b | c |\n|---|---|---|\n", |n| {
                format!("| {n} | x | y |\n")
            }),
        ),
        (
            "cjk",
            Units("", |_| "中文段落的测试内容，包含标点。\n\n".to_owned()),
        ),
        (
            "frontmatter",
            Bytes(|size| {
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
            }),
        ),
        (
            "callouts",
            Units("", |n| format!("> [!note] Title {n}\n> body\n\n")),
        ),
        (
            "code",
            Units("", |n| format!("```rust\nfn f{n}() {{}}\n```\n\n")),
        ),
        (
            "inline",
            Units("", |n| format!("*a{n}* **b** `c` ~~d~~ ==e== ")),
        ),
        ("tags", Units("", |n| format!("#tag-{n} "))),
        // Adversarial: the least text per construct.
        ("bare-tags", Units("", |_| "#a ".to_owned())),
        ("bare-embeds", Units("", |_| "![[a]]".to_owned())),
        ("bare-tasks", Units("", |_| "- [ ]\n".to_owned())),
        ("bare-block-ids", Units("", |_| "a ^b\n\n".to_owned())),
        (
            "deep-list",
            Units("", |n| format!("{}- a\n", "  ".repeat(n % 32))),
        ),
        ("bare-quotes", Units("", |_| ">\n\n".to_owned())),
        (
            "deep-quotes",
            Units("", |n| format!("{}a\n\n", "> ".repeat(n % 32 + 1))),
        ),
        (
            "cell-table",
            Units("|a|a|a|a|a|a|a|a|\n|-|-|-|-|-|-|-|-|\n", |_| {
                "|a|a|a|a|a|a|a|a|\n".to_owned()
            }),
        ),
        ("one-row-tables", Units("", |_| "|a|\n|-|\n\n".to_owned())),
        ("ref-defs", Units("", |n| format!("[r{n}]: /u\n"))),
        ("ref-links", Units("", |_| "[a][b] ".to_owned())),
        ("inline-links", Units("", |_| "[a](b)".to_owned())),
        ("autolinks", Units("", |_| "<a:b> ".to_owned())),
        ("html-blocks", Units("", |_| "<div>\n\n".to_owned())),
        ("inline-html", Units("", |_| "<b>a</b> ".to_owned())),
        ("emphasis-runs", Units("", |_| "*a*".to_owned())),
        ("open-emphasis", Units("", |_| "*a ".to_owned())),
        ("strong-runs", Units("", |_| "**a**".to_owned())),
        ("code-spans", Units("", |_| "`a` ".to_owned())),
        ("hard-breaks", Units("", |_| "a\\\n".to_owned())),
        ("rules", Units("", |_| "---\n\n".to_owned())),
        ("bare-fences", Units("", |_| "```\n```\n".to_owned())),
        ("footnotes", Units("", |n| format!("[^{n}]: a\n\n"))),
        ("one-char-paragraphs", Units("", |_| "a\n\n".to_owned())),
        // A long heading governs every leaf below it (rv-360 P1-2a).
        (
            "long-heading-sections",
            Bytes(|size| long_heading(size, "##\n")),
        ),
        (
            "long-heading-code-spans",
            Bytes(|size| long_heading(size, "`a` ")),
        ),
        (
            "long-heading-links",
            Bytes(|size| long_heading(size, "[[a]]")),
        ),
        // A wide header over one-cell rows: padded cells (rv-360 P1-2b).
        ("wide-tables-k64", Bytes(|size| wide_tables(size, 64, 64))),
        (
            "wide-tables-k362",
            Bytes(|size| wide_tables(size, 362, 724)),
        ),
        ("heading-code", Units("", |_| "# `a`\n".to_owned())),
        // Headings carrying a block id or a code span, and code under bare
        // headings (rv-360b P2-1).
        ("heading-block-ids", Units("", |_| "# ^b\n".to_owned())),
        (
            "heading-code-over-headings",
            Units("", |_| "# `a`\n#\n".to_owned()),
        ),
        (
            "headings-then-code-span",
            Units("", |_| "#\n#\n#\n`a`".to_owned()),
        ),
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
    for (shape, page) in shapes() {
        for n in page.samples(shape) {
            let raw = page.build(n);
            let doc = model::build(raw.clone(), syntax::parse(&raw));
            let tree = model::allocation::document(&doc).tree;
            drop(doc);
            let nodes = syntax::parse(&raw).len();
            let parse = parse_peak(&raw);
            let measured = cold_load_peak(&store, &raw);
            let declared = registry::residency::load_peak(raw.len() as u64);
            println!(
                "load-peak shape={shape} n={n} len={} tree={tree} nodes={nodes} parse={parse} \
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
