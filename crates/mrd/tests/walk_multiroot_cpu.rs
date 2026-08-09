//! `mrd walk` pays for the roots its addresses name, and for no others (W5).
//!
//! `mrd walk` never traverses INTO a root: `view::walk::forward_edges` maps
//! `corpus.ambient_docs()` alone, and a cross-root target is a leaf by
//! construction — the ambient corpus holds no key spelled `root:path`, so the
//! BFS cannot expand one. A mounted root's pages are read for exactly one
//! purpose: colouring an edge that names that root, which is the set
//! `walk_cmd::lock_addressed_roots` collects.
//!
//! The measure is CPU, not wall: wall-clock on a contended host measures the
//! neighbours, and this defect class is dominated by directory I/O.
//! `getrusage(RUSAGE_CHILDREN)` reads user+sys for the child alone; wall is
//! printed only for the record.
//!
//! Negative control: restore `load_mounts()` at the `mrd walk` call site and
//! this target reddens.

use std::time::{Duration, Instant};

mod multiroot_fixture;
use multiroot_fixture as fixture;

/// The CPU budget — the one `status_multiroot_cpu.rs` measured for the same
/// fixture and the same narrowing; walk's post-fix work is status's ambient
/// build plus one BFS over a two-page corpus.
const CPU_BUDGET: Duration = Duration::from_millis(600);

#[test]
fn walk_cpu_under_budget_with_a_populated_mount_table() {
    let sb = fixture::sandbox();

    // Four declared roots — the input a `HOME`-in-a-temp-dir fixture deletes.
    let names = fixture::plant_declared_roots(&sb);
    let ws = fixture::init_workspace(&sb);
    fixture::assert_table_is_populated(&sb, &ws, &names);

    // The page under walk, and one page it draws from — so the walk has a real edge to expand
    // rather than terminating on an empty adjacency. Neither names a root: this is the common case
    // the narrowing must make cheap.
    std::fs::write(
        ws.join("target.md"),
        "# Target\n\n## Design\n\nthe page the claim draws from.\n",
    )
    .expect("target page");
    std::fs::write(
        ws.join("claim.md"),
        "# Claim\n\n## Body\n\nsee [[target]].\n",
    )
    .expect("claim page");

    // Warm the page cache with a throwaway run, then measure the next one.
    let _ = fixture::run(&sb, &ws, &["walk", "claim.md"]);
    let cpu_before = fixture::children_cpu();
    let wall_start = Instant::now();
    let out = fixture::run(&sb, &ws, &["walk", "claim.md"]);
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
        "walk ran: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    eprintln!(
        "walk over {} declared roots ({} dirs, {} pages unread): \
         CPU {} ms (budget {} ms), wall {} ms (recorded, NOT gated)",
        fixture::ROOTS,
        fixture::ROOTS * fixture::DIRS_PER_ROOT,
        fixture::ROOTS * fixture::PAGES_PER_ROOT,
        cpu.as_millis(),
        CPU_BUDGET.as_millis(),
        wall.as_millis(),
    );
    // Measurement done — reap any resident before the sandbox dies (perf hygiene).
    fixture::teardown_daemon(&sb);
    assert!(
        cpu < CPU_BUDGET,
        "walk must build only the mount roots its own lock addresses name. \
         With {} declared roots that no lock addresses, it burned {} ms of CPU \
         against a {} ms budget — the eager-loader shape (W2's defect class, \
         left in walk until W5).",
        fixture::ROOTS,
        cpu.as_millis(),
        CPU_BUDGET.as_millis(),
    );
}
