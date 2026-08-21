//! `mrd run` — the local run plane mounted on the CLI. The argv surface is locked:
//!
//! ```text
//! mrd run <PAGE> [TASK] [-- ARGS] --env K=V --dry --list --json
//! ```
//!
//! No argv JSON: positional args ride verbatim after `--`, env rides as repeated
//! `--env KEY=VALUE` pairs, and both are contract-validated pre-eval.
//!
//! # Exit triad
//! - **0** — clean: `--list`, a completed `--dry`, a clean run. A foreign write landing
//!   BEFORE the exec window is reported (`pre-exec delta:` line), never refused — a
//!   foreign advance re-derives and proceeds (2026-08-15 no-guard amendment,
//!   `docs/run-plane.md`; the former `--fatal-preexec` opt-in is RETIRED with the
//!   plane's premise refusals).
//! - **1** — the run plane refused or failed: eval fault, a bash fence under a read-only
//!   convention, workspace busy, in-window out-of-band delta, timeout, bash nonzero
//!   exit (the foreign-edit and root-mismatch legs are RETIRED — same amendment).
//! - **2** — the invocation is wrong (usage, addressing, contract violation) or the tool failed
//!   pre-run. TASK omitted with several declared tasks lists them and exits 2 — unless one is
//!   named `default`, which runs (the 2026-08-19 default-task amendment, `docs/run-plane.md`).
//!
//! # The three legs
//! `--list` surfaces every declared task with its contract, and its caps where capabilities
//! apply — a bash row states `effects: undeclared` and claims no authority (capabilities do not
//! apply to bash; gate `crates/mrd/tests/law_no_caps_on_bash.rs`). `--dry` on starlark evaluates
//! the block through the U5 `evaluate` seam and prints the full effect set, applying nothing;
//! `--dry` on bash shows the block and refuses to exec. The execute leg composes the run through
//! the U7 runner — with the empty S1 ruleset ([`S1_RULES`]) — and renders the U9 report (text
//! and `--json` off one struct, exec facts `RunReport`-sourced).

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
use run::fence::{GuaranteeClass, TaskLanguage};
use run::runner::{self, CascadeError, RunSpec, RunnerError, TaskOutcome};
use serde_json::json;

use crate::{Fail, Format, current_dir};

/// Empty run-birth fields for the CLI entry (no frame passthrough).
static EMPTY_RUN_FIELDS: BTreeMap<String, String> = BTreeMap::new();

/// The run-plane leg of the triad: the invocation was well-formed, the plane refused or failed.
const EXIT_RUN: u8 = 1;

/// The deterministic invocation id a `--dry` evaluation sees, so dry output is byte-stable run
/// over run.
const DRY_INVOCATION: &str = "dry";

/// The S1 ruleset the CLI hands the runner: always empty. Cascade applies are receipt-less by
/// phase-2 scoping, so S1 must short-circuit the cascade before any apply — the runner's vacuous
/// stop on an empty ruleset is that short-circuit.
const S1_RULES: &[effects::Rule] = &[];

/// The workspace-relative receipt file `mrd run` appends to — the plane's own
/// [`run::executor::RECEIPT_FILE`] (one convention, both doors — § A.8).
const RECEIPT_FILE: &str = run::executor::RECEIPT_FILE;

/// A run-plane refusal (exit 1) — distinct from [`Fail::tool`]'s exit 2.
fn fail_run(message: String) -> Fail {
    Fail::with_code(EXIT_RUN, message)
}

/// Addressing faults are invocation faults: exit 2.
fn fail_address(e: &AddressError) -> Fail {
    Fail::tool(e.to_string())
}

/// A page miss names the root it anchored to and which rung named it. The ref
/// is the part of the invocation most likely to be correct; the root is the
/// part the environment may have swapped underneath it (dogfood F6: a sticky
/// `MERIDIAN_WORKSPACE` made the correct ref miss, and the bare refusal
/// pointed at the ref). A `cwd-default` answer carries no root
/// (`Answer::root` is `None`) — a defaulted cwd is not a workspace, so that
/// miss stays bare. The anchored miss carries the family's fitted respelling
/// when it is earned — the same sentence, the same one computation, as the
/// read door's ([`crate::path_law::cwd_respell_suffix`]).
fn fail_address_in(e: &AddressError, answer: &workspace::Answer, cwd: &Path) -> Fail {
    match (e, answer.root()) {
        (AddressError::PageNotFound { path }, Some(root)) => {
            let mut m = format!(
                "{e} (workspace {}, source: {})",
                root.display(),
                answer.tier().word()
            );
            if let Some(suffix) = crate::path_law::cwd_respell_suffix(root, cwd, path) {
                m.push_str(&suffix);
            }
            Fail::tool(m)
        }
        _ => fail_address(e),
    }
}

/// The rooted miss: scoped to the NAMED root (F4) — it names which root was
/// searched and its bound workspace, and never carries the ambient lane's
/// tier word or cwd respelling, advice for a different mistake.
fn fail_address_rooted(
    e: &AddressError,
    rooted: &crate::rooted::RootedRef,
    root: &fs::WorkspaceRoot,
) -> Fail {
    match e {
        AddressError::PageNotFound { .. } => Fail::tool(format!(
            "{e} (root `{}`, workspace {})",
            rooted.name,
            root.0.display()
        )),
        _ => fail_address(e),
    }
}

/// Cap faults split by leg: a bash fence under a read-only convention is the plane refusing a
/// well-formed invocation (exit 1); malformed declarations are authoring faults (exit 2).
fn fail_caps(e: &CapsError) -> Fail {
    match e {
        CapsError::BashFenceRefused { .. } => fail_run(e.to_string()),
        CapsError::BadCap { .. }
        | CapsError::BadGlob { .. }
        | CapsError::RetiredTarget { .. }
        | CapsError::BadPattern { .. }
        | CapsError::Declaration { .. }
        | CapsError::TableEntry { .. } => Fail::tool(e.to_string()),
    }
}

/// Executor refusals all land on the run leg (exit 1), carrying their typed `Display`.
fn fail_exec(e: &ExecError) -> Fail {
    fail_run(e.to_string())
}

/// Map a runner refusal onto the triad. Pre-eval faults (addressing, contract) are invocation
/// faults (exit 2); everything past the gate is the run plane refusing (exit 1), with
/// `ExecError` legs routed through [`fail_exec`].
fn fail_runner(e: &RunnerError) -> Fail {
    match e {
        RunnerError::Address(e) => fail_address(e),
        RunnerError::Contract(e) => Fail::tool(e.to_string()),
        RunnerError::Violation(e) => Fail::tool(e.to_string()),
        RunnerError::Caps(e) => fail_caps(e),
        RunnerError::Starlark(DispatchError::Exec(err))
        | RunnerError::Bash(BashError::Phase1(err)) => fail_exec(err),
        RunnerError::Cascade(e) => match &**e {
            CascadeError::Apply { error, .. } => {
                let mut fail = fail_exec(error);
                fail.message = format!("cascade: {}", fail.message);
                fail
            }
            other => fail_run(format!("cascade: {other}")),
        },
        other => fail_run(other.to_string()),
    }
}

/// The parsed `mrd run` invocation.
// Three independent argv switches ARE the surface: --json composes with the
// other legs, so an enum would invent coupling the CLI does not have.
#[allow(clippy::struct_excessive_bools)]
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
    /// Parse the tail after `run`. Unknown flags, a malformed or duplicate `--env`, and a third
    /// positional all refuse with exit 2.
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

/// Run `mrd run <tail>`. Errors [`Fail`] on the triad's 1/2 legs — see the module docs for the
/// mapping.
pub(crate) fn dispatch(tail: &[String]) -> Result<(), Fail> {
    let mut parsed = RunArgs::parse(tail)?;
    let cwd = current_dir()?;
    let answer = workspace::resolve(&cwd)
        .map_err(|e| Fail::tool(format!("workspace resolution failed: {e:?}")))?;
    // The rooted lane (§4.1 colon law), under the 2026-08-18 authority ruling
    // (rooted-refs-everywhere): a head-colon PAGE runs exactly as if the
    // caller had cd'd into the named root — the page load, the convention
    // ceiling, the timeout, the scratch dir, and the receipt all bind to the
    // PAGE's tree, never the standing one. The runtime cwd decides nothing on
    // this lane. A resolution refusal is the address answer (exit 1, the
    // `{workspace, error}` frame under `--json`), never a literal-path run.
    let rooted = match crate::rooted::enter(
        &parsed.page,
        "run",
        "Nothing was executed and no receipt was written.",
    ) {
        Ok(Some((rel, rooted))) => {
            parsed.page = rel;
            Some(rooted)
        }
        Ok(None) => None,
        // The refusal frames with the workspace the caller stands in — no
        // target workspace exists to name.
        Err(error) => {
            let ambient = answer.root_or_cwd().to_path_buf();
            return Err(crate::engine::json_refusal(
                parsed.format(),
                &ambient,
                &error,
            ));
        }
    };
    // The ambient lane takes the strict resolution: a tree outside every
    // defined root refuses (exit 2) instead of executing against a cwd that
    // was never a workspace.
    let root = match &rooted {
        Some(r) => fs::WorkspaceRoot(r.workspace.clone()),
        None => match answer.root() {
            Some(root) => fs::WorkspaceRoot(root.to_path_buf()),
            None => fs::WorkspaceRoot(
                crate::resolve::resolve_runtime(&cwd)
                    .map_err(|e| {
                        Fail::tool(format!(
                            "cannot resolve workspace for {}: {e}",
                            cwd.display()
                        ))
                    })?
                    .workspace,
            ),
        },
    };
    // §1 admission, before the page is read: without it `load_page` resolves an
    // absolute spelling verbatim and this door EXECUTED a page from outside the
    // workspace — writing the receipt into the workspace's own
    // `receipts/run.md` (wire-contract §12.1, the door-family clause; § A.8:
    // `page` is workspace-relative). Ordered ABOVE the echo-law rebind: the
    // family refuses the absolute spelling with its fitted respell, so the
    // rebind below no-ops for every admitted argv (the same string back) and
    // stays the resolution seam for the wire door.
    crate::path_law::admit(
        &root.0,
        &parsed.page,
        "run",
        "Nothing was executed and no receipt was written.",
    )?;
    // §2.1 echo law at the argv boundary: the receipt and the plane's page
    // addressing (pre-eval load, task resolution, the report's identity) key
    // on the page's ONE workspace-relative spelling, so the admitted ref
    // resolves here and rides root-relative everywhere below. (The
    // foreign-edit scan that once keyed on it is RETIRED — the 2026-08-15
    // no-guard-on-effects ruling — and the CLI hands the runner an empty
    // ruleset, [`S1_RULES`], so no rule scoping consumes it here.) A ref
    // resolving outside the root has no such spelling and stays verbatim —
    // refusing it is the path-law door family's business.
    if let Some(rel) = fs::workspace_relative(&root, &parsed.page) {
        parsed.page = rel;
    }
    // The two roots answer different questions: `root` above is where files
    // are read, `declaring_root` here is whether anything is entitled to
    // declare policy. On the ambient lane they coincide whenever the ladder
    // answered (a cwd default declares nothing, so no convention ceiling is in
    // force). On the rooted lane BOTH are the page's tree — the authority
    // ruling: a workspace that declares read-only keeps its own ceiling no
    // matter where the caller stands, so a rooted ref can never be a
    // permission bypass by cd.
    let declaring_root = match &rooted {
        Some(r) => Some(r.workspace.as_path()),
        None => answer.root(),
    };
    let doc = address::load_page(&root, Path::new(&parsed.page)).map_err(|e| match &rooted {
        Some(r) => fail_address_rooted(&e, r, &root),
        None => fail_address_in(&e, &answer, &cwd),
    })?;
    let (conventions, _source) =
        caps::load_conventions(declaring_root).map_err(|e| fail_caps(&e))?;

    if parsed.list {
        return list_tasks(&root, &parsed.page, &doc, &conventions, parsed.format());
    }

    // TASK omitted: one binding runs, or among several the one named `default`
    // (the declared election — address::DEFAULT_TASK); several with no
    // `default` list themselves and exit 2.
    let resolved = match address::resolve_task(&doc, parsed.task.as_deref()) {
        Ok(resolved) => resolved,
        Err(e @ AddressError::ManyTasks { .. }) => {
            list_tasks(&root, &parsed.page, &doc, &conventions, parsed.format())?;
            return Err(fail_address(&e));
        }
        Err(e) => return Err(fail_address(&e)),
    };
    let task = &resolved.binding.name;

    // Contract gate: violations exit 2 with the declared contract shown.
    let contract = contracts::contract_for(&doc, task).map_err(|e| Fail::tool(e.to_string()))?;
    contracts::validate(task, &contract, &parsed.args, &parsed.env).map_err(|violation| {
        Fail::tool(format!(
            "{violation}\ndeclared contract: args: [{}], env: [{}]",
            contract.args_declared().join(", "),
            contract.env.join(", ")
        ))
    })?;

    // Deny-by-default on starlark; bash is unsandboxed, and under check-*/verify-* refuses on
    // the run leg. The value feeds no leg below (execute and rehearse both re-resolve inside
    // the plane); the call stands for its refusal, so both legs teach caps faults pre-run.
    caps::resolve_authority(&doc, task, resolved.block.lang, &conventions)
        .map_err(|e| fail_caps(&e))?;

    if parsed.dry {
        return dry(&root, declaring_root, &parsed);
    }

    execute(&root, declaring_root, &parsed, task)
}

/// The execute leg: compose the run through the U7 runner (empty ruleset — see [`S1_RULES`]),
/// render the U9 report, and map the outcome onto the exit triad. The CLI is the boundary that
/// mints the invocation id and the time fact; nothing below does.
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
        scratch: &scratch,
        timeout,
        declaring_root,
        limits: EvalLimits::default(),
        // The CLI is its own host: the receipt keeps the plane's `run:<task>`
        // self-label, and U16 stands as written — the step runs where `mrd`
        // runs (§ A.8 amends the WIRE arm only).
        actor: None,
        step_cwd: None,
        // The CLI is a separate process: no resident memo in reach — the
        // drawer memo is this lane's instrument (card run-observation-unification).
        observations: run::dispatch_bash::ObservationSource::Drawer,
        // A separate process with no ring in reach: CLI commits stay
        // external change (§18 row 12; § A.8 Delta honesty, CLI arm).
        delta: None,
        // No frame fields ride the CLI entry today — a birth lands unstamped
        // (the documented bare-door behavior); the wire arm is the stamped
        // lane (cap `run.fields`). No ring in reach either.
        fields: &EMPTY_RUN_FIELDS,
        birth_seq: None,
        // The CLI entry is the documented bare door (md-create-ambient-
        // paths): a bare birth path stays workspace-root-relative here; the
        // caller-resolved ambient lane is the wire arm's (cap `run.ambient`).
        ambient: None,
    };

    // Bash stdout streams live to our stdout while the record tees it out-of-tree; starlark
    // ignores the sink.
    let mut live = std::io::stdout();
    let result = runner::run(root, &spec, S1_RULES, &mut live);
    let _ = std::fs::remove_dir_all(&scratch);
    let report = result.map_err(|e| fail_runner(&e))?;

    // One object, text and json off the same struct, with RunReport-sourced exec facts.
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

/// The exit leg for a run the runner carried to a report: a bash phase-2 refusal is a run-plane
/// failure even though the report rendered — exit 1, except a signaled step, which exits
/// 128+signal.
fn exit_leg(report: &runner::RunReport) -> Result<(), Fail> {
    let TaskOutcome::Bash(outcome) = &report.outcome else {
        return Ok(());
    };
    let cause = match &outcome.phase2 {
        Phase2::Applied { .. } => return Ok(()),
        // The effects refused, the run was recorded — both halves are said, in that order.
        Phase2::RefusedExecFailed { .. } => match &outcome.status {
            ExecStatus::Exited { code } => format!(
                "bash exited {code} — no effect applied; the run is recorded with its exit code"
            ),
            // Unreachable: this variant is built only under `Exited`.
            other => format!("bash ended {other:?} — no effect applied"),
        },
        Phase2::RefusedSignaled => match &outcome.status {
            ExecStatus::Signaled { signal } => {
                let leg = u8::try_from(128 + *signal).unwrap_or(1);
                return Err(Fail::with_code(
                    leg,
                    format!("bash was signaled ({signal}) — run interrupted"),
                ));
            }
            // Unreachable: this variant is built only under `Signaled`.
            other => format!("bash ended {other:?} — no effect applied"),
        },
        // User voice, never phase vocabulary (report-voice audit, 2026-08-15):
        // the sibling exited-nonzero arm's "no effect applied" is the model.
        Phase2::RefusedTimeout => {
            "bash timed out — process group killed, no effect applied".to_owned()
        }
        Phase2::RefusedDetection => outcome.detection.to_string(),
        Phase2::RefusedExec { error, .. } => return Err(fail_exec(error)),
    };
    Err(fail_run(cause))
}

/// Mint the run identity: a unique, path-safe invocation id and a unix-seconds time fact.
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

/// `--list`: every declared task with its language, guarantee class, contract, and resolved
/// caps. A broken binding shows its typed error as the row, so it never hides the rest.
fn list_tasks(
    _root: &fs::WorkspaceRoot,
    page: &str,
    doc: &model::Document,
    conventions: &Conventions,
    format: Format,
) -> Result<(), Fail> {
    let bindings = address::declared(doc).map_err(|e| fail_address(&e))?;
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
                            "args": contract.args_declared(),
                            "env": contract.env,
                        });
                        // The `caps` key exists only where capabilities do; an unsandboxed row
                        // states its effects instead.
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
                        // The class cell renders only where the guarantee is
                        // POSITIVE (`hermetic`) — `unsandboxed` names a sandbox
                        // that does not exist (ZT ruling, 2026-08-15). The
                        // `--json` `guarantee` key is unchanged.
                        let class = resolved.block.lang.guarantee_class();
                        let mut line = if class == GuaranteeClass::Unsandboxed {
                            format!("  {}  {lang}", b.name)
                        } else {
                            format!("  {}  {lang}  {}", b.name, class.as_str())
                        };
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
                            let _ = write!(line, "  args: {}", contract.args_declared().join(", "));
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

/// `--dry` — the no-apply inspection leg: the plane's own rehearsal seam
/// ([`runner::rehearse`]) runs EVERY gate the live run enforces — address →
/// contract → caps → eval → the executor's choke-point admission — and
/// applies nothing. Refusals ride [`fail_runner`], the live mapping, so a
/// rehearsed refusal reads exactly as the live one (dogfood r2 F2).
fn dry(
    root: &fs::WorkspaceRoot,
    declaring_root: Option<&Path>,
    parsed: &RunArgs,
) -> Result<(), Fail> {
    let spec = runner::RehearseSpec {
        page: &parsed.page,
        task: parsed.task.as_deref(),
        args: parsed.args.clone(),
        env: parsed.env.clone(),
        invocation_id: DRY_INVOCATION,
        now: None,
        declaring_root,
        limits: EvalLimits::default(),
        actor: None,
        // The CLI's dry leg matches its live leg: bare door, no ambient.
        ambient: None,
    };
    let rehearsal = runner::rehearse(root, &spec).map_err(|e| fail_runner(&e))?;
    match rehearsal.outcome {
        runner::Rehearsed::Starlark { effects } => {
            dry_starlark(parsed, &rehearsal.task, &effects);
        }
        runner::Rehearsed::Bash { source } => dry_bash(parsed, &rehearsal.task, &source),
    }
    Ok(())
}

/// Starlark `--dry` rendering: the full effect set prints, nothing applied.
fn dry_starlark(parsed: &RunArgs, task: &str, effects: &[effects::Effect]) {
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
}

/// Bash `--dry` rendering: show the block and refuse to exec. Bash under `--dry` produces no
/// descriptors — only running it would, and inventing them would be fiction.
fn dry_bash(parsed: &RunArgs, task: &str, source: &str) {
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
                    "source": source,
                }))
                .expect("json render")
            );
        }
        Format::Human => {
            // No guarantee word for bash: there is no sandbox, so the negation
            // names nothing (ZT ruling, 2026-08-15). `--json` keeps the class.
            println!("dry run: task '{task}' (bash) — NOT executed");
            println!("effects: {}", caps::UNDECLARED_EFFECTS);
            println!("--- block ---");
            for line in source.lines() {
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

    /// The 2026-08-15 no-guard amendment retired the `--fatal-preexec`
    /// opt-in with the plane's premise refusals: the flag no longer parses,
    /// and the usage line no longer teaches it.
    #[test]
    fn fatal_preexec_flag_is_retired() {
        let fail = RunArgs::parse(&strings(&["notes.md", "census", "--fatal-preexec"]))
            .expect_err("a retired flag refuses as unknown");
        assert_eq!(fail.code, 2);
        assert!(fail.message.contains("unknown flag"), "{}", fail.message);

        let usage = RunArgs::parse(&[]).expect_err("no PAGE refuses");
        assert!(
            !usage.message.contains("--fatal-preexec"),
            "{}",
            usage.message
        );
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

    /// The CLI invocation of the runner uses the empty ruleset: the call site is pinned to
    /// [`S1_RULES`], and this test pins [`S1_RULES`].
    #[test]
    fn cli_hands_the_runner_an_empty_ruleset() {
        assert!(S1_RULES.is_empty(), "S1 must not evaluate cascade rules");
    }

    #[test]
    fn triad_run_leg_mappings() {
        // Workspace busy (LOCK_NB) → exit 1, named.
        let busy = fail_exec(&ExecError::WorkspaceBusy);
        assert_eq!(busy.code, EXIT_RUN);
        assert!(busy.message.contains("workspace busy"));

        // Bash fence under a read-only convention: the plane refuses → 1;
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
