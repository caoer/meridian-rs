//! Shared e2e-fixture helpers for the registry integration suite.
//!
//! Each integration-test binary compiles this module separately and uses a
//! different subset of it, so an unused helper here is the normal case —
//! hence the module-wide allow.
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
