//! **`mrd links` pays for the roots its wikilink/embed targets NAME, and for no others.**
//!
//! W5 narrowed `walk`/`check`/`status` and deliberately left `engine.rs`'s ephemeral
//! links path alone (unmeasured). Measured on the multi-root table with a workspace
//! that names **zero** roots, the residual burned ~27 s CPU — the same O(declared
//! roots) shape W2/W5 fixed — while the already-narrowed siblings sat at ~0.17 s on
//! the identical input. The needed set is not a lock-item scan: it is
//! `walk_cmd::link_addressed_roots` over ambient wikilink/embed targets. The table
//! itself is never narrowed.
//!
//! # The measure is CPU, not wall
//! Same law as `walk_multiroot_cpu.rs` / `status_multiroot_cpu.rs`: host contention
//! inflates wall; `getrusage(RUSAGE_CHILDREN)` attributes user+sys to the child.
//!
//! # Negative control
//! Point `MRD_BIN` at an engine that still calls `load_mounts()` in
//! `in_process_links`, or restore that call site, and this target reddens.

use std::time::{Duration, Instant};

mod multiroot_fixture;
use multiroot_fixture as fixture;

/// Same measured budget the W5 multi-root gates carry for this fixture shape.
const CPU_BUDGET: Duration = Duration::from_millis(600);

#[test]
fn links_cpu_under_budget_with_a_populated_mount_table() {
    let sb = fixture::sandbox();

    let names = fixture::plant_declared_roots(&sb);
    let ws = fixture::init_workspace(&sb);
    fixture::assert_table_is_populated(&sb, &ws, &names);

    // Ambient-only edges — the common case the narrowing must make cheap. No root
    // spelling, so `link_addressed_roots` is empty and no root corpus may be built.
    std::fs::write(
        ws.join("local.md"),
        "# Local\n\n## Body\n\nthe page the claim draws from.\n",
    )
    .expect("local page");
    std::fs::write(
        ws.join("claim.md"),
        "# Claim\n\n## Body\n\nsee [[local]].\n",
    )
    .expect("claim page");

    let _ = fixture::run(&sb, &ws, &["links", "--json"]);
    let cpu_before = fixture::children_cpu();
    let wall_start = Instant::now();
    let out = fixture::run(&sb, &ws, &["links", "--json"]);
    let wall = wall_start.elapsed();
    let cpu = fixture::children_cpu()
        .checked_sub(cpu_before)
        .expect("children CPU is cumulative, so it never goes backwards");

    let code = out.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        code == 0,
        "links ran (exit {code}): stdout={stdout} stderr={stderr}"
    );
    assert!(
        stdout.contains("\"source\": \"ephemeral\"") || stdout.contains("\"source\":\"ephemeral\""),
        "links must take the ephemeral path (the residual under test); got:\n{stdout}\n{stderr}"
    );

    eprintln!(
        "links over {} declared roots ({} dirs, {} pages unread): \
         CPU {} ms (budget {} ms), wall {} ms (recorded, NOT gated)",
        fixture::ROOTS,
        fixture::ROOTS * fixture::DIRS_PER_ROOT,
        fixture::ROOTS * fixture::PAGES_PER_ROOT,
        cpu.as_millis(),
        CPU_BUDGET.as_millis(),
        wall.as_millis(),
    );
    fixture::teardown_daemon(&sb);
    assert!(
        cpu < CPU_BUDGET,
        "links must build only the mount roots its own wikilink/embed targets name. \
         With {} declared roots that no ambient link addresses, it burned {} ms of CPU \
         against a {} ms budget — the eager-loader residual W5 flagged in engine.rs.",
        fixture::ROOTS,
        cpu.as_millis(),
        CPU_BUDGET.as_millis(),
    );
}
