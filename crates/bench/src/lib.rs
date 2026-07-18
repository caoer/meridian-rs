//! Criterion benches (corpus sweep, per-assertion p99 enforcement) — out of
//! default-members.
//!
//! # Charter
//! **Owns:** the workspace's benchmarks: rung 1's corpus-sweep throughput
//! (baseline to beat: 103 MB/s lane, 2.16 s cold rebuild) and rung 6's
//! declared-p99 enforcement (a release exceeding an assertion's declared budget
//! fails CI here). Dedicated member so criterion-class dev-deps never pollute
//! the libraries — mirrors the pulldown-cmark fork's own bench-member layout.
//!
//! **Never does:** ship code, run in default builds (`cargo bench -p bench` is
//! the entry; the member sits outside `default-members`).
