//! `mrd unregister` — drop the daemon registry entry (when a daemon answers) and remove the
//! workspaces drawer. The split is deliberate and works with the daemon down: the registry
//! entry is removed only when a daemon is reachable, while the drawer is removed whenever this
//! process can SEE it — the cache root has to resolve and the drawer directory has to be
//! probeable. An ephemeral tree (cwd-default, no daemon, never registered) has neither —
//! unregister is then a clean no-op.
//!
//! It also works with the DIRECTORY gone, which is the stale-entry class a registry sweep exists
//! to remove: a path that cannot be canonicalized is matched as given, the spelling
//! [`registry::Registry::unregister`] already documents as its fallback key. A vanished path that
//! matches nothing refuses (exit 2) rather than reporting the clean no-op above — with no tree
//! there, "nothing was registered" and "you typed it wrong" are the same output otherwise.
//!
//! That refusal speaks only what this invocation checked, because the split above means the two
//! halves are not checked together: the registry is queried only when a daemon answers the ping,
//! and the drawer is looked at on every path through the door but not always successfully. With
//! no daemon there is no registry fact to report — an entry may still be keyed by that path — and
//! the same discipline covers the path itself and the drawer, both probed with `try_exists` so
//! "could not look" never reads back as "not there".

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
        // this door remove the tree the operator did NOT name. With no PATH the
        // cwd is ambient and the override still answers, as it always did.
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

    // Drawer: removed whenever this process can see it (no-op when ephemeral or
    // absent). Both rungs here folded "could not look" into "not there" — the
    // failed `cache_root()` was swallowed whole by an `if let Ok`, and `exists()`
    // turns EACCES, ELOOP and ESTALE into a plain `false`. The refusal below then
    // asserted "no drawer is keyed by that exact path" about a drawer sitting on
    // disk that nobody was allowed to look at. Same fold, one layer deeper than
    // the base probe above — and unlike that one it is NOT harmless everywhere:
    // the identical fold in `cache clean` authorizes a REMOVAL (`cache_cmd.rs`).
    let drawer = match cache::cache_root() {
        Err(e) => Drawer::Unexamined(format!("the cache root could not be resolved ({e})")),
        Ok(cache_root) => {
            let drawer_dir = cache::drawer_dir(&cache_root, &workspace);
            match drawer_dir.try_exists() {
                Err(e) => Drawer::Unexamined(format!("the drawer could not be examined ({e})")),
                Ok(false) => Drawer::Absent,
                Ok(true) => {
                    cache::remove_drawer(&drawer_dir).map_err(|e| {
                        Fail::tool(format!(
                            "cannot remove drawer {}: {e}",
                            drawer_dir.display()
                        ))
                    })?;
                    gc::maybe_auto_gc(&cache_root);
                    Drawer::Removed
                }
            }
        }
    };

    // A vanished directory that matched nothing is NOT the documented clean
    // no-op. That no-op is about a tree that is present and simply was never
    // registered — running it again changes nothing and says so. Here there is
    // no tree at all, so nothing this invocation could ever have acted on
    // existed: exit 0 would confirm a removal that did not happen, and a
    // mistyped path would read back as a completed sweep.
    if vanished && daemon_removed != Some(true) && !matches!(drawer, Drawer::Removed) {
        return Err(Fail::tool(nothing_removed(
            &base,
            &probe,
            daemon_removed,
            &drawer,
        )));
    }

    report(format, &workspace, daemon_removed, &drawer);
    Ok(())
}

/// What this invocation learned about the workspace's drawer.
///
/// The retired `bool` could not tell "there was no drawer" from "this process
/// was not allowed to look", because `exists()` folds both into `false`. Every
/// reader of that bool then spoke the first meaning: the refusal asserted an
/// absence, and the human report printed "none present", for a drawer still on
/// disk. Three states, so "could not look" is reported as itself.
enum Drawer {
    /// Present, and removed by this run.
    Removed,
    /// Looked at, and not there.
    Absent,
    /// NOT looked at — the reason, rendered as a clause for the operator.
    Unexamined(String),
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
/// `Some(true)` cannot reach here (the caller's guard excludes it). `drawer`
/// carries the same distinction for the drawer half: an unexamined drawer is
/// named as unexamined, never as an absence — the fold this half still had
/// after the registry half lost it (card `drawer-exists-folds-eacces`).
fn nothing_removed(
    base: &Path,
    probe: &std::io::Result<bool>,
    daemon_removed: Option<bool>,
    drawer: &Drawer,
) -> String {
    let presence = match probe {
        Ok(_) => "the directory does not exist".to_owned(),
        Err(e) => format!("the directory could not be examined ({e})"),
    };
    // `Drawer::Removed` cannot reach here either (same guard); it rides with
    // `Absent` only to keep the match total.
    let (found, hint) = match (daemon_removed, drawer) {
        // A daemon answered and held no entry, and the drawer was looked at:
        // both facts are ours to assert.
        (Some(_), Drawer::Absent | Drawer::Removed) => (
            "neither a registry entry nor a drawer is keyed by that exact path".to_owned(),
            "",
        ),
        // A daemon answered, but the drawer could not be looked at: assert the
        // half that was checked, leave the other open.
        (Some(_), Drawer::Unexamined(why)) => (
            format!(
                "no registry entry is keyed by that exact path, and {why}, \
                 so a drawer may still be keyed by it"
            ),
            "",
        ),
        // No daemon answered: the drawer fact stands alone.
        (None, Drawer::Absent | Drawer::Removed) => (
            "no drawer is keyed by that exact path, and the registry was NOT checked \
             — no daemon answered, so an entry may still be registered under it"
                .to_owned(),
            " Start the daemon and run this again to sweep the registry entry too.",
        ),
        // Neither half produced a fact. The message asserts nothing at all: it
        // reports two lookups that did not happen, which is the whole of what
        // this invocation knows.
        (None, Drawer::Unexamined(why)) => (
            format!(
                "{why}, so a drawer may still be keyed by that path, and the registry \
                 was NOT checked — no daemon answered, so an entry may still be \
                 registered under it"
            ),
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

fn report(format: Format, workspace: &Path, daemon_removed: Option<bool>, drawer: &Drawer) {
    match format {
        Format::Json => {
            // `null` means the same thing on both keys: this run did not find
            // out. `drawer_unexamined` carries why, so a machine consumer is
            // never left with a bare null it has to guess about.
            let (drawer_removed, drawer_unexamined) = match drawer {
                Drawer::Removed => (Some(true), None),
                Drawer::Absent => (Some(false), None),
                Drawer::Unexamined(why) => (None, Some(why.as_str())),
            };
            let value = json!({
                "workspace": workspace.display().to_string(),
                "daemon_entry_removed": daemon_removed,
                "drawer_removed": drawer_removed,
                "drawer_unexamined": drawer_unexamined,
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
            match drawer {
                Drawer::Removed => println!("  drawer:  removed"),
                Drawer::Absent => println!("  drawer:  none present"),
                // Never "none present": nothing was looked at.
                Drawer::Unexamined(why) => println!("  drawer:  {why} — it may still be on disk"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Drawer, nothing_removed};
    use std::io;
    use std::path::Path;

    /// The message an unreadable drawer produces, built the way `run` builds it
    /// from a `try_exists` error — so the tests below bite on the same bytes an
    /// operator reads, without needing a mode-000 directory (which proves
    /// nothing when the suite runs as root).
    fn unexaminable_drawer() -> Drawer {
        let denied = io::Error::from(io::ErrorKind::PermissionDenied);
        Drawer::Unexamined(format!("the drawer could not be examined ({denied})"))
    }

    /// No daemon answered: the message may not speak about the registry as if
    /// it had been queried, and must say the lookup did not happen — the
    /// operator's next move (start a daemon, run it again) depends on it.
    ///
    /// This is also the ZERO CONTROL for the drawer half below: the drawer WAS
    /// looked at here, so the absence is asserted in full. An assertion that
    /// only ever passes cannot bite.
    #[test]
    fn with_no_daemon_the_refusal_reports_the_registry_as_unchecked() {
        let text = nothing_removed(Path::new("/gone/tree"), &Ok(false), None, &Drawer::Absent);
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
        let text = nothing_removed(
            Path::new("/gone/tree"),
            &Ok(false),
            Some(false),
            &Drawer::Absent,
        );
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
        let text = nothing_removed(
            Path::new("/locked/tree"),
            &Err(denied),
            None,
            &Drawer::Absent,
        );
        assert!(
            text.contains("could not be examined"),
            "a path that could not be probed says so — got: {text}",
        );
        assert!(
            !text.contains("does not exist"),
            "and never claims an absence it did not observe — got: {text}",
        );
    }

    /// The drawer half of the same class. Pre-fix, `drawer_dir.exists()` folded
    /// EACCES to `false` and this exact refusal came back for a drawer that was
    /// physically on disk (executed probe, card `drawer-exists-folds-eacces`):
    ///
    /// > nothing was unregistered for /gone/tree: the directory does not exist,
    /// > and no drawer is keyed by that exact path, and the registry was NOT
    /// > checked …
    ///
    /// The absence assertion in the middle is the sentence that must not appear
    /// when nobody looked.
    #[test]
    fn an_unexaminable_drawer_is_not_reported_as_absent() {
        let text = nothing_removed(
            Path::new("/gone/tree"),
            &Ok(false),
            None,
            &unexaminable_drawer(),
        );
        assert!(
            !text.contains("no drawer is keyed by that exact path"),
            "the pre-fix sentence asserted an absence nobody observed — got: {text}",
        );
        assert!(
            text.contains("the drawer could not be examined"),
            "the drawer lookup that failed must be reported as itself — got: {text}",
        );
        assert!(
            text.contains("a drawer may still be keyed by that path"),
            "and the unknown must stay open — got: {text}",
        );
        assert!(
            text.contains("the registry was NOT checked"),
            "the registry half still reports itself unchecked — got: {text}",
        );
    }

    /// A daemon answered, so the registry fact is real — but the drawer fact is
    /// not, and the both-checked wording may not cover for it.
    #[test]
    fn an_unexaminable_drawer_does_not_borrow_the_both_checked_wording() {
        let text = nothing_removed(
            Path::new("/gone/tree"),
            &Ok(false),
            Some(false),
            &unexaminable_drawer(),
        );
        assert!(
            !text.contains("neither a registry entry nor a drawer"),
            "that sentence asserts a drawer absence nobody observed — got: {text}",
        );
        assert!(
            text.contains("no registry entry is keyed by that exact path"),
            "the queried half is still asserted — got: {text}",
        );
        assert!(
            text.contains("the drawer could not be examined"),
            "and the unqueried half is named as such — got: {text}",
        );
    }

    /// The other rung of the same half: `cache_root()` failing was swallowed by
    /// an `if let Ok`, so the drawer was never looked for at all — and the
    /// refusal said "no drawer is keyed by that exact path" anyway.
    #[test]
    fn an_unresolved_cache_root_is_not_reported_as_an_absent_drawer() {
        let drawer =
            Drawer::Unexamined("the cache root could not be resolved (no HOME)".to_owned());
        let text = nothing_removed(Path::new("/gone/tree"), &Ok(false), None, &drawer);
        assert!(
            !text.contains("no drawer is keyed by that exact path"),
            "an unresolved cache root is not evidence of an absent drawer — got: {text}",
        );
        assert!(
            text.contains("the cache root could not be resolved"),
            "the reason the drawer was never looked for is the operator's next move — got: {text}",
        );
    }
}
