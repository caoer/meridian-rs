//! `mrd check` — the pure READ validity verb (U2.10; d2 §3 check).
//!
//! ```text
//! mrd check [--core] [--staged] [--json]
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
//! Read-only. Exit triad (§4 preamble):
//! - **0** — green: the journal dates the live tree and its chain is continuous.
//! - **1** — a finding: a broken journal chain (cites the row). A check finding,
//!   never a door refusal (refusal-amendment). **Grey rides this leg too** (S3-R5
//!   and S3-R8, spelled by S3-R6): when the journal cannot date the tree — no rows,
//!   or a last receipt the live tree no longer matches — the verb refuses
//!   `grey(cannot-assess)`. Unknown is not clean, and a hook that rejects on
//!   non-zero must reject what nobody could vouch for. The triad stays CLOSED: no
//!   fourth code. The exit answers "may this proceed?" (red and grey both say no);
//!   the reason word, distinct on both faces, says why.
//! - **2** — bad invocation, or an unreadable workspace / journal.

use std::path::Path;

use check::{CoreReport, GREY_CANNOT_ASSESS, JournalTrace, PinRow};
use receipt::anchor::{ObjectAnchor, PENDING_ANCHOR_TTL};
use serde_json::{Value, json};

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

    match parsed.format {
        Format::Json => {
            let value = to_json(&canonical, &worktree.report, &interval, staged.as_ref());
            println!("{}", serde_json::to_string_pretty(&value).expect("json"));
        }
        Format::Human => print!(
            "{}",
            render_human(&canonical, &worktree.report, &interval, staged.as_ref())
        ),
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

    // Worst-of ACROSS INTERVALS, then worst-of within one: red is reported first,
    // grey next, green last. Both refuse on the SAME leg (S3-R6: the exit code
    // answers only "may this proceed?"; no fourth code), so the prefix is the same
    // verb and the REASON WORD in each line is what tells a finding from an
    // absence of evidence. Saying "found a lie" over a pending-anchor blob would
    // be a claim wider than the evidence — nothing lied, a blob is simply held by
    // nothing durable.
    //
    // The STAGED interval refuses on the same leg as the worktree one and says
    // which interval it is: a refusal a reader cannot locate is one they cannot
    // act on, and "the bytes on your disk are fine" plus "the bytes you are about
    // to commit are not" are different instructions.
    let mut intervals: Vec<(&str, &CoreReport)> = vec![(WORKTREE, &worktree.report)];
    if let Some(staged) = staged.as_ref() {
        intervals.push((STAGED, &staged.report));
    }
    if let Some((label, summary)) = intervals
        .iter()
        .find_map(|(label, report)| report.red_summary().map(|s| (*label, s)))
    {
        return Err(Fail {
            code: EXIT_FINDING,
            message: format!("check refuses ({label}): {}", summary.replace('\n', "; ")),
        });
    }
    if let Some((label, summary)) = intervals
        .iter()
        .find_map(|(label, report)| report.grey_summary().map(|s| (*label, s)))
    {
        return Err(Fail {
            code: EXIT_FINDING,
            message: format!("check refuses ({label}): {}", summary.replace('\n', "; ")),
        });
    }
    Ok(())
}

/// The interval a worktree read spans — the bytes on disk.
const WORKTREE: &str = "worktree";

/// The interval a COMMIT spans — the bytes the index carries. One name for it, in
/// the human render, the `--json` face and every refusal, so a reader who learns
/// the word once can find it everywhere (S3-R6).
const STAGED: &str = "staged";

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
}

impl Check {
    fn parse(args: &[String]) -> Result<Self, Fail> {
        let mut json = false;
        let mut staged = false;
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
                let _ = writeln!(out, "  chain: RED — {summary}");
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

    #[test]
    fn parse_rejects_stray_positional() {
        assert_eq!(Check::parse(&["extra".to_string()]).unwrap_err().code, 2);
    }
}
