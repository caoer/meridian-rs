//! Shared e2e-fixture helpers for the registry integration suite.
//!
//! Each integration-test binary compiles this module separately and uses a
//! different subset of it, so an unused helper here is the normal case —
//! hence the module-wide allow.
//!
//! # Fixture rule: a fixture that starts a daemon owns its teardown
//!
//! Both halves of this rule used to be checked by reading source TEXT
//! (`tests/fixture_drain_budget.rs`, card `registry-tests-drain-residue`).
//! Both are now structural, and the guard is deleted — the point was never to
//! check the invariant better, it was to stop the invariant from being
//! breakable (card `fixture-drain-guard-followups`).
//!
//! 1. **Raise the budget — or say so in the environment.** A fixture that
//!    builds its own `Config` sets `config.drain_cold_builds =
//!    Duration::from_secs(30)`. A fixture that only SPAWNS an `mrd` whose
//!    daemon auto-starts never holds a `Config` at all, and sets
//!    `MRD_DRAIN_COLD_BUILDS=30` in that process's environment instead.
//!    `registry::DEFAULT_DRAIN_COLD_BUILDS` is 2 s for two reasons, and the
//!    second is the one usually missed: it must stay under the tightest
//!    CLIENT flock/respawn budget (mrd CLI `SPAWN_READY_TIMEOUT`, 5 s), AND
//!    it is exactly the engine kicker's own `COLD_BUILD_WAIT`
//!    (`crates/registry/src/registry.rs:171`). The ceiling says how large it
//!    may not be; `COLD_BUILD_WAIT` says why it is precisely 2. Neither is a
//!    fixture budget: on a loaded box a cold build parks well past 2 s, at
//!    which point shutdown gives up and the `TempDir` goes away under the
//!    builder.
//!    Enforced at runtime by a `debug_assert` on the hazard PAIR — cache root
//!    under `std::env::temp_dir()` AND the production budget — in BOTH
//!    `RunningServer::start` and, because an auto-spawned daemon's stderr is
//!    `Stdio::null()` and its panic reaches nobody, in the client before it
//!    spawns (`registry::Config::drain_budget_hazard`).
//! 2. **Own the teardown order — by holding ONE field.** Use
//!    `registry::TestServer`: it owns the server and the `TempDir` together
//!    and stops the server in its own `Drop::drop`, which Rust runs before any
//!    of its own fields' drop glue. The order is therefore not a rule to
//!    remember but a property of the type, and a fixture holding one field has
//!    no order of its own to get wrong. (The old rule — "declare the server
//!    first, because struct fields drop in declaration order and locals in
//!    reverse" — is why this type exists; it cost three hand-fixed
//!    inversions.)
//!
//! The same rule governs `crates/mrd/tests/`.
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
