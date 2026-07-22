//! `mrd run` — the local run plane mounted on the CLI (U10; verdict ruling 7,
//! plan decisions #8/#12). The argv surface is LOCKED:
//!
//! ```text
//! mrd run <PAGE> [TASK] [-- ARGS] --env K=V --dry --list --json
//! ```
//!
//! No argv JSON: positional args ride verbatim after `--`, env rides as
//! repeated `--env KEY=VALUE` pairs, and both are contract-validated pre-eval
//! (U2 — deny-by-default covers inputs too).
//!
//! # Exit triad (decision #12)
//! - **0** — clean: `--list`, a completed `--dry`, a clean run.
//! - **1** — the RUN PLANE refused or failed: eval fault, a bash fence under a
//!   read-only convention, foreign edit, workspace busy, root mismatch,
//!   timeout, bash nonzero exit. The invocation was well-formed; the plane
//!   said no.
//! - **2** — the INVOCATION is wrong (usage, addressing, contract violation)
//!   or the tool failed pre-run. TASK omitted with several declared tasks
//!   lists them and exits 2 — the CLI never guesses.
//!
//! # Half-scope (U7 gate)
//! `--list` and `--dry` are complete. `--dry` on starlark is END-TO-END
//! truth: the hermetic kernel evaluates the block through the U5 `evaluate`
//! seam and the FULL effect set prints — nothing applies. `--dry` on bash
//! shows the block + resolved caps and REFUSES to exec (no descriptor
//! fiction, decision #18). The execute leg itself refuses loudly until the
//! U7 runner (compose + guarantee labeler) lands.

use std::collections::BTreeMap;
use std::path::Path;

use run::address::{self, AddressError, ResolvedTask};
use run::caps::{self, CapResolution, CapSource, CapsError, Conventions};
use run::contracts::{self, Contract};
use run::executor::ExecError;
use run::fence::TaskLanguage;
use rules::EvalLimits;
use serde_json::json;

use crate::{Fail, Format, current_dir};

/// The run-plane leg of the triad: the invocation was well-formed, the plane
/// refused or failed.
const EXIT_RUN: u8 = 1;

/// The deterministic invocation id a `--dry` evaluation sees: `--dry` records
/// nothing, and a fixed id keeps dry output byte-stable run over run.
const DRY_INVOCATION: &str = "dry";

/// A run-plane refusal (exit 1) — distinct from [`Fail::tool`]'s exit 2.
fn fail_run(message: String) -> Fail {
    Fail {
        code: EXIT_RUN,
        message,
    }
}

/// Addressing faults are invocation faults: exit 2 (plan §5).
fn fail_address(e: &AddressError) -> Fail {
    Fail::tool(e.to_string())
}

/// Cap faults split by leg: a bash fence under a read-only convention is the
/// PLANE refusing a well-formed invocation (exit 1); malformed declarations
/// and an unreadable `.meridian.toml` are authoring/tool faults (exit 2).
fn fail_caps(e: &CapsError) -> Fail {
    match e {
        CapsError::BashFenceRefused { .. } => fail_run(e.to_string()),
        CapsError::BadCap { .. } | CapsError::BadPattern { .. } | CapsError::Toml { .. } => {
            Fail::tool(e.to_string())
        }
    }
}

/// Executor refusals all land on the run leg (exit 1) — the U7 runner wiring
/// adopts this mapping wholesale. The two named legs the triad review asked
/// for: `ForeignEdit` (decision #26 refusal, never a silent overwrite) and
/// the workspace-busy lock refusal (decision #9) both exit 1.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "adopted by the U7 runner wiring (execute leg)")
)]
fn fail_exec(e: &ExecError) -> Fail {
    let message = match e {
        ExecError::Io { reason } if reason.contains("workspace lock") => {
            format!("workspace busy: {reason}")
        }
        other => other.to_string(),
    };
    fail_run(message)
}

/// The parsed `mrd run` invocation.
#[derive(Debug)]
struct RunArgs {
    page: String,
    task: Option<String>,
    /// Positional task args — everything after `--`, verbatim.
    args: Vec<String>,
    /// `--env KEY=VALUE` pairs.
    env: BTreeMap<String, String>,
    dry: bool,
    list: bool,
    json: bool,
}

impl RunArgs {
    /// Parse the tail after `run`. Unknown flags, a malformed / duplicate
    /// `--env`, or a third positional refuse loudly (exit 2) — nothing is
    /// silently ignored.
    fn parse(tail: &[String]) -> Result<Self, Fail> {
        let mut page = None;
        let mut task = None;
        let mut args = Vec::new();
        let mut env = BTreeMap::new();
        let mut dry = false;
        let mut list = false;
        let mut json = false;
        let mut i = 0;
        while i < tail.len() {
            match tail[i].as_str() {
                "--" => {
                    args.extend(tail[i + 1..].iter().cloned());
                    break;
                }
                "--dry" => dry = true,
                "--list" => list = true,
                "--json" => json = true,
                "--env" => {
                    i += 1;
                    let Some(pair) = tail.get(i) else {
                        return Err(Fail::tool("--env needs KEY=VALUE".to_owned()));
                    };
                    let Some((key, value)) = pair.split_once('=') else {
                        return Err(Fail::tool(format!("--env '{pair}' is not KEY=VALUE")));
                    };
                    if key.is_empty() {
                        return Err(Fail::tool(format!("--env '{pair}' has an empty key")));
                    }
                    if env.insert(key.to_owned(), value.to_owned()).is_some() {
                        return Err(Fail::tool(format!("--env '{key}' given twice")));
                    }
                }
                flag if flag.starts_with('-') => {
                    return Err(Fail::tool(format!("unknown flag: {flag}")));
                }
                value if page.is_none() => page = Some(value.to_owned()),
                value if task.is_none() => task = Some(value.to_owned()),
                value => return Err(Fail::tool(format!("unexpected argument: {value}"))),
            }
            i += 1;
        }
        let Some(page) = page else {
            return Err(Fail::tool(
                "usage: mrd run <PAGE> [TASK] [-- ARGS] --env K=V --dry --list --json".to_owned(),
            ));
        };
        if list && (task.is_some() || dry || !args.is_empty() || !env.is_empty()) {
            return Err(Fail::tool(
                "--list takes only <PAGE> (and --json) — it lists every task; \
                 name a TASK with --dry to inspect one"
                    .to_owned(),
            ));
        }
        Ok(Self {
            page,
            task,
            args,
            env,
            dry,
            list,
            json,
        })
    }

    fn format(&self) -> Format {
        if self.json {
            Format::Json
        } else {
            Format::Human
        }
    }
}

/// Run `mrd run <tail>`.
///
/// # Errors
/// [`Fail`] on the triad's 1/2 legs — see the module docs for the mapping.
pub(crate) fn dispatch(tail: &[String]) -> Result<(), Fail> {
    let parsed = RunArgs::parse(tail)?;
    let cwd = current_dir()?;
    let resolution = workspace::resolve(&cwd)
        .map_err(|e| Fail::tool(format!("workspace resolution failed: {e:?}")))?;
    let root = fs::WorkspaceRoot(resolution.workspace);
    let doc = address::load_page(&root, Path::new(&parsed.page)).map_err(|e| fail_address(&e))?;
    let conventions = caps::load_conventions(&root.0).map_err(|e| fail_caps(&e))?;

    if parsed.list {
        return list_tasks(&root, &parsed.page, &doc, &conventions, parsed.format());
    }

    // TASK omitted: one binding runs; several list themselves and exit 2 —
    // the CLI never guesses (§2.1).
    let resolved = match address::resolve_task(&doc, parsed.task.as_deref()) {
        Ok(resolved) => resolved,
        Err(e @ AddressError::ManyTasks { .. }) => {
            list_tasks(&root, &parsed.page, &doc, &conventions, parsed.format())?;
            return Err(fail_address(&e));
        }
        Err(e) => return Err(fail_address(&e)),
    };
    let task = &resolved.binding.name;

    // Contract gate (U2): violations exit 2 WITH the declared contract shown.
    let contract = contracts::contract_for(&doc, task).map_err(|e| Fail::tool(e.to_string()))?;
    contracts::validate(task, &contract, &parsed.args, &parsed.env).map_err(|violation| {
        Fail::tool(format!(
            "{violation}\ndeclared contract: args: [{}], env: [{}]",
            contract.args.join(", "),
            contract.env.join(", ")
        ))
    })?;

    // Capability resolution (U2, deny-by-default; bash under check-*/verify-*
    // refuses on the run leg).
    let explicit = caps::explicit_caps(&doc, task).map_err(|e| fail_caps(&e))?;
    let caps = caps::resolve_caps(task, resolved.block.lang, explicit.as_ref(), &conventions)
        .map_err(|e| fail_caps(&e))?;

    if parsed.dry {
        return dry(&root, &parsed, &resolved, &caps);
    }

    // HALF-SCOPE: the execute leg lands with the U7 runner.
    Err(Fail::tool(format!(
        "task '{task}' resolved ({} fence, caps ok) — the execute path lands \
         with the U7 runner; use --dry for the effect truth or --list for the surface",
        resolved.block.lang.as_str()
    )))
}

/// One task's `--list` / listing row facts.
fn task_row(
    doc: &model::Document,
    conventions: &Conventions,
    name: &str,
) -> Result<(ResolvedTask, Contract, CapResolution), String> {
    let resolved = address::resolve_task(doc, Some(name)).map_err(|e| e.to_string())?;
    let contract = contracts::contract_for(doc, name).map_err(|e| e.to_string())?;
    let explicit = caps::explicit_caps(doc, name).map_err(|e| e.to_string())?;
    let caps = caps::resolve_caps(name, resolved.block.lang, explicit.as_ref(), conventions)
        .map_err(|e| e.to_string())?;
    Ok((resolved, contract, caps))
}

/// Render a cap source for humans.
fn source_label(source: &CapSource) -> String {
    match source {
        CapSource::Explicit => "explicit".to_owned(),
        CapSource::Convention(pattern) => format!("convention '{pattern}'"),
        CapSource::DenyDefault => "deny-default".to_owned(),
    }
}

/// The effective cap list as strings (empty = read-only).
fn cap_strings(resolution: &CapResolution) -> Vec<String> {
    resolution
        .effective
        .0
        .iter()
        .map(caps::Cap::as_string)
        .collect()
}

/// `--list`: every declared task with its language, guarantee class,
/// contract, and resolved caps. A broken binding shows its typed error as
/// the row — one broken declaration never hides the rest and never vanishes.
fn list_tasks(
    _root: &fs::WorkspaceRoot,
    page: &str,
    doc: &model::Document,
    conventions: &Conventions,
    format: Format,
) -> Result<(), Fail> {
    let bindings = address::bindings(doc).map_err(|e| fail_address(&e))?;
    match format {
        Format::Json => {
            let rows: Vec<serde_json::Value> = bindings
                .iter()
                .map(|b| match task_row(doc, conventions, &b.name) {
                    Ok((resolved, contract, caps)) => json!({
                        "task": b.name,
                        "lang": resolved.block.lang.as_str(),
                        "guarantee": resolved.block.lang.guarantee_class().as_str(),
                        "args": contract.args,
                        "env": contract.env,
                        "caps": {
                            "effective": cap_strings(&caps),
                            "source": source_label(&caps.source),
                            "narrowed": caps.narrowed.iter().map(caps::Cap::as_string).collect::<Vec<_>>(),
                        },
                    }),
                    Err(error) => json!({ "task": b.name, "error": error }),
                })
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({ "page": page, "tasks": rows }))
                    .expect("json render")
            );
        }
        Format::Human => {
            if bindings.is_empty() {
                println!("{page}: no tasks declared");
                return Ok(());
            }
            println!("tasks on {page}:");
            for b in &bindings {
                use std::fmt::Write as _;
                match task_row(doc, conventions, &b.name) {
                    Ok((resolved, contract, caps)) => {
                        let lang = resolved.block.lang.as_str();
                        let class = resolved.block.lang.guarantee_class().as_str();
                        let cap_list = cap_strings(&caps);
                        let mut line = format!(
                            "  {}  {lang}  {class}  caps: {} [{}]",
                            b.name,
                            if cap_list.is_empty() {
                                "(read-only)".to_owned()
                            } else {
                                cap_list.join(", ")
                            },
                            source_label(&caps.source),
                        );
                        if !caps.narrowed.is_empty() {
                            let _ = write!(
                                line,
                                "  narrowed: {}",
                                caps.narrowed
                                    .iter()
                                    .map(caps::Cap::as_string)
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            );
                        }
                        if !contract.args.is_empty() {
                            let _ = write!(line, "  args: {}", contract.args.join(", "));
                        }
                        if !contract.env.is_empty() {
                            let _ = write!(line, "  env: {}", contract.env.join(", "));
                        }
                        println!("{line}");
                    }
                    Err(error) => println!("  {}  error: {error}", b.name),
                }
            }
        }
    }
    Ok(())
}

/// `--dry` — the no-apply inspection leg, split by fence language.
fn dry(
    root: &fs::WorkspaceRoot,
    parsed: &RunArgs,
    resolved: &ResolvedTask,
    caps: &CapResolution,
) -> Result<(), Fail> {
    match resolved.block.lang {
        TaskLanguage::Starlark => dry_starlark(root, parsed, resolved, caps),
        TaskLanguage::Bash => {
            dry_bash(parsed, resolved, caps);
            Ok(())
        }
    }
}

/// Starlark `--dry`: END-TO-END descriptor truth. The hermetic kernel
/// evaluates the block (U5 `evaluate` seam — the SAME pure half the real run
/// uses) and the full effect set prints. Nothing applies; an eval fault is a
/// run-plane failure (exit 1).
fn dry_starlark(
    root: &fs::WorkspaceRoot,
    parsed: &RunArgs,
    resolved: &ResolvedTask,
    caps: &CapResolution,
) -> Result<(), Fail> {
    let task = &resolved.binding.name;
    let (_, root_at_eval) = fs::domain_snapshot(root)
        .map_err(|e| Fail::tool(format!("corpus snapshot failed: {e}")))?;
    let dispatch = run::dispatch_starlark::StarlarkDispatch {
        page: &parsed.page,
        task,
        task_rev: &resolved.task_rev,
        source: &resolved.block.source,
        args: parsed.args.clone(),
        env: parsed.env.clone(),
        invocation_id: DRY_INVOCATION,
        now: None,
        root_at_eval: &root_at_eval,
        caps: &caps.effective,
        receipt: None,
        takeover: false,
        limits: EvalLimits::default(),
    };
    let effects =
        run::dispatch_starlark::evaluate(&dispatch).map_err(|e| fail_run(format!("eval: {e}")))?;
    match parsed.format() {
        Format::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "page": parsed.page,
                    "task": task,
                    "lang": "starlark",
                    "guarantee": "hermetic",
                    "dry": true,
                    "applied": false,
                    "effects": effects,
                }))
                .expect("json render")
            );
        }
        Format::Human => {
            println!(
                "dry run: task '{task}' (starlark, hermetic) — {} effect(s), nothing applied",
                effects.len()
            );
            for (i, effect) in effects.iter().enumerate() {
                let args = effect
                    .args
                    .iter()
                    .map(|(k, v)| format!("{k}={v:?}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                println!("  {}. {} {args}", i + 1, effect.kind.as_str());
            }
        }
    }
    Ok(())
}

/// Bash `--dry`: show the block + the resolved caps and REFUSE to exec — bash
/// under `--dry` produces NO descriptors (running it would; inventing them
/// would be fiction, decision #18). Exit 0: the dry inspection succeeded, and
/// the refusal to exec is its content, not a failure.
fn dry_bash(parsed: &RunArgs, resolved: &ResolvedTask, caps: &CapResolution) {
    let task = &resolved.binding.name;
    let cap_list = cap_strings(caps);
    match parsed.format() {
        Format::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "page": parsed.page,
                    "task": task,
                    "lang": "bash",
                    "guarantee": "detected",
                    "dry": true,
                    "executed": false,
                    "caps": {
                        "effective": cap_list,
                        "source": source_label(&caps.source),
                    },
                    "source": resolved.block.source,
                }))
                .expect("json render")
            );
        }
        Format::Human => {
            println!("dry run: task '{task}' (bash, detected) — NOT executed");
            println!(
                "declared caps: {} [{}]",
                if cap_list.is_empty() {
                    "(read-only)".to_owned()
                } else {
                    cap_list.join(", ")
                },
                source_label(&caps.source),
            );
            println!("--- block ---");
            for line in resolved.block.source.lines() {
                println!("  {line}");
            }
            println!("--- end ---");
            println!("bash is not executed under --dry: its effects only exist by running it");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn parse_locked_surface() {
        let p = RunArgs::parse(&strings(&[
            "notes.md",
            "fix-drift",
            "--env",
            "HOME_WIKI=/w",
            "--dry",
            "--json",
            "--",
            "page",
            "--not-a-flag",
        ]))
        .expect("parse");
        assert_eq!(p.page, "notes.md");
        assert_eq!(p.task.as_deref(), Some("fix-drift"));
        assert_eq!(p.args, vec!["page", "--not-a-flag"]);
        assert_eq!(p.env.get("HOME_WIKI").map(String::as_str), Some("/w"));
        assert!(p.dry && p.json && !p.list);
    }

    #[test]
    fn parse_refusals_are_exit_2() {
        for tail in [
            vec![],                                              // no PAGE
            strings(&["p.md", "t", "extra"]),                    // third positional
            strings(&["p.md", "--flag"]),                        // unknown flag
            strings(&["p.md", "--env"]),                         // --env no value
            strings(&["p.md", "--env", "NOEQ"]),                 // not KEY=VALUE
            strings(&["p.md", "--env", "=v"]),                   // empty key
            strings(&["p.md", "--env", "A=1", "--env", "A=2"]),  // duplicate
            strings(&["p.md", "t", "--list"]),                   // list + TASK
            strings(&["p.md", "--list", "--dry"]),               // list + dry
        ] {
            let fail = RunArgs::parse(&tail).expect_err("must refuse");
            assert_eq!(fail.code, 2, "{tail:?} → {}", fail.message);
        }
    }

    #[test]
    fn triad_run_leg_mappings() {
        // Foreign edit (decision #26) → exit 1, never exit 2.
        let fe = fail_exec(&ExecError::ForeignEdit {
            target: "fm:status".to_owned(),
            last_governed: "aaaa".to_owned(),
            current: "bbbb".to_owned(),
        });
        assert_eq!(fe.code, EXIT_RUN);
        assert!(fe.message.contains("foreign edit"));

        // Workspace busy (decision #9 lock) → exit 1, named.
        let busy = fail_exec(&ExecError::Io {
            reason: "workspace lock: held".to_owned(),
        });
        assert_eq!(busy.code, EXIT_RUN);
        assert!(busy.message.contains("workspace busy"));

        // Root mismatch (out-of-band change) → exit 1.
        let rm = fail_exec(&ExecError::RootMismatch {
            expected: "e".to_owned(),
            actual: "a".to_owned(),
        });
        assert_eq!(rm.code, EXIT_RUN);

        // Bash fence under a read-only convention: the PLANE refuses → 1;
        // a malformed cap string is an authoring fault → 2.
        assert_eq!(
            fail_caps(&CapsError::BashFenceRefused {
                task: "check-x".to_owned(),
                pattern: "check-*".to_owned(),
            })
            .code,
            EXIT_RUN
        );
        assert_eq!(
            fail_caps(&CapsError::BadCap {
                raw: "nonsense".to_owned(),
            })
            .code,
            2
        );
    }
}
