//! `mrd skill <NAME>` — emit an agent-facing contract document to stdout.
//!
//! ```text
//! mrd skill hook
//! ```
//!
//! # THE VERB IS AN EMITTER, AND THAT IS THE WHOLE CONTRACT
//! It prints one markdown document and does nothing else: no file is written, no
//! git directory is read, no workspace is resolved, no daemon is dialed. The
//! reader of the document does the placing.
//!
//! The predecessor (`mrd hook install | uninstall | status`) wrote into
//! `$GIT_DIR/hooks` and therefore had to carry an uninstaller that refused a
//! foreign file, a `flock` held across a read-decide-write section, a downgrade
//! guard with its own escape, and a three-valued currency report — four planes of
//! imperative machinery encoding rules that are, at the end, prose an agent can
//! read. **The markdown IS the contract now**, and every one of those rules is
//! legible content of [`HOOK`] rather than a code path that has to be trusted.
//!
//! # WHY `skill` AND NOT A FLAG
//! `mrd skill <NAME>` is the repo's existing two-level verb shape (`mrd cache ls`,
//! `mrd view status`, `mrd journal genesis`), and NAME is the axis that will grow:
//! a second document is a second file under `skills/`, not a second flag on an
//! unrelated verb. A `--hook` flag would have had to hang off something, and
//! nothing it could hang off shares this verb's contract — every other verb
//! resolves a workspace, and this one deliberately does not.
//!
//! # STDOUT IS THE PRODUCT, SO IT CARRIES NOTHING ELSE
//! No header, no path, no byte count, no trailing summary — a caller pipes this
//! into a file or into an agent's context, and anything else on the stream is
//! corruption of the artifact. There is no `--json` face: the document is
//! markdown, and a JSON envelope around a markdown string is a second contract
//! for the same bytes.
//!
//! Exits:
//! - **0** — the document was written to stdout.
//! - **2** — bad invocation (no name, an unknown name, an unknown flag, a second
//!   positional). **There is no exit-1 leg**: an emitter has no findings, and a
//!   document that printed is the whole of what this verb can succeed at.

use crate::Fail;

/// The commit-fence contract, the document `mrd skill hook` emits.
///
/// **Compiled in from `skills/hook.md`**, so the artifact an agent reads and the
/// artifact a human reviews in the repository are the same bytes — a document
/// built by string-concatenation in Rust is one nobody reads as markdown until
/// it is too late to notice it says the wrong thing.
///
/// The fence body inside it is the executable half, and
/// `crates/mrd/tests/skill_hook_emit.rs` holds it to
/// [`crate::hook::FENCE_VERSION`], [`crate::hook::FENCED_HOOKS`] and
/// [`crate::hook::HOOK_MARKER`] so the document and the engine that reads placed
/// fences cannot drift apart.
const HOOK: &str = include_str!("skills/hook.md");

/// Every document this verb can emit, by the name the CLI takes.
///
/// **A list, so an unknown name is refused against something.** `mrd skill
/// nonsense` names what it could have asked for rather than exiting on a bare
/// "unknown" — the refusal is where the surface is discoverable from.
const SKILLS: [(&str, &str); 1] = [("hook", HOOK)];

/// Run `mrd skill <NAME>`.
///
/// # Errors
/// [`Fail`] exit 2 on a missing name, an unknown name, an unknown flag, or a
/// second positional.
pub(crate) fn dispatch(args: &[String]) -> Result<(), Fail> {
    let name = parse(args)?;
    let Some((_, body)) = SKILLS.iter().find(|(n, _)| *n == name) else {
        return Err(Fail::tool(format!(
            "unknown skill: {name} (known: {})",
            names()
        )));
    };
    // `print!`, never `println!`: the document ends in its own newline, and a
    // second one is a byte the caller did not ask for in an artifact whose whole
    // contract is that stdout carries the document and nothing else.
    print!("{body}");
    Ok(())
}

/// The known names, for a refusal to teach with.
fn names() -> String {
    SKILLS
        .iter()
        .map(|(n, _)| *n)
        .collect::<Vec<_>>()
        .join(", ")
}

/// The one positional. **Flags are refused rather than ignored** — including
/// `--json`, which this verb has no face for and must not silently accept as if
/// it did.
fn parse(args: &[String]) -> Result<&str, Fail> {
    let mut name: Option<&str> = None;
    for arg in args {
        match arg.as_str() {
            "--json" => {
                return Err(Fail::tool(
                    "skill has no --json face: the document is markdown, and this verb writes it \
                     to stdout verbatim"
                        .to_owned(),
                ));
            }
            flag if flag.starts_with('-') => {
                return Err(Fail::tool(format!("unknown flag: {flag}")));
            }
            value if name.is_none() => name = Some(value),
            value => return Err(Fail::tool(format!("unexpected argument: {value}"))),
        }
    }
    name.ok_or_else(|| Fail::tool(format!("skill needs a name ({})", names())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_name_teaches_the_names_it_could_have_taken() {
        let err = parse(&[]).expect_err("a bare `mrd skill` is a bad invocation");
        assert_eq!(err.code, 2);
        assert!(
            err.message.contains("hook"),
            "the refusal is where this surface is discoverable from: {}",
            err.message
        );
    }

    #[test]
    fn an_unknown_flag_is_a_loud_exit_2_never_ignored() {
        let err = parse(&["--nope".to_owned()]).expect_err("unknown flag refuses");
        assert_eq!(err.code, 2);
    }

    #[test]
    fn json_is_refused_by_name_rather_than_swallowed_as_an_unknown_flag() {
        // Every other verb in this CLI takes `--json`, so a caller WILL try it
        // here. "unknown flag: --json" would read as an oversight; the refusal
        // has to say the verb has no such face and why.
        let err =
            parse(&["--json".to_owned(), "hook".to_owned()]).expect_err("there is no JSON face");
        assert_eq!(err.code, 2);
        assert!(
            err.message.contains("markdown"),
            "the refusal names the reason, not just the flag: {}",
            err.message
        );
    }

    #[test]
    fn a_second_positional_refuses_rather_than_silently_winning() {
        let err = parse(&["hook".to_owned(), "extra".to_owned()])
            .expect_err("two names is an ambiguous invocation");
        assert_eq!(err.code, 2);
    }

    #[test]
    fn every_declared_skill_carries_a_document() {
        // The lookup is by name over a list, so a name with an empty body would
        // exit 0 having printed nothing — a success indistinguishable from the
        // document being empty.
        for (name, body) in SKILLS {
            assert!(
                !body.trim().is_empty(),
                "skill {name} emits nothing, and exit 0 would say it worked"
            );
            assert!(
                body.ends_with('\n'),
                "skill {name} does not end in a newline, so `print!` truncates its last line"
            );
        }
    }
}
