//! `mrd unregister` — drop the daemon registry entry (when a daemon answers) and remove the
//! workspaces drawer. The split is deliberate and works with the daemon down: the registry
//! entry is removed only when a daemon is reachable, while the drawer is always removed. An
//! ephemeral tree (cwd-default, no daemon, never registered) has neither — unregister is then a
//! clean no-op.
//!
//! It also works with the DIRECTORY gone, which is the stale-entry class a registry sweep exists
//! to remove: a path that cannot be canonicalized is matched as given, the spelling
//! [`registry::Registry::unregister`] already documents as its fallback key. A vanished path that
//! matches nothing refuses (exit 2) rather than reporting the clean no-op above — with no tree
//! there, "nothing was registered" and "you typed it wrong" are the same output otherwise.

use std::path::{Path, PathBuf};

use registry::Client;
use serde_json::json;

use crate::gc;
use crate::resolve::resolve_runtime_lenient;
use crate::{Fail, Format, current_dir};

/// Run `mrd unregister [PATH]`.
pub(crate) fn run(target_arg: Option<&str>, format: Format) -> Result<(), Fail> {
    let cwd = current_dir()?;
    let base = match target_arg {
        Some(p) if Path::new(p).is_absolute() => PathBuf::from(p),
        Some(p) => cwd.join(p),
        None => cwd,
    };
    // Lenient on purpose: unregister must run outside a defined root — dropping
    // a stale drawer/registry entry is legitimate exactly when the markers are
    // already gone (the strict lane would refuse with OutsideWorkspace).
    //
    // Leniency has to reach one rung further than the ladder does. A workspace
    // whose DIRECTORY is gone cannot be resolved at all — there is nothing left
    // to canonicalize — and that is precisely the stale entry this door exists
    // to remove. The registry already spells the contract from the other side:
    // `Registry::unregister` "matches on the canonical path when the directory
    // still resolves, else on the path as given". Resolving first threw that
    // away, refusing before the request was ever made.
    let vanished = !base.exists();
    let workspace = if vanished {
        // The path as given, absolutized above — never a relative spelling, so
        // it cannot string-equal some other live entry's canonical key.
        base.clone()
    } else {
        // The operand decides. `MERIDIAN_WORKSPACE` used to answer this rung
        // before the argument was ever canonicalized, so a live override made
        // this door remove the tree the operator did NOT name (advisor ruling
        // 2026-08-23, `unregister-env-override-vs-explicit-path`). With no PATH
        // the cwd is ambient and the override still answers, as it always did.
        let ladder_base = match target_arg {
            Some(_) => workspace::Base::Named(&base),
            None => workspace::Base::Cwd(&base),
        };
        resolve_runtime_lenient(ladder_base)
            .map_err(|e| {
                Fail::tool(format!(
                    "cannot resolve workspace for {}: {e}",
                    base.display()
                ))
            })?
            .workspace
    };

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

    // A vanished directory that matched nothing is NOT the documented clean
    // no-op. That no-op is about a tree that is present and simply was never
    // registered — running it again changes nothing and says so. Here there is
    // no tree at all, so nothing this invocation could ever have acted on
    // existed: exit 0 would confirm a removal that did not happen, and a
    // mistyped path would read back as a completed sweep.
    if vanished && daemon_removed != Some(true) && !drawer_removed {
        return Err(Fail::tool(format!(
            "nothing to unregister for {}: the directory does not exist, and no registry entry or drawer is keyed by that exact path. \
             A registration is keyed by its canonical path — unregister it by the path `mrd cache ls` reports.",
            base.display()
        )));
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
