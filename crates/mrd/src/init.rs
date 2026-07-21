//! `mrd init` — create the `.meridian.toml` marker, register the drawer
//! sentinel, and run M2 reconciliation (decision 0001 round 4, amendment M2).

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::gc;
use crate::{Fail, Format, current_dir};

/// The canonical marker filename — `meridian-rs` is its only writer. The
/// `workspace` crate keys identity on this file's EXISTENCE, never its body, so
/// the body is a minimal committed-format record, safe to check into a repo.
const MARKER: &str = ".meridian.toml";

/// The committed marker body. No timestamp — a committed file must not churn.
const MARKER_BODY: &str = "\
# meridian workspace marker — created by `mrd init`.
# Identity is defined by this file's existence; mrd is its only writer.
version = 1
created_by = \"mrd\"
";

/// Run `mrd init [PATH]`: deny-check, marker, drawer sentinel, M2 reconcile.
pub(crate) fn run(target_arg: Option<&str>, format: Format) -> Result<(), Fail> {
    let cwd = current_dir()?;
    let target = resolve_target(&cwd, target_arg)?;

    // Deny ceiling BEFORE any write — refuse $HOME, /, mount points, the cache
    // root, etc. with a typed reason (exit 2).
    if let Some(reason) = workspace::deny_reason(&target) {
        return Err(Fail::tool(format!(
            "refusing to init a workspace at {}: it is the {reason}",
            target.display()
        )));
    }

    // Marker: create only if absent — a re-init never clobbers an existing one.
    let marker_path = target.join(MARKER);
    let marker_created = if marker_path.exists() {
        false
    } else {
        fs::write(&marker_path, MARKER_BODY).map_err(|e| {
            Fail::tool(format!(
                "cannot write marker {}: {e}",
                marker_path.display()
            ))
        })?;
        true
    };

    // Register the drawer sentinel (records the canonical workspace path).
    let drawer = cache::CacheDrawer::open(&target);
    drawer.register().map_err(|e| {
        Fail::tool(format!(
            "cannot register the drawer for {}: {e}",
            target.display()
        ))
    })?;
    let persisted = !drawer.is_ephemeral();

    // M2 reconciliation + opportunistic auto-GC, only with a real cache root.
    let mut retired: Vec<String> = Vec::new();
    if let Ok(cache_root) = cache::cache_root() {
        retired = reconcile_descendants(&cache_root, &target)?;
        gc::maybe_auto_gc(&cache_root);
    }

    report(
        format,
        &target,
        &marker_path,
        marker_created,
        persisted,
        &drawer,
        &retired,
    );
    Ok(())
}

/// Retire every drawer whose workspace is a strict DESCENDANT of `target`: a
/// tier-4 leftover now shadowed by the new marker root. Each retired sentinel
/// records `superseded_by = target` (amendment M2) so `cache clean` can reap it
/// and a probe reads it as retired. The new marker root's own drawer is skipped.
fn reconcile_descendants(cache_root: &Path, target: &Path) -> Result<Vec<String>, Fail> {
    let drawers = cache::list_drawers(cache_root).map_err(|e| {
        Fail::tool(format!(
            "cannot list drawers under {}: {e}",
            cache_root.display()
        ))
    })?;
    let superseded_by = target.to_string_lossy().into_owned();
    let mut retired = Vec::new();
    for info in drawers {
        let ws = Path::new(&info.workspace);
        if ws != target && ws.starts_with(target) {
            cache::supersede(&info.drawer_dir, &superseded_by).map_err(|e| {
                Fail::tool(format!(
                    "cannot retire descendant drawer {}: {e}",
                    info.drawer_dir.display()
                ))
            })?;
            retired.push(info.workspace);
        }
    }
    Ok(retired)
}

/// Canonicalize the target: the `PATH` argument (relative to cwd) or the cwd.
fn resolve_target(cwd: &Path, arg: Option<&str>) -> Result<PathBuf, Fail> {
    let raw = match arg {
        Some(p) if Path::new(p).is_absolute() => PathBuf::from(p),
        Some(p) => cwd.join(p),
        None => cwd.to_path_buf(),
    };
    workspace::canonicalize(&raw).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace path {}: {e}",
            raw.display()
        ))
    })
}

#[allow(clippy::fn_params_excessive_bools, clippy::too_many_arguments)]
fn report(
    format: Format,
    target: &Path,
    marker_path: &Path,
    marker_created: bool,
    persisted: bool,
    drawer: &cache::CacheDrawer,
    retired: &[String],
) {
    let drawer_dir = drawer.dir().map(|d| d.display().to_string());
    match format {
        Format::Json => {
            let value = json!({
                "workspace": target.display().to_string(),
                "marker": marker_path.display().to_string(),
                "marker_created": marker_created,
                "drawer": drawer_dir,
                "drawer_persisted": persisted,
                "retired": retired,
            });
            println!("{}", serde_json::to_string_pretty(&value).expect("json"));
        }
        Format::Human => {
            println!("initialized workspace {}", target.display());
            let marker_state = if marker_created {
                "created"
            } else {
                "already present"
            };
            println!("  marker:  {} ({marker_state})", marker_path.display());
            match &drawer_dir {
                Some(d) if persisted => println!("  drawer:  {d} (registered)"),
                _ => println!("  drawer:  ephemeral (no cache root)"),
            }
            if retired.is_empty() {
                println!("  reconcile: no descendant drawers to retire");
            } else {
                println!(
                    "  reconcile: retired {} descendant drawer(s):",
                    retired.len()
                );
                for w in retired {
                    println!("    - {w}");
                }
            }
        }
    }
}
