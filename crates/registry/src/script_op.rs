//! § A.7 `script` — the in-process lane of the run plane's script entry
//! (phase-2 script-plane ruling, 2026-08-12; `docs/wire-contract.md` § A.7,
//! `docs/run-plane.md` § The script entry, the entry-world amendment).
//!
//! One frame carries the whole program. The daemon runs the currency pass
//! ONCE at entry, pins the entry world (an [`Arc`] clone of the resident
//! engine — a rebuild swaps the map entry while the held clone keeps the
//! entry generation alive for exactly this attempt), evaluates the program
//! in-process under the kernel's own containment, threads each armed row's
//! CAS token from the ENTRY world, and commits through the one write
//! choke-point with `if_fingerprint` = the entry fingerprint. The trace is
//! the response body.
//!
//! Laws held here, each pinned by a test in this module or in
//! `tests/script_op.rs`:
//!
//! - **Entry world**: reads of hash-domain members serve the pinned entry
//!   state — foreign mid-program changes to the domain are invisible; the
//!   commit's §5.1 guard runs against the LIVE world unchanged, so a moved
//!   world refuses and nothing lands. An out-of-domain path stays addressable
//!   (§12.1: hash domain ⊂ addressable domain) and serves from a live
//!   single-file disk load, outside the pin exactly as the fingerprint never
//!   covered its bytes — the wire lane serves what the CLI lane serves.
//! - **Read-your-own-writes**: a read of a target the program itself armed
//!   serves the ARMED content and that content's own rev.
//! - **Entry-rev threading**: the license is the recording's, the value is
//!   the entry world's — an overlay rev is never a CAS token.
//! - **Containment**: the kernel's fuel/mem/depth/source ceilings and
//!   `catch_unwind` (both inside `effects::eval_script`); the wall clock
//!   binds at entry, at every read builtin, and pre-commit.
//! - **Not the banned snapshot**: the pin is attempt-scoped; nothing survives
//!   the answer (the engines map still holds ONE generation).

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use effects::trace::{CommitLeg, Refusal, ScriptTrace};
use effects::{
    ArmedEdit, ReadFault, ScriptCtx, ScriptHost, ScriptLimits, ScriptRecording, SecFacts, TocEntry,
    TocFacts, hpath_addresses,
};
use serde_json::value::RawValue;
use serde_json::{Map, Value, json};
use wire::{ErrorBody, ErrorCode, PlanEdit, ReadSel, Recovery, ResponseBody};

use crate::engine::WorkspaceEngine;
use crate::registry::Registry;
use wire_serve::rev::Rev;

/// Serve one `script` frame end to end, answering the full NDJSON line.
///
/// Routed from `handle_line` BEFORE the generic `serve_wire` path because the
/// success body is the run-plane `ScriptTrace`, not a [`ResponseBody`]
/// variant — the trace embeds the §4.4 splice response verbatim, and one
/// commit-fact shape means the wire types stay untouched. Error frames stay
/// typed and leave through the ordinary v2/v3 renderer.
pub(crate) fn serve_line(
    registry: &Registry,
    attached: Option<&Path>,
    obj: &Map<String, Value>,
    rev: Rev,
) -> String {
    let id = obj.get("id").and_then(Value::as_u64);
    let started = Instant::now();
    // Strict decode first (rev-agnostic), then the v3 gate — the read/create
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
    let wire::Op::Script {
        source,
        args,
        files,
        actor,
        now,
        receipt,
        if_root,
        dry,
        expect_armed,
        effects,
        invocation,
    } = op
    else {
        // decode() maps the "script" tag to Op::Script only; any other arm
        // here is a routing defect, answered loud rather than misdispatched.
        return error_line(id, ErrorBody::new(ErrorCode::Internal), rev);
    };
    let request = ScriptArgs {
        id,
        source,
        args,
        files,
        actor,
        now,
        receipt,
        if_root,
        dry: dry.unwrap_or(false),
        expect_armed,
        effects,
        invocation,
    };
    match serve(registry, ws, &request) {
        Ok(trace) => {
            let body = serde_json::to_value(&trace).expect("a ScriptTrace serializes");
            let mut frame = json!({"id": id, "ok": true, "body": body});
            let duration_us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
            wire_serve::rev::attach_meta(&mut frame, duration_us);
            let mut line = serde_json::to_string(&frame).expect("a script frame serializes");
            line.push('\n');
            line
        }
        Err(error) => error_line(id, *error, rev),
    }
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
struct ScriptArgs {
    id: Option<u64>,
    source: String,
    args: std::collections::BTreeMap<String, String>,
    files: Vec<String>,
    actor: Option<String>,
    now: Option<String>,
    receipt: Option<wire::ReceiptAddr>,
    if_root: Option<wire::Root>,
    dry: bool,
    expect_armed: Option<String>,
    /// Effects mode (script-effects ruling): non-empty switches to the LIVE
    /// program model. The decode wall owns the combination laws.
    effects: Vec<String>,
    /// The host-minted run-identity base, present exactly with `effects`.
    invocation: Option<String>,
}

/// The attempt: entry pass → pin → eval → thread → gate → commit → trace.
///
/// # Errors
/// A §8 frame ONLY for what never reached the entry: the entry pass itself
/// failing (env class), or the reaper winning the warm→pin race (retry).
/// Once an entry fingerprint exists, every exit is a trace.
fn serve(
    registry: &Registry,
    ws: &Path,
    request: &ScriptArgs,
) -> Result<ScriptTrace, Box<ErrorBody>> {
    // ONE currency pass, at entry (Law A-3c scope unchanged, moved in time).
    registry.warm_or_build(ws).map_err(|e| {
        let mut error = ErrorBody::new(ErrorCode::IoError);
        error.message = Some(format!("the entry pass could not prove the corpus: {e}"));
        Box::new(error)
    })?;
    // Pin the entry world. A reaper that won the warm→pin race is the same
    // transient the read path names (`corpus_race`, retry).
    let Some(world) = registry.engine_snapshot(ws) else {
        let mut error = ErrorBody::new(ErrorCode::CorpusRace);
        error.message = Some(
            "the warm engine was reaped between the entry pass and the pin — transient; retry"
                .to_owned(),
        );
        return Err(Box::new(error));
    };
    let entry = world.at_fingerprint.0.clone();

    // The caller's own pre-eval guard: zero evaluation, zero reads.
    if let Some(pinned) = &request.if_root
        && pinned.0 != entry
    {
        return Ok(ScriptTrace::guard_refused(entry, pinned.0.clone()));
    }

    // Effects mode: the LIVE program model (script-effects ruling, § A.7
    // effects paragraph). Forked here — after the entry fingerprint is known
    // (it rides the trace as the world-at-start fact) and before the pure
    // lane's pinned-world machinery, none of which this model uses.
    if !request.effects.is_empty() {
        return Ok(live_serve(registry, ws, request, &entry));
    }

    let deadline = Instant::now() + effects::DEFAULT_WALL_CLOCK;
    let ctx = ScriptCtx {
        id: "script".to_owned(),
        args: request.args.clone(),
        files: request.files.clone(),
        effects: Vec::new(),
    };
    let ws_root = fs::WorkspaceRoot(ws.to_path_buf());
    let mut eval = {
        let mut host = EntryWorldHost {
            world: Arc::clone(&world),
            root: wire::Root(entry.clone()),
            ws: fs::WorkspaceRoot(ws.to_path_buf()),
            deadline,
            actor: request.actor.clone().unwrap_or_default(),
            overlay: None,
        };
        effects::eval_script(&request.source, &ctx, ScriptLimits::default(), &mut host)
    };

    // A failed evaluation never commits; zero armed is the read-class exit.
    if eval.outcome.is_err() || eval.armed.is_empty() {
        return Ok(ScriptTrace::assemble(entry, &eval, CommitLeg::NotIssued));
    }

    // Entry-rev threading: the license is the recording's, the value is the
    // entry world's (run-plane § the entry-rev law).
    eval.armed = thread_entry(
        &eval.armed,
        &eval.recording,
        &world,
        &ws_root,
        &wire::Root(entry.clone()),
    );

    // The pre-splice armed-set gate — after threading, before anything is
    // issued; same wording as the CLI lane, one law two doors.
    if let Some(expected) = &request.expect_armed {
        let actual = effects::digest::armed_digest(&effects::digest::ArmedRow::of_all(&eval.armed));
        if *expected != actual {
            return Ok(ScriptTrace::assemble(
                entry,
                &eval,
                CommitLeg::Refused(Refusal::minted(
                    Recovery::Fix,
                    format!(
                        "expect_armed_mismatch: this run armed {actual}, the caller pinned \
                         {expected}. The armed set is not the one that was authorized, so NO \
                         splice was issued — nothing was sent, nothing landed, no fingerprint \
                         advanced. re-arm: run the arm leg again and gate the set it publishes"
                    ),
                )),
            ));
        }
    }

    // Wall site 3: pre-commit. Nothing issued on a lapsed clock.
    let leg = if Instant::now() > deadline {
        CommitLeg::Refused(Refusal::minted(
            Recovery::Retry,
            "the script entry's wall clock elapsed before the commit was issued — the armed \
             edits were never sent, nothing landed, and no fingerprint advanced. re-run: the \
             reads that ran cost the budget",
        ))
    } else {
        commit(registry, ws, request, &eval, &entry)
    };
    Ok(ScriptTrace::assemble(entry, &eval, leg))
}

/// Effects mode: the LIVE program (script-effects ruling, 2026-08-13;
/// `docs/run-plane.md` § Effects mode). `read()` serves the live disk NOW;
/// `put()` applies NOW through the wire splice door with the guard's own
/// `force` bypass — no rev, no snapshot, no CAS; `run()` executes at call
/// time through the § A.8 seam and its row is the program's value. No commit
/// leg, no armed set, no rollback; the trace records the acts in call order
/// and the outcome word is `effects`.
fn live_serve(registry: &Registry, ws: &Path, request: &ScriptArgs, entry: &str) -> ScriptTrace {
    let deadline = Instant::now() + effects::DEFAULT_WALL_CLOCK;
    let ctx = ScriptCtx {
        id: "script".to_owned(),
        args: request.args.clone(),
        files: request.files.clone(),
        effects: request.effects.clone(),
    };
    let (eval, acts) = {
        let mut host = LiveHost {
            registry,
            ws: fs::WorkspaceRoot(ws.to_path_buf()),
            ws_path: ws.to_path_buf(),
            root: wire::Root(entry.to_owned()),
            deadline,
            actor: request.actor.clone().unwrap_or_default(),
            now: request.now.clone(),
            // Decode wall: effects ⇒ invocation present.
            invocation: request.invocation.clone().unwrap_or_default(),
            run_seq: std::cell::Cell::new(0),
            reads_seen: std::cell::Cell::new(0),
            acts: std::cell::RefCell::new(Vec::new()),
        };
        let eval = effects::eval_script(&request.source, &ctx, ScriptLimits::default(), &mut host);
        let acts = host.acts.into_inner();
        (eval, acts)
    };
    // The base trace: reads in call order (the recording's), outcome by the
    // pure assembler — then this model's own words: interleave the live acts
    // at their recorded positions and rename a clean exit `effects`.
    let outcome_ok = eval.outcome.is_ok();
    let mut trace = ScriptTrace::assemble(entry.to_owned(), &eval, CommitLeg::NotIssued);
    let mut inserted = 0usize;
    for (after_reads, act) in acts {
        let mut index = 0usize;
        let mut reads_passed = 0usize;
        for (i, e) in trace.trace.iter().enumerate() {
            index = i + 1;
            if matches!(
                e,
                effects::trace::TraceEntry::Read(_) | effects::trace::TraceEntry::Echo(_)
            ) {
                reads_passed += 1;
            }
            if reads_passed >= after_reads {
                break;
            }
        }
        if after_reads == 0 {
            index = 0;
        }
        let at = (index + inserted).min(trace.trace.len());
        trace.trace.insert(at, act);
        inserted += 1;
    }
    if outcome_ok {
        trace.outcome = effects::trace::ScriptOutcome::Effects;
    }
    trace
}

/// The effects-mode host: live disk reads, immediate splices, call-time runs.
/// Every act is journaled with the read count at act time, so the trace can
/// interleave acts and reads in true call order.
struct LiveHost<'a> {
    registry: &'a Registry,
    ws: fs::WorkspaceRoot,
    ws_path: std::path::PathBuf,
    root: wire::Root,
    deadline: Instant,
    actor: String,
    now: Option<String>,
    invocation: String,
    run_seq: std::cell::Cell<u32>,
    reads_seen: std::cell::Cell<usize>,
    acts: std::cell::RefCell<Vec<(usize, effects::trace::TraceEntry)>>,
}

impl LiveHost<'_> {
    fn within_deadline(&self, what: &str) -> Result<(), effects::EffectFault> {
        if Instant::now() > self.deadline {
            return Err(effects::EffectFault {
                reason: format!(
                    "the script entry's wall clock elapsed before {what} — budgets bind \
                     unchanged in effects mode"
                ),
            });
        }
        Ok(())
    }

    fn load_live(&self, path: &str) -> Result<model::Document, String> {
        wire_serve::load_doc(&self.ws, &wire::Path(path.to_owned())).map_err(|e| error_text(&e))
    }
}

impl effects::ScriptHost for LiveHost<'_> {
    fn toc(&mut self, path: &str, _armed: &[ArmedEdit]) -> Result<TocFacts, ReadFault> {
        let fault = |reason: String| ReadFault {
            path: path.to_owned(),
            section: None,
            reason,
        };
        if Instant::now() > self.deadline {
            return Err(fault("the script entry's wall clock elapsed".to_owned()));
        }
        let doc = self.load_live(path).map_err(&fault)?;
        self.reads_seen.set(self.reads_seen.get() + 1);
        Ok(toc_facts_of(&doc, path, &self.root))
    }

    fn cat(
        &mut self,
        path: &str,
        section: &str,
        _armed: &[ArmedEdit],
    ) -> Result<SecFacts, ReadFault> {
        let fault = |reason: String| ReadFault {
            path: path.to_owned(),
            section: Some(section.to_owned()),
            reason,
        };
        if Instant::now() > self.deadline {
            return Err(fault("the script entry's wall clock elapsed".to_owned()));
        }
        let doc = self.load_live(path).map_err(&fault)?;
        let sec =
            wire_serve::read::selector_to_secref(&doc, &ReadSel::parse(section)).map_err(&fault)?;
        let facts = match wire_serve::read::cat(&doc, Some(sec)) {
            Ok(ResponseBody::Cat {
                content, node_rev, ..
            }) => SecFacts {
                text: content,
                rev: node_rev.0,
            },
            Ok(_) => unreachable!("wire_serve::read::cat answers a Cat body"),
            Err(error) => return Err(fault(error_text(&error))),
        };
        self.reads_seen.set(self.reads_seen.get() + 1);
        Ok(facts)
    }

    fn actor(&self) -> &str {
        &self.actor
    }

    fn put_live(
        &mut self,
        path: &str,
        items: Vec<PlanEdit>,
        line: u32,
    ) -> Result<(), effects::EffectFault> {
        self.within_deadline("a live put")?;
        let refuse = |reason: String| effects::EffectFault { reason };
        let args = wire_serve::write::SpliceArgs {
            id: None,
            path: wire::Path(path.to_owned()),
            origin: wire_serve::guard::Origin::Wire,
            actor: (!self.actor.is_empty()).then(|| self.actor.clone()),
            now: self.now.clone(),
            receipt: None,
            // The ruled model: no rev, no snapshot, no CAS — the guard's own
            // `force` bypass, under the same write flock as every splice.
            if_root: None,
            dry: false,
            force: true,
            edits: Vec::new(),
            plan_edits: items.clone(),
            pin: None,
        };
        let mints = self.registry.read_mints(&self.ws_path);
        let ring = self.registry.ring(&self.ws_path);
        let outcome = wire_serve::write::splice(&self.ws, Some(&*ring), &args, &[], Some(&mints))
            .map_err(|e| refuse(format!("put: {}", error_text(&e))))?;
        let fingerprint_after = outcome.committed.as_ref().map(|frame| {
            let after = frame.delta.root_after.0.clone();
            ring.advance(frame.clone());
            after
        });
        self.acts.borrow_mut().push((
            self.reads_seen.get(),
            effects::trace::TraceEntry::Wrote(effects::trace::WroteEntry {
                path: path.to_owned(),
                line,
                edits: items.len(),
                fingerprint_after,
            }),
        ));
        Ok(())
    }

    fn run_live(
        &mut self,
        page: &str,
        task: Option<&str>,
        args: Vec<String>,
        env: std::collections::BTreeMap<String, String>,
        dry: bool,
        line: u32,
    ) -> Result<serde_json::Value, effects::EffectFault> {
        self.within_deadline("a live run")?;
        let seq = self.run_seq.get();
        self.run_seq.set(seq + 1);
        let invocation = format!("{}-r{seq}", self.invocation);
        let target = wire::RunTarget {
            page: page.to_owned(),
            task: task.map(str::to_owned),
            args,
            env,
            dry: Some(dry),
        };
        let row = crate::run_op::row_for_target(
            &self.ws,
            &self.ws_path,
            &target,
            &invocation,
            (!self.actor.is_empty()).then_some(self.actor.as_str()),
            self.now.as_deref(),
        );
        self.acts.borrow_mut().push((
            self.reads_seen.get(),
            effects::trace::TraceEntry::Ran(effects::trace::RanEntry {
                page: page.to_owned(),
                line,
                row: row.clone(),
            }),
        ));
        Ok(row)
    }
}

/// The one guarded splice, issued daemon-side through the same choke-point
/// every wire splice takes — `Origin::Wire`, the ring advanced on a real
/// commit (this lane mints Deltas; the CLI put lane's row-12 gap does not
/// extend here).
fn commit(
    registry: &Registry,
    ws: &Path,
    request: &ScriptArgs,
    eval: &effects::ScriptEval,
    entry: &str,
) -> CommitLeg {
    let paths = eval.content_paths();
    let [path] = paths.as_slice() else {
        // Unreachable by construction — the arm-time law refuses first
        // (`multi_file_write_set`); it still SPEAKS, the CLI lane's own words.
        return CommitLeg::Refused(Refusal::minted(
            Recovery::Fix,
            format!(
                "the armed set writes {} content paths; one script commits to ONE file. NO \
                 splice was issued — nothing was sent, nothing landed, no fingerprint advanced. \
                 fix: arm one content path",
                paths.len()
            ),
        ));
    };
    let args = wire_serve::write::SpliceArgs {
        id: request.id,
        path: wire::Path(path.clone()),
        origin: wire_serve::guard::Origin::Wire,
        actor: request.actor.clone(),
        now: request.now.clone(),
        receipt: request.receipt.clone(),
        if_root: Some(wire::Root(entry.to_owned())),
        dry: request.dry,
        force: false,
        edits: Vec::new(),
        plan_edits: eval.armed.iter().map(|armed| armed.edit.clone()).collect(),
        pin: None,
    };
    // H1 order: the mint store and ring handles are taken outside any engine
    // borrow (none is held here — the entry world is an Arc, not a lock).
    let mints = registry.read_mints(ws);
    let ring = registry.ring(ws);
    let ws_root = fs::WorkspaceRoot(ws.to_path_buf());
    // The splice is a function call here, so a lost answer cannot happen —
    // except as a panic mid-splice, which is the same indeterminacy: caught,
    // spoken as `commit_unknown`, never an unwind through the connection
    // thread (`docs/run-plane.md` § A controlled failure exit SPEAKS).
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        wire_serve::write::splice(&ws_root, Some(&*ring), &args, &[], Some(&mints))
    }));
    let Ok(outcome) = caught else {
        return CommitLeg::Unknown(lost_commit(request.dry));
    };
    match outcome {
        Ok(out) => {
            if let Some(frame) = out.committed {
                ring.advance(frame);
            }
            let raw = v3_body_raw(&out.body);
            if request.dry {
                CommitLeg::Rehearsal(raw)
            } else {
                CommitLeg::Response(raw)
            }
        }
        Err(error) => {
            let raw = v3_error_raw(&error);
            if matches!(error.code, ErrorCode::RootMismatch) {
                CommitLeg::Conflict(raw)
            } else {
                CommitLeg::Refused(refusal_of(&raw))
            }
        }
    }
}

/// The engine-minted refusal for a commit whose outcome is NOT KNOWN — the
/// in-process analog of a lost answer (the splice panicked mid-flight). The
/// class splits on `dry` exactly as the CLI lane's `lost_answer` does.
fn lost_commit(dry: bool) -> Refusal {
    if dry {
        return Refusal::minted(
            Recovery::Retry,
            "the commit leg panicked mid-splice. This was a DRY run — it rehearses everything \
             except disk, so nothing could have been committed and the workspace is unchanged. \
             retry: re-run the same script, a rehearsal writes nothing",
        );
    }
    Refusal::minted(
        Recovery::Resync,
        "the commit leg panicked mid-splice. The splice was ISSUED, so whether the workspace \
         carries this run is UNKNOWN. resync: re-read and re-plan, never resend, because a \
         resend writes twice",
    )
}

/// The §4.4 splice response body, serialized in the v3 vocabulary — the SAME
/// bytes a v3 wire client would have received, so the two lanes' traces embed
/// one spelling. The projection is the one implementation (`rev.rs`), reached
/// through a throwaway frame.
fn v3_body_raw(body: &ResponseBody) -> Box<RawValue> {
    let mut frame = json!({
        "id": Value::Null,
        "ok": true,
        "body": serde_json::to_value(body).expect("a splice body serializes"),
    });
    wire_serve::rev::project_response(&mut frame);
    RawValue::from_string(frame["body"].to_string()).expect("projected body is JSON")
}

/// A §8 error body in the v3 vocabulary, as raw bytes for the conflict leg.
fn v3_error_raw(error: &ErrorBody) -> Box<RawValue> {
    let mut frame = json!({
        "id": Value::Null,
        "ok": false,
        "error": serde_json::to_value(error).expect("an error body serializes"),
    });
    wire_serve::rev::project_response(&mut frame);
    RawValue::from_string(frame["error"].to_string()).expect("projected error is JSON")
}

/// The wire's refusal triple off the projected error bytes — the same read
/// the CLI lane performs on the frame it receives, so the two lanes classify
/// one refusal one way.
fn refusal_of(error: &RawValue) -> Refusal {
    let parsed: Option<Value> = serde_json::from_str(error.get()).ok();
    let field = |name: &str| -> Option<String> {
        parsed
            .as_ref()
            .and_then(|e| e.get(name))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    let code = field("code");
    let recovery = field("recovery")
        .and_then(|class| serde_json::from_value::<Recovery>(Value::String(class)).ok())
        .or_else(|| {
            code.as_ref().and_then(|code| {
                serde_json::from_value::<ErrorCode>(Value::String(code.clone()))
                    .ok()
                    .map(ErrorCode::recovery)
            })
        });
    let reason = field("message").unwrap_or_else(|| error.get().to_owned());
    Refusal {
        code,
        recovery,
        reason,
    }
}

/// Thread each armed row's CAS token: LICENSE from the recording (any read of
/// the row's grain this attempt, overlay-served included — the
/// write-follows-read law binds per attempt), VALUE from the entry world (the
/// pre-batch state the §4.4 guards resolve against; an overlay rev names a
/// state no disk ever carried and is never a token). An unlicensed row keeps
/// `rev: None` and meets the engine's own `guard_required` — the same refusal
/// the CLI lane's unread target meets.
fn thread_entry(
    armed: &[ArmedEdit],
    recording: &ScriptRecording,
    world: &WorkspaceEngine,
    ws: &fs::WorkspaceRoot,
    root: &wire::Root,
) -> Vec<ArmedEdit> {
    armed
        .iter()
        .map(|arm| {
            let mut arm = arm.clone();
            match &mut arm.edit {
                PlanEdit::SetProperty {
                    rev: rev @ None, ..
                } => {
                    let licensed = recording
                        .reads
                        .iter()
                        .any(|r| r.path == arm.path && r.section.is_none());
                    if licensed {
                        *rev = entry_toc(world, ws, root, &arm.path).map(|facts| facts.rev);
                    }
                }
                PlanEdit::Append {
                    hpath,
                    rev: rev @ None,
                    ..
                } => {
                    let licensed = recording.reads.iter().any(|r| {
                        if r.path != arm.path {
                            return false;
                        }
                        match (&r.section, &r.face) {
                            (Some(recorded), effects::ReadFace::Section(_)) => {
                                hpath_addresses(recorded, hpath)
                            }
                            (None, effects::ReadFace::Toc(facts)) => facts
                                .toc
                                .iter()
                                .any(|entry| hpath_addresses(&entry.section, hpath)),
                            _ => false,
                        }
                    });
                    if licensed {
                        *rev = entry_toc(world, ws, root, &arm.path).and_then(|facts| {
                            facts
                                .toc
                                .iter()
                                .find(|entry| hpath_addresses(&entry.section, hpath))
                                .map(|entry| entry.rev.clone())
                        });
                    }
                }
                _ => {}
            }
            arm
        })
        .collect()
}

/// The §4.1 toc face of `path` at the ENTRY state — the one builder threading
/// and the host share, so a threaded token equals what an entry read served,
/// by construction. An out-of-domain target values from the live disk file
/// (§12.1: a guarded write's CAS token for an out-of-domain page is mintable
/// at the read door like any other) — the same single-file load `doc_for`
/// serves its reads from, and the same file the commit's own guards resolve
/// against.
fn entry_toc(
    world: &WorkspaceEngine,
    ws: &fs::WorkspaceRoot,
    root: &wire::Root,
    path: &str,
) -> Option<TocFacts> {
    if let Some(doc) = world.docs.get(path) {
        return Some(toc_facts_of(doc, path, root));
    }
    wire_serve::load_doc(ws, &wire::Path(path.to_owned()))
        .ok()
        .map(|doc| toc_facts_of(&doc, path, root))
}

/// Build the script-face [`TocFacts`] from one document — the daemon-side twin
/// of the wire client's composition, built from the same serve arms
/// (`wire_serve::read::{toc, read_props}` equivalents) so the face bytes agree
/// across lanes: rev = the served `file_rev`, `fm` decoded per § A.6, toc rows
/// as `/`-joined hpaths (what `ReadSel::parse` splits again) or `^anchor`,
/// words = the composed read's own count.
fn toc_facts_of(doc: &model::Document, path: &str, root: &wire::Root) -> TocFacts {
    let body = wire_serve::read::toc(doc, &wire::Path(path.to_owned()), root);
    let ResponseBody::Toc {
        file_rev, nodes, ..
    } = body
    else {
        unreachable!("wire_serve::read::toc answers a Toc body");
    };
    let toc = nodes
        .iter()
        .filter_map(|node| {
            let anchor = node.anchor.as_ref().map(|id| format!("^{id}"));
            let section = match &node.hpath {
                Some(hpath) => hpath
                    .iter()
                    .map(|seg| seg.h.as_str())
                    .collect::<Vec<_>>()
                    .join("/"),
                None => anchor.clone()?,
            };
            Some(TocEntry {
                section,
                anchor,
                rev: node.node_rev.0.clone(),
            })
        })
        .collect();
    let fm = wire_serve::read::props_of(doc)
        .into_iter()
        .map(|prop| (prop.key, prop.value))
        .collect();
    TocFacts {
        rev: file_rev.0,
        fm,
        toc,
        words: wire_serve::read::words_of(doc),
    }
}

/// The entry world plus the program's own armed overlay — the § A.7 read
/// seam. Domain members serve at memory speed: no locks, no passes, no disk.
/// An out-of-domain path serves from a live single-file disk load (§12.1),
/// the one read that leaves memory.
struct EntryWorldHost {
    world: Arc<WorkspaceEngine>,
    root: wire::Root,
    /// The workspace directory — the §12.1 fallback loads from it.
    ws: fs::WorkspaceRoot,
    deadline: Instant,
    actor: String,
    /// The overlay document for the ONE armed content path, cached by armed
    /// count (the armed list is append-only and single-path by the arm law).
    overlay: Option<(String, usize, model::Document)>,
}

impl EntryWorldHost {
    /// Wall site 2: every read builtin checks the clock before serving.
    fn within_deadline(&self, path: &str, section: Option<&str>) -> Result<(), ReadFault> {
        if Instant::now() > self.deadline {
            return Err(ReadFault {
                path: path.to_owned(),
                section: section.map(ToOwned::to_owned),
                reason: "the script entry's wall clock elapsed — the attempt is over budget; \
                         nothing was committed"
                    .to_owned(),
            });
        }
        Ok(())
    }

    /// Resolve the ENTRY document for `path`: the pinned corpus first, the
    /// unserved condition as a fault, then the §12.1 addressability fallback —
    /// a real file under the root but outside the hash domain serves from a
    /// single-file LIVE disk load (the same `load_doc` the daemon's read doors
    /// and the write path run, so all lanes agree on what a path serves).
    /// Out-of-domain reads sit outside the pin and outside the stand-still
    /// guarantee exactly as the fingerprint never covered them.
    fn entry_doc(
        &self,
        path: &str,
        fault: &dyn Fn(String) -> ReadFault,
    ) -> Result<std::borrow::Cow<'_, model::Document>, ReadFault> {
        if let Some(doc) = self.world.docs.get(path) {
            return Ok(std::borrow::Cow::Borrowed(doc));
        }
        if let Some(condition) = self.world.unserved.get(path) {
            return Err(fault(format!(
                "the corpus cannot serve this member: {condition}"
            )));
        }
        wire_serve::load_doc(&self.ws, &wire::Path(path.to_owned()))
            .map(std::borrow::Cow::Owned)
            .map_err(|e| fault(error_text(&e)))
    }

    /// The document a read of `path` serves: the entry doc (or its §12.1
    /// disk-load fallback), or — when the program itself armed edits on
    /// `path` — that base with those edits applied, in arm order
    /// (read-your-own-writes). What you read is what is hashed: the overlay
    /// document's revs are minted from the overlay bytes.
    fn doc_for(
        &mut self,
        path: &str,
        section: Option<&str>,
        armed: &[ArmedEdit],
    ) -> Result<std::borrow::Cow<'_, model::Document>, ReadFault> {
        let fault = |reason: String| ReadFault {
            path: path.to_owned(),
            section: section.map(ToOwned::to_owned),
            reason,
        };
        let rows: Vec<&ArmedEdit> = armed.iter().filter(|a| a.path == path).collect();
        if rows.is_empty() {
            return self.entry_doc(path, &fault);
        }
        let cached = self
            .overlay
            .as_ref()
            .is_some_and(|(p, count, _)| p == path && *count == rows.len());
        if !cached {
            let base = self.entry_doc(path, &fault)?;
            let doc = overlay_doc(&base, path, &rows).map_err(fault)?;
            self.overlay = Some((path.to_owned(), rows.len(), doc));
        }
        Ok(std::borrow::Cow::Borrowed(
            &self.overlay.as_ref().expect("just cached").2,
        ))
    }
}

/// Render a §8 error body the way this lane's read faults read: the message
/// when the frame carries one (load refusals open with their own code), else
/// the code token with the cause when one exists.
fn error_text(error: &ErrorBody) -> String {
    if let Some(message) = &error.message {
        return message.clone();
    }
    let code = serde_json::to_value(error.code)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("{:?}", error.code));
    match &error.cause {
        Some(cause) => format!("{code}: {cause}"),
        None => code,
    }
}

/// Apply the program's own armed rows to the entry document — the dry
/// pipeline in memory: lower → validate → candidate, one reparse, no disk.
/// A row set that cannot apply to the entry state has no overlay content to
/// serve, so the read refuses naming the first refusing law (the commit
/// would meet the same refusal from the engine's own mouth).
fn overlay_doc(
    base: &model::Document,
    path: &str,
    rows: &[&ArmedEdit],
) -> Result<model::Document, String> {
    let plan: Vec<PlanEdit> = rows.iter().map(|row| row.edit.clone()).collect();
    wire_serve::write::overlay_candidate(base, &wire::Path(path.to_owned()), &plan).map_err(
        |error| {
            format!(
                "the armed edits cannot apply to the entry state — the commit would meet                  this same refusal: {}",
                error
                    .message
                    .clone()
                    .unwrap_or_else(|| format!("{:?}", error.code))
            )
        },
    )
}

impl ScriptHost for EntryWorldHost {
    fn toc(&mut self, path: &str, armed: &[ArmedEdit]) -> Result<TocFacts, ReadFault> {
        self.within_deadline(path, None)?;
        let root = self.root.clone();
        let doc = self.doc_for(path, None, armed)?;
        Ok(toc_facts_of(&doc, path, &root))
    }

    fn cat(
        &mut self,
        path: &str,
        section: &str,
        armed: &[ArmedEdit],
    ) -> Result<SecFacts, ReadFault> {
        self.within_deadline(path, Some(section))?;
        let fault = |reason: String| ReadFault {
            path: path.to_owned(),
            section: Some(section.to_owned()),
            reason,
        };
        // The one human-string→selector door, then the shared resolver — the
        // dewey lane is served here since the read-alignment ruling
        // (2026-08-13): one `selector_matches` resolution, every door.
        let doc = self.doc_for(path, Some(section), armed)?;
        let sec = wire_serve::read::selector_to_secref(&doc, &ReadSel::parse(section))
            .map_err(|reason| fault(reason))?;
        match wire_serve::read::cat(&doc, Some(sec)) {
            Ok(ResponseBody::Cat {
                content, node_rev, ..
            }) => Ok(SecFacts {
                text: content,
                rev: node_rev.0,
            }),
            Ok(_) => unreachable!("wire_serve::read::cat answers a Cat body"),
            Err(error) => Err(fault(
                error
                    .message
                    .clone()
                    .unwrap_or_else(|| format!("{:?}", error.code)),
            )),
        }
    }

    fn actor(&self) -> &str {
        &self.actor
    }
}

#[cfg(test)]
mod tests {
    //! The module-grain pins the wire harness cannot reach deterministically:
    //! mid-program foreign-edit invisibility (no interleave point exists over
    //! one socket), entry-rev-vs-overlay-rev threading, and the read-site
    //! wall clock. The wire-grain laws live in `tests/script_op.rs`.

    use super::*;
    use crate::state::StateStore;
    use effects::{ReadFace, ReadPosition, ReadRecord, ScriptRecording};
    use std::fs::{create_dir_all, write};
    use std::path::PathBuf;
    use std::time::Duration;

    const DOC: &str = "---\nstatus: open\n---\n# Alpha\n\none two three\n";

    fn registry_in(home: &Path) -> Registry {
        let cache_root = home.join("cache");
        create_dir_all(&cache_root).unwrap();
        Registry::new(
            StateStore::new(home.join("state.json")),
            cache_root,
            Vec::new(),
        )
    }

    fn seeded_ws(home: &Path) -> PathBuf {
        let ws = home.join("ws");
        create_dir_all(&ws).unwrap();
        write(ws.join("doc.md"), DOC).unwrap();
        std::fs::canonicalize(&ws).unwrap()
    }

    fn pinned_world(registry: &Registry, ws: &Path) -> (Arc<WorkspaceEngine>, wire::Root) {
        registry.warm_or_build(ws).expect("entry pass");
        let world = registry.engine_snapshot(ws).expect("pinned world");
        let root = wire::Root(world.at_fingerprint.0.clone());
        (world, root)
    }

    fn host_of(
        world: &Arc<WorkspaceEngine>,
        root: &wire::Root,
        ws: &Path,
        deadline: Instant,
    ) -> EntryWorldHost {
        EntryWorldHost {
            world: Arc::clone(world),
            root: root.clone(),
            ws: fs::WorkspaceRoot(ws.to_path_buf()),
            deadline,
            actor: String::new(),
            overlay: None,
        }
    }

    /// The entry-world law, both halves: a foreign disk edit AFTER entry is
    /// invisible to the program's reads (they serve the pinned entry state),
    /// and the commit's §5.1 guard runs against the LIVE world — the same
    /// foreign edit refuses the splice, so nothing lands on a moved world.
    #[test]
    fn a_foreign_edit_after_entry_is_invisible_to_reads_and_refuses_the_commit() {
        let tmp = tempfile::tempdir().unwrap();
        let registry = registry_in(tmp.path());
        let ws = seeded_ws(tmp.path());
        let (world, root) = pinned_world(&registry, &ws);

        // The foreign edit lands on disk between entry and the first read.
        write(ws.join("doc.md"), DOC.replace("open", "parked")).unwrap();

        // Read half: the entry world serves the ENTRY bytes.
        let mut host = host_of(&world, &root, &ws, Instant::now() + Duration::from_secs(7));
        let face = host.toc("doc.md", &[]).expect("entry world serves");
        assert_eq!(
            face.fm.get("status").map(String::as_str),
            Some("open"),
            "a foreign mid-program change is invisible: reads span ONE state"
        );

        // Commit half: the §5.1 guard folds the LIVE world under the flock.
        let args = wire_serve::write::SpliceArgs {
            id: None,
            path: wire::Path("doc.md".to_owned()),
            origin: wire_serve::guard::Origin::Wire,
            actor: None,
            now: None,
            receipt: None,
            if_root: Some(root.clone()),
            dry: false,
            force: false,
            edits: Vec::new(),
            plan_edits: vec![PlanEdit::SetProperty {
                key: "status".to_owned(),
                value: "done".to_owned(),
                rev: Some(face.rev.clone()),
            }],
            pin: None,
        };
        let ws_root = fs::WorkspaceRoot(ws.clone());
        let refused = wire_serve::write::splice(&ws_root, None, &args, &[], None)
            .expect_err("a moved world refuses the commit");
        assert!(
            matches!(refused.code, ErrorCode::RootMismatch),
            "the §5.1 world guard, checked first: {refused:?}"
        );
        assert_eq!(
            std::fs::read_to_string(ws.join("doc.md")).unwrap(),
            DOC.replace("open", "parked"),
            "nothing landed — the foreign state stands untouched"
        );
    }

    /// The entry-rev law: the license is the recording's, the value is the
    /// entry world's. A recording whose only read of the target served the
    /// OVERLAY (its rev names bytes no disk carried) still licenses the row —
    /// and the threaded token is the ENTRY rev, never the overlay rev. An
    /// empty recording licenses nothing.
    #[test]
    fn threading_licenses_from_the_recording_and_values_from_the_entry_world() {
        let tmp = tempfile::tempdir().unwrap();
        let registry = registry_in(tmp.path());
        let ws = seeded_ws(tmp.path());
        let (world, root) = pinned_world(&registry, &ws);
        let entry_rev = entry_toc(&world, &fs::WorkspaceRoot(ws.clone()), &root, "doc.md")
            .expect("doc in world")
            .rev;

        let armed = vec![ArmedEdit {
            path: "doc.md".to_owned(),
            edit: PlanEdit::SetProperty {
                key: "status".to_owned(),
                value: "done".to_owned(),
                rev: None,
            },
            line: 2,
            depth: 0,
        }];
        let overlay_read = ScriptRecording {
            actor: String::new(),
            reads: vec![ReadRecord {
                path: "doc.md".to_owned(),
                section: None,
                line: 3,
                position: ReadPosition::Echo,
                face: ReadFace::Toc(TocFacts {
                    rev: "feedfacefeedface".to_owned(), // an overlay rev: no disk state carries it
                    fm: std::collections::BTreeMap::new(),
                    toc: Vec::new(),
                    words: 0,
                }),
            }],
        };

        let threaded = thread_entry(
            &armed,
            &overlay_read,
            &world,
            &fs::WorkspaceRoot(ws.clone()),
            &root,
        );
        let PlanEdit::SetProperty { rev, .. } = &threaded[0].edit else {
            panic!("shape preserved");
        };
        assert_eq!(
            rev.as_deref(),
            Some(entry_rev.as_str()),
            "the token is the ENTRY rev — the pre-batch state the §4.4 guards \
             resolve against — never the overlay rev the read served"
        );

        let unlicensed = thread_entry(
            &armed,
            &ScriptRecording {
                actor: String::new(),
                reads: Vec::new(),
            },
            &world,
            &fs::WorkspaceRoot(ws.clone()),
            &root,
        );
        let PlanEdit::SetProperty { rev, .. } = &unlicensed[0].edit else {
            panic!("shape preserved");
        };
        assert!(
            rev.is_none(),
            "a row whose target the attempt never read threads nothing and \
             meets the engine's own guard_required"
        );
    }

    /// Wall site 2: the read builtin refuses on a lapsed clock — typed, in
    /// the entry's own vocabulary, before any serve.
    #[test]
    fn the_wall_clock_binds_at_the_read_builtin() {
        let tmp = tempfile::tempdir().unwrap();
        let registry = registry_in(tmp.path());
        let ws = seeded_ws(tmp.path());
        let (world, root) = pinned_world(&registry, &ws);

        let lapsed = Instant::now()
            .checked_sub(Duration::from_millis(1))
            .expect("the clock is past its first millisecond");
        let mut host = host_of(&world, &root, &ws, lapsed);
        let fault = host.toc("doc.md", &[]).expect_err("lapsed clock refuses");
        assert!(
            fault.reason.contains("wall clock elapsed"),
            "the refusal names the budget: {}",
            fault.reason
        );
    }

    /// The commit leg's panic wall (review B1): a panicked splice is the
    /// in-process analog of a lost answer — the leg is [`CommitLeg::Unknown`],
    /// and the assembled trace SPEAKS: `commit_unknown: true` beside a
    /// refusal-shaped fault whose class splits on `dry` (resync live — the
    /// splice was ISSUED, so never resend; retry under dry — a rehearsal
    /// writes nothing). Never a plain refusal claiming nothing was applied:
    /// that is the one fabrication the marker exists to prevent. The panic
    /// itself has no honest trigger through the public op (the splice is
    /// refusal-shaped on every input), so the gate pins the leg the
    /// `catch_unwind` in `commit` feeds and the trace it assembles to.
    #[test]
    fn a_panicked_splice_speaks_commit_unknown_never_a_plain_refusal() {
        let eval = effects::ScriptEval {
            outcome: Ok(effects::ScriptFacts {
                bindings: std::collections::BTreeMap::new(),
            }),
            armed: vec![ArmedEdit {
                path: "doc.md".to_owned(),
                edit: PlanEdit::SetProperty {
                    key: "status".to_owned(),
                    value: "done".to_owned(),
                    rev: None,
                },
                line: 2,
                depth: 0,
            }],
            recording: ScriptRecording {
                actor: String::new(),
                reads: Vec::new(),
            },
            telemetry: effects::ScriptTelemetry {
                fuel_used: 0,
                mem_used: 0,
                reads_used: 0,
                wall_ms: 0,
            },
        };

        let live = ScriptTrace::assemble("entry-fp", &eval, CommitLeg::Unknown(lost_commit(false)));
        assert!(live.commit_unknown, "the marker is the whole point");
        assert_eq!(live.outcome, effects::trace::ScriptOutcome::Refused);
        assert!(live.commit.is_none(), "no answer exists to embed");
        let json = serde_json::to_value(&live).expect("a trace serializes");
        assert_eq!(
            json["commit_unknown"],
            Value::Bool(true),
            "the marker crosses the wire, never elided as a default"
        );
        let fault = live.fault.expect("refusal-shaped fault");
        assert_eq!(
            fault.recovery,
            Some(Recovery::Resync),
            "issued ⇒ resync, never resend"
        );
        assert!(
            fault.reason.contains("UNKNOWN"),
            "the reason states the indeterminacy: {}",
            fault.reason
        );

        let dry = ScriptTrace::assemble("entry-fp", &eval, CommitLeg::Unknown(lost_commit(true)));
        assert!(dry.commit_unknown);
        assert_eq!(
            dry.fault.expect("refusal-shaped fault").recovery,
            Some(Recovery::Retry),
            "a rehearsal writes nothing, so re-running is safe"
        );
    }

    /// The §12.1 seam of the entry-world law: an out-of-domain path (here a
    /// dot-directory page the default ignore excludes) is ADDRESSABLE and
    /// LIVE — it serves from a single-file disk load on every read, so a
    /// foreign mid-program change to it IS visible, exactly as the entry
    /// fingerprint never covered its bytes. The wire lane serves what the
    /// CLI lane serves (ruled 2026-08-12: "mrd mcp should be same as cli").
    #[test]
    fn an_out_of_domain_path_serves_live_from_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let registry = registry_in(tmp.path());
        let ws = seeded_ws(tmp.path());
        create_dir_all(ws.join(".obsidian")).unwrap();
        write(
            ws.join(".obsidian/hidden.md"),
            "---\nstatus: open\n---\n# Hidden\n",
        )
        .unwrap();
        let (world, root) = pinned_world(&registry, &ws);
        assert!(
            !world.docs.contains_key(".obsidian/hidden.md"),
            "premise: the dot-directory page is outside the hash domain"
        );

        let mut host = host_of(&world, &root, &ws, Instant::now() + Duration::from_secs(7));
        let face = host
            .toc(".obsidian/hidden.md", &[])
            .expect("§12.1: addressable by explicit path");
        assert_eq!(face.fm.get("status").map(String::as_str), Some("open"));

        // The stand-still guarantee does not extend outside the domain: a
        // foreign edit BETWEEN reads is visible, because each read is a live
        // single-file load the pin never covered.
        write(
            ws.join(".obsidian/hidden.md"),
            "---\nstatus: parked\n---\n# Hidden\n",
        )
        .unwrap();
        let moved = host.toc(".obsidian/hidden.md", &[]).expect("still serves");
        assert_eq!(
            moved.fm.get("status").map(String::as_str),
            Some("parked"),
            "out-of-domain reads are LIVE — the pin never covered this file"
        );
    }
}
