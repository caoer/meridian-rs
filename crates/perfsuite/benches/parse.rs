//! Seam bench: `syntax::parse` — the corpus-sweep lane.
//!
//! # Metric contract (claims.toml)
//! - `parse.throughput.corpus` — MB/s over `vault-2026 seed=1 files=2000`;
//!   baseline 103 MB/s, gate ≥ 82 (parser-bench rust-pulldown lane).
//! - `parse.p99.file` — per-file latency p99 via the hdr path; baseline 3.2 ms.
//! - `parse.p99.monster_10mb` — recipe `monster-10mb`; baseline 473 ms.
//! - `parse.reparse.full_46kb` — single-file reparse; baseline 0.22 ms.
//!
//! Until rung 1 gives `syntax::parse` a body, this measures corpus traversal
//! only (harness overhead floor); the claims stay UNTESTED. When rung 1 lands:
//! swap the loop body to `syntax::parse(text)` and record the four claims via
//! `perfsuite::measure::LatencyRun` + `record_measurement` (see benches/roundtrip.rs
//! for the live pattern).

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;

use perfsuite::corpus;
use perfsuite::profile::{Profile, Recipe};

/// Criterion working set: 200 files keeps dev iteration snappy; claim runs
/// use the full files=2000 recipe on the perf lane.
fn working_set() -> Vec<(String, String)> {
    let profile = Profile::resolve("vault-2026").expect("bundled profile loads");
    let recipe = Recipe::new(profile, 1, Some(200));
    let dir = corpus::ensure(&recipe).expect("corpus generates");
    corpus::load_files(&dir).expect("corpus loads")
}

fn bench_parse(c: &mut Criterion) {
    let files = working_set();
    let total: u64 = files.iter().map(|(_, text)| text.len() as u64).sum();
    let mut group = c.benchmark_group("parse");
    group.throughput(Throughput::Bytes(total));
    group.bench_function("placeholder_corpus_traversal", |b| {
        b.iter(|| {
            let mut n = 0usize;
            for (_, text) in &files {
                n += black_box(text.as_str()).len();
            }
            n
        });
    });
    group.finish();
}

criterion_group!(benches, bench_parse);
criterion_main!(benches);
