//! `mrd check` — the pure READ validity verb (U2.10; d2 §3 check).
//!
//! ```text
//! mrd check [--core] [--staged] [--commit-gate] [--json]
//! ```
//!
//! Runs the convention-free CORE (layer 0) over the resolved workspace: date the
//! receipt journal against the live tree (last-receipt-vs-live) and, when that
//! holds, recompute the journal's chain continuity; then read the PIN PLANE — the
//! pin verdicts and the anchoring state of every pinned blob. `status = freshness,
//! check = validity` — this answers "what lies?", writing nothing and minting no
//! receipt.
//!
//! **U14 — the two planes fail independently.** Until this verb could see the pin
//! plane, a green here meant *"baseline provable AND nothing the JOURNAL plane can
//! see"*, and the fence built on it passed a corpus whose lock arrived by clone or
//! pull while its source moved (`check` green / `walk` `red content-drifted` /
//! `status` `lock red content-drifted`, one corpus, one run) and a corpus holding
//! a blob no ref reaches — a fact no journal row will ever carry. The pin colours
//! come from `view::walk::lock_pin_colors`, the SAME call `mrd status`'s lock axis
//! makes over the SAME corpus build, so the three planes agree by construction and
//! not by coincidence.
//!
//! # THE INTERVAL THIS VERB SPANS, stated because a check is only as wide as it
//! (S3-R29)
//! Two intervals, both named in every answer:
//!
//! - **`worktree`** — the bytes on disk. Always assessed.
//! - **`staged`** — the bytes the git INDEX carries, assessed whenever it carries
//!   anything the worktree does not, because **that is the interval a commit
//!   records**.
//!
//! **F1 — why the second one exists.** `domain_snapshot` reads the worktree; git
//! commits the index. Forge a pinned section, `git add` it, restore the exact
//! governed bytes to the worktree, and the shipped verb answered `chain: green /
//! pins: green`, exit 0 — over bytes no commit would record — while
//! `git show HEAD:<page>` came back carrying the forgery. The fence built on this
//! verb read a true statement about the wrong interval. `git add -p`,
//! `git commit <pathspec>`, `git stash` and any concurrent writer between
//! `git add` and hook fire reach the same gap.
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
//! The interval above says WHICH BYTES an answer covers. `--commit-gate` says WHICH
//! QUESTION is put to them, and the two are independent.
//!
//! Without it the verb asks *"is everything this corpus's record says true?"* — a
//! claim about the whole **write history**, whose answer is **permanent** once a row
//! breaks. With it the verb asks the narrower, per-commit question:
//!
//! > **were these bytes produced by a governed write?**
//!
//! **Why a second question and not a second exit code.** Three distinct
//! propositions rode the single exit `1` out of `mrd check --staged`: a journal
//! chain break (about the **past** — permanent by design), an out-of-band write in
//! this index (about **this interval** — per-commit), and `grey(cannot-assess)`
//! (about **evidence availability** — per-state). A pre-commit fence branches on the
//! code alone, so it answered a permanent fact about the ledger's history with the
//! same byte as a per-commit fact about bytes that fact never examined. Past the
//! first break the exit stopped varying with what was staged: a guard whose verdict
//! no longer depends on the thing it guards carries **zero information about it**,
//! and the only ways past are the unrecorded escapes — so the per-commit
//! enforcement is destroyed too. The fix is not a fourth code and not a reason word
//! the fence must parse; it is asking the question whose answer is actually
//! per-commit.
//!
//! **The permanence is untouched.** `mrd check` unscoped stays RED forever, citing
//! the same row. A break never heals, is never acknowledged, and is never cleared —
//! and under `--commit-gate` it is **printed on stderr at every commit**. The
//! blocking is downgraded; the telling never is.
//!
//! **A gated pass over a broken record is never spelled green.** With the chain
//! broken, the record that accounts for the interval may itself hold a forged row,
//! so the acceptance is genuinely weaker and carries the weaker word
//! `accounted(unvouched-record)`. Stating that weakness is the price of unblocking;
//! hiding it would be the same defect wearing the other sign.
//!
//! Read-only. Exit triad (§4 preamble):
//! - **0** — green: the journal dates the live tree and its chain is continuous.
//!   Under `--commit-gate`: the interval a commit records is accounted for.
//! - **1** — a finding: a broken journal chain (cites the row). A check finding,
//!   never a door refusal (refusal-amendment). **Grey rides this leg too** (S3-R5
//!   and S3-R8, spelled by S3-R6): when the journal cannot date the tree — no rows,
//!   or a last receipt the live tree no longer matches — the verb refuses
//!   `grey(cannot-assess)`. Unknown is not clean, and a hook that rejects on
//!   non-zero must reject what nobody could vouch for. The triad stays CLOSED: no
//!   fourth code. The exit answers "may this proceed?" (red and grey both say no);
//!   the reason word, distinct on both faces, says why.
//! - **2** — bad invocation, or an unreadable workspace / journal.
//!
//! `--commit-gate` keeps all three meanings exactly. Only the question changes, so
//! a caller that branches on the code alone still reads a code that means what it
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

use std::path::Path;

use check::{Accounted, CoreReport, GREY_CANNOT_ASSESS, JournalTrace, PinPlane, PinRow};
use receipt::anchor::{ObjectAnchor, PENDING_ANCHOR_TTL};
use serde_json::{Value, json};

use crate::hook;
use crate::{Fail, Format, current_dir};

/// The finding leg of the triad: the invocation was well-formed, the core found a
/// lie (a chain break or a foreign edit).
const EXIT_FINDING: u8 = 1;

/// Run `mrd check [--core] [--json]`: resolve the workspace and run the layer-0
/// core, printing the verdict.
///
/// # Errors
/// [`Fail`] exit 2 on a bad invocation or an unreadable workspace/journal; exit 1
/// when the core reddens (chain break or foreign edit).
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

    // U11/F6 — the REAL mount table, through the one loader `mrd walk` uses. A
    // default (empty) table here is what made the cross-root pin axis answer
    // `grey(unmounted)` for a bound root in all three of its states.
    let mounts = crate::walk_cmd::load_mounts();

    // ONE read of the worktree, whose bytes feed the fold AND the corpus — the
    // reason `domain_snapshot` returns both. A second read would let the two
    // planes describe two different worktrees.
    let domain = fs::domain::Domain::load(&root)
        .map_err(|e| Fail::tool(format!("cannot read the hash domain: {e}")))?;
    let (worktree_files, worktree_fold) = fs::domain_snapshot(&root)
        .map_err(|e| Fail::tool(format!("cannot read the corpus: {e}")))?;
    let worktree_journal = check::journal_page(&root)
        .map_err(|e| Fail::tool(format!("cannot read the receipt journal: {e}")))?;

    // THE SECOND INTERVAL (F1), and only when the caller asked the question it
    // answers. `git` reports what the INDEX carries wherever it differs from the
    // worktree; absent divergence the two intervals coincide and one pass answers
    // both.
    let interval = if parsed.staged {
        staged_interval(&root, &domain, &worktree_files, &worktree_journal)
    } else {
        Interval::NotAsked
    };

    let worktree = assess(
        &root,
        &mounts,
        worktree_files,
        &worktree_fold.0,
        &worktree_journal,
    )?;

    let staged = match &interval {
        Interval::Diverges(bytes) => Some(Assessed {
            paths: bytes.paths.clone(),
            report: assess_staged(
                &root,
                &mounts,
                bytes.files.clone(),
                &bytes.fold,
                &bytes.journal,
                &worktree_journal,
            )?,
        }),
        _ => None,
    };

    let gate = parsed.commit_gate.then(|| {
        build_gate(
            &interval,
            &worktree,
            staged.as_ref(),
            &worktree_journal,
            &worktree_fold.0,
        )
    });

    // **The CHECKOUT's fence coverage — read here, reported below, and reachable
    // from no exit path in this function** (row 21). It is deliberately taken on
    // every invocation, gated and ungated alike: a reading whose presence depended
    // on a flag would be a reading the operator has to know to ask for, which is
    // the silence this line closes.
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

    // **DOWNGRADE THE BLOCKING, NEVER THE TELLING.** The standing break is printed
    // on every gated run that has one — pass or refuse — and on stderr, so it
    // survives a caller that reads stdout. No acknowledgement state exists to
    // silence it: a store of "I have seen this" would be durable state that must
    // itself be attested, a new forgery surface guarding a fact that is already
    // permanent. Telling it every time reaches the same end with nothing new to
    // forge.
    if let Some(standing) = gate.as_ref().and_then(Gate::standing_report) {
        eprint!("{standing}");
    }

    // FAIL CLOSED on an interval that was ASKED FOR and could not be read. The
    // caller asked what a commit would record; degrading silently to the worktree
    // answer is the F1 shape again with one more step in front of it — a true
    // statement about the wrong bytes.
    if let Interval::CannotAsk(detail) = &interval {
        return Err(Fail {
            code: EXIT_FINDING,
            message: format!(
                "check refuses ({STAGED}): {GREY_CANNOT_ASSESS} — {detail}; the interval a \
                 commit records could not be read, and a commit nobody could vouch for is not \
                 a verified one"
            ),
        });
    }

    // **The scoped exit, and it reads ONE interval.** The worst-of below is the
    // corpus-wide question's answer and stays exactly as it was for every caller
    // that asks it; a gated run never reaches it.
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
            return Err(Fail {
                code: EXIT_FINDING,
                message: refusal_list(label, &summary),
            });
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
        out.push_str(&format!("\n  {}. {finding}", i + 1));
    }
    out
}

/// **Which interval a commit records**, and the scoped question put to it. When the
/// index diverges that is the staged bytes; when it coincides, or there is no
/// repository at all, the worktree IS that interval and its own render says so.
///
/// Either way ONE interval answers, and the record it is read against is always the
/// WORKTREE's journal — the most complete one the engine has, and the one the
/// worktree pass separately validates in the same run.
fn build_gate<'a>(
    interval: &'a Interval,
    worktree: &'a Assessed,
    staged: Option<&'a Assessed>,
    worktree_journal: &'a str,
    worktree_fold: &'a str,
) -> Gate<'a> {
    let (label, journal, root, pins) = match (interval, staged) {
        (Interval::Diverges(bytes), Some(staged)) => (
            STAGED,
            bytes.journal.as_str(),
            bytes.fold.as_str(),
            &staged.report.pins,
        ),
        _ => (
            WORKTREE,
            worktree_journal,
            worktree_fold,
            &worktree.report.pins,
        ),
    };
    Gate {
        label,
        accounted: check::interval_accounted(journal, root, worktree_journal),
        pins,
        record: &worktree.report.trace,
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

/// The gated pass whose record vouches for itself — the full-strength word.
const ACCOUNTED: &str = "accounted";

/// The gated pass whose record does **not** vouch for itself. A separate word
/// because it is a separate, weaker claim: the interval is accounted for by a
/// ledger whose own chain is broken or unreadable, so the acceptance rests on
/// something nothing vouches for. Never spelled green (S3-R6 — one vocabulary,
/// distinct words for distinct causes).
const ACCOUNTED_UNVOUCHED: &str = "accounted(unvouched-record)";

/// **What `--commit-gate` reads, and the standing fact it reports beside it.**
///
/// # One interval decides the exit — that is the whole law
/// Worst-of ACROSS intervals is right for the unscoped question, which is a claim
/// about the corpus. It is wrong for this one: a finding from the worktree
/// interval would swamp a clean answer about the bytes a commit records, which is
/// precisely how a permanent fact came to be spent as a per-commit verdict. So the
/// gate names ONE interval — the one a commit records — and reads two planes of it:
/// whether the record accounts for it, and whether its pins hold. Every other
/// reading in this run is REPORTED and gates nothing.
struct Gate<'a> {
    /// Which interval the exit reads. Named in the render and in every refusal,
    /// because a refusal a reader cannot locate is one they cannot act on.
    label: &'a str,
    /// The scoped question's answer over that interval — this is the EXIT.
    accounted: Accounted,
    /// That same interval's pin plane — the second gated plane. A pin is a claim
    /// about the bytes being committed, so it belongs to the interval, not to the
    /// history.
    pins: &'a PinPlane,
    /// **The RECORD's own standing: reported, never gated on.** This is the
    /// permanent proposition, and holding it here rather than in the exit is what
    /// keeps it from being spent on every future commit.
    record: &'a JournalTrace,
}

impl Gate<'_> {
    /// May this commit proceed? Both gated planes must answer, and an unread plane
    /// is not a clean one — unknown is not clean, on either.
    fn permits(&self) -> bool {
        self.accounted.is_accounted() && !self.pins.is_red() && !self.pins.cannot_assess()
    }

    /// The verdict WORD. On the passing leg it says whether the record that let the
    /// commit through could vouch for itself; on the refusing leg it says whether
    /// something was read and found bad, or whether nothing could be read at all.
    fn word(&self) -> &'static str {
        if self.permits() {
            if self.record.vouches() {
                ACCOUNTED
            } else {
                ACCOUNTED_UNVOUCHED
            }
        } else if self.accounted.is_red() || self.pins.is_red() {
            RED
        } else {
            GREY_CANNOT_ASSESS
        }
    }

    /// Why — the half of the answer the exit code cannot carry. Names the plane
    /// that decided, so a reader is never left inferring which of two gated planes
    /// spoke.
    fn detail(&self) -> String {
        if self.permits() || !self.accounted.is_accounted() {
            return self.accounted.detail();
        }
        // The journal plane accounted for the interval and the PIN plane refused.
        // Cite its first finding only — the render above carries the full list, and
        // repeating it here is how a reader comes to believe there are two.
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

    /// The scoped exit — the closed triad, over one interval. `0` it is accounted
    /// for; `1` it is not, or could not be assessed; `2` never comes from here,
    /// because a bad invocation is refused before any interval is read.
    ///
    /// # Errors
    /// [`Fail`] exit 1 when the interval a commit records is not accounted for, or
    /// its pin plane refuses.
    fn exit(&self) -> Result<(), Fail> {
        if self.permits() {
            return Ok(());
        }
        Err(Fail {
            code: EXIT_FINDING,
            message: format!(
                "check refuses ({}): {} — {}",
                self.label,
                self.word(),
                self.detail()
            ),
        })
    }

    /// **The STANDING BREAK, told on every run that has one** (never a blocker,
    /// never withheld). `None` when the record vouches for itself and there is
    /// nothing standing to tell.
    ///
    /// It goes to **stderr** so that it survives a caller reading stdout, and it is
    /// emitted whether the gate permits or refuses: a fact nobody is told each time
    /// is a fact that gets forgotten, and forgetting is exactly what an
    /// acknowledgement store would institutionalise. Telling it every time is the
    /// same end with no new state to attest and no new forgery surface.
    fn standing_report(&self) -> Option<String> {
        if self.record.vouches() {
            return None;
        }
        let finding = self
            .record
            .red_summary()
            .or_else(|| self.record.grey_summary())?;
        Some(format!(
            "meridian: the RECORD this commit was gated against does not vouch for itself.\n  \
             {finding}\n  This is a STANDING fact about the write history, not a finding about \
             the bytes you are committing: it does not clear, it is never acknowledged away, and \
             `mrd check` reports it on every run. This commit was gated on the {} interval alone \
             — {}.\n",
            self.label,
            self.word()
        ))
    }
}

// ---------------------------------------------------------------------------
// the fence line — the checkout, not the corpus (row 21)
// ---------------------------------------------------------------------------

/// The clause every fence line carries, spelled ONCE so the two faces cannot drift
/// apart on the one claim this whole reading rests on.
const FENCE_REPORTED: &str = "REPORTED, never gated on — fence coverage is a property of this \
                              local checkout and not of the corpus, so this line does not move \
                              check's exit";

/// **What the local CHECKOUT's fence looks like — a reading beside the verdict,
/// never part of it** (row 21).
///
/// # It is not a fourth proposition
/// The chain, the `foreign_edit` trace and the pin plane are propositions about the
/// corpus's bytes and their write history. Fence coverage is a proposition about
/// the **local checkout's configuration**: a different subject on a different axis,
/// which never competed for the exit code, so the closed triad above is not under
/// pressure here and needs no defending.
///
/// # The isolation is STRUCTURAL, not a promise
/// This type is not a field of [`Gate`]; nothing in [`worst_of_exit`],
/// [`Gate::permits`] or [`Gate::exit`] can name it; and [`observe_fence`] returns
/// no `Result`, so there is no error for a caller to propagate into a [`Fail`].
/// The two checkouts of one corpus — one fenced, one not — exit identically
/// because there is no path by which they could differ.
///
/// # Read-only, and it leaves no souvenir
/// It reads through [`hook::status`], which opens the door files and nothing
/// else. A root must not come away with anything in its git dir as the souvenir
/// of being looked at — and since the install plane retired there is no writer in
/// this engine that could leave one.
struct Fence {
    /// The one word for this checkout: [`hook::Coverage::word`] when the root can
    /// carry a fence, [`hook::Unfenceable::word`] when it cannot. **Never
    /// re-spelled here** (S3-R6) — an operator who learned these words from
    /// `mrd skill hook`'s document reads the same ones off this face.
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
    /// How many of those doors carry a fence this engine's line wrote — the
    /// COVERAGE axis, and **blind to currency by construction**
    /// ([`hook::Coverage::fenced_doors`]). A door standing at an older or a newer
    /// generation is counted here; whether it is current is [`Fence::word`]'s
    /// claim, never this one's.
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

/// Read the checkout's fence state.
///
/// **It cannot fail into the exit.** Every outcome of the survey — a reachable
/// root, or one the fence cannot reach — is a [`Fence`] to report, so no branch
/// here produces a value a caller could turn into a [`Fail`].
fn observe_fence(workspace: &Path) -> Fence {
    match hook::status(workspace) {
        Ok(coverage) => {
            let fenced = coverage.fenced_doors();
            Fence {
                word: coverage.word(),
                // `Coverage::teaching` is silent on the two states where the set
                // agrees with itself, because a reader who went looking for the
                // door set has it in front of them. **This reader did not** —
                // the line is unasked-for, and `absent` is precisely the state
                // row 21 exists to stop being silent about.
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
        // A root the fence cannot reach is reported with its OBSERVED reason word
        // and its teaching — never as a bare absence, and never as a refusal.
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

/// The fence line(s) on the human face: the checkout's coverage, then the doors
/// when it has any.
///
/// **Two lines rather than one**, for the reason [`Fence::doors`] gives: the set's
/// word cannot carry which door disagrees, and the per-door line can.
///
/// # EACH CLAUSE NAMES ITS OWN AXIS
/// The first line composes **two instruments**: the count is COVERAGE — how many
/// doors this engine's line wrote — and the set word is CURRENCY — whether what
/// they carry is the generation this engine writes. They are measured separately
/// and they disagree on purpose: a fully skewed checkout is `3 of 3` doors and
/// `installed-superseded`, which is the reading that keeps `installed-partial` and
/// `installed-superseded` different states.
///
/// So the count's clause may claim ONLY what [`Fence::fenced`] measured — the
/// marker, at whatever generation it declares — and says so outright. The clause
/// it replaced (*"carry this engine's fence"*) asserted the currency the count
/// never took: beside `installed-superseded` it was false as written, and beside
/// `installed-ahead` it contradicted its own teaching one clause later. That is
/// `hook.rs`'s standing hazard — *nothing can prompt an operator to re-place a
/// fence if the reporting face calls a superseded one "installed"* — compressed
/// into a sub-sentence.
fn fence_lines(fence: &Fence) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    // The teachings come from two different types and only some of them end in a
    // full stop; trimming it here is what keeps one render grammar over both.
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
        // No door plane, so no door line. The same law the `--json` face states at
        // `interval_json`: an absence is not an empty reading of something.
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
        // What THIS ENGINE writes, beside what the files declare (per door below).
        // A verdict that does not disclose its judge cannot be checked by a third
        // party, and in a version skew both participants are inside it.
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

/// **What was asked about the second interval, and what came back** — the state
/// the render states and the exit fails closed on.
///
/// Four outcomes, each a DIFFERENT fact about this run, so none of them is
/// rendered as another's silence (the same law the `pins:`/`anchoring:` split
/// runs on): not asked · asked and there is no index · asked and the index adds
/// nothing · asked and it carries other bytes.
enum Interval {
    /// `--staged` was not passed: the index was never read, and this answer is
    /// about the worktree only. **Said out loud** — a reader must not take a
    /// worktree green for a statement about their commit.
    NotAsked,
    /// Asked, and this workspace is not a git repository, so no index exists to
    /// record anything. Marker-beats-git makes that a supported state, not a
    /// degradation.
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
    fold: String,
    journal: String,
}

/// Assess ONE interval: build its corpus, colour its pins through the one
/// computer with the real mount table, and run the layer-0 core over ITS bytes.
fn assess(
    root: &fs::WorkspaceRoot,
    mounts: &crate::walk_cmd::Mounts,
    files: fs::DomainFiles,
    fold: &str,
    journal: &str,
) -> Result<Assessed, Fail> {
    let (_index, docs) =
        fs::build_corpus(files).map_err(|e| Fail::tool(format!("cannot build the corpus: {e}")))?;
    let corpus = mounts.rooted(&docs);
    let pins = pin_rows(&corpus, mounts.set());
    Ok(Assessed {
        paths: Vec::new(),
        report: check::core_of(root, journal, fold, &docs, &pins),
    })
}

/// [`assess`] for the STAGED interval, dated against the RECORD.
///
/// The difference is one plane: a legitimately staged INTERMEDIATE governed state is
/// not the CURRENT one — `git add` stages content without the journal — so dating it
/// against its own last row is a false red on the commonest path there is
/// (`git add`, then any further governed write). `check::core_of_staged` asks
/// instead whether the record accounts for these bytes AND the staged journal is a
/// truthful prefix of it. The pin plane and the object store are identical.
fn assess_staged(
    root: &fs::WorkspaceRoot,
    mounts: &crate::walk_cmd::Mounts,
    files: fs::DomainFiles,
    fold: &str,
    journal: &str,
    worktree_journal: &str,
) -> Result<CoreReport, Fail> {
    let (_index, docs) =
        fs::build_corpus(files).map_err(|e| Fail::tool(format!("cannot build the corpus: {e}")))?;
    let corpus = mounts.rooted(&docs);
    let pins = pin_rows(&corpus, mounts.set());
    Ok(check::core_of_staged(
        root,
        journal,
        fold,
        worktree_journal,
        &docs,
        &pins,
    ))
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
    worktree_journal: &str,
) -> Interval {
    let repo = git::Repo::at(&root.0);
    let divergence = match repo.staged_divergence() {
        Ok(divergence) => divergence,
        // No repository, no index, nothing a commit here could record.
        Err(git::GitFail::NotARepo { .. }) => return Interval::NoRepository,
        // Inside a repository and git could not answer. NOT the worktree answer
        // wearing a wider label — the caller asked about the commit's interval and
        // this run cannot speak about it.
        Err(other) => return Interval::CannotAsk(other.to_string()),
    };
    if divergence.is_empty() {
        return Interval::Coincides;
    }

    let (files, fold) = fs::overlay_snapshot(worktree_files, &divergence, domain);
    let journal = divergence
        .iter()
        .find(|(rel, _)| rel == fs::domain::RESERVED_JOURNAL_PATH)
        .map_or_else(
            || worktree_journal.to_owned(),
            |(_, content)| {
                content.as_ref().map_or_else(String::new, |bytes| {
                    String::from_utf8_lossy(bytes).into_owned()
                })
            },
        );
    let paths = divergence
        .iter()
        .filter(|(rel, _)| {
            domain.contains(Path::new(rel)) || rel == fs::domain::RESERVED_JOURNAL_PATH
        })
        .map(|(rel, _)| rel.clone())
        .collect::<Vec<_>>();
    if paths.is_empty() {
        // Everything that diverges is outside the hash domain and is not the
        // journal — code, assets, a lock file. Neither interval reads those, so
        // there is nothing here a second assessment could say.
        return Interval::Coincides;
    }
    Interval::Diverges(StagedBytes {
        paths,
        files,
        fold: fold.0,
        journal,
    })
}

/// Colour every `meridian-lock` pin in the corpus through **the one pin
/// computer** — `view::walk::lock_pin_colors_rooted`, which is exactly what
/// `mrd status`'s lock axis reads and what colours a `mrd walk` listing.
///
/// This is the seam that makes the three planes agree BY CONSTRUCTION. A `check`
/// that re-derived pin colours would be a second implementation of corpus index →
/// ref resolution → selector → fingerprint compare, and a second copy of that
/// chain is how the pin plane and the decoration plane once came to hash two
/// different documents for one ref. There is one computer here, not three that
/// happen to match today.
///
/// **F6 — the computer was right and its INPUT was blind.** This verb handed it
/// `lock_pin_colors(docs)`, which resolves against `MountSet::default()` and an
/// ambient-only corpus, so on a BOUND root every cross-root pin answered
/// `grey(unmounted)` whether its target matched, had drifted, or had been
/// restored — three states, one answer, under the fence. The agree-by-construction
/// structure above is what made this ONE edit rather than three: the corpus and
/// the mount table are now the caller's to supply, and `mrd walk` supplies the
/// same two through the same loader.
///
/// The label rides along from `color_label` for the same reason: the reason words
/// (`content-drifted`, `unmounted`, `path-unseeable`, …) are spelled once, in
/// `view`, and are never re-spelled by this verb (S3-R6/S3-R59).
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
}

impl Check {
    fn parse(args: &[String]) -> Result<Self, Fail> {
        let mut json = false;
        let mut staged = false;
        let mut commit_gate = false;
        for arg in args {
            match arg.as_str() {
                "--json" => json = true,
                // `--core` names layer 0 explicitly; it is the default today, so it
                // is accepted and needs no separate branch.
                "--core" => {}
                // git's own word for the index, deliberately (`git diff --staged`)
                // — one vocabulary, and never a second spelling for a concept the
                // operator's other tool already named.
                "--staged" => staged = true,
                // The QUESTION, not a second interval: it names the caller — a
                // pre-commit gate — rather than a mechanism, so what it changes is
                // legible from the flag alone.
                "--commit-gate" => {
                    commit_gate = true;
                    staged = true;
                }
                flag if flag.starts_with('-') => {
                    return Err(Fail::tool(format!("unknown flag: {flag}")));
                }
                value => {
                    return Err(Fail::tool(format!("unexpected argument: {value}")));
                }
            }
        }
        Ok(Check {
            format: if json { Format::Json } else { Format::Human },
            staged,
            commit_gate,
        })
    }
}

/// Render the core verdict as a human block: the header, the chain line, the
/// `foreign_edit` line, and one line per drifted claim (none at the CLI today).
///
/// Both detector lines render `grey(cannot-assess)` when the journal cannot date
/// the tree — with no row, or with a last receipt the live root no longer
/// continues. Neither may borrow the word the assessed path earns, and neither may
/// accuse: the mismatch is rendered as the evidence it is.
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

/// **The PREDICATE the staged interval asserts, stated because it is WEAKER than the
/// worktree's and S3-R29 is about the claim as much as the byte range.**
///
/// The worktree pass asks *"are these bytes the CURRENT governed state?"*. The staged
/// pass cannot: `git add` stages content without the journal, so an ordinary
/// `git add` followed by any further governed write leaves a staged tree that is an
/// EARLIER governed state, and refusing it is a false red on the commonest path there
/// is. So this interval asks the weaker question, and says which one it asked.
fn staged_predicate_line() -> String {
    format!(
        "  {STAGED} asserts: these bytes were PRODUCED BY A GOVERNED WRITE (they match a receipt \
         in the journal) and the journal being committed is a truthful PREFIX of it — NOT that \
         they are the current governed state, which a partial stage legitimately is not\n"
    )
}

/// One interval's verdict lines — the journal TRACE, the claims, and the pin
/// plane. Shared by both intervals so a reader compares like with like, and so a
/// line can never exist for one interval and not the other.
fn render_report(report: &CoreReport) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();

    match &report.trace {
        JournalTrace::NoBaseline => {
            let _ = writeln!(
                out,
                "  chain: {GREY_CANNOT_ASSESS} — the receipt journal carries no row, so there \
                 is no chain to recompute"
            );
            let _ = writeln!(
                out,
                "  foreign_edit: {GREY_CANNOT_ASSESS} — the receipt journal carries no last \
                 receipt to attribute the live tree against"
            );
        }
        JournalTrace::StaleBaseline(m) => {
            let _ = writeln!(
                out,
                "  chain: {GREY_CANNOT_ASSESS} — the journal's last receipt ^{} does not account \
                 for the live tree, so its rows cannot be read against it",
                m.last_receipt
            );
            let _ = writeln!(
                out,
                "  foreign_edit: {GREY_CANNOT_ASSESS} — tree root {} does not continue the last \
                 receipt ^{} (recorded root_after={}); something advanced the tree that the \
                 journal does not account for, and an out-of-writer edit is not the only door \
                 that leaves this trace",
                m.live_root, m.last_receipt, m.recorded_root
            );
        }
        JournalTrace::Assessed { chain } => {
            if let Some(summary) = chain.red_summary() {
                let _ = writeln!(out, "  chain: {RED} — {summary}");
            } else {
                let _ = writeln!(out, "  chain: green");
            }
            let _ = writeln!(out, "  foreign_edit: none");
        }
    }

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
    // The anchoring THREE-STATE as a reading (GAP A), with its POPULATION beside
    // it (S3-R23(5)): the same empty orphan list means one thing over fifty pinned
    // blobs and something else entirely over none, and a reading that cannot tell
    // them apart is how coverage disappears with nothing failing.
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

    // ── the RUN PLANE (G3) ──────────────────────────────────────────────────
    // Pre-exec receipts with no completion. REPORTED, never gated on — like
    // the fence line, this does not move check's exit. Receipts from before
    // the completion marker can never clear, and a permanent red is how a
    // reader learns to stop reading a plane.
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

/// The `--json` shape: the workspace plus the core object (chain breaks, the
/// `foreign_edit`, the drifted claims) and the top-level `red` verdict.
///
/// When the journal cannot date the tree, both journal detectors are `null` —
/// *not assessed*, never a `{"green": true}` a reader could bank on — and a
/// `cannot_assess` block carries the reason word, the detectors it covers, the
/// detail, and the `baseline` evidence (`null` when there is no row at all).
/// `red` stays honest: grey is not red. The assessed shape is untouched.
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
        value["commit_gate"] = json!({
            "gated_interval": gate.label,
            "permits": gate.permits(),
            "verdict": gate.word(),
            "detail": gate.detail(),
            // The permanent proposition, reported and never gated on. `false` here
            // with `permits: true` is the whole point of the flag, not a
            // contradiction.
            "record_vouches": gate.record.vouches(),
            "standing_report": match gate.standing_report() {
                None => Value::Null,
                Some(report) => Value::String(report),
            },
        });
    }
    // **Top-level, and never inside [`interval_json`]** (row 21): the fence is a
    // reading of the CHECKOUT, so it is not per-interval and must not be copied
    // into the staged object as though a commit's bytes had a fence state of their
    // own. It is unconditional for the reason its absence would be a lie: this
    // reading was taken on every run.
    value["fence"] = fence_json(fence);
    value
}

/// One interval's verdict as the shipped `check --json` object.
fn interval_json(workspace: &Path, report: &CoreReport) -> Value {
    let claims: Vec<Value> = report
        .drifted_claims
        .iter()
        .map(|c| json!({ "selector": c.selector, "detail": c.detail }))
        .collect();
    let pins = pins_json(report);

    let JournalTrace::Assessed { chain } = &report.trace else {
        let baseline = match &report.trace {
            JournalTrace::StaleBaseline(m) => json!({
                "last_receipt": m.last_receipt,
                "recorded_root": m.recorded_root,
                "live_root": m.live_root,
            }),
            _ => Value::Null,
        };
        return json!({
            "workspace": workspace.display().to_string(),
            "red": report.is_red(),
            "cannot_assess": {
                "reason": GREY_CANNOT_ASSESS,
                "detectors": ["chain", "foreign_edit"],
                "detail": report.trace.grey_summary().unwrap_or_default(),
                "baseline": baseline,
            },
            "core": {
                "chain": Value::Null,
                "foreign_edit": Value::Null,
                "drifted_claims": claims,
            },
            "pins": pins,
        });
    };

    let breaks: Vec<Value> = chain
        .breaks
        .iter()
        .map(|b| {
            json!({
                "row_anchor": b.row_anchor,
                "line_no": b.line_no,
                "expected_root_before": b.expected_root_before,
                "found_root_before": b.found_root_before,
            })
        })
        .collect();
    json!({
        "workspace": workspace.display().to_string(),
        "red": report.is_red(),
        "core": {
            "chain": { "green": chain.is_green(), "breaks": breaks },
            // Assessed ⇔ the last receipt accounts for the live tree, so this key
            // is null by construction here. It stays in the shape: an absent field
            // reads as "not checked", and this one WAS checked (S3-R8 moved its
            // only non-null case into `cannot_assess`).
            "foreign_edit": Value::Null,
            "drifted_claims": claims,
        },
        "pins": pins,
    })
}

/// The `pins` block: the CLAIM plane's findings and the RETRIEVAL plane's
/// anchoring reading, each carrying its own reason word verbatim (S3-R6 — distinct
/// on the `--json` face as well as the human one).
///
/// `anchoring` is `null` when the object store could not be asked, and the reason
/// is stated in `anchoring_cannot_assess` — *not assessed*, never an empty array a
/// reader could bank as clean. The `pending_anchor` array is a reading of a plane
/// that WAS asked, so its emptiness means something; a `null` says nothing was.
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
            // The three-state READING plus its POPULATION (S3-R23(5)): an empty
            // `orphaned` over `asked: 0` is a reading of nothing, not a clean bill.
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

    /// `--commit-gate` IMPLIES `--staged`: the interval it gates on is the one the
    /// index carries, and a gate without it would read the wrong bytes while
    /// reporting confidently. Drop the implication and the fence must pass two
    /// flags that mean nothing apart — this fails.
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

    /// A `PinPlane` with nothing to report — the pin axis held flat so the gate
    /// arms below measure the JOURNAL axis alone.
    fn clean_pins() -> PinPlane {
        PinPlane {
            red: Vec::new(),
            grey: Vec::new(),
            orphaned: Vec::new(),
            anchored: 0,
            pending: 0,
            never: 0,
            cannot_ask: None,
        }
    }

    /// A journal page from `(anchor, root_before, root_after)` triples — the three
    /// facts a row asserts, in the shipped row grammar.
    fn chain_of(rows: &[(&str, &str, &str)]) -> String {
        use std::fmt::Write as _;
        let mut page = String::new();
        for (anchor, before, after) in rows {
            let _ = writeln!(
                page,
                "- op=splice path=p.md root_before={before} root_after={after} edits=0 ^{anchor}"
            );
        }
        page
    }

    /// **A broken record downgrades the BLOCKER and never the REPORT.** One state,
    /// three assertions that must hold together: the gate permits, the word is the
    /// weaker one, and the standing break is told anyway citing its row.
    ///
    /// Drop the scoping and `permits` goes false. Spell the pass `green` and the
    /// word assertion fails. Suppress the report — the acknowledgement design this
    /// card rejected — and the standing assertion fails.
    #[test]
    fn a_broken_record_permits_the_interval_and_still_tells_the_break() {
        let record = chain_of(&[
            ("r-000001", "b3:GENESIS", "b3:R1"),
            ("r-000002", "b3:SPLICED", "b3:R2"),
        ]);
        let trace = check::journal_trace_of(&record, "b3:R2");
        let pins = clean_pins();
        let gate = Gate {
            label: STAGED,
            accounted: check::interval_accounted(&record, "b3:R2", &record),
            pins: &pins,
            record: &trace,
        };

        assert!(
            gate.permits(),
            "the interval a commit records is accounted for"
        );
        assert_eq!(
            gate.word(),
            ACCOUNTED_UNVOUCHED,
            "and the pass carries the WEAKER word, because the record that let it \
             through cannot vouch for itself"
        );
        assert_ne!(gate.word(), ACCOUNTED, "it is not the full-strength pass");
        let standing = gate.standing_report().expect("a broken record has one");
        assert!(
            standing.contains("r-000002"),
            "the standing report cites the broken row: {standing}"
        );
        assert!(
            standing.contains("does not clear"),
            "and states the permanence rather than implying it: {standing}"
        );
    }

    /// The full-strength pass, and its silence. An intact record has nothing
    /// standing to tell, and a report printed unconditionally would be noise that
    /// teaches operators to skip the one that matters.
    #[test]
    fn an_intact_record_passes_at_full_strength_and_has_nothing_standing_to_tell() {
        let record = chain_of(&[
            ("r-000001", "b3:GENESIS", "b3:R1"),
            ("r-000002", "b3:R1", "b3:R2"),
        ]);
        let trace = check::journal_trace_of(&record, "b3:R2");
        let pins = clean_pins();
        let gate = Gate {
            label: WORKTREE,
            accounted: check::interval_accounted(&record, "b3:R2", &record),
            pins: &pins,
            record: &trace,
        };
        assert!(gate.permits());
        assert_eq!(gate.word(), ACCOUNTED, "nothing weakens this one");
        assert_eq!(
            gate.standing_report(),
            None,
            "and there is no standing fact to report"
        );
    }

    /// **The gate refuses on either gated plane, and says which one spoke.** A
    /// refusal a reader cannot locate is one they cannot act on, so the journal leg
    /// and the pin leg must not answer in each other's words.
    #[test]
    fn the_gate_refuses_on_either_plane_and_names_the_one_that_decided() {
        let record = chain_of(&[("r-000001", "b3:GENESIS", "b3:R1")]);
        let trace = check::journal_trace_of(&record, "b3:R1");

        let pins = clean_pins();
        let journal_leg = Gate {
            label: STAGED,
            accounted: check::interval_accounted(&record, "b3:FORGED", &record),
            pins: &pins,
            record: &trace,
        };
        assert!(!journal_leg.permits(), "a root nothing produced is refused");
        assert_eq!(journal_leg.word(), RED, "read, and found bad");
        assert!(
            journal_leg.detail().contains("b3:FORGED"),
            "citing the fold that matches nothing: {}",
            journal_leg.detail()
        );

        let unreadable = PinPlane {
            cannot_ask: Some("the object store could not be asked".to_string()),
            ..clean_pins()
        };
        let pin_leg = Gate {
            label: STAGED,
            accounted: check::interval_accounted(&record, "b3:R1", &record),
            pins: &unreadable,
            record: &trace,
        };
        assert!(
            pin_leg.accounted.is_accounted(),
            "FIXTURE: the journal plane is satisfied, so only the pin plane can refuse"
        );
        assert!(!pin_leg.permits(), "an UNREAD plane is not a clean one");
        assert_eq!(
            pin_leg.word(),
            GREY_CANNOT_ASSESS,
            "and it refuses as an absence of evidence, not as a finding"
        );
    }

    /// **The empty record refuses without accusing.** Under the ruled cache-resident
    /// journal this is the common case, not an edge one: a fresh clone carries zero
    /// rows and has tampered with nothing.
    #[test]
    fn an_empty_record_refuses_as_an_absence_and_not_as_a_finding() {
        let trace = check::journal_trace_of("", "b3:R1");
        let pins = clean_pins();
        let gate = Gate {
            label: WORKTREE,
            accounted: check::interval_accounted("", "b3:R1", ""),
            pins: &pins,
            record: &trace,
        };
        assert!(!gate.permits(), "unknown is not clean — it fails CLOSED");
        assert_eq!(
            gate.word(),
            GREY_CANNOT_ASSESS,
            "grey, never red: nothing was read, so nobody may be accused"
        );
        assert_ne!(
            gate.word(),
            RED,
            "the fresh-clone path is not a tampering path"
        );
        assert!(
            gate.detail().contains("tampered with nothing"),
            "and the words say so: {}",
            gate.detail()
        );
    }

    #[test]
    fn parse_rejects_stray_positional() {
        assert_eq!(Check::parse(&["extra".to_string()]).unwrap_err().code, 2);
    }
}
