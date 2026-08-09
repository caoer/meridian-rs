//! Bash dispatch (U6a) — two-phase apply around supervised exec
//! (plan §2, decisions #13/#19/#21/#22; verdict rulings 1/2):
//!
//! ```text
//! ┌ LOCKED WINDOW (one flock, u4-gate addendum on #19) ──────────────┐
//! │ phase 1: pre-exec receipt commit  →  root_after_phase1 snapshot  │
//! └──────────────────────────────────────────────────────────────────┘
//!   → U6b bracket OPENS against the computed root (PreExecMismatch ⇒
//!     refuse; exec never starts)
//!   → exec (invocation cwd — U16, `setsid`, timeout→group SIGKILL;
//!     stdout teed; md.* descriptors on the shim fd)
//!   → U6b bracket CLOSES after group kill (residual-compare #19 +
//!     config #20 + symlink #25) — verdict on EVERY path
//!   → phase 2 (clean window only): shim batch through executor choke
//!     point, pinned to COMPUTED `root_after_phase1` — never re-read
//!     after bash (#19).
//! ```
//!
//! **No `Exec` `EffectKind`** (ruling 1): only md.* descriptors on the shim
//! are effects.
//!
//! # Failure matrix (S2/S6/#21)
//! - phase-1 refusal → nothing ran, nothing committed ([`BashError::Phase1`]).
//! - exec nonzero → effect batch REFUSES; run still RECORDED (G3b: completion
//!   is not success). Completion receipt carries exit code.
//! - signaled → no completion receipt; phase-1 pre-exec receipt stands (S2).
//! - timeout → group SIGKILL; distinct state; phase 2 refuses; no completion.
//! - truncated/malformed shim → fails closed; whole phase-2 batch refuses (S6).
//! - executor refusal in phase 2 → typed; nothing applied.
//! - window not clean (U6b) → phase 2 refuses ([`Phase2::RefusedDetection`]);
//!   verdict on [`BashOutcome::detection`]; NEVER rolled back (ruling 2).
//!
//! # Detection (U6b)
//! Bracket opens against computed `root_after_phase1`, closes after group
//! kill; phase 2 gates on [`Detection::is_clean`]. Verdict is an observation
//! about the window, not a claim about the block. Rendered on every OUTCOME
//! path; a [`BashError`] aborts with bracket unclosed and no verdict.

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;

use effects::Effect;
use model::MerkleRoot;

use crate::caps::Authority;
use crate::exec::{self, ExecSpec, ExecStatus};
use crate::executor::{self, Applied, ApplyRequest, ExecError, ReceiptAddr, WorkspaceLock};
use crate::record::{self, RecordError, RunLog, StdoutRecord};
use crate::shim::{self, ShimError, ShimStream};
use crate::snapshot::{Detection, ExecBracket, OpenRefusal};

/// The authority every apply on this path rides: bash is an unsandboxed shell
/// with undeclared effects (`docs/laws.md` § Amendment — capabilities do not
/// apply to bash; gate `crates/mrd/tests/law_no_caps_on_bash.rs`).
///
/// One constant rather than a [`BashDispatch`] field — the law made
/// structural: the dispatcher holds no capability input for a caller to hand it.
const BASH_AUTHORITY: Authority = Authority::Unsandboxed;

/// One bash block dispatch: the addressed block's facts plus the
/// caller-supplied identity (§9 — nothing here mints or reads a clock).
#[derive(Debug)]
pub struct BashDispatch<'a> {
    /// The page the task lives on (workspace-relative).
    pub page: &'a str,
    /// The task name (effect provenance, receipt actor).
    pub task: &'a str,
    /// The addressed block's `node_rev` — the procedure-hash the pre-exec AND
    /// completion receipts attest (from the resolved task; §9). The pre-exec
    /// receipt pins the procedure the run is ABOUT to execute (orphan-lint).
    pub task_rev: &'a str,
    /// The fence's inner source.
    pub source: &'a str,
    /// Contract-validated positional args.
    pub args: Vec<String>,
    /// Contract-validated env — the only caller env the block sees.
    pub env: BTreeMap<String, String>,
    /// Caller-supplied invocation id (also names the stdout log).
    pub invocation_id: &'a str,
    /// Caller-supplied time fact.
    pub now: Option<&'a str>,
    /// Phase-1 pre-exec receipt address (S2: the orphan-lint anchor —
    /// records the invocation and the root phase 1 committed against).
    pub pre_receipt: Option<ReceiptAddr>,
    /// Phase-2 completion receipt address (rides the shim batch commit).
    pub receipt: Option<ReceiptAddr>,
    /// Decision-#26 explicit foreign-edit takeover (phase 2 only).
    pub takeover: bool,
    /// The caller-created out-of-tree scratch directory (artifacts; NOT the
    /// cwd — U16 runs the step in the invocation cwd).
    pub scratch: &'a Path,
    /// The wall-clock ceiling (#21; resolve via [`exec::configured_timeout`]).
    pub timeout: Duration,
}

/// What one bash dispatch produced. The exec facts are ALWAYS present —
/// phase-2 refusals are typed states, not lost information.
#[derive(Debug)]
pub struct BashOutcome {
    /// The committed phase-1 pre-exec receipt line (`None` without an
    /// address).
    pub pre_receipt_line: Option<String>,
    /// How the step ended (timeout is distinct, #21).
    pub status: ExecStatus,
    /// The stdout record (U8): sealed BEFORE any phase-2 receipt batch (S8).
    /// A record failure does not un-run the block — it is carried typed and
    /// the orphan lint finds a receipt whose log is missing.
    pub stdout: Result<StdoutRecord, RecordError>,
    /// Captured stderr (report diagnostics).
    pub stderr: Vec<u8>,
    /// The raw captured shim stream (report diagnostics; the parsed truth is
    /// in [`BashOutcome::phase2`]).
    pub shim: ShimStream,
    /// The exec-window detection verdict (U6b) — rendered on EVERY outcome,
    /// orthogonal to how the exec ended (a delta is named even when the step
    /// also timed out or failed). The U7 labeler consumes this as the
    /// type-level evidence for the #23 gate.
    pub detection: Detection,
    /// The phase-2 verdict.
    pub phase2: Phase2,
}

/// The phase-2 verdict — the S2/S6 failure matrix, typed.
#[derive(Debug)]
pub enum Phase2 {
    /// The step exited 0 with a whole stream; the md.* batch committed (or
    /// there was nothing to apply — `applied` is `None` on zero descriptors).
    Applied {
        /// The effects the block emitted, in emission order.
        effects: Vec<Effect>,
        /// The executor's commit result (`None` when the block emitted no
        /// descriptor — nothing to apply is not a fault).
        applied: Option<Applied>,
    },
    /// The step exited NONZERO — the effect batch refuses (nothing the block
    /// emitted applies), and the run is recorded anyway: **completion is not
    /// success** (G3b). The completion receipt carries the exit code, so a check
    /// that merely found something is no longer the same bytes as a crash.
    /// The code is in [`BashOutcome::status`].
    RefusedExecFailed {
        /// The completion receipt commit — an EMPTY batch under a real receipt.
        applied: Applied,
    },
    /// The step was killed by a signal the supervisor did not send — it did
    /// NOT complete, so no completion receipt is written and the pre-exec receipt
    /// stands alone (the orphan lint finds it, correctly). The signal is in
    /// [`BashOutcome::status`].
    RefusedSignaled,
    /// The wall-clock ceiling passed (#21) — the group was `SIGKILL`ed and
    /// phase 2 refuses.
    RefusedTimeout,
    /// The shim stream failed closed (S6) — NOTHING from it applies.
    RefusedShim(ShimError),
    /// The exec window did not verify clean (U6b: out-of-band delta,
    /// mid-window config change, symlink, or no verdict — fail closed) —
    /// phase 2 refuses outright; NOTHING from the shim applies. The verdict
    /// with the named delta is [`BashOutcome::detection`]; the ungoverned
    /// change is NEVER rolled back (ruling 2 — rollback would be a second
    /// write path with invented authority).
    RefusedDetection,
    /// The executor refused the batch — typed, nothing applied.
    RefusedExec {
        /// The effects that were refused (the report names them).
        effects: Vec<Effect>,
        /// The executor's refusal.
        error: ExecError,
    },
}

/// Why the dispatch failed BEFORE there was an exec to report on.
#[derive(Debug)]
pub enum BashError {
    /// The stdout log could not be created — refused before phase 1, nothing
    /// committed.
    Record(RecordError),
    /// Phase 1 (the pre-exec receipt commit) refused — nothing ran. Includes
    /// [`ExecError::WorkspaceBusy`] (decision #9).
    Phase1(ExecError),
    /// Computing a corpus root inside the locked window failed. When it was
    /// the post-phase-1 snapshot, the pre-exec receipt stands committed (the
    /// orphan lint finds it).
    Root {
        /// The underlying failure.
        reason: String,
    },
    /// Spawning bash failed — the block never ran. The phase-1 pre-exec
    /// receipt stands committed (the orphan lint finds it).
    Spawn {
        /// The underlying failure.
        reason: String,
    },
    /// The U6b detection bracket refused to open: a pre-exec root mismatch
    /// (out-of-band change BETWEEN the phase-1 commit and the bracket
    /// opening — nothing ran, nothing to accuse) or a guarded-snapshot
    /// refusal (symlink #25 / I/O). The exec never started; a committed
    /// phase-1 pre-exec receipt stands (the orphan lint finds it).
    Detection(OpenRefusal),
}

impl std::fmt::Display for BashError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BashError::Record(e) => write!(f, "stdout record: {e}"),
            BashError::Phase1(e) => write!(f, "phase 1: {e}"),
            BashError::Root { reason } => write!(f, "corpus root: {reason}"),
            BashError::Spawn { reason } => write!(f, "spawn: {reason}"),
            BashError::Detection(e) => write!(f, "detection bracket: {e}"),
        }
    }
}

impl std::error::Error for BashError {}

/// Run one bash block through the full two-phase bracket. `live` receives the
/// stdout stream as the child emits it (ruling 7 — stdout is data; the log
/// outranks the live sink, and a dead sink never kills the record).
///
/// # Errors
/// [`BashError`] — a failure BEFORE exec facts exist. Everything from exec
/// onward is a typed [`Phase2`] state inside the outcome.
pub fn run(
    root: &fs::WorkspaceRoot,
    d: &BashDispatch<'_>,
    live: &mut (dyn Write + Send),
) -> Result<BashOutcome, BashError> {
    // 0. The stdout log — created first, so an unwritable log refuses the
    // run before anything commits.
    let log = RunLog::create(&root.0, d.invocation_id).map_err(BashError::Record)?;

    // 0b. PRE-FLIGHT (G3): ask the guard whether it would refuse BEFORE
    // anything commits — see `preflight` for why the probe runs first and
    // where a refusal is recorded.
    let mut log = preflight(root, d, log)?;

    // 1. THE LOCKED WINDOW (u4-gate addendum on #19): the phase-1 commit and
    // the root_after_phase1 snapshot happen under ONE flock — no gap another
    // run could commit into between the two.
    let (root_after_phase1, pre_receipt_line) = {
        let lock = WorkspaceLock::acquire(&root.0).map_err(|e| {
            BashError::Phase1(if e.kind() == io::ErrorKind::WouldBlock {
                ExecError::WorkspaceBusy
            } else {
                ExecError::Io {
                    reason: format!("workspace lock: {e}"),
                }
            })
        })?;
        let root0 = snapshot(root)?;
        match &d.pre_receipt {
            Some(addr) => {
                let applied = executor::apply_under(
                    &lock,
                    root,
                    &ApplyRequest {
                        page: d.page,
                        task: d.task,
                        task_rev: d.task_rev,
                        invocation_id: d.invocation_id,
                        now: d.now,
                        effects: &[],
                        authority: &BASH_AUTHORITY,
                        pin_root: &root0,
                        live_root: &root0,
                        receipt: Some(addr.clone()),
                        takeover: false,
                        exec: None, // pre-exec: no child has run yet
                        depth: 0,
                    },
                )
                .map_err(BashError::Phase1)?;
                // Still inside the lock: the root the receipt commit made.
                (snapshot(root)?, applied.receipt_line)
            }
            None => (root0, None),
        }
    };

    // 2. U6b: the detection bracket opens against the flock-COMPUTED root
    // (#19 addendum — the computed root is the authority, never a re-read).
    // A PreExecMismatch means the tree moved between the phase-1 commit and
    // here: the exec never starts (the pre-exec receipt stands; the orphan
    // lint finds it).
    let bracket = ExecBracket::open(root, &root_after_phase1).map_err(BashError::Detection)?;

    // 3. Exec under the supervision bracket (#21/S3), stdout teed into the
    // record (U8) live. seal() runs inside the consumer — the record is
    // durable BEFORE any phase-2 receipt batch (S8 order).
    let spec = ExecSpec {
        source: d.source,
        args: &d.args,
        env: &d.env,
        scratch: d.scratch,
        project_root: &root.0,
        timeout: d.timeout,
    };
    let (result, stdout) = exec::exec_streaming(&spec, move |mut out| {
        let mut streamed = record::stream(&mut out, &mut log, live);
        if matches!(streamed, Err(RecordError::Live { .. })) {
            // The audience died; the record outranks it — keep logging to
            // EOF so the child is never blocked on a full stdout pipe.
            streamed = record::stream(&mut out, &mut log, &mut io::sink());
        }
        streamed.and_then(|_| log.seal())
    })
    .map_err(|e| BashError::Spawn {
        reason: e.to_string(),
    })?;

    // 4. U6b: close the bracket now the group is dead (S3) — UNCONDITIONAL,
    // so the delta is named on every exit path (timeout / nonzero /
    // shim-fail included). The verdict rides the outcome.
    let detection = bracket.close();

    // 5. The failure matrix (S2/S6/#21) behind the detection gate (#14): a
    // window that did not verify clean refuses phase 2 outright — nothing
    // from the shim applies, and the named delta is NEVER rolled back
    // (ruling 2). Otherwise phase 2 runs ONLY off a clean exit and a whole
    // stream.
    let phase2 = if detection.is_clean() {
        match &result.status {
            ExecStatus::TimedOut { .. } => Phase2::RefusedTimeout,
            ExecStatus::Signaled { .. } => Phase2::RefusedSignaled,
            ExecStatus::Exited { code } => match shim::parse(&result.shim) {
                Err(e) => Phase2::RefusedShim(e),
                // A zero-descriptor run still HAPPENED: without a completion
                // receipt, "authorised and never started" and "ran and changed
                // nothing" would be the same bytes. A completion IS an event,
                // so it enters the attested record (a refusal, asserting a
                // non-event, is logged outside it). U13: the sealed exec facts
                // ride the completion receipt.
                Ok(descriptors) => {
                    let exec = exec_record(&result.status, &stdout, &d.env);
                    if *code == 0 {
                        let effects = shim::to_effects(
                            &descriptors,
                            d.task,
                            d.invocation_id,
                            &root_after_phase1.0,
                        );
                        apply_phase2(root, d, &root_after_phase1, effects, exec.as_ref())
                    } else {
                        // G3b: completion is not success. The descriptors are
                        // discarded (a failed block's effects never apply) but
                        // the run is still recorded with its exit code — a
                        // check exiting nonzero on a finding must not leave
                        // the same bytes as a crash.
                        completion_receipt(root, d, &root_after_phase1, exec.as_ref())
                    }
                }
            },
        }
    } else {
        Phase2::RefusedDetection
    };

    Ok(BashOutcome {
        pre_receipt_line,
        status: result.status,
        stdout,
        stderr: result.stderr,
        shim: result.shim,
        detection,
        phase2,
    })
}

/// G3 pre-flight: ask the guard whether it would refuse BEFORE anything
/// commits — by the time the step-2 bracket can refuse, the phase-1 receipt
/// is already committed, an attested receipt no reader can tell from a
/// zero-effect completed run's. This NARROWS the window, it does not close it
/// (a change landing between probe and bracket-open still lands a receipt —
/// the orphan lint's subject).
///
/// A refusal asserts that nothing happened, so it is recorded in the run log
/// under `.meridian/`, outside the hash domain. Takes the log by value and
/// hands it back unrefused: sealing consumes it, and a refusal seals it here.
fn preflight(
    root: &fs::WorkspaceRoot,
    d: &BashDispatch<'_>,
    mut log: RunLog,
) -> Result<RunLog, BashError> {
    let Err(e) = fs::guard::StepGuard::probe(root) else {
        return Ok(log);
    };
    let refusal = format!(
        "pre-flight refused before any commit: {e}\n\
         invocation: {} (never started)\n\
         page: {} task: {}\n",
        d.invocation_id, d.page, d.task,
    );
    log.append(refusal.as_bytes()).map_err(BashError::Record)?;
    log.seal().map_err(BashError::Record)?;
    Err(BashError::Detection(OpenRefusal::Guard(e)))
}

/// G3b — the completion receipt for a failed run: nonzero exit under a clean
/// window, so the batch is empty by policy and the receipt records the
/// completion with its exit code (a fact the engine SEALED, not inferred).
/// Routing through `apply` with `effects: &[]` reuses the one choke point's
/// commit discipline rather than inventing a second write path. A commit
/// failure leaves the pre-exec receipt alone — the orphan lint finds it.
fn completion_receipt(
    root: &fs::WorkspaceRoot,
    d: &BashDispatch<'_>,
    root_after_phase1: &MerkleRoot,
    exec: Option<&record::ExecRecord>,
) -> Phase2 {
    match executor::apply(
        root,
        &ApplyRequest {
            page: d.page,
            task: d.task,
            task_rev: d.task_rev,
            invocation_id: d.invocation_id,
            now: d.now,
            effects: &[],
            authority: &BASH_AUTHORITY,
            pin_root: root_after_phase1,
            live_root: root_after_phase1,
            receipt: d.receipt.clone(),
            takeover: d.takeover,
            exec,
            depth: 0,
        },
    ) {
        Ok(applied) => Phase2::RefusedExecFailed { applied },
        Err(error) => Phase2::RefusedExec {
            effects: Vec::new(),
            error,
        },
    }
}

/// Phase 2: the shim batch through the executor's one choke point, pinned
/// AND validated against the COMPUTED `root_after_phase1` (#19 — never a
/// re-read around the bash step; out-of-band detection is U6b's bracket).
/// `exec` (U13) rides into the committed completion receipt.
fn apply_phase2(
    root: &fs::WorkspaceRoot,
    d: &BashDispatch<'_>,
    root_after_phase1: &MerkleRoot,
    effects: Vec<Effect>,
    exec: Option<&record::ExecRecord>,
) -> Phase2 {
    match executor::apply(
        root,
        &ApplyRequest {
            page: d.page,
            task: d.task,
            task_rev: d.task_rev,
            invocation_id: d.invocation_id,
            now: d.now,
            effects: &effects,
            authority: &BASH_AUTHORITY,
            pin_root: root_after_phase1,
            live_root: root_after_phase1,
            receipt: d.receipt.clone(),
            takeover: d.takeover,
            exec,
            depth: 0,
        },
    ) {
        Ok(applied) => Phase2::Applied {
            effects,
            applied: Some(applied),
        },
        Err(error) => Phase2::RefusedExec { effects, error },
    }
}

/// The receipt's exec facts (U13): built only for an exited child whose
/// stdout record SEALED — S8 holds because the record exists only after its
/// log fsync'd (`seal()` runs inside the exec consumer, before phase 2). An
/// unsealed record leaves `exec` absent and the orphan lint finds a receipt
/// whose log is missing. Env enters as KEYS ONLY (S7, by construction in
/// [`record::ExecRecord::new`]).
fn exec_record(
    status: &ExecStatus,
    stdout: &Result<StdoutRecord, RecordError>,
    env: &BTreeMap<String, String>,
) -> Option<record::ExecRecord> {
    let ExecStatus::Exited { code } = status else {
        return None;
    };
    let Ok(record) = stdout else {
        return None;
    };
    Some(record::ExecRecord::new(*code, record.clone(), env))
}

/// One corpus-root snapshot ([`fs::domain_snapshot`]) with the dispatch's
/// error shape.
fn snapshot(root: &fs::WorkspaceRoot) -> Result<MerkleRoot, BashError> {
    fs::domain_snapshot(root)
        .map(|(_, r)| r)
        .map_err(|e| BashError::Root {
            reason: e.to_string(),
        })
}
