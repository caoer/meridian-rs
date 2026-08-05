//! The env-var bridge — the two llm-wiki environment variables checked against the bound
//! mount table (U9).
//!
//! # Charter
//! **Owns:** the two bridged variable names, the check of each stated path against the
//! bound table, the four bridge states, the file-wins rule, and the once-per-process
//! divergence report.
//!
//! **Never does:** canonicalize a path itself, decide what a root is called, or change an
//! exit code. Canonicalization at bind is [`crate::mount`]'s and is reused whole through
//! [`MountTable::by_path`]; a divergence is a **note**, not a refusal.
//!
//! # The inversion, and the period this module IS
//!
//! `2026-07-24-meridian-md-entry-point.md` §5 (**silver**) ruled the relationship
//! flipped: the two llm-wiki environment variables — `CCC_LLM_WIKI_PATH` and the repo
//! root — **become mount entries** in `MERIDIAN.md`, and the wiki stops being the frame
//! that contains meridian. The migration is an **inversion, not a break**:
//!
//! > *Bridge period: ccc-statusd and the skills keep honoring the env vars while > they
//! > are derived from — or checked against — the file; then the env vars > demote to
//! > overrides only.*
//!
//! This module is the **"checked against"** half. Nothing here removes a variable, and
//! nothing here reads one to decide where a root lives — the file decides that. The
//! demotion to overrides is the period's END and is not this unit.
//!
//! # What "checked against" resolves to — the outcome the ruling implied
//!
//! *"Checked against"* implies an outcome the ratifying decision never stated: what
//! happens when the variable and the file **disagree**. Plan §5 rules it:
//!
//! > **THE FILE WINS, and the divergence is REPORTED ONCE PER PROCESS, never >
//! > silently.**
//!
//! **Fail-loud here would brick the CLI on every machine that exports the variable**, and
//! a bridge period whose mismatch is fatal is not a bridge. So divergence rides a note
//! and **never an exit code**: [`check`] returns a report, it does not return an error,
//! and there is no `Err` variant for a caller to be tempted into propagating.
//!
//! "The file wins" is made observable rather than asserted in prose: [`Bridged::mount`]
//! is `Some` **only** on agreement. A variable that states a tree the table does not bind
//! therefore names **no root at all** — the mount table stays the single authority on
//! which roots exist, which is precisely what the inversion means.
//!
//! # Why the check routes through `by_path`, and what it would otherwise build
//!
//! Measured on this machine before this module existed (S3-R7):
//!
//! | Fact | Value | |---|---| | `CCC_LLM_WIKI_PATH` | `/Users/Shared/projects/field-notes/`
//! — the real path, **with a trailing slash** | | `CCC_LLM_WIKI_REPOS_ROOT` |
//! `/Users/Shared/repos` | | `/Users/Shared/repos/field-notes` | a **symlink** to
//! `/Users/Shared/projects/field-notes` |
//!
//! A **literal** inversion — taking each variable's bytes as a mount path — binds **one
//! tree twice under two names**: two canonical refs over identical bytes with identical
//! `sec_rev`, which the read-mint recheck cannot tell apart, so **a receipt minted on ref
//! A would gate a pin on ref B.** That is a read-mint bypass, and it is the reason this
//! module compares **nothing** itself: [`MountTable::by_path`] canonicalizes its argument
//! by the same rule [`crate::mount::bind`] used, so the symlinked spelling, the
//! trailing-slash spelling and the real path are one lookup with one answer. A second
//! comparison written here would be a second canonicalization law, and the two would
//! drift.
//!
//! **The repos root shows the same trap from the other side, and it is the half a reader
//! would assume away.** `/Users/Shared/repos` *appears* to contain the wiki at
//! `/Users/Shared/repos/field-notes`, so a string-prefix reading would call the two mounts
//! nested and refuse them. Canonicalized they are **siblings** —
//! `/Users/Shared/projects/field-notes` is not under `/Users/Shared/repos` — and the
//! nesting **dissolves**. Canonicalization is therefore not only what collapses a false
//! distinction; it is also what dissolves a false collision, and a bridge reasoning on
//! the stated bytes would get **both** directions wrong.
//!
//! # The empty table is UNCHECKED, and that is what keeps this instrument alive
//!
//! An instrument's precision is what buys its survival (S3-R23): a check that cries wolf
//! is deleted by the next person it inconveniences, and then the invariant has no guard
//! at all.
//!
//! **Every machine today is state A** — no `MERIDIAN.md`, an empty mount table — while
//! both variables are exported. A bridge that called that a divergence would fire on
//! **100% of machines on its first run**, teach nothing, and be unset or patched out
//! within a day. So an empty table is [`BridgeState::Unchecked`]: the inversion has not
//! happened here, there is nothing to be inconsistent *with*, and the variable is
//! honoured in silence.
//!
//! This reuses the config plane's own nil-vs-empty identity rather than adding a branch:
//! state A (absent) and state D (a clean parse declaring zero mounts) both produce the
//! empty table and both reach this one arm.
//!
//! # The latch is per VARIABLE, and the word is deliberate
//!
//! The ruling says *once per process*. The latch is held **per variable**, not once for
//! the pair, because the two carry **different** facts: a shared latch would silence the
//! second variable's distinct divergence entirely, and "never silently" is the half of
//! the ruling that would be lost. Each divergence is reported once per process; neither
//! is reported twice.
//!
//! The latch is taken **at check time** and the report is carried in the returned value,
//! so holding a [`Bridged`] cannot print twice by accident — the same shape
//! [`crate::Config`] uses to make "no partial load" a property of the type rather than of
//! today's call sites.
//!
//!
//!
//!
//!
//!
//!
//!

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::CONFIG_FILENAME;
use crate::mount::MountTable;

/// The llm-wiki path variable — rung 1 of the bridge (schema §5's first mount).
pub const WIKI_PATH_ENV_VAR: &str = "CCC_LLM_WIKI_PATH";

/// The repo-root variable — *"the repo root"* the ratifying decision names
/// without spelling. Measured rather than assumed: this is the variable the
/// llm-wiki's own address layer resolves through (`$CCC_LLM_WIKI_REPOS_ROOT/<slug>`,
/// `LLM_WIKI.md`), and the one ccc-statusd's `effectiveSessionsRoot` already
/// reads beside `CCC_LLM_WIKI_PATH`.
pub const REPOS_ROOT_ENV_VAR: &str = "CCC_LLM_WIKI_REPOS_ROOT";

/// Which bridged variable. Closed: the ruling names two, and a third would be a
/// new mount entry with its own ruling, not a new arm here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeVar {
    /// `CCC_LLM_WIKI_PATH`.
    WikiPath,
    /// `CCC_LLM_WIKI_REPOS_ROOT`.
    ReposRoot,
}

impl BridgeVar {
    /// Both variables, in the order [`check`] reports them.
    pub const ALL: [BridgeVar; 2] = [BridgeVar::WikiPath, BridgeVar::ReposRoot];

    /// The variable's name in the environment.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            BridgeVar::WikiPath => WIKI_PATH_ENV_VAR,
            BridgeVar::ReposRoot => REPOS_ROOT_ENV_VAR,
        }
    }

    /// The latch slot. Private: the count is an implementation fact of the
    /// once-per-process report, not part of the vocabulary.
    fn slot(self) -> usize {
        match self {
            BridgeVar::WikiPath => 0,
            BridgeVar::ReposRoot => 1,
        }
    }
}

/// What the bridge decided about one variable.
///
/// Closed, and **none of the four is an error** — the bridge period's whole
/// point is that a disagreement is survivable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeState {
    /// The variable is unset, empty, or whitespace-only. It states no path, so
    /// there is nothing to check. Treated as unset exactly as an empty
    /// `MERIDIAN_CONFIG` is (`crate::resolve_rung`), so the two env axes cannot
    /// diverge on what "empty" means.
    Unset,
    /// The mount table binds no roots — state A or state D. The inversion has
    /// not happened on this machine, so the variable is honoured in silence.
    /// **This arm is what keeps the instrument from crying wolf on every
    /// unmigrated machine**, which is every machine today.
    Unchecked,
    /// The stated path canonicalizes onto a bound root. The **acceptance half**
    /// (S3-R8(c)): the variable and the file agree, and agreement binds
    /// **silently** — an operator who did the migration correctly is told
    /// nothing, which is the reward for doing it.
    Agrees {
        /// The canonical root name the file binds this tree to.
        mount: String,
        /// The canonical path both spellings resolve to.
        canonical: PathBuf,
    },
    /// The variable states a tree the table does not bind. **The file wins**:
    /// this variable names no root, and the divergence is reported once per
    /// process.
    Diverges {
        /// The path the variable states, verbatim — the spelling an operator
        /// will grep their shell profile for.
        stated: String,
        /// Why it did not match, in the operator's terms.
        detail: String,
    },
}

impl BridgeState {
    /// The state word — one spelling, used in the human line and in `--json`
    /// alike, for the same reason [`crate::mount::MountState::word`] is.
    #[must_use]
    pub fn word(&self) -> &'static str {
        match self {
            BridgeState::Unset => "unset",
            BridgeState::Unchecked => "unchecked",
            BridgeState::Agrees { .. } => "agrees",
            BridgeState::Diverges { .. } => "diverges",
        }
    }
}

/// One variable, checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bridged {
    var: BridgeVar,
    state: BridgeState,
    report: Option<String>,
}

impl Bridged {
    /// Which variable this is about.
    #[must_use]
    pub fn var(&self) -> BridgeVar {
        self.var
    }

    /// What the bridge decided.
    #[must_use]
    pub fn state(&self) -> &BridgeState {
        &self.state
    }

    /// The canonical root name this variable names — **`Some` only on
    /// agreement.**
    ///
    /// This is "the file wins" as a value rather than a sentence: a variable
    /// stating a tree the file does not bind names no root, because the mount
    /// table is the single authority on which roots exist.
    #[must_use]
    pub fn mount(&self) -> Option<&str> {
        match &self.state {
            BridgeState::Agrees { mount, .. } => Some(mount),
            _ => None,
        }
    }

    /// The canonical path the variable resolved onto — `Some` only on
    /// agreement, and the answer to *where did this root resolve from*.
    #[must_use]
    pub fn canonical(&self) -> Option<&Path> {
        match &self.state {
            BridgeState::Agrees { canonical, .. } => Some(canonical),
            _ => None,
        }
    }

    /// The divergence note — `Some` on the **first** observation of this
    /// variable's divergence in this process, `None` on every later one and on
    /// every non-diverging state.
    ///
    /// It is a note and never a refusal: it does not open with `refused:`, it
    /// carries no [`crate::NO_PARTIAL_LOAD_CLAUSE`], and nothing loaded is
    /// undone by it.
    #[must_use]
    pub fn report(&self) -> Option<&str> {
        self.report.as_deref()
    }
}

/// The two environment values the bridge reads, taken as data rather than from
/// the process — the same injection [`crate::Env`] uses, and for the same
/// reason: the states are distinguished BY these values, so a test that had to
/// mutate the process environment could not assert them in parallel.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BridgeEnv {
    /// `CCC_LLM_WIKI_PATH`. `None` = unset.
    pub wiki_path: Option<String>,
    /// `CCC_LLM_WIKI_REPOS_ROOT`. `None` = unset.
    pub repos_root: Option<String>,
}

impl BridgeEnv {
    /// Read both variables from the process environment. The one place the
    /// process is read.
    #[must_use]
    pub fn from_process() -> BridgeEnv {
        BridgeEnv {
            wiki_path: std::env::var(WIKI_PATH_ENV_VAR).ok(),
            repos_root: std::env::var(REPOS_ROOT_ENV_VAR).ok(),
        }
    }

    fn stated(&self, var: BridgeVar) -> Option<&str> {
        match var {
            BridgeVar::WikiPath => self.wiki_path.as_deref(),
            BridgeVar::ReposRoot => self.repos_root.as_deref(),
        }
    }
}

/// The once-per-process report latch — one flag per variable.
struct ReportLatch {
    taken: [AtomicBool; BridgeVar::ALL.len()],
}

impl ReportLatch {
    const fn new() -> ReportLatch {
        ReportLatch {
            taken: [AtomicBool::new(false), AtomicBool::new(false)],
        }
    }

    /// True exactly once per variable per latch. `swap` is the whole mechanism:
    /// two threads observing the same divergence at the same moment get one
    /// `true` between them, so "once" survives concurrency rather than assuming
    /// it away.
    fn take(&self, var: BridgeVar) -> bool {
        !self.taken[var.slot()].swap(true, Ordering::SeqCst)
    }
}

/// The process-global latch. `check` uses this one; the tests drive their own,
/// so the once-property is asserted deterministically rather than depending on
/// which test ran first.
static REPORTED: ReportLatch = ReportLatch::new();

/// Check both bridged variables against the bound mount table.
///
/// Returns one [`Bridged`] per variable in [`BridgeVar::ALL`] order. **Never
/// fails**: a divergence is a note, because a bridge period whose mismatch is
/// fatal is not a bridge.
#[must_use]
pub fn check(env: &BridgeEnv, table: &MountTable) -> Vec<Bridged> {
    check_in(env, table, &REPORTED)
}

fn check_in(env: &BridgeEnv, table: &MountTable, latch: &ReportLatch) -> Vec<Bridged> {
    BridgeVar::ALL
        .into_iter()
        .map(|var| check_one(var, env.stated(var), table, latch))
        .collect()
}

fn check_one(
    var: BridgeVar,
    stated: Option<&str>,
    table: &MountTable,
    latch: &ReportLatch,
) -> Bridged {
    let plain = |state| Bridged {
        var,
        state,
        report: None,
    };

    let Some(stated) = stated.map(str::trim).filter(|s| !s.is_empty()) else {
        return plain(BridgeState::Unset);
    };
    if table.mounts().is_empty() {
        return plain(BridgeState::Unchecked);
    }

    // The ONE comparison, and it is not written here: `by_path` canonicalizes
    // its argument by the same rule `bind` used, so a symlinked spelling and a
    // trailing slash reach the mount they name. A literal compare in this
    // module would be a second canonicalization law.
    if let Some(mount) = table.by_path(Path::new(stated)) {
        return plain(BridgeState::Agrees {
            mount: mount.name().to_string(),
            // A mount `by_path` matched has a canonical path by construction —
            // the match IS on it. Rendered rather than unwrapped so a future
            // widening of the lookup cannot turn a miss into a panic.
            canonical: mount
                .canonical_path()
                .map_or_else(|| PathBuf::from(stated), Path::to_path_buf),
        });
    }

    // Only the DETAIL re-reads the filesystem, and only to say WHY. The
    // decision above is `by_path`'s alone, so there is still one comparison.
    let why = if std::fs::canonicalize(stated).is_ok() {
        "which resolves here but is not a root the file binds"
    } else {
        "which does not resolve on this machine, so it cannot be a bound root"
    };
    let detail = format!(
        "{} states {stated}, {why} — the file declares the roots and this variable is checked against it, so the FILE WINS and {} names no root.",
        var.name(),
        var.name()
    );
    let report = latch.take(var).then(|| {
        format!(
            "note: {detail} Honoured for the bridge period and reported once per process. Fix: add a meridian-mount block for {stated} to {CONFIG_FILENAME}, or unset {}.",
            var.name()
        )
    });

    Bridged {
        var,
        state: BridgeState::Diverges {
            stated: stated.to_string(),
            detail,
        },
        report,
    }
}

#[cfg(test)]
mod tests;
