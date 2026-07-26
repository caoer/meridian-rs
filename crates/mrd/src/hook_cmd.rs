//! `mrd hook` — the CLI face of the pre-commit fence (U15).
//!
//! ```text
//! mrd hook install   [PATH] [--json]
//! mrd hook uninstall [PATH] [--json]
//! mrd hook status    [PATH] [--json]
//! ```
//!
//! The plane itself is [`crate::hook`]; this module parses, renders, and maps a
//! refusal onto the exit triad. `PATH` defaults to the current directory and is
//! the **meridian workspace root** — the fence is installed per
//! `$GIT_COMMON_DIR`, so N linked worktrees of one repository share one hook and
//! one install.
//!
//! # Per root means ONE INVOCATION PER ROOT, and every one reports
//! There is deliberately no `--all`: the roots an operator wants fenced are not
//! a set this engine owns (the mount table is the config plane's, and a root
//! need not be mounted to be a git repository). What the plan requires instead
//! is that an **unreachable root is named at install time, never silently
//! skipped** — so every invocation reports its root's state, and a refusal
//! carries its reason word and its teaching rather than exiting quietly.
//!
//! Exits, the same triad every verb rides:
//! - **0** — the fence is installed (or was already), removed, or reported.
//! - **1** — this root refuses: it cannot carry the fence, with the reason word.
//! - **2** — bad invocation.

use std::path::PathBuf;

use serde_json::json;

use crate::hook::{self, HookHere, Installed, Removed, Unfenceable};
use crate::{Fail, Format, current_dir};

/// Run `mrd hook <install|uninstall|status> [PATH] [--json]`.
///
/// # Errors
/// [`Fail`] exit 2 on a bad invocation, exit 1 when the root refuses the fence.
pub(crate) fn dispatch(args: &[String]) -> Result<(), Fail> {
    let Some(sub) = args.first() else {
        return Err(Fail::tool(
            "hook needs a subcommand (install | uninstall | status)".to_owned(),
        ));
    };
    let parsed = Parsed::parse(&args[1..])?;
    let workspace = match parsed.path {
        Some(p) => PathBuf::from(p),
        None => current_dir()?,
    };

    match sub.as_str() {
        "install" => render(
            parsed.format,
            "install",
            hook::install(&workspace).map(|(f, state)| {
                let word = match state {
                    Installed::Fresh => "installed",
                    Installed::AlreadyInstalled => "already-installed",
                };
                (f, word.to_owned(), None)
            }),
        ),
        "uninstall" => render(
            parsed.format,
            "uninstall",
            hook::uninstall(&workspace).map(|(f, state)| {
                let word = match state {
                    Removed::Removed => "removed",
                    Removed::Absent => "absent",
                };
                (f, word.to_owned(), None)
            }),
        ),
        "status" => render(
            parsed.format,
            "status",
            hook::status(&workspace).map(|(f, here)| match here {
                HookHere::None => (f, "absent".to_owned(), None),
                // "Installed" and "installed, but an OLDER fence" are different
                // facts about the disk and are reported apart (R40). An older
                // fence runs `mrd check` where this one runs `mrd check --staged`,
                // so it reads the worktree and passes a staged forgery — a
                // reinstall is owed, and only this line can say so.
                HookHere::Ours { current: true } => (f, "installed".to_owned(), None),
                HookHere::Ours { current: false } => (
                    f,
                    "installed-superseded".to_owned(),
                    Some(
                        "an older fence is installed; `mrd hook install` refreshes it (idempotent)"
                            .to_owned(),
                    ),
                ),
                HookHere::Foreign { first_line } => {
                    (f, "foreign-hook".to_owned(), Some(first_line))
                }
            }),
        ),
        other => Err(Fail::tool(format!("unknown hook subcommand: {other}"))),
    }
}

/// Print the verdict, then exit on it. **The refusal is printed before the
/// non-zero exit** — a refusal an operator cannot read teaches nothing, which is
/// the same reason `mrd config` prints its table before refusing.
fn render(
    format: Format,
    verb: &str,
    outcome: Result<(hook::Fenceable, String, Option<String>), Unfenceable>,
) -> Result<(), Fail> {
    match outcome {
        Ok((fenceable, state, detail)) => {
            match format {
                Format::Json => {
                    let value = json!({
                        "verb": verb,
                        "state": state,
                        "workspace": fenceable.workspace.display().to_string(),
                        "common_dir": fenceable.common_dir.display().to_string(),
                        "hook": fenceable.hook_path.display().to_string(),
                        "detail": detail,
                    });
                    println!("{}", serde_json::to_string_pretty(&value).expect("json"));
                }
                Format::Human => {
                    println!("hook {verb} {}", fenceable.workspace.display());
                    println!("  state:      {state}");
                    println!("  common-dir: {}", fenceable.common_dir.display());
                    println!("  hook:       {}", fenceable.hook_path.display());
                    if let Some(detail) = &detail {
                        println!("  existing:   {detail}");
                    }
                }
            }
            // `status` REPORTS a foreign hook; it does not refuse one. The
            // install path is where that state is a refusal, and conflating the
            // two would leave an operator unable to look without being told no.
            Ok(())
        }
        Err(refusal) => {
            match format {
                Format::Json => {
                    let value = json!({
                        "verb": verb,
                        "state": "refused",
                        "reason": refusal.word(),
                        "teaching": refusal.teaching(),
                    });
                    println!("{}", serde_json::to_string_pretty(&value).expect("json"));
                }
                Format::Human => {
                    println!("hook {verb}: refused");
                    println!("  reason:   {}", refusal.word());
                    println!("  {}", refusal.teaching());
                }
            }
            Err(Fail::findings(format!(
                "hook {verb} refuses {}: {}",
                refusal.word(),
                refusal.teaching()
            )))
        }
    }
}

/// The parsed tail: an optional positional root and `--json`.
#[derive(Debug)]
struct Parsed {
    path: Option<String>,
    format: Format,
}

impl Parsed {
    fn parse(tail: &[String]) -> Result<Self, Fail> {
        let mut path = None;
        let mut json = false;
        for arg in tail {
            match arg.as_str() {
                "--json" => json = true,
                flag if flag.starts_with('-') => {
                    return Err(Fail::tool(format!("unknown flag: {flag}")));
                }
                value if path.is_none() => path = Some(value.to_owned()),
                value => {
                    return Err(Fail::tool(format!("unexpected argument: {value}")));
                }
            }
        }
        Ok(Parsed {
            path,
            format: if json { Format::Json } else { Format::Human },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_flag_is_a_loud_exit_2_never_ignored() {
        let err = Parsed::parse(&["--nope".to_owned()]).expect_err("unknown flag refuses");
        assert_eq!(err.code, 2);
    }

    #[test]
    fn a_second_positional_refuses_rather_than_silently_winning() {
        let err = Parsed::parse(&["/a".to_owned(), "/b".to_owned()])
            .expect_err("two roots is an ambiguous invocation");
        assert_eq!(err.code, 2);
    }
}
