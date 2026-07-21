//! `mrd cache ls` and `mrd cache clean` — drawer listing and explicit sweeps.
//!
//! Both read the reverse map from the drawer SENTINELS (the sole authority for
//! hash → workspace path, amendment C3) via [`cache::list_drawers`]. `ls` is a
//! pure read; `clean` reaps stale-by-last-use drawers (skipping any a live
//! workspace holds locked), drawers whose workspace path no longer exists, and
//! drawers retired by M2 reconciliation. `--all` removes every registered
//! drawer.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;

use crate::{Fail, Format};

/// Run `mrd cache ls [--json]`.
pub(crate) fn ls(format: Format) -> Result<(), Fail> {
    let mut drawers = list()?;
    drawers.sort_by(|a, b| a.workspace.cmp(&b.workspace));
    match format {
        Format::Json => {
            let rows: Vec<_> = drawers.iter().map(row_json).collect();
            println!("{}", serde_json::to_string_pretty(&rows).expect("json"));
        }
        Format::Human => print_table(&drawers),
    }
    Ok(())
}

/// Run `mrd cache clean [--all] [--json]`.
pub(crate) fn clean(all: bool, format: Format) -> Result<(), Fail> {
    let Ok(cache_root) = cache::cache_root() else {
        report_clean(format, &[], 0);
        return Ok(());
    };
    let mut removed: Vec<String> = Vec::new();
    let mut freed: u64 = 0;

    if all {
        for info in list_under(&cache_root)? {
            cache::remove_drawer(&info.drawer_dir).map_err(|e| {
                Fail::tool(format!(
                    "cannot remove drawer {}: {e}",
                    info.drawer_dir.display()
                ))
            })?;
            freed += info.size_bytes;
            removed.push(info.drawer_dir.display().to_string());
        }
    } else {
        // Stale-by-last-use, skip-locked: reuse the cache GC (it never reaps a
        // drawer a live workspace holds the flock on).
        let report = cache::gc(&cache_root, cache::DEFAULT_GC_THRESHOLD)
            .map_err(|e| Fail::tool(format!("last-use sweep failed: {e}")))?;
        freed += report.freed_bytes;
        removed.extend(report.removed.iter().map(|d| d.display().to_string()));

        // Plus drawers whose workspace vanished, or that M2 retired — both dead,
        // never live, so a blocking removal is safe.
        for info in list_under(&cache_root)? {
            let workspace_gone = !Path::new(&info.workspace).exists();
            let retired = info.superseded_by.is_some();
            if workspace_gone || retired {
                cache::remove_drawer(&info.drawer_dir).map_err(|e| {
                    Fail::tool(format!(
                        "cannot remove drawer {}: {e}",
                        info.drawer_dir.display()
                    ))
                })?;
                freed += info.size_bytes;
                removed.push(info.drawer_dir.display().to_string());
            }
        }
    }

    report_clean(format, &removed, freed);
    Ok(())
}

/// List drawers under the resolved cache root; an unresolved root is an empty
/// list, never an error (matches `cache ls` on a machine with no cache yet).
fn list() -> Result<Vec<cache::DrawerInfo>, Fail> {
    match cache::cache_root() {
        Ok(root) => list_under(&root),
        Err(_) => Ok(Vec::new()),
    }
}

fn list_under(cache_root: &Path) -> Result<Vec<cache::DrawerInfo>, Fail> {
    cache::list_drawers(cache_root).map_err(|e| {
        Fail::tool(format!(
            "cannot list drawers under {}: {e}",
            cache_root.display()
        ))
    })
}

fn row_json(info: &cache::DrawerInfo) -> serde_json::Value {
    json!({
        "key": info.key,
        "workspace": info.workspace,
        "version_segment": info.version_segment,
        "size_bytes": info.size_bytes,
        "last_use": info.last_use,
        "superseded_by": info.superseded_by,
        "drawer_dir": info.drawer_dir.display().to_string(),
    })
}

fn print_table(drawers: &[cache::DrawerInfo]) {
    if drawers.is_empty() {
        println!("no drawers");
        return;
    }
    println!(
        "{:<16}  {:<10}  {:>8}  {:>9}  WORKSPACE",
        "KEY", "VERSION", "SIZE", "LAST-USE"
    );
    for info in drawers {
        let flag = if info.superseded_by.is_some() {
            "  [retired]"
        } else {
            ""
        };
        println!(
            "{:<16}  {:<10}  {:>8}  {:>9}  {}{flag}",
            info.key,
            info.version_segment,
            human_size(info.size_bytes),
            age(info.last_use),
            info.workspace,
        );
    }
}

fn report_clean(format: Format, removed: &[String], freed: u64) {
    match format {
        Format::Json => {
            let value = json!({ "removed": removed, "freed_bytes": freed });
            println!("{}", serde_json::to_string_pretty(&value).expect("json"));
        }
        Format::Human => {
            println!(
                "removed {} drawer(s), freed {}",
                removed.len(),
                human_size(freed)
            );
            for dir in removed {
                println!("  - {dir}");
            }
        }
    }
}

#[allow(clippy::cast_precision_loss)]
fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// A coarse "N{d,h,m}" age for the last-use column.
fn age(last_use: u64) -> String {
    let secs = now_secs().saturating_sub(last_use);
    if secs >= 86_400 {
        format!("{}d", secs / 86_400)
    } else if secs >= 3_600 {
        format!("{}h", secs / 3_600)
    } else if secs >= 60 {
        format!("{}m", secs / 60)
    } else {
        format!("{secs}s")
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
