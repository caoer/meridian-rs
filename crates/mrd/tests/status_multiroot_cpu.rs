//! The perf lane's multi-root profile — the shape `status_walltime.rs`
//! cannot see: its fixture sets `HOME` to a bare temp dir, so the machine
//! mount table is empty and the eager loader has no roots to walk. This
//! target measures `mrd status` with a populated mount table: four declared
//! roots, each a dirent-heavy corpus, none named by any lock address in the
//! workspace under test — status must build none of those corpora and pay
//! the ambient workspace only.

use std::time::{Duration, Instant};

mod multiroot_fixture;
use multiroot_fixture as fixture;

/// The CPU budget — measured, never copied from the 1000 ms wall budget next
/// door, which bounds a different verb shape on a different clock.
const CPU_BUDGET: Duration = Duration::from_millis(600);

#[test]
fn status_cpu_under_budget_with_a_populated_mount_table() {
    let sb = fixture::sandbox();

    // Roots, workspace and anti-blindness assert all live in
    // [`multiroot_fixture`], so the three multi-root CPU gates measure through
    // one table.
    let names = fixture::plant_declared_roots(&sb);
    let ws = fixture::init_workspace(&sb);
    fixture::assert_table_is_populated(&sb, &ws, &names);

    // Warm the page cache with a throwaway run, then measure the next one.
    let _ = fixture::run(&sb, &ws, &["status"]);
    let cpu_before = fixture::children_cpu();
    let wall_start = Instant::now();
    let out = fixture::run(&sb, &ws, &["status"]);
    let wall = wall_start.elapsed();
    // `RUSAGE_CHILDREN` is cumulative and monotonic; the checked form says so
    // rather than trusting it — an underflow would print as an under-budget
    // PASS.
    let cpu = fixture::children_cpu()
        .checked_sub(cpu_before)
        .expect("children CPU is cumulative, so it never goes backwards");

    let code = out.status.code().unwrap_or(-1);
    assert!(
        code == 0 || code == 1,
        "status ran: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    eprintln!(
        "status over {} declared roots ({} dirs, {} pages unread): \
         CPU {} ms (budget {} ms), wall {} ms (recorded, NOT gated)",
        fixture::ROOTS,
        fixture::ROOTS * fixture::DIRS_PER_ROOT,
        fixture::ROOTS * fixture::PAGES_PER_ROOT,
        cpu.as_millis(),
        CPU_BUDGET.as_millis(),
        wall.as_millis(),
    );
    assert!(
        cpu < CPU_BUDGET,
        "status must build only the mount roots its own lock addresses name. \
         With {} declared roots that no lock addresses, it burned {} ms of \
         CPU against a {} ms budget — the eager-loader shape (W2).",
        fixture::ROOTS,
        cpu.as_millis(),
        CPU_BUDGET.as_millis(),
    );

    // Perf-lane hygiene: any auto-spawned resident dies with the sandbox, not
    // after it.
    fixture::teardown_daemon(&sb);
}
