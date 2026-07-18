//! Seam bench: `model::build` — dialect stream → governed tree.
//!
//! # Metric contract (claims.toml)
//! - `assemble.p99.file` — per-file assembly latency p99 (hdr path) over
//!   `vault-2026 seed=1 files=2000`; baseline TBD on first fleet run.
//!
//! Dormant until rung 1 (`model::build` is `todo!()`). When it lands: parse
//! each corpus file once in setup, bench `model::build(raw, nodes)` per file,
//! record `assemble.p99.file` via `perfsuite::measure` (pattern: benches/roundtrip.rs).

use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

fn bench_assemble(c: &mut Criterion) {
    c.bench_function("assemble/noop_until_rung_1", |b| {
        b.iter(|| black_box(1u64).wrapping_add(1));
    });
}

criterion_group!(benches, bench_assemble);
criterion_main!(benches);
