//! **Ephemeral `mrd sql` pays for the roots ambient wikilink/embed targets NAME.**
//!
//! W5 flagged `sql.rs`'s eager `load_mounts()` as unmeasured and cautioned that the
//! call site *names* a deliberate S3-R59 assembly (pin plane + link plane, one
//! owner) — so it was not assumed to be the same defect. Measured on the
//! multi-root table with a workspace that names **zero** roots: ~27 s CPU, same
//! shape as the links residual, against ~0.17 s for the already-narrowed
//! walk/status siblings. The S3-R59 comment is about **one loader owner**, not
//! about loading every declared root's corpus. The view projects ambient docs
//! only; mounted pages exist so `resolve_ref` can land a rooted spelling.
//!
//! # Tier-4 bare is the residual
//! A git-root workspace can degrade into a cold last-published `view.duckdb` and
//! never rebuild; a bare (unregistered) cwd always takes `:memory:` and always
//! hits `build_and_run_ephemeral` → `load_mounts_for`. That is the path under
//! test.
//!
//! # Negative control
//! Point `MRD_BIN` at an engine that still calls `load_mounts()` in
//! `build_and_run_ephemeral`, or restore that call site, and this target reddens.

use std::time::{Duration, Instant};

mod multiroot_fixture;
use multiroot_fixture as fixture;

const CPU_BUDGET: Duration = Duration::from_millis(600);

#[test]
fn sql_ephemeral_cpu_under_budget_with_a_populated_mount_table() {
    let sb = fixture::sandbox();

    let names = fixture::plant_declared_roots(&sb);
    // Bare workspace — tier-4, always ephemeral `:memory:`. Still needs a cwd the
    // anti-blindness assert can run from; config is process-global via MERIDIAN_CONFIG.
    let bare = sb.tmp.path().join("bare");
    std::fs::create_dir_all(&bare).expect("bare");
    std::fs::write(
        bare.join("local.md"),
        "# Local\n\n## Body\n\nthe page the claim draws from.\n",
    )
    .expect("local page");
    std::fs::write(
        bare.join("claim.md"),
        "# Claim\n\n## Body\n\nsee [[local]].\n",
    )
    .expect("claim page");

    // Anti-blindness needs a workspace `mrd` accepts as cwd; bare is fine for config.
    fixture::assert_table_is_populated(&sb, &bare, &names);

    let _ = fixture::run(&sb, &bare, &["sql", "SELECT count(*) FROM doc", "--json"]);
    let cpu_before = fixture::children_cpu();
    let wall_start = Instant::now();
    let out = fixture::run(&sb, &bare, &["sql", "SELECT count(*) FROM doc", "--json"]);
    let wall = wall_start.elapsed();
    let cpu = fixture::children_cpu()
        .checked_sub(cpu_before)
        .expect("children CPU is cumulative, so it never goes backwards");

    let code = out.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        code == 0,
        "sql ran (exit {code}): stdout={stdout} stderr={stderr}"
    );
    assert!(
        stdout.contains("\"row_count\": 1") || stdout.contains("\"row_count\":1"),
        "sql must return the count row; got:\n{stdout}\n{stderr}"
    );

    eprintln!(
        "sql ephemeral over {} declared roots ({} dirs, {} pages unread): \
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
        "ephemeral sql must build only the mount roots ambient wikilink/embed targets name. \
         With {} declared roots that no ambient link addresses, it burned {} ms of CPU \
         against a {} ms budget — the eager-loader residual W5 flagged in sql.rs.",
        fixture::ROOTS,
        cpu.as_millis(),
        CPU_BUDGET.as_millis(),
    );
}
