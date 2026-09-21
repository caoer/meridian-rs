//! Ephemeral `mrd sql` pays only for the roots ambient wikilink/embed targets
//! name. The view projects ambient docs only; mounted pages exist so
//! `resolve_ref` can land a rooted spelling.
//!
//! A git-anchored workspace with no cache root takes the `:memory:` lane.
//! Its explicit mount configuration stays populated while `HOME` and
//! `XDG_CACHE_HOME` are empty, so no persistent drawer can answer the query.
//!
//! Negative control: restore an eager `load_mounts()` in
//! `load_corpus` and this target reddens.

use std::path::Path;
use std::process::Output;
use std::time::{Duration, Instant};

mod common;
mod multiroot_fixture;
use multiroot_fixture as fixture;

const CPU_BUDGET: Duration = Duration::from_millis(600);

fn run_ephemeral(sb: &fixture::Sandbox, ws: &Path) -> Output {
    fixture::command(sb, ws, &["sql", "SELECT count(*) FROM doc", "--json"])
        .env("HOME", "")
        .env("XDG_CACHE_HOME", "")
        .output()
        .expect("spawn ephemeral sql")
}

#[test]
fn sql_ephemeral_cpu_under_budget_with_a_populated_mount_table() {
    let sb = fixture::sandbox();

    let names = fixture::plant_declared_roots(&sb);
    // Resolution requires a declared workspace. The .git anchor admits it;
    // run_ephemeral removes cache roots while preserving MERIDIAN_CONFIG.
    let bare = sb.tmp.path().join("bare");
    std::fs::create_dir_all(bare.join(".git")).expect("workspace anchor");
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

    // Prove the mounted roots remain declared and bound before measuring.
    fixture::assert_table_is_populated(&sb, &bare, &names);

    let _ = run_ephemeral(&sb, &bare);
    let cpu_before = fixture::children_cpu();
    let wall_start = Instant::now();
    let out = run_ephemeral(&sb, &bare);
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
    let body: serde_json::Value = serde_json::from_str(&stdout).expect("SQL JSON frame");
    assert_eq!(
        body["row_count"], 1,
        "SQL must return the count row: {body}"
    );
    assert_eq!(
        body["rows"],
        serde_json::json!([[2]]),
        "the query must see both ambient pages and no mounted pages: {body}"
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
