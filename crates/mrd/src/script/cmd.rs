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
use wire::{PlanEdit, ReadSel};

use super::trace::{CommitLeg, ScriptOutcome, ScriptTrace};
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
        let actual =
            super::digest::armed_digest(&eval.armed.iter().map(|a| &a.edit).collect::<Vec<_>>());
        if *expected != actual {
            return Ok(ScriptTrace::assemble(
                entry,
                &eval,
                CommitLeg::Refused(format!(
                    "expect_armed_mismatch: this run armed {actual}, the caller pinned \
                     {expected}. The armed set is not the one that was authorized, so NO \
                     splice was issued — nothing was sent, nothing landed, no fingerprint \
                     advanced. re-arm: run the arm leg again and gate the set it publishes"
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
        CommitLeg::Refused(
            "the script entry's wall clock elapsed before the commit was issued — the armed \
             edits were never sent, nothing landed, and no fingerprint advanced. re-run: the \
             reads that ran cost the budget"
                .to_owned(),
        )
    } else {
        commit(door, parsed, &eval, &entry)?
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
    let ReadSel::Hpath { hpath: parsed } = ReadSel::parse(recorded) else {
        return false;
    };
    // Empty segments carry no address — `ReadSel::parse("/A")` yields `["", "A"]`
    // and the arm filters them, so the comparison filters them on both sides or
    // one leading slash decides a CAS token.
    let mut recorded = parsed
        .iter()
        .map(|seg| seg.h.as_str())
        .filter(|h| !h.is_empty());
    let mut armed = hpath
        .iter()
        .map(|seg| seg.h.as_str())
        .filter(|h| !h.is_empty());
    loop {
        match (recorded.next(), armed.next()) {
            (None, None) => return true,
            (a, b) if a != b => return false,
            _ => {}
        }
    }
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
fn commit(
    door: &mut dyn Door,
    parsed: &Script,
    eval: &ScriptEval,
    entry: &str,
) -> Result<CommitLeg, Fail> {
    let paths = eval.content_paths();
    let [path] = paths.as_slice() else {
        // Unreachable by construction: a second content path refuses at arm
        // time (`multi_file_write_set`), so this attempt never reaches here.
        return Err(Fail::tool(format!(
            "the armed set writes {} content paths; one script commits to ONE file",
            paths.len()
        )));
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

    let line = door
        .call(&request)
        .map_err(|e| Fail::tool(format!("the daemon did not answer `splice`: {e}")))?;
    let frame = Frame::parse(&line).map_err(|e| Fail::tool(e.to_string()))?;
    match (frame.ok, frame.body, frame.error) {
        (true, Some(body), _) if parsed.dry => Ok(CommitLeg::Rehearsal(body)),
        (true, Some(body), _) => Ok(CommitLeg::Response(body)),
        (true, None, _) => Err(Fail::tool("`splice` answered ok with no body".to_owned())),
        // A moved world is the conflict leg: the mismatch extras ride the
        // daemon's own bytes, so `{expected, actual, changed}` need no re-typing.
        (false, _, Some(error)) if is_mismatch(&error) => Ok(CommitLeg::Conflict(error)),
        // Every other refusal is a refusal, not a fault, and its message is the
        // engine's own.
        (false, _, Some(error)) => Ok(CommitLeg::Refused(refusal_reason(&error))),
        (false, _, None) => Err(Fail::tool(format!(
            "`splice` refused with no error body: {}",
            line.trim()
        ))),
    }
}

/// Is this the world-grain guard failing (§5.1)? The v3 session spells it
/// `fingerprint_mismatch`; the projection is the daemon's.
fn is_mismatch(error: &RawValue) -> bool {
    serde_json::from_str::<Value>(error.get())
        .ok()
        .and_then(|e| e.get("code").and_then(Value::as_str).map(str::to_owned))
        .is_some_and(|code| code == "fingerprint_mismatch")
}

/// The engine's own refusal wording, carried verbatim into the fault reason —
/// re-phrasing it here would fork the text in two places.
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
    }
}

/// The exit triad's findings leg: a run that produced nothing has to leave
/// through a non-zero exit or the caller cannot tell it from a commit.
fn exit_of(trace: &ScriptTrace) -> Result<(), Fail> {
    match trace.outcome {
        ScriptOutcome::Committed | ScriptOutcome::NoEffect => Ok(()),
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
