//! Shared e2e-fixture helpers for the registry integration suite.
//!
//! Each integration-test binary compiles this module separately and uses a
//! different subset of it, so an unused helper here is the normal case —
//! hence the module-wide allow.
//!
//! # Fixture rule: a fixture that starts a daemon owns its teardown
//!
//! Two lines, both enforced by `tests/fixture_drain_budget.rs`, both the
//! class-2 flake (pipelines 1098/1101: `background drawer rebuild failed for
//! /tmp/.tmp…/ws (No such file or directory)`):
//!
//! 1. **Raise the budget.** Any fixture config passed to
//!    `RunningServer::start` sets `config.drain_cold_builds =
//!    Duration::from_secs(30)`. The `registry::DEFAULT_DRAIN_COLD_BUILDS`
//!    default is 2 s because it must stay under the tightest CLIENT
//!    flock/respawn budget (mrd CLI 5 s) — it is not a fixture budget, and on
//!    a loaded box a cold build parks well past it, at which point shutdown
//!    gives up and the `TempDir` goes away under the builder.
//! 2. **Order the fields.** A fixture that keeps the `TempDir` and the server
//!    in a STRUCT declares the **server first**. Struct fields drop in
//!    declaration order; locals drop in reverse. So the same two values that
//!    tear down correctly as `let tmp = …; let server = …;` tear down
//!    INVERTED as `struct F { _tmp: TempDir, server: RunningServer }`. A
//!    `_`-prefixed name signals "unused", never "drops last". The alternative
//!    is an explicit `impl Drop`.
//!
//! The same rule governs `crates/mrd/tests/`; the guard reads both trees.
#![allow(dead_code)]

use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// Client retry budget against `corpus_warming`.
///
/// 30 s, chosen to survive nyc-2 loadavg ~190 (pipelines 1098/1101). Not the
/// kicker's unpublished 2 s (`COLD_BUILD_WAIT`).
pub(crate) const WARM_BUDGET: Duration = Duration::from_secs(30);

/// True when this frame is the §3.2 `corpus_warming` refusal (`recovery: retry`).
pub(crate) fn is_warming(resp: &Value) -> bool {
    resp["ok"] != json!(true) && resp["error"]["code"] == json!("corpus_warming")
}

/// Run `op` until it is not `corpus_warming`, or [`WARM_BUDGET`] elapses.
///
/// **Fixture rule** (card `registry-tests-corpus-warming-under-load`): the
/// engine's `corpus_warming` / `recovery: retry` is the contract. A fixed 2s
/// wait is the kicker's unpublished bound, not a client deadline. Any
/// non-warming refusal is returned immediately — this is not a blanket retry.
pub(crate) fn honour_retry(mut op: impl FnMut() -> Value) -> Value {
    let started = Instant::now();
    loop {
        let resp = op();
        if !is_warming(&resp) {
            return resp;
        }
        assert!(
            started.elapsed() < WARM_BUDGET,
            "corpus_warming persisted past {WARM_BUDGET:?} (engine recovery: retry); last: {resp}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}
