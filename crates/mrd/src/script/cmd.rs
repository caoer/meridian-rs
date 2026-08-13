//! `mrd script` — the script entry's consumer plane (`docs/run-plane.md`
//! § The script entry).
//!
//! ```text
//! mrd script [--actor A] [--now T] [--receipt PATH#ANCHOR] [--if-fingerprint FP]
//!            [--expect-armed DIGEST] [--files PATH]… [--args JSON] [--dry]
//!            [--json]  < script.star
//! ```
//!
//! The source rides stdin, matching `mrd put`'s stdin seam. Everything else is
//! argv, and every input is the caller's: the engine mints no identity and reads
//! no clock (§9).
//!
//! **The transaction is stand-still optimistic, and the entry is
//! single-attempt.** One `fingerprint` (§4.7) pins the entry fingerprint, reads
//! run LIVE against it, and the commit is ONE `splice` carrying
//! `if_fingerprint` = that same value, which §5.1 checks first. A world that
//! moved refuses `fingerprint_mismatch` and NOTHING commits — the entry never
//! retries, because the retry loop is the host's (`attempts:N` is a host fact,
//! never a field of this trace).
//!
//! A caller may pin its own `--if-fingerprint`. It is checked against the minted
//! entry fingerprint before evaluation — a fast-fail courtesy, refusing with
//! zero reads and nothing armed — and the SAME value still rides the commit, so
//! a world that moves during eval is still caught. Two checks, one value.
//!
//! A caller may also pin `--expect-armed <digest>`: the digest of the armed set
//! it authorized. It is checked after rev threading and BEFORE the splice is
//! issued, so a run that armed anything else sends nothing at all. This is the
//! commit half of the arm/commit split — the host gates the arm's rows, then
//! hands this digest to the commit child so "the two children arm identically"
//! becomes a measurement instead of an argument. The digest has exactly one
//! definition ([`super::digest::armed_digest`]) and the trace publishes it, so
//! the host copies a string rather than re-deriving a second canonicalization.
//!
//! **Zero wire delta.** The ops are `hello`, `fingerprint`, `toc`, `cat`,
//! `splice` — every one already on the wire. This verb invents no op, no field
//! and no request shape.
//!
//! **The human face is non-normative** (§ The `mrd script` human-mode face). The
//! MCP host owns the normative text face and renders it from the trace;
//! `mrd script --json` emitting that trace is the contract between them. Two
//! normative renderers in two languages would drift.
//!
//! Exit triad: 0 committed or `no_effect` / 1 conflict, fault or refusal / 2 bad
//! invocation.

use std::collections::BTreeMap;
use std::io::Read as _;
use std::time::{Duration, Instant};

use effects::{
    ArmedEdit, ReadFace, ScriptCtx, ScriptEval, ScriptLimits, ScriptRecording, eval_script,
};
use registry::Client;
use serde_json::value::RawValue;
use serde_json::{Value, json};
use wire::{ErrorCode, PlanEdit, Recovery};

use super::trace::{CommitLeg, Refusal, ScriptOutcome, ScriptTrace};
use super::wire_host::{Door, Frame, SocketDoor, WireHost};
use crate::{Fail, Format, current_dir, engine};

/// The script entry's wall clock — the budget that binds around the kernel
/// (pure evaluation is fuel-bounded; only wire I/O is unbounded in time).
///
/// It binds at three layers inside this process, and every one is load-bearing
/// (`docs/run-plane.md` § Where the budgets bind): before every round trip
/// ([`WireHost::ask`]), on the socket itself ([`SocketDoor::connect`]), and
/// before the commit is issued ([`run`]). The MCP host's own bound on the child
/// process is the fourth layer and lives in the other repo.
pub(crate) const WALL_CLOCK: Duration = Duration::from_secs(7);

/// Run `mrd script [flags] < script.star`. Errors [`Fail`] — exit 2 on a bad
/// invocation; exit 1 when the run conflicted, faulted or was refused.
pub(crate) fn dispatch(args: &[String]) -> Result<(), Fail> {
    let parsed = Script::parse(args)?;
    let source = read_stdin_source()?;
    let cwd = current_dir()?;
    let resolved = crate::resolve::resolve_runtime(&cwd).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace for {}: {e}",
            cwd.display()
        ))
    })?;

    // The script entry has no degrade leg: it must execute AS the caller, and a
    // daemonless in-process write arrives actor-absent (`run-plane.md` § the
    // seam table, "wire-client mode"). No daemon ⇒ refuse, never write anyway.
    let client = Client::from_default()
        .map_err(|e| Fail::tool(format!("cannot resolve the daemon socket: {e}")))?;
    engine::ensure_daemon(&client).map_err(|e| {
        Fail::tool(format!(
            "the script entry runs as a wire client and no daemon answered ({e}). {}",
            engine::degrade_reason().unwrap_or_else(|| {
                "It writes AS you through the one door, so there is no daemonless leg.".to_owned()
            })
        ))
    })?;
    let mut door = SocketDoor::connect(client.socket_path(), &resolved.workspace)
        .map_err(|e| Fail::tool(format!("cannot dial the daemon: {e}")))?;

    let trace = run(&mut door, &parsed, &source)?;
    emit(&parsed, &trace);
    exit_of(&trace)
}

/// One attempt end to end, driven from parsed argv — the seam the design tests
/// hold, so the ops this verb puts on the socket are observable without a
/// daemon. It is the SAME path [`dispatch`] takes: parse, then run.
///
/// # Errors
/// The diagnostic the CLI would print: a bad invocation, or a transport
/// failure. A conflict, a fault and a refusal are OUTCOMES, not errors — they
/// come back as a trace.
pub fn attempt(args: &[String], source: &str, door: &mut dyn Door) -> Result<ScriptTrace, String> {
    let parsed = Script::parse(args).map_err(|fail| fail.message)?;
    run(door, &parsed, source).map_err(|fail| fail.message)
}

/// The whole single attempt, against any [`Door`].
///
/// # Errors
/// A transport failure, or a refusal that never reached evaluation.
pub(crate) fn run(door: &mut dyn Door, parsed: &Script, source: &str) -> Result<ScriptTrace, Fail> {
    // 1. The entry fingerprint (§4.7) — the premise the whole run is consistent
    //    with, and the value the commit will guard on.
    let entry = fingerprint(door)?;

    // 2. The caller's own guard, checked pre-eval: zero reads, nothing armed.
    if let Some(pinned) = &parsed.if_fingerprint
        && *pinned != entry
    {
        return Ok(ScriptTrace::guard_refused(entry, pinned));
    }

    // 3. Evaluate. Reads lower to `toc`/`cat` through the same door.
    let ctx = ScriptCtx {
        id: "script".to_owned(),
        args: parsed.args.clone(),
        files: parsed.files.clone(),
        effects: Vec::new(),
    };
    let deadline = Instant::now() + WALL_CLOCK;
    let mut eval = {
        let mut host = WireHost::new(door, parsed.actor.clone().unwrap_or_default(), deadline);
        eval_script(source, &ctx, ScriptLimits::default(), &mut host)
    };

    // 4. A failed evaluation never commits; zero armed is the read-class exit —
    //    no splice, no receipt, no fingerprint advance, nothing on disk.
    if eval.outcome.is_err() || eval.armed.is_empty() {
        return Ok(ScriptTrace::assemble(entry, &eval, CommitLeg::NotIssued));
    }

    // 5. Each armed row is guarded on the rev the script ITSELF read, then ONE
    //    splice. The trace is assembled from the guarded list, so what it shows
    //    is what went on the wire.
    eval.armed = guarded(&eval.armed, &eval.recording);
    // 5a. The caller's armed-set expectation, checked HERE — after rev threading,
    // because the threaded token is part of what was armed, and before anything
    // is issued. A mismatch means this child armed a set the host never gated, so
    // the refusal has to happen while the splice is still unsent: detection after
    // landing is not refusal (`docs/run-plane.md` § Sub-amendment (the armed-set
    // expectation, `--expect-armed`)).
    if let Some(expected) = &parsed.expect_armed {
        // Both sides of this comparison are the SAME function over the SAME
        // type: the arm published `armed_digest` of its rows, the host copied
        // the string verbatim, and this recomputes it here. Nothing strips or
        // re-adds the domain tag on either side — it rides inside the one
        // definition — so a tagged pin cannot false-refuse a tagged engine.
        let actual = super::digest::armed_digest(&super::digest::ArmedRow::of_all(&eval.armed));
        if *expected != actual {
            return Ok(ScriptTrace::assemble(
                entry,
                &eval,
                // ENGINE-MINTED: no frame crossed the wire, so no §8 code — and
                // the class is `fix`, not `retry`: re-running this exact request
                // arms the same set and refuses again. Only the caller changes it.
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
    // The commit is a wire call like any other, so the clock binds on it like
    // any other (§ Where the budgets bind, layer 3). A run whose clock elapsed
    // during evaluation refuses PRE-COMMIT: nothing is issued, so nothing lands
    // and the refusal is the whole answer. Without this the last leg of the
    // entry was the one leg no clock touched.
    let leg = if Instant::now() > deadline {
        // ENGINE-MINTED: nothing was sent, so the same request may succeed on a
        // faster world — the §8 reading of `retry`, named here because no frame
        // named it for us.
        CommitLeg::Refused(Refusal::minted(
            Recovery::Retry,
            "the script entry's wall clock elapsed before the commit was issued — the armed \
             edits were never sent, nothing landed, and no fingerprint advanced. re-run: the \
             reads that ran cost the budget",
        ))
    } else {
        commit(door, parsed, &eval, &entry)
    };
    Ok(ScriptTrace::assemble(entry, &eval, leg))
}

/// Thread each armed plan row's CAS token from the script's OWN recorded reads.
///
/// Every wire door demands a fingerprint for an edit that changes existing
/// content, or an explicit `force` (`wire-serve::guard`) — and the two grains
/// differ: `set_property` takes the **file** rev, because frontmatter semantics
/// are file-scoped, and `append` takes the **node** rev of the section it lands
/// in. A script holds both already: `read(path)` recorded the file rev and the
/// section map, `read(path, section=…)` recorded that section's rev.
///
/// So the guard is the read the decision was made against — the read-then-write
/// CAS the wire exists for, not a token minted to satisfy a check. A row whose
/// target the script never read keeps `rev: None` and meets the engine's own
/// teaching refusal, which is the honest answer to writing what you did not read.
///
/// Reads are LIVE, so the LAST recorded read of a target is the freshest picture
/// the script had; an already-set `rev` is never overwritten.
fn guarded(armed: &[ArmedEdit], recording: &ScriptRecording) -> Vec<ArmedEdit> {
    armed
        .iter()
        .map(|arm| {
            let mut arm = arm.clone();
            match &mut arm.edit {
                PlanEdit::SetProperty {
                    rev: rev @ None, ..
                } => {
                    *rev = file_rev_of(recording, &arm.path);
                }
                PlanEdit::Append {
                    hpath,
                    rev: rev @ None,
                    ..
                } => {
                    *rev = section_rev_of(recording, &arm.path, hpath);
                }
                _ => {}
            }
            arm
        })
        .collect()
}

/// The file rev the script last read for `path` — the doc-root token
/// `set_property` demands.
fn file_rev_of(recording: &ScriptRecording, path: &str) -> Option<String> {
    recording
        .reads
        .iter()
        .rev()
        .find_map(|read| match &read.face {
            ReadFace::Toc(facts) if read.path == path => Some(facts.rev.clone()),
            _ => None,
        })
}

/// The node rev the script last read for one section of `path` — the token an
/// `append` demands. A section read carries it directly; a toc read carries the
/// whole map, and the section is found by the address the row publishes.
fn section_rev_of(
    recording: &ScriptRecording,
    path: &str,
    hpath: &[wire::HpathSeg],
) -> Option<String> {
    recording.reads.iter().rev().find_map(|read| {
        if read.path != path {
            return None;
        }
        match &read.face {
            ReadFace::Section(facts)
                if read.section.as_deref().is_some_and(|s| addresses(s, hpath)) =>
            {
                Some(facts.rev.clone())
            }
            ReadFace::Toc(facts) => facts
                .toc
                .iter()
                .find(|entry| addresses(&entry.section, hpath))
                .map(|entry| entry.rev.clone()),
            ReadFace::Section(_) => None,
        }
    })
}

/// Does the recorded section spelling `recorded` address the same node as the
/// armed row's `hpath`?
///
/// **Both sides go through [`ReadSel::parse`]** — the one human-string→selector
/// door, which is also what `put(section=…)` arms through
/// (`effects::script_edit::section_segments`) and what `read(path, section=…)`
/// resolves through. Comparing a normalized JOIN against the raw recorded string
/// instead made the two spellings of one address disagree: `read(section="/A/B")`
/// records `/A/B`, the armed row joins to `A/B`, no rev threads, and the write
/// meets `guard_required` for a read that HAPPENED. A rev is a CAS token, so a
/// missed match is not a missed optimization — it is a refusal blaming the
/// caller for the parser's disagreement with itself.
///
/// The non-heading spellings answer `false` rather than a segment comparison: an
/// `^anchor` row and a dewey ordinal are real addresses that an `append` (which
/// carries an hpath) cannot be pointed at, so they match no armed row.
fn addresses(recorded: &str, hpath: &[wire::HpathSeg]) -> bool {
    // The ONE matcher, shared with the § A.7 entry-rev threading — a licensed
    // row must not depend on which lane evaluated it (moved 2026-08-12).
    effects::hpath_addresses(recorded, hpath)
}

/// The §4.7 integrity rung: mint the entry fingerprint.
fn fingerprint(door: &mut dyn Door) -> Result<String, Fail> {
    let line = door
        .call(&json!({"op": "fingerprint"}))
        .map_err(|e| Fail::tool(format!("the daemon did not answer `fingerprint`: {e}")))?;
    let frame = Frame::parse(&line).map_err(|e| Fail::tool(e.to_string()))?;
    let body: Value = frame
        .ok
        .then_some(frame.body.as_ref())
        .flatten()
        .and_then(|body| serde_json::from_str(body.get()).ok())
        .ok_or_else(|| Fail::tool(format!("`fingerprint` refused: {}", line.trim())))?;
    body.get("fingerprint")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| Fail::tool("`fingerprint` answered no fingerprint".to_owned()))
}

/// The one guarded splice: the armed list IS `plan_edits[]`, carried verbatim
/// (ruling B′), against the one content path the arm-time law allows.
///
/// The response never round-trips through a typed shape — the leg the trace
/// embeds is the daemon's own bytes.
fn commit(door: &mut dyn Door, parsed: &Script, eval: &ScriptEval, entry: &str) -> CommitLeg {
    let paths = eval.content_paths();
    let [path] = paths.as_slice() else {
        // Unreachable by construction: a second content path refuses at arm
        // time (`multi_file_write_set`), so this attempt never reaches here.
        // That unreachability is the only reason this door has no test, and it
        // is pinned at the law rather than asserted here —
        // `crates/mrd/tests/script_controlled_exits_speak.rs`
        // § `door_367_is_unreachable_only_because_the_arm_time_law_refuses_first`.
        // It still SPEAKS: nothing was sent, so the class is `fix` and the
        // workspace guarantee is a fact rather than a hope.
        return CommitLeg::Refused(Refusal::minted(
            Recovery::Fix,
            format!(
                "the armed set writes {} content paths; one script commits to ONE file. NO splice \
                 was issued — nothing was sent, nothing landed, no fingerprint advanced. fix: arm \
                 one content path",
                paths.len()
            ),
        ));
    };
    let mut request = json!({
        "op": "splice",
        "path": path,
        "plan_edits": eval.armed.iter().map(|armed| &armed.edit).collect::<Vec<_>>(),
        "if_fingerprint": entry,
    });
    if let Some(actor) = &parsed.actor {
        request["actor"] = json!(actor);
    }
    if let Some(now) = &parsed.now {
        request["now"] = json!(now);
    }
    if let Some((rpath, anchor)) = &parsed.receipt {
        request["receipt"] = json!({"path": rpath, "anchor": anchor});
    }
    if parsed.dry {
        request["dry"] = json!(true);
    }

    // ⭐ FROM HERE THE REQUEST IS ON THE WIRE, and every door below is a
    // CONTROLLED exit that must SPEAK (`docs/run-plane.md` § A controlled failure
    // exit SPEAKS). None of them may return `Err(Fail)`: that leaves through
    // `mrd::run` with prose on stderr and NOTHING on stdout, and a consumer
    // reading a nonzero exit beside an absent trace cannot tell this from a
    // process killed mid-write — the two have opposite remedies.
    let line = match door.call(&request) {
        Ok(line) => line,
        // The request went out; the answer never came. This engine does NOT know
        // whether the splice landed, and no word in the outcome set can say that,
        // so the trace says it in band.
        Err(e) => {
            return CommitLeg::Unknown(lost_answer(
                parsed.dry,
                &format!("the daemon did not answer `splice`: {e}"),
            ));
        }
    };
    let frame = match Frame::parse(&line) {
        Ok(frame) => frame,
        // The daemon answered bytes this engine cannot read. Same indeterminacy
        // as no answer at all: it may have applied the splice before replying.
        Err(e) => {
            return CommitLeg::Unknown(lost_answer(
                parsed.dry,
                &format!("the daemon's answer to `splice` would not parse: {e}"),
            ));
        }
    };
    match (frame.ok, frame.body, frame.error) {
        (true, Some(body), _) if parsed.dry => CommitLeg::Rehearsal(body),
        (true, Some(body), _) => CommitLeg::Response(body),
        // `ok` with no body: the daemon says it SUCCEEDED and hands nothing to
        // describe it with. The commit is not recoverable as a fact — there are no
        // bytes to embed — so the honest answer is the same one the lost answer
        // gets. Re-read; the workspace, not this trace, is the authority now.
        (true, None, _) => CommitLeg::Unknown(lost_answer(
            parsed.dry,
            "`splice` answered ok with no body, so there is no commit fact to carry",
        )),
        // A moved world is the conflict leg: the mismatch extras ride the
        // daemon's own bytes, so `{expected, actual}` need no re-typing.
        (false, _, Some(error)) if is_mismatch(&error) => CommitLeg::Conflict(error),
        // Every other refusal is a refusal, not a fault, and its message is the
        // engine's own.
        (false, _, Some(error)) => CommitLeg::Refused(refusal_of(&error)),
        // `ok: false` — the daemon REFUSED, so nothing landed; it just did not say
        // why. Determinate, so a plain refusal is honest here, and the class is
        // `respawn`: a frame that violates §8's own shape is a broken channel, not
        // a request the caller can fix.
        (false, _, None) => CommitLeg::Refused(Refusal::minted(
            Recovery::Respawn,
            format!(
                "`splice` refused with no error body, so the refusal cannot be classed: {}. \
                 Nothing landed — the daemon refused — but the answer violates the §8 frame \
                 shape. respawn: the channel is the fault, not the script",
                line.trim()
            ),
        )),
    }
}

/// The engine-minted refusal for a splice whose outcome is NOT KNOWN — the answer
/// never came, or came unreadable.
///
/// The class is the whole point, and it splits on `--dry`, exactly as the
/// consumer's own killed-engine face already splits
/// (`ccc-mcp-server internal/mcpserver/scriptexec.go`, `scriptKilledRefusal`):
///
/// - a live run is `resync`, because a splice already on the wire is the daemon's
///   to finish — re-read, never resend, or the resend writes twice;
/// - a `--dry` run is `retry`, because a rehearsal runs everything except disk,
///   so it provably committed nothing. Declaring `resync` there would tell a
///   caller their file might have changed when it could not — the same
///   fabrication, aimed the other way.
///
/// No `code`: no frame minted one, and inventing a §8 value no daemon can answer
/// with is the thing the triple's own clause forbids.
fn lost_answer(dry: bool, locus: &str) -> Refusal {
    if dry {
        return Refusal::minted(
            Recovery::Retry,
            format!(
                "{locus}. This was a DRY run — it rehearses everything except disk, so nothing \
                 could have been committed and the workspace is unchanged. retry: re-run the same \
                 script, a rehearsal writes nothing"
            ),
        );
    }
    Refusal::minted(
        Recovery::Resync,
        format!(
            "{locus}. The splice was ISSUED, so whether the workspace carries this run is \
             UNKNOWN — a commit already on the wire is the daemon's to finish. resync: re-read \
             and re-plan, never resend, because a resend writes twice"
        ),
    )
}

/// Is this the world-grain guard failing (§5.1)? The v3 session spells it
/// `fingerprint_mismatch`; the projection is the daemon's.
fn is_mismatch(error: &RawValue) -> bool {
    serde_json::from_str::<Value>(error.get())
        .ok()
        .and_then(|e| e.get("code").and_then(Value::as_str).map(str::to_owned))
        .is_some_and(|code| code == "fingerprint_mismatch")
}

/// The wire's refusal triple, carried across the boundary TYPED: the §8 `code`,
/// the closed `recovery` class, and the engine's own wording.
///
/// `recovery` has ONE source and a stated precedence — the frame's own field
/// first, because the daemon is the authority on the refusal it minted; on a
/// frame that carries none, the §8 frozen table's binding for the code
/// ([`wire::ErrorCode::recovery`]), which is that same source read a second way
/// and never a second table. Neither available is absence, not a guess.
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
    Refusal {
        code,
        recovery,
        reason: refusal_reason(error),
    }
}

/// The engine's own refusal wording, carried verbatim into the fault reason —
/// re-phrasing it here would fork the text in two places. It is a RENDERING of
/// the refusal; the class rides `Refusal::recovery`, never this string.
fn refusal_reason(error: &RawValue) -> String {
    let parsed: Option<Value> = serde_json::from_str(error.get()).ok();
    let field = |name: &str| -> Option<String> {
        parsed
            .as_ref()
            .and_then(|e| e.get(name))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    match (field("code"), field("message")) {
        (Some(code), Some(message)) => format!("{code}: {message}"),
        (Some(code), None) => code,
        (None, Some(message)) => message,
        (None, None) => error.get().to_owned(),
    }
}

/// Print the trace: the JSON contract, or the non-normative operator face.
fn emit(parsed: &Script, trace: &ScriptTrace) {
    match parsed.format {
        Format::Json => {
            println!("{}", serde_json::to_string_pretty(trace).expect("json"));
        }
        Format::Human => print_human(trace),
    }
}

/// The operator face — **non-normative**, deliberately thin: the normative face
/// is the MCP host's, rendered from the same trace this verb emits under
/// `--json`.
fn print_human(trace: &ScriptTrace) {
    println!(
        "script @ {} — {}",
        trace.entry_fingerprint,
        outcome_word(trace.outcome)
    );
    for armed in trace.armed_entries() {
        let verb = if armed.committed { "wrote" } else { "armed" };
        println!("  {verb} {} (line {})", armed.path, armed.line);
    }
    if let Some(fault) = &trace.fault {
        match fault.line {
            Some(line) => println!("  SCRIPT: at line {line} — {}", fault.reason),
            None => println!("  SCRIPT: {}", fault.reason),
        }
    }
    // The armed block above renders `armed … [not committed]` for an unknown
    // commit, because nothing zipped it committed. That reads as a promise the
    // engine cannot make, so the operator face states the indeterminacy too. It is
    // non-normative, and it still may not lie.
    if trace.commit_unknown {
        println!("  COMMIT UNKNOWN: the splice was issued and never answered for");
    }
    let telemetry = &trace.telemetry;
    println!(
        "  fuel {} · mem {} · reads {} · {} ms",
        telemetry.fuel_used, telemetry.mem_used, telemetry.reads_used, telemetry.wall_ms
    );
}

/// The outcome's own word, as the JSON spells it.
fn outcome_word(outcome: ScriptOutcome) -> &'static str {
    match outcome {
        ScriptOutcome::Committed => "committed",
        ScriptOutcome::NoEffect => "no_effect",
        ScriptOutcome::Conflict => "conflict",
        ScriptOutcome::Fault => "fault",
        ScriptOutcome::Refused => "refused",
        // The CLI lane is pure-only (no --effects surface); the word exists
        // for the § A.7 wire lane and is spelled here for exhaustiveness.
        ScriptOutcome::Effects => "effects",
    }
}

/// The exit triad's findings leg: a run that produced nothing has to leave
/// through a non-zero exit or the caller cannot tell it from a commit.
fn exit_of(trace: &ScriptTrace) -> Result<(), Fail> {
    match trace.outcome {
        // `Effects` is the § A.7 wire lane's clean-exit word; this pure-only
        // lane cannot produce it, and a trace carrying it still exits clean.
        ScriptOutcome::Committed | ScriptOutcome::NoEffect | ScriptOutcome::Effects => Ok(()),
        ScriptOutcome::Conflict => Err(Fail::findings(
            "fingerprint_mismatch — the world moved; nothing committed. resync: re-read and \
             re-plan, never resend"
                .to_owned(),
        )),
        ScriptOutcome::Fault | ScriptOutcome::Refused => {
            Err(Fail::findings(trace.fault.as_ref().map_or_else(
                || "the script produced nothing".to_owned(),
                |f| f.reason.clone(),
            )))
        }
    }
}

/// The parsed `script` invocation. Every field is the caller's own input.
pub(crate) struct Script {
    /// `--actor` (§9): the identity the commit carries and `me()` returns.
    pub(crate) actor: Option<String>,
    /// `--now` (§9): caller-supplied time. The engine reads no clock.
    pub(crate) now: Option<String>,
    /// `--receipt PATH#ANCHOR`: the receipt companion rides the same batch.
    pub(crate) receipt: Option<(String, String)>,
    /// `--if-fingerprint`: the caller's own guard, checked twice on one value.
    pub(crate) if_fingerprint: Option<String>,
    /// `--expect-armed`: the armed-set expectation. The digest the caller was
    /// shown by the arm leg; this run refuses PRE-SPLICE unless its own armed
    /// set hashes to the same value.
    pub(crate) expect_armed: Option<String>,
    /// `--files` (repeatable): host-enumerated paths, sorted. Paths only —
    /// content enters through `read()` alone, which is what keeps replay
    /// byte-identical.
    pub(crate) files: Vec<String>,
    /// `--args JSON`: a JSON object of strings, injected inert as a dict.
    pub(crate) args: BTreeMap<String, String>,
    /// `--dry`: rehearse the commit — everything except disk.
    pub(crate) dry: bool,
    format: Format,
}

impl Script {
    fn parse(args: &[String]) -> Result<Self, Fail> {
        let mut actor = None;
        let mut now = None;
        let mut receipt = None;
        let mut if_fingerprint = None;
        let mut expect_armed = None;
        let mut files: Vec<String> = Vec::new();
        let mut script_args: BTreeMap<String, String> = BTreeMap::new();
        let mut dry = false;
        let mut json = false;
        let mut it = args.iter();
        while let Some(arg) = it.next() {
            let mut value_of = |flag: &str| -> Result<String, Fail> {
                it.next()
                    .cloned()
                    .ok_or_else(|| Fail::tool(format!("{flag} needs a value")))
            };
            match arg.as_str() {
                "--json" => json = true,
                "--dry" => dry = true,
                "--actor" => actor = Some(value_of("--actor")?),
                "--now" => {
                    let value = value_of("--now")?;
                    // The strict wire decode is not on this path, so the §9
                    // format law is checked here, exactly as the server would.
                    if !wire::now_is_rfc3339(&value) {
                        return Err(Fail::tool(format!(
                            "--now must be RFC 3339 (YYYY-MM-DDTHH:MM:SS[.frac](Z|±HH:MM)): {value}"
                        )));
                    }
                    now = Some(value);
                }
                "--if-fingerprint" => if_fingerprint = Some(value_of("--if-fingerprint")?),
                "--expect-armed" => expect_armed = Some(value_of("--expect-armed")?),
                "--files" => files.push(value_of("--files")?),
                "--args" => {
                    let value = value_of("--args")?;
                    script_args = serde_json::from_str(&value).map_err(|e| {
                        Fail::tool(format!(
                            "--args takes a JSON object of strings: {e}. Nothing was evaluated."
                        ))
                    })?;
                }
                "--receipt" => {
                    let value = value_of("--receipt")?;
                    let Some((rpath, anchor)) = value.split_once('#') else {
                        return Err(Fail::tool(format!(
                            "--receipt wants PATH#ANCHOR (a block anchor address): {value}"
                        )));
                    };
                    if rpath.is_empty() || anchor.is_empty() {
                        return Err(Fail::tool(format!(
                            "--receipt wants PATH#ANCHOR (both parts non-empty): {value}"
                        )));
                    }
                    receipt = Some((rpath.to_owned(), anchor.to_owned()));
                }
                flag if flag.starts_with('-') => {
                    return Err(Fail::tool(format!("unknown flag: {flag}")));
                }
                value => {
                    return Err(Fail::tool(format!(
                        "script takes its source on STDIN, not as an argument: {value}"
                    )));
                }
            }
        }
        // Sorted, because `files[]` is a host-enumerated set and the entry is
        // replayed against the same order it ran on.
        files.sort();
        Ok(Script {
            actor,
            now,
            receipt,
            if_fingerprint,
            expect_armed,
            files,
            args: script_args,
            dry,
            format: if json { Format::Json } else { Format::Human },
        })
    }
}

/// Read the script source from stdin — the `mrd put` seam, one heredoc.
fn read_stdin_source() -> Result<String, Fail> {
    let mut raw = String::new();
    std::io::stdin()
        .read_to_string(&mut raw)
        .map_err(|e| Fail::tool(format!("cannot read the script from stdin: {e}")))?;
    if raw.trim().is_empty() {
        return Err(Fail::tool(
            "script wants its Starlark source on stdin — the module top level IS the program"
                .to_owned(),
        ));
    }
    Ok(raw)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use effects::{ReadPosition, ReadRecord, ScriptRecording, SecFacts, TocEntry, TocFacts};
    use wire::HpathSeg;

    use super::{ArmedEdit, PlanEdit, ReadFace, guarded};

    const HERE: &str = "cards/one.md";
    const THERE: &str = "cards/two.md";

    /// A toc read of `path`, publishing `file_rev` for the file and `note_rev`
    /// for its `Notes` section.
    fn toc_read(path: &str, file_rev: &str, note_rev: &str) -> ReadRecord {
        ReadRecord {
            path: path.to_owned(),
            section: None,
            line: 1,
            position: ReadPosition::Echo,
            face: ReadFace::Toc(TocFacts {
                rev: file_rev.to_owned(),
                fm: BTreeMap::new(),
                toc: vec![TocEntry {
                    section: "Notes".to_owned(),
                    anchor: None,
                    rev: note_rev.to_owned(),
                }],
                words: 7,
            }),
        }
    }

    /// A section read of `path`'s `Notes`, publishing `rev`.
    fn section_read(path: &str, rev: &str) -> ReadRecord {
        ReadRecord {
            path: path.to_owned(),
            section: Some("Notes".to_owned()),
            line: 2,
            position: ReadPosition::Echo,
            face: ReadFace::Section(SecFacts {
                text: "body".to_owned(),
                rev: rev.to_owned(),
            }),
        }
    }

    fn recording(reads: Vec<ReadRecord>) -> ScriptRecording {
        ScriptRecording {
            actor: "8ab41c02".to_owned(),
            reads,
        }
    }

    fn arm(path: &str, edit: PlanEdit) -> ArmedEdit {
        ArmedEdit {
            path: path.to_owned(),
            edit,
            line: 3,
            depth: 0,
        }
    }

    fn set_owner() -> PlanEdit {
        PlanEdit::SetProperty {
            key: "owner".to_owned(),
            value: "8ab41c02".to_owned(),
            rev: None,
        }
    }

    fn append_notes() -> PlanEdit {
        PlanEdit::Append {
            hpath: vec![HpathSeg {
                h: "Notes".to_owned(),
                n: None,
            }],
            body: "hi".to_owned(),
            rev: None,
        }
    }

    fn threaded(row: &ArmedEdit) -> Option<&str> {
        match &row.edit {
            PlanEdit::SetProperty { rev, .. } | PlanEdit::Append { rev, .. } => rev.as_deref(),
            _ => None,
        }
    }

    /// **The accident, made law.** [`guarded`] looks a row's CAS token up BY
    /// `arm.path`, so a script that read two files threads each row from the
    /// file that row targets. Nothing stated this and no test pinned it — and it
    /// is what narrows R4 today: a commit child that resolved to a different
    /// file cannot inherit the gated file's rev, because the lookup is keyed on
    /// the address, not on read order.
    ///
    /// An unstated accident either becomes law or becomes a regression. This
    /// makes it law.
    ///
    /// The fixture is built so read ORDER points the other way: the reads for
    /// `HERE` come LAST, so a lookup that took the freshest read regardless of
    /// path — the obvious refactor — would thread `HERE`'s revs onto rows armed
    /// at `THERE`, and every assertion below would fail.
    #[test]
    fn a_rows_rev_is_looked_up_by_its_own_path_not_by_read_order() {
        let recording = recording(vec![
            toc_read(THERE, "there-file", "there-note"),
            section_read(THERE, "there-section"),
            toc_read(HERE, "here-file", "here-note"),
            section_read(HERE, "here-section"),
        ]);
        let rows = guarded(
            &[
                arm(THERE, set_owner()),
                arm(THERE, append_notes()),
                arm(HERE, set_owner()),
                arm(HERE, append_notes()),
            ],
            &recording,
        );

        assert_eq!(
            threaded(&rows[0]),
            Some("there-file"),
            "set_property threads the FILE rev of its own target, not the last file read"
        );
        assert_eq!(
            threaded(&rows[1]),
            Some("there-section"),
            "append threads the NODE rev of its own target's section, not the last one read"
        );
        assert_eq!(
            threaded(&rows[2]),
            Some("here-file"),
            "and the other target threads its own — the lookup is keyed, not disabled"
        );
        assert_eq!(threaded(&rows[3]), Some("here-section"));
    }

    /// The same law from the other side: a row whose target the script never
    /// read threads NOTHING, even when some other file was read and its rev sits
    /// right there in the recording. `rev: None` meets the engine's own teaching
    /// refusal, which is the honest answer to writing what you did not read — a
    /// path-blind lookup would instead hand it a stranger's token and satisfy
    /// the wire's CAS check with a guard nobody's read backs.
    #[test]
    fn a_row_targeting_an_unread_path_threads_no_rev() {
        let recording = recording(vec![
            toc_read(HERE, "here-file", "here-note"),
            section_read(HERE, "here-section"),
        ]);
        let rows = guarded(
            &[arm(THERE, set_owner()), arm(THERE, append_notes())],
            &recording,
        );

        assert_eq!(threaded(&rows[0]), None, "set_property on an unread file");
        assert_eq!(threaded(&rows[1]), None, "append into an unread file");
    }
}
