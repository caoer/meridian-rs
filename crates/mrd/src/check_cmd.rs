//! `mrd check` — the pure READ validity verb (U2.10; d2 §3 check).
//!
//! ```text
//! mrd check [--core] [--staged] [--commit-gate] [--json]
//! ```
//!
//! Runs the convention-free CORE (layer 0) over the resolved workspace: observe
//! every claim against the current tree, then read the PIN PLANE — the pin verdicts
//! and the anchoring state of every pinned blob. `status = freshness, check =
//! validity` — this answers "what lies?", writing nothing and minting no receipt.
//!
//! # WRITE HISTORY IS NOT ASSESSED — the LAW, said out loud (ZT, 2026-08-03)
//! Verbatim: *"Engine does not have memory. It should not have. History is pin to
//! git when we lock. Anything between locks is not history."*
//!
//! This verb used to date the receipt journal against the live tree and recompute
//! chain continuity. The journal is deleted and **none of it is rebuilt as check
//! memory** — not because the power was lost, but because it was never this
//! engine's to hold. `check` answers **at-rest / at-touch truth: does the world
//! still match the pins.** Archaeology lives in git; attribution lives in
//! transcript JSONL.
//!
//! An out-of-band edit followed by a governed write is not detected here, and that
//! is **outside the engine's domain by design** rather than a tolerated hole.
//!
//! One consequence IS disclosed, because the green narrowed: a corpus that once
//! answered `grey(cannot-assess)` ("I cannot date your write history") now answers
//! green ("the world still matches the pins"). **The claim behind that green is
//! smaller, not stronger.** So every face carries a `write_history: not-assessed`
//! line WITH its reason — the reason is what makes it read as design rather than as
//! a gap. Without it a reader carries the old, wider green forward.
//!
//! **U14 — the planes fail independently.** A lock that arrives by clone or pull
//! while its source moved, and a blob no ref reaches, are facts no journal row ever
//! carried — which is why the pin plane outlived the journal plane. Pin colours come
//! from `view::walk::lock_pin_colors`, the SAME call `mrd status`'s lock axis makes
//! over the SAME corpus build, so the planes agree by construction, not coincidence.
//!
//! # THE INTERVAL THIS VERB SPANS (S3-R29)
//! Two intervals, both named in every answer:
//!
//! - **`worktree`** — the bytes on disk. Always assessed.
//! - **`staged`** — the bytes the git INDEX carries, assessed whenever it carries
//!   anything the worktree does not, because **that is the interval a commit
//!   records**.
//!
//! **F1 — why the second one exists.** `domain_snapshot` reads the worktree; git
//! commits the index. Forge a pinned section, `git add` it, restore the exact
//! governed bytes to the worktree, and the shipped verb answered green, exit 0 —
//! over bytes no commit would record — while `git show HEAD:<page>` came back
//! carrying the forgery. `git add -p`, `git commit <pathspec>`, `git stash` and any
//! concurrent writer between `git add` and hook fire reach the same gap.
//!
//! The interval is now the ONLY thing that separates the two passes. The staged
//! interval once asked a WEAKER question than the worktree one — dated against the
//! record rather than its own last row, so a legitimately partial stage was not a
//! false red — and that difference was entirely journal. Both passes now run the
//! same reads over different bytes.
//!
//! The exit is **worst-of across both intervals**, and every refusal names the
//! interval it came from: *"the bytes on your disk are fine"* and *"the bytes you
//! are about to commit are not"* are different instructions to an operator.
//!
//! `--core` names layer 0 explicitly (the default today). The armed layer-1
//! evaluation is the `check` engine surface the door mounts (U4.2) — its
//! change-framing over a whole tree lands with that door, not this verb.
//!
//! # THE QUESTION THIS VERB IS ASKED (`--commit-gate`)
//! The interval says WHICH BYTES an answer covers. `--commit-gate` says WHICH
//! QUESTION is put to them, and the two are independent.
//!
//! Unscoped, the verb reports every plane it reads, worst-of across both intervals.
//! `--commit-gate` narrows the exit to ONE interval — the one a commit records —
//! because a finding from the worktree would otherwise swamp a clean answer about
//! the bytes being committed.
//!
//! **What it gates on is now the pin plane, and only that.** The flag once rested on
//! two planes: the journal-derived `Accounted` (was this interval's journal a
//! truthful prefix of the record, and did its tree fold to a root some governed
//! write produced) and the pin plane. The first died with the journal. The passing
//! word changed with it — `accounted` and `accounted(unvouched-record)` both
//! asserted something about a RECORD that no longer exists, so the word is now
//! `pins-hold`, which names the plane that actually answered.
//!
//! # THE FAIL-CLOSED READING MOVED FROM DEFAULT TO OPT-IN (`--require-pins`)
//! A corpus that declares NO pin now PASSES the gate. It used to refuse, and the
//! refusal was **journal mechanics rather than independent doctrine**: an empty
//! record meant no baseline, no baseline meant a grey write-history plane, and the
//! grey gated the exit. That antecedent died with the journal, so what was removed
//! is a mechanism with no input left — not a safety principle.
//!
//! *"Unknown is not clean"* is untouched where it applies. It protects the case
//! where the gate CANNOT ASSESS something it claims to gate — a grey pin, an
//! unaskable object store — and both **still fail closed**. Zero pins is not that
//! case: nothing is unknown because nothing was asked, and over zero pins *"does the
//! world still match the pins"* is vacuously true.
//!
//! **But a shell script cannot read a disclosure line.** `write_history:
//! not-assessed` and `pin_coverage: 0` are legible to a human and invisible to
//! `if [ $? -ne 0 ]`, so a caller that wants no-coverage to mean REFUSE says so with
//! `--require-pins` and gets it in the exit code, under its own word
//! (`no-pin-coverage`, never grey's — grey means a question was put and could not be
//! answered, and here none was put).
//!
//! It is OPT-IN, and that is the load-bearing half: a fail-closed default on pinless
//! workspaces would turn this gate into a coverage mandate nobody ruled, and make it
//! un-adoptable on every vault that has not started pinning. **The shipped fence
//! (`mrd skill hook`) runs the bare gate and therefore inherits the permissive
//! default** — deliberately, and a checkout that wants strictness edits its own hook.
//!
//! Read-only. Exit triad (§4 preamble):
//! - **0** — green: every claim converged, every pin holds, every pinned blob is
//!   anchored. Under `--commit-gate`: the interval a commit records holds its pins.
//! - **1** — a finding: a drifted claim, a red pin, or an orphaned blob. A check
//!   finding, never a door refusal (refusal-amendment). **Grey rides this leg too**
//!   (S3-R5/S3-R8, spelled by S3-R6): a grey pin or an object store that could not be
//!   asked refuses `grey(cannot-assess)`. Unknown is not clean, and a hook that
//!   rejects on non-zero must reject what nobody could vouch for. The triad stays
//!   CLOSED: no fourth code. The exit answers "may this proceed?" (red and grey both
//!   say no); the reason word, distinct on both faces, says why.
//! - **2** — bad invocation, or an unreadable workspace.
//!
//! `--commit-gate` keeps all three meanings exactly. Only the question changes, so a
//! caller that branches on the code alone still reads a code that means what it
//! always meant.
//!
//! # THE FENCE LINE — A READING OF THE CHECKOUT, BESIDE THE VERDICT (row 21)
//! Every line above is a proposition about the corpus's bytes or their write
//! history. The `fence:` line is not: it is a proposition about the **local
//! checkout's configuration**, a different subject on a different axis, and it
//! **never touches the exit code** ([`Fence`]).
//!
//! `$GIT_DIR/hooks` is never a tracked path, so no clone, fetch or pull can carry
//! the fence. Fence coverage is therefore **per-checkout and opt-in, permanently**,
//! and a fresh clone being unfenced is a SUPPORTED state — one a user needs told,
//! not a finding. **The defect this line closes is the SILENCE, not the absence**;
//! colouring `check` on it would make governance unreachable in every fresh clone,
//! which is the permanent-fact-as-per-commit-verdict defect above wearing a new
//! sign.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use std::path::Path;

use check::{CoreReport, GREY_CANNOT_ASSESS, PinPlane, PinRow, WRITE_HISTORY_NOT_ASSESSED};
use model::Document;
use receipt::anchor::{ObjectAnchor, PENDING_ANCHOR_TTL};
use serde_json::{Value, json};

use crate::hook;
use crate::{Fail, Format, current_dir};

/// The finding leg of the triad: the invocation was well-formed, the core found a
/// lie (a chain break or a foreign edit).
const EXIT_FINDING: u8 = 1;

/// Run `mrd check [--core] [--json]`: resolve the workspace and run the layer-0 core, printing
/// the verdict. Errors [`Fail`] exit 2 on a bad invocation or an unreadable workspace/journal;
/// exit 1 when the core reddens (chain break or foreign edit).
///
///
///
pub(crate) fn dispatch(args: &[String]) -> Result<(), Fail> {
    let parsed = Check::parse(args)?;
    let cwd = current_dir()?;
    let resolved = crate::resolve::resolve_runtime(&cwd).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace for {}: {e}",
            cwd.display()
        ))
    })?;
    let canonical = workspace::canonicalize(&resolved.workspace).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace {} ({e})",
            resolved.workspace.display()
        ))
    })?;
    let root = fs::WorkspaceRoot(canonical.clone());

    // ONE read of the worktree, whose bytes feed the fold AND the corpus — the reason
    // `domain_snapshot` returns both. A second read would let the two planes describe two
    // different worktrees.
    let domain = fs::domain::Domain::load(&root)
        .map_err(|e| Fail::tool(format!("cannot read the hash domain: {e}")))?;
    let (worktree_files, _worktree_fold) = fs::domain_snapshot(&root)
        .map_err(|e| Fail::tool(format!("cannot read the corpus: {e}")))?;

    // THE SECOND INTERVAL (F1), and only when the caller asked the question it answers. `git`
    // reports what the INDEX carries wherever it differs from the worktree; absent divergence the
    // two intervals coincide and one pass answers both.
    //
    let interval = if parsed.staged {
        staged_interval(&root, &domain, &worktree_files)
    } else {
        Interval::NotAsked
    };

    // W5 — **the corpora are built before the mount table, because they are what says which roots
    // the table must build.** Each interval's corpus is built exactly once here and then assessed,
    // rather than built inside `assess`: the root set is a question about the corpus, so asking it
    // costs a build, and a build per question would have paid twice for the same bytes.
    //
    let worktree_docs = build_corpus(worktree_files)?;
    let staged_docs = match &interval {
        Interval::Diverges(bytes) => Some(build_corpus(bytes.files.clone())?),
        _ => None,
    };

    // U11/F6 — the REAL mount table, through the one loader `mrd walk` uses. A default (empty)
    // table here is what made the cross-root pin axis answer `grey(unmounted)` for a bound root in
    // all three of its states. W5 — the table is whole; only the CORPORA narrow, to the roots this
    // check's own lock addresses NAME.
    //
    //
    //
    //
    //
    //
    //
    //
    let mut needed = crate::walk_cmd::lock_addressed_roots(&worktree_docs);
    if let Some(docs) = &staged_docs {
        needed.append(&mut crate::walk_cmd::lock_addressed_roots(docs));
    }
    let mounts = crate::walk_cmd::load_mounts_for(&needed);

    let worktree = assess(&root, &mounts, &worktree_docs);

    let staged = match (&interval, &staged_docs) {
        (Interval::Diverges(bytes), Some(docs)) => Some(Assessed {
            paths: bytes.paths.clone(),
            report: assess(&root, &mounts, docs).report,
        }),
        _ => None,
    };

    let gate = parsed
        .commit_gate
        .then(|| build_gate(&interval, &worktree, staged.as_ref(), parsed.require_pins));

    // **The CHECKOUT's fence coverage — read here, reported below, and reachable from no exit path
    // in this function** (row 21). It is deliberately taken on every invocation, gated and ungated
    // alike: a reading whose presence depended on a flag would be a reading the operator has to
    // know to ask for, which is the silence this line closes.
    //
    let fence = observe_fence(&canonical);

    emit(
        parsed.format,
        &canonical,
        &worktree.report,
        &interval,
        staged.as_ref(),
        gate.as_ref(),
        &fence,
    );

    // FAIL CLOSED on an interval that was ASKED FOR and could not be read. The caller asked what a
    // commit would record; degrading silently to the worktree answer is the F1 shape again with
    // one more step in front of it — a true statement about the wrong bytes.
    //
    if let Interval::CannotAsk(detail) = &interval {
        return Err(Fail::with_code(
            EXIT_FINDING,
            format!(
                "check refuses ({STAGED}): {GREY_CANNOT_ASSESS} — {detail}; the interval a \
                 commit records could not be read, and a commit nobody could vouch for is not \
                 a verified one"
            ),
        ));
    }

    // **The scoped exit, and it reads ONE interval.** The worst-of below is the corpus-wide
    // question's answer and stays exactly as it was for every caller that asks it; a gated run
    // never reaches it.
    if let Some(gate) = gate.as_ref() {
        return gate.exit();
    }

    worst_of_exit(&worktree.report, staged.as_ref())
}

/// **The corpus-wide question's exit: worst-of ACROSS INTERVALS**, then worst-of
/// within one — red is reported first, grey next, green last.
///
/// Both refuse on the SAME leg (S3-R6: the exit code answers only "may this
/// proceed?"; no fourth code), so the prefix is the same verb and the REASON WORD
/// in each line is what tells a finding from an absence of evidence. Saying "found
/// a lie" over a pending-anchor blob would be a claim wider than the evidence —
/// nothing lied, a blob is simply held by nothing durable.
///
/// The STAGED interval refuses on the same leg as the worktree one and says which
/// interval it is: a refusal a reader cannot locate is one they cannot act on, and
/// "the bytes on your disk are fine" plus "the bytes you are about to commit are
/// not" are different instructions.
///
/// **`--commit-gate` never reaches here.** This is the whole-corpus claim, and
/// spending it on a per-commit question is the defect that flag exists to close.
///
/// # Errors
/// [`Fail`] exit 1 on the worst finding across the assessed intervals.
fn worst_of_exit(worktree: &CoreReport, staged: Option<&Assessed>) -> Result<(), Fail> {
    let mut intervals: Vec<(&str, &CoreReport)> = vec![(WORKTREE, worktree)];
    if let Some(staged) = staged {
        intervals.push((STAGED, &staged.report));
    }
    for summarise in [CoreReport::red_summary, CoreReport::grey_summary] {
        if let Some((label, summary)) = intervals
            .iter()
            .find_map(|(label, report)| summarise(report).map(|s| (*label, s)))
        {
            return Err(Fail::with_code(EXIT_FINDING, refusal_list(label, &summary)));
        }
    }
    Ok(())
}

/// The refusal line for a summary that carries N findings: a COUNT, then one
/// finding per line, numbered.
///
/// `red_summary`/`grey_summary` already build exactly this list, one finding per
/// `\n`. The old spelling re-serialized it with `.replace('\n', "; ")`, which
/// scaled into an unwrapped wall the moment a corpus carried more than the two
/// findings this repo produces — and stacked a third separator (`;`) on top of
/// the `:` and ` — ` the findings already use, so neither an eye nor a script
/// could tell where one ended (issue-04). The count is what tells a reader they
/// have reached the end of the list rather than the end of their terminal.
fn refusal_list(label: &str, summary: &str) -> String {
    let findings: Vec<&str> = summary.lines().collect();
    let noun = if findings.len() == 1 {
        "finding"
    } else {
        "findings"
    };
    let mut out = format!("check refuses ({label}) — {} {noun}:", findings.len());
    for (i, finding) in findings.iter().enumerate() {
        let _ = write!(out, "\n  {}. {finding}", i + 1);
    }
    out
}

/// **Which interval a commit records**, and the scoped question put to it. When the index
/// diverges that is the staged bytes; when it coincides, or there is no repository at all, the
/// worktree IS that interval and its own render says so. Either way ONE interval answers, and
/// the record it is read against is always the WORKTREE's journal — the most complete one the
/// engine has, and the one the worktree pass separately validates in the same run.
///
///
fn build_gate<'a>(
    interval: &'a Interval,
    worktree: &'a Assessed,
    staged: Option<&'a Assessed>,
    require_pins: bool,
) -> Gate<'a> {
    let (label, pins) = match (interval, staged) {
        (Interval::Diverges(_), Some(staged)) => (STAGED, &staged.report.pins),
        _ => (WORKTREE, &worktree.report.pins),
    };
    Gate {
        label,
        pins,
        require_pins,
    }
}

/// Print the verdict on the caller's chosen face. One writer for both, so a face
/// can never be given a reading the other was not.
fn emit(
    format: Format,
    workspace: &Path,
    worktree: &CoreReport,
    interval: &Interval,
    staged: Option<&Assessed>,
    gate: Option<&Gate<'_>>,
    fence: &Fence,
) {
    match format {
        Format::Json => {
            let value = to_json(workspace, worktree, interval, staged, gate, fence);
            println!("{}", serde_json::to_string_pretty(&value).expect("json"));
        }
        Format::Human => print!(
            "{}",
            render_human(workspace, worktree, interval, staged, gate, fence)
        ),
    }
}

/// The interval a worktree read spans — the bytes on disk.
const WORKTREE: &str = "worktree";

/// The interval a COMMIT spans — the bytes the index carries. One name for it, in
/// the human render, the `--json` face and every refusal, so a reader who learns
/// the word once can find it everywhere (S3-R6).
const STAGED: &str = "staged";

/// The finding colour, spelled ONCE — the chain line, the pin lines and the gate's
/// verdict word all mean the same thing by it (S3-R6/S3-R59).
const RED: &str = "RED";

/// The gated pass. Names the plane that actually answered, and cannot be misread as a claim
/// about write history. **It replaced `accounted` / `accounted(unvouched-record)`, and the
/// replacement is the honesty law applied to VOCABULARY rather than only to types.
///
///
///
///
///
///
const PINS_HOLD: &str = "pins-hold";

/// **What `--commit-gate` reads.** One interval decides the exit — that is still the whole law
/// Worst-of ACROSS intervals is right for the unscoped question, which is a claim about the
/// corpus. It is wrong for this one: a finding from the worktree interval would swamp a clean
/// answer about the bytes a commit records. So the gate names ONE interval — the one a commit
/// records — and reads it.
///
///
///
///
///
///
///
///
///
///
struct Gate<'a> {
    /// Which interval the exit reads. Named in the render and in every refusal,
    /// because a refusal a reader cannot locate is one they cannot act on.
    label: &'a str,
    /// That interval's pin plane — now the ONLY gated plane. A pin is a claim about the bytes being
    /// committed, so it belongs to the interval, not to the history.
    pins: &'a PinPlane,
    /// **The caller asked for fail-closed-on-no-coverage** (`--require-pins`).
    /// Off by default; see [`NO_COVERAGE`] for the whole reasoning.
    require_pins: bool,
}

/// The refusal word for a corpus that declares no pin at all, under
/// `--require-pins`.
///
/// **Distinct from [`GREY_CANNOT_ASSESS`] on purpose.** Grey means *a question was
/// put and could not be answered*; this is not that. Every question that was put
/// was answered — there simply were none. Spelling it grey would say the gate tried
/// and failed, which is the one thing that did not happen here.
const NO_COVERAGE: &str = "no-pin-coverage";

impl Gate<'_> {
    /// May this commit proceed? An unread plane is not a clean one — unknown is not
    /// clean.
    ///
    /// # Zero pins is VACUOUS TRUTH, not unknown — and that is why the default passes
    /// The fail-closed doctrine (*"unknown is not clean"*) protects the case where
    /// the gate CANNOT ASSESS something it claims to gate. That case is a grey pin
    /// or an unaskable object store, and it keeps failing closed here, untouched.
    ///
    /// A corpus with no pins has declared nothing. The gate's whole question is
    /// *"does the world still match the pins"*, and over zero pins that is
    /// vacuously yes. Nothing is unknown because nothing was asked.
    ///
    /// This USED to exit 1, and the reason was journal mechanics rather than
    /// independent doctrine: an empty record meant no baseline, no baseline meant a
    /// grey write-history plane, and the grey gated. That antecedent died with the
    /// journal by ruling, so the pass is not a safety principle being overturned —
    /// it is a mechanism with no input left being removed.
    ///
    /// # But the doctrine survives as the CALLER'S choice
    /// A shell script cannot read a disclosure line. `write_history: not-assessed`
    /// and `pin_coverage: 0` are legible to a human and invisible to `if [ $? -ne 0
    /// ]`, so a caller who wants *"no coverage ⇒ refuse"* must be able to say it in
    /// the exit code. `--require-pins` is that, and it is OPT-IN: a fail-closed
    /// default on pinless workspaces would silently turn this gate into a coverage
    /// mandate nobody ruled, and make it un-adoptable on every vault that has not
    /// started pinning. Doctrine preserved as a choice; adoption preserved as the
    /// default.
    fn permits(&self) -> bool {
        !self.pins.is_red() && !self.pins.cannot_assess() && !self.uncovered()
    }

    /// The corpus declares no pin, and the caller asked to refuse exactly that.
    fn uncovered(&self) -> bool {
        self.require_pins && self.pins.declared == 0
    }

    /// The verdict WORD — what was read, and what it said.
    fn word(&self) -> &'static str {
        if self.permits() {
            PINS_HOLD
        } else if self.pins.is_red() {
            RED
        } else if self.uncovered() {
            NO_COVERAGE
        } else {
            GREY_CANNOT_ASSESS
        }
    }

    /// Why — the half of the answer the exit code cannot carry.
    fn detail(&self) -> String {
        if self.permits() {
            return "every pin in the interval holds and every pinned blob is durably \
                 anchored; write history is not assessed — the engine keeps no memory by design"
                .to_string();
        }
        // Before the pin findings, because it is not one: nothing is wrong with
        // this corpus, there is simply nothing in it to be right or wrong. Saying
        // so plainly is the difference between a caller learning it has no
        // coverage and a caller believing it has a defect.
        if self.uncovered() {
            return "this corpus declares no pin at all, so the gate had nothing to read — \
                    you asked for --require-pins, which refuses exactly that. Without the \
                    flag this is a PASS: over zero pins, `does the world still match the \
                    pins` is vacuously true"
                .to_string();
        }
        if let Some(pin) = self.pins.red.first().or_else(|| self.pins.grey.first()) {
            return format!("pin: {}", pin_line(pin));
        }
        if let Some(orphan) = self.pins.orphaned.first() {
            return format!(
                "{}: {} objects.{} ({}) is reachable from no ref, and the file hashes to {} now \
                 — no commit will anchor it",
                orphan.state.word(),
                orphan.src_path,
                orphan.key,
                orphan.blob_sha,
                orphan.live
            );
        }
        self.pins.cannot_ask.clone().unwrap_or_default()
    }

    /// The scoped exit — the closed triad, over one interval. `0` the pins hold;
    /// `1` they do not, or could not be assessed; `2` never comes from here, because
    /// a bad invocation is refused before any interval is read.
    ///
    /// # Errors
    /// [`Fail`] exit 1 when the interval's pin plane refuses or cannot be read.
    fn exit(&self) -> Result<(), Fail> {
        if self.permits() {
            return Ok(());
        }
        Err(Fail::with_code(
            EXIT_FINDING,
            format!(
                "check refuses ({}): {} — {}",
                self.label,
                self.word(),
                self.detail()
            ),
        ))
    }
}

// the fence line — the checkout, not the corpus (row 21)
//
//

/// The clause every fence line carries, spelled ONCE so the two faces cannot drift
/// apart on the one claim this whole reading rests on.
const FENCE_REPORTED: &str = "REPORTED, never gated on — fence coverage is a property of this \
                              local checkout and not of the corpus, so this line does not move \
                              check's exit";

/// **What the local CHECKOUT's fence looks like — a reading beside the verdict, never part of
/// it** (row 21). It is not a fourth proposition The claims-realised findings and the pin plane
/// are propositions about the corpus's bytes and their write history. Fence coverage is a
/// proposition about the **local checkout's configuration**: a different subject on a different
/// axis, which never competed for the exit code, so the closed triad above is not under
/// pressure here and needs no defending.
///
///
///
///
///
///
///
///
///
///
///
///
///
///
///
///
struct Fence {
    /// The one word for this checkout: [`hook::Coverage::word`] when the root can carry a fence,
    /// [`hook::Unfenceable::word`] when it cannot. **Never re-spelled here** (S3-R6) — an operator
    /// who learned these words from `mrd skill hook`'s document reads the same ones off this face.
    ///
    word: &'static str,
    /// What was observed, and what can be done about it.
    teaching: String,
    /// **The door plane, or `None` when this root has none.** Three doors can
    /// disagree, so the set's one word is not the whole reading — `installed-partial`
    /// names a disagreement without saying where it is, and a reader who cannot
    /// locate it cannot act on it.
    ///
    /// `None` rather than an empty list for a root the fence cannot reach: a
    /// submodule or a non-repository has no hook directory to read, which is not
    /// the same fact as a hook directory read and found empty.
    doors: Option<Vec<FenceDoor>>,
    /// How many of those doors carry a fence this engine's line wrote — the COVERAGE axis, and
    /// **blind to currency by construction** ([`hook::Coverage::fenced_doors`]). A door standing at
    /// an older or a newer generation is counted here; whether it is current is [`Fence::word`]'s
    /// claim, never this one's.
    ///
    fenced: usize,
}

/// One door of the install set, as this face reports it.
struct FenceDoor {
    /// The hook's git name — one of [`hook::FENCED_HOOKS`].
    name: &'static str,
    /// This door's own state word, from [`hook::Door::word`].
    word: &'static str,
    /// The generation THIS file declares, never the asking engine's.
    version: Option<u32>,
}

/// Read the checkout's fence state. **It cannot fail into the exit.** Every outcome of the
/// survey — a reachable root, or one the fence cannot reach — is a [`Fence`] to report, so no
/// branch here produces a value a caller could turn into a [`Fail`].
///
///
fn observe_fence(workspace: &Path) -> Fence {
    match hook::status(workspace) {
        Ok(coverage) => {
            let fenced = coverage.fenced_doors();
            Fence {
                word: coverage.word(),
                // `Coverage::teaching` is silent on the two states where the set agrees with itself, because a
                // reader who went looking for the door set has it in front of them. **This reader did not** —
                // the line is unasked-for, and `absent` is precisely the state row 21 exists to stop being
                // silent about.
                //
                teaching: coverage
                    .teaching()
                    .unwrap_or_else(|| agreed_teaching(fenced)),
                doors: Some(
                    coverage
                        .doors
                        .iter()
                        .map(|door| FenceDoor {
                            name: door.name,
                            word: door.word(),
                            version: door.version(),
                        })
                        .collect(),
                ),
                fenced,
            }
        }
        // A root the fence cannot reach is reported with its OBSERVED reason word and its teaching —
        // never as a bare absence, and never as a refusal.
        Err(refusal) => Fence {
            word: refusal.word(),
            teaching: refusal.teaching(),
            doors: None,
            fenced: 0,
        },
    }
}

/// The two states [`hook::Coverage::teaching`] leaves unworded, said here because
/// this face speaks to a reader who did not ask for a fence report.
fn agreed_teaching(fenced: usize) -> String {
    // The count is already in the line, so neither of these repeats it — what the
    // count cannot say is WHY, and that is what each says instead.
    if fenced == 0 {
        "`$GIT_DIR/hooks` is never a tracked path, so no clone, fetch or pull carries a fence \
         and a fresh checkout is unfenced BY DESIGN — `mrd skill hook` emits what to place to \
         fence this one, per checkout and opt-in"
            .to_owned()
    } else {
        "this checkout is fully fenced, at the generation this engine writes".to_owned()
    }
}

/// The fence line(s) on the human face: the checkout's coverage, then the doors when it has
/// any. **Two lines rather than one**, for the reason [`Fence::doors`] gives: the set's word
/// cannot carry which door disagrees, and the per-door line can. EACH CLAUSE NAMES ITS OWN AXIS
/// The first line composes **two instruments**: the count is COVERAGE — how many doors this
/// engine's line wrote — and the set word is CURRENCY — whether what they carry is the
/// generation this engine writes.
///
///
///
///
///
///
///
///
///
///
///
///
///
///
///
///
fn fence_lines(fence: &Fence) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    // The teachings come from two different types and only some of them end in a full stop;
    // trimming it here is what keeps one render grammar over both.
    let teaching = fence.teaching.trim_end_matches('.');
    match &fence.doors {
        Some(doors) => {
            let _ = writeln!(
                out,
                "  fence: {} — {} of {} doors carry this engine's fence marker, at any \
                 generation; {teaching} · {FENCE_REPORTED}",
                fence.word,
                fence.fenced,
                doors.len()
            );
            let _ = writeln!(
                out,
                "  fence doors: {}",
                doors
                    .iter()
                    .map(|door| format!("{} {}", door.name, door.word))
                    .collect::<Vec<_>>()
                    .join(" · ")
            );
        }
        // No door plane, so no door line. The same law the `--json` face states at `interval_json`: an
        // absence is not an empty reading of something.
        None => {
            let _ = writeln!(
                out,
                "  fence: {} — {teaching} · {FENCE_REPORTED}",
                fence.word
            );
        }
    }
    out
}

/// The fence block on the `--json` face.
///
/// **The door-plane keys are ABSENT when there is no door plane** — this face's own
/// law, stated at [`interval_json`]: *an absent field reads as "not checked"*. A
/// `null` would say the doors WERE read and came back as nothing, which for a
/// submodule or a non-repository is a different fact and a false one.
fn fence_json(fence: &Fence) -> Value {
    let mut value = json!({
        "state": fence.word,
        "fenceable": fence.doors.is_some(),
        "teaching": fence.teaching,
        // What THIS ENGINE writes, beside what the files declare (per door below). A verdict that does
        // not disclose its judge cannot be checked by a third party, and in a version skew both
        // participants are inside it.
        "engine_version": hook::FENCE_VERSION,
        // The card's central claim, machine-readable: a consumer must be able to
        // read off this face that the block did not decide the exit.
        "gates_the_exit": false,
    });
    if let Some(doors) = &fence.doors {
        value["doors"] = doors
            .iter()
            .map(|door| {
                json!({
                    "name": door.name,
                    "state": door.word,
                    "fence_version": door.version,
                })
            })
            .collect::<Vec<Value>>()
            .into();
        // The POPULATION beside the reading (S3-R23(5)): "one door fenced" means
        // one thing out of three and something else out of one.
        value["fenced_doors"] = json!(fence.fenced);
        value["total_doors"] = json!(doors.len());
    }
    value
}

/// One interval's verdict, with the paths that made it a separate interval.
struct Assessed {
    /// The domain paths whose index bytes differ from the worktree's — empty for
    /// the worktree interval itself.
    paths: Vec<String>,
    report: CoreReport,
}

/// **What was asked about the second interval, and what came back** — the state the render
/// states and the exit fails closed on. Four outcomes, each a DIFFERENT fact about this run, so
/// none of them is rendered as another's silence (the same law the `pins:`/`anchoring:` split
/// runs on): not asked · asked and there is no index · asked and the index adds nothing · asked
/// and it carries other bytes.
///
///
enum Interval {
    /// `--staged` was not passed: the index was never read, and this answer is about the worktree
    /// only. **Said out loud** — a reader must not take a worktree green for a statement about
    /// their commit.
    NotAsked,
    /// Asked, and this workspace is not a git repository, so no index exists to record anything.
    /// Marker-beats-git makes that a supported state, not a degradation.
    ///
    NoRepository,
    /// Asked, and the index carries nothing the worktree does not — one pass
    /// answers both intervals. **A FACT about this run**, not an absence.
    Coincides,
    /// Asked, and the index carries other bytes for these paths.
    Diverges(StagedBytes),
    /// Asked INSIDE a repository, and git could not be asked. Refuses.
    CannotAsk(String),
}

/// The staged interval's own bytes: the overlaid snapshot, its fold, its journal,
/// and the paths that diverge.
struct StagedBytes {
    paths: Vec<String>,
    files: fs::DomainFiles,
}

/// Assess ONE interval: build its corpus, colour its pins through the one computer with the
/// real mount table, and run the layer-0 core over ITS bytes. **There is no longer a separate
/// staged assessor.** `assess_staged` existed only to date the staged interval against the
/// record instead of its own last row — a journal question. With the journal deleted the two
/// intervals differ only in which bytes built the corpus, which is this function's argument.
///
///
fn assess(
    root: &fs::WorkspaceRoot,
    mounts: &crate::walk_cmd::Mounts,
    docs: &BTreeMap<String, Document>,
) -> Assessed {
    let corpus = mounts.rooted(docs);
    let pins = pin_rows(&corpus, mounts.set());
    Assessed {
        paths: Vec::new(),
        report: check::core_of(root, docs, &pins),
    }
}

/// One interval's bytes, parsed into the corpus both the root scan and the assessment read.
/// **Split out of [`assess`] by W5**, which needs the corpus in hand *before* the mount table
/// exists — the roots worth building are read off
///
///
///
///
///
fn build_corpus(files: fs::DomainFiles) -> Result<BTreeMap<String, Document>, Fail> {
    let (_index, docs) =
        fs::build_corpus(files).map_err(|e| Fail::tool(format!("cannot build the corpus: {e}")))?;
    Ok(docs)
}

/// **The interval the commit spans** (F1): the worktree snapshot with the INDEX's
/// bytes overlaid, or `None` when the index carries nothing the worktree does not.
///
/// # Why this is not the same question as "is my worktree clean"
/// `git` commits the index. A snapshot of the worktree is a snapshot of a
/// different interval, and the two part company on `git add` + edit,
/// `git add -p`, `git commit <pathspec>`, `git stash`, and any concurrent writer
/// between `git add` and hook fire. Staging a forged file and restoring the
/// worktree left the shipped fence reading bytes no commit would record: green,
/// exit 0, forged bytes in history.
///
/// The reserved journal is handled here rather than in the fold because it is
/// root-EXCLUDED from the hash domain by named law — its bytes never enter a
/// merkle root, so the overlay would drop them, and a staged journal forgery
/// would be assessed against the worktree's journal. It is the one file whose
/// interval has to be picked out by hand.
///
/// A workspace that is not a git repository has ONE interval, and that is a
/// supported state (marker-beats-git), not a degradation: there is no index, so
/// nothing can diverge from it.
fn staged_interval(
    root: &fs::WorkspaceRoot,
    domain: &fs::domain::Domain,
    worktree_files: &fs::DomainFiles,
) -> Interval {
    let repo = git::Repo::at(&root.0);
    let divergence = match repo.staged_divergence() {
        Ok(divergence) => divergence,
        // No repository, no index, nothing a commit here could record.
        Err(git::GitFail::NotARepo { .. }) => return Interval::NoRepository,
        // Inside a repository and git could not answer. NOT the worktree answer wearing a wider label
        // — the caller asked about the commit's interval and this run cannot speak about it.
        //
        Err(other) => return Interval::CannotAsk(other.to_string()),
    };
    if divergence.is_empty() {
        return Interval::Coincides;
    }

    // The reserved journal used to be picked out by hand here: it was root-EXCLUDED from the hash
    // domain, so the overlay dropped it and a staged journal forgery would have been assessed
    // against the worktree's copy. The carve-out is gone with the journal — there is no reserved
    // page and nothing to hand-pick.
    let (files, _fold) = fs::overlay_snapshot(worktree_files, &divergence, domain);
    let paths = divergence
        .iter()
        .filter(|(rel, _)| domain.contains(Path::new(rel)))
        .map(|(rel, _)| rel.clone())
        .collect::<Vec<_>>();
    if paths.is_empty() {
        // Everything that diverges is outside the hash domain — code, assets, a lock file. Neither
        // interval reads those, so there is nothing here a second assessment could say.
        //
        return Interval::Coincides;
    }
    Interval::Diverges(StagedBytes { paths, files })
}

/// Colour every `meridian-lock` pin in the corpus through **the one pin computer** —
/// `view::walk::lock_pin_colors_rooted`, which is exactly what `mrd status`'s lock axis reads
/// and what colours a `mrd walk` listing. This is the seam that makes the three planes agree BY
/// CONSTRUCTION. A `check` that re-derived pin colours would be a second implementation of
/// corpus index → ref resolution → selector → fingerprint compare, and a second copy of that
/// chain is how the pin plane and the decoration plane once came to hash two different
/// documents for one ref. There is one computer here, not three that happen to match today.
///
///
///
///
///
///
///
///
///
///
///
///
///
///
///
///
fn pin_rows(corpus: &model::RootedCorpus<'_>, mounts: &addr::MountSet) -> Vec<PinRow> {
    view::walk::lock_pin_colors_rooted(corpus, mounts)
        .into_iter()
        .map(|pin| PinRow {
            src_path: pin.src_path,
            declared_ref: pin.declared_ref,
            label: view::walk::color_label(&pin.color),
            color: pin.color,
        })
        .collect()
}

/// The parsed `check` invocation: the output format (the `--core` flag names
/// layer 0 explicitly, the default today, so it carries no extra state).
#[derive(Debug)]
struct Check {
    format: Format,
    /// Also assess **the interval a commit would record** — the git index (F1).
    ///
    /// Off by default, and that default is the honest one: outside a commit the
    /// index is a staging area mid-edit, not a claim about anything. A governed
    /// write that is not staged yet is the ordinary state of every working
    /// repository, and assessing it as "what a commit records" turns *"I have
    /// written and not staged"* into a refusal. **The pre-commit fence passes this
    /// flag, because at that instant the index IS what is being committed.**
    staged: bool,
    /// Ask the **per-commit** question instead of the corpus-wide one, and gate the
    /// exit on the interval a commit records alone (§ THE QUESTION THIS VERB IS
    /// ASKED).
    ///
    /// **Implies [`Check::staged`]**, because the interval it gates on is the one
    /// the index carries and there is no coherent commit gate without it. One flag
    /// for the fence to pass, not two that can be passed apart and mean nothing.
    commit_gate: bool,
    /// **Refuse a corpus that declares NO PIN** (`--require-pins`), turning the
    /// gate's vacuous pass into a refusal for callers that want coverage enforced.
    ///
    /// Opt-in, and only meaningful beside [`Check::commit_gate`] — passing it alone
    /// is refused as a bad invocation rather than silently ignored, because a flag
    /// that appears to tighten a gate nobody asked for is worse than no flag.
    /// Reasoning: [`Gate::permits`].
    require_pins: bool,
}

impl Check {
    fn parse(args: &[String]) -> Result<Self, Fail> {
        let mut json = false;
        let mut staged = false;
        let mut commit_gate = false;
        let mut require_pins = false;
        for arg in args {
            match arg.as_str() {
                "--json" => json = true,
                // `--core` names layer 0 explicitly; it is the default today, so it
                // is accepted and needs no separate branch.
                "--core" => {}
                // git's own word for the index, deliberately (`git diff --staged`) — one vocabulary, and never
                // a second spelling for a concept the operator's other tool already named.
                //
                "--staged" => staged = true,
                // The QUESTION, not a second interval: it names the caller — a pre-commit gate — rather than a
                // mechanism, so what it changes is legible from the flag alone.
                //
                "--commit-gate" => {
                    commit_gate = true;
                    staged = true;
                }
                // The STRICTNESS the caller chooses, kept a separate word from the question itself:
                // `--commit-gate` says which question, this says how strict the answer must be. Folding it
                // into the first would have made one flag mean two things and left no way to ask the ordinary
                // question.
                //
                "--require-pins" => require_pins = true,
                flag if flag.starts_with('-') => {
                    return Err(Fail::tool(format!("unknown flag: {flag}")));
                }
                value => {
                    return Err(Fail::tool(format!("unexpected argument: {value}")));
                }
            }
        }
        // Refused rather than ignored: a caller who typed `--require-pins` believes a gate is being
        // tightened, and the one thing a fence's verb must never do is answer confidently about a
        // question it did not ask.
        if require_pins && !commit_gate {
            return Err(Fail::tool(
                "--require-pins tightens --commit-gate and means nothing without it: it says \
                 REFUSE a corpus that declares no pin, and only the commit gate refuses"
                    .to_string(),
            ));
        }
        Ok(Check {
            format: if json { Format::Json } else { Format::Human },
            staged,
            commit_gate,
            require_pins,
        })
    }
}

/// Render the core verdict as a human block: the header, the chain line, the the write-history
/// disclosure, and one line per drifted claim (none at the CLI today). Both detector lines
/// render `grey(cannot-assess)` when the journal cannot date the tree — with no row, or with a
/// last receipt the live root no longer continues. Neither may borrow the word the assessed
/// path earns, and neither may accuse: the mismatch is rendered as the evidence it is.
///
///
fn render_human(
    workspace: &Path,
    worktree: &CoreReport,
    interval: &Interval,
    staged: Option<&Assessed>,
    gate: Option<&Gate<'_>>,
    fence: &Fence,
) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(out, "check core {}", workspace.display());
    let _ = writeln!(out, "  interval: {}", interval_line(interval));
    out.push_str(&render_report(worktree));
    if let Some(staged) = staged {
        let _ = writeln!(
            out,
            "  interval: {STAGED} — the bytes a commit would record for {}",
            staged.paths.join(", ")
        );
        out.push_str(&staged_predicate_line());
        out.push_str(&render_report(&staged.report));
    }
    // The gate block comes LAST and adds to the readings rather than replacing
    // them: the interval reports above are true descriptions of what was read, and
    // this says which one the exit answered for. A render that dropped them would
    // hide the standing break the gate deliberately does not block on.
    if let Some(gate) = gate {
        let _ = writeln!(out, "  commit-gate: {} — {}", gate.word(), gate.detail());
        let _ = writeln!(
            out,
            "  gated on: {} — the interval a commit records, and nothing else; every other \
             reading above is REPORTED and gates nothing",
            gate.label
        );
    }
    // The fence line comes LAST, after every reading that could have decided the
    // exit, because it is the one reading that could not have (row 21).
    out.push_str(&fence_lines(fence));
    out
}

/// **STATE THE INTERVAL WHENEVER YOU STATE THE CHECK** (S3-R29). The line is
/// unconditional, and each of the four states says a different thing: a reader may
/// never have to infer which bytes a verdict rested on, and *"the index agrees"* is
/// a FACT about this run rather than an absence worth omitting.
fn interval_line(interval: &Interval) -> String {
    match interval {
        Interval::NotAsked => format!(
            "{WORKTREE} — the bytes on disk. The git INDEX was not read, so this says nothing \
             about what a commit would record: `mrd check --staged` asks that question"
        ),
        Interval::NoRepository => format!(
            "{WORKTREE} — this workspace is not a git repository, so there is no index and \
             nothing else a commit could record"
        ),
        Interval::Coincides => format!(
            "{WORKTREE} + {STAGED} — the index carries nothing the worktree does not, so one \
             pass answers for both and this IS the interval a commit would record"
        ),
        Interval::CannotAsk(detail) => format!(
            "{WORKTREE} only — {STAGED} could NOT be read ({detail}), and the refusal below is \
             that, not a verdict about your bytes"
        ),
        Interval::Diverges(staged) => format!(
            "{WORKTREE} — the bytes on disk; {} path(s) differ in the index and are assessed \
             separately below",
            staged.paths.len()
        ),
    }
}

/// **The PREDICATE the staged interval asserts** (S3-R29 is about the claim as much
/// as the byte range).
///
/// # THIS LINE USED TO BE FALSE, and it is repaired here rather than left standing
/// It read: *"these bytes were PRODUCED BY A GOVERNED WRITE (they match a receipt in
/// the journal) and the journal being committed is a truthful PREFIX of it"*. Both
/// halves name the journal, both describe reads this verb no longer performs, and
/// the sentence was printed on every staged render — **an assertion of an unobserved
/// property, which is exactly the banned member the honesty law names**, and the same
/// defect `foreign_edit: none` was deleted for.
///
/// It also contradicted this file's own module doc four hundred lines above, which
/// already records that the two passes *"now run the same reads over different
/// bytes"*. The weaker-question framing was entirely journal: the staged pass was
/// dated against the record rather than its own last row, so a legitimately partial
/// stage was not a false red. With the record gone the asymmetry goes with it.
///
/// So the interval is now the ONLY thing separating the two passes, and the line says
/// what it actually asserts: the pin plane, over the bytes a commit records.
fn staged_predicate_line() -> String {
    format!(
        "  {STAGED} asserts: every pin this corpus declares still holds against THESE bytes — \
         the ones a commit would record, which are not necessarily the ones on disk. Write \
         history is not assessed here either: the engine keeps no memory, so this says nothing \
         about how these bytes came to be staged\n"
    )
}

/// One interval's verdict lines — the journal TRACE, the claims, and the pin plane. Shared by
/// both intervals so a reader compares like with like, and so a line can never exist for one
/// interval and not the other.
fn render_report(report: &CoreReport) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();

    // **The disclosure, and it is MANDATORY** (advisor ruling, gate 1 §2). The
    // `chain:` and `foreign_edit:` lines used to stand here. `foreign_edit: none`
    // was the sharpest false green in this file — it read "assessed, nothing found"
    // about a property the verb no longer observes. Deleting the lines silently
    // would leave a reader carrying the old, wider green forward, so the narrowing
    // is STATED instead — with its REASON, so it reads as the law it is (the engine
    // keeps no memory) rather than as a gap the verb is apologising for.
    let _ = writeln!(
        out,
        "  write_history: {WRITE_HISTORY_NOT_ASSESSED} — the engine keeps no memory by design: \
         history is pinned to git at lock, and anything between locks is not history. This verb \
         answers at-rest truth only (does the world still match the pins), so chain continuity \
         and last-receipt-vs-live are not checked here at all — not grey, NOT CHECKED"
    );

    for claim in &report.drifted_claims {
        let _ = writeln!(
            out,
            "  claim not realised: {} — {}",
            claim.selector, claim.detail
        );
    }

    // ── the PIN PLANE (U14) ─────────────────────────────────────────────────
    // Two lines, always both present, because their silences mean different
    // things: `pins:` reads the CLAIM plane (did the content drift) and
    // `anchoring:` reads the RETRIEVAL plane (is the blob durably held). A verb
    // that printed only the failing one would leave a reader unable to tell
    // "assessed and clean" from "never looked".
    let pins = &report.pins;
    if pins.red.is_empty() && pins.grey.is_empty() {
        let _ = writeln!(out, "  pins: green");
    } else {
        for pin in pins.red.iter().chain(&pins.grey) {
            let _ = writeln!(out, "  pins: {}", pin_line(pin));
        }
    }
    // The anchoring THREE-STATE as a reading (GAP A), with its POPULATION beside it (S3-R23(5)):
    // the same empty orphan list means one thing over fifty pinned blobs and something else
    // entirely over none, and a reading that cannot tell them apart is how coverage disappears
    // with nothing failing. THE SIGHT LINE, stated before the reading it bounds (ruling ).
    //
    //
    //
    //
    //
    //
    //
    if !pins.out_of_jurisdiction.is_empty() {
        let _ = writeln!(
            out,
            "  anchoring scope: {} pin{} outside this root's object store, NOT measured here — {}",
            pins.out_of_jurisdiction.len(),
            if pins.out_of_jurisdiction.len() == 1 {
                ""
            } else {
                "s"
            },
            pins.out_of_jurisdiction.join(" · ")
        );
    }
    if let Some(detail) = &pins.cannot_ask {
        let _ = writeln!(out, "  anchoring: {GREY_CANNOT_ASSESS} — {detail}");
    } else if pins.asked() == 0 {
        let _ = writeln!(out, "  anchoring: no pinned objects");
    } else {
        let _ = writeln!(
            out,
            "  anchoring: {} {} · {} {} · {} {}",
            pins.anchored,
            ObjectAnchor::Anchored.word(),
            pins.pending,
            ObjectAnchor::PendingAnchor.word(),
            pins.never,
            ObjectAnchor::NeverAnchored.word()
        );
        if pins.pending > 0 {
            let _ = writeln!(out, "  {PENDING_ANCHOR_TTL}");
        }
        for orphan in &pins.orphaned {
            let _ = writeln!(
                out,
                "  anchoring: {} ORPHANED — {} objects.{} ({}) is reachable from no ref and the \
                 file hashes to {} now, so no commit will anchor it",
                orphan.state.word(),
                orphan.src_path,
                orphan.key,
                orphan.blob_sha,
                orphan.live
            );
        }
    }

    // ── the RUN PLANE (G3) ────────────────────────────────────────────────── Pre-exec receipts
    // with no completion. REPORTED, never gated on — like the fence line, this does not move
    // check's exit. Receipts from before the completion marker can never clear, and a permanent
    // red is how a reader learns to stop reading a plane.
    //
    if let Some(rendered) = check::orphan::render(&report.orphans) {
        let _ = writeln!(out, "{rendered}");
    }
    out
}

/// One pin row as a render line: the page, the ref it declares, and the colour
/// label its ONE computer produced. Never re-spells a reason word.
fn pin_line(pin: &PinRow) -> String {
    if pin.declared_ref.is_empty() {
        format!("{} — {}", pin.label, pin.src_path)
    } else {
        format!("{} — {} → {}", pin.label, pin.src_path, pin.declared_ref)
    }
}

/// The `--json` shape: the workspace plus the core object (the drifted claims), the
/// write-history disclosure, and the top-level `red` verdict.
///
///
///
///
///
///
fn to_json(
    workspace: &Path,
    worktree: &CoreReport,
    interval: &Interval,
    staged: Option<&Assessed>,
    gate: Option<&Gate<'_>>,
    fence: &Fence,
) -> Value {
    let mut value = interval_json(workspace, worktree);
    // **`red` is the VERDICT, so it is worst-of across intervals** — a reader who
    // banks the top-level flag must not be told the workspace is honest because
    // the bytes on disk are, while the ones being committed are not. The
    // per-interval detail stays under `core`/`pins` (worktree) and
    // `interval.staged` — additive keys, so a consumer of the shipped shape reads
    // exactly what it read before.
    let refuses_staged = staged.is_some_and(|s| s.report.is_red());
    value["red"] = Value::Bool(worktree.is_red() || refuses_staged);
    value["interval"] = json!({
        // The STATE, not merely the list — `asked: false` and "asked, and the
        // index adds nothing" are different facts, and a face that spelled both
        // as `["worktree"]` would make the first read like the second.
        "state": match interval {
            Interval::NotAsked => "not-asked",
            Interval::NoRepository => "no-repository",
            Interval::Coincides => "coincides",
            Interval::Diverges(_) => "diverges",
            Interval::CannotAsk(_) => GREY_CANNOT_ASSESS,
        },
        "spans_the_commit": matches!(interval, Interval::Coincides | Interval::NoRepository)
            || staged.is_some(),
        "cannot_ask_detail": match interval {
            Interval::CannotAsk(detail) => Value::String(detail.clone()),
            _ => Value::Null,
        },
        "diverged_paths": staged.map(|s| s.paths.clone()).unwrap_or_default(),
        "staged": match staged {
            None => Value::Null,
            Some(staged) => interval_json(workspace, &staged.report),
        },
    });
    // **The key is ABSENT when the scoped question was not asked** — this face's own
    // law, stated at [`interval_json`]: *an absent field reads as "not checked"*. A
    // `null` would say the gate WAS asked and had nothing to say, which is a
    // different fact and a false one. Absence also leaves the shipped shape
    // byte-identical, so every existing consumer reads exactly what it read before.
    //
    // The two propositions stay apart on this face as well: `verdict` is what the
    // exit answered about THIS interval, `record_vouches` is the standing fact it
    // refused to spend.
    if let Some(gate) = gate {
        // `record_vouches` and `standing_report` were REMOVED with the record they reported on. There
        // is no ledger left to vouch for itself, so a key saying whether it does would be answering
        // about nothing.
        value["commit_gate"] = json!({
            "gated_interval": gate.label,
            "permits": gate.permits(),
            "verdict": gate.word(),
            "detail": gate.detail(),
            "gated_planes": ["pins"],
            "write_history": WRITE_HISTORY_NOT_ASSESSED,
            // **The POPULATION the gate read, on the face a machine reads** (S3-R23(5)). `permits: true`
            // over `pin_coverage: 0` and over `pin_coverage: 50` are entirely different assurances, and a
            // caller that cannot tell them apart is the caller `--require-pins` exists for. Both keys live
            // INSIDE `commit_gate`, which is itself absent unless the gate was asked — so the shipped
            // `pins` block is byte-identical and no existing consumer moves.
            //
            //
            "pin_coverage": gate.pins.declared,
            "require_pins": gate.require_pins,
        });
    }
    // **Top-level, and never inside [`interval_json`]** (row 21): the fence is a reading of the
    // CHECKOUT, so it is not per-interval and must not be copied into the staged object as though
    // a commit's bytes had a fence state of their own. It is unconditional for the reason its
    // absence would be a lie: this reading was taken on every run.
    //
    value["fence"] = fence_json(fence);
    value
}

/// One interval's verdict as the `check --json` object.
///
/// # BREAKING CHANGE — `core.chain` and `core.foreign_edit` are REMOVED, not nulled
/// This face's own law is that **an absent field reads as "not checked"**, and the
/// two keys were carried as `null` precisely because the opposite was true: they
/// HAD been checked, and null said "checked, nothing to report". Neither is checked
/// now. Keeping them as null would assert a read that never happened — the same
/// false green in JSON that `foreign_edit: none` was in the human render — so they
/// are gone, along with the `cannot_assess` block that named them as its detectors.
///
/// `write_history` replaces all three. It is not a colour and not a detector: it is
/// the statement that this verb does not look, so a consumer cannot mistake the
/// narrowed green for the wider one it used to mean.
fn interval_json(workspace: &Path, report: &CoreReport) -> Value {
    let claims: Vec<Value> = report
        .drifted_claims
        .iter()
        .map(|c| json!({ "selector": c.selector, "detail": c.detail }))
        .collect();
    json!({
        "workspace": workspace.display().to_string(),
        "red": report.is_red(),
        "write_history": WRITE_HISTORY_NOT_ASSESSED,
        "core": { "drifted_claims": claims },
        "pins": pins_json(report),
    })
}

/// The `pins` block: the CLAIM plane's findings and the RETRIEVAL plane's anchoring reading,
/// each carrying its own reason word verbatim (S3-R6 — distinct on the `--json` face as well as
/// the human one).
///
///
///
///
///
fn pins_json(report: &CoreReport) -> Value {
    let pins = &report.pins;
    let row = |p: &PinRow| json!({ "src_path": p.src_path, "declared_ref": p.declared_ref, "color": p.label });
    let orphaned: Vec<Value> = pins
        .orphaned
        .iter()
        .map(|o| {
            json!({
                "src_path": o.src_path,
                "key": o.key,
                "blob_sha": o.blob_sha,
                "state": o.state.word(),
                "live": o.live,
                "nudge": PENDING_ANCHOR_TTL,
            })
        })
        .collect();
    json!({
        "red": pins.red.iter().map(row).collect::<Vec<_>>(),
        "grey": pins.grey.iter().map(row).collect::<Vec<_>>(),
        "anchoring": match &pins.cannot_ask {
            Some(_) => Value::Null,
            // The three-state READING plus its POPULATION (S3-R23(5)): an empty `orphaned` over `asked: 0`
            // is a reading of nothing, not a clean bill.
            None => json!({
                "asked": pins.asked(),
                "anchored": pins.anchored,
                "pending_anchor": pins.pending,
                "never_anchored": pins.never,
                "orphaned": orphaned,
            }),
        },
        "anchoring_cannot_assess": match &pins.cannot_ask {
            Some(detail) => json!({ "reason": GREY_CANNOT_ASSESS, "detail": detail }),
            None => Value::Null,
        },
        // The SIGHT LINE (ruling ) — the population this plane did NOT measure, because those blobs
        // live in another root's object store. Count AND refs: a bare count cannot be acted on, and a
        // silent skip is the false clean. Present on BOTH faces because a machine reader must be able
        // to see the same narrowing a human does.
        //
        "anchoring_out_of_jurisdiction": json!({
            "count": pins.out_of_jurisdiction.len(),
            "refs": pins.out_of_jurisdiction,
            "owner": "u13_per_root_anchoring",
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_accepts_core_and_json() {
        let c = Check::parse(&["--core".to_string(), "--json".to_string()]).expect("parse");
        assert!(matches!(c.format, Format::Json));
        assert!(
            !c.staged,
            "the index is read only when ASKED: a bare invocation answers about the \
             worktree, because an unstaged governed write is the ordinary state of \
             every working repository"
        );
    }

    #[test]
    fn parse_accepts_staged_and_it_is_off_by_default() {
        let c = Check::parse(&["--staged".to_string()]).expect("parse");
        assert!(c.staged, "the interval a commit records was asked for");
        assert!(
            !Check::parse(&[]).expect("parse").staged,
            "and the default is OFF — the fence passes the flag, nothing else has to"
        );
    }

    #[test]
    fn parse_rejects_unknown_flag() {
        assert_eq!(Check::parse(&["--nope".to_string()]).unwrap_err().code, 2);
    }

    /// `--commit-gate` IMPLIES `--staged`: the interval it gates on is the one the index carries,
    /// and a gate without it would read the wrong bytes while reporting confidently. Drop the
    /// implication and the fence must pass two flags that mean nothing apart — this fails.
    ///
    #[test]
    fn commit_gate_implies_staged_and_is_off_by_default() {
        let c = Check::parse(&["--commit-gate".to_string()]).expect("parse");
        assert!(c.commit_gate, "the scoped question was asked");
        assert!(
            c.staged,
            "and it brought the interval a commit records with it"
        );
        let plain = Check::parse(&["--staged".to_string()]).expect("parse");
        assert!(
            !plain.commit_gate,
            "while `--staged` alone still asks the corpus-wide question — the \
             shipped invocation keeps its shipped meaning"
        );
    }

    /// A pin plane with nothing to report — the gate's passing input. `declared: 1` on purpose:
    /// this is a plane that WAS asked something and had no complaint, which is a different input
    /// from one that was asked nothing. The zero-coverage case is its own fixture
    /// ([`no_pins_declared`]), because `--require-pins` is the one reader that tells them apart.
    ///
    ///
    fn holding_pins() -> PinPlane {
        PinPlane {
            red: Vec::new(),
            grey: Vec::new(),
            orphaned: Vec::new(),
            anchored: 0,
            pending: 0,
            never: 0,
            cannot_ask: None,
            declared: 1,
            out_of_jurisdiction: Vec::new(),
        }
    }

    /// A corpus that declares NO pin — clean, and covering nothing.
    fn no_pins_declared() -> PinPlane {
        PinPlane {
            declared: 0,
            ..holding_pins()
        }
    }

    fn pin(color: model::selector::Color, label: &str) -> PinRow {
        PinRow {
            src_path: "claim.md".to_string(),
            declared_ref: "source.md#S".to_string(),
            color,
            label: label.to_string(),
        }
    }

    /// **The gate passes on the plane it actually reads, and says so in a word that claims nothing
    /// more.** `pins-hold` replaced `accounted` / `accounted(unvouched-record)` because both
    /// asserted something about a RECORD the engine no longer keeps .
    ///
    #[test]
    fn a_holding_pin_plane_permits_and_the_word_claims_only_the_plane_it_read() {
        let pins = holding_pins();
        let gate = Gate {
            label: WORKTREE,
            pins: &pins,
            require_pins: false,
        };
        assert!(
            gate.permits(),
            "nothing was found and nothing was unreadable"
        );
        assert_eq!(
            gate.word(),
            PINS_HOLD,
            "the word names the plane that answered"
        );
        assert!(gate.exit().is_ok(), "so the commit proceeds");
        assert!(
            !gate.word().contains("accounted"),
            "the record-era vocabulary must not come back — there is no record to \
             account for anything"
        );
        assert!(
            gate.detail().contains("no memory"),
            "and the pass states the law rather than implying a wider check: {}",
            gate.detail()
        );
    }

    /// **Zero pins passes by DEFAULT — vacuous truth, not unknown.** The gate asks
    /// *"does the world still match the pins"*, and over no pins that is vacuously
    /// yes. This exited 1 before the ruling, and the reason was journal mechanics:
    /// an empty record meant no baseline, and the grey baseline gated. That
    /// antecedent is gone, so the refusal had no input left.
    #[test]
    fn a_corpus_that_declares_no_pin_permits_by_default() {
        let pins = no_pins_declared();
        let gate = Gate {
            label: WORKTREE,
            pins: &pins,
            require_pins: false,
        };
        assert!(gate.permits(), "nothing was asked, so nothing is unknown");
        assert_eq!(gate.word(), PINS_HOLD);
        assert!(gate.exit().is_ok());
    }

    /// **The fail-closed doctrine, preserved as the caller's choice.** The SAME plane under
    /// `--require-pins` refuses — and with its own word, never grey's, because nothing here was
    /// unanswerable.
    #[test]
    fn require_pins_turns_zero_coverage_into_a_refusal() {
        let pins = no_pins_declared();
        let gate = Gate {
            label: WORKTREE,
            pins: &pins,
            require_pins: true,
        };
        assert!(
            !gate.permits(),
            "the caller asked for coverage and got none"
        );
        assert_eq!(
            gate.word(),
            NO_COVERAGE,
            "its own word — grey would say the gate tried and failed, and it did not try"
        );
        assert_ne!(
            gate.word(),
            GREY_CANNOT_ASSESS,
            "the two are distinct facts and must stay distinct words"
        );
        assert!(gate.exit().is_err(), "and it fails CLOSED, as asked");

        // The other direction, same flag: coverage present ⇒ the flag is silent.
        let covered = holding_pins();
        let strict = Gate {
            label: WORKTREE,
            pins: &covered,
            require_pins: true,
        };
        assert!(
            strict.permits(),
            "the flag refuses ABSENCE of coverage, never coverage itself — a flag \
             that refused both would be a gate wired shut"
        );
    }

    /// **A red pin refuses, and the gate is load-bearing in that direction.** Without this arm the
    /// test above passes over a gate that permits unconditionally.
    #[test]
    fn a_red_pin_refuses_the_commit() {
        let pins = PinPlane {
            red: vec![pin(
                model::selector::Color::Red(model::selector::RedReason::Drifted),
                "red content-drifted",
            )],
            ..holding_pins()
        };
        let gate = Gate {
            label: STAGED,
            pins: &pins,
            require_pins: false,
        };
        assert!(
            !gate.permits(),
            "the ledger claims content that is not there"
        );
        assert_eq!(gate.word(), RED);
        let err = gate.exit().expect_err("a red pin is a refusal");
        assert!(
            format!("{err:?}").contains(STAGED),
            "and the refusal names the interval, so a reader can locate it"
        );
    }

    /// **Unknown is not clean.** A grey pin refuses on the same leg as a red one, and carries the
    /// distinct reason word — the exit says do-not-proceed, the word says why.
    ///
    #[test]
    fn an_unreadable_pin_plane_refuses_as_an_absence_and_not_as_a_finding() {
        let pins = PinPlane {
            grey: vec![pin(
                model::selector::Color::Grey(model::selector::GreyReason::Ambiguous),
                "grey unmounted",
            )],
            ..holding_pins()
        };
        let gate = Gate {
            label: WORKTREE,
            pins: &pins,
            require_pins: false,
        };
        assert!(!gate.permits(), "an unread plane is not a clean one");
        assert_eq!(
            gate.word(),
            GREY_CANNOT_ASSESS,
            "and it is never spelled as a finding — nobody was accused"
        );
        assert!(gate.exit().is_err(), "while still failing CLOSED");
    }

    /// An object store that could not be asked is the OTHER grey antecedent, and it refuses too — a
    /// `cannot_ask` that permitted would be the false green this verb exists to close, one plane
    /// over from the one the engine is ruled not to hold.
    ///
    #[test]
    fn an_unaskable_object_store_refuses_the_commit() {
        let pins = PinPlane {
            cannot_ask: Some("the object store could not be asked".to_string()),
            ..holding_pins()
        };
        let gate = Gate {
            label: WORKTREE,
            pins: &pins,
            require_pins: false,
        };
        assert!(!gate.permits());
        assert_eq!(gate.word(), GREY_CANNOT_ASSESS);
    }

    #[test]
    fn parse_rejects_stray_positional() {
        assert_eq!(Check::parse(&["extra".to_string()]).unwrap_err().code, 2);
    }
}
