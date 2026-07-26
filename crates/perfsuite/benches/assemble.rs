//! Seam bench: `model::build` — dialect stream → governed tree.
//!
//! # Metric contract (claims.toml)
//! - `assemble.p99.file` — per-file assembly latency p99 (hdr path) over
//!   `vault-2026 seed=1 files=2000`; baseline TBD on first fleet run.
//!
//! `model::build` consumes its input (`raw: String`, `nodes: Vec<DialectNode>`),
//! so every iteration needs its own copy. Parsing and cloning happen OUTSIDE
//! the timed window — the claim is assembly, not parse+clone+assembly — which
//! is why the hdr run records by hand instead of using `LatencyRun::run`.
//!
//! Hand-written `main` (not `criterion_main!`): criterion runs first, then the
//! hdr claim run records its measurement. In the PR smoke lane
//! (`cargo bench -- --test`) the claim run downshifts to the 200-file criterion
//! corpus and stages NOTHING — a subsampled number would lie under a
//! full-corpus claim id (`cold_ingest.rs` established the split).

use std::hint::black_box;
use std::io;
use std::io::Write as _;
use std::time::Instant;

use criterion::{BatchSize, Criterion, criterion_group};

use perfsuite::corpus;
use perfsuite::measure::{LatencyRun, record_measurement};
use perfsuite::profile::{Profile, Recipe};

/// Criterion working set: 200 files keeps dev iteration (and the smoke lane)
/// snappy; the claim run uses the full files=2000 recipe the row registers.
const WORKING_SET: u32 = 200;
const FULL_RECIPE: u32 = 2_000;

/// Pre-parsed assembly inputs: `(raw, dialect nodes)` per corpus file.
fn inputs(files: u32) -> Vec<(String, Vec<syntax::DialectNode>)> {
    let profile = Profile::resolve("vault-2026").expect("bundled profile loads");
    let recipe = Recipe::new(profile, 1, Some(files));
    let dir = corpus::ensure(&recipe).expect("corpus generate-or-reuse");
    corpus::load_files(&dir)
        .expect("load corpus files")
        .into_iter()
        .map(|(_rel, text)| {
            let nodes = syntax::parse(&text);
            (text, nodes)
        })
        .collect()
}

fn bench_assemble(c: &mut Criterion) {
    let inputs = inputs(WORKING_SET);
    let mut i = 0usize;
    c.bench_function("assemble/build_vault2026", |b| {
        b.iter_batched(
            || {
                let input = inputs[i % inputs.len()].clone();
                i += 1;
                input
            },
            |(raw, nodes)| model::build(raw, nodes),
            BatchSize::SmallInput,
        );
    });
}

/// hdr p99 of one `model::build`, cycling through the corpus. Stages the claim
/// only on full-recipe runs.
fn record_assemble_p99(full_recipe: bool) {
    let (files, warmup, iters) = if full_recipe {
        (FULL_RECIPE, 500u32, 8_000u32)
    } else {
        (WORKING_SET, 20, 200)
    };

    let inputs = inputs(files);
    let mut run = LatencyRun::new();
    for n in 0..(warmup + iters) {
        // Untimed: `build` consumes its input, so each iteration owns a copy.
        let (raw, nodes) = inputs[n as usize % inputs.len()].clone();
        let start = Instant::now();
        let doc = model::build(raw, nodes);
        let elapsed = start.elapsed();
        black_box(&doc);
        if n >= warmup {
            run.record(elapsed);
        }
    }

    let m = run.to_p99_measurement("assemble.p99.file", "us");
    if !full_recipe {
        let _ = writeln!(
            io::stderr(),
            "assemble.p99.file: smoke run, p99 {:.2} µs over {} samples ({files} files) — not staged",
            m.value,
            m.samples
        );
        return;
    }
    match record_measurement(&m) {
        Ok(path) => {
            let _ = writeln!(
                io::stderr(),
                "claim assemble.p99.file: p99 {:.2} µs over {} samples ({} files) → {}",
                m.value,
                m.samples,
                inputs.len(),
                path.display()
            );
        }
        Err(e) => eprintln!("claim assemble.p99.file staging failed: {e}"),
    }
}

criterion_group!(benches, bench_assemble);

fn main() {
    let smoke = std::env::args().any(|a| a == "--test");
    benches();
    Criterion::default().configure_from_args().final_summary();
    record_assemble_p99(!smoke);
}
