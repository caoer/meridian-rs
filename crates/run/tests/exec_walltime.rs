//! Bash step-end reaping wall-time budget — PERF lane only, not the PR lane.
//!
//! `ci.yml`: wall-time numbers are NEVER gated on shared runners; that is
//! `perf.yml`'s job on the pinned fleet. This target requires `perf-walltime`:
//! `cargo test --workspace` skips it (no wall-clock gate, no vacuous pass);
//! `ci.yml` still compiles/lints it; `perf.yml` runs it for real.
//!
//! Correctness keeps the load-insensitive EVENT half
//! (`exec.rs::a_background_child_is_reaped_at_step_end`): step succeeds and
//! the post-step leak file is absent. This file gates only the duration (5s
//! budget — relocation from correctness, not relaxation).

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
