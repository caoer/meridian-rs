//! `resident` — the warm engine's memory footprint. Every other lane asks how
//! fast; this one asks how much a warm workspace costs to keep. The daemon
//! holds every parsed corpus (`model::Docs`: raw bytes + governed trees) resident
//! until idle-reap, so this number × the corpus size is what a workspace
//! costs the machine for as long as it stays warm.
//!
//! # Metric contract (claims.toml)
//! - `resident.heap.bytes_per_mb.vault_2026` — heap bytes `model::Docs` keeps
//!   resident per MB of corpus (raw bytes, governed trees, the path map),
//!   over `vault-2026 seed=1 files=2000`.
//!
//! How it measures: a counting `#[global_allocator]` wraps the system
//! allocator and tracks live bytes, so the number is the exact byte total the
//! engine asked the allocator to keep — deterministic for a given corpus and
//! tree, the same on every host, unlike RSS (page-granular, allocator- and
//! fragmentation-dependent). It excludes allocator overhead, so it is a floor
//! on RSS; what a code change moves, it moves here first.
//!
//! Hand-written `main`, `harness = false`: under the PR smoke lane
//! (`cargo bench -- --test`) it downshifts to a 200-file subsample and stages
//! nothing — a subsampled number would lie under a full-corpus claim id.

#![allow(
    clippy::cast_precision_loss,
    reason = "byte counts to MB floats; perf reporting, not arithmetic"
)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use perfsuite::corpus;
use perfsuite::measure::{Measurement, record_measurement};
use perfsuite::profile::{Profile, Recipe};

const SMOKE_FILES: u32 = 200;

/// Live heap bytes, as requested from the allocator (`Layout::size`), never
/// as the allocator rounds them.
static LIVE: AtomicUsize = AtomicUsize::new(0);

/// The counting allocator: the system allocator plus one live-bytes counter.
struct Counting;

// SAFETY: every call forwards to `System` with the same layout and pointer;
// the counter is bookkeeping on the side and never touches the memory.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        }
        p
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() {
            LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
            LIVE.fetch_add(new_size, Ordering::Relaxed);
        }
        p
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn live() -> usize {
    LIVE.load(Ordering::Relaxed)
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

/// Node census over one tree.
#[derive(Default, Clone, Copy)]
struct Census {
    nodes: u64,
    sections: u64,
    /// `children` capacity beyond length, in nodes — the doubling slack a
    /// push-built tree keeps until it is shrunk.
    slack: u64,
}

fn census(node: &model::Node) -> Census {
    let here = Census {
        nodes: 1,
        sections: u64::from(matches!(node.kind, model::NodeKind::Section { .. })),
        slack: (node.children.capacity() - node.children.len()) as u64,
    };
    node.children.iter().fold(here, |acc, c| {
        let c = census(c);
        Census {
            nodes: acc.nodes + c.nodes,
            sections: acc.sections + c.sections,
            slack: acc.slack + c.slack,
        }
    })
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let smoke = args.iter().any(|a| a == "--test");
    let profile_arg = flag(&args, "--profile").unwrap_or_else(|| "vault-2026".to_owned());
    let profile = Profile::resolve(&profile_arg).expect("profile resolves");
    let files = if smoke {
        Some(SMOKE_FILES)
    } else {
        flag(&args, "--files").map(|s| s.parse::<u32>().expect("--files is a number"))
    };
    let full_recipe = files.is_none_or(|n| n == profile.files);
    let recipe = Recipe::new(profile, 1, files);
    let suffix = recipe.profile.name.replace('-', "_");
    let id = format!("resident.heap.bytes_per_mb.{suffix}");

    eprintln!("ensuring corpus {} (untimed)…", recipe.id());
    let dir = corpus::ensure(&recipe).expect("corpus generates");

    // Warm every lazily-initialised buffer the parse path owns (thread
    // locals, hasher state) on a throwaway document, so the delta below is
    // the corpus and nothing else.
    {
        let (_, text) = corpus::load_files(&dir)
            .expect("corpus loads")
            .into_iter()
            .next()
            .expect("a non-empty corpus");
        let nodes = syntax::parse(&text);
        black_box(model::build(text, nodes));
    }

    // Everything the daemon keeps per workspace: `model::Docs` — the raw bytes
    // (moved, never copied, into `Document.raw`), the governed trees, and the
    // path-keyed map. The baseline is taken before the corpus is read, so the
    // raw strings count exactly as the engine holds them.
    let before = live();
    let loaded = corpus::load_files(&dir).expect("corpus loads");
    let corpus_bytes: u64 = loaded.iter().map(|(_, text)| text.len() as u64).sum();
    let docs: model::Docs = loaded
        .into_iter()
        .map(|(rel, text)| {
            let nodes = syntax::parse(&text);
            (rel, Arc::new(model::build(text, nodes)))
        })
        .collect();
    let resident = (live() - before) as u64;
    black_box(&docs);

    let Census {
        nodes,
        sections,
        slack,
    } = docs
        .values()
        .map(|d| census(&d.root))
        .fold(Census::default(), |acc, c| Census {
            nodes: acc.nodes + c.nodes,
            sections: acc.sections + c.sections,
            slack: acc.slack + c.slack,
        });
    let node_size = size_of::<model::Node>() as u64;
    let tree_bytes = resident.saturating_sub(corpus_bytes);
    let mb = corpus_bytes as f64 / 1_000_000.0;
    let bytes_per_mb = resident as f64 / mb.max(f64::MIN_POSITIVE);
    println!(
        "{id}: {} files, {mb:.2} MB corpus → {:.2} MB resident ({:.2}× the corpus; {bytes_per_mb:.0} bytes/MB)",
        docs.len(),
        resident as f64 / 1_000_000.0,
        resident as f64 / corpus_bytes.max(1) as f64,
    );
    println!(
        "  trees + map: {:.2} MB over {nodes} nodes ({sections} sections) — {:.0} bytes/node",
        tree_bytes as f64 / 1_000_000.0,
        tree_bytes as f64 / nodes.max(1) as f64,
    );
    println!(
        "  of which Node structs: {node_size} B each, {:.2} MB in children vectors, plus {:.2} MB of capacity slack ({slack} spare slots)",
        (nodes * node_size) as f64 / 1_000_000.0,
        (slack * node_size) as f64 / 1_000_000.0,
    );

    if full_recipe {
        let m = Measurement {
            id,
            value: bytes_per_mb,
            unit: "bytes/MB".to_owned(),
            samples: 1,
            dist: None,
        };
        let path = record_measurement(&m).expect("staging dir writable");
        println!("  staged → {}", path.display());
    } else {
        println!("  smoke/subsampled run — not staged");
    }
    drop(docs);
}
