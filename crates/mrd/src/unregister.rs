//! `mrd unregister` — drop the daemon registry entry (when a daemon answers) and remove the
//! workspaces drawer. The split is deliberate and works with the daemon down: the registry
//! entry is removed only when a daemon is reachable, while the drawer is always removed. An
//! ephemeral tree (cwd-default, no daemon, never registered) has neither — unregister is then a
//! clean no-op.
//!
//!

use std::path::{Path, PathBuf};

use registry::Client;
use serde_json::json;

use crate::gc;
use crate::resolve::resolve_runtime;
use crate::{Fail, Format, current_dir};

/// Run `mrd unregister [PATH]`.
pub(crate) fn run(target_arg: Option<&str>, format: Format) -> Result<(), Fail> {
    let cwd = current_dir()?;
    let base = match target_arg {
        Some(p) if Path::new(p).is_absolute() => PathBuf::from(p),
        Some(p) => cwd.join(p),
        None => cwd,
    };
    let resolved = resolve_runtime(&base).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace for {}: {e}",
            base.display()
        ))
    })?;
    let workspace = resolved.workspace;

    // Daemon entry: removed only when a daemon answers a ping.
    let mut daemon_removed: Option<bool> = None;
    if let Ok(client) = Client::from_default()
        && client.ping().unwrap_or(false)
    {
        let removed = client
            .unregister(&workspace)
            .map_err(|e| Fail::tool(format!("daemon unregister failed: {e}")))?;
        daemon_removed = Some(removed);
    }

    // Drawer: always removed (no-op when ephemeral or absent).
    let mut drawer_removed = false;
    if let Ok(cache_root) = cache::cache_root() {
        let drawer_dir = cache::drawer_dir(&cache_root, &workspace);
        let existed = drawer_dir.exists();
        cache::remove_drawer(&drawer_dir).map_err(|e| {
            Fail::tool(format!(
                "cannot remove drawer {}: {e}",
                drawer_dir.display()
            ))
        })?;
        drawer_removed = existed;
        if existed {
            gc::maybe_auto_gc(&cache_root);
        }
    }

    report(format, &workspace, daemon_removed, drawer_removed);
    Ok(())
}

fn report(format: Format, workspace: &Path, daemon_removed: Option<bool>, drawer_removed: bool) {
    match format {
        Format::Json => {
            let value = json!({
                "workspace": workspace.display().to_string(),
                "daemon_entry_removed": daemon_removed,
                "drawer_removed": drawer_removed,
            });
            println!("{}", serde_json::to_string_pretty(&value).expect("json"));
        }
        Format::Human => {
            println!("unregistered workspace {}", workspace.display());
            match daemon_removed {
                Some(true) => println!("  daemon:  entry removed"),
                Some(false) => println!("  daemon:  no entry was registered"),
                None => println!("  daemon:  not reachable (drawer-only unregister)"),
            }
            if drawer_removed {
                println!("  drawer:  removed");
            } else {
                println!("  drawer:  none present");
            }
        }
    }
}
