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
//! # The three legs
//! `--list` surfaces every declared task with its contract, and its caps where
//! capabilities apply — a bash row states `effects: undeclared` and claims no
//! authority at all (`docs/laws.md` § Amendment — capabilities do not apply to
//! bash; gate `crates/mrd/tests/law_no_caps_on_bash.rs`). `--dry`
//! on starlark is END-TO-END truth: the hermetic kernel evaluates the block
//! through the U5 `evaluate` seam and the FULL effect set prints — nothing
//! applies. `--dry` on bash shows the block and REFUSES to
//! exec (no descriptor fiction, decision #18). The execute leg composes the
//! run through the U7 runner — with the EMPTY S1 ruleset ([`S1_RULES`]) —
//! and renders the U9 report (text and `--json` off one struct, exec facts
//! `RunReport`-sourced).

use std::collections::BTreeMap;
use std::path::Path;

use effects::EvalLimits;
use run::address::{self, AddressError, ResolvedTask};
use run::caps::{self, Authority, CapResolution, CapSource, CapsError, Conventions};
use run::contracts::{self, Contract};
use run::dispatch_bash::{BashError, Phase2};
use run::dispatch_starlark::DispatchError;
use run::exec::ExecStatus;
use run::executor::{ExecError, ReceiptAddr};
use run::fence::TaskLanguage;
use run::runner::{self, CascadeError, RunSpec, RunnerError, TaskOutcome};
use serde_json::json;

use crate::{Fail, Format, current_dir};

/// The run-plane leg of the triad: the invocation was well-formed, the plane
/// refused or failed.
const EXIT_RUN: u8 = 1;

/// The deterministic invocation id a `--dry` evaluation sees: `--dry` records
/// nothing, and a fixed id keeps dry output byte-stable run over run.
const DRY_INVOCATION: &str = "dry";

/// The S1 ruleset the CLI hands the runner: EMPTY, always (u7-gate forward
/// condition). Cascade applies are receipt-less by phase-2 scoping, so S1
/// must short-circuit the cascade before any apply — the runner's vacuous
/// stop on an empty ruleset is that short-circuit, and the CLI surface has
/// no rules input to widen it.
const S1_RULES: &[effects::Rule] = &[];

/// The workspace-relative receipt file `mrd run` appends to (executor
/// convention: address policy is the caller's; the executor scans every
/// `.md` beside it for foreign-edit anchors).
const RECEIPT_FILE: &str = "receipts/run.md";

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
/// PLANE refusing a well-formed invocation (exit 1); malformed declarations and
/// a root `MERIDIAN.md` that does not read as a root declaration are
/// authoring/tool faults (exit 2).
fn fail_caps(e: &CapsError) -> Fail {
    match e {
        CapsError::BashFenceRefused { .. } => fail_run(e.to_string()),
        CapsError::BadCap { .. } | CapsError::BadPattern { .. } | CapsError::Declaration { .. } => {
            Fail::tool(e.to_string())
        }
    }
}

/// Executor refusals all land on the run leg (exit 1). The two named legs of
/// the triad review: `ForeignEdit` (decision #26 refusal, never a silent
/// overwrite) and `WorkspaceBusy` (decision #9, the `LOCK_NB` refusal) — both
/// carry their typed `Display` and exit 1.
fn fail_exec(e: &ExecError) -> Fail {
    fail_run(e.to_string())
}

/// Map a runner refusal onto the triad. Pre-eval faults (addressing,
/// contract) are invocation faults (exit 2); everything past the gate is the
/// run plane refusing (exit 1), with `ExecError` legs routed through
/// [`fail_exec`].
fn fail_runner(e: &RunnerError) -> Fail {
    match e {
        RunnerError::Address(e) => fail_address(e),
        RunnerError::Contract(e) => Fail::tool(e.to_string()),
        RunnerError::Violation(e) => Fail::tool(e.to_string()),
        RunnerError::Caps(e) => fail_caps(e),
        RunnerError::Starlark(DispatchError::Exec(err))
        | RunnerError::Bash(BashError::Phase1(err)) => fail_exec(err),
        RunnerError::Cascade(CascadeError::Apply { error, .. }) => {
            let mut fail = fail_exec(error);
            fail.message = format!("cascade: {}", fail.message);
            fail
        }
        other => fail_run(other.to_string()),
    }
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
    let answer = workspace::resolve(&cwd)
        .map_err(|e| Fail::tool(format!("workspace resolution failed: {e:?}")))?;
    // `root_or_cwd` preserves today's behavior exactly: an unanchored tree runs
    // against the cwd. Whether `mrd run` should instead REFUSE a cwd-default
    // answer is the run-plane/surfaces card's call, not this one's — the
    // accessor is here so that acceptance is visible in the source and
    // greppable, rather than inherited from a struct field.
    let root = fs::WorkspaceRoot(answer.root_or_cwd().to_path_buf());
    // The two roots answer different questions and are deliberately not the same
    // expression: `root_or_cwd` above is WHERE FILES ARE READ, `answer.root()`
    // here is WHETHER ANYTHING IS ENTITLED TO DECLARE POLICY. They coincide
    // whenever the ladder answered; on a cwd default the first still runs and the
    // second is `None`, so no convention ceiling is in force.
    let declaring_root = answer.root();
    let doc = address::load_page(&root, Path::new(&parsed.page)).map_err(|e| fail_address(&e))?;
    let (conventions, _source) =
        caps::load_conventions(declaring_root).map_err(|e| fail_caps(&e))?;

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

    // Authority resolution (U2, deny-by-default on starlark; bash is
    // unsandboxed, and under check-*/verify-* refuses on the run leg).
    let authority = caps::resolve_authority(&doc, task, resolved.block.lang, &conventions)
        .map_err(|e| fail_caps(&e))?;

    if parsed.dry {
        return dry(&root, &parsed, &resolved, &authority);
    }

    execute(&root, declaring_root, &parsed, task)
}

/// The execute leg: compose the run through the U7 runner (empty ruleset —
/// see [`S1_RULES`]), render the U9 report, and map the outcome onto the
/// exit triad. The CLI is the §9 boundary: it mints the invocation id and
/// the time fact here, and nowhere below does.
fn execute(
    root: &fs::WorkspaceRoot,
    declaring_root: Option<&Path>,
    parsed: &RunArgs,
    task: &str,
) -> Result<(), Fail> {
    let (invocation_id, now) = mint_identity()?;
    let timeout =
        run::exec::configured_timeout(declaring_root).map_err(|e| Fail::tool(e.to_string()))?;
    let scratch = root.0.join(".meridian/scratch").join(&invocation_id);
    std::fs::create_dir_all(&scratch).map_err(|e| Fail::tool(format!("scratch dir: {e}")))?;

    let spec = RunSpec {
        page: &parsed.page,
        task: Some(task),
        args: parsed.args.clone(),
        env: parsed.env.clone(),
        invocation_id: &invocation_id,
        now: Some(&now),
        receipt: Some(ReceiptAddr {
            path: RECEIPT_FILE.to_owned(),
            anchor: format!("r-{invocation_id}"),
        }),
        pre_receipt: Some(ReceiptAddr {
            path: RECEIPT_FILE.to_owned(),
            anchor: format!("p-{invocation_id}"),
        }),
        takeover: false,
        scratch: &scratch,
        timeout,
        declaring_root,
        limits: EvalLimits::default(),
    };

    // Bash stdout streams LIVE to our stdout while the record tees it
    // out-of-tree (U8); starlark ignores the sink.
    let mut live = std::io::stdout();
    let result = runner::run(root, &spec, S1_RULES, &mut live);
    let _ = std::fs::remove_dir_all(&scratch);
    let report = result.map_err(|e| fail_runner(&e))?;

    // The U9 report: one object, text and json off the same struct — with
    // the RunReport-sourced exec facts (never receipt-sourced).
    let rendered = run::report::render(&report);
    match parsed.format() {
        Format::Json => println!(
            "{}",
            rendered
                .to_json()
                .map_err(|e| Fail::tool(format!("report encode: {e}")))?
        ),
        Format::Human => print!("{}", rendered.to_text()),
    }
    exit_leg(&report)
}

/// The exit leg for a run the runner carried to a report: a bash phase-2
/// refusal is a run-plane failure even though the report rendered — exit 1,
/// except a signaled step, which exits 128+signal (130 on SIGINT, plan §5).
fn exit_leg(report: &runner::RunReport) -> Result<(), Fail> {
    let TaskOutcome::Bash(outcome) = &report.outcome else {
        return Ok(());
    };
    let cause = match &outcome.phase2 {
        Phase2::Applied { .. } => return Ok(()),
        // G3b: the effects refused, the run was recorded. Both halves are said,
        // in that order — the operator's next move follows from the exit code,
        // not from the receipt.
        Phase2::RefusedExecFailed { .. } => match &outcome.status {
            ExecStatus::Exited { code } => format!(
                "bash exited {code} — no effect applied; the run is recorded with its exit code"
            ),
            // Unreachable: this variant is built only under `Exited`.
            other => format!("bash ended {other:?} — phase 2 refused"),
        },
        Phase2::RefusedSignaled => match &outcome.status {
            ExecStatus::Signaled { signal } => {
                let leg = u8::try_from(128 + *signal).unwrap_or(1);
                return Err(Fail {
                    code: leg,
                    message: format!("bash was signaled ({signal}) — run interrupted"),
                });
            }
            // Unreachable: this variant is built only under `Signaled`.
            other => format!("bash ended {other:?} — phase 2 refused"),
        },
        Phase2::RefusedTimeout => {
            "bash timed out — process group killed, phase 2 refused".to_owned()
        }
        Phase2::RefusedShim(e) => format!("effect-shim stream refused: {e}"),
        Phase2::RefusedDetection => outcome.detection.to_string(),
        Phase2::RefusedExec { error, .. } => return Err(fail_exec(error)),
    };
    Err(fail_run(cause))
}

/// Mint the run identity at the §9 boundary: a unique, path-safe invocation
/// id and a unix-seconds time fact.
fn mint_identity() -> Result<(String, String), Fail> {
    let elapsed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| Fail::tool(format!("system clock: {e}")))?;
    let secs = elapsed.as_secs();
    let id = format!(
        "run-{secs}{:03}-{}",
        elapsed.subsec_millis(),
        std::process::id()
    );
    Ok((id, secs.to_string()))
}

/// One task's `--list` / listing row facts.
fn task_row(
    doc: &model::Document,
    conventions: &Conventions,
    name: &str,
) -> Result<(ResolvedTask, Contract, Authority), String> {
    let resolved = address::resolve_task(doc, Some(name)).map_err(|e| e.to_string())?;
    let contract = contracts::contract_for(doc, name).map_err(|e| e.to_string())?;
    let authority = caps::resolve_authority(doc, name, resolved.block.lang, conventions)
        .map_err(|e| e.to_string())?;
    Ok((resolved, contract, authority))
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
                    Ok((resolved, contract, authority)) => {
                        let mut row = json!({
                            "task": b.name,
                            "lang": resolved.block.lang.as_str(),
                            "guarantee": resolved.block.lang.guarantee_class().as_str(),
                            "args": contract.args,
                            "env": contract.env,
                        });
                        // The `caps` key exists only where capabilities do —
                        // an unsandboxed row states its effects instead.
                        match authority.capabilities() {
                            Some(caps) => row["caps"] = json!({
                                "effective": cap_strings(caps),
                                "source": source_label(&caps.source),
                                "narrowed": caps.narrowed.iter().map(caps::Cap::as_string).collect::<Vec<_>>(),
                            }),
                            None => row["effects"] = json!(caps::UNDECLARED_EFFECTS),
                        }
                        row
                    }
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
                    Ok((resolved, contract, authority)) => {
                        let lang = resolved.block.lang.as_str();
                        let class = resolved.block.lang.guarantee_class().as_str();
                        let mut line = format!("  {}  {lang}  {class}", b.name);
                        // An unsandboxed row describes its effects; a
                        // capability row states its grant and what narrowed it.
                        match authority.capabilities() {
                            None => {
                                let _ = write!(line, "  effects: {}", caps::UNDECLARED_EFFECTS);
                            }
                            Some(caps) => {
                                let cap_list = cap_strings(caps);
                                let _ = write!(
                                    line,
                                    "  caps: {} [{}]",
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
                            }
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
    authority: &Authority,
) -> Result<(), Fail> {
    match resolved.block.lang {
        TaskLanguage::Starlark => dry_starlark(root, parsed, resolved, authority),
        TaskLanguage::Bash => {
            dry_bash(parsed, resolved);
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
    authority: &Authority,
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
        authority,
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

/// Bash `--dry`: show the block and REFUSE to exec — bash under `--dry`
/// produces NO descriptors (running it would; inventing them would be fiction,
/// decision #18). Exit 0: the dry inspection succeeded, and the refusal to exec
/// is its content, not a failure.
///
/// It takes no authority, and there is nothing to pass it: a bash block is an
/// unsandboxed shell with undeclared effects (`docs/laws.md` § Amendment), so
/// the two facts below are the whole honest description.
fn dry_bash(parsed: &RunArgs, resolved: &ResolvedTask) {
    let task = &resolved.binding.name;
    let class = TaskLanguage::Bash.guarantee_class().as_str();
    match parsed.format() {
        Format::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "page": parsed.page,
                    "task": task,
                    "lang": "bash",
                    "guarantee": class,
                    "dry": true,
                    "executed": false,
                    "effects": caps::UNDECLARED_EFFECTS,
                    "source": resolved.block.source,
                }))
                .expect("json render")
            );
        }
        Format::Human => {
            println!("dry run: task '{task}' (bash, {class}) — NOT executed");
            println!("effects: {} (unsandboxed shell)", caps::UNDECLARED_EFFECTS);
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
            vec![],                                             // no PAGE
            strings(&["p.md", "t", "extra"]),                   // third positional
            strings(&["p.md", "--flag"]),                       // unknown flag
            strings(&["p.md", "--env"]),                        // --env no value
            strings(&["p.md", "--env", "NOEQ"]),                // not KEY=VALUE
            strings(&["p.md", "--env", "=v"]),                  // empty key
            strings(&["p.md", "--env", "A=1", "--env", "A=2"]), // duplicate
            strings(&["p.md", "t", "--list"]),                  // list + TASK
            strings(&["p.md", "--list", "--dry"]),              // list + dry
        ] {
            let fail = RunArgs::parse(&tail).expect_err("must refuse");
            assert_eq!(fail.code, 2, "{tail:?} → {}", fail.message);
        }
    }

    /// u7-gate forward condition: the CLI invocation of the runner uses the
    /// EMPTY ruleset — cascade applies are receipt-less by phase-2 scoping,
    /// so S1 short-circuits the cascade via the runner's vacuous stop. The
    /// call site is pinned to [`S1_RULES`]; this test pins [`S1_RULES`].
    #[test]
    fn cli_hands_the_runner_an_empty_ruleset() {
        assert!(S1_RULES.is_empty(), "S1 must not evaluate cascade rules");
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

        // Workspace busy (decision #9, LOCK_NB) → exit 1, named.
        let busy = fail_exec(&ExecError::WorkspaceBusy);
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
