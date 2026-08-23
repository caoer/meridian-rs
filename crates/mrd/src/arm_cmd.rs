//! `mrd arm` — the ARM act as a verb: the attest path (armed-plane rung 4).
//!
//! ```text
//! mrd arm <ID> --mode <off|warn|block|armed> --rev <16HEX> [--at DIR] [--json]
//! ```
//!
//! Arming attests a rule: a reviewer approves the resolved page AT the rev they
//! read, and the act pins exactly that. `--rev` is therefore REQUIRED and has no
//! live-rev default — a default would attest bytes nobody read. The rev is read
//! where a reviewer reads it: `mrd rules` (the winner line's `rev=`) or
//! `mrd read`.
//!
//! The act is `policy::armed::arm` — narrow to the arm root, resolve through
//! the one resolver, admit the attestation only at the live rev, LOAD the
//! winner through the loader the fire path runs, pin the winner — and every
//! refusal is that act's own teaching fault. A page that registers but does
//! not load refuses `ArmFault::Unloadable` and the artifact is left
//! byte-untouched: attesting bytes that cannot become a rule pins law that can
//! never fire. The disk edge
//! is `wire_serve::armed_disk::ArmSession`: the write flock from read to
//! commit, rename-atomic artifact landing, the once-armed marker created on
//! the first arm (artifact first, marker second — the safe crash order).
//!
//! Re-arm semantics, per (id, arm root):
//! - row absent → added;
//! - row identical (page + rev + mode) → no-op success, "unchanged" — the
//!   crash-recovery re-run costs nothing;
//! - row differs → REPLACED: the withdraw-then-arm every drift refusal
//!   commands ("re-arm at the live rev"). Replacement passed the whole act,
//!   so a re-arm can never skip a gate the first arm ran.
//!
//! A corrupt EXISTING artifact refuses the act and is left byte-untouched:
//! overwriting an unreadable artifact would silently disarm rows nobody can
//! read. Fail closed, teach the fault.
//!
//! Exit triad: **0** armed (or already-armed, unchanged) · **1** refused (an
//! arm fault, drift, a corrupt artifact, a busy write lock) · **2** bad
//! invocation.

use serde_json::json;

use policy::armed::{ArmFault, ArmRequest, ArmRoot, Mode, parse_artifact};
use wire_serve::armed_disk::ArmSession;

use crate::{Fail, Format, current_dir};

/// The finding leg of the triad.
const EXIT_REFUSED: u8 = 1;

/// Run `mrd arm <ID> --mode M --rev R [--at DIR] [--json]`.
///
/// # Errors
/// [`Fail`] — exit 2 on a bad invocation (unknown flag, an illegal mode word,
/// a malformed rev, an unusable arm root); exit 1 on every refusal of the act
/// itself (arm faults, drift, corrupt artifact, busy lock).
pub(crate) fn dispatch(args: &[String]) -> Result<(), Fail> {
    let parsed = Arm::parse(args)?;
    let cwd = current_dir()?;
    let resolved = crate::resolve::resolve_runtime(workspace::Base::Cwd(&cwd)).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace for {}: {e}",
            cwd.display()
        ))
    })?;
    let canonical = workspace::canonicalize(&resolved.workspace).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace {} ({e})",
            resolved.workspace.display()
        ))
    })?;

    // Discovery through the one shared feed (`rules_walk`), so the act arms
    // exactly what `mrd rules` shows — never a second enumeration.
    let anchor = crate::rules_walk::user_anchor(&config::Env::from_process());
    let walk = crate::rules_walk::walk_rules(&canonical, anchor.as_deref())
        .map_err(|e| Fail::tool(format!("cannot read the workspace corpus: {e}")))?;
    let index = walk.into_index();

    // The disk session: flock now, so the artifact read, the compose, and the
    // commit see one world.
    let root = fs::WorkspaceRoot(canonical.clone());
    let session = ArmSession::open(&root).map_err(|e| {
        if e.kind() == std::io::ErrorKind::WouldBlock {
            Fail::with_code(
                EXIT_REFUSED,
                "another meridian writer holds .meridian/write.lock — transient; retry".to_owned(),
            )
        } else {
            Fail::tool(format!("cannot take the write lock: {e}"))
        }
    })?;

    // The standing artifact, strictly parsed. Corrupt refuses the act and the
    // page stays byte-untouched — a clobber would silently disarm unknown rows.
    let mut artifact = match session.artifact() {
        None => policy::armed::ArmedArtifact::default(),
        Some(page) => parse_artifact(&page).map_err(|corrupt| {
            Fail::with_code(
                EXIT_REFUSED,
                format!(
                    "{corrupt} — the arm act refuses to overwrite an artifact it cannot read; \
                     repair the page (or remove it on a never-armed workspace), then re-arm"
                ),
            )
        })?,
    };

    // The act: all validation is policy's, every fault its own teaching. The
    // winner's bytes and the limits are the SAME pair the fire path is handed
    // (`wire_serve::armed_disk::resolve_at`), so a page that arms is a page
    // that would load — and both reads happen under the session's flock.
    let pages = wire_serve::armed_disk::DiskPages::new(&root);
    let act = policy::armed::arm(
        &index,
        &parsed.root,
        [ArmRequest {
            id: parsed.id.clone(),
            mode: parsed.mode,
            attested_rev: parsed.rev.clone(),
        }],
        &pages,
        policy::CheckLimits::default(),
    )
    .map_err(|faults| {
        let rendered: Vec<String> = faults
            .iter()
            .map(|fault| {
                let mut line = fault.to_string();
                // The silent-non-registration teaching (card
                // rules-silent-nonregistration): an id that resolves to
                // nothing MAY have a carrier the hash domain excludes — a
                // page that reads as working law and governs nothing. Runs
                // only on this failure path, never on a clean arm.
                if let ArmFault::Unresolved { id, .. } = fault {
                    for (page, reason) in excluded_carriers(&canonical, id) {
                        use std::fmt::Write as _;
                        let _ = write!(
                            line,
                            "\n  a page carrying id `{id}` exists and is OUTSIDE the hash \
                             domain: `{page}`, excluded by {reason} — law that cannot be \
                             hashed cannot be attested; move the page into the domain, then \
                             arm at the rev `mrd rules` prints"
                        );
                    }
                }
                line
            })
            .collect();
        Fail::with_code(EXIT_REFUSED, rendered.join("\n"))
    })?;
    let fresh = act.rows()[0].clone();

    // Re-arm join on the (id, arm root) key.
    let withdrawn = artifact.remove(&parsed.id, &parsed.root);
    if withdrawn.as_ref() == Some(&fresh) {
        report(&parsed, &canonical, &fresh, None, false, true);
        return Ok(());
    }
    artifact.merge(act).map_err(|faults| {
        // Unreachable by construction: the key was withdrawn above. Loud
        // anyway — a silent drop here would lose the reviewer's act.
        let rendered: Vec<String> = faults.iter().map(ToString::to_string).collect();
        Fail::with_code(EXIT_REFUSED, rendered.join("\n"))
    })?;

    let landing = session
        .commit(&artifact.render())
        .map_err(|e| Fail::tool(format!("the attest write failed: {e}")))?;

    report(
        &parsed,
        &canonical,
        &fresh,
        withdrawn.as_ref(),
        landing.first_arm,
        false,
    );
    Ok(())
}

/// The domain-excluded pages that carry `id` — the other half of the
/// silent-non-registration lint (card rules-silent-nonregistration; the
/// `mrd rules` half is its `not offered to registration` dot line).
///
/// Feeds are the two §12.1 exclusion classes (`fs::dot_declined_markdown`,
/// `fs::declined_markdown`), each narrowed by the ONE registrar law
/// (`rules_walk::rule_candidates_among`) — an id match here means the page
/// would have REGISTERED had the domain held it. Enumeration failure yields
/// an empty answer, never a second fault: this teaching decorates a refusal
/// that already stands on its own.
fn excluded_carriers(
    workspace: &std::path::Path,
    id: &policy::RuleId,
) -> Vec<(String, &'static str)> {
    let root = fs::WorkspaceRoot(workspace.to_path_buf());
    let feeds = [
        (
            fs::dot_declined_markdown(&root),
            "a dot-prefixed path segment",
        ),
        (
            fs::declined_markdown(&root),
            "a `meridian/domain.md` ignore rule",
        ),
    ];
    let mut found = Vec::new();
    for (feed, reason) in feeds {
        let Ok(pages) = feed else { continue };
        let readable: Vec<(String, String)> = pages
            .into_iter()
            .filter_map(|rel| {
                std::fs::read_to_string(workspace.join(&rel))
                    .ok()
                    .map(|bytes| (rel, bytes))
            })
            .collect();
        let (candidates, _undecidable) = crate::rules_walk::rule_candidates_among(&readable);
        for (page, candidate) in candidates {
            if candidate.as_ref() == Some(id) {
                found.push((page, reason));
            }
        }
    }
    found
}

/// One report, two faces.
fn report(
    parsed: &Arm,
    workspace: &std::path::Path,
    row: &policy::armed::ArmedRow,
    replaced: Option<&policy::armed::ArmedRow>,
    first_arm: bool,
    unchanged: bool,
) {
    match parsed.format {
        Format::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "workspace": workspace.display().to_string(),
                    "arm": {
                        "id": row.id().as_str(),
                        "mode": row.mode().as_str(),
                        "scope": row.scope().to_string(),
                        "page": row.page(),
                        "rev": row.rev(),
                        "replaced": replaced.map(|old| json!({
                            "rev": old.rev(),
                            "mode": old.mode().as_str(),
                            "page": old.page(),
                        })),
                        "first_arm": first_arm,
                        "unchanged": unchanged,
                    }
                }))
                .expect("json"),
            );
        }
        Format::Human => {
            if unchanged {
                println!(
                    "{id} already armed {mode} at {scope} — unchanged",
                    id = row.id(),
                    mode = row.mode(),
                    scope = row.scope(),
                );
                return;
            }
            let verb = if replaced.is_some() {
                "re-armed"
            } else {
                "armed"
            };
            println!(
                "{verb} {id} {mode} at {scope} — pinned {page} @ {rev}",
                id = row.id(),
                mode = row.mode(),
                scope = row.scope(),
                page = row.page(),
                rev = row.rev(),
            );
            if let Some(old) = replaced {
                println!(
                    "  replaced the row attested @ {rev} ({mode})",
                    rev = old.rev(),
                    mode = old.mode(),
                );
            }
            if first_arm {
                println!("workspace armed: once-armed marker created (genesis arm)");
            }
        }
    }
}

// ── the invocation ────────────────────────────────────────────────────────────

/// The parsed `arm` invocation.
#[derive(Debug)]
struct Arm {
    id: policy::RuleId,
    mode: Mode,
    rev: String,
    root: ArmRoot,
    format: Format,
}

impl Arm {
    fn parse(args: &[String]) -> Result<Self, Fail> {
        let mut id: Option<String> = None;
        let mut mode: Option<String> = None;
        let mut rev: Option<String> = None;
        let mut at: Option<String> = None;
        let mut json = false;

        let mut walk = args.iter();
        while let Some(arg) = walk.next() {
            match arg.as_str() {
                "--json" => json = true,
                "--mode" | "--rev" | "--at" => {
                    let value = walk
                        .next()
                        .ok_or_else(|| Fail::tool(format!("{arg} takes a value")))?;
                    let slot = match arg.as_str() {
                        "--mode" => &mut mode,
                        "--rev" => &mut rev,
                        _ => &mut at,
                    };
                    if slot.replace(value.clone()).is_some() {
                        return Err(Fail::tool(format!("{arg} was given twice")));
                    }
                }
                other if other.starts_with('-') => {
                    return Err(Fail::tool(format!("unknown flag: {other}")));
                }
                value if id.is_none() => id = Some(value.to_owned()),
                value => {
                    return Err(Fail::tool(format!("unexpected argument: {value}")));
                }
            }
        }

        let id = id.ok_or_else(|| {
            Fail::tool("arm takes the rule ID to attest: mrd arm <ID> --mode M --rev R".to_owned())
        })?;
        let id = policy::RuleId::parse(&id)
            .map_err(|fault| Fail::tool(format!("not a legal rule id: {fault}")))?;

        let mode_word = mode.ok_or_else(|| {
            Fail::tool(format!(
                "--mode is required — a check arms {check}, a hook arms {hook}",
                check = Mode::vocabulary(policy::RuleKind::Check),
                hook = Mode::vocabulary(policy::RuleKind::Hook),
            ))
        })?;
        let mode = Mode::parse(&mode_word).ok_or_else(|| {
            Fail::tool(format!(
                "`{mode_word}` is outside the closed mode vocabulary — a check arms {check}, a \
                 hook arms {hook}",
                check = Mode::vocabulary(policy::RuleKind::Check),
                hook = Mode::vocabulary(policy::RuleKind::Hook),
            ))
        })?;

        let rev = rev.ok_or_else(|| {
            Fail::tool(
                "--rev is required: the rev IS the attestation — a reviewer approves the page AT \
                 the rev they read, and the act refuses when the live page has drifted off it. \
                 Read it where a reviewer does: `mrd rules` prints the winner's rev=, and \
                 `mrd read` serves the page"
                    .to_owned(),
            )
        })?;
        if rev.len() != 16
            || !rev
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        {
            return Err(Fail::tool(format!(
                "`{rev}` is not a page rev (16 lowercase hex) — read it from `mrd rules` (the \
                 winner line's rev=)"
            )));
        }

        // The refusal names what was passed and teaches the flag's own law —
        // the sealed `ArmRoot::parse` fault speaks in artifact terms, so the
        // wrapper owes the caller the CLI-face constraint and the default.
        let at_given = at.as_deref().unwrap_or(".");
        let root = ArmRoot::parse(at_given).map_err(|fault| {
            Fail::tool(format!(
                "--at `{at_given}` is not a legal arm root ({fault}) — --at takes a \
                 workspace-relative directory, default `.` (the workspace root)"
            ))
        })?;

        Ok(Arm {
            id,
            mode,
            rev,
            root,
            format: if json { Format::Json } else { Format::Human },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Arm, Fail> {
        let owned: Vec<String> = args.iter().map(|a| (*a).to_string()).collect();
        Arm::parse(&owned)
    }

    #[test]
    fn a_full_invocation_parses() {
        let arm = parse(&[
            "close-verdict",
            "--mode",
            "block",
            "--rev",
            "0123456789abcdef",
            "--at",
            "sessions/s1",
        ])
        .expect("parses");
        assert_eq!(arm.id.as_str(), "close-verdict");
        assert_eq!(arm.mode, Mode::Block);
        assert_eq!(arm.rev, "0123456789abcdef");
        assert_eq!(arm.root.as_str(), "sessions/s1");
    }

    #[test]
    fn the_root_defaults_to_the_workspace() {
        let arm = parse(&["x", "--mode", "off", "--rev", "0123456789abcdef"]).expect("parses");
        assert_eq!(arm.root, ArmRoot::workspace());
    }

    #[test]
    fn a_missing_rev_teaches_the_attestation_and_where_to_read_it() {
        let fail = parse(&["x", "--mode", "block"]).expect_err("--rev is the attestation");
        assert_eq!(fail.code, 2);
        assert!(
            fail.message.contains("--rev") && fail.message.contains("mrd rules"),
            "{}",
            fail.message
        );
    }

    #[test]
    fn a_malformed_rev_is_a_bad_invocation() {
        for bad in ["short", "DEADBEEFDEADBEEF", "zzzzzzzzzzzzzzzz"] {
            let fail = parse(&["x", "--mode", "block", "--rev", bad]).expect_err("not a page rev");
            assert_eq!(fail.code, 2, "{bad}");
            assert!(
                fail.message.contains("16 lowercase hex"),
                "{}",
                fail.message
            );
        }
    }

    #[test]
    fn a_mode_outside_the_vocabulary_teaches_both_kinds() {
        let fail = parse(&["x", "--mode", "on", "--rev", "0123456789abcdef"])
            .expect_err("closed vocabulary");
        assert_eq!(fail.code, 2);
        assert!(
            fail.message.contains("off|warn|block") && fail.message.contains("off|armed"),
            "{}",
            fail.message
        );
    }

    #[test]
    fn an_absolute_at_is_refused_teaching_the_workspace_relative_law() {
        let fail = parse(&[
            "x",
            "--mode",
            "block",
            "--rev",
            "0123456789abcdef",
            "--at",
            "/tmp",
        ])
        .expect_err("an absolute arm root is a bad invocation");
        assert_eq!(fail.code, 2);
        assert!(
            fail.message.contains("workspace-relative")
                && fail.message.contains("/tmp")
                && fail.message.contains("`.`"),
            "the refusal names the constraint, what was passed, and the default: {}",
            fail.message
        );
    }

    #[test]
    fn the_resolver_scope_spelling_is_refused_in_at() {
        let fail = parse(&[
            "x",
            "--mode",
            "block",
            "--rev",
            "0123456789abcdef",
            "--at",
            "workspace:0",
        ])
        .expect_err("layer:depth is not a directory");
        assert_eq!(fail.code, 2);
        assert!(
            fail.message.contains("layer:depth") && fail.message.contains("ARM ROOT"),
            "{}",
            fail.message
        );
    }
}
