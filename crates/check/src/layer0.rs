//! Layer 0 — the rule-free core of `check` (d2 §3 check: "Layer 0 (`check
//! --core`): chain integrity by full recompute, claims realised, and the
//! mechanical journal TRACE … pack-free").
//!
//! Three pack-free reads over the run plane, none of which writes a byte:
//! - [`journal_trace`] — the baseline check (last-receipt-vs-live) and, only when
//!   that baseline is current, the chain recompute (the U2.1
//!   [`receipt::journal::check_chain`] primitive);
//! - [`claims_realised`] — observe each claim against the current tree and report
//!   the drifted ones (the realise engine's pure detection, run read-only here);
//! - [`pin_plane`] — **U14, the sibling read that stops the journal plane from
//!   answering for a plane it cannot see.** The pin plane is two planes at once:
//!   the CLAIM plane (`pins:` — did the content drift?) and the RETRIEVAL plane
//!   (`objects:` — is the blob durably anchored?), and the fence's verb needs
//!   both.

use std::collections::BTreeMap;
use std::io;

use fs::WorkspaceRoot;
use model::Document;
use model::selector::Color;
use realise::{CheckOutcome, Claim};
use receipt::anchor::{ObjectAnchor, ObjectAnchorFacts};
use receipt::journal::{ChainReport, ParsedRow, check_chain, parse_rows};

/// The journal TRACE over a workspace: what the reserved receipt journal can
/// still prove about the live tree. Mechanical, source-3, no rule, no cap.
///
/// **A verdict requires a CURRENT baseline** (S3-R8). Both journal detectors rest
/// on one assumption — that the last receipt's recorded `root_after` still
/// accounts for the live tree — and that assumption fails in two measured ways:
///
/// - the journal carries **no row** ([`JournalTrace::NoBaseline`]): a workspace
///   with no guarded write behind it has nothing to compare against, so a silent
///   detector proves nothing;
/// - the last receipt **no longer matches** the live tree
///   ([`JournalTrace::StaleBaseline`]): something advanced the tree that the
///   journal does not account for.
///
/// So the trace spells both as states of its own instead of letting the detectors
/// report a green they never earned (the false green) or a red they cannot support
/// (the false red). *Outside sight is never verified* (R26; S5 honest degradation).
///
/// **U32 closed the first regime and narrowed the second; U35 closed the rest of
/// the second.** A governed `splice` journals its row (U32), and so now do the two
/// byte-landing doors that bypass the wire choke-point — `mrd realise --truth
/// file`'s INDEX deploy and the run plane's batch commit (U35). A governed write
/// through any of them leaves a baseline that is CURRENT, so the trace reads a
/// real verdict instead of refusing.
///
/// **The attribution stays withheld, and that is not an oversight** (S3-R12(a)):
/// a [`StaleBaseline`](JournalTrace::StaleBaseline) still means only *"something
/// advanced the tree that the journal does not account for"*, which is what an
/// out-of-band edit produces. Whether the closed doors make a narrower claim
/// supportable is a SEPARATE question the ruling deferred — naming a cause is a
/// claim this crate does not make here; stating the evidence is one it does.
#[derive(Debug)]
pub enum JournalTrace {
    /// **Grey — cannot assess.** The reserved journal carries no row: no pair of
    /// rows to recompute a chain over, no last receipt to attribute the live tree
    /// against.
    NoBaseline,
    /// **Grey — cannot assess.** The journal carries rows, but its last receipt
    /// does not account for the live tree. The cause is NOT determinable from
    /// here: an out-of-writer edit and any write door that lands bytes without
    /// journaling leave the identical evidence.
    StaleBaseline(BaselineMismatch),
    /// The last receipt accounts for the live tree: the baseline is current, so the
    /// journal's own rows are a verdict.
    Assessed {
        /// The chain-continuity report from the U2.1 primitive — red cites the
        /// spliced/forged row that fails to continue the chain.
        chain: ChainReport,
    },
}

/// The evidence of a stale baseline: the last governed receipt, the root it
/// recorded, and the root the tree folds to now.
///
/// This was `ForeignEdit`, and the rename is the correction: the same three facts
/// were rendered as the CLAIM *"an out-of-writer edit landed with no receipt"*,
/// which was false on every governed splice — measured on the deployed binary
/// against a fully governed corpus (finding-01). U32 gave the splice its row and
/// U35 gave the last two byte-landing doors theirs, so a governed corpus no longer
/// reaches here at all; the accusation stays withdrawn because an ordinary
/// out-of-band edit leaves exactly this evidence, and whether a narrower claim is
/// now supportable is the separate question S3-R12(a) deferred.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaselineMismatch {
    /// The last receipt row's anchor (`r-NNNNNN`) — the last journaled write.
    pub last_receipt: String,
    /// The tree root that receipt recorded (`root_after`).
    pub recorded_root: String,
    /// The live tree root now — it differs, so the baseline is out of date.
    pub live_root: String,
}

impl JournalTrace {
    /// The TRACE found a lie: the journal's own rows fail to chain, read against a
    /// baseline that is current. Grey is not red — a trace that could not look at
    /// anything, or could not date what it looked at, is a different answer from
    /// "the tree is honest".
    #[must_use]
    pub fn is_red(&self) -> bool {
        match self {
            JournalTrace::NoBaseline | JournalTrace::StaleBaseline(_) => false,
            JournalTrace::Assessed { chain } => chain.is_red(),
        }
    }

    /// The TRACE could not assess: it has no baseline, or cannot show the baseline
    /// it has is current. Never green, never red — grey.
    #[must_use]
    pub fn cannot_assess(&self) -> bool {
        matches!(
            self,
            JournalTrace::NoBaseline | JournalTrace::StaleBaseline(_)
        )
    }

    /// **Does this record vouch for ITSELF?** Only an assessed, continuous chain
    /// does: it was read against a current baseline and every row continues the one
    /// before it.
    ///
    /// This is the axis [`interval_accounted`] deliberately does NOT fold into its
    /// answer. A record that cannot vouch for itself can still ACCOUNT for an
    /// interval — the rows it carries are the only rows there are — but the
    /// acceptance then rests on a ledger nothing vouches for, and that is a weaker
    /// claim which must be spelled as one. Two facts, two readings, never one byte.
    #[must_use]
    pub fn vouches(&self) -> bool {
        matches!(self, JournalTrace::Assessed { chain } if chain.is_green())
    }

    /// A red render citing the broken row, or `None` when the TRACE is not red.
    /// This is the string the `check --core` verb renders.
    #[must_use]
    pub fn red_summary(&self) -> Option<String> {
        let JournalTrace::Assessed { chain } = self else {
            return None;
        };
        chain.red_summary()
    }

    /// The grey render: what could not be assessed and why, or `None` when the
    /// TRACE had a current baseline to read.
    #[must_use]
    pub fn grey_summary(&self) -> Option<String> {
        match self {
            JournalTrace::NoBaseline => Some(NO_BASELINE_DETAIL.to_string()),
            JournalTrace::StaleBaseline(m) => Some(format!(
                "the live tree root {} does not continue the last receipt ^{} (recorded \
                 root_after={}) — something advanced the tree that the journal does not account \
                 for, so the journal cannot date the tree and neither detector can be read",
                m.live_root, m.last_receipt, m.recorded_root
            )),
            JournalTrace::Assessed { .. } => None,
        }
    }
}

/// The one sentence that explains the empty-journal grey — the engine states the
/// missing evidence, never a colour alone.
pub const NO_BASELINE_DETAIL: &str = "the receipt journal carries no row, so the chain recompute and the \
     foreign_edit trace have nothing to compare against";

/// The reason word this grey carries, verbatim on BOTH faces — the human render
/// and `--json` — per S3-R6. The colour alone is not the answer: `grey(unmounted)`
/// and `grey(cannot-assess)` are different refusals that share exit 1, so the word
/// is what tells them apart. One spelling, ruled centrally, because U11/U14 render
/// the same grammar.
pub const GREY_CANNOT_ASSESS: &str = "grey(cannot-assess)";

/// Recompute the journal TRACE over `root`: read the reserved receipt journal
/// page, date it against the live tree, and recompute chain continuity
/// ([`check_chain`]) only when the dating holds. Reads two things (the journal
/// bytes and the tree fold); writes nothing.
///
/// The journal is root-EXCLUDED, so its own bytes never enter the live tree fold
/// — splicing a forged row changes the chain (caught by [`check_chain`]) but not
/// the tree root.
///
/// **The baseline is checked before anything is claimed** (S3-R8): zero rows is
/// [`JournalTrace::NoBaseline`]; a last receipt that no longer accounts for the
/// live tree is [`JournalTrace::StaleBaseline`]. Both are grey. Only past that
/// gate does the chain recompute become a verdict a reader can act on.
///
/// # Errors
/// [`io::Error`] when the journal page or the domain snapshot cannot be read.
pub fn journal_trace(root: &WorkspaceRoot) -> io::Result<JournalTrace> {
    let page = read_journal(root)?;
    let live_root = fs::domain_snapshot(root)?.1.0;
    Ok(journal_trace_of(&page, &live_root))
}

/// [`journal_trace`] over an interval whose BYTES the caller already holds: the
/// journal page and the folded tree root of THAT interval, rather than of the
/// worktree.
///
/// # Why the trace is reachable without a root (F1)
/// The worktree and the git index are two intervals, and a pre-commit fence is
/// asked about the second one. The dating and the chain recompute are pure
/// functions of (journal bytes, tree root), so an interval that can produce those
/// two facts gets the SAME verdict computer — never a second implementation of
/// it. [`journal_trace`] is this function plus the two disk reads.
///
/// **Both arguments must come from ONE interval.** A journal page from the
/// worktree dated against the index's fold is a compare between two worlds, and
/// its answer describes neither.
#[must_use]
pub fn journal_trace_of(journal_page: &str, live_root: &str) -> JournalTrace {
    let rows = parse_rows(journal_page);
    match detect_baseline(&rows, live_root) {
        None => JournalTrace::NoBaseline,
        Some(Err(mismatch)) => JournalTrace::StaleBaseline(mismatch),
        Some(Ok(())) => JournalTrace::Assessed {
            chain: check_chain(&rows),
        },
    }
}

/// [`journal_trace_of`] for an interval that is **not the worktree**, dated against
/// the RECORD rather than only against its own last row.
///
/// `staged_page`/`staged_root` are that interval's own journal bytes and fold;
/// `worktree_page` is the worktree's journal — the most complete record of what the
/// engine has written, and the one the worktree pass separately validates in the
/// same run.
///
/// # The predicate this widens, stated because the claim changed (S3-R29)
/// Dating a snapshot against its OWN journal's last row asks *"are these bytes the
/// CURRENT governed state?"* — and **a legitimately staged intermediate state is
/// not.** `git add` stages content without the journal, so an ordinary
/// `git add` → further governed write leaves the staged tree folding to an earlier
/// receipt's `root_after` while the staged journal still ends before it. Measured:
/// the staged fold equalled `^r-000002`'s `root_after` while the staged journal
/// ended at `^r-000001`. Refusing that is a FALSE RED — the engine wrote every byte
/// of both states — and it brakes `git add -p` and every `git add` followed by a
/// further governed write, which is the common path on a fleet writing continuously.
///
/// So the question here is *"were these bytes PRODUCED BY A GOVERNED WRITE, and is
/// the journal being committed a truthful prefix of the record?"* — weaker than the
/// worktree's question, and **the two conditions are a PAIR because each catches
/// what the other misses**:
///
/// - **membership alone** would let a staged JOURNAL forgery through: the journal is
///   root-EXCLUDED from the hash domain, so splicing a row moves no fold at all.
/// - **the prefix alone** would let a staged CONTENT forgery through: forging a
///   governed file leaves the journal untouched, so the prefix holds trivially.
///
/// Neither condition is sufficient and the arms that prove it are separate
/// (`a_staged_journal_forgery_…` and
/// `the_fence_refuses_a_commit_whose_index_carries_an_out_of_band_write`).
///
/// # It only WIDENS — it never turns a red into a green
/// The own-journal test runs first and unchanged, and the chain recompute runs on
/// the STAGED rows whichever way the baseline was satisfied, so a spliced row still
/// reddens. A forgery matches no receipt and keeps the grey it had.
#[must_use]
pub fn staged_trace(staged_page: &str, staged_root: &str, worktree_page: &str) -> JournalTrace {
    let rows = parse_rows(staged_page);
    // ONE owner for the pair. This widener and the scoped verdict must never come
    // to disagree about what "accounted for" means — two copies of a two-condition
    // test agree only until someone edits one of them.
    let accounted = || interval_accounted(staged_page, staged_root, worktree_page).is_accounted();
    match detect_baseline(&rows, staged_root) {
        // Its own journal dates it: the shipped answer, untouched.
        Some(Ok(())) => JournalTrace::Assessed {
            chain: check_chain(&rows),
        },
        Some(Err(mismatch)) => {
            if accounted() {
                JournalTrace::Assessed {
                    chain: check_chain(&rows),
                }
            } else {
                JournalTrace::StaleBaseline(mismatch)
            }
        }
        // No rows staged at all — a repository whose journal is not committed. The
        // empty row list is trivially a prefix, so this rests entirely on the fold
        // matching a receipt in the record; with no record either, it stays grey.
        None => {
            if accounted() {
                JournalTrace::Assessed {
                    chain: check_chain(&rows),
                }
            } else {
                JournalTrace::NoBaseline
            }
        }
    }
}

/// **The interval a commit records, as a verdict a caller can HOLD** — the
/// promotion of the [`is_prefix_of`] + [`accounts_for`] pair from a baseline
/// widener to an addressable answer.
///
/// # Why this is a different question from [`JournalTrace`], and not a softer one
/// [`JournalTrace`] answers *"is everything this record says true?"* — about the
/// corpus's whole write history, and **permanent** once a row breaks: *"a
/// hand-inserted row breaks the chain mechanically and a forger must re-forge the
/// entire suffix"* ([`receipt::journal`] lines 18-21). No later governed write can
/// make a past break untrue, and nothing here tries to.
///
/// A pre-commit gate does not ask that question. It asks the narrower one —
/// **were THESE bytes produced by a governed write?** — a fact about THIS interval,
/// answered fresh every commit. Spending the permanent answer on the per-commit
/// question builds a gate whose verdict stops varying with its input: past the
/// first break it refuses every commit, carrying zero information about the thing it
/// guards, so the only remaining paths are the unrecorded escapes. That is strictly
/// worse than reporting the two apart, because it destroys the per-commit
/// enforcement as well.
///
/// **The permanence is correct and this verdict does not touch it.** The record's
/// own standing is [`JournalTrace::vouches`] — reported beside this answer, never
/// folded into it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Accounted {
    /// Both conditions hold: the interval's journal is a truthful PREFIX of the
    /// record, and the interval's tree folds to a root some governed write produced.
    Yes,
    /// **Grey — the record carries no row at all**, so it accounts for nothing and
    /// could refuse nothing either. This is EVIDENCE AVAILABILITY, not a finding: a
    /// clone whose journal has not travelled reaches it having tampered with
    /// nothing, and rendering it as a lie would accuse every such workspace.
    NoRecord,
    /// The interval's journal carries a row the record does not have. Membership
    /// cannot see this — the journal is root-EXCLUDED from the hash domain, so a
    /// spliced row moves no fold at all.
    ForeignJournal,
    /// The interval's tree folds to a root no governed write ever produced. The
    /// prefix cannot see this — forging a governed file leaves the journal
    /// untouched, so the prefix holds trivially.
    ForeignBytes {
        /// The fold that matches no recorded root — the evidence, stated.
        root: String,
    },
}

impl Accounted {
    /// The interval is accounted for: the gate may let it through.
    #[must_use]
    pub fn is_accounted(&self) -> bool {
        matches!(self, Accounted::Yes)
    }

    /// Nothing could be read, so nothing was assessed. Never green, never red —
    /// and never rendered as tampering, which is the whole reason it is its own
    /// state rather than a refusal detail.
    #[must_use]
    pub fn cannot_assess(&self) -> bool {
        matches!(self, Accounted::NoRecord)
    }

    /// The record was READ, and it contradicts the interval. A finding.
    #[must_use]
    pub fn is_red(&self) -> bool {
        matches!(
            self,
            Accounted::ForeignJournal | Accounted::ForeignBytes { .. }
        )
    }

    /// What was asked and what came back, in words — the half of the answer that
    /// tells the four states apart once the exit has collapsed them to two.
    #[must_use]
    pub fn detail(&self) -> String {
        match self {
            Accounted::Yes => "the journal being committed is a truthful prefix of the record, \
                 and the tree it commits folds to a root the record produced"
                .to_string(),
            Accounted::NoRecord => format!(
                "{NO_RECORD_DETAIL} — an absence of evidence about your bytes, not a finding \
                 against them"
            ),
            Accounted::ForeignJournal => {
                "the journal being committed carries a row the record does not have; the journal \
                 is root-EXCLUDED from the hash domain, so a spliced row moves no fold and only \
                 the prefix can see it"
                    .to_string()
            }
            Accounted::ForeignBytes { root } => format!(
                "the tree being committed folds to {root}, which no governed write ever \
                 produced; forging a governed file leaves the journal untouched, so only \
                 membership can see it"
            ),
        }
    }
}

/// The one sentence that explains the empty-RECORD grey. Distinct from
/// [`NO_BASELINE_DETAIL`] because the two are different absences with different
/// fixes: that one is *"this journal cannot date this tree"*, this one is *"there is
/// no journal to date anything with"*.
pub const NO_RECORD_DETAIL: &str = "the record carries no row, so nothing accounts for these bytes — a clone whose journal has \
     not travelled reaches this state having tampered with nothing";

/// **Is this interval accounted for by the record?** — the scoped question, whose
/// answer is a fact about THESE bytes and is recomputed at every commit.
///
/// `interval_page`/`interval_root` are the journal bytes and tree fold of the
/// interval being asked about (the git index at a pre-commit gate, the worktree
/// when the two coincide). `record_page` is the most complete journal the engine
/// has — the worktree's.
///
/// The two conditions are a PAIR and each catches what the other misses; the arms
/// that prove each one necessary are
/// [`a_forged_tree_is_refused_and_the_prefix_condition_alone_would_not_see_it`] and
/// [`the_widened_path_refuses_a_journal_the_record_does_not_contain`].
///
/// # The empty record is checked FIRST, and that ordering is the point
/// With no rows, [`accounts_for`] is false and [`is_prefix_of`] is false for any
/// non-empty interval — so a bare pair would spell *"there is no ledger"* as
/// *"you forged this"*. Under a journal that does not travel with a clone, that
/// mis-spelling is the COMMON case, not an edge one. Absence of evidence gets its
/// own state ([`Accounted::NoRecord`]) so it can be refused without being accused.
#[must_use]
pub fn interval_accounted(
    interval_page: &str,
    interval_root: &str,
    record_page: &str,
) -> Accounted {
    let recorded = parse_rows(record_page);
    if recorded.is_empty() {
        return Accounted::NoRecord;
    }
    let rows = parse_rows(interval_page);
    if !is_prefix_of(&rows, &recorded) {
        return Accounted::ForeignJournal;
    }
    if !accounts_for(&recorded, interval_root) {
        return Accounted::ForeignBytes {
            root: interval_root.to_string(),
        };
    }
    Accounted::Yes
}

/// Is `rows` a PREFIX of `recorded` — the same rows, in the same order, from the
/// start? A staged journal that is an earlier prefix of the record is a truthful
/// earlier journal; one that diverges anywhere has a row the record does not have.
///
/// Compared field by field on the three facts a row asserts (its anchor and its two
/// roots), never by page bytes: a prefix must survive reformatting of the page it
/// sits in, and a row whose ROOTS were altered must not survive at all.
fn is_prefix_of(rows: &[ParsedRow], recorded: &[ParsedRow]) -> bool {
    rows.len() <= recorded.len()
        && rows.iter().zip(recorded).all(|(a, b)| {
            a.anchor == b.anchor && a.root_before == b.root_before && a.root_after == b.root_after
        })
}

/// Does the RECORD account for `root` — is it a tree root some governed write
/// produced (a row's `root_after`) or started from (a `root_before`, which covers
/// the genesis state no row's `root_after` names)?
///
/// **This is the whole discriminator between a legitimate intermediate state and a
/// forgery**: the engine only ever recorded roots it produced, so a forged tree
/// matches nothing here, while every state the engine passed through matches
/// exactly one row.
fn accounts_for(recorded: &[ParsedRow], root: &str) -> bool {
    recorded
        .iter()
        .any(|row| row.root_after == root || row.root_before == root)
}

/// **The baseline check** (d2 §3 journal TRACE, "last-receipt-vs-live"): can the
/// journal date the tree it is being read against? `None` = no rows at all;
/// `Some(Err)` = the last receipt's recorded `root_after` is not the live root;
/// `Some(Ok)` = the last receipt accounts for the live tree.
///
/// A mismatch is deliberately NOT called a `foreign_edit` any more. The same
/// evidence is produced by an out-of-writer edit and by any write door that lands
/// bytes without journaling, so naming one of the two is a claim wider than the
/// evidence — measured as a false red on a fully governed corpus (finding-01,
/// whose splice door U32 closed).
fn detect_baseline(rows: &[ParsedRow], live_root: &str) -> Option<Result<(), BaselineMismatch>> {
    let last = rows.last()?;
    if last.root_after == live_root {
        return Some(Ok(()));
    }
    Some(Err(BaselineMismatch {
        last_receipt: last.anchor.clone(),
        recorded_root: last.root_after.clone(),
        live_root: live_root.to_string(),
    }))
}

/// The reserved receipt journal page's bytes as the WORKTREE carries them — the
/// journal half of an interval, for a caller assembling one itself
/// ([`journal_trace_of`]).
///
/// One owner for this path and its absent case: a caller that read the page
/// itself would be a second reader of the same reserved location, free to
/// disagree about whether a missing journal is an error (it is not — a genesis
/// workspace has none).
///
/// # Errors
/// [`io::Error`] when the page exists and cannot be read.
pub fn journal_page(root: &WorkspaceRoot) -> io::Result<String> {
    read_journal(root)
}

/// Read the reserved receipt journal page bytes, or the empty string when the
/// page does not exist yet (a genesis workspace has no journal). Reads the raw
/// bytes directly — the row grammar is line-oriented, so no parse is needed.
fn read_journal(root: &WorkspaceRoot) -> io::Result<String> {
    let page = root.0.join(fs::domain::RESERVED_JOURNAL_PATH);
    match std::fs::read_to_string(&page) {
        Ok(text) => Ok(text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e),
    }
}

/// One claim that is NOT realised: its selector and why the observation drifted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimFinding {
    /// The claim selector (the board-card key; d2 §5.4).
    pub selector: String,
    /// Why the check failed — the observed-vs-expected detail from the drift.
    pub detail: String,
}

/// Claims-realised (d2 §3): observe each claim against the current tree and
/// report the ones that did not converge. A pure read — no apply, no cap (the
/// `check` verb never converges; the realise loop does). Reuses the realise
/// engine's [`Check`] detection rather than forking it, so `check` reads exactly
/// what `realise` would converge.
///
/// # Errors
/// [`realise::CheckError`] when a claim's observation itself faults (page load /
/// I/O) — distinct from a clean [`CheckOutcome::Drifted`], which is a finding, not
/// an error.
pub fn claims_realised(
    root: &WorkspaceRoot,
    claims: &[Claim],
) -> Result<Vec<ClaimFinding>, realise::CheckError> {
    let mut drifted = Vec::new();
    for claim in claims {
        match claim.check.observe(root)? {
            CheckOutcome::Converged => {}
            CheckOutcome::Drifted { detail } => drifted.push(ClaimFinding {
                selector: claim.selector.clone(),
                detail,
            }),
        }
    }
    Ok(drifted)
}

// ---------------------------------------------------------------------------
// U14 — the PIN PLANE: what the journal plane cannot see
// ---------------------------------------------------------------------------

/// One pin row as the ONE pin computer coloured it, handed to [`pin_plane`].
///
/// # Why this arrives as a FACT rather than being computed here
/// The colour of a pin is `view`'s to compute — it is the end of a chain that
/// runs corpus index → ref resolution → selector → fingerprint compare, and a
/// SECOND copy of that chain is exactly how the pin plane and the decoration
/// plane once came to hash two different documents for one ref. `check` may not
/// take `view` (S3-R3 authorized the `git` and `lock` leaves, and nothing wider),
/// so the caller reads `view::walk::lock_pin_colors` — **the same call
/// `mrd status`'s lock axis makes and the same colouring `mrd walk` lists** — and
/// hands the rows here.
///
/// That is the fact/classify split this workspace already runs on
/// ([`receipt::anchor::ObjectAnchorFacts`]: *"the caller asks git; this crate
/// never shells out"*), and it is what makes the three planes agree **by
/// construction** rather than by assertion: there is one computer, not three that
/// match today.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinRow {
    /// The page whose `meridian-lock` block declares this pin.
    pub src_path: String,
    /// The declared ref, verbatim — empty on a lock-refusal row, which declares
    /// no ref and carries its refusal in [`PinRow::color`].
    pub declared_ref: String,
    /// The colour the ONE pin computer gave it — what decides red from grey.
    pub color: Color,
    /// The colour RENDERED by that same computer (`view::walk::color_label`):
    /// `red content-drifted`, `grey unmounted (root 'x')`, and so on.
    ///
    /// Carried rather than re-derived because the reason words are spelled ONCE,
    /// in `view`, and a second speller in this crate would be two literals that
    /// agree only until someone edits one of them (S3-R59). Same shape as
    /// [`ClaimFinding::detail`], which likewise arrives pre-rendered from the
    /// engine that owns the words.
    pub label: String,
}

/// One `objects:` entry whose blob is **ORPHANED**: no ref reaches it, and the
/// file it is the blob OF no longer hashes to it — so no commit of that file will
/// ever anchor it either. The pin's evidence is held by nothing and is on its way
/// to nothing.
///
/// # Why the refusal is this pair and not a state on its own (measured, S3-R23(1))
/// *"Measure the ground truth BEFORE you write the check."* Measured across the
/// ordinary governed lifecycle of one pin:
///
/// | moment | `ObjectAnchor` | why |
/// |---|---|---|
/// | after `mrd pin`, before `git add` | **never-anchored** | a non-vibe pin hashes without `-w`, so the object is not in the database at all |
/// | at pre-commit hook time (staged) | **pending-anchor** | `git add` wrote the blob; the commit that would anchor it is the one being fenced |
/// | after the commit | **anchored** | a ref reaches it |
///
/// **So a refusal on `pending-anchor` refuses EVERY governed commit, and a refusal
/// on `never-anchored` refuses every workspace between `pin` and `git add`.**
/// Verified: wiring the refusal to `pending-anchor` reddened all three legs of
/// `u35_journaled_doors.rs`, which drive a REAL `git commit` through the RATIFIED
/// fence over fully governed corpora. That is the "fence refuses every governed
/// commit" failure S3-R8 migrated a whole unit to prevent, arriving from the other
/// side.
///
/// The state that is a defect in every one of those moments is the ORPHAN: the
/// blob is unreachable **and** the file's current bytes no longer produce it. The
/// three states stay a READING ([`PinPlane::anchored`] / `pending` / `never`); this
/// pair is the finding.
///
/// **No new reason word is minted** (S3-R49, convention over invention): the
/// finding cites [`ObjectAnchor::word`] for whichever of the two non-durable states
/// it is, and states the orphaning as EVIDENCE beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrphanedBlob {
    /// The page whose `meridian-lock` block declares this object.
    pub src_path: String,
    /// The `objects:` key, verbatim (what the blob is FOR).
    pub key: String,
    /// The blob sha, verbatim — an object id in git's world, not the engine's.
    pub blob_sha: String,
    /// Which non-durable state git reports it in — the EXISTING word, reused.
    pub state: ObjectAnchor,
    /// What the file hashes to now, or why it could not be hashed. The evidence
    /// that no commit will anchor the recorded blob.
    pub live: String,
}

/// The pin plane's reading over one corpus: what `check` can say about the pins
/// that the receipt journal will never carry.
///
/// **The journal plane and the pin plane fail INDEPENDENTLY, and that is the
/// whole finding.** A workspace whose last receipt accounts for the live tree and
/// whose chain is continuous — a journal plane with nothing at all to report — can
/// still hold a pin whose target drifted (the lock arrived by clone or pull while
/// its source moved) or a blob no ref reaches. Measured on this tree: `check`
/// green / `walk` `red content-drifted` / `status` `lock red content-drifted`, one
/// corpus, one run.
#[derive(Debug)]
pub struct PinPlane {
    /// Pins whose colour is RED — the ledger claims content that is no longer
    /// there.
    pub red: Vec<PinRow>,
    /// Pins whose colour is GREY — outside sight, each carrying its own reason
    /// word (S3-R6: one vocabulary, distinct words for distinct causes).
    pub grey: Vec<PinRow>,
    /// `objects:` entries whose blob nothing holds and nothing will — the
    /// anchoring FINDING. See [`OrphanedBlob`] for why the refusal is this pair
    /// and not a bare state.
    pub orphaned: Vec<OrphanedBlob>,
    /// **The anchoring three-state, as a READING** (GAP A: the verb was
    /// structurally blind to it; now it is not). Counts, because the three states
    /// are the ordinary lifecycle of a pin and not three verdicts.
    ///
    /// The counts also carry the POPULATION (S3-R23(5)): an empty `orphaned` list
    /// means two completely different things at zero pinned objects and at fifty,
    /// and *"a gate whose population empties is the quietest way for coverage to
    /// disappear"*.
    pub anchored: usize,
    /// Blobs git holds that no ref reaches — durable only until `gc.pruneExpire`
    /// ([`receipt::anchor::PENDING_ANCHOR_TTL`]). The normal state of anything
    /// staged, so a reading and never a gate here.
    pub pending: usize,
    /// Blobs git does not hold at all. The normal state of a non-vibe pin before
    /// `git add`, so likewise a reading.
    pub never: usize,
    /// Why the object store could not be asked at all, when it could not be.
    /// `Some` ⇒ the retrieval plane is UNREAD, never read clean.
    pub cannot_ask: Option<String>,
}

impl PinPlane {
    /// The pin plane found a finding: a red pin, or an ORPHANED blob.
    ///
    /// **An orphan rides the finding leg, not the grey one**, because it is an
    /// ANSWER and not the absence of one: git was asked, git replied, and the reply
    /// is that the pin's evidence is held by nothing and no commit will anchor it.
    /// Grey is reserved for the question that could not be put.
    ///
    /// **The three anchoring states themselves are NOT findings** — they are the
    /// ordinary lifecycle of a pin, and refusing on either non-durable one refuses
    /// every governed commit. [`OrphanedBlob`] carries the measurement.
    #[must_use]
    pub fn is_red(&self) -> bool {
        !self.red.is_empty() || !self.orphaned.is_empty()
    }

    /// How many `objects:` entries git was asked about — the population behind the
    /// three-state reading (S3-R23(5)).
    #[must_use]
    pub fn asked(&self) -> usize {
        self.anchored + self.pending + self.never
    }

    /// The pin plane could not assess something: a grey pin, or an object store
    /// it could not ask. Never green, never red.
    #[must_use]
    pub fn cannot_assess(&self) -> bool {
        !self.grey.is_empty() || self.cannot_ask.is_some()
    }

    /// Nothing to report and everything asked — the assessed green. Distinct from
    /// "no pins in the corpus" only in that both are true readings; a corpus that
    /// declares no pins genuinely owes nothing.
    #[must_use]
    pub fn is_green(&self) -> bool {
        !self.is_red() && !self.cannot_assess()
    }
}

/// Read the pin plane over `docs`: sort the caller's pin colours, then ask THIS
/// root's object store about every `objects:` entry the corpus declares.
///
/// `docs` must be the SAME corpus build the pin colours came from — a second
/// build would let the two halves describe two different corpora, which is the
/// trap `mrd status`'s two axes are explicitly built to avoid.
///
/// # It asks nothing it does not have to
/// A corpus that declares no `objects:` entry asks git nothing: nothing is
/// referenced, so nothing can be owed, and a workspace outside a repository stays
/// green rather than being refused for a question nobody asked.
///
/// # Every unanswerable question is REPORTED, never skipped
/// A value that is not an object id, a key naming a root whose store this read
/// cannot reach, git absent, the directory not a repository — all four leave the
/// retrieval plane UNREAD, and an unread plane is [`PinPlane::cannot_ask`], never
/// an empty (falsely clean) one. *Outside sight is never verified.*
#[must_use]
pub fn pin_plane(
    root: &WorkspaceRoot,
    docs: &BTreeMap<String, Document>,
    pins: &[PinRow],
) -> PinPlane {
    let mut plane = PinPlane {
        red: Vec::new(),
        grey: Vec::new(),
        orphaned: Vec::new(),
        anchored: 0,
        pending: 0,
        never: 0,
        cannot_ask: None,
    };
    for pin in pins {
        match pin.color {
            Color::Red(_) => plane.red.push(pin.clone()),
            Color::Grey(_) => plane.grey.push(pin.clone()),
            Color::Green => {}
        }
    }
    match objects_in(docs) {
        Err(detail) => plane.cannot_ask = Some(detail),
        Ok(objects) if objects.is_empty() => {}
        Ok(objects) => {
            if let Err(detail) = ask_store(root, &objects, &mut plane) {
                plane.cannot_ask = Some(detail);
            }
        }
    }
    plane
}

/// One `objects:` entry located in the corpus.
#[derive(Debug)]
struct ObjectRef {
    src_path: String,
    key: String,
    blob_sha: String,
}

/// Every `objects:` entry the corpus declares, or the reason the plane is unread.
///
/// [`lock::find`] is the parser — the SAME owner `view::walk::lock_objects` uses,
/// so a page's objects can never be read here by a second reader that disagrees
/// with the listing surfaces about what the page declares.
///
/// A page whose lock REFUSED contributes no objects, exactly as it contributes
/// none to the listing planes: its damage is already named by the grey
/// `lock-refused` PIN row the same page projects, and naming one defect twice is
/// how a reader comes to believe there are two.
fn objects_in(docs: &BTreeMap<String, Document>) -> Result<Vec<ObjectRef>, String> {
    let mut out = Vec::new();
    for (path, doc) in docs {
        let Ok(Some(found)) = lock::find(doc) else {
            continue;
        };
        for (key, blob_sha) in found.lock.objects {
            let oid = blob_sha.to_ascii_lowercase();
            if !git::is_oid(&oid) {
                return Err(format!(
                    "`{path}` objects.{key} is not an object id, so git cannot be asked about it"
                ));
            }
            // A key naming ANOTHER root names another object store, and this read
            // holds one handle: the ambient one. Falling back to it would ask the
            // WRONG database and answer confidently — a wrong SUCCESS, which is
            // the one shape a fence's verb must never produce. So it refuses, and
            // says which root it could not reach.
            match addr::Addr::parse(&key) {
                Ok(addr) => {
                    if let Some(name) = addr.root() {
                        return Err(format!(
                            "`{path}` objects.{key} names root `{name}`, whose object store this \
                             read cannot ask — the anchoring check runs against THAT root's git \
                             repo, and asking this one would answer a different repository's \
                             question in its name"
                        ));
                    }
                }
                Err(e) => {
                    return Err(format!(
                        "`{path}` objects.{key} is not an address, so WHICH git to ask is \
                         unknown: {e}"
                    ));
                }
            }
            out.push(ObjectRef {
                src_path: path.clone(),
                key,
                blob_sha: oid,
            });
        }
    }
    Ok(out)
}

/// Ask ONE object store about every entry, in ONE pass, and classify.
///
/// Two git calls at most, never a call per blob: ONE `git rev-list --objects
/// --all` into the reachable set (O(1) membership) and ONE batched `git cat-file
/// --batch-check` for presence. [`ObjectAnchor::classify`] then reads the pair —
/// the same classifier `mrd status`'s gauge runs, so the two surfaces cannot
/// disagree about what a blob's state is called.
///
/// **Both facts come from THIS handle in THIS pass** — the per-root law's own
/// warning (U13): *a fact pair split across two stores is a wrong answer, not a
/// stale one.*
///
/// **All three states are counted as a READING; only the ORPHAN is a finding.**
/// The three states are the ordinary lifecycle of one pin (never-anchored →
/// pending-anchor → anchored, measured in [`OrphanedBlob`]'s table), so refusing on
/// either non-durable one refuses every governed commit. A blob is orphaned when it
/// is unreachable **and** the file it is the blob OF no longer hashes to it: nothing
/// holds it and no commit will.
///
/// That second question costs one `git hash-object` per NON-ANCHORED entry only —
/// an anchored blob is durable and needs no follow-up — so the common case stays at
/// the two batched calls above.
fn ask_store(
    root: &WorkspaceRoot,
    objects: &[ObjectRef],
    plane: &mut PinPlane,
) -> Result<(), String> {
    let repo = git::Repo::at(root.0.clone());
    let reachable = repo
        .reachable_objects()
        .map_err(|e| format!("the object store could not be asked: {e}"))?;
    let oids: Vec<&str> = objects.iter().map(|o| o.blob_sha.as_str()).collect();
    let info = repo
        .object_info(&oids)
        .map_err(|e| format!("the object store could not be asked: {e}"))?;

    for (object, present) in objects.iter().zip(info) {
        let facts = ObjectAnchorFacts {
            object_present: present.is_some(),
            reachable_from_commit: reachable.contains(&object.blob_sha),
        };
        let state = ObjectAnchor::classify(&facts);
        match state {
            ObjectAnchor::Anchored => {
                plane.anchored += 1;
                continue;
            }
            ObjectAnchor::PendingAnchor => plane.pending += 1,
            ObjectAnchor::NeverAnchored => plane.never += 1,
        }
        // Does the file still hash to this blob? If it does, committing the file
        // anchors it and this is a lifecycle moment, not a defect. If it does not,
        // nothing holds the blob and nothing will.
        let live = match repo.blob_oid(&root.0.join(&object.key)) {
            Ok(oid) if oid.eq_ignore_ascii_case(&object.blob_sha) => continue,
            Ok(oid) => oid,
            Err(e) => format!("the file could not be hashed ({e})"),
        };
        plane.orphaned.push(OrphanedBlob {
            src_path: object.src_path.clone(),
            key: object.key.clone(),
            blob_sha: object.blob_sha.clone(),
            state,
            live,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use realise::Check;

    use super::*;

    /// Build a [`ParsedRow`] with the two roots and an anchor — the columns the
    /// baseline check reads (line number is irrelevant to detection).
    fn row(anchor: &str, root_before: &str, root_after: &str) -> ParsedRow {
        ParsedRow {
            anchor: anchor.to_string(),
            op: "splice".to_string(),
            path: "a.md".to_string(),
            actor: None,
            now: None,
            root_before: root_before.to_string(),
            root_after: root_after.to_string(),
            edits: 0,
            line_no: 1,
        }
    }

    /// A read-only claim (no apply) over a fixed [`CheckOutcome`] — the pure-read
    /// half of a realise [`Claim`], for the claims-realised tests.
    fn fixed_claim(selector: &str, outcome: CheckOutcome) -> Claim {
        struct Fixed(CheckOutcome);
        impl Check for Fixed {
            fn observe(&self, _root: &WorkspaceRoot) -> Result<CheckOutcome, realise::CheckError> {
                Ok(self.0.clone())
            }
        }
        Claim {
            selector: selector.to_string(),
            check: Box::new(Fixed(outcome)),
            apply: None,
            retry_budget: 0,
        }
    }

    /// The baseline check fires: when the live root differs from the last
    /// receipt's recorded `root_after`, the baseline is stale and the mismatch
    /// cites the last receipt and both roots. Dropping the comparison makes
    /// [`detect_baseline`] answer `Some(Ok)`, and this test FAILS (the gate's
    /// falsification).
    #[test]
    fn baseline_is_stale_when_live_root_drifts_from_last_receipt() {
        let rows = [
            row("r-000001", "b3:0", "b3:1"),
            row("r-000002", "b3:1", "b3:2"),
        ];
        let m = detect_baseline(&rows, "b3:LIVE_DRIFTED")
            .expect("rows are present, so there IS a baseline to date")
            .expect_err("live root != last receipt root_after ⇒ the baseline is stale");
        assert_eq!(m.last_receipt, "r-000002", "cites the last receipt");
        assert_eq!(m.recorded_root, "b3:2", "the recorded root_after");
        assert_eq!(m.live_root, "b3:LIVE_DRIFTED", "the live root");
    }

    /// The baseline check is load-bearing in BOTH directions: when the live root
    /// EQUALS the last receipt's recorded root, the journal dates the tree and the
    /// rows may be read. A check that always faulted fails here; one that never
    /// faulted fails the test above. Together they pin the comparison.
    #[test]
    fn baseline_is_current_when_live_root_matches_last_receipt() {
        let rows = [
            row("r-000001", "b3:0", "b3:1"),
            row("r-000002", "b3:1", "b3:2"),
        ];
        assert_eq!(
            detect_baseline(&rows, "b3:2"),
            Some(Ok(())),
            "live root == last receipt root_after ⇒ the baseline is current"
        );
    }

    /// An empty journal has no last receipt — there is no baseline to date at all,
    /// which [`journal_trace`] answers as [`JournalTrace::NoBaseline`] (grey).
    #[test]
    fn empty_journal_has_no_baseline_to_attribute_against() {
        assert_eq!(detect_baseline(&[], "b3:anything"), None);
    }

    /// The grey is a state of the TRACE, not a colour a renderer chooses: it is
    /// neither red nor summarisable as red, and it says why in words.
    #[test]
    fn no_baseline_is_grey_and_never_red() {
        let trace = JournalTrace::NoBaseline;
        assert!(!trace.is_red(), "grey is not red");
        assert!(trace.cannot_assess(), "grey names itself");
        assert_eq!(trace.red_summary(), None, "nothing to report as a lie");
        assert_eq!(
            trace.grey_summary().as_deref(),
            Some(NO_BASELINE_DETAIL),
            "the grey carries its reason"
        );
        assert_eq!(
            GREY_CANNOT_ASSESS, "grey(cannot-assess)",
            "S3-R6 rules ONE spelling for this reason word, shared with U11/U14 — \
             a local respelling fractures three units"
        );
    }

    /// **The false red, withdrawn** (S3-R8). A stale baseline is grey, not red:
    /// the same mismatch is left by an out-of-writer edit and by any write door
    /// that lands bytes without journaling, so check states the mismatch as
    /// evidence and declines to name a cause.
    ///
    /// U32 removed the splice from that list of doors — the render may no longer
    /// cite it, because a splice now journals its row and a governed corpus never
    /// reaches this state.
    #[test]
    fn a_stale_baseline_is_grey_and_states_evidence_without_naming_a_cause() {
        let trace = JournalTrace::StaleBaseline(BaselineMismatch {
            last_receipt: "r-000001".to_string(),
            recorded_root: "b3:RECORDED".to_string(),
            live_root: "b3:LIVE".to_string(),
        });
        assert!(
            !trace.is_red(),
            "a non-journaling write door produces this state — accusing is the false red"
        );
        assert!(trace.cannot_assess(), "grey names itself");
        assert_eq!(trace.red_summary(), None, "no lie may be claimed from it");
        let grey = trace.grey_summary().expect("the grey carries its reason");
        assert!(
            grey.contains("r-000001") && grey.contains("b3:RECORDED") && grey.contains("b3:LIVE"),
            "the EVIDENCE survives the withdrawn claim: {grey}"
        );
        assert!(
            grey.contains("does not account for"),
            "and it says what it could not do, rather than who did it: {grey}"
        );
        assert!(
            !grey.contains("splice"),
            "U32: a governed splice journals its row, so it is no longer a cause \
             this state can have — citing it teaches the reader a defect that is \
             fixed: {grey}"
        );
    }

    /// A TRACE read against a current baseline is assessed: it can be green (and
    /// `cannot_assess` is false), so the grey cannot leak into that path.
    #[test]
    fn an_assessed_trace_is_never_grey() {
        let trace = JournalTrace::Assessed {
            chain: check_chain(&[row("r-000001", "b3:0", "b3:1")]),
        };
        assert!(!trace.cannot_assess(), "rows were read — this is a verdict");
        assert!(!trace.is_red(), "one honest row, no break");
        assert_eq!(trace.grey_summary(), None);
    }

    /// A drifted claim surfaces as a [`ClaimFinding`]; a converged one does not.
    #[test]
    fn claims_realised_reports_only_drifted_claims() {
        let root = WorkspaceRoot(std::path::PathBuf::from("/nonexistent"));
        let claims = [
            fixed_claim("green", CheckOutcome::Converged),
            fixed_claim(
                "red",
                CheckOutcome::Drifted {
                    detail: "status: 'open' is 'closed'".to_string(),
                },
            ),
        ];
        let drifted = claims_realised(&root, &claims).expect("clean observations");
        assert_eq!(drifted.len(), 1, "only the drifted claim surfaces");
        assert_eq!(drifted[0].selector, "red");
        assert_eq!(drifted[0].detail, "status: 'open' is 'closed'");
    }

    // ── U14: the pin plane ──────────────────────────────────────────────────

    /// A corpus of one page, built the way `fs::build_corpus` builds one.
    fn corpus(path: &str, text: &str) -> BTreeMap<String, Document> {
        let doc = model::build(text.to_string(), syntax::parse(text));
        BTreeMap::from([(path.to_string(), doc)])
    }

    /// A `meridian-lock` block carrying one `objects:` entry, in the canonical
    /// bytes the engine mints.
    fn objects_page(key: &str, sha: &str) -> String {
        format!("# P\n\n```meridian-lock\nversion: 1\nobjects:\n  \"{key}\": \"{sha}\"\n```\n")
    }

    fn pin(color: Color) -> PinRow {
        PinRow {
            src_path: "claim.md".to_string(),
            declared_ref: "source.md#S/G".to_string(),
            label: "rendered by the ONE computer".to_string(),
            color,
        }
    }

    /// The sort is load-bearing in BOTH directions: a red pin is a FINDING and a
    /// grey pin is a refusal-for-want-of-evidence, and collapsing either into the
    /// other is what S3-R6's separate reason words exist to prevent. A sort that
    /// always said red fails the grey arm; one that always said grey fails the red.
    #[test]
    fn a_red_pin_is_a_finding_and_a_grey_pin_is_an_absence_of_evidence() {
        let root = WorkspaceRoot(std::path::PathBuf::from("/nonexistent"));
        let docs = BTreeMap::new();

        let red = pin_plane(
            &root,
            &docs,
            &[pin(Color::Red(model::selector::RedReason::Drifted))],
        );
        assert!(red.is_red(), "a drifted pin is a lie the ledger is telling");
        assert!(
            !red.cannot_assess(),
            "it was assessed — that is why it is red"
        );

        let grey = pin_plane(
            &root,
            &docs,
            &[pin(Color::Grey(model::selector::GreyReason::Ambiguous))],
        );
        assert!(!grey.is_red(), "grey is not red");
        assert!(grey.cannot_assess(), "grey names itself");
        assert!(!grey.is_green(), "and it is never green");

        let green = pin_plane(&root, &docs, &[pin(Color::Green)]);
        assert!(green.is_green(), "a green pin leaves nothing to report");
    }

    /// **An ORPHANED blob rides the FINDING leg; the bare three states do not.**
    ///
    /// The orphan is an ANSWER — git was asked and replied — not a question that
    /// could not be put, so grey is wrong for it. And the three states on their own
    /// are the ordinary lifecycle of a pin: a plane holding a `pending` and a
    /// `never` count with no orphan among them is **green**, because refusing there
    /// would refuse every governed commit (measured — see [`OrphanedBlob`]).
    #[test]
    fn an_orphaned_blob_is_a_finding_while_the_bare_three_states_are_a_reading() {
        let lifecycle = PinPlane {
            red: Vec::new(),
            grey: Vec::new(),
            orphaned: Vec::new(),
            anchored: 1,
            pending: 1,
            never: 1,
            cannot_ask: None,
        };
        assert_eq!(lifecycle.asked(), 3, "the population is the three states");
        assert!(
            lifecycle.is_green(),
            "staged and not-yet-added blobs are lifecycle moments, not defects — \
             refusing here refuses every governed commit"
        );

        let orphan = PinPlane {
            orphaned: vec![OrphanedBlob {
                src_path: "claim.md".to_string(),
                key: "source.md".to_string(),
                blob_sha: "a".repeat(40),
                state: ObjectAnchor::PendingAnchor,
                live: "b".repeat(40),
            }],
            ..lifecycle
        };
        assert!(orphan.is_red(), "the fence must refuse an orphan");
        assert!(
            !orphan.cannot_assess(),
            "and it must not be filed as an absence of evidence"
        );
    }

    /// **Honest degradation, in the direction that matters.** An object store that
    /// cannot be asked leaves the retrieval plane UNREAD — grey, never an empty
    /// `pending_anchor` list a reader could bank as clean. A `cannot_ask` that
    /// reported green would be the false green this whole unit exists to close,
    /// rebuilt one plane over.
    #[test]
    fn an_unaskable_object_store_is_grey_never_an_empty_clean_reading() {
        let root = WorkspaceRoot(std::path::PathBuf::from("/nonexistent"));
        let docs = corpus("claim.md", &objects_page("source.md", &"a".repeat(40)));
        let plane = pin_plane(&root, &docs, &[]);
        assert!(
            plane.cannot_ask.is_some(),
            "there is no repository at /nonexistent — the question could not be put"
        );
        assert!(plane.cannot_assess(), "so the plane is grey");
        assert!(!plane.is_green(), "and it is not clean");
        assert!(
            plane.asked() == 0 && plane.orphaned.is_empty() && !plane.is_red(),
            "the empty counts must NOT be readable as a verdict — `cannot_ask` is \
             what carries the meaning here"
        );
    }

    /// A corpus declaring no `objects:` entry asks git NOTHING: nothing is
    /// referenced, so nothing can be owed. Without this arm every workspace
    /// outside a repository would refuse, and a guard that blocks everything is
    /// indistinguishable from one that guards nothing (S3-R8(c)).
    #[test]
    fn a_corpus_that_references_no_blob_asks_nothing_and_stays_green() {
        let root = WorkspaceRoot(std::path::PathBuf::from("/nonexistent"));
        let docs = corpus("claim.md", "# Claim\n\nno lock at all.\n");
        let plane = pin_plane(&root, &docs, &[]);
        assert!(
            plane.cannot_ask.is_none(),
            "git was never asked, so there is nothing it could not answer"
        );
        assert!(plane.is_green(), "and the reading is a true zero");
    }

    /// A key naming ANOTHER root names another object store. This read holds one
    /// handle — the ambient one — so it REFUSES and says which root it could not
    /// reach. Falling back to the ambient store would ask the wrong database and
    /// answer confidently: a wrong SUCCESS, the one shape a fence's verb must
    /// never produce.
    #[test]
    fn a_rooted_objects_key_refuses_rather_than_asking_the_wrong_store() {
        let docs = corpus(
            "claim.md",
            &objects_page("alpha:source.md", &"a".repeat(40)),
        );
        let detail = objects_in(&docs).expect_err("a rooted key is not askable from here");
        assert!(
            detail.contains("alpha"),
            "it names the root whose store it could not ask: {detail}"
        );
        assert!(
            detail.contains("claim.md"),
            "and the page that declares it: {detail}"
        );
    }

    /// A value that is not an object id is REPORTED, never skipped: git cannot be
    /// asked about it, so its anchoring is unmeasurable — and a read that dropped
    /// it would report a corrupt retrieval plane as a true zero.
    #[test]
    fn a_value_that_is_not_an_object_id_is_reported_never_skipped() {
        let docs = corpus("claim.md", &objects_page("source.md", "not-an-oid"));
        let detail = objects_in(&docs).expect_err("git cannot be asked about a non-oid");
        assert!(
            detail.contains("not an object id"),
            "and it says why: {detail}"
        );
    }

    /// A page whose lock REFUSED contributes no objects — exactly as it
    /// contributes none to the listing planes. Its damage is already named by the
    /// grey `lock-refused` PIN row the same page projects, and naming one defect
    /// twice is how a reader comes to believe there are two.
    #[test]
    fn a_refused_lock_contributes_no_objects_because_its_pin_row_already_names_it() {
        let two_blocks = format!(
            "# P\n\n```meridian-lock\nversion: 1\n```\n\n```meridian-lock\nversion: 1\nobjects:\n  \
             \"source.md\": \"{}\"\n```\n",
            "a".repeat(40)
        );
        let docs = corpus("claim.md", &two_blocks);
        assert!(
            lock::find(docs.get("claim.md").expect("page")).is_err(),
            "the fixture is a REFUSED lock — two blocks on one page"
        );
        assert!(
            objects_in(&docs)
                .expect("a refusal is not this reader's error")
                .is_empty(),
            "so it declares no askable object here"
        );
    }

    /// A claim whose observation faults propagates the error (fail-loud), never a
    /// false green.
    #[test]
    fn claims_realised_propagates_an_observation_fault() {
        struct Faults;
        impl Check for Faults {
            fn observe(&self, _root: &WorkspaceRoot) -> Result<CheckOutcome, realise::CheckError> {
                Err(realise::CheckError {
                    selector: "faulty".to_string(),
                    reason: "observation faulted".to_string(),
                })
            }
        }
        let root = WorkspaceRoot(std::path::PathBuf::from("/nonexistent"));
        let claim = Claim {
            selector: "faulty".to_string(),
            check: Box::new(Faults),
            apply: None,
            retry_budget: 0,
        };
        let err = claims_realised(&root, std::slice::from_ref(&claim))
            .expect_err("a faulting observation is an error, not a drift");
        assert_eq!(err.reason, "observation faulted");
    }

    // ── the STAGED interval's baseline: the PAIR, each condition shown NECESSARY ──

    /// A journal page from `(anchor, root_before, root_after)` triples — the three
    /// facts a row asserts, in the shipped row grammar.
    fn journal(rows: &[(&str, &str, &str)]) -> String {
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

    /// The RECORD used by every case below: three governed writes, r1 -> r2 -> r3.
    fn record() -> String {
        journal(&[
            ("r-000001", "b3:GENESIS", "b3:R1"),
            ("r-000002", "b3:R1", "b3:R2"),
            ("r-000003", "b3:R2", "b3:R3"),
        ])
    }

    /// **THE FALSE RED, FIXED.** A legitimate partial stage: the staged journal is an
    /// earlier PREFIX (it ends at r1) while the staged tree folds to r2's
    /// `root_after`, because `git add` stages content without the journal. Dating it
    /// against its own last row calls that "something advanced the tree that the
    /// journal does not account for" — when the engine wrote both states.
    #[test]
    fn a_staged_intermediate_governed_state_is_accounted_for() {
        let staged = journal(&[("r-000001", "b3:GENESIS", "b3:R1")]);
        // The two conditions, measured apart — this is the arm that says WHY it passes.
        let rows = parse_rows(&staged);
        let recorded = parse_rows(&record());
        assert!(is_prefix_of(&rows, &recorded), "a truthful earlier journal");
        assert!(
            accounts_for(&recorded, "b3:R2"),
            "a root the engine produced"
        );
        // And its own journal does NOT date it, which is why the shipped test alone
        // produced the false red.
        assert!(
            matches!(detect_baseline(&rows, "b3:R2"), Some(Err(_))),
            "the own-journal test fails here BY CONSTRUCTION — that is the false red"
        );
        assert!(
            matches!(
                staged_trace(&staged, "b3:R2", &record()),
                JournalTrace::Assessed { ref chain } if chain.is_green()
            ),
            "so the record dates it instead, and the chain over the staged rows is green"
        );
    }

    /// **MEMBERSHIP IS NECESSARY — the prefix alone would let a CONTENT forgery
    /// through.** Forging a governed file leaves the journal untouched, so the prefix
    /// condition holds trivially; only membership can tell that no governed write
    /// ever produced these bytes.
    #[test]
    fn a_forged_tree_is_refused_and_the_prefix_condition_alone_would_not_see_it() {
        let staged = journal(&[("r-000001", "b3:GENESIS", "b3:R1")]);
        let rows = parse_rows(&staged);
        let recorded = parse_rows(&record());
        assert!(
            is_prefix_of(&rows, &recorded),
            "THE POINT: the forgery did not touch the journal, so the prefix holds"
        );
        assert!(
            !accounts_for(&recorded, "b3:FORGED"),
            "and membership is the ONLY condition that fails — it is load-bearing"
        );
        assert!(
            matches!(
                staged_trace(&staged, "b3:FORGED", &record()),
                JournalTrace::StaleBaseline(_)
            ),
            "so the interval refuses, grey, with the evidence it has"
        );
    }

    /// **THE PREFIX IS NECESSARY on the widened path** — membership alone would let a
    /// journal the record does not contain vouch for a staged tree.
    ///
    /// The journal is root-EXCLUDED from the hash domain, so rows invented only in
    /// the index move no fold. Here the staged tree folds to a REAL recorded root
    /// (so membership passes) while the staged journal carries a row the record
    /// never had — and the widened path refuses it on the prefix alone.
    #[test]
    fn the_widened_path_refuses_a_journal_the_record_does_not_contain() {
        let staged = journal(&[
            ("r-000001", "b3:GENESIS", "b3:R1"),
            ("r-000002", "b3:R1", "b3:INVENTED"),
        ]);
        let rows = parse_rows(&staged);
        let recorded = parse_rows(&record());
        // The widened path is the one under test: this journal does NOT date the
        // staged tree by itself, so acceptance can only come from the record.
        assert!(
            matches!(detect_baseline(&rows, "b3:R2"), Some(Err(_))),
            "the own-journal test does not date it — the widening is what is being asked"
        );
        assert!(
            accounts_for(&recorded, "b3:R2"),
            "THE POINT: membership PASSES — the staged tree is a real recorded root"
        );
        assert!(
            !is_prefix_of(&rows, &recorded),
            "and the prefix is the ONLY condition that fails — it is load-bearing"
        );
        assert!(
            matches!(
                staged_trace(&staged, "b3:R2", &record()),
                JournalTrace::StaleBaseline(_)
            ),
            "so it refuses"
        );
    }

    /// **RESIDUAL #2's staged face, asserted as the state it is rather than left to
    /// be discovered.** A journal fabricated in the index whose invented row DATES
    /// its own staged tree is ACCEPTED — the own-journal test is satisfied and the
    /// chain is continuous, so neither the record nor `check_chain` is consulted.
    ///
    /// **This is not new and it is not this widening's doing**: it is the same
    /// answer the first interval fix gave, and the same answer the shipped engine
    /// gives (which never reads the index at all). It is
    /// `fs::domain::RESERVED_JOURNAL_PATH` residual 2 — *root-preserving online
    /// forged-row insertion* — reached through the index, and it still requires
    /// writing the reserved journal out of band, which the receipt-engine-only
    /// restriction refuses and the git witness records.
    ///
    /// Asserted here so that a later reader finds the boundary stated, not implied:
    /// **the prefix condition bounds the WIDENED path, not the own-journal path.**
    #[test]
    fn a_fabricated_journal_that_dates_its_own_staged_tree_is_residual_two_not_a_new_hole() {
        let staged = journal(&[
            ("r-000001", "b3:GENESIS", "b3:R1"),
            ("r-000002", "b3:R1", "b3:FABRICATED"),
        ]);
        let rows = parse_rows(&staged);
        assert!(
            check_chain(&rows).is_green(),
            "the fabricated row CHAINS, so the chain recompute cannot see it"
        );
        assert!(
            matches!(detect_baseline(&rows, "b3:FABRICATED"), Some(Ok(()))),
            "and it dates its own staged tree, so the own-journal test accepts first"
        );
        assert!(
            matches!(
                staged_trace(&staged, "b3:FABRICATED", &record()),
                JournalTrace::Assessed { ref chain } if chain.is_green()
            ),
            "ACCEPTED — stated plainly. The record is not consulted on this path, and \
             pretending otherwise in a test name would be the false claim this lane hunts"
        );
        assert!(
            !accounts_for(&parse_rows(&record()), "b3:FABRICATED"),
            "the record never produced that root — which is what an s4 rider now owns"
        );
    }

    /// **A workspace with no record refuses, rather than accepting anything.** An
    /// empty row list is trivially a prefix, so this case rests entirely on
    /// membership — and with nothing recorded, nothing is accounted for.
    #[test]
    fn an_empty_record_accounts_for_nothing() {
        assert!(
            matches!(staged_trace("", "b3:R2", ""), JournalTrace::NoBaseline),
            "no rows anywhere is not a licence"
        );
    }

    // ── S4-R19: the pair as an ADDRESSABLE verdict ───────────────────────────

    /// **The four states are four different facts, and each one is REACHED.** A
    /// verdict that could only ever answer two of them is the one-bit channel this
    /// promotion exists to widen — so every arm here names a state no other arm
    /// produces, and a collapse of any pair fails at least one of them.
    #[test]
    fn the_scoped_verdict_tells_its_four_states_apart() {
        let record = record();
        let prefix = journal(&[("r-000001", "b3:GENESIS", "b3:R1")]);

        assert_eq!(
            interval_accounted(&prefix, "b3:R2", &record),
            Accounted::Yes,
            "a truthful prefix over a root the record produced is accounted for"
        );
        assert_eq!(
            interval_accounted(&prefix, "b3:FORGED", &record),
            Accounted::ForeignBytes {
                root: "b3:FORGED".to_string()
            },
            "a root no governed write produced is a FINDING, and the evidence is the root"
        );
        assert_eq!(
            interval_accounted(
                &journal(&[
                    ("r-000001", "b3:GENESIS", "b3:R1"),
                    ("r-000002", "b3:R1", "b3:INVENTED"),
                ]),
                "b3:R2",
                &record
            ),
            Accounted::ForeignJournal,
            "a row the record never had is a FINDING the fold cannot see — the journal \
             is root-EXCLUDED, so a spliced row moves nothing"
        );
        assert_eq!(
            interval_accounted(&prefix, "b3:R2", ""),
            Accounted::NoRecord,
            "and no record at all is an ABSENCE, not either finding"
        );
    }

    /// **The empty record is checked FIRST, and the ordering is load-bearing.**
    /// Both halves of the pair are false over an empty record, so a bare pair spells
    /// *"there is no ledger"* as *"you forged this"*. Under a journal that does not
    /// travel with a clone that mis-spelling is the COMMON case — every fresh clone
    /// accused of tampering.
    ///
    /// Reorder the checks and this fails: the answer becomes `ForeignJournal`.
    #[test]
    fn an_absent_record_is_an_absence_of_evidence_and_never_an_accusation() {
        let staged = journal(&[("r-000001", "b3:GENESIS", "b3:R1")]);
        let rows = parse_rows(&staged);

        // The measurement that makes the ordering necessary, stated rather than
        // implied: over an empty record BOTH conditions fail.
        assert!(
            !is_prefix_of(&rows, &[]),
            "the prefix condition is false over an empty record"
        );
        assert!(
            !accounts_for(&[], "b3:R1"),
            "and so is membership — a bare pair has no way to say WHICH absence this is"
        );

        let verdict = interval_accounted(&staged, "b3:R1", "");
        assert_eq!(
            verdict,
            Accounted::NoRecord,
            "so the state is named instead"
        );
        assert!(verdict.cannot_assess(), "grey — nothing could be read");
        assert!(!verdict.is_red(), "and never red: nobody was accused");
        assert!(!verdict.is_accounted(), "while still failing CLOSED");
        assert!(
            verdict.detail().contains("tampered with nothing"),
            "and the words say so out loud: {}",
            verdict.detail()
        );
    }

    /// **Accounting for an interval and vouching for a record are DIFFERENT
    /// questions**, and keeping them apart is the whole fix: the first is about
    /// THESE bytes and is answered fresh each commit, the second is about the past
    /// and is permanent.
    ///
    /// A broken chain must not stop the record accounting for an interval — that
    /// coupling is what made one out-of-band edit un-fence a repo forever. Collapse
    /// the two and this fails on its first assertion.
    #[test]
    fn a_broken_record_still_accounts_for_an_interval_and_still_fails_to_vouch() {
        // r2 does not continue r1: a hand-inserted row, mechanically.
        let broken = journal(&[
            ("r-000001", "b3:GENESIS", "b3:R1"),
            ("r-000002", "b3:SPLICED", "b3:R2"),
        ]);
        assert!(
            journal_trace_of(&broken, "b3:R2").is_red(),
            "FIXTURE: the chain is broken and the trace says so"
        );
        assert!(
            !journal_trace_of(&broken, "b3:R2").vouches(),
            "so the record cannot vouch for ITSELF — permanent, by design"
        );
        assert_eq!(
            interval_accounted(&broken, "b3:R2", &broken),
            Accounted::Yes,
            "and it STILL accounts for an interval it produced — the per-commit \
             question has its own answer, which is the entire point"
        );
        assert_eq!(
            interval_accounted(&broken, "b3:NEVER_PRODUCED", &broken),
            Accounted::ForeignBytes {
                root: "b3:NEVER_PRODUCED".to_string()
            },
            "and it still REFUSES what it never produced: the downgrade is of the \
             blocker, never of the detector"
        );
    }

    /// `vouches` is load-bearing in BOTH directions — a reading that always said
    /// yes would spell every broken record as intact, and one that always said no
    /// would print a standing break over corpora that have none, teaching operators
    /// to ignore it.
    #[test]
    fn only_an_assessed_continuous_chain_vouches_for_itself() {
        assert!(
            journal_trace_of(&record(), "b3:R3").vouches(),
            "assessed and continuous — the one state that vouches"
        );
        assert!(
            !journal_trace_of("", "b3:R3").vouches(),
            "no baseline: nothing was read, so nothing is vouched for"
        );
        assert!(
            !journal_trace_of(&record(), "b3:DRIFTED").vouches(),
            "stale baseline: the record cannot date the tree it is read against"
        );
        assert!(
            !JournalTrace::Assessed {
                chain: check_chain(&[
                    row("r-000001", "b3:GENESIS", "b3:R1"),
                    row("r-000002", "b3:SPLICED", "b3:R2"),
                ])
            }
            .vouches(),
            "broken chain: read, and found discontinuous"
        );
    }
}
