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

use run::modes;
use wire::{ErrorBody, ErrorCode};

use crate::registry::Registry;
use wire_serve::rev::Rev;

/// Empty run-birth fields for callers with no frame passthrough in reach
/// (the script plane's in-script `run()` — its entry carries no `fields`
/// today, so a birth it commits lands unstamped, the documented bare-door
/// behavior).
pub(crate) static EMPTY_RUN_FIELDS: std::collections::BTreeMap<String, String> =
    std::collections::BTreeMap::new();

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
        prelude,
        actor,
        now,
        fields,
        ambient,
    } = op
    else {
        // decode() maps the "run" tag to Op::Run only; any other arm here is
        // a routing defect, answered loud rather than misdispatched.
        return error_line(id, ErrorBody::new(ErrorCode::Internal), rev);
    };
    let request = RunArgs {
        targets,
        invocation,
        prelude,
        actor,
        now,
        fields,
        ambient,
    };
    // A mode-bearing target reads the PINNED corpus, so the submission takes
    // the same § 3.2 cold gate the script op takes: on a cold workspace the
    // drawer rebuilds in the background and the attempt never begins
    // (`corpus_warming`, retry). A fire is not the read door, so § 3.2's
    // never-blocked promise does not cover it — it refuses, it does not wait,
    // and it does not bypass.
    if request.targets.iter().any(modes::is_mode_target)
        && let Err(error) = crate::server::cold_gate_wire(registry, ws)
    {
        return error_line(id, *error, rev);
    }
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
    /// Load-phase source (cap `run.mode`), one per call, shared by every
    /// mode-bearing target. Inert on a task target — the shipped path never
    /// reads it, which is what keeps "byte-identical for existing callers"
    /// a fact rather than a hope.
    prelude: Option<String>,
    actor: Option<String>,
    now: Option<String>,
    /// § A.2.1 passthrough for run-plane births (cap `run.fields`) —
    /// verbatim onto every `md.create` birth's `ctx.fields`.
    fields: std::collections::BTreeMap<String, String>,
    /// The caller's ambient directory (cap `run.ambient`), workspace-
    /// relative — bare `md.create` paths birth under it (md-create-ambient-
    /// paths, shape (c)). Path-law-validated at the strict decode wall.
    ambient: Option<String>,
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
    // A second ring handle: the create door's SeqSink for run-plane births.
    let birth_ring = registry.ring(ws);
    // Observation unification (engine-warm-cost design § 5): the daemon door
    // serves the bash bracket's observations from the workspace's resident
    // domain memo — the same instrument every currency pass runs on.
    let cache = registry.domain_cache(ws);
    let host = RunHost {
        sink: &sink,
        birth_seq: &*birth_ring,
        fields: &request.fields,
        ambient: request.ambient.as_deref(),
        cache: &cache,
    };
    // The mode-bearing rows pin the RESIDENT snapshot — an `Arc` clone, the
    // script op's entry (`script_op.rs`) — and never run the
    // `domain_snapshot` fold the task path takes. Pinned once per submission,
    // so every row of one call reads one corpus.
    let any_mode = request.targets.iter().any(modes::is_mode_target);
    let pinned = any_mode
        .then(|| {
            // Deliberately discarded: `cold_gate_wire` above has already
            // refused the cold case with `corpus_warming`, so reaching here
            // means a warm (or warming) workspace and this call is the nudge,
            // not the gate. A bare `.ok()` otherwise reads as a swallowed
            // failure. (PR 195 review, e9f1ae35, N1.)
            registry.warm_or_build(ws).ok();
            registry.engine_snapshot(ws)
        })
        .flatten();
    // The § 2.2 module cache, resident per workspace: a block is evaluated
    // once per rev, so a warm fire is one function call. The CLI has no
    // equivalent and passes `None` — a fresh process per invocation has
    // nowhere to keep one.
    let modules = any_mode.then(|| registry.modules(ws));
    let mut rows = Vec::with_capacity(request.targets.len());
    for (index, target) in request.targets.iter().enumerate() {
        let invocation = format!("{}-t{index}", request.invocation);
        if modes::is_mode_target(target) {
            rows.push(match &pinned {
                Some(world) => match world.docs.get(&target.page) {
                    Some(doc) => modes::mode_row(
                        &modes::ModeWorld {
                            doc,
                            root: &root,
                            declaring_root: Some(ws),
                            observed_root: &world.at_fingerprint,
                            prelude: request.prelude.as_deref(),
                            doors: modes::Doors {
                                delta: Some(host.sink),
                                birth_seq: Some(host.birth_seq),
                                fields: Some(host.fields),
                                ambient: host.ambient,
                            },
                            cache: modules.as_deref().map(|m| m as &dyn modes::ModuleCache),
                        },
                        target,
                        &invocation,
                        request.actor.as_deref(),
                        request.now.as_deref(),
                    ),
                    // The pinned corpus does not carry this page. Answered on
                    // the row so its siblings still report for themselves.
                    None => json!({
                        "page": target.page,
                        "invocation": invocation,
                        "result": "refused",
                        "fault": {
                            "class": "bad_path",
                            "reason": format!(
                                "no such page in the pinned corpus: {}", target.page
                            ),
                        },
                    }),
                },
                // The reaper won the warm→pin race — the same transient the
                // read path names, answered on the row so its siblings still
                // report for themselves.
                None => json!({
                    "page": target.page,
                    "invocation": invocation,
                    "result": "refused",
                    "fault": {
                        "class": "corpus_race",
                        "reason": "the warm engine was reaped between the entry pass and \
                                   the pin — transient; retry",
                    },
                }),
            });
            continue;
        }
        rows.push(row_for_target(
            &root,
            ws,
            target,
            &invocation,
            request.actor.as_deref(),
            request.now.as_deref(),
            &host,
        ));
    }
    rows
}

/// The daemon-side facilities one submission's targets ride: the workspace
/// ring's frame mint (§ A.8 Delta honesty) and the resident domain memo (the
/// observation instrument, card run-observation-unification). Both doors —
/// the § A.8 op arm and the § A.7 in-script `run()` — hold one per
/// submission, so the two instruments cannot drift apart between doors.
pub(crate) struct RunHost<'a> {
    pub(crate) sink: &'a crate::delta_sink::RingSink,
    /// The workspace ring as the create door's `SeqSink` — run-plane births
    /// (`md.create`) mint numbered frames like any door write.
    pub(crate) birth_seq: &'a dyn wire_serve::seq::SeqSink,
    /// § A.2.1 run-frame `fields` (cap `run.fields`) — verbatim onto every
    /// `md.create` birth's `ctx.fields` this request commits.
    pub(crate) fields: &'a std::collections::BTreeMap<String, String>,
    /// The run frame's `ambient` (cap `run.ambient`) — the caller's ambient
    /// directory every bare birth target this request commits resolves
    /// under. `None` when the host attached none.
    pub(crate) ambient: Option<&'a str>,
    pub(crate) cache: &'a std::sync::Mutex<fs::DomainCache>,
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
    host: &RunHost<'_>,
) -> Value {
    // §2.1 echo law at the wire boundary: the receipt fact, the row's `page`
    // addressing, and the refusal rows all echo the target ref, so it
    // resolves to its ONE workspace-relative spelling here, before either
    // leg. A ref resolving outside the root has no such spelling and stays
    // verbatim — refusing it is the path-law door family's business.
    let resolved;
    let target = match fs::workspace_relative(root, &target.page) {
        Some(page) => {
            resolved = wire::RunTarget {
                page,
                ..target.clone()
            };
            &resolved
        }
        None => target,
    };
    if target.dry.unwrap_or(false) {
        dry_row(root, ws, target, invocation, actor, now, host)
    } else {
        execute_row(root, ws, target, invocation, actor, now, host)
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
    host: &RunHost<'_>,
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
        scratch: &scratch,
        timeout,
        declaring_root: Some(ws),
        limits: EvalLimits::default(),
        actor,
        // § A.8's U16 amendment: a daemon has no meaningful cwd — the step
        // runs at the bound workspace root.
        step_cwd: Some(ws),
        delta: Some(host.sink),
        fields: host.fields,
        birth_seq: Some(host.birth_seq),
        ambient: host.ambient,
        // The daemon lane (card run-observation-unification): bracket
        // observations from the resident domain memo; the drawer memo stays
        // the CLI's instrument.
        observations: run::dispatch_bash::ObservationSource::Resident(host.cache),
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
            // Live grammar (ZT ruling 2026-08-14, walk-wire boundary): the
            // pointer names the anchor as the file carries it (`^r-…`) and
            // joins with `§`, never the retired `#` — the spelling every
            // face teaching speaks. The path stays workspace-root-relative.
            obj.insert(
                "receipt".to_owned(),
                json!(format!("{RECEIPT_FILE} §^r-{invocation}")),
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
    host: &RunHost<'_>,
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
        // Dry/live parity: the rehearsal resolves birth targets under the
        // SAME ambient the live leg would, so a dry effect list shows the
        // resolved landing paths (dogfood r2 F2).
        ambient: host.ambient,
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
        CapsError::BadCap { .. }
        | CapsError::BadGlob { .. }
        | CapsError::RetiredTarget { .. }
        | CapsError::BadPattern { .. }
        | CapsError::Declaration { .. }
        | CapsError::TableEntry { .. } => "invocation",
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
