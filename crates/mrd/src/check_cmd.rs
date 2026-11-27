//! `mrd check` — the pure read validity verb.
//!
//! ```text
//! mrd check [--core] [--staged] [--commit-gate] [--json] [-- PATH…]
//! ```
//!
//! Runs the convention-free core (layer 0) over the resolved workspace: observe
//! every claim against the current tree, then read the pin plane — the pin verdicts
//! and the anchoring state of every pinned blob — then the armed plane: the armed
//! rows, and every armed `rules/query` run over the snapshot ([`crate::check_armed`],
//! `docs/armed-plane.md` § Part A3). `-- PATH…` limits query rows and pins to those
//! paths; a rule-level finding counts whatever the paths. `status = freshness, check =
//! validity` — this answers "what lies?", writing nothing and minting no receipt.
//!
//! Write history is not assessed: the engine keeps no memory. `check` answers
//! at-rest truth — does the world still match the pins. Every face carries a
//! `write_history: not-assessed` line naming the narrowing (what the green
//! stopped covering; the WHY is docs/`--help` teaching — report-voice pass).
//!
//! Pin colours come from the one pin computer `mrd status`'s lock axis reads —
//! as rows from the resident daemon's `pins` door (`docs/wire-contract.md`
//! § A.13), or coloured in this process when a staged interval diverges from
//! the worktree, an armed query fires, or no daemon serves the workspace.
//!
//! # The interval this verb spans
//! - **`worktree`** — the bytes on disk. Always assessed.
//! - **`staged`** — the bytes the git index carries, assessed whenever it carries
//!   anything the worktree does not: that is the interval a commit records
//!   (`domain_snapshot` reads the worktree; git commits the index). Both passes run
//!   the same reads over different bytes.
//!
//! The exit is worst-of across both intervals, and every refusal names the
//! interval it came from.
//!
//! `--core` names layer 0 explicitly (the default today).
//!
//! # `--commit-gate`
//! Narrows the exit to one interval — the one a commit records — so a finding from
//! the worktree cannot swamp a clean answer about the bytes being committed. It
//! gates on the pin plane alone; the passing word is `pins-hold`.
//!
//! # `--require-pins`
//! A corpus that declares no pin passes the gate: nothing is unknown because
//! nothing was asked, and over zero pins "does the world still match the pins" is
//! vacuously true. A grey pin or an unaskable object store still fails closed. A
//! caller that wants no-coverage to mean refuse says so with `--require-pins` and
//! gets it in the exit code, under its own word (`no-pin-coverage`, never grey's).
//! Opt-in: a fail-closed default would make the gate un-adoptable on every vault
//! that has not started pinning.
//!
//! Read-only. Exit triad:
//! - **0** — green: every claim converged, every pin holds, every pinned blob is
//!   anchored. Under `--commit-gate`: the interval a commit records holds its pins.
//! - **1** — a finding: a drifted claim, a red pin, or an orphaned blob. Grey
//!   rides this leg too: a grey pin or an unaskable object store refuses
//!   `grey(cannot-assess)` — unknown is not clean.
//! - **2** — bad invocation, or an unreadable workspace.
//!
//! # The fence line
//! `fence:` is a proposition about the local checkout's configuration, not the
//! corpus, and it never touches the exit code ([`Fence`]). `$GIT_DIR/hooks` is
//! never a tracked path, so fence coverage is per-checkout and opt-in; a fresh
//! clone being unfenced is a supported state.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use check::{CoreReport, GREY_CANNOT_ASSESS, PinPlane, PinRow, WRITE_HISTORY_NOT_ASSESSED};
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
#[allow(clippy::too_many_lines)] // top-level verb dispatch: resolve, guard, assess, emit, exit
pub(crate) fn dispatch(args: &[String]) -> Result<(), Fail> {
    let parsed = Check::parse(args)?;
    let cwd = current_dir()?;
    let resolved = crate::resolve::resolve_runtime(workspace::Base::Cwd(&cwd)).map_err(|e| {
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

    let domain = fs::domain::Domain::load(&root)
        .map_err(|e| Fail::tool(format!("cannot read the hash domain: {e}")))?;
    let paths = workspace_paths(&canonical, &cwd, &parsed.paths)?;

    // One-at-a-time non-blocking slot lock + session-lifetime/wall-clock guard
    // (`docs/status.md` § `mrd check`): refuse `busy` immediately when another
    // check holds the slot, and die with the caller or wall-clock bound.
    let _slot = crate::check_guard::acquire_slot(parsed.format, &canonical)?;
    let scoped_gate =
        parsed.commit_gate && !paths.is_empty() && !paths.iter().any(String::is_empty);
    let _watchdog =
        crate::check_guard::Watchdog::arm(crate::check_guard::wall_clock_bound(!paths.is_empty()));

    let (interval, mut worktree, staged, armed) = if scoped_gate {
        let built = assess_scoped(&root, &domain, &paths)?;
        (built.interval, built.worktree, built.staged, None)
    } else {
        // The second interval, only when asked for: absent divergence the two intervals
        // coincide and one pass answers both.
        let probe = if parsed.staged {
            probe_staged(&root, &domain)
        } else {
            StagedProbe::Settled(Interval::NotAsked)
        };
        // The armed plane — red rows and armed queries. The commit gate stays the pin
        // plane alone, so it runs no query.
        let settled_armed = if parsed.commit_gate {
            Some(None)
        } else {
            crate::check_armed::settled_without_corpus(&root).map(Some)
        };

        // Two readings need this process to hold the corpus: a staged interval whose
        // index bytes diverge from the worktree, and an armed query. Every other check
        // takes its pin rows from the resident daemon (`wire-contract.md` § A.13).
        let served = match (&probe, &settled_armed) {
            (StagedProbe::Settled(_), Some(_)) => {
                let served = crate::pins_lane::from_daemon(&canonical, parsed.format)?;
                if served.is_none() {
                    crate::engine::voice_degrade(&crate::engine::EngineSource::Ephemeral);
                }
                served
            }
            _ => None,
        };
        match (served, probe, settled_armed) {
            (Some(plane), StagedProbe::Settled(interval), Some(armed)) => {
                crate::voice_excluded_note(&plane.excluded);
                let worktree = assess_rows(
                    &root,
                    &domain,
                    &plane.rows,
                    &mounted_stores(&plane.rows),
                    &paths,
                )?;
                (interval, worktree, None, armed)
            }
            (_, probe, _) => {
                let built = assess_in_process(&root, &domain, probe, &paths, parsed.commit_gate)?;
                (built.interval, built.worktree, built.staged, built.armed)
            }
        }
    };
    // The run ledger is engine state outside every interval (wire-contract
    // §6.1), so the G3 orphan lint reads it once, from disk, for the worktree.
    worktree.report.orphans = check::orphan::orphaned_runs(&root).map_err(|e| {
        Fail::tool(format!(
            "cannot read the run ledger {}: {e}",
            run::executor::RECEIPT_FILE
        ))
    })?;

    let gate = parsed
        .commit_gate
        .then(|| build_gate(&interval, &worktree, staged.as_ref(), parsed.require_pins));

    let armed_findings = armed
        .as_ref()
        .map(|plane| plane.findings(&paths))
        .unwrap_or_default();

    // The checkout's fence coverage — read here, reported below, reachable from no
    // exit path. Taken on every invocation, gated and ungated alike.
    let fence = observe_fence(&canonical);

    emit(
        parsed.format,
        &canonical,
        &worktree.report,
        &interval,
        staged.as_ref(),
        gate.as_ref(),
        &fence,
        armed.as_ref().map(|plane| (plane, paths.as_slice())),
    );

    // Fail closed on an interval that was asked for and could not be read: degrading
    // silently to the worktree answer would be a true statement about the wrong bytes.
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

    // The scoped exit reads one interval; the worst-of below answers the corpus-wide
    // question. A gated run never reaches it.
    if let Some(gate) = gate.as_ref() {
        return gate.exit();
    }

    worst_of_exit(&worktree.report, staged.as_ref(), &armed_findings)
}

/// The `-- PATH…` operands as workspace-relative paths. Absolute or relative to
/// the cwd; a path that no longer exists (a deleted file in a push) is resolved
/// through its parent, so it still names where it was.
///
/// # Errors
/// [`Fail`] exit 2 for a path outside the workspace or with no UTF-8 spelling.
fn workspace_paths(workspace: &Path, cwd: &Path, raw: &[String]) -> Result<Vec<String>, Fail> {
    raw.iter()
        .map(|given| {
            let joined = cwd.join(given);
            let resolved = std::fs::canonicalize(&joined)
                .or_else(|_| {
                    let name = joined
                        .file_name()
                        .ok_or_else(|| std::io::Error::other("no file name"))?;
                    let parent = joined.parent().unwrap_or(Path::new("/"));
                    std::fs::canonicalize(parent).map(|p| p.join(name))
                })
                .unwrap_or(joined);
            let rel = resolved.strip_prefix(workspace).map_err(|_| {
                Fail::tool(format!(
                    "`{given}` is outside the workspace {} — check reads one workspace",
                    workspace.display()
                ))
            })?;
            rel.to_str()
                .map(|s| s.trim_end_matches('/').to_owned())
                .ok_or_else(|| Fail::tool(format!("`{given}` has no UTF-8 spelling")))
        })
        .collect()
}

/// The corpus-wide question's exit: worst-of across intervals — the interval
/// carrying the worst colour answers. Red and grey refuse on the same leg; the
/// reason word tells a finding from an absence of evidence, and every refusal
/// names its interval. `--commit-gate` never reaches here.
///
/// **The SELECTION is worst-of; the LIST is not.** Once an interval is chosen it
/// refuses with every finding it holds, red then grey, because the exit code is
/// one bit and the enumeration is its reason half: a numbered fix-list that drops
/// the greys sends an operator away believing the corpus clean once the reds are
/// fixed (status.md § The findings enumeration is COMPLETE, never worst-of).
///
/// # Errors
/// [`Fail`] exit 1 on the worst finding across the assessed intervals.
fn worst_of_exit(
    worktree: &CoreReport,
    staged: Option<&Assessed>,
    armed: &[String],
) -> Result<(), Fail> {
    let mut intervals: Vec<(&str, &CoreReport)> = vec![(WORKTREE, worktree)];
    if let Some(staged) = staged {
        intervals.push((STAGED, &staged.report));
    }
    let mut refusals = Vec::new();
    for worst in [CoreReport::is_red, CoreReport::cannot_assess] {
        if let Some((label, report)) = intervals
            .iter()
            .find_map(|(label, report)| worst(report).then_some((*label, *report)))
        {
            refusals.push(refusal_list(label, &all_findings(report)));
            break;
        }
    }
    // The armed plane's findings ride beside the core's, never instead of them.
    if !armed.is_empty() {
        refusals.push(refusal_list(ARMED, &armed.join("\n")));
    }
    if refusals.is_empty() {
        Ok(())
    } else {
        Err(Fail::with_code(EXIT_FINDING, refusals.join("\n")))
    }
}

/// The armed plane's label in a refusal.
const ARMED: &str = "armed";

/// Every finding one interval holds, worst-of ORDERED and never worst-of
/// SELECTED: the red lines first, then the grey ones, each keeping the reason
/// word its own plane spelled.
fn all_findings(report: &CoreReport) -> String {
    [report.red_summary(), report.grey_summary()]
        .into_iter()
        .flatten()
        .collect::<Vec<String>>()
        .join("\n")
}

/// The refusal line for a summary that carries N findings: a count, then one
/// finding per line, numbered.
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

/// Which interval a commit records, and the scoped question put to it. When the index
/// diverges that is the staged bytes; when it coincides, or there is no repository at
/// all, the worktree is that interval. Either way one interval answers.
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
#[allow(clippy::too_many_arguments)] // one writer for both faces, fed every reading
fn emit(
    format: Format,
    workspace: &Path,
    worktree: &CoreReport,
    interval: &Interval,
    staged: Option<&Assessed>,
    gate: Option<&Gate<'_>>,
    fence: &Fence,
    armed: Option<(&crate::check_armed::ArmedPlane, &[String])>,
) {
    match format {
        Format::Json => {
            let mut value = to_json(workspace, worktree, interval, staged, gate, fence);
            if let Some((plane, paths)) = armed {
                let (rules, rows) = plane.json(paths);
                value["red"] = Value::Bool(
                    value["red"].as_bool().unwrap_or(false) || !plane.findings(paths).is_empty(),
                );
                value["rules"] = rules;
                value["armed_rows"] = rows;
                value["paths"] = json!(paths);
            }
            println!("{}", serde_json::to_string_pretty(&value).expect("json"));
        }
        Format::Human => {
            let mut armed_lines = String::new();
            if let Some((plane, paths)) = armed {
                if !paths.is_empty() {
                    let _ = writeln!(
                        armed_lines,
                        "  paths: {} — query rows and pins count under these paths only",
                        crate::capped_sample(paths)
                    );
                }
                armed_lines.push_str(&plane.render(paths, staged.is_some()));
            }
            print!(
                "{}",
                render_human(
                    workspace,
                    worktree,
                    interval,
                    staged,
                    gate,
                    fence,
                    &armed_lines
                )
            );
        }
    }
}

/// The interval a worktree read spans — the bytes on disk.
const WORKTREE: &str = "worktree";

/// The interval a commit spans — the bytes the index carries. One name for it, in
/// the human render, the `--json` face and every refusal.
const STAGED: &str = "staged";

/// The finding colour, spelled once — the chain line, the pin lines and the gate's
/// verdict word all mean the same thing by it.
const RED: &str = "RED";

/// The gated pass. Names the plane that actually answered, and cannot be misread
/// as a claim about write history.
const PINS_HOLD: &str = "pins-hold";

/// What `--commit-gate` reads: one interval decides the exit — the one a commit
/// records — because a finding from the worktree interval would swamp a clean
/// answer about the bytes being committed.
struct Gate<'a> {
    /// Which interval the exit reads. Named in the render and in every refusal.
    label: &'a str,
    /// That interval's pin plane — the only gated plane. A pin is a claim about the bytes being
    /// committed, so it belongs to the interval, not to the history.
    pins: &'a PinPlane,
    /// The caller asked for fail-closed-on-no-coverage (`--require-pins`). Off by
    /// default; see [`NO_COVERAGE`].
    require_pins: bool,
}

/// The refusal word for a corpus that declares no pin at all, under
/// `--require-pins`. Distinct from [`GREY_CANNOT_ASSESS`] on purpose: grey means
/// a question was put and could not be answered, and here none was put.
const NO_COVERAGE: &str = "no-pin-coverage";

impl Gate<'_> {
    /// May this commit proceed? An unread plane is not a clean one. Zero pins is
    /// vacuous truth, not unknown, so the default passes and `--require-pins` is the
    /// caller's opt-in refusal.
    fn permits(&self) -> bool {
        !self.pins.is_red() && !self.pins.cannot_assess() && !self.uncovered()
    }

    /// The corpus declares no pin, and the caller asked to refuse exactly that.
    fn uncovered(&self) -> bool {
        self.require_pins && self.pins.declared == 0
    }

    /// The verdict word — what was read, and what it said.
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
                 anchored; write history is not assessed"
                .to_string();
        }
        // Before the pin findings, because it is not one: there is nothing in this
        // corpus to be right or wrong.
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
    /// `1` they do not, or could not be assessed; `2` never comes from here, since
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

// ── the fence line — the checkout, not the corpus ─────────────────────────────

/// The clause every fence line carries, spelled once so the two faces cannot drift
/// apart on the claim this reading rests on.
const FENCE_REPORTED: &str = "REPORTED, never gated on — fence coverage is a property of this \
                              local checkout and not of the corpus, so this line does not move \
                              check's exit";

/// What the local checkout's fence looks like — a reading beside the verdict, never
/// part of it. Fence coverage is a proposition about the local checkout's
/// configuration, a different subject on a different axis.
struct Fence {
    /// The one word for this checkout: [`hook::Coverage::word`] when the root can carry a fence,
    /// [`hook::Unfenceable::word`] when it cannot. Never re-spelled here.
    word: &'static str,
    /// What was observed, and what can be done about it.
    teaching: String,
    /// The door plane, or `None` when this root has none. Doors can disagree, so the
    /// set's one word is not the whole reading.
    ///
    /// `None` rather than an empty list for a root the fence cannot reach: a
    /// submodule or a non-repository has no hook directory to read, which is not
    /// the same fact as a hook directory read and found empty.
    doors: Option<Vec<FenceDoor>>,
    /// How many of those doors carry a fence this engine's line wrote — the coverage axis,
    /// blind to currency ([`hook::Coverage::fenced_doors`]); whether a door is current is
    /// [`Fence::word`]'s claim, never this one's.
    fenced: usize,
}

/// One door of the install set, as this face reports it.
struct FenceDoor {
    /// The hook's git name — one of [`hook::FENCED_HOOKS`].
    name: &'static str,
    /// This door's own state word, from [`hook::Door::word`].
    word: &'static str,
    /// The generation this file declares, never the asking engine's.
    version: Option<u32>,
}

/// Read the checkout's fence state. It cannot fail into the exit: every outcome of
/// the survey is a [`Fence`] to report, so no branch here produces a value a caller
/// could turn into a [`Fail`].
fn observe_fence(workspace: &Path) -> Fence {
    match hook::status(workspace) {
        Ok(coverage) => {
            let fenced = coverage.fenced_doors();
            Fence {
                word: coverage.word(),
                // `Coverage::teaching` is silent on the two agreed states; this reader did not ask
                // for a fence report, so those states are worded here.
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
        // A root the fence cannot reach is reported with its observed reason word and its
        // teaching — never as a bare absence, and never as a refusal.
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
    // The count is already in the line; each of these says the why the count cannot.
    if fenced == 0 {
        "`$GIT_DIR/hooks` is never a tracked path, so no clone, fetch or pull carries a fence \
         and a fresh checkout is unfenced BY DESIGN — `mrd skill hook` emits what to place to \
         fence this one, per checkout and opt-in"
            .to_owned()
    } else {
        "this checkout is fully fenced, at the generation this engine writes".to_owned()
    }
}

/// The fence line(s) on the human face: the checkout's coverage, then the doors when
/// it has any. Two lines rather than one — the set's word cannot carry which door
/// disagrees. On the first line the count is coverage, the set word is currency.
fn fence_lines(fence: &Fence) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    // The teachings come from two different types and only some of them end in a full stop;
    // trimming here keeps one render grammar over both.
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
        // No door plane, so no door line: an absence is not an empty reading of something.
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
/// The door-plane keys are absent when there is no door plane — this face's own law,
/// stated at [`interval_json`]: an absent field reads as "not checked". A `null`
/// would say the doors were read and came back as nothing.
fn fence_json(fence: &Fence) -> Value {
    let mut value = json!({
        "state": fence.word,
        "fenceable": fence.doors.is_some(),
        "teaching": fence.teaching,
        // What this engine writes, beside what the files declare (per door below).
        "engine_version": hook::FENCE_VERSION,
        // Machine-readable: a consumer must be able to read off this face that the
        // block did not decide the exit.
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
        // The population beside the reading: "one door fenced" means one thing out
        // of three and something else out of one.
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

/// What was asked about the second interval, and what came back. Each outcome is a
/// different fact about this run, so none is rendered as another's silence.
enum Interval {
    /// `--staged` was not passed: the index was never read, and this answer is about
    /// the worktree only — said out loud in the render.
    NotAsked,
    /// Asked, and this workspace is not a git repository, so no index exists to record
    /// anything. A supported state, not a degradation.
    NoRepository,
    /// Asked, and the index carries nothing the worktree does not — one pass
    /// answers both intervals. A fact about this run, not an absence.
    Coincides,
    /// Asked, and the index carries other bytes for these paths.
    Diverges(StagedBytes),
    /// Asked inside a repository, and git could not be asked. Refuses.
    CannotAsk(String),
}

/// The staged interval's own bytes: the overlaid snapshot and the paths that diverge.
struct StagedBytes {
    paths: Vec<String>,
    files: fs::DomainFiles,
}

/// Everything the in-process lane read.
struct Built {
    interval: Interval,
    worktree: Assessed,
    staged: Option<Assessed>,
    armed: Option<crate::check_armed::ArmedPlane>,
}

/// Bounded path-scoped `--commit-gate -- PATH…`: reads only the markdown
/// holders under `paths` and the direct targets their `meridian-lock` blocks
/// pin (ambient and bound mounted roots), without scanning or parsing the whole
/// corpus or entering the `.build` gate.
#[allow(clippy::too_many_lines)] // linear staged+worktree assembly over bounded paths
fn assess_scoped(
    root: &fs::WorkspaceRoot,
    domain: &fs::domain::Domain,
    paths: &[String],
) -> Result<Built, Fail> {
    let scoped_phase = timing::phase("check.scoped");
    let mut worktree_holder_paths = BTreeSet::new();
    for rel in paths {
        collect_md_under(&root.0, rel, &mut worktree_holder_paths);
    }

    let repo = git::Repo::at(&root.0);
    let mut div_map: BTreeMap<String, Option<Vec<u8>>> = BTreeMap::new();
    let mut git_status: Option<Interval> = None;
    match repo.staged_divergence_paths(paths) {
        Ok(entries) => {
            for (rel, bytes) in entries {
                div_map.insert(rel, bytes);
            }
        }
        Err(git::GitFail::NotARepo { .. }) => git_status = Some(Interval::NoRepository),
        Err(other) => git_status = Some(Interval::CannotAsk(other.to_string())),
    }

    let mut unserved: BTreeMap<String, String> = BTreeMap::new();
    let mut worktree_in_domain: model::Docs = BTreeMap::new();
    let mut excluded_holders: model::Docs = BTreeMap::new();
    for rel in &worktree_holder_paths {
        let Ok(bytes) = std::fs::read(root.0.join(rel)) else {
            continue;
        };
        if domain.contains(Path::new(rel)) {
            parse_scoped_doc(rel, bytes, &mut worktree_in_domain, &mut unserved);
        } else if let Ok(text) = String::from_utf8(bytes) {
            let doc = model::build(text.clone(), syntax::parse(&text));
            excluded_holders.insert(rel.clone(), std::sync::Arc::new(doc));
        }
    }

    let mut staged_in_domain: model::Docs = worktree_in_domain.clone();
    for (rel, opt_bytes) in &div_map {
        if !is_md_path(rel) || !domain.contains(Path::new(rel)) {
            continue;
        }
        match opt_bytes {
            Some(bytes) => {
                parse_scoped_doc(rel, bytes.clone(), &mut staged_in_domain, &mut unserved);
            }
            None => {
                staged_in_domain.remove(rel);
            }
        }
    }

    let (mount_set, bound_roots) = bind_mount_table_headers();
    let mut ambient_targets: BTreeSet<String> = BTreeSet::new();
    let mut mounted_targets: BTreeMap<addr::MountName, BTreeSet<String>> = BTreeMap::new();
    for doc in worktree_in_domain
        .values()
        .chain(staged_in_domain.values())
        .chain(excluded_holders.values())
    {
        for item in view::read_face::page_lock_items(doc) {
            if item.lock_refusal.is_some() {
                continue;
            }
            let Ok(parsed_addr) = addr::Addr::parse(&item.to_path) else {
                continue;
            };
            match parsed_addr.root() {
                None => {
                    ambient_targets.insert(item.to_path);
                }
                Some(mount_name) => {
                    if bound_roots.contains_key(mount_name) {
                        mounted_targets
                            .entry(mount_name.clone())
                            .or_default()
                            .insert(parsed_addr.path().to_owned());
                    }
                }
            }
        }
    }

    if git_status.is_none() && !ambient_targets.is_empty() {
        let target_list: Vec<String> = ambient_targets.iter().cloned().collect();
        match repo.staged_divergence_paths(&target_list) {
            Ok(entries) => {
                for (rel, bytes) in entries {
                    div_map.entry(rel).or_insert(bytes);
                }
            }
            Err(other) => git_status = Some(Interval::CannotAsk(other.to_string())),
        }
    }

    let diverged_paths: Vec<String> = div_map
        .keys()
        .filter(|rel| domain.contains(Path::new(rel)))
        .cloned()
        .collect();
    let interval = match git_status {
        Some(settled) => settled,
        None if diverged_paths.is_empty() => Interval::Coincides,
        None => Interval::Diverges(StagedBytes {
            paths: diverged_paths.clone(),
            files: fs::DomainFiles::default(),
        }),
    };

    let mut worktree_docs = worktree_in_domain.clone();
    for rel in &ambient_targets {
        if worktree_docs.contains_key(rel) || !domain.contains(Path::new(rel)) {
            continue;
        }
        let full = root.0.join(rel);
        if full.is_file()
            && let Ok(bytes) = std::fs::read(&full)
        {
            parse_scoped_doc(rel, bytes, &mut worktree_docs, &mut unserved);
        }
    }
    crate::voice_unserved(&unserved);

    let staged_docs = matches!(interval, Interval::Diverges(_)).then(|| {
        let mut docs = staged_in_domain.clone();
        for rel in &ambient_targets {
            if !domain.contains(Path::new(rel)) {
                continue;
            }
            if let Some(opt_bytes) = div_map.get(rel) {
                match opt_bytes {
                    Some(bytes) => {
                        let mut staged_unserved = BTreeMap::new();
                        parse_scoped_doc(rel, bytes.clone(), &mut docs, &mut staged_unserved);
                    }
                    None => {
                        docs.remove(rel);
                    }
                }
            } else if let Some(doc) = worktree_docs.get(rel) {
                docs.entry(rel.clone())
                    .or_insert_with(|| std::sync::Arc::clone(doc));
            }
        }
        docs
    });

    let mut mounted_docs: BTreeMap<addr::MountName, model::Docs> = BTreeMap::new();
    let mut mounted_stores: BTreeMap<addr::MountName, PathBuf> = BTreeMap::new();
    for (name, targets) in &mounted_targets {
        let mount_root = &bound_roots[name];
        mounted_stores.insert(name.clone(), mount_root.clone());
        let mount_ws = fs::WorkspaceRoot(mount_root.clone());
        let Ok(mount_domain) = fs::domain::Domain::load(&mount_ws) else {
            continue;
        };
        let mut docs: model::Docs = BTreeMap::new();
        let mut mount_unserved = BTreeMap::new();
        for rel in targets {
            if !mount_domain.contains(Path::new(rel)) {
                continue;
            }
            let full = mount_root.join(rel);
            if full.is_file()
                && let Ok(bytes) = std::fs::read(&full)
            {
                parse_scoped_doc(rel, bytes, &mut docs, &mut mount_unserved);
            }
        }
        crate::voice_unserved(&mount_unserved);
        mounted_docs.insert(name.clone(), docs);
    }

    let rows_for = |docs: &model::Docs, holders: &model::Docs| {
        let mut corpus = model::RootedCorpus::ambient(docs)
            .with_hash_domain(domain)
            .with_ambient_disk(root);
        for (name, mdocs) in &mounted_docs {
            corpus = corpus.with_root(name.clone(), mdocs);
        }
        let colouring = timing::phase("check.pins");
        let colors = view::walk::lock_pin_colors_for_holders(
            &corpus,
            &mount_set,
            holders
                .iter()
                .chain(excluded_holders.iter())
                .map(|(p, d)| (p.as_str(), d.as_ref())),
        );
        colouring.stop();
        crate::pins_lane::rows(colors)
    };

    let worktree = assess_rows(
        root,
        domain,
        &rows_for(&worktree_docs, &worktree_in_domain),
        &mounted_stores,
        paths,
    )?;
    let staged = match (&interval, &staged_docs) {
        (Interval::Diverges(_), Some(docs)) => Some(Assessed {
            paths: diverged_paths,
            report: assess_rows(
                root,
                domain,
                &rows_for(docs, &staged_in_domain),
                &mounted_stores,
                paths,
            )?
            .report,
        }),
        _ => None,
    };
    scoped_phase.stop();
    Ok(Built {
        interval,
        worktree,
        staged,
        armed: None,
    })
}

fn is_md_path(rel: &str) -> bool {
    Path::new(rel)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
}

fn collect_md_under(root: &Path, rel: &str, out: &mut BTreeSet<String>) {
    let full = root.join(rel);
    if full.is_file() {
        if is_md_path(rel) {
            out.insert(rel.to_owned());
        }
        return;
    }
    if !full.is_dir() {
        return;
    }
    let mut stack = vec![full];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            if name == ".git" {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else if path.is_file()
                && let Ok(stripped) = path.strip_prefix(root)
                && let Some(text) = stripped.to_str()
                && is_md_path(text)
            {
                out.insert(text.to_owned());
            }
        }
    }
}

fn parse_scoped_doc(
    rel: &str,
    bytes: Vec<u8>,
    docs: &mut model::Docs,
    unserved: &mut BTreeMap<String, String>,
) {
    match String::from_utf8(bytes) {
        Ok(text) => {
            let doc = model::build(text.clone(), syntax::parse(&text));
            docs.insert(rel.to_owned(), std::sync::Arc::new(doc));
        }
        Err(_) => {
            unserved.insert(rel.to_owned(), "is not valid UTF-8".to_owned());
        }
    }
}

/// Bind `MERIDIAN.md`'s mount table into `(MountSet, bound_roots)` without
/// reading or parsing any mounted root's corpus.
fn bind_mount_table_headers() -> (addr::MountSet, BTreeMap<addr::MountName, PathBuf>) {
    let env = config::Env::from_process();
    let Ok(resolution) = config::resolve(&env) else {
        return (addr::MountSet::default(), BTreeMap::new());
    };
    let Ok(table) = resolution.bind(&env) else {
        return (addr::MountSet::default(), BTreeMap::new());
    };
    let mut bound = Vec::new();
    let mut unreachable = Vec::new();
    let mut roots = BTreeMap::new();
    for mount in table.mounts() {
        let Ok(name) = addr::MountName::parse(mount.name()) else {
            continue;
        };
        if mount.state().refuses() {
            let detail = match mount.state() {
                config::mount::MountState::PathUnseeable { detail } => detail.clone(),
                other => other.detail(),
            };
            unreachable.push((name, mount.declared_path().to_owned(), detail));
            continue;
        }
        let Some(path) = mount.canonical_path() else {
            continue;
        };
        bound.push(name.clone());
        roots.insert(name, path.to_path_buf());
    }
    let mut set = addr::MountSet::new(bound);
    for (name, path, detail) in unreachable {
        set = set.with_unreachable(name, path, detail);
    }
    (set, roots)
}

/// The in-process lane: one read of the worktree, whose bytes feed the fold, the
/// corpus, every interval's pin rows and the armed queries.
fn assess_in_process(
    root: &fs::WorkspaceRoot,
    domain: &fs::domain::Domain,
    probe: StagedProbe,
    paths: &[String],
    commit_gate: bool,
) -> Result<Built, Fail> {
    crate::build_gate::enter();
    // The same walk lists the markdown the domain does NOT carry: the note
    // below and the out-of-domain pin holders both read that one list.
    let (worktree_files, worktree_fold, outside) = fs::domain_snapshot_with_excluded(root)
        .map_err(|e| Fail::tool(format!("cannot read the corpus: {e}")))?;
    let interval = staged_interval(probe, domain, &worktree_files);

    // The corpora are built before the mount table, because they say which roots the
    // table must build; each interval's corpus is built exactly once.
    let worktree_docs = build_corpus(worktree_files);
    // `check` assesses every pin the corpus carries — a population the caller
    // did not name — so it owes the enumerator clause (§12.1): it may exclude
    // what its attestation cannot reach, never SILENTLY. Voiced once, for the
    // worktree interval: the excluded population is domain-derived, and both
    // intervals stand under the same domain.
    crate::voice_excluded_listed(&outside, &worktree_docs);
    let staged_docs = match &interval {
        Interval::Diverges(bytes) => Some(build_corpus(bytes.files.clone())),
        _ => None,
    };

    // The real mount table, through the one loader `mrd walk` uses. The table is whole;
    // only the corpora narrow, to the roots this check's own lock addresses name.
    let mut needed = crate::walk_cmd::lock_addressed_roots(&worktree_docs);
    if let Some(docs) = &staged_docs {
        needed.append(&mut crate::walk_cmd::lock_addressed_roots(docs));
    }
    let mounts = crate::walk_cmd::load_mounts_for(&needed);
    // D-F: each MOUNTED root's store answers durability for the form-3 rows
    // that name it — the map is the loaded corpora's own (name, bound path)
    // pairs, so the store asked is the tree the row was coloured against.
    let mounted: BTreeMap<addr::MountName, PathBuf> = mounts
        .corpora
        .iter()
        .map(|m| (m.name.clone(), m.root.clone()))
        .collect();

    // The pins the workspace DECLARES outside the hash domain. Read once: the
    // two intervals differ in which bytes were HASHED, and an excluded page's
    // bytes are hashed in neither.
    //
    // ⚠️ DECLARED BLIND SPOT: these bytes come from the WORKTREE for both
    // intervals. A holder that is BOTH domain-excluded AND staged-modified is
    // read at its worktree bytes under `--commit-gate`, so a pin ROW added or
    // removed in the index alone is not seen. Strictly more than today, where
    // the row does not exist at either interval; named rather than assumed.
    let excluded = excluded_holders(root, &outside);

    // One interval's rows: coloured through the one computer with the real mount
    // table, under the same domain its snapshot was taken under.
    let rows_of = |docs: &model::Docs| {
        let corpus = mounts.rooted(docs, domain, root);
        let colouring = timing::phase("check.pins");
        let colors =
            view::walk::lock_pin_colors_rooted_with_sources(&corpus, mounts.set(), &excluded);
        colouring.stop();
        crate::pins_lane::rows(colors)
    };
    let worktree = assess_rows(root, domain, &rows_of(&worktree_docs), &mounted, paths)?;
    let staged = match (&interval, &staged_docs) {
        (Interval::Diverges(bytes), Some(docs)) => Some(Assessed {
            paths: bytes.paths.clone(),
            report: assess_rows(root, domain, &rows_of(docs), &mounted, paths)?.report,
        }),
        _ => None,
    };
    // The armed plane over the worktree snapshot.
    let armed = (!commit_gate)
        .then(|| crate::check_armed::assess(root, &worktree_docs, domain, &worktree_fold.0));
    Ok(Built {
        interval,
        worktree,
        staged,
        armed,
    })
}

/// Assess one interval off its pin rows — the resident daemon's
/// (`wire-contract.md` § A.13) or this process's own, one shape: sort the
/// colours and run the layer-0 core's anchoring questions.
fn assess_rows(
    root: &fs::WorkspaceRoot,
    domain: &fs::domain::Domain,
    rows: &[wire::PinRow],
    mounted: &BTreeMap<addr::MountName, PathBuf>,
    paths: &[String],
) -> Result<Assessed, Fail> {
    let mut pins = Vec::new();
    // `-- PATH…`: a pin counts where its holding page lies under the paths.
    for row in rows
        .iter()
        .filter(|row| crate::check_armed::under(paths, &row.holder))
    {
        let class =
            check::PinClass::of_words(&row.color, row.reason.as_deref()).ok_or_else(|| {
                Fail::tool(format!(
                    "the pin row of `{}` carries the colour `{}`, which this build does not know",
                    row.holder, row.color
                ))
            })?;
        pins.push(PinRow {
            src_path: row.holder.clone(),
            declared_ref: crate::pins_lane::declared_ref(row),
            class,
            label: crate::pins_lane::label(row),
        });
    }
    // The anchoring questions are asked for the hashed corpus's holders under
    // `paths` only — matching the pin rows above so `-- PATH…` never scans git
    // history for unrelated cards across the corpus.
    let objects: Vec<check::LockObject> = rows
        .iter()
        .filter(|row| {
            crate::check_armed::under(paths, &row.holder)
                && crate::pins_lane::holder_in_domain(domain, row)
        })
        .filter_map(|row| {
            Some(check::LockObject {
                src_path: row.holder.clone(),
                key: row.object.clone()?,
                hash: row.hash.clone()?,
            })
        })
        .collect();
    let core = timing::phase("check.core");
    let report = check::core_of(root, &objects, &pins, mounted);
    core.stop();
    Ok(Assessed {
        paths: Vec::new(),
        report,
    })
}

/// Each bound root the rows' objects name, with its path — the stores the
/// anchoring questions are asked of when the daemon coloured the rows.
fn mounted_stores(rows: &[wire::PinRow]) -> BTreeMap<addr::MountName, PathBuf> {
    let needed: BTreeSet<addr::MountName> = rows
        .iter()
        .filter_map(|row| {
            addr::Addr::parse(row.object.as_deref()?)
                .ok()?
                .root()
                .cloned()
        })
        .collect();
    if needed.is_empty() {
        return BTreeMap::new();
    }
    let env = config::Env::from_process();
    let Some(table) = config::resolve(&env)
        .ok()
        .and_then(|resolution| resolution.bind(&env).ok())
    else {
        return BTreeMap::new();
    };
    table
        .mounts()
        .iter()
        .filter_map(|mount| {
            let name = addr::MountName::parse(mount.name()).ok()?;
            if !needed.contains(&name) || mount.state().refuses() {
                return None;
            }
            Some((name, mount.canonical_path()?.to_path_buf()))
        })
        .collect()
}

/// The markdown under the root the hash domain does NOT carry, parsed as pin
/// SOURCES and nothing else.
///
/// `mrd pin` admits an out-of-domain holder and mints the pin at exit 0, so
/// these pages hold claims the workspace has made. Reading the pin plane from
/// the hashed corpus alone lets `--commit-gate` assert *every pin in the
/// interval holds* over a population it narrowed, and answer green over a
/// drifted pin (`docs/status.md`, the pin-population clause).
///
/// The exclusion is decided by the SAME predicate that decides the population —
/// `domain.contains` — and never by a path shape. A path-shaped test over a
/// content-defined population is blind to whatever the content rule excludes
/// that the path rule does not: the dot-segment floor and the
/// `meridian/domain.md` ignore list are two classes of one exclusion, and a
/// reading keyed to the first silently drops the second.
///
/// An unreadable member is SKIPPED here rather than refusing the verb: this
/// widens a population the caller already gets nothing from, so a failure to
/// read one leaves the assessment exactly where it stands today. It is never
/// counted as an absence of pins.
fn excluded_holders(root: &fs::WorkspaceRoot, outside: &[fs::ExcludedMarkdown]) -> model::Docs {
    let reading = timing::phase("check.excluded");
    let mut files: fs::DomainFiles = Vec::new();
    for md in outside {
        let Ok(bytes) = std::fs::read(root.0.join(&md.rel)) else {
            continue;
        };
        files.push((md.rel.clone(), bytes));
    }
    let (_index, docs, _unserved) = fs::build_corpus(files);
    reading.stop();
    docs
}

/// One interval's bytes, parsed into the corpus both the root scan and the assessment
/// read. Split out of [`assess`], which needs the corpus in hand before the mount
/// table exists. The unserved map is voiced right here and never leaves: the
/// domain-excluded census enumerates the declined class from the root
/// ([`crate::voice_excluded`]), which is structurally disjoint from it.
fn build_corpus(files: fs::DomainFiles) -> model::Docs {
    let (_index, docs, unserved) = fs::build_corpus(files);
    crate::voice_unserved(&unserved);
    docs
}

/// What git says about the interval a commit spans, before any corpus is read.
///
/// `git` commits the index, not the worktree — the two part company on `git add` +
/// edit, `git add -p`, `git commit <pathspec>`, `git stash`, and any concurrent
/// writer between `git add` and hook fire. A workspace that is not a git repository
/// has one interval, a supported state.
enum StagedProbe {
    /// The interval is settled without the index's bytes.
    Settled(Interval),
    /// The index carries other bytes for these domain paths.
    Diverges {
        divergence: Vec<git::StagedPath>,
        paths: Vec<String>,
    },
}

fn probe_staged(root: &fs::WorkspaceRoot, domain: &fs::domain::Domain) -> StagedProbe {
    let repo = git::Repo::at(&root.0);
    let divergence = match repo.staged_divergence() {
        Ok(divergence) => divergence,
        // No repository, no index, nothing a commit here could record.
        Err(git::GitFail::NotARepo { .. }) => return StagedProbe::Settled(Interval::NoRepository),
        // Inside a repository and git could not answer — never the worktree answer
        // wearing a wider label.
        Err(other) => return StagedProbe::Settled(Interval::CannotAsk(other.to_string())),
    };
    let paths = divergence
        .iter()
        .filter(|(rel, _)| domain.contains(Path::new(rel)))
        .map(|(rel, _)| rel.clone())
        .collect::<Vec<_>>();
    if paths.is_empty() {
        // Nothing diverges, or everything that does is outside the hash domain —
        // code, assets, a lock file — so there is nothing here a second assessment
        // could say.
        return StagedProbe::Settled(Interval::Coincides);
    }
    StagedProbe::Diverges { divergence, paths }
}

/// The interval the commit spans: the worktree snapshot with the index's bytes
/// overlaid, or the settled interval when the index carries nothing the worktree
/// does not.
fn staged_interval(
    probe: StagedProbe,
    domain: &fs::domain::Domain,
    worktree_files: &fs::DomainFiles,
) -> Interval {
    match probe {
        StagedProbe::Settled(interval) => interval,
        StagedProbe::Diverges { divergence, paths } => {
            let (files, _fold) = fs::overlay_snapshot(worktree_files, &divergence, domain);
            Interval::Diverges(StagedBytes { paths, files })
        }
    }
}

/// The parsed `check` invocation: the output format (the `--core` flag names
/// layer 0 explicitly, the default today, so it carries no extra state).
#[derive(Debug)]
struct Check {
    format: Format,
    /// Also assess the interval a commit would record — the git index.
    ///
    /// Off by default: outside a commit the index is a staging area mid-edit, not a
    /// claim about anything. The pre-commit fence passes this flag, because at that
    /// instant the index is what is being committed.
    staged: bool,
    /// Ask the per-commit question instead of the corpus-wide one, and gate the exit
    /// on the interval a commit records alone. Implies [`Check::staged`] — there is
    /// no coherent commit gate without the index's interval.
    commit_gate: bool,
    /// Refuse a corpus that declares no pin (`--require-pins`). Opt-in, and only
    /// meaningful beside [`Check::commit_gate`] — passing it alone is refused as a
    /// bad invocation rather than silently ignored.
    require_pins: bool,
    /// The `-- PATH…` operands, as given: query rows and pins count under them
    /// only. Empty means the whole workspace.
    paths: Vec<String>,
}

impl Check {
    fn parse(args: &[String]) -> Result<Self, Fail> {
        let mut json = false;
        let mut staged = false;
        let mut commit_gate = false;
        let mut require_pins = false;
        let mut paths = Vec::new();
        let mut operands = false;
        for arg in args {
            if operands {
                paths.push(arg.clone());
                continue;
            }
            match arg.as_str() {
                "--" => operands = true,
                "--json" => json = true,
                // `--core` names layer 0 explicitly; it is the default today, so it
                // is accepted and needs no separate branch.
                "--core" => {}
                // git's own word for the index, deliberately (`git diff --staged`).
                "--staged" => staged = true,
                // The question, not a second interval: it names the caller — a
                // pre-commit gate — rather than a mechanism.
                "--commit-gate" => {
                    commit_gate = true;
                    staged = true;
                }
                // The strictness the caller chooses, kept a separate word from the
                // question itself: `--commit-gate` says which question, this says how
                // strict the answer must be.
                "--require-pins" => require_pins = true,
                flag if flag.starts_with('-') => {
                    return Err(Fail::tool(format!("unknown flag: {flag}")));
                }
                value => {
                    return Err(Fail::tool(format!("unexpected argument: {value}")));
                }
            }
        }
        // Refused rather than ignored: a caller who typed `--require-pins` believes
        // a gate is being tightened.
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
            paths,
        })
    }
}

/// Render the core verdict as a human block: the header, the interval line(s), the
/// write-history disclosure, one line per drifted claim, the pin plane, then the gate
/// and fence lines.
fn render_human(
    workspace: &Path,
    worktree: &CoreReport,
    interval: &Interval,
    staged: Option<&Assessed>,
    gate: Option<&Gate<'_>>,
    fence: &Fence,
    armed: &str,
) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(out, "check core {}", workspace.display());
    let _ = writeln!(out, "  interval: {}", interval_line(interval));
    out.push_str(&render_report(worktree));
    if let Some(staged) = staged {
        out.push_str(&staged_interval_line(&staged.paths));
        out.push_str(&staged_predicate_line());
        out.push_str(&render_report(&staged.report));
    }
    // The gate block adds to the readings rather than replacing them: it says which
    // interval the exit answered for, without hiding a break it does not block on.
    if let Some(gate) = gate {
        let _ = writeln!(out, "  commit-gate: {} — {}", gate.word(), gate.detail());
        let _ = writeln!(
            out,
            "  gated on: {} — the interval a commit records, and nothing else; every other \
             reading above is REPORTED and gates nothing",
            gate.label
        );
    }
    // The armed plane: red rows and armed queries (`crate::check_armed`).
    out.push_str(armed);
    // The fence line comes last, after every reading that could have decided the
    // exit, because it is the one reading that could not have.
    out.push_str(&fence_lines(fence));
    out
}

/// State the interval whenever you state the check. The line is unconditional: a
/// reader may never have to infer which bytes a verdict rested on.
fn interval_line(interval: &Interval) -> String {
    match interval {
        Interval::NotAsked => format!(
            "{WORKTREE} — the bytes on disk. The git INDEX was not read, so this says nothing \
             about what a commit would record: `mrd check --staged` asks that question for pins"
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

/// The staged interval's own line, naming the paths a commit would record.
///
/// Extracted so a test can hold the line's promise: the PROSE sample is capped
/// by [`crate::capped_sample`], the COUNT is not (it rides [`interval_line`]
/// above, which prints `paths.len()` for the same population), and the complete
/// set stays on the machine answer as `diverged_paths`. A staged commit is
/// unbounded — a rebase or a generated tree stages thousands of paths — so the
/// uncapped join here was the same shape decision 0017 ended elsewhere
/// (card cap-convention-audit).
fn staged_interval_line(paths: &[String]) -> String {
    format!(
        "  interval: {STAGED} — the bytes a commit would record for {}\n",
        crate::capped_sample(paths)
    )
}

/// The predicate the staged interval asserts: the pin plane, over the bytes a commit
/// records.
fn staged_predicate_line() -> String {
    format!(
        "  {STAGED} asserts: every pin this corpus declares still holds against THESE bytes — \
         the ones a commit would record, which are not necessarily the ones on disk. Write \
         history is not assessed here either: the engine keeps no memory, so this says nothing \
         about how these bytes came to be staged\n"
    )
}

/// One interval's verdict lines — the disclosure, the claims, and the pin plane.
/// Shared by both intervals so a line can never exist for one and not the other.
fn render_report(report: &CoreReport) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();

    // The disclosure is mandatory: dropping it would leave a reader carrying the old,
    // wider green forward. It states the NARROWING — what the green stopped covering,
    // and where the answer lives — never the engine's mechanism: the WHY (the engine
    // keeps no memory) is docs/status.md's and `mrd --help`'s to teach, on demand,
    // not a footnote charged to every invocation (the report-voice law).
    let _ = writeln!(
        out,
        "  write_history: {WRITE_HISTORY_NOT_ASSESSED} — chain continuity and \
         last-receipt-vs-live are not checked; history is in git"
    );

    for claim in &report.drifted_claims {
        let _ = writeln!(
            out,
            "  claim not realised: {} — {}",
            claim.selector, claim.detail
        );
    }

    // ── the pin plane ───────────────────────────────────────────────────────
    // Two lines, always both present, because their silences mean different things:
    // `pins:` reads the claim plane (did the content drift) and `anchoring:` reads
    // the retrieval plane (is the blob durably held).
    let pins = &report.pins;
    if pins.red.is_empty() && pins.grey.is_empty() && pins.unattested.is_empty() {
        let _ = writeln!(out, "  pins: green");
    } else {
        // Every row, gating and not — the enumeration is COMPLETE, never
        // worst-of. The unattested rows are NAMED here and gate nothing
        // (§12.1 enumerator clause); their own reason word says which they are.
        for pin in pins.red.iter().chain(pins.every_grey()) {
            let _ = writeln!(out, "  pins: {}", pin_line(pin));
        }
    }
    // The anchoring three-state with its population beside it: an empty orphan list
    // means one thing over fifty pinned blobs and another over none. The sight line
    // is stated before the reading it bounds.
    // D-F narrowed this population to UNMOUNTED roots only: a mounted root's
    // form-3 rows are asked of that root's own store and counted in the
    // three-state reading above.
    if !pins.out_of_jurisdiction.is_empty() {
        let _ = writeln!(
            out,
            "  anchoring scope: {} pin{} in unmounted roots' object stores, NOT measured here — {}",
            pins.out_of_jurisdiction.len(),
            if pins.out_of_jurisdiction.len() == 1 {
                ""
            } else {
                "s"
            },
            // Capped prose, complete wire: the count is in this same sentence
            // and the whole set rides `anchoring_out_of_jurisdiction.refs` on
            // `--json`. The separator normalises to the house spelling — one
            // `capped_sample` for every human face (card cap-convention-audit).
            crate::capped_sample(&pins.out_of_jurisdiction)
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

    // ── the run plane ───────────────────────────────────────────────────────
    // Pre-exec receipts with no completion. Reported, never gated on — like the
    // fence line, this does not move check's exit.
    if let Some(rendered) = check::orphan::render(&report.orphans) {
        let _ = writeln!(out, "{rendered}");
    }
    out
}

/// One pin row as a render line: the page, the ref it declares, and the colour
/// label its one computer produced. Never re-spells a reason word.
fn pin_line(pin: &PinRow) -> String {
    if pin.declared_ref.is_empty() {
        format!("{} — {}", pin.label, pin.src_path)
    } else {
        format!("{} — {} → {}", pin.label, pin.src_path, pin.declared_ref)
    }
}

/// The `--json` shape: the workspace plus the core object (the drifted claims), the
/// write-history disclosure, and the top-level `red` verdict.
fn to_json(
    workspace: &Path,
    worktree: &CoreReport,
    interval: &Interval,
    staged: Option<&Assessed>,
    gate: Option<&Gate<'_>>,
    fence: &Fence,
) -> Value {
    let mut value = interval_json(workspace, worktree);
    // `red` is the verdict, so it is worst-of across intervals; the per-interval
    // detail stays under `core`/`pins` (worktree) and `interval.staged`.
    let refuses_staged = staged.is_some_and(|s| s.report.is_red());
    value["red"] = Value::Bool(worktree.is_red() || refuses_staged);
    value["interval"] = json!({
        // The state, not merely the list — `asked: false` and "asked, and the
        // index adds nothing" are different facts.
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
    // The key is absent when the scoped question was not asked — this face's own
    // law, stated at [`interval_json`]: an absent field reads as "not checked". A
    // `null` would say the gate was asked and had nothing to say.
    if let Some(gate) = gate {
        value["commit_gate"] = json!({
            "gated_interval": gate.label,
            "permits": gate.permits(),
            "verdict": gate.word(),
            "detail": gate.detail(),
            "gated_planes": ["pins"],
            "write_history": WRITE_HISTORY_NOT_ASSESSED,
            // The population the gate read: `permits: true` over `pin_coverage: 0`
            // and over `pin_coverage: 50` are different assurances.
            "pin_coverage": gate.pins.declared,
            "require_pins": gate.require_pins,
        });
    }
    // Top-level, never inside [`interval_json`]: the fence is a reading of the
    // checkout, not per-interval, and it is taken on every run.
    value["fence"] = fence_json(fence);
    value
}

/// One interval's verdict as the `check --json` object.
///
/// This face's law: an absent field reads as "not checked" — a `null` would assert
/// a read that never happened. `write_history` is not a colour and not a detector;
/// it is the statement that this verb does not look.
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

/// The `pins` block: the claim plane's findings and the retrieval plane's anchoring
/// reading, each carrying its own reason word verbatim.
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
        // The sight line for the CLAIM plane, beside `anchoring_out_of_jurisdiction`
        // for the retrieval plane: pins whose target the hash domain excludes.
        // Reported, never gated (§12.1 verdict-plane clause). Its own key rather
        // than a member of `grey`, so a scripted caller gating on `grey` keeps
        // refusing for want of a measure and never for a declared exclusion —
        // and so the exclusion is never silent on this face either.
        "unattested": pins.unattested.iter().map(row).collect::<Vec<_>>(),
        "anchoring": match &pins.cannot_ask {
            Some(_) => Value::Null,
            // The three-state reading plus its population: an empty `orphaned` over
            // `asked: 0` is a reading of nothing, not a clean bill.
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
        // The sight line — the population this plane did not measure, because those
        // blobs live in another root's object store. Count and refs: a bare count
        // cannot be acted on.
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

    /// `--commit-gate` implies `--staged`: the interval it gates on is the one the
    /// index carries, and a gate without it would read the wrong bytes.
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

    /// A pin plane with nothing to report — the gate's passing input. `declared: 1`
    /// on purpose: a plane that was asked something is a different input from one
    /// that was asked nothing ([`no_pins_declared`]).
    fn holding_pins() -> PinPlane {
        PinPlane {
            red: Vec::new(),
            grey: Vec::new(),
            unattested: Vec::new(),
            orphaned: Vec::new(),
            anchored: 0,
            pending: 0,
            never: 0,
            cannot_ask: None,
            declared: 1,
            out_of_jurisdiction: Vec::new(),
        }
    }

    /// A corpus that declares no pin — clean, and covering nothing.
    fn no_pins_declared() -> PinPlane {
        PinPlane {
            declared: 0,
            ..holding_pins()
        }
    }

    fn pin(color: &model::selector::Color, label: &str) -> PinRow {
        PinRow {
            src_path: "claim.md".to_string(),
            declared_ref: "source.md#S".to_string(),
            class: check::PinClass::of(color),
            label: label.to_string(),
        }
    }

    /// The gate passes on the plane it actually reads, and says so in a word that
    /// claims nothing more.
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
            gate.detail().contains("write history is not assessed"),
            "and the pass names the narrowing rather than implying a wider check: {}",
            gate.detail()
        );
        assert!(
            !gate.detail().contains("no memory"),
            "the mechanism WHY is docs/--help teaching, never a footnote on the \
             verdict detail (report-voice pass): {}",
            gate.detail()
        );
    }

    /// Zero pins passes by default — vacuous truth, not unknown: over no pins
    /// "does the world still match the pins" is vacuously yes.
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

    /// The fail-closed doctrine, preserved as the caller's choice: the same plane
    /// under `--require-pins` refuses — with its own word, never grey's.
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

    /// A red pin refuses, and the gate is load-bearing in that direction.
    #[test]
    fn a_red_pin_refuses_the_commit() {
        let pins = PinPlane {
            red: vec![pin(
                &model::selector::Color::Red(model::selector::RedReason::Drifted),
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

    /// Unknown is not clean: a grey pin refuses on the same leg as a red one, and
    /// carries the distinct reason word.
    #[test]
    fn an_unreadable_pin_plane_refuses_as_an_absence_and_not_as_a_finding() {
        let pins = PinPlane {
            grey: vec![pin(
                &model::selector::Color::Grey(model::selector::GreyReason::Ambiguous),
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

    /// An object store that could not be asked is the other grey antecedent, and it
    /// refuses too.
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

    /// A report holding both colours — the shape `mrd-dogfood` s14-70 measured
    /// on the live corpus: red pins beside one grey.
    fn red_beside_grey() -> CoreReport {
        CoreReport {
            drifted_claims: vec![check::ClaimFinding {
                selector: "target.md#Beta".to_string(),
                detail: "observed != expected".to_string(),
            }],
            pins: PinPlane {
                red: vec![pin(
                    &model::selector::Color::Red(model::selector::RedReason::Drifted),
                    "red content-drifted",
                )],
                grey: vec![pin(
                    &model::selector::Color::Grey(model::selector::GreyReason::Ambiguous),
                    "grey unverifiable-fingerprint",
                )],
                ..holding_pins()
            },
            orphans: Vec::new(),
        }
    }

    /// The masking probe: with reds present the grey is still counted and named.
    /// v1.0.0 returned the red summary alone — "4 findings" while five questions
    /// stood — so an operator who fixed the list believed the corpus clean.
    #[test]
    fn the_findings_list_keeps_the_grey_beside_the_reds() {
        let report = red_beside_grey();
        let refusal = worst_of_exit(&report, None, &[]).expect_err("reds refuse");
        let text = format!("{refusal:?}");
        assert!(
            text.contains("3 findings"),
            "the count is everything listed — one claim, one red pin, one grey pin: {text}"
        );
        assert!(
            text.contains("grey unverifiable-fingerprint"),
            "the grey is NAMED, not merely counted: {text}"
        );
        assert!(
            text.contains("red content-drifted"),
            "and the reds keep their lines: {text}"
        );
        let red_at = text.find("red content-drifted").expect("red line");
        let grey_at = text
            .find("grey unverifiable-fingerprint")
            .expect("grey line");
        assert!(
            red_at < grey_at,
            "worst-of ORDER survives — the selection is what stopped being worst-of: {text}"
        );
    }

    /// The control (s14-40): the same grey with no red anywhere is still the
    /// one-finding list, on the same leg. Unmasking must not have changed it.
    #[test]
    fn a_lone_grey_is_still_the_whole_list() {
        let report = CoreReport {
            drifted_claims: Vec::new(),
            pins: PinPlane {
                grey: vec![pin(
                    &model::selector::Color::Grey(model::selector::GreyReason::Ambiguous),
                    "grey unverifiable-fingerprint",
                )],
                ..holding_pins()
            },
            orphans: Vec::new(),
        };
        let refusal = worst_of_exit(&report, None, &[]).expect_err("grey fails closed");
        let text = format!("{refusal:?}");
        assert!(text.contains("1 finding"), "{text}");
        assert!(text.contains("grey unverifiable-fingerprint"), "{text}");
    }

    /// A green report exits 0 — the widened list must not invent a refusal.
    #[test]
    fn a_green_report_still_exits_zero() {
        let report = CoreReport {
            drifted_claims: Vec::new(),
            pins: holding_pins(),
            orphans: Vec::new(),
        };
        assert!(worst_of_exit(&report, None, &[]).is_ok());
    }

    #[test]
    fn parse_rejects_stray_positional() {
        assert_eq!(Check::parse(&["extra".to_string()]).unwrap_err().code, 2);
    }

    fn paths(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("staged/file{i:02}.md")).collect()
    }

    /// The staged interval names a BOUNDED sample of the diverging paths. The
    /// count is not on this line by design — `interval_line` prints
    /// `paths.len()` for the same population — and `diverged_paths` on
    /// `--json` stays the complete carrier.
    #[test]
    fn the_staged_interval_line_samples_the_paths() {
        let all = paths(40);
        let said = staged_interval_line(&all);

        // Positive control first: a bound is trivially satisfied by an absent line.
        assert!(
            said.contains("the bytes a commit would record"),
            "the staged line did not fire, so this gate would pass vacuously: {said}"
        );
        let rest = all.len() - crate::EXCLUDED_SHOWN;
        assert!(
            said.contains(&format!("and {rest} more")),
            "the line must say how many paths it did NOT name — a sample that \
             does not admit it is a sample reads as the whole list: {said}"
        );
        let named = all.iter().filter(|p| said.contains(p.as_str())).count();
        assert_eq!(
            named,
            crate::EXCLUDED_SHOWN,
            "the line named {named} paths; the cap is {}. A rebase or a \
             generated tree stages thousands: {said}",
            crate::EXCLUDED_SHOWN
        );

        // And the count the operator needs is still voiced, uncapped, beside it.
        let interval = interval_line(&Interval::Diverges(StagedBytes {
            paths: all.clone(),
            files: fs::DomainFiles::default(),
        }));
        assert!(
            interval.contains(&format!("{} path(s) differ", all.len())),
            "the FULL count must survive the cap — it is the half that keeps \
             the divergence non-silent: {interval}"
        );
    }

    /// A population under the cap is named in full and claims no remainder — a
    /// remainder clause that fires at zero teaches readers to ignore it.
    #[test]
    fn a_short_staged_list_is_named_in_full() {
        let all = paths(2);
        let said = staged_interval_line(&all);
        for path in &all {
            assert!(said.contains(path.as_str()), "{path} must be named: {said}");
        }
        assert!(
            !said.contains(" more"),
            "there is no remainder, so the line must not claim one: {said}"
        );
    }

    fn jurisdiction_report(refs: Vec<String>) -> CoreReport {
        CoreReport {
            drifted_claims: Vec::new(),
            pins: PinPlane {
                out_of_jurisdiction: refs,
                ..holding_pins()
            },
            orphans: Vec::new(),
        }
    }

    /// The out-of-jurisdiction line states the FULL count and samples the refs.
    /// The complete set rides `anchoring_out_of_jurisdiction.refs` on `--json`.
    #[test]
    fn the_out_of_jurisdiction_line_counts_all_and_samples_the_refs() {
        let refs: Vec<String> = (0..25).map(|i| format!("outside{i:02}.md#S")).collect();
        let said = render_report(&jurisdiction_report(refs.clone()));

        assert!(
            said.contains("anchoring scope:"),
            "the line did not fire, so this gate would pass vacuously: {said}"
        );
        assert!(
            said.contains(&format!("{} pins in unmounted roots", refs.len())),
            "the COUNT is never capped — it is what makes the skip non-silent: {said}"
        );
        let rest = refs.len() - crate::EXCLUDED_SHOWN;
        assert!(
            said.contains(&format!("and {rest} more")),
            "the line must admit the remainder: {said}"
        );
        let named = refs.iter().filter(|r| said.contains(r.as_str())).count();
        assert_eq!(
            named,
            crate::EXCLUDED_SHOWN,
            "the line named {named} refs; the cap is {}: {said}",
            crate::EXCLUDED_SHOWN
        );
    }

    /// Under the cap: named in full, no remainder clause, and the singular
    /// still agrees with the count.
    #[test]
    fn a_short_out_of_jurisdiction_list_is_named_in_full() {
        let said = render_report(&jurisdiction_report(vec!["outside.md#S".to_string()]));
        assert!(
            said.contains("1 pin in unmounted roots"),
            "one ref stays singular: {said}"
        );
        assert!(
            said.contains("outside.md#S"),
            "the lone ref is named: {said}"
        );
        assert!(
            !said.contains(" more"),
            "there is no remainder, so the line must not claim one: {said}"
        );
    }
}
