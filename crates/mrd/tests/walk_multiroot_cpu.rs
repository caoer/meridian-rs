//! **`mrd walk` pays for the roots its addresses NAME, and for no others** (W5).
//!
//! W2 fixed this defect class in `mrd status` and deliberately left `walk` and
//! `check` alone, reasoning that their addresses *may traverse roots*, so the
//! needed set could not be a lock-item scan and the narrowing would have to be a
//! different design. **Measured, that reasoning is wrong, and the way it is
//! wrong is a structural fact worth pinning down here.**
//!
//! `mrd walk` never traverses INTO a root. `view::walk::forward_edges` maps
//! `corpus.ambient_docs()` alone, and a cross-root target is a LEAF by
//! construction — the ambient corpus holds no key spelled `root:path`, so the
//! BFS cannot expand one (`steps_from`, and the comment there saying so). A
//! mounted root's pages are therefore read for exactly one purpose: colouring an
//! edge that NAMES that root. That is precisely the set
//! `walk_cmd::lock_addressed_roots` collects, so status's narrowing is not
//! merely adaptable to walk — it is the same question with the same answer.
//!
//! # The measurement that made this a defect rather than a preference
//! Same workspace, same machine, same minute, four declared roots bound in the
//! machine's real `~/MERIDIAN.md` and none of them named by any lock:
//!
//! | verb | CPU before | CPU after |
//! |---|---:|---:|
//! | `status` (narrowed by W2) | 0.05 s | 0.05 s |
//! | `walk` | **5.48 s** | **0.03 s** |
//! | `check` | **5.48 s** | **0.06 s** |
//!
//! Two verbs of the same engine were 110× apart from their own sibling for the
//! whole life of the W2 fix, on the identical input.
//!
//! # The measure is CPU, not wall
//! The host these lanes run on is shared, and the W2 investigation measured the
//! consequence directly: `mrd read` burns 0.01 s of CPU and took 5.3 s of wall
//! under load 41, carrying no defect at all. Wall-clock on a contended host
//! measures the neighbours. CPU is the load-independent number, and CPU is what
//! this defect moves, because 72 % of its samples are `getdirentries64`/`open`.
//! `getrusage(RUSAGE_CHILDREN)` reads exactly that, for the child and no one
//! else, so this file asserts on user+sys and prints wall only for the record.
//!
//! # Negative control — how to redden this gate
//! Point `MRD_BIN` at an engine built before W5 (or restore
//! `load_mounts()` at the `mrd walk` call site) and run this target. The
//! measured red/green pair is recorded on the card; no source edit is needed to
//! reproduce it.

use std::time::{Duration, Instant};

mod multiroot_fixture;
use multiroot_fixture as fixture;

/// **The CPU budget.** See the module header for why it is CPU and not wall, and
/// [`multiroot_fixture`] for the table it is measured through. The budget is the
/// one `status_multiroot_cpu.rs` measured for the same fixture and the same
/// narrowing — walk's post-fix work is status's ambient build plus one BFS over
/// a corpus of two pages, which is smaller than the spread the budget already
/// carries. The arms measured on this host, release profile:
///
/// | arm | CPU |
/// |---|---:|
/// | narrowed (`load_mounts_for`) | 30 ms |
/// | eager (`load_mounts`, pre-W5) | 5 480 ms |
///
/// The arms are ~180× apart; 600 ms sits 20× above the fix and 9× below the
/// defect. The narrowed arm is INSENSITIVE to the fixture's corpus constants —
/// it never reads those roots at all — which is the invariant the budget stands
/// on, so the margin does not shrink as the fixture grows.
const CPU_BUDGET: Duration = Duration::from_millis(600);

#[test]
fn walk_cpu_under_budget_with_a_populated_mount_table() {
    let sb = fixture::sandbox();

    // Four declared roots — the input a `HOME`-in-a-temp-dir fixture deletes.
    let names = fixture::plant_declared_roots(&sb);
    let ws = fixture::init_workspace(&sb);
    fixture::assert_table_is_populated(&sb, &ws, &names);

    // The page under walk, and one page it draws from — so the walk has a real
    // edge to expand rather than terminating on an empty adjacency. Neither
    // names a root: this is the common case the narrowing must make cheap.
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
    // `RUSAGE_CHILDREN` is cumulative and monotonic, so the later read can only
    // be the larger one — but the checked form says so rather than trusting it,
    // and an underflow here would print as a wildly under-budget PASS.
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
