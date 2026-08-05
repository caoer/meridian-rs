//! **`mrd check` pays for the roots its addresses NAME, and for no others** (W5).
//!
//! The sibling of `walk_multiroot_cpu.rs`, and the same defect class W2 fixed in
//! `status` and left standing here. Read that file's header for why the needed
//! set turned out to be one computation for all three verbs; this one records
//! what is different about `check`.
//!
//! # `check` needed a REORDER, not just a narrower call
//! `status` and `walk` already held their corpus when they loaded the mount
//! table, so narrowing was a one-line change of call. `check` loaded the table
//! FIRST and built its corpus after, inside `assess` — and the roots worth
//! building are a question about that very corpus. So the corpora are now built
//! before the table, `assess` takes documents instead of bytes, and the parse
//! happens exactly once per interval rather than once to ask and once to answer.
//!
//! # And a UNION, because `check` has two intervals
//! `--staged` assesses a second corpus, the index's, against the SAME table
//! (F1). The staged bytes may pin a root the worktree's do not, so the needed
//! set is the union over both — a set computed per interval would have built the
//! table for the first and then read it for the second. Over-collecting costs a
//! corpus; under-collecting cannot pass silently, because the one consumer of a
//! skipped root refuses out loud and by name (`walk_cmd::load_mounts_for`).
//! `staged_covers_a_root_the_worktree_does_not` in
//! `check_staged_root_union.rs` is the gate on that union.
//!
//! # The measure is CPU, not wall
//! See `walk_multiroot_cpu.rs` § *The measure is CPU, not wall*. Same host, same
//! reasoning, same `getrusage(RUSAGE_CHILDREN)` instrument.
//!
//! # Negative control — how to redden this gate
//! Point `MRD_BIN` at an engine built before W5 (or restore `load_mounts()` at
//! the `mrd check` call site) and run this target. The measured red/green pair
//! is recorded on the card; no source edit is needed to reproduce it.

use std::time::{Duration, Instant};

mod multiroot_fixture;
use multiroot_fixture as fixture;

/// **The CPU budget.** See `walk_multiroot_cpu.rs` for the budget's derivation —
/// same fixture, same narrowing, same host. `check` does strictly more ambient
/// work than `walk` (the fold and the fence reading), and it measured 60 ms
/// against walk's 30 ms, so the same 600 ms bound carries 10× of headroom over
/// the fix and sits 9× under the 5 480 ms defect arm.
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
