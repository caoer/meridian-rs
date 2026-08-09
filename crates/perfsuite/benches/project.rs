//! Seam bench: `wire_map::project` — the named model→wire projection seam (review C1):
//! tree-flatten + `text_prefix_16b` + node ordering.
//!
//! # Metric contract (claims.toml)
//! - `project.p99.file` — per-document projection latency p99 (hdr path) over `vault-2026
//!   seed=1 files=2000`; baseline TBD on first fleet run.
//!
//! UNWIRED: perfsuite carries no `wire-map` dev-dependency. The claim row
//! stays registered and UNTESTED — register the gate, never fabricate a PASS.
//! To wire it: add `wire-map = { workspace = true }` to `[dev-dependencies]`,
//! then mirror `benches/policy.rs` — build documents in setup, then
//! `LatencyRun::run(500, 8_000, || black_box(wire_map::project(doc)))` and
//! stage `project.p99.file` as `us`.
//!
//! No criterion function: a noop bench would publish a 1 ns estimate under
//! this seam's name, which reads as a measurement of the projection and is
//! not one.

fn main() {
    eprintln!(
        "project bench unwired: wire_map::project landed, but perfsuite has no \
         wire-map dev-dependency (see this file's header)"
    );
}
