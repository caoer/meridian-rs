//! Rung-1 bench scaffold: corpus-sweep throughput lands here (baseline 103 MB/s,
//! 2.16 s cold rebuild). Placeholder measures harness overhead until
//! `syntax::parse` has a body.

use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;

fn parse_placeholder(c: &mut Criterion) {
    c.bench_function("noop-until-rung-1", |b| {
        b.iter(|| black_box("# placeholder\n").len())
    });
}

criterion_group!(benches, parse_placeholder);
criterion_main!(benches);
