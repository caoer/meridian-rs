//! Perf harness: recipe → generated corpus → seam benches → claim verdicts →
//! three renderings (human `RESULTS.md`, agent `latest.json`, append-only
//! dated run JSONs).
//!
//! Owns the perf truth: `corpusgen` (profile-driven, seeded, deterministic
//! markdown generator; corpora are cached under `corpus::corpora_root`, never
//! committed — an in-tree corpus enters the repo workspace's hash domain and
//! balloons the resident daemon; a recipe file IS the corpus), the claims
//! registry ([`claims`] — every perf claim is data in `claims.toml`, joined
//! to measurements into PASS/FAIL/MEASURED/UNTESTED verdicts), the
//! hdrhistogram p99 measurement path ([`measure`] — criterion sampling
//! underestimates tails), and run reports ([`report`]).
//!
//! Never defines ground truth (the frozen GT pack in `testsuite` is the only
//! byte-exact truth; generated corpora carry construct *inventories*, a lower
//! bound), ships in the everyday build path (out of `default-members`), or
//! gates on wall time measured on shared runners (the perf lane runs on a
//! pinned fleet host).

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
