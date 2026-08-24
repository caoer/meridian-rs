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
//!
//! That refusal speaks only what this invocation checked, because the split above means the two
//! halves are not checked together: the drawer is looked at on every path through the door, while
//! the registry is queried only when a daemon answers the ping. With no daemon there is no
//! registry fact to report — an entry may still be keyed by that path — and the same discipline
//! covers the path itself, probed with `try_exists` so "could not look" never reads back as "not
//! there".

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
    //
    // Probed once, and with `try_exists` rather than `exists`: `exists()` folds
    // EACCES, ELOOP and ESTALE into a plain `false`, so a path this process was
    // not ALLOWED to look at reads back identically to one that is gone — and
    // the refusal below would then assert "the directory does not exist" about
    // a directory nobody could check. Every reader of `vanished` still treats
    // "cannot tell" the way `exists()` did, which is safe here because a miss
    // can only end in that refusal, never in a removal of the wrong tree.
    let probe = base.try_exists();
    let vanished = !matches!(probe, Ok(true));
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
        return Err(Fail::tool(nothing_removed(&base, &probe, daemon_removed)));
    }

    report(format, &workspace, daemon_removed, drawer_removed);
    Ok(())
}

/// The refusal for a vanished path this invocation removed nothing for.
///
/// It states what was CHECKED and nothing else. The retired sentence said "no
/// registry entry or drawer is keyed by that exact path" on every leg —
/// including the one where no daemon answered, where the registry was never
/// queried at all. That leg is ordinary, not exotic: a vanished path on a host
/// with no daemon running and the drawer already swept. Asserting the absence
/// of an entry nobody looked for is the kind of sentence an operator acts on,
/// so it is now two sentences, one per fact, and the unknown stays unknown.
///
/// `probe` is the `try_exists` result for `base`, so "could not look" (EACCES,
/// ELOOP, ESTALE) is reported as itself rather than as "does not exist".
/// `daemon_removed` is `None` exactly when no daemon answered the ping;
/// `Some(true)` cannot reach here (the caller's guard excludes it).
fn nothing_removed(
    base: &Path,
    probe: &std::io::Result<bool>,
    daemon_removed: Option<bool>,
) -> String {
    let presence = match probe {
        Ok(_) => "the directory does not exist".to_owned(),
        Err(e) => format!("the directory could not be examined ({e})"),
    };
    let (found, hint) = match daemon_removed {
        // A daemon answered and held no entry: both facts are ours to assert.
        Some(_) => (
            "neither a registry entry nor a drawer is keyed by that exact path",
            "",
        ),
        // No daemon answered: the drawer fact stands alone.
        None => (
            "no drawer is keyed by that exact path, and the registry was NOT checked \
             — no daemon answered, so an entry may still be registered under it",
            " Start the daemon and run this again to sweep the registry entry too.",
        ),
    };
    format!(
        "nothing was unregistered for {}: {presence}, and {found}. \
         A registration is keyed by its canonical path — unregister it by the path \
         `mrd cache ls` reports.{hint}",
        base.display()
    )
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

#[cfg(test)]
mod tests {
    use super::nothing_removed;
    use std::io;
    use std::path::Path;

    /// No daemon answered: the message may not speak about the registry as if
    /// it had been queried, and must say the lookup did not happen — the
    /// operator's next move (start a daemon, run it again) depends on it.
    #[test]
    fn with_no_daemon_the_refusal_reports_the_registry_as_unchecked() {
        let text = nothing_removed(Path::new("/gone/tree"), &Ok(false), None);
        assert!(
            text.contains("the registry was NOT checked"),
            "the unqueried registry must be named as unchecked — got: {text}",
        );
        assert!(
            text.contains("an entry may still be registered"),
            "and the unknown must stay open — got: {text}",
        );
        assert!(
            !text.contains("no registry entry or drawer"),
            "the retired sentence asserted an absence nobody looked for — got: {text}",
        );
        assert!(
            !text.contains("neither a registry entry nor a drawer"),
            "nor may the daemon-answered wording leak onto this leg — got: {text}",
        );
        assert!(
            text.contains("no drawer is keyed by that exact path"),
            "the drawer WAS checked, so that half is still asserted — got: {text}",
        );
        assert!(
            text.contains("/gone/tree"),
            "the refusal names the path — got: {text}",
        );
    }

    /// A daemon answered and held no entry: both facts were checked, so both
    /// are asserted, and there is nothing to go start.
    #[test]
    fn with_a_daemon_answering_the_refusal_asserts_both_facts() {
        let text = nothing_removed(Path::new("/gone/tree"), &Ok(false), Some(false));
        assert!(
            text.contains("neither a registry entry nor a drawer is keyed by that exact path"),
            "a queried registry is reported as queried — got: {text}",
        );
        assert!(
            !text.contains("NOT checked"),
            "and the unchecked wording must not appear — got: {text}",
        );
        assert!(
            !text.contains("Start the daemon"),
            "nor the hint that only helps the daemonless leg — got: {text}",
        );
    }

    /// `try_exists` erred: the path was not looked at, so the refusal says that
    /// instead of claiming the directory is gone. `exists()` folded this case
    /// into `false` and the message read identically to a real absence.
    #[test]
    fn an_unreadable_path_is_not_reported_as_absent() {
        let denied = io::Error::from(io::ErrorKind::PermissionDenied);
        let text = nothing_removed(Path::new("/locked/tree"), &Err(denied), None);
        assert!(
            text.contains("could not be examined"),
            "a path that could not be probed says so — got: {text}",
        );
        assert!(
            !text.contains("does not exist"),
            "and never claims an absence it did not observe — got: {text}",
        );
    }
}
