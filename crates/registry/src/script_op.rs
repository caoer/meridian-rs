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
//! choke-point under the §4.6 TOUCH-SET premises (run-plane amendment,
//! 2026-08-15): entry-vs-live at exactly the nodes the attempt touched,
//! never the whole-corpus entry fingerprint. The trace is the response body.
//!
//! Laws held here, each pinned by a test in this module or in
//! `tests/script_op.rs`:
//!
//! - **Entry world**: reads of hash-domain members serve the pinned entry
//!   state — foreign mid-program changes to the domain are invisible; the
//!   commit verifies entry-vs-live at the TOUCH SET (§4.6), so a moved
//!   touched node refuses and nothing lands, while foreign motion outside
//!   the touch set never refuses. An out-of-domain path stays addressable
//!   (§12.1: hash domain ⊂ addressable domain) and serves from a live
//!   single-file disk load, outside the pin exactly as the fingerprint never
//!   covered its bytes — the wire lane serves what the CLI lane serves.
//! - **Read-your-own-writes**: a read of a target the program itself armed
//!   serves the ARMED content and that content's own rev.
//! - **Entry-rev threading**: license-free since the CAS relaxation (ruling
//!   2026-08-13) — every rev-less row values from the entry world; an overlay
//!   rev is never a CAS token. Consistency lives at the commit's §5.1 world
//!   guard, not in a read ritual.
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
    ArmedEdit, ReadFault, ScriptCtx, ScriptHost, ScriptLimits, SecFacts, TocEntry, TocFacts,
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
        token_count_endpoint,
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
        token_count_endpoint,
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
    /// The harness measuring endpoint for the `token_count` effect (leg B):
    /// present exactly when the effect is declared AND the consumer bound
    /// one. The decode wall refuses it without the effect; absent with the
    /// effect declared is legal — the builtin then refuses "unbound".
    token_count_endpoint: Option<String>,
}

/// The § A.7 literals-first refusal for an illegal `files[]` list, or `None`
/// when the members are legal.
///
/// A pattern expands IN PLACE, so every member standing after it binds at an
/// index that moves with the day's match count: measured, a zero-match pattern
/// rebound the literal the caller addressed as `files[1]` to `files[0]` and
/// armed mode applied the retargeted write. Dry and armed refuse alike — arm
/// and commit are two calls, and a dry-only warning cannot hold the door the
/// armed call walks through.
fn member_order_refusal(files: &[String]) -> Option<Refusal> {
    let (pattern, literal) = policy::first_member_order_fault(files)?;
    let literals = files.iter().filter(|m| !policy::is_glob_pattern(m)).count();
    Some(Refusal::minted(
        Recovery::Fix,
        format!(
            "files_member_order: files[{pattern}] is the pattern \"{}\" and files[{literal}] is \
             the literal path \"{}\" standing after it. A pattern expands in place, so that \
             literal binds at whatever index the day's match count leaves it — on a zero-match \
             day it becomes files[{pattern}], and a fixed-index write lands on a document you \
             did not name. Nothing was evaluated, nothing was armed, and the workspace is \
             unchanged. re-issue with every literal member BEFORE every pattern member — the \
             literals then bind at files[0..{literals}] in call order, whatever the patterns \
             match",
            files[pattern], files[literal],
        ),
    ))
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

    // § A.7 literals-first, checked before expansion so the illegal list is
    // never bound.
    if let Some(refusal) = member_order_refusal(&request.files) {
        return Ok(ScriptTrace::entry_refused(entry, refusal));
    }

    // § A.7 pattern expansion, at entry, against the entry world's hash-domain
    // membership — the same walk that pinned the entry fingerprint, so the
    // expansion is deterministic within the attempt. Matching is the ONE scope
    // glob grammar (`policy::expand_globs`); the rows ride the recording so
    // the trace shows them and replay replays them. Zero matches contributes
    // zero paths — data, not a refusal.
    let (files, expansions) = if request.files.iter().any(|m| policy::is_glob_pattern(m)) {
        let corpus: Vec<String> = world
            .docs
            .keys()
            .chain(world.unserved.keys())
            .cloned()
            .collect();
        let (expanded, rows) = policy::expand_globs(&request.files, &corpus);
        let records = rows
            .into_iter()
            .map(|(pattern, matched)| effects::ExpansionRecord { pattern, matched })
            .collect();
        (expanded, records)
    } else {
        (request.files.clone(), Vec::new())
    };

    // Effects mode: the LIVE program model (script-effects ruling, § A.7
    // effects paragraph). Forked here — after the entry fingerprint is known
    // (it rides the trace as the world-at-start fact) and before the pure
    // lane's pinned-world machinery, none of which this model uses.
    if !request.effects.is_empty() {
        return Ok(live_serve(registry, ws, request, &entry, files, expansions));
    }

    let deadline = Instant::now() + effects::DEFAULT_WALL_CLOCK;
    let ctx = ScriptCtx {
        id: "script".to_owned(),
        args: request.args.clone(),
        files,
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
    // The expansion rows are entry facts: every exit below carries them.
    eval.recording.expansions = expansions;

    // A failed evaluation never commits; zero armed is the read-class exit.
    if eval.outcome.is_err() || eval.armed.is_empty() {
        return Ok(ScriptTrace::assemble(entry, &eval, CommitLeg::NotIssued));
    }

    // Entry-rev threading, license-free: the value is the entry world's
    // (run-plane § the entry-rev law, as amended by the CAS relaxation).
    eval.armed = thread_entry(&eval.armed, &world, &ws_root, &wire::Root(entry.clone()));

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
        commit(registry, ws, request, &eval, &world, &entry)
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
fn live_serve(
    registry: &Registry,
    ws: &Path,
    request: &ScriptArgs,
    entry: &str,
    files: Vec<String>,
    expansions: Vec<effects::ExpansionRecord>,
) -> ScriptTrace {
    let deadline = Instant::now() + effects::DEFAULT_WALL_CLOCK;
    let ctx = ScriptCtx {
        id: "script".to_owned(),
        args: request.args.clone(),
        files,
        effects: request.effects.clone(),
    };
    let (eval, acts) = {
        let mut host = LiveHost {
            registry,
            ws: fs::WorkspaceRoot(ws.to_path_buf()),
            ws_path: ws.to_path_buf(),
            root: wire::Root(entry.to_owned()),
            deadline: std::cell::Cell::new(deadline),
            actor: request.actor.clone().unwrap_or_default(),
            now: request.now.clone(),
            // Decode wall: effects ⇒ invocation present.
            invocation: request.invocation.clone().unwrap_or_default(),
            token_count_endpoint: request.token_count_endpoint.clone(),
            run_seq: std::cell::Cell::new(0),
            reads_seen: std::cell::Cell::new(0),
            acts: std::cell::RefCell::new(Vec::new()),
        };
        let mut eval =
            effects::eval_script(&request.source, &ctx, ScriptLimits::default(), &mut host);
        // Entry facts ride the live lane's trace too.
        eval.recording.expansions = expansions;
        let acts = host.acts.into_inner();
        (eval, acts)
    };
    // The base trace: reads in call order (the recording's), outcome by the
    // pure assembler — then this model's own words: interleave the live acts
    // at their recorded positions and rename a clean exit `effects`.
    let outcome_ok = eval.outcome.is_ok();
    let mut trace = ScriptTrace::assemble(entry.to_owned(), &eval, CommitLeg::NotIssued);
    for (inserted, (after_reads, act)) in acts.into_iter().enumerate() {
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
            // An act before the first read still happened after the entry
            // facts — never ahead of the expansion/bound rows that open the
            // trace (order-bind ruling: the binding is the header).
            index = trace
                .trace
                .iter()
                .take_while(|e| {
                    matches!(
                        e,
                        effects::trace::TraceEntry::Expanded(_)
                            | effects::trace::TraceEntry::Bound { .. }
                    )
                })
                .count();
        }
        let at = (index + inserted).min(trace.trace.len());
        trace.trace.insert(at, act);
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
    /// The script clock's deadline. A cell because a live `run()` pushes it
    /// forward by the run's own elapsed — the run plane's walks and child are
    /// metered on the run plane's own budget (`run.timeout_secs`), never the
    /// caller's script clock (dogfood r2 D-USER F8).
    deadline: std::cell::Cell<Instant>,
    actor: String,
    now: Option<String>,
    invocation: String,
    /// The harness measuring endpoint (leg B) — the consumer daemon's own
    /// socket, dialed per `token_count()` call. None refuses "unbound".
    token_count_endpoint: Option<String>,
    run_seq: std::cell::Cell<u32>,
    reads_seen: std::cell::Cell<usize>,
    acts: std::cell::RefCell<Vec<(usize, effects::trace::TraceEntry)>>,
}

impl LiveHost<'_> {
    fn within_deadline(&self, what: &str) -> Result<(), effects::EffectFault> {
        if Instant::now() > self.deadline.get() {
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

impl ScriptHost for LiveHost<'_> {
    fn toc(&mut self, path: &str, _armed: &[ArmedEdit]) -> Result<TocFacts, ReadFault> {
        let fault = |reason: String| ReadFault {
            path: path.to_owned(),
            section: None,
            reason,
        };
        if Instant::now() > self.deadline.get() {
            return Err(fault("the script entry's wall clock elapsed".to_owned()));
        }
        let doc = self.load_live(path).map_err(&fault)?;
        self.reads_seen.set(self.reads_seen.get() + 1);
        Ok(toc_facts_of(&doc, path, &self.root))
    }

    fn cat(
        &mut self,
        path: &str,
        section: &ReadSel,
        _armed: &[ArmedEdit],
    ) -> Result<SecFacts, ReadFault> {
        let fault = |reason: String| ReadFault {
            path: path.to_owned(),
            section: Some(section.display()),
            reason,
        };
        if Instant::now() > self.deadline.get() {
            return Err(fault("the script entry's wall clock elapsed".to_owned()));
        }
        let doc = self.load_live(path).map_err(&fault)?;
        let sec = wire_serve::read::selector_to_secref(&doc, section).map_err(&fault)?;
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
            premises: Vec::new(),
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
        let cache = self.registry.domain_cache(&self.ws_path);
        let outcome = wire_serve::write::splice_with_mints(
            &self.ws,
            Some(&*ring),
            &args,
            &[],
            wire_serve::write::Mints {
                ambient: Some(&mints),
                foreign: None,
            },
            Some(&cache),
        )
        .map_err(|e| refuse(format!("put: {}", error_text(&e))))?;
        // The sink recorded the frame inside the flock (`SeqSink::committed`);
        // the outcome's frame is data here, never re-advanced.
        let fingerprint_after = outcome
            .committed
            .as_ref()
            .map(|frame| frame.delta.root_after.0.clone());
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
    ) -> Result<Value, effects::EffectFault> {
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
        // Delta honesty (§ A.7 effects paragraph): a live run() mints per
        // committed batch through the same sink seam as the § A.8 op arm.
        let sink = crate::delta_sink::RingSink::new(self.registry.ring(&self.ws_path));
        // Observation unification (§ A.7 shares the § A.8 seam): the in-script
        // run() serves its bracket observations from the same resident memo.
        let cache = self.registry.domain_cache(&self.ws_path);
        let host = crate::run_op::RunHost {
            sink: &sink,
            cache: &cache,
        };
        // The clock stops while the run plane executes: admission was checked
        // above; the dispatch below — its walks, its child — is bounded by the
        // run plane's own `run.timeout_secs`, and its elapsed pushes the
        // script deadline forward so it never costs the caller's clock. The
        // run COUNT stays bounded by the kernel's run ceiling.
        let started = Instant::now();
        let row = crate::run_op::row_for_target(
            &self.ws,
            &self.ws_path,
            &target,
            &invocation,
            (!self.actor.is_empty()).then_some(self.actor.as_str()),
            self.now.as_deref(),
            &host,
        );
        self.deadline.set(self.deadline.get() + started.elapsed());
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

    fn token_count_live(&mut self, text: &str) -> Result<i64, effects::EffectFault> {
        self.within_deadline("a live token_count")?;
        let refuse = |reason: String| effects::EffectFault { reason };
        let Some(endpoint) = &self.token_count_endpoint else {
            return Err(refuse(
                "token_count is unbound — this frame carried no `token_count_endpoint`, and \
                 the engine never counts tokens itself (the count is a harness API call)"
                    .to_owned(),
            ));
        };
        // The dial deadline caps at the REMAINING wall clock: the harness
        // verb may park up to its own waiter bound, and this call never
        // outlives the entry's budget.
        let remaining = self
            .deadline
            .get()
            .saturating_duration_since(Instant::now());
        token_count_dial(endpoint, text, remaining)
            .map_err(|reason| refuse(format!("token_count: {reason}")))
    }
}

/// One measurement over the harness endpoint's NDJSON wire: write the
/// consumer daemon's `token_count` verb frame — identityless, so the daemon
/// applies its optional-session default and picks the measuring instrument —
/// read the one `response` line, and answer `data.tokens`. Every failure is
/// a reason string for the effect fault; a lapsed read deadline names the
/// wall clock, because that is what bound it.
fn token_count_dial(
    endpoint: &str,
    text: &str,
    remaining: std::time::Duration,
) -> Result<i64, String> {
    use std::io::{BufRead, BufReader, Write};
    if remaining.is_zero() {
        return Err(
            "the script entry's wall clock elapsed before the measurement was sent".to_owned(),
        );
    }
    let stream = std::os::unix::net::UnixStream::connect(endpoint).map_err(|e| {
        format!("the measuring endpoint at {endpoint} did not answer the dial: {e}")
    })?;
    stream
        .set_read_timeout(Some(remaining))
        .and_then(|()| stream.set_write_timeout(Some(remaining)))
        .map_err(|e| format!("could not bound the endpoint call: {e}"))?;
    let mut frame = serde_json::json!({"type": "token_count", "text": text}).to_string();
    frame.push('\n');
    let mut writer = &stream;
    writer
        .write_all(frame.as_bytes())
        .map_err(|e| format!("sending the measurement to {endpoint} failed: {e}"))?;
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line).map_err(|e| {
        if matches!(
            e.kind(),
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
        ) {
            "the script entry's wall clock elapsed while waiting for the endpoint's answer"
                .to_owned()
        } else {
            format!("reading the endpoint's answer failed: {e}")
        }
    })?;
    if line.is_empty() {
        return Err(format!(
            "the measuring endpoint at {endpoint} closed without answering"
        ));
    }
    let answer: Value = serde_json::from_str(&line).map_err(|e| {
        format!(
            "the endpoint's answer is not a JSON line ({e}): {}",
            line.trim()
        )
    })?;
    if let Some(error) = answer.get("error").and_then(Value::as_str)
        && !error.is_empty()
    {
        return Err(error.to_owned());
    }
    answer
        .get("data")
        .and_then(|d| d.get("tokens"))
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("the endpoint's answer carries no count: {}", line.trim()))
}

/// The §4.6 touch-set commit premises (run-plane amendment, 2026-08-15;
/// wire-contract § A.7): entry-vs-live verified at exactly the nodes this
/// attempt touched — every served read's file, every pattern expansion's
/// matched member, every armed target. Each premise claims the ENTRY world's
/// own leaf token, spelled under the entry fingerprint's identity, so the
/// door's §5.4 premise check IS the entry-vs-live verify, O(touch set). An
/// armed path absent at entry mints the `absent` premise (§5.6) — the birth
/// refuses if anything was born there since entry.
///
/// Not premised, by law: reads served from OUTSIDE the entry hash domain —
/// the fingerprint never covered those bytes (§12.1), exactly as before.
/// Stated limit: a read the program made of an ABSENT path is not in the
/// recording (only served faces are recorded), so foreign birth at a merely
/// LOOKED-AT empty path cannot refuse; the touch-set floor — the armed
/// writes — is always covered.
fn touch_premises(
    eval: &effects::ScriptEval,
    world: &WorkspaceEngine,
    entry: &str,
) -> Vec<wire_serve::guard::Premise> {
    use wire_serve::guard::{Premise, PremiseValue};
    // The entry identity every premise token respells under: same hash law,
    // same domain generation as the entry fingerprint itself.
    let Some(identity) = model::parse_root(entry)
        .and_then(|p| p.position.checked_sub(1))
        .map(model::RootVersion::law2)
    else {
        // An unparseable entry token cannot mint premises; the commit then
        // rides the per-row CAS + the caller's own widening guard alone.
        return Vec::new();
    };
    let armed: Vec<String> = eval.content_paths();
    let mut ordered: Vec<&str> = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for read in &eval.recording.reads {
        if seen.insert(read.path.as_str()) {
            ordered.push(&read.path);
        }
    }
    for expansion in &eval.recording.expansions {
        for member in &expansion.matched {
            if seen.insert(member.as_str()) {
                ordered.push(member);
            }
        }
    }
    for path in &armed {
        if seen.insert(path.as_str()) {
            ordered.push(path);
        }
    }
    let mut premises = Vec::new();
    for path in ordered {
        let entry_bytes: Option<&[u8]> = world
            .docs
            .get(path)
            .map(|d| d.raw.as_bytes())
            .or_else(|| world.unserved.get(path).map(String::as_bytes));
        match entry_bytes {
            Some(bytes) => premises.push(Premise {
                scope: Some(std::path::PathBuf::from(path)),
                value: PremiseValue::Token(identity.token(model::leaf_digest(bytes)).0),
            }),
            None if armed.iter().any(|a| a == path) => premises.push(Premise {
                scope: Some(std::path::PathBuf::from(path)),
                value: PremiseValue::Absent,
            }),
            // A served read outside the entry hash domain: never covered by
            // the fingerprint law, so it holds no premise.
            None => {}
        }
    }
    premises
}

/// The one guarded splice, issued daemon-side through the same choke-point
/// every wire splice takes — `Origin::Wire`, the ring advanced on a real
/// commit (this lane mints Deltas; the CLI put lane's row-12 gap does not
/// extend here).
///
/// The commit's authority is the §4.6 TOUCH SET ([`touch_premises`]), not
/// the whole-corpus entry fingerprint: a foreign write outside the touch set
/// no longer refuses; inside it refuses `fingerprint_mismatch` naming the
/// moved premise's scope (§5.7). The caller's own `if_fingerprint` rides
/// through as a WIDENING premise — strictest wins, never sufficient alone.
fn commit(
    registry: &Registry,
    ws: &Path,
    request: &ScriptArgs,
    eval: &effects::ScriptEval,
    world: &WorkspaceEngine,
    entry: &str,
) -> CommitLeg {
    let paths = eval.content_paths();
    let premises = touch_premises(eval, world, entry);
    // H1 order: the mint store and ring handles are taken outside any engine
    // borrow (none is held here — the entry world is an Arc, not a lock).
    let mints = registry.read_mints(ws);
    let ring = registry.ring(ws);
    let ws_root = fs::WorkspaceRoot(ws.to_path_buf());
    // The splice is a function call here, so a lost answer cannot happen —
    // except as a panic mid-splice, which is the same indeterminacy: caught,
    // spoken as `commit_unknown`, never an unwind through the connection
    // thread (`docs/run-plane.md` § A controlled failure exit SPEAKS).
    //
    // One armed path is the single §4.4 splice; N paths are the §4.4 SET form
    // (`splice.set`) — one sealed commit under the entry guard, per
    // run-plane.md § One COMMIT per attempt.
    let cache = registry.domain_cache(ws);
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if let [path] = paths.as_slice() {
            let args = wire_serve::write::SpliceArgs {
                premises: premises.clone(),
                id: request.id,
                path: wire::Path(path.clone()),
                origin: wire_serve::guard::Origin::Wire,
                actor: request.actor.clone(),
                now: request.now.clone(),
                receipt: request.receipt.clone(),
                // The caller's own token stays a widening premise (§4.6) —
                // the engine's authority is the touch set above.
                if_root: request.if_root.clone(),
                dry: request.dry,
                force: false,
                edits: Vec::new(),
                plan_edits: eval.armed.iter().map(|armed| armed.edit.clone()).collect(),
                pin: None,
            };
            wire_serve::write::splice_with_mints(
                &ws_root,
                Some(&*ring),
                &args,
                &[],
                wire_serve::write::Mints {
                    ambient: Some(&mints),
                    foreign: None,
                },
                Some(&cache),
            )
        } else {
            let args = wire_serve::write::SpliceSetArgs {
                premises: premises.clone(),
                id: request.id,
                files: set_files(&paths, &eval.armed),
                origin: wire_serve::guard::Origin::Wire,
                actor: request.actor.clone(),
                now: request.now.clone(),
                receipt: request.receipt.clone(),
                // Caller widening only — the touch set is the authority.
                if_root: request.if_root.clone(),
                dry: request.dry,
                force: false,
            };
            wire_serve::write::splice_set_with_cache(
                &ws_root,
                Some(&*ring),
                &args,
                &[],
                Some(&cache),
            )
        }
    }));
    let Ok(outcome) = caught else {
        return CommitLeg::Unknown(lost_commit(request.dry));
    };
    match outcome {
        Ok(out) => {
            // The sink recorded any committed frame inside the flock
            // (`SeqSink::committed`) — nothing to advance here.
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

/// The armed list grouped into §4.4 set members: one entry per distinct
/// content path in first-arm order, each carrying its own rows in arm order.
fn set_files(paths: &[String], armed: &[ArmedEdit]) -> Vec<wire::SpliceFile> {
    paths
        .iter()
        .map(|p| wire::SpliceFile {
            path: wire::Path(p.clone()),
            edits: Vec::new(),
            plan_edits: armed
                .iter()
                .filter(|a| a.path == *p)
                .map(|a| a.edit.clone())
                .collect(),
        })
        .collect()
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

/// Thread each armed row's CAS token from the ENTRY world, unconditionally —
/// the CAS relaxation (ruling 2026-08-13, dissolves the write-follows-read
/// license): no read ritual gates a row, the value is the entry state's own
/// (file rev for `set_property`, the section's node rev for `append`), and an
/// overlay rev is never a token by construction — this function consults only
/// the entry toc, so a rev naming bytes no disk carried cannot be minted.
///
/// Consistency does not ride these tokens: the commit carries `if_root` = the
/// entry fingerprint, and a world that moved since entry refuses there (§5.1,
/// checked first) before any row's rev is compared. On an unmoved world every
/// threaded token matches by construction — the tokens exist to satisfy the
/// unchanged wire guard, one law for every door, while the author writes
/// rev-free (put parity: append cannot clobber; a destructive row is guarded
/// by the entry-fingerprint snapshot the engine already holds).
///
/// A target the entry state cannot name (an absent section, an unloadable
/// path) threads nothing and meets the engine's own target-class refusal at
/// the splice.
fn thread_entry(
    armed: &[ArmedEdit],
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
                    *rev = entry_toc(world, ws, root, &arm.path).map(|facts| facts.rev);
                }
                PlanEdit::Append {
                    hpath,
                    rev: rev @ None,
                    ..
                } => {
                    *rev = entry_toc(world, ws, root, &arm.path).and_then(|facts| {
                        facts
                            .toc
                            .iter()
                            // Segment-true (the one matcher family): a row whose
                            // raw heading carries `/` threads exactly like any
                            // other.
                            .find(|entry| entry.addresses(hpath))
                            .map(|entry| entry.rev.clone())
                    });
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
                // The raw segments behind the joined spelling — the feedable
                // machine address (D-1), `n` carried when the wire minted one.
                hpath: node.hpath.clone().unwrap_or_default(),
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
        section: &ReadSel,
        armed: &[ArmedEdit],
    ) -> Result<SecFacts, ReadFault> {
        let display = section.display();
        self.within_deadline(path, Some(&display))?;
        let fault = |reason: String| ReadFault {
            path: path.to_owned(),
            section: Some(display.clone()),
            reason,
        };
        // The kernel's `section=` boundary already parsed the one selector
        // grammar; the shared resolver serves it — the dewey lane is served
        // here since the read-alignment ruling (2026-08-13): one
        // `selector_matches` resolution, every door.
        let doc = self.doc_for(path, Some(&display), armed)?;
        let sec = wire_serve::read::selector_to_secref(&doc, section).map_err(&fault)?;
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
    use effects::ScriptRecording;
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
            premises: Vec::new(),
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

    /// The entry-rev law under the CAS relaxation (ruling 2026-08-13): the
    /// value is the entry world's, and no license gates it — a row whose
    /// target the attempt never read threads the ENTRY rev all the same, at
    /// its own grain (file rev for `set_property`, node rev for `append`).
    /// An overlay rev is never a CAS token by construction: threading consults
    /// only the entry toc, so a token naming bytes no disk carried cannot be
    /// minted here.
    #[test]
    fn threading_values_from_the_entry_world_without_a_license() {
        let tmp = tempfile::tempdir().unwrap();
        let registry = registry_in(tmp.path());
        let ws = seeded_ws(tmp.path());
        let (world, root) = pinned_world(&registry, &ws);
        let entry = entry_toc(&world, &fs::WorkspaceRoot(ws.clone()), &root, "doc.md")
            .expect("doc in world");
        let alpha_rev = entry
            .toc
            .iter()
            .find(|row| row.section == "Alpha")
            .expect("the seeded section")
            .rev
            .clone();

        let armed = vec![
            ArmedEdit {
                path: "doc.md".to_owned(),
                edit: PlanEdit::SetProperty {
                    key: "status".to_owned(),
                    value: "done".to_owned(),
                    rev: None,
                },
                line: 2,
                depth: 0,
            },
            ArmedEdit {
                path: "doc.md".to_owned(),
                edit: PlanEdit::Append {
                    hpath: vec![wire::HpathSeg {
                        h: "Alpha".to_owned(),
                        n: None,
                    }],
                    body: "four\n".to_owned(),
                    rev: None,
                },
                line: 3,
                depth: 0,
            },
        ];

        let threaded = thread_entry(&armed, &world, &ws_root_of(&ws), &root);
        let PlanEdit::SetProperty { rev, .. } = &threaded[0].edit else {
            panic!("shape preserved");
        };
        assert_eq!(
            rev.as_deref(),
            Some(entry.rev.as_str()),
            "an unread target threads the ENTRY file rev — the read ritual is \
             dissolved; the §5.1 world guard is the enforcement point"
        );
        let PlanEdit::Append { rev, .. } = &threaded[1].edit else {
            panic!("shape preserved");
        };
        assert_eq!(
            rev.as_deref(),
            Some(alpha_rev.as_str()),
            "an append threads its section's ENTRY node rev, rev-free for the \
             author (put parity)"
        );
    }

    fn ws_root_of(ws: &Path) -> fs::WorkspaceRoot {
        fs::WorkspaceRoot(ws.to_path_buf())
    }

    /// A minimal request for driving [`commit`] directly.
    fn request_of(if_root: Option<wire::Root>) -> ScriptArgs {
        ScriptArgs {
            id: None,
            source: String::new(),
            args: std::collections::BTreeMap::new(),
            files: Vec::new(),
            actor: Some("agent:test".to_owned()),
            now: None,
            receipt: None,
            if_root,
            dry: false,
            expect_armed: None,
            effects: Vec::new(),
            invocation: None,
            token_count_endpoint: None,
        }
    }

    /// One armed `set_property` on `doc.md`, threaded from the entry world —
    /// the smallest committable armed set.
    fn armed_on_doc(world: &Arc<WorkspaceEngine>, ws: &Path, root: &wire::Root) -> Vec<ArmedEdit> {
        let armed = vec![ArmedEdit {
            path: "doc.md".to_owned(),
            edit: PlanEdit::SetProperty {
                key: "status".to_owned(),
                value: "done".to_owned(),
                rev: None,
            },
            line: 1,
            depth: 0,
        }];
        thread_entry(&armed, world, &ws_root_of(ws), root)
    }

    fn eval_of(armed: Vec<ArmedEdit>, recording: ScriptRecording) -> effects::ScriptEval {
        effects::ScriptEval {
            outcome: Ok(effects::ScriptFacts {
                bindings: std::collections::BTreeMap::new(),
            }),
            armed,
            recording,
            telemetry: effects::ScriptTelemetry {
                fuel_used: 0,
                mem_used: 0,
                reads_used: 0,
                wall_ms: 0,
            },
        }
    }

    /// §4.6 touch premises: reads, expansion members and armed targets each
    /// hold the ENTRY leaf token; an armed BIRTH path holds `absent`; and the
    /// entry-value spelling is byte-identical to the door's own live mint —
    /// the two instruments cannot drift.
    #[test]
    fn touch_premises_cover_reads_expansions_and_arms_at_entry_values() {
        let tmp = tempfile::tempdir().unwrap();
        let registry = registry_in(tmp.path());
        let ws = seeded_ws(tmp.path());
        let (world, root) = pinned_world(&registry, &ws);
        let mut host = host_of(&world, &root, &ws, Instant::now() + Duration::from_secs(7));
        let face = host.toc("doc.md", &[]).expect("entry world serves");

        let mut recording = ScriptRecording::default();
        recording.reads.push(effects::ReadRecord {
            path: "doc.md".to_owned(),
            section: None,
            line: 1,
            position: effects::ReadPosition::Echo,
            face: effects::ReadFace::Toc(face),
        });
        recording.expansions.push(effects::ExpansionRecord {
            pattern: "*.md".to_owned(),
            matched: vec!["doc.md".to_owned()],
        });
        let armed = vec![
            ArmedEdit {
                path: "doc.md".to_owned(),
                edit: PlanEdit::SetProperty {
                    key: "status".to_owned(),
                    value: "done".to_owned(),
                    rev: None,
                },
                line: 1,
                depth: 0,
            },
            ArmedEdit {
                path: "new/born.md".to_owned(),
                edit: PlanEdit::Create {
                    parent_hpath: Vec::new(),
                    title: "Born".to_owned(),
                    body: String::new(),
                    rev: None,
                },
                line: 2,
                depth: 0,
            },
        ];
        let eval = eval_of(armed, recording);

        let premises = touch_premises(&eval, &world, &root.0);
        assert_eq!(
            premises.len(),
            2,
            "doc.md deduplicated across the three sources"
        );
        let doc = &premises[0];
        assert_eq!(doc.scope.as_deref(), Some(Path::new("doc.md")));
        let wire_serve::guard::PremiseValue::Token(entry_token) = &doc.value else {
            panic!("an existing member holds its entry token");
        };
        // Cross-instrument: the entry-value spelling equals the door's own
        // live mint while nothing has moved.
        let live =
            wire_serve::write::scope_token(&ws_root_of(&ws), None, Some(Path::new("doc.md")))
                .expect("mint")
                .expect("token");
        assert_eq!(*entry_token, live, "entry spelling ≡ door spelling");
        assert_eq!(
            premises[1],
            wire_serve::guard::Premise {
                scope: Some(PathBuf::from("new/born.md")),
                value: wire_serve::guard::PremiseValue::Absent,
            },
            "an armed birth premises ABSENCE at entry (§5.6)"
        );
    }

    /// The §4.6 headline: a foreign DISJOINT birth between entry and commit
    /// no longer refuses the script commit — the touch set held.
    #[test]
    fn a_disjoint_foreign_birth_no_longer_refuses_the_commit() {
        let tmp = tempfile::tempdir().unwrap();
        let registry = registry_in(tmp.path());
        let ws = seeded_ws(tmp.path());
        let (world, root) = pinned_world(&registry, &ws);
        let eval = eval_of(armed_on_doc(&world, &ws, &root), ScriptRecording::default());

        // The disjoint foreign birth: a path the attempt never touched.
        write(ws.join("mover.md"), "# Mover\n\ndisjoint.\n").unwrap();

        // Absorb the foreign write into the door's instrument by a full
        // observation — the daemon's event feed does this in production;
        // the currency card (bug-trusted-overlay-unvouched) owns that
        // wiring, so this test does not depend on it.
        registry
            .domain_cache(&ws)
            .lock()
            .unwrap()
            .root(&ws_root_of(&ws))
            .unwrap();

        let leg = commit(&registry, &ws, &request_of(None), &eval, &world, &root.0);
        assert!(
            matches!(leg, CommitLeg::Response(_)),
            "the touch set held — a disjoint birth must not refuse: {leg:?}"
        );
        assert_eq!(
            std::fs::read_to_string(ws.join("doc.md")).unwrap(),
            DOC.replace("open", "done"),
            "the armed edit landed"
        );
    }

    /// The counter-proof: a foreign edit INSIDE the touch set still refuses,
    /// and the refusal names the moved premise's scope (§5.7).
    #[test]
    fn a_foreign_edit_inside_the_touch_set_refuses_naming_the_scope() {
        let tmp = tempfile::tempdir().unwrap();
        let registry = registry_in(tmp.path());
        let ws = seeded_ws(tmp.path());
        let (world, root) = pinned_world(&registry, &ws);
        let eval = eval_of(armed_on_doc(&world, &ws, &root), ScriptRecording::default());

        // The foreign edit lands on the ARMED file itself.
        write(ws.join("doc.md"), DOC.replace("open", "parked")).unwrap();

        // Absorb the foreign write into the door's instrument by a full
        // observation — the daemon's event feed does this in production;
        // the currency card (bug-trusted-overlay-unvouched) owns that
        // wiring, so this test does not depend on it.
        registry
            .domain_cache(&ws)
            .lock()
            .unwrap()
            .root(&ws_root_of(&ws))
            .unwrap();

        let leg = commit(&registry, &ws, &request_of(None), &eval, &world, &root.0);
        let CommitLeg::Conflict(raw) = leg else {
            panic!("a moved touched node refuses as a conflict: {leg:?}");
        };
        let frame: Value = serde_json::from_str(raw.get()).unwrap();
        assert_eq!(
            frame["code"], "fingerprint_mismatch",
            "v3 spelling: {frame}"
        );
        assert_eq!(
            frame["scope"], "doc.md",
            "the refusal names the moved premise's scope: {frame}"
        );
        assert_eq!(
            std::fs::read_to_string(ws.join("doc.md")).unwrap(),
            DOC.replace("open", "parked"),
            "nothing landed — the foreign state stands"
        );
    }

    /// The caller's own `if_fingerprint` stays a WIDENING premise at the
    /// commit: strictest wins, so a moved WORLD refuses even when the touch
    /// set held (§4.6 — the caller pinned the world and gets world grain).
    #[test]
    fn the_callers_own_token_stays_a_widening_premise_at_commit() {
        let tmp = tempfile::tempdir().unwrap();
        let registry = registry_in(tmp.path());
        let ws = seeded_ws(tmp.path());
        let (world, root) = pinned_world(&registry, &ws);
        let eval = eval_of(armed_on_doc(&world, &ws, &root), ScriptRecording::default());

        // Disjoint birth: the touch set holds; the WORLD moved.
        write(ws.join("mover.md"), "# Mover\n\ndisjoint.\n").unwrap();

        // Absorb the foreign write into the door's instrument by a full
        // observation — the daemon's event feed does this in production;
        // the currency card (bug-trusted-overlay-unvouched) owns that
        // wiring, so this test does not depend on it.
        registry
            .domain_cache(&ws)
            .lock()
            .unwrap()
            .root(&ws_root_of(&ws))
            .unwrap();

        let leg = commit(
            &registry,
            &ws,
            &request_of(Some(root.clone())),
            &eval,
            &world,
            &root.0,
        );
        assert!(
            matches!(leg, CommitLeg::Conflict(_)),
            "the caller pinned the world; the world moved; strictest wins: {leg:?}"
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
                expansions: Vec::new(),
                actor: String::new(),
                reads: Vec::new(),
                files: Vec::new(),
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

#[cfg(test)]
mod run_charging_tests {
    //! run-walk-real-roots (dogfood r2 D-USER F8): a live `run()`'s execution
    //! is metered on the run plane's own budget, never the script clock. The
    //! module-grain pin — the wire harness cannot place a deadline.

    use super::*;
    use crate::state::StateStore;
    use std::fs::{create_dir_all, write};
    use std::time::{Duration, Instant};

    const TASKS: &str = "\
---
task.nap: \"[[#^b1]]\"
---

# Tasks

```bash
sleep 0.5
```
^b1
";

    /// The script clock admits a `run()` and then STOPS while the run plane
    /// executes: the run's own elapsed — its walks and its child — never
    /// costs the caller's wall clock. Before the fix, a run outliving the
    /// remaining clock made every later act fault (`wall clock elapsed
    /// before a live put`) even though the program's own compute was
    /// milliseconds.
    #[cfg(unix)]
    #[test]
    fn a_live_runs_execution_never_costs_the_script_clock() {
        let tmp = tempfile::tempdir().unwrap();
        let cache_root = tmp.path().join("cache");
        create_dir_all(&cache_root).unwrap();
        let registry = Registry::new(
            StateStore::new(tmp.path().join("state.json")),
            cache_root,
            Vec::new(),
        );
        let ws = tmp.path().join("ws");
        create_dir_all(&ws).unwrap();
        write(ws.join("tasks.md"), TASKS).unwrap();
        let ws = std::fs::canonicalize(&ws).unwrap();
        registry.warm_or_build(&ws).expect("entry pass");
        let world = registry.engine_snapshot(&ws).expect("pinned world");
        let entry = world.at_fingerprint.0.clone();

        // A budget the 0.5s nap CANNOT fit inside: only stopping the clock
        // during the run's execution lets the next act through.
        let host = LiveHost {
            registry: &registry,
            ws: fs::WorkspaceRoot(ws.clone()),
            ws_path: ws.clone(),
            root: wire::Root(entry),
            deadline: std::cell::Cell::new(Instant::now() + Duration::from_millis(250)),
            actor: String::new(),
            now: None,
            invocation: "scr-t1".to_owned(),
            token_count_endpoint: None,
            run_seq: std::cell::Cell::new(0),
            reads_seen: std::cell::Cell::new(0),
            acts: std::cell::RefCell::new(Vec::new()),
        };
        let mut host = host;
        let row = host
            .run_live(
                "tasks.md",
                Some("nap"),
                Vec::new(),
                std::collections::BTreeMap::default(),
                false,
                1,
            )
            .expect("the run is admitted and answers a row");
        assert!(row.is_object(), "a § A.8 row came back: {row}");
        assert!(
            host.within_deadline("a live put").is_ok(),
            "the run plane's own elapsed must not cost the script clock — \
             the act AFTER a long run is still admitted"
        );
    }
}
