//! `mrd check` pays for the roots its addresses name, and for no others (W5). Sibling of
//! `walk_multiroot_cpu.rs` — same fixture, same narrowing. `check` builds its corpora before
//! the mount table, `assess` takes documents instead of bytes, and the parse happens exactly
//! once per interval.

use std::time::{Duration, Instant};

mod multiroot_fixture;
use multiroot_fixture as fixture;

/// The CPU budget. See `walk_multiroot_cpu.rs` for the derivation — same fixture, same host.
/// `check` measured 60 ms against walks 30 ms, so the same 600 ms bound carries 10× headroom
/// over the fix and sits 9× under the 5 480 ms defect arm.
const CPU_BUDGET: Duration = Duration::from_millis(600);

#[test]
fn check_cpu_under_budget_with_a_populated_mount_table() {
    let sb = fixture::sandbox();

    let names = fixture::plant_declared_roots(&sb);
    let ws = fixture::init_workspace(&sb);
    fixture::assert_table_is_populated(&sb, &ws, &names);

    // An ordinary page, naming no root — the common case.
    std::fs::write(ws.join("claim.md"), "# Claim\n\n## Body\n\nordinary.\n").expect("claim page");

    // Warm the page cache with a throwaway run, then measure the next one.
    let _ = fixture::run(&sb, &ws, &["check"]);
    let cpu_before = fixture::children_cpu();
    let wall_start = Instant::now();
    let out = fixture::run(&sb, &ws, &["check"]);
    let wall = wall_start.elapsed();
    let cpu = fixture::children_cpu()
        .checked_sub(cpu_before)
        .expect("children CPU is cumulative, so it never goes backwards");

    let code = out.status.code().unwrap_or(-1);
    assert!(
        code == 0 || code == 1,
        "check ran: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    eprintln!(
        "check over {} declared roots ({} dirs, {} pages unread): \
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
        "check must build only the mount roots its own lock addresses name. \
         With {} declared roots that no lock addresses, it burned {} ms of CPU \
         against a {} ms budget — the eager-loader shape (W2's defect class, \
         left in check until W5).",
        fixture::ROOTS,
        cpu.as_millis(),
        CPU_BUDGET.as_millis(),
    );

    // Perf-lane hygiene: reap any resident before the sandbox dies.
    fixture::teardown_daemon(&sb);
}
