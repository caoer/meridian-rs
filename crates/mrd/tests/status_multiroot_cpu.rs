//! **The perf lane's multi-root profile — the shape `status_walltime.rs` cannot see.**
//!
//! `status_walltime.rs` gates 1000 ms and passed green for the whole life of the
//! W2 defect, while the real verb took 20-22 s in the field. The reason is its
//! fixture, not its budget: it sets `HOME` to a bare temp dir, so the machine
//! mount table is EMPTY, so the eager loader had no roots to walk. **The gate
//! removed the input that dominated the cost and then reported a bound it had
//! never tested.** A perf gate whose fixture deletes the dominant input is not a
//! slow gate — it is a green light with no lamp behind it.
//!
//! So this target measures `mrd status` with a **populated** mount table: four
//! declared roots, each a dirent-heavy corpus, none of them named by any lock
//! address in the workspace under test. Post-W2 (`load_mounts_for` +
//! `lock_addressed_roots`, `a8fdb356`) status builds none of those four corpora
//! and pays the ambient workspace only. Pre-W2 it built all four.
//!
//! # The measure is CPU, not wall
//! The host this lane runs on is shared, and the W2 investigation measured the
//! consequence directly: `mrd read` burns 0.01 s of CPU and took 5.3 s of wall
//! under load 41, carrying no defect at all. Wall-clock on a contended host
//! measures the neighbours. The load-independent number in the same
//! investigation is CPU — 9.7 s → 1.4 s across the fix — and CPU is what the
//! defect moves, because 72 % of its samples are `getdirentries64`/`open`.
//! `getrusage(RUSAGE_CHILDREN)` reads exactly that, for the child and no one
//! else, so this file asserts on user+sys and prints wall only for the record.
//!
//! # Negative control — how to redden this gate
//! In `crates/mrd/src/status_cmd.rs`, replace
//!
//! ```text
//!     let mounts = crate::walk_cmd::load_mounts_for(&lock_addressed_roots(&docs));
//! ```
//!
//! with the pre-W2 eager call, `crate::walk_cmd::load_mounts()`, and run this
//! target. That is the one-line reintroduction of the defect class, and the
//! measured red/green pair is recorded on the card.

use std::time::{Duration, Instant};

mod multiroot_fixture;
use multiroot_fixture as fixture;

/// **The CPU budget.** See the module header for why it is CPU and not wall.
/// MEASURED, never copied forward from the 1000 ms wall budget next door — that
/// number bounds a different verb shape on a different clock, and inheriting it
/// would be the same unasked question this file exists to ask.
///
/// The two arms, debug profile, one host, the one-line negative control:
///
/// | arm | CPU |
/// |---|---:|
/// | narrowed (`load_mounts_for`) | 186 / 189 / 208 ms over three runs |
/// | eager (`load_mounts`, W2 restored) | 2733 ms |
///
/// The arms are 13× apart and 600 ms sits inside that gap: 3× of headroom over
/// the fix's spread, 4.6× under the defect. The narrowed arm is also INSENSITIVE
/// to the corpus constants — 186 ms measured at both 8,000 and 100,000
/// declared-root directories — which is the invariant the budget stands on, so
/// the margin does not shrink as the fixture grows.
const CPU_BUDGET: Duration = Duration::from_millis(600);

#[test]
fn status_cpu_under_budget_with_a_populated_mount_table() {
    let sb = fixture::sandbox();

    // Four declared roots — the input the sibling gate deletes — the workspace
    // under test, and the fixture's own anti-blindness assert. All three live in
    // [`multiroot_fixture`] since W5, so the three multi-root CPU gates measure
    // through ONE table and cannot drift into measuring three different ones.
    let names = fixture::plant_declared_roots(&sb);
    let ws = fixture::init_workspace(&sb);
    fixture::assert_table_is_populated(&sb, &ws, &names);

    // Warm the page cache with a throwaway run, then measure the next one.
    let _ = fixture::run(&sb, &ws, &["status"]);
    let cpu_before = fixture::children_cpu();
    let wall_start = Instant::now();
    let out = fixture::run(&sb, &ws, &["status"]);
    let wall = wall_start.elapsed();
    // `RUSAGE_CHILDREN` is cumulative and monotonic, so the later read can only
    // be the larger one — but the checked form says so rather than trusting it,
    // and an underflow here would print as a wildly under-budget PASS.
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
    // after it. `status` itself is pure-local today; the Drop + explicit call
    // still close the class if a future probe starts a daemon.
    fixture::teardown_daemon(&sb);
}
