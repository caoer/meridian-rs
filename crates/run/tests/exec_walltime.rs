//! Bash step-end reaping wall-time budget — the PERF lane's gate, not the PR
//! lane's.
//!
//! `ci.yml` states the law this file obeys: *"Wall-time numbers are NEVER gated
//! in this lane (shared-runner noise); that's perf.yml's job on the pinned fleet
//! runner."* While the `elapsed < 5s` assert lived in
//! `run/tests/exec.rs::a_background_child_is_reaped_at_step_end` that law was
//! contradicted — `cargo test --workspace` gated a wall-clock number on whatever
//! machine happened to run it. Two failures under host load this session (U13
//! worker during a build-load suite; w6 worker at load ~20): both measured the
//! background-child reaping path past 5s (one recorded at **15.03s**) while the
//! same binary in isolation stayed green 3/3 (~0.33s). The assert was measuring
//! contention, not the S3 reaping contract.
//!
//! So the wall-clock MOVED, and **the 5s budget did not move with it.** The
//! mechanism is `required-features = ["perf-walltime"]` in `crates/run/Cargo.toml`:
//! `cargo test --workspace` skips this target entirely (no wall-clock gate, and
//! no vacuous pass either), `ci.yml` still compiles and lints it so it cannot
//! bit-rot, and `perf.yml` runs it for real on the pinned runner.
//!
//! The correctness suite keeps the load-insensitive EVENT half of the same
//! scenario (`exec.rs::a_background_child_is_reaped_at_step_end`): step succeeds
//! and the post-step leak file is absent. This file gates only the duration.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use run::exec::{self, ExecSpec};

#[test]
fn a_background_child_is_reaped_within_the_wall_budget() {
    // Same scenario as the correctness-lane event gate in exec.rs — background
    // writer holding pipes for 15s, parent exits 0. Without group SIGKILL the
    // supervisor waits the full 15s; with it the step ends now. The 5s ceiling
    // is the budget that left the correctness suite; it is not relaxed here.
    let tmp = tempfile::tempdir().unwrap();
    let env = BTreeMap::new();
    let started = Instant::now();
    let src = format!(
        "( sleep 15; echo leaked > '{}/leak.txt'; echo late ) & exit 0",
        tmp.path().display()
    );
    let r = exec::exec(&ExecSpec {
        source: &src,
        args: &[],
        env: &env,
        scratch: tmp.path(),
        project_root: tmp.path(),
        timeout: Duration::from_secs(30),
    })
    .unwrap();
    assert!(r.status.success());
    let elapsed = started.elapsed();
    eprintln!("exec background-reap wall: {elapsed:?}");
    assert!(
        elapsed < Duration::from_secs(5),
        "step end must not wait for the background child: {elapsed:?}"
    );
}
