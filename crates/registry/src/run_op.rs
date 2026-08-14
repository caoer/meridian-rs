//! § A.8 `run` — page-task execution over the wire (run-crossing ruling,
//! 2026-08-13; `docs/wire-contract.md` § A.8, `docs/run-plane.md`).
//!
//! A LIST of targets on the bound workspace, run SEQUENTIALLY in list order
//! through the run plane's own seam ([`run::runner::run`]) — the serve path
//! consumes the plane, it never re-implements it (the charter amendment).
//! Each target answers for itself in a per-target row; no aggregate boolean
//! exists in the body. This op is also the transport of the ruled production
//! ARMING door ("use mrd run to run it"): arming gets no surface of its own —
//! a rule page's activation task is a task like any other, and the receipt is
//! the arming record.
//!
//! Laws held here, each pinned by `tests/run_op.rs`:
//!
//! - **Per-target independence**: a refused or failed target never halts,
//!   gates, or colors a later one; rows land in request order.
//! - **§9 identity**: the engine mints none — `invocation` is the host's
//!   base, per-target ids derive `<base>-t<index>`; a supplied `actor`
//!   threads into the receipt's actor fact.
//! - **Receipts are the plane's own**: engine-appended to
//!   [`run::executor::RECEIPT_FILE`] under the per-target anchor
//!   `r-<invocation>` — no receipt field exists on the wire.
//! - **U16 amendment**: the bash step's cwd on THIS op is the bound
//!   workspace root; the CLI entry is unchanged.
//! - **Refusal split**: the CLI triad's exit-2 family answers
//!   `class:"invocation"` rows, the exit-1 family `class:"run"` rows; §8
//!   `ok:false` frames answer only what never reached the plane.

use std::path::Path;
use std::time::Instant;

use effects::EvalLimits;
use run::address::AddressError;
use run::caps::{self, CapsError};
use run::executor::{RECEIPT_FILE, ReceiptAddr};
use run::fence::TaskLanguage;
use run::runner::{self, RunSpec, RunnerError};
use serde_json::{Map, Value, json};
use wire::{ErrorBody, ErrorCode};

use crate::registry::Registry;
use wire_serve::rev::Rev;

/// The wire arm hands the runner the same empty S1 ruleset as the CLI: the
/// cascade short-circuits vacuously before any apply.
const S1_RULES: &[effects::Rule] = &[];

/// Serve one `run` frame end to end, answering the full NDJSON line.
///
/// Routed from `handle_line` BEFORE the generic `serve_wire` path because the
/// success body embeds the run plane's own report objects verbatim — not a
/// `ResponseBody` variant.
pub(crate) fn serve_line(
    registry: &Registry,
    attached: Option<&Path>,
    obj: &Map<String, Value>,
    rev: Rev,
) -> String {
    let id = obj.get("id").and_then(Value::as_u64);
    let started = Instant::now();
    // Strict decode first (rev-agnostic), then the v3 gate — the script
    // dispatch order, so a malformed frame teaches its field wall on any rev.
    let op = match wire_serve::decode::decode(obj, rev) {
        Ok(op) => op,
        Err(error) => return error_line(id, *error, rev),
    };
    if rev != Rev::V3 {
        return error_line(id, ErrorBody::new(ErrorCode::UnknownOp), rev);
    }
    let Some(ws) = attached else {
        return error_line(
            id,
            *wire_serve::bad_request("no workspace bound — send `hello` with a `workspace` first"),
            rev,
        );
    };
    let wire::Op::Run {
        targets,
        invocation,
        actor,
        now,
    } = op
    else {
        // decode() maps the "run" tag to Op::Run only; any other arm here is
        // a routing defect, answered loud rather than misdispatched.
        return error_line(id, ErrorBody::new(ErrorCode::Internal), rev);
    };
    let request = RunArgs {
        targets,
        invocation,
        actor,
        now,
    };
    let rows = serve(registry, ws, &request);
    let mut frame = json!({"id": id, "ok": true, "body": {"targets": rows}});
    let duration_us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
    wire_serve::rev::attach_meta(&mut frame, duration_us);
    let mut line = serde_json::to_string(&frame).expect("a run frame serializes");
    line.push('\n');
    line
}

/// One typed error frame, rendered per negotiated rev (the `wire_line` path).
fn error_line(id: Option<u64>, error: ErrorBody, rev: Rev) -> String {
    crate::server::wire_line(
        &wire::Response {
            id,
            ok: false,
            payload: wire::ResponsePayload::Error { error },
        },
        rev,
        None,
    )
}

/// The decoded request, owned.
struct RunArgs {
    targets: Vec<wire::RunTarget>,
    invocation: String,
    actor: Option<String>,
    now: Option<String>,
}

/// The attempt: per target in list order, drive the plane and mint one row.
/// Each target answers for itself — the loop never breaks on a refusal.
///
/// Never refuses: decode already answered the frame-shape family, and plane
/// refusals ride the rows themselves (§ A.8).
fn serve(registry: &Registry, ws: &Path, request: &RunArgs) -> Vec<Value> {
    let root = fs::WorkspaceRoot(ws.to_path_buf());
    // Delta honesty (§ A.8): every committed batch of every target mints its
    // frame on the bound workspace's ring, inside the executor's flock.
    let sink = crate::delta_sink::RingSink::new(registry.ring(ws));
    let mut rows = Vec::with_capacity(request.targets.len());
    for (index, target) in request.targets.iter().enumerate() {
        let invocation = format!("{}-t{index}", request.invocation);
        rows.push(row_for_target(
            &root,
            ws,
            target,
            &invocation,
            request.actor.as_deref(),
            request.now.as_deref(),
            &sink,
        ));
    }
    rows
}

/// One target → one row, whichever door invoked it — the § A.8 op arm's loop
/// body and the effects-mode `run()` builtin (§ A.7) share this seam, so the
/// two doors cannot drift. `sink` is the workspace ring's frame mint: both
/// doors are daemon-side, so both mint (Delta honesty); a dry target commits
/// nothing and therefore mints nothing.
pub(crate) fn row_for_target(
    root: &fs::WorkspaceRoot,
    ws: &Path,
    target: &wire::RunTarget,
    invocation: &str,
    actor: Option<&str>,
    now: Option<&str>,
    sink: &crate::delta_sink::RingSink,
) -> Value {
    if target.dry.unwrap_or(false) {
        dry_row(root, ws, target, invocation, actor, now)
    } else {
        execute_row(root, ws, target, invocation, actor, now, sink)
    }
}

/// Execute one target through the plane's own seam and mint its row: the
/// report object verbatim plus addressing, or a refusal row.
fn execute_row(
    root: &fs::WorkspaceRoot,
    ws: &Path,
    target: &wire::RunTarget,
    invocation: &str,
    actor: Option<&str>,
    now: Option<&str>,
    sink: &crate::delta_sink::RingSink,
) -> Value {
    let timeout = match run::exec::configured_timeout(Some(ws)) {
        Ok(t) => t,
        Err(e) => return refusal_row(target, invocation, "invocation", &e.to_string(), None),
    };
    let scratch = root.0.join(".meridian/scratch").join(invocation);
    if let Err(e) = std::fs::create_dir_all(&scratch) {
        return refusal_row(
            target,
            invocation,
            "run",
            &format!("scratch dir: {e}"),
            None,
        );
    }
    let spec = RunSpec {
        page: &target.page,
        task: target.task.as_deref(),
        args: target.args.clone(),
        env: target.env.clone(),
        invocation_id: invocation,
        now,
        receipt: Some(ReceiptAddr {
            path: RECEIPT_FILE.to_owned(),
            anchor: format!("r-{invocation}"),
        }),
        pre_receipt: Some(ReceiptAddr {
            path: RECEIPT_FILE.to_owned(),
            anchor: format!("p-{invocation}"),
        }),
        takeover: false,
        scratch: &scratch,
        timeout,
        declaring_root: Some(ws),
        limits: EvalLimits::default(),
        actor,
        // § A.8's U16 amendment: a daemon has no meaningful cwd — the step
        // runs at the bound workspace root.
        step_cwd: Some(ws),
        delta: Some(sink),
    };
    // No live stream on the wire: the report's own sealed stdout record is
    // the exec-facts surface (§ A.8 — "this op streams nothing").
    let mut live = std::io::sink();
    let result = runner::run(root, &spec, S1_RULES, &mut live);
    let _ = std::fs::remove_dir_all(&scratch);
    match result {
        Ok(report) => {
            let mut row = serde_json::to_value(run::report::render(&report))
                .expect("a run report serializes");
            let obj = row.as_object_mut().expect("a report is an object");
            obj.insert("page".to_owned(), json!(target.page));
            obj.insert("invocation".to_owned(), json!(invocation));
            obj.insert(
                "receipt".to_owned(),
                json!(format!("{RECEIPT_FILE}#r-{invocation}")),
            );
            obj.insert("dry".to_owned(), json!(false));
            row
        }
        Err(e) => runner_refusal_row(root, target, invocation, &e),
    }
}

/// The plane's dry leg, row tense: the rehearsal seam ([`runner::rehearse`])
/// runs EVERY gate the live run enforces — address → contract → caps → eval →
/// the executor's own choke-point admission — and applies nothing. Refusals
/// arrive as the live run's own [`RunnerError`] values and ride the SAME
/// mapping ([`runner_refusal_row`]), so a rehearsed refusal is byte-identical
/// to the live one (dogfood r2 F2: dry-green must predict live-green).
fn dry_row(
    root: &fs::WorkspaceRoot,
    ws: &Path,
    target: &wire::RunTarget,
    invocation: &str,
    actor: Option<&str>,
    now: Option<&str>,
) -> Value {
    let spec = runner::RehearseSpec {
        page: &target.page,
        task: target.task.as_deref(),
        args: target.args.clone(),
        env: target.env.clone(),
        invocation_id: invocation,
        now,
        declaring_root: Some(ws),
        limits: EvalLimits::default(),
        actor,
    };
    match runner::rehearse(root, &spec) {
        Ok(rehearsal) => match rehearsal.outcome {
            runner::Rehearsed::Starlark { effects } => json!({
                "page": target.page,
                "invocation": invocation,
                "task": rehearsal.task,
                "lang": "starlark",
                "guarantee": "hermetic",
                "dry": true,
                "applied": false,
                "effects": effects,
            }),
            runner::Rehearsed::Bash { source } => json!({
                "page": target.page,
                "invocation": invocation,
                "task": rehearsal.task,
                "lang": "bash",
                "guarantee": TaskLanguage::Bash.guarantee_class().as_str(),
                "dry": true,
                "executed": false,
                "effects": caps::UNDECLARED_EFFECTS,
                "source": source,
            }),
        },
        Err(e) => runner_refusal_row(root, target, invocation, &e),
    }
}

/// Map a runner refusal onto the row classes — the CLI triad's split, row
/// tense: pre-eval faults (addressing, contract, authoring) are
/// `class:"invocation"`; everything past the gate is `class:"run"`.
fn runner_refusal_row(
    root: &fs::WorkspaceRoot,
    target: &wire::RunTarget,
    invocation: &str,
    error: &RunnerError,
) -> Value {
    match error {
        RunnerError::Address(e) => address_refusal_row(target, invocation, e),
        RunnerError::Contract(e) => {
            refusal_row(target, invocation, "invocation", &e.to_string(), None)
        }
        RunnerError::Violation(e) => {
            refusal_row(target, invocation, "invocation", &e.to_string(), None)
        }
        RunnerError::Caps(e) => caps_refusal_row(target, invocation, e),
        other => {
            // The several-tasks listing never lands here; every other address
            // fault did above. `root` keeps the signature honest for future
            // row enrichment without a second mapping seam.
            let _ = root;
            refusal_row(target, invocation, "run", &other.to_string(), None)
        }
    }
}

/// Address faults are the exit-2 family; the several-tasks fault carries the
/// declared names on its row (the CLI's list-then-exit-2, row tense).
fn address_refusal_row(target: &wire::RunTarget, invocation: &str, error: &AddressError) -> Value {
    let declared = match error {
        AddressError::ManyTasks { available } => Some(available.clone()),
        _ => None,
    };
    refusal_row(
        target,
        invocation,
        "invocation",
        &error.to_string(),
        declared,
    )
}

/// Cap faults split by leg exactly as at the CLI: a bash fence under a
/// read-only convention is the plane refusing (`run`); malformed
/// declarations are authoring faults (`invocation`).
fn caps_refusal_row(target: &wire::RunTarget, invocation: &str, error: &CapsError) -> Value {
    let class = match error {
        CapsError::BashFenceRefused { .. } => "run",
        CapsError::BadCap { .. } | CapsError::BadPattern { .. } | CapsError::Declaration { .. } => {
            "invocation"
        }
    };
    refusal_row(target, invocation, class, &error.to_string(), None)
}

/// One refusal row: addressing plus the typed cause, verbatim.
fn refusal_row(
    target: &wire::RunTarget,
    invocation: &str,
    class: &str,
    reason: &str,
    declared_tasks: Option<Vec<String>>,
) -> Value {
    let mut refusal = json!({"class": class, "reason": reason});
    if let Some(declared) = declared_tasks {
        refusal["declared_tasks"] = json!(declared);
    }
    let mut row = json!({
        "page": target.page,
        "invocation": invocation,
        "refusal": refusal,
    });
    if let Some(task) = &target.task {
        row["task"] = json!(task);
    }
    row
}
