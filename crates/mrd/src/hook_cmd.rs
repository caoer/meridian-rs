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

use crate::hook::{self, Installed, Removed, Unfenceable};
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
            // The downgrade guard's escape is the ratified one, read here so the
            // CLI face and the fence body honour the same grammar.
            hook::install(&workspace, &hook::force_from_env()).map(|(f, state)| {
                let (word, detail) = match state {
                    Installed::Fresh => ("installed", None),
                    Installed::AlreadyInstalled => ("already-installed", None),
                    // A migration is not an idempotent refresh, and reporting it
                    // as one hides the doors that were open until just now (R40).
                    Installed::Completed { added } => (
                        "completed",
                        Some(format!(
                            "{added} door(s) were unfenced and are now covered; \
                             this root was fenced by an install set of fewer doors"
                        )),
                    ),
                };
                (f, word.to_owned(), detail)
            }),
        ),
        "uninstall" => render(
            parsed.format,
            "uninstall",
            hook::uninstall(&workspace).map(|(f, state)| {
                let (word, detail) = match state {
                    Removed::Removed { doors } => {
                        ("removed", Some(format!("{doors} door(s) unfenced")))
                    }
                    Removed::Absent => ("absent", None),
                };
                (f, word.to_owned(), detail)
            }),
        ),
        // The whole SET's state, in one word, with its teaching. "Installed",
        // "installed by an older fence" and "installed at two of three doors" are
        // different facts about the disk and are reported apart (R40).
        "status" => render(
            parsed.format,
            "status",
            hook::status(&workspace)
                .map(|(f, coverage)| (f, coverage.word().to_owned(), coverage.teaching())),
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
            // Read the set back after the verb acted. **The report is of the disk,
            // never of what the verb intended** — and it is what carries
            // `fence_version`, the datum the fence has always declared and nothing
            // read. Read-only: no lock, no write.
            let coverage = hook::coverage(&fenceable);
            match format {
                Format::Json => {
                    let value = json!({
                        "verb": verb,
                        "state": state,
                        "workspace": fenceable.workspace.display().to_string(),
                        "common_dir": fenceable.common_dir.display().to_string(),
                        // The install set, per door. A scalar `hook` was a claim
                        // that one path was the whole fence, and it was not.
                        "hooks": coverage.doors.iter().map(|door| json!({
                            "name": door.name,
                            "path": door.path.display().to_string(),
                            "state": door.word(),
                            "fence_version": door.version(),
                        })).collect::<Vec<_>>(),
                        // What the FILES declare, and what THIS ENGINE writes,
                        // beside each other. A verdict that does not disclose its
                        // judge cannot be checked by a third party — which is the
                        // only way a version skew is ever caught, because in a
                        // skew both participants are inside it.
                        "fence_version": coverage.fence_version(),
                        "engine_version": hook::FENCE_VERSION,
                        "detail": detail,
                    });
                    println!("{}", serde_json::to_string_pretty(&value).expect("json"));
                }
                Format::Human => {
                    println!("hook {verb} {}", fenceable.workspace.display());
                    println!("  state:      {state}");
                    println!("  common-dir: {}", fenceable.common_dir.display());
                    for door in &coverage.doors {
                        println!(
                            "  hook:       {} [{}{}]",
                            door.path.display(),
                            door.word(),
                            door.version()
                                .map_or_else(String::new, |v| format!(" fence {v}"))
                        );
                    }
                    println!("  engine:     fence {}", hook::FENCE_VERSION);
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
