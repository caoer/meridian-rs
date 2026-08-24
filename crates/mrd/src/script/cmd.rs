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
//! **One lane: the whole attempt IS the wire `script` op.** The daemon pins the
//! entry, expands, evaluates and commits; this verb parses argv, sends the
//! program, and renders the trace that comes back. So the system holds exactly
//! ONE commit-premise implementation, which is what `run-plane.md`:931 means by
//! *"the touch-set law covers ALL script lanes (S1), same product as MCP
//! `script`"*.
//!
//! **Single-attempt, and the commit's authority is the TOUCH SET** — the nodes
//! the attempt itself touched: every served read's file, every pattern
//! expansion's matched member, every armed target, verified entry-vs-live at
//! exactly those nodes. A foreign write OUTSIDE that set does not refuse; one
//! INSIDE it refuses `fingerprint_mismatch` naming the moved scope, and nothing
//! commits. The entry never retries — the retry loop is the host's (`attempts:N`
//! is a host fact, never a field of this trace).
//!
//! *(Until card `script-door-commit-premise-world-grain-vs-touch-set`, a
//! pattern-less `files[]` drove a SECOND transaction here that guarded the
//! commit on the whole-corpus entry fingerprint — so any fleet write anywhere
//! refused it. That is the law `run-plane.md`:930-943 records as amended and
//! DELETED, and it is why a 64-file slice refused while all 64 of its targets
//! stood still.)*
//!
//! A caller may pin its own `--if-fingerprint`. It stays legal as a **widening**
//! premise — strictest wins, never sufficient alone, never able to drop write
//! coverage — checked against the minted entry fingerprint before evaluation as
//! a fast-fail courtesy, and again at the commit. A pin that is not a
//! `Root`-family token at all refuses BEFORE the compare, as a REFUSED trace
//! with the raw bytes debug-quoted (§ A.7's malformed arm; `run-plane.md` § the
//! pre-eval caller guard) — never as a moved world.
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
//! **Zero wire delta.** The ops are `hello` and `script` — both already on the
//! wire. This verb invents no op, no field and no request shape.
//!
//! **It needs a daemon.** This door writes AS you through the one socket, so
//! there is no daemonless leg. With none running it auto-spawns one and waits
//! for it to bind ([`crate::engine::ensure_daemon`]); if that never happens it
//! refuses by name and nothing is evaluated. That has always been true of this
//! verb — the check sits above every lane it ever had.
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
use std::time::Duration;

use registry::Client;
use serde_json::json;
use wire::ErrorCode;

use super::trace::{ScriptOutcome, ScriptTrace};
use super::wire_host::{Door, Frame, SocketDoor};
use crate::{Fail, Format, current_dir, engine};

/// The script door's §1 consequence clause — what did NOT happen because the
/// refusal fired.
const SCRIPT_CONSEQUENCE: &str = "Nothing was evaluated and nothing was written.";

/// Resolve the rooted `--files` members to their ONE shared workspace,
/// rewriting each rooted member to its rel half (the wire never carries a
/// root prefix). `Ok(None)` when every member is bare, or when the rooted
/// members all resolve to the ambient workspace itself (the bare form under
/// another name — one name per thing).
///
/// The one-root law is the customer face's (ccc-statusd MCP script tool,
/// verbatim): "Every entry must share one declared root. Inside the program
/// each entry is that root-relative path." Members landing in two workspaces
/// — two roots, or a bare member beside a foreign-rooted one — refuse loud.
///
/// # Errors
/// The rooted seam's `bad_path` family, plus the mixed-tree refusal.
fn rooted_files_workspace(
    files: &mut [String],
    ambient: &std::path::Path,
) -> Result<Option<std::path::PathBuf>, Box<wire::ErrorBody>> {
    let mut target: Option<(addr::MountName, std::path::PathBuf)> = None;
    let mut bare = false;
    for member in files.iter_mut() {
        if !crate::rooted::is_rooted(member) {
            bare = true;
            continue;
        }
        let (rel, rooted) = crate::rooted::resolve(member, "script", SCRIPT_CONSEQUENCE)?;
        match &target {
            None => target = Some((rooted.name, rooted.workspace)),
            Some((name, ws)) if *ws != rooted.workspace => {
                let mut e = wire::ErrorBody::new(ErrorCode::BadPath);
                e.path = Some(wire::Path(member.clone()));
                e.message = Some(format!(
                    "--files members resolve through more than one root (`{name}` and \
                     `{}`) — every files[] entry resolves through one root; that root is \
                     the workspace, and in-program paths are relative to it (the script \
                     one-root law). One program is one guarded write to one workspace. \
                     {SCRIPT_CONSEQUENCE}",
                    rooted.name
                ));
                return Err(Box::new(e));
            }
            Some(_) => {}
        }
        *member = rel;
    }
    let Some((name, ws)) = target else {
        return Ok(None);
    };
    // A rooted spelling of the ambient workspace itself is the bare form
    // under another name — normalize (the pin door's same-root posture).
    let ambient_canonical =
        workspace::canonicalize(ambient).unwrap_or_else(|_| ambient.to_path_buf());
    if ws == ambient_canonical {
        return Ok(None);
    }
    if bare {
        let mut e = wire::ErrorBody::new(ErrorCode::BadPath);
        e.message = Some(format!(
            "--files mixes bare members (the ambient workspace, {}) with members rooted \
             in `{name}` ({}) — every files[] entry resolves through one root; that root \
             is the workspace, and in-program paths are relative to it (the script \
             one-root law). Spell every member `{name}:rel`, or run from that root. \
             {SCRIPT_CONSEQUENCE}",
            ambient_canonical.display(),
            ws.display()
        ));
        return Err(Box::new(e));
    }
    Ok(Some(ws))
}

/// The script entry's wall clock — the budget that binds around the kernel
/// (pure evaluation is fuel-bounded; only wire I/O is unbounded in time).
///
/// **One layer binds in THIS process now**: the socket itself
/// ([`SocketDoor::connect`]), which bounds the one round trip this verb makes.
/// The other two layers moved with the lane — the daemon binds the clock at the
/// read builtin and again before the commit is issued
/// (`registry::script_op`, `docs/run-plane.md` § Where the budgets bind) — and
/// the MCP host's own bound on the child process is the outermost.
pub(crate) const WALL_CLOCK: Duration = Duration::from_secs(7);

/// Run `mrd script [flags] < script.star`. Errors [`Fail`] — exit 2 on a bad
/// invocation; exit 1 when the run conflicted, faulted or was refused.
pub(crate) fn dispatch(args: &[String]) -> Result<(), Fail> {
    let mut parsed = Script::parse(args)?;
    let source = read_stdin_source()?;
    let cwd = current_dir()?;
    let resolved = crate::resolve::resolve_runtime(workspace::Base::Cwd(&cwd)).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace for {}: {e}",
            cwd.display()
        ))
    })?;
    // The rooted lane on `--files` (§4.1 colon law, 2026-08-18
    // rooted-refs-everywhere), under the customer face's one-root law — the
    // ccc-statusd MCP script tool, verbatim: "Every files[] entry resolves
    // through one root; that root is the workspace; in-program paths are
    // relative to it." Rooted members resolve here, the ONE workspace they
    // share binds the connection at the hello, and the rel halves ride the
    // wire — a rooted glob member expands engine-side in that workspace,
    // exactly as a bare glob does in the ambient one. Members landing in two
    // trees refuse loud: one program is one guarded write to one workspace.
    let workspace = match rooted_files_workspace(&mut parsed.files, &resolved.workspace) {
        Ok(Some(ws)) => ws,
        Ok(None) => resolved.workspace.clone(),
        // The refusal frames with the workspace the caller stands in — no
        // one target workspace exists to name.
        Err(error) => {
            return Err(engine::json_refusal(
                parsed.output_format(),
                &resolved.workspace,
                &error,
            ));
        }
    };

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
    // The script entry's budget IS real (§ Where the budgets bind), so its
    // handshake keeps the entry's wall clock as the backstop — unchanged. What
    // changed is that a tick inside it now asks whether the daemon is alive
    // instead of asserting that it is not.
    let mut door = SocketDoor::connect(client.socket_path(), &workspace, WALL_CLOCK)
        .map_err(|e| Fail::tool(e.teach("no daemon answered the dial")))?;

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
/// **One lane.** Every attempt forwards as the wire `script` op: the daemon
/// pins the entry, expands, evaluates and commits under the §4.6 TOUCH SET
/// ([`registry::script_op::touch_premises`]'s law), and the answer is the
/// `ScriptTrace` this verb renders.
///
/// It used to fork here — a `files[]` carrying a glob forwarded (expansion is
/// the engine's, never a CLI-private one), and everything else drove a local
/// transaction whose commit guarded on the WHOLE-CORPUS entry fingerprint. That
/// second premise is the defect: `run-plane.md`:930-943 records the world-grain
/// law as amended and DELETED, and :931 names this lane — *"the touch-set law
/// covers ALL script lanes (S1), same product as MCP `script`"*. One product
/// means one commit path, so the fork is gone and, with it, the transaction
/// itself: `run_local`, `guarded`, `mint_for`, `mint_toc`, `file_rev_of`,
/// `section_rev_of`, `fingerprint` and `commit` were deleted in this card's
/// PR 2, together with the read lowering they drove
/// (`super::wire_host::WireHost`).
///
/// The local transaction could not simply be re-premised in place: touch-set
/// premises digest each touched file's WHOLE bytes, and this side recorded only
/// the served FACE of a read (a `cat` of one section is not the file). Porting
/// it would have bought a second, subtly different premise implementation —
/// the drift class that produced the bug.
///
/// # Errors
/// A transport failure, or a refusal that never reached evaluation.
pub(crate) fn run(door: &mut dyn Door, parsed: &Script, source: &str) -> Result<ScriptTrace, Fail> {
    forward(door, parsed, source)
}

/// The three facts every INDETERMINATE exit of [`forward`] owes its caller: the
/// program was sent, a commit may therefore have landed daemon-side, and the
/// remedy is a fresh read rather than a resend.
///
/// It is a module const rather than a `forward`-local one only because an item
/// after a statement is confusing (`clippy::items_after_statements`); its reader
/// is `forward` and nothing else.
const MAY_HAVE_LANDED: &str = "The program was SENT, so whether the workspace carries this \
     run is UNKNOWN — a commit already on the wire is the daemon's to finish. Verify with a \
     fresh read of the workspace fingerprint before retrying; never resend, because a resend \
     writes twice";

/// Forward one whole attempt as the § A.7 wire `script` op — the pattern
/// lane. The daemon owns the entry pin, the expansion, the evaluation, and
/// the commit; the trace that comes back is the one contract both lanes
/// speak, embedded verbatim.
///
/// # Errors
/// A transport failure, an unparseable answer, or a §8 error frame (a
/// refusal that never reached an entry — e.g. no workspace bound).
fn forward(door: &mut dyn Door, parsed: &Script, source: &str) -> Result<ScriptTrace, Fail> {
    let mut request = json!({"op": "script", "source": source});
    if !parsed.files.is_empty() {
        request["files"] = json!(parsed.files);
    }
    if !parsed.args.is_empty() {
        request["args"] = json!(parsed.args);
    }
    if let Some(actor) = &parsed.actor {
        request["actor"] = json!(actor);
    }
    if let Some(now) = &parsed.now {
        request["now"] = json!(now);
    }
    if let Some((rpath, anchor)) = &parsed.receipt {
        request["receipt"] = json!({"path": rpath, "anchor": anchor});
    }
    if let Some(pinned) = &parsed.if_fingerprint {
        request["if_fingerprint"] = json!(pinned);
    }
    if let Some(digest) = &parsed.expect_armed {
        request["expect_armed"] = json!(digest);
    }
    if parsed.dry {
        request["dry"] = json!(true);
    }
    // ⭐ FROM HERE THE PROGRAM IS ON THE WIRE. Every exit below is a CONTROLLED
    // exit and must SPEAK (`docs/run-plane.md` § A controlled failure exit
    // SPEAKS) — the same law `commit()` states at its own bracket.
    //
    // **A trace may carry ONLY daemon-supplied facts — never mint.** That is the
    // line that reconciles the two laws in tension here. Where the daemon's
    // answer supplies what a trace needs, this speaks a trace. Where it does not
    // — a lost answer, unreadable bytes — the entry fingerprint is the DAEMON's
    // and was never received, so a trace here would have to fabricate its first
    // field, which is the very thing the `NoPremise` control arm exists to
    // forbid (`script_controlled_exits_speak.rs`, "synthesizing one would mint a
    // fact"). Those paths stay `Err` and carry the indeterminacy in PROSE
    // instead, stating three facts every time: the daemon did not answer, a
    // commit MAY have landed daemon-side, and the remedy is a fresh read before
    // any retry.
    //
    // The law is therefore FORMALLY UNMET on those two paths, and that is
    // recorded rather than papered over: an honest "premise unknown" spelling in
    // `ScriptTrace` is a trace-contract change, carded as
    // `script-trace-premise-unknown-spelling` (it intersects
    // `wire-contract-a8-null-vs-empty-clause` — null vs empty vs absent is one
    // decision). Until it lands, THE PROSE IS THE OPERATING SURFACE, so the
    // prose is what the suite pins ([`MAY_HAVE_LANDED`], just above).
    let line = door.call(&request).map_err(|e| {
        Fail::tool(format!(
            "the daemon did not answer `script` ({e}). {MAY_HAVE_LANDED}"
        ))
    })?;
    let frame = Frame::parse(&line).map_err(|e| {
        Fail::tool(format!(
            "the daemon answered `script` in bytes this engine cannot read ({e}). \
             {MAY_HAVE_LANDED}"
        ))
    })?;
    match (frame.ok, frame.body.as_ref(), frame.error.as_ref()) {
        (true, Some(body), _) => serde_json::from_str::<ScriptTrace>(body.get()).map_err(|e| {
            Fail::tool(format!(
                "the daemon answered `script` with a trace this build cannot read ({e}) — \
                 engine and CLI likely disagree on the trace shape; align their versions. \
                 {MAY_HAVE_LANDED}"
            ))
        }),
        // DETERMINATE, and the daemon said so itself: a refusal frame that never
        // reached an entry means nothing was attempted, so there is no
        // indeterminacy to state and none is invented.
        //
        // RULING (afe34e1a, identical to e57663f7's, 2026-08-23): *a readable §8
        // refusal speaks as `CommitLeg::Refused` IFF every trace fact is
        // daemon-supplied — the refusal frame's own premise token, code, and
        // words, nothing minted.* The instruction that came with it was to check
        // whether a premise token is there before writing that branch.
        //
        // CHECKED, and the branch is unreachable, so this arm correctly stays
        // `Err`. Two independent reasons:
        //
        // 1. `registry::script_op` emits a §8 error frame for the `script` op
        //    ONLY from above the entry pin. `serve()` has exactly three
        //    `Err(ErrorBody)` exits and all three stand before
        //    `let entry = world.at_fingerprint.0.clone()` — the cold gate
        //    (`corpus_warming`), the entry pass (`io_error`), and the warm→pin
        //    race (`corpus_race`); `serve_line()` adds four further above still
        //    (a decode refusal, `unknown_op` on a non-v3 session, `bad_request`
        //    for an unbound workspace, and the internal routing arm). Every
        //    other terminal — including a moved touch set and an
        //    `expect_armed` mismatch — comes back `ok: true` INSIDE a trace.
        // 2. `wire::ErrorBody` carries no slot bound to a code THIS DOOR can
        //    emit that could hold an entry premise: on those codes
        //    `expected`/`actual` are the §8 `cas_mismatch` / `root_mismatch`
        //    node tokens, `new_fingerprint` is the `cas_mismatch` resend token,
        //    and `scope` is the scoped-premise spelling. **Stated in its bounded
        //    form on purpose** (reviewer correction, PR 1 verdict at
        //    `00fa6d624`): the struct DOES have a root-family slot shape —
        //    `stale_view` (§10.2) carries `required` / `as_of_root` /
        //    `live_root`, all `Root`-typed — so "no slot at all" was too wide.
        //    `stale_view` is not among the codes above, which is why it changes
        //    nothing here. Reason 1 — every §8 exit of this door stands ABOVE
        //    the entry pin — is the load-bearing half; this one is the narrower
        //    corroboration. A trace built here would still have to fabricate
        //    `entry_fingerprint`, which is exactly what the ruling forbids.
        //
        // Aperture: read at this head, on the daemon in THIS tree. The CLI dials
        // whatever daemon is resident, so a future engine that answered `script`
        // with a premise-bearing refusal frame would make the branch reachable —
        // and would then need this arm to grow one. That is the trigger, stated
        // rather than left to be rediscovered.
        (false, _, Some(error)) => Err(Fail::tool(format!(
            "`script` refused before any entry existed, so nothing was evaluated and nothing \
             landed: {}",
            error.get()
        ))),
        _ => Err(Fail::tool(format!(
            "the daemon's `script` answer violates the §8 frame shape (no body, no error). \
             {MAY_HAVE_LANDED}"
        ))),
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
    for entry in &trace.trace {
        match entry {
            super::TraceEntry::Expanded(row) => println!(
                "  expanded {} -> {} file(s)",
                row.pattern,
                row.matched.len()
            ),
            super::TraceEntry::Bound { index, path } => {
                println!("  bound files[{index}] -> {path}");
            }
            _ => {}
        }
    }
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
    /// `--files` (repeatable): paths in call order — `files[i]` is the i-th
    /// flag the caller typed (order-bind ruling). Paths only — content enters
    /// through `read()` alone, which is what keeps replay byte-identical.
    pub(crate) files: Vec<String>,
    /// `--args JSON`: a JSON object of strings, injected inert as a dict.
    pub(crate) args: BTreeMap<String, String>,
    /// `--dry`: rehearse the commit — everything except disk.
    pub(crate) dry: bool,
    format: Format,
}

impl Script {
    /// The output format, for the refusal envelope seam ([`engine::json_refusal`]).
    pub(crate) fn output_format(&self) -> Format {
        self.format
    }

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
        // Call order preserved: `files[i]` is the i-th `--files` the caller
        // typed (order-bind ruling — the wire door binds the same way), and
        // replay reconstructs that same order from the recording.
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
    use std::io;

    use serde_json::{Value, json};

    use super::Door;

    /// A door that answers the § A.7 `script` op with a canned trace and
    /// refuses every other op — the ONE lane forwards EVERYTHING, so a
    /// `fingerprint`/`toc`/`splice` trip here is a law violation.
    struct ScriptOnly {
        script_frames: Vec<Value>,
    }

    impl Door for ScriptOnly {
        fn call(&mut self, request: &Value) -> io::Result<String> {
            assert_eq!(
                request["op"], "script",
                "the one lane forwards the WHOLE attempt as one script op — \
                 it spends no other trip: {request}"
            );
            self.script_frames.push(request.clone());
            let trace = json!({
                "entry_fingerprint": "b3:feedface",
                "outcome": "no_effect",
                "trace": [{"kind": "expanded", "pattern": "notes/*.md", "matched": []}],
                "bindings": {"n": "0"},
                "telemetry": {"fuel_used": 1, "mem_used": 1, "reads_used": 0, "wall_ms": 1},
            });
            Ok(json!({"id": null, "ok": true, "body": trace}).to_string())
        }
    }

    /// § A.7 patterns (OQ3 ruling): a `--files` member carrying `*` forwards
    /// the whole attempt through the daemon's script op — the engine expands,
    /// never a CLI-private glob — and the daemon's trace comes back verbatim.
    #[test]
    fn a_files_pattern_forwards_the_attempt_to_the_daemon() {
        let mut door = ScriptOnly {
            script_frames: Vec::new(),
        };
        let argv = [
            "--files".to_owned(),
            "notes/*.md".to_owned(),
            "--actor".to_owned(),
            "8ab41c02".to_owned(),
        ];
        let trace = super::super::cmd::attempt(&argv, "n = len(files)\n", &mut door)
            .expect("the forwarded attempt answers");
        assert_eq!(door.script_frames.len(), 1, "exactly one wire trip");
        let sent = &door.script_frames[0];
        assert_eq!(
            sent["files"],
            json!(["notes/*.md"]),
            "the pattern rides verbatim"
        );
        assert_eq!(sent["actor"], json!("8ab41c02"));
        assert_eq!(sent["source"], json!("n = len(files)\n"));
        assert_eq!(trace.entry_fingerprint, "b3:feedface");
        assert_eq!(
            trace.bindings.get("n").map(String::as_str),
            Some("0"),
            "the daemon's trace is embedded verbatim"
        );
    }
}
