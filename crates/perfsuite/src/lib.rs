//! Perf harness: the pipeline is **recipe → generated corpus → seam benches →
//! claim verdicts → three renderings** (human `RESULTS.md`, agent
//! `latest.json`, append-only dated run JSONs).
//!
//! # Charter
//! **Owns:** the perf truth. `corpusgen` (profile-driven, seeded, deterministic
//! markdown generator — corpora are cached under the per-user meridian cache
//! root (`corpus::corpora_root`), never committed and never in-tree (an
//! in-tree corpus enters the repo workspace's hash domain and balloons the
//! resident daemon); a recipe file IS the corpus), the claims registry
//! ([`claims`] — every perf claim is data in `claims.toml`, joined to
//! measurements into PASS/FAIL/MEASURED/UNTESTED verdicts), the hdrhistogram
//! p99 measurement path ([`measure`] — criterion sampling underestimates
//! tails; `Budget{class, p99_us}` claims go through here), and run reports
//! ([`report`]).
//!
//! **Never does:** define ground truth (the frozen GT pack in `testsuite` is
//! the only byte-exact truth; generated corpora carry construct *inventories*,
//! a lower bound — see [`gen`]'s recognizable-context invariant), ship in the
//! everyday build path (out of `default-members`), or gate on wall time
//! measured on shared runners (the perf lane runs on a pinned fleet host).
//!
//! # Rungs
//! Day 1: harness + placeholder seam benches; `transport.codec` claims measure
//! live (`NdjsonCodec` is implemented). Rung 1 flips the four parse claims from
//! UNTESTED (baselines: the prior `rust-pulldown` parse baseline — 103 MB/s corpus,
//! 3.2 ms p99/file, 473 ms monster p99, 0.22 ms reparse). Rung 6 wires policy
//! per-assertion budgets from ruleset data (`threshold_source = "ruleset"`).

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    reason = "measurement harness: lossy numeric casts are inherent to stats plumbing; per-fn error/panic/must-use annotations add noise to internal tooling"
)]

pub mod claims;
pub mod corpus;
pub mod generator;
pub mod inventory;
pub mod measure;
pub mod profile;
pub mod report;
