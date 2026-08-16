//! `mrd rm` — guarded file death through the § A.3 remove door.
//!
//! ```text
//! mrd rm <PAGE> --rev <FILE_REV> [--if-fingerprint FP] [--dry] [--actor A] [--now T] [--json]
//! ```
//!
//! The write model's third mutation on the CLI face, completing `mrd new`
//! (birth) and `mrd put` (edit). Remove-what-you-read: `--rev` is the record's
//! whole-file rev from a prior read, required at parse — the engine demands it
//! from every origin, deletion being the one write with no recovery. The
//! referential check runs inside the engine's write flock: any inbound
//! wikilink, embed, or ambient `meridian-lock` pin refuses `remove_refused`
//! naming every referring file, its edge kind, and its edge count. There is
//! no `--force` — the door has none.
//!
//! Routes as a wire `remove` to the running daemon ([`crate::write_ipc`]). There
//! is no in-process publication path.
//!
//! Exit triad: 0 removed (or a dry rehearsal that passed) / 1 refused (EVERY
//! engine refusal — `remove_refused`, `cas_mismatch`, `root_mismatch`,
//! `file_not_found`, an armed gate refusal — the engine's verbatim message) /
//! 2 bad invocation (the CLI's own refusals, before any engine contact).
//!
//! Under `--json` both legs answer on stdout: a commit the `{workspace, rm}`
//! frame, an engine refusal the `{workspace, error}` envelope — never an
//! empty stdout a machine consumer cannot branch on.

use serde_json::{Value, json};

use crate::{Fail, Format, current_dir, engine, write_ipc};

/// Run `mrd rm <PAGE> --rev <FILE_REV> [flags]`. Errors [`Fail`] — exit 2 on a
/// bad invocation (bad flags, a missing `--rev`, a malformed `--now`); exit 1
/// on every engine refusal, message verbatim.
pub(crate) fn dispatch(args: &[String]) -> Result<(), Fail> {
    let parsed = Rm::parse(args)?;
    let cwd = current_dir()?;
    let resolved = crate::resolve::resolve_runtime(&cwd).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace for {}: {e}",
            cwd.display()
        ))
    })?;
    let mut request = json!({
        "op": "remove",
        "path": parsed.path,
        "if_file_rev": parsed.rev,
    });
    if let Some(actor) = &parsed.actor {
        request["actor"] = json!(actor);
    }
    if let Some(now) = &parsed.now {
        request["now"] = json!(now);
    }
    if let Some(fp) = &parsed.if_fingerprint {
        request["if_fingerprint"] = json!(fp);
    }
    if parsed.dry {
        request["dry"] = json!(true);
    }

    let mut door = write_ipc::connect(&resolved.workspace)?;
    let body = write_ipc::call(&mut door, &request)
        .map_err(|e| engine::json_refusal(parsed.format, &resolved.workspace, &e))?;
    let body = write_ipc::project_body(body);

    match parsed.format {
        Format::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "workspace": resolved.workspace.display().to_string(),
                    "rm": body,
                }))
                .expect("json")
            );
        }
        Format::Human => print_human(&parsed, &body),
    }
    Ok(())
}

/// The human summary: what died (or was rehearsed), at which rev, and the
/// fingerprint transition — the engine's vocabulary, never a delivery claim.
fn print_human(parsed: &Rm, body: &Value) {
    let rev = body
        .get("file_rev_before")
        .and_then(Value::as_str)
        .unwrap_or("?");
    if parsed.dry {
        println!("dry run: {} would be removed, nothing touched", parsed.path);
        println!("  file_rev: {rev}");
        return;
    }
    let after = body
        .get("fingerprint_after")
        .and_then(Value::as_str)
        .unwrap_or("?");
    println!("removed {}", parsed.path);
    println!("  file_rev: {rev}");
    println!("  fingerprint: {after}");
}

/// The parsed `rm` invocation.
#[derive(Debug)]
struct Rm {
    path: String,
    /// The record's whole-file rev from a prior read (remove-what-you-read).
    rev: String,
    actor: Option<String>,
    now: Option<String>,
    if_fingerprint: Option<String>,
    dry: bool,
    format: Format,
}

impl Rm {
    fn parse(args: &[String]) -> Result<Self, Fail> {
        let mut positional: Option<String> = None;
        let mut rev: Option<String> = None;
        let mut actor: Option<String> = None;
        let mut now: Option<String> = None;
        let mut if_fingerprint: Option<String> = None;
        let mut dry = false;
        let mut json = false;
        let mut it = args.iter();
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "--json" => json = true,
                "--dry" => dry = true,
                "--rev" => {
                    let value = it
                        .next()
                        .ok_or_else(|| Fail::tool("--rev needs a value".to_owned()))?;
                    rev = Some(value.clone());
                }
                "--actor" => {
                    let value = it
                        .next()
                        .ok_or_else(|| Fail::tool("--actor needs a value".to_owned()))?;
                    actor = Some(value.clone());
                }
                "--now" => {
                    let value = it
                        .next()
                        .ok_or_else(|| Fail::tool("--now needs a value".to_owned()))?;
                    // The wire dispatch strict-decode is not on this path, so validate the §9
                    // format law here, exactly as the server would.
                    if !wire::now_is_rfc3339(value) {
                        return Err(Fail::tool(format!(
                            "--now must be RFC 3339 (YYYY-MM-DDTHH:MM:SS[.frac](Z|±HH:MM)): {value}"
                        )));
                    }
                    now = Some(value.clone());
                }
                "--if-fingerprint" => {
                    let value = it
                        .next()
                        .ok_or_else(|| Fail::tool("--if-fingerprint needs a value".to_owned()))?;
                    if_fingerprint = Some(value.clone());
                }
                "--force" => {
                    return Err(Fail::tool(
                        "rm has no --force: deletion is the one write with no recovery, so the \
                         remove door carries no escape hatch. A referenced record refuses with \
                         its referrer list — unlink those edges, then rerun."
                            .to_owned(),
                    ));
                }
                flag if flag.starts_with('-') => {
                    return Err(Fail::tool(format!("unknown flag: {flag}")));
                }
                value if positional.is_none() => positional = Some(value.to_owned()),
                value => return Err(Fail::tool(format!("unexpected argument: {value}"))),
            }
        }
        let Some(path) = positional else {
            return Err(Fail::tool("rm needs a PAGE".to_owned()));
        };
        let Some(rev) = rev else {
            return Err(Fail::tool(
                "rm needs --rev FILE_REV — remove-what-you-read: read the page first (a toc or \
                 cat serves its whole-file rev), then pass that rev here. The engine demands \
                 the token from every origin; there is no force."
                    .to_owned(),
            ));
        };
        Ok(Rm {
            path,
            rev,
            actor,
            now,
            if_fingerprint,
            dry,
            format: if json { Format::Json } else { Format::Human },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::Rm;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    /// The CLI's own strictness: a rev-less rm is a bad invocation (exit 2),
    /// refused before any engine contact — the parse teaches the read that
    /// mints the token.
    #[test]
    fn rm_without_rev_is_a_bad_invocation() {
        let err = Rm::parse(&args(&["notes/old.md"])).expect_err("rev-less rm refuses");
        assert!(
            format!("{err:?}").contains("--rev"),
            "the refusal teaches the missing flag: {err:?}"
        );
    }

    /// `--force` refuses at parse with the door's teaching — the flag does not
    /// exist on this op, and silence would imply it might.
    #[test]
    fn rm_force_refuses_at_parse() {
        let err =
            Rm::parse(&args(&["notes/old.md", "--rev", "abc", "--force"])).expect_err("no force");
        assert!(
            format!("{err:?}").contains("no --force"),
            "the refusal names the missing escape hatch: {err:?}"
        );
    }

    /// The happy parse: positional page, required rev, optional guards.
    #[test]
    fn rm_parse_carries_the_guards() {
        let rm = Rm::parse(&args(&[
            "notes/old.md",
            "--rev",
            "e3c4acaceb75b907",
            "--if-fingerprint",
            "b3:aa",
            "--dry",
        ]))
        .expect("parses");
        assert_eq!(rm.path, "notes/old.md");
        assert_eq!(rm.rev, "e3c4acaceb75b907");
        assert_eq!(rm.if_fingerprint.as_deref(), Some("b3:aa"));
        assert!(rm.dry);
    }
}
