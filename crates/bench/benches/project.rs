//! Seam bench: `wire_map::project` — the named model→wire projection seam
//! (review C1): tree-flatten + `text_prefix_16b` + node ordering.
//!
//! # Metric contract (claims.toml)
//! - `project.p99.file` — per-document projection latency p99 (hdr path) over
//!   `vault-2026 seed=1 files=2000`; baseline TBD on first fleet run.
//!
//! Dormant until rung 1 (`wire_map::project` is `todo!()`). When it lands:
//! build documents in setup (parse + assemble), bench `project(&doc)` per
//! document, record `project.p99.file` via `bench::measure`.

use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

fn bench_project(c: &mut Criterion) {
    c.bench_function("project/noop_until_rung_1", |b| {
        b.iter(|| black_box(1u64).wrapping_add(1));
    });
}

criterion_group!(benches, bench_project);
criterion_main!(benches);
