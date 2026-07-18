//! Seam bench: `policy::evaluate` — rung-6 per-assertion p99 enforcement.
//!
//! # Engine shape (ZT ruling 2026-07-18): declarative spine, Lua predicates
//! Rules are two-layer — a declarative manifest (id, scope selector, severity,
//! budget, fixtures) and an mlua predicate `check(node, facts) → verdict`,
//! sandboxed Redis-style (no io/os/clock/random, fuel-limited, memory-capped,
//! ordered fact arrays only). That splits budget enforcement in two, mirroring
//! this harness's criterion-vs-hdr split:
//!
//! - **Fuel budgets** — deterministic instruction counts, engine-enforced at
//!   eval time: same bytes + same facts ⇒ same fuel, forever. Platform-free,
//!   so fuel checks are *tests* (testsuite territory), not benches.
//! - **Wall-time p99** — `Budget{class, p99_us}` per rule, real latency on
//!   real hardware: THIS bench's job, via the hdr path on the fleet runner.
//!
//! # Metric contract (claims.toml)
//! - `policy.assertion.p99` — µs per predicate evaluation, p99.
//!   `threshold_source = "ruleset"`: gates come from each rule's manifest
//!   budget, never from claims.toml. Enforcement shape when rung 6 lands: one
//!   hdr run per rule id over corpus-derived facts, each asserted against its
//!   own declared `p99_us`; any overrun is a FAIL verdict.
//! - `policy.pack_load.fixtures` — ms to load a pack and run every rule's
//!   fixtures (a rule whose fixtures fail doesn't load). This is the
//!   "rule iteration in milliseconds" promise, held as a perf claim.
//!
//! Dormant until rung 6 (`policy::compile`/`evaluate`/`vocab` are `todo!()`).

use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

fn bench_policy(c: &mut Criterion) {
    c.bench_function("policy/noop_until_rung_6", |b| {
        b.iter(|| black_box(1u64).wrapping_add(1));
    });
}

criterion_group!(benches, bench_policy);
criterion_main!(benches);
