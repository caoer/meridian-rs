//! The env-var bridge — the two llm-wiki environment variables checked against
//! the bound mount table.
//!
//! Owns: the two bridged variable names, the check of each stated path against
//! the bound table, the four bridge states, the file-wins rule, and the
//! once-per-process divergence report.
//!
//! Never does: canonicalize a path itself, decide what a root is called, or
//! change an exit code. Canonicalization is reused whole through
//! [`MountTable::by_path`]; a divergence is a note, not a refusal.
//!
//! When the variable and the file disagree, the file wins and the divergence is
//! reported once per process, never silently. [`check`] has no `Err` variant,
//! and [`Bridged::mount`] is `Some` only on agreement — a variable stating a
//! tree the table does not bind names no root at all.
//!
//! The check routes through [`MountTable::by_path`], which canonicalizes by the
//! same rule bind used, so a symlinked spelling, a trailing-slash spelling and
//! the real path are one lookup with one answer; a second comparison here would
//! be a second canonicalization law.
//!
//! An empty mount table is [`BridgeState::Unchecked`]: the inversion has not
//! happened on that machine, so the variable is honoured in silence. The latch
//! is per variable, taken at check time, with the report carried in the
//! returned value.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::CONFIG_FILENAME;
use crate::mount::MountTable;

/// The llm-wiki path variable — rung 1 of the bridge (schema §5's first mount).
pub const WIKI_PATH_ENV_VAR: &str = "CCC_LLM_WIKI_PATH";

/// The repo-root variable — the one the llm-wiki's address layer resolves
/// through (`$CCC_LLM_WIKI_REPOS_ROOT/<slug>`).
pub const REPOS_ROOT_ENV_VAR: &str = "CCC_LLM_WIKI_REPOS_ROOT";

/// Which bridged variable. Closed: a third would be a new mount entry, not a
/// new arm here.
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

    /// The latch slot.
    fn slot(self) -> usize {
        match self {
            BridgeVar::WikiPath => 0,
            BridgeVar::ReposRoot => 1,
        }
    }
}

/// What the bridge decided about one variable. Closed; none of the four is an
/// error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeState {
    /// The variable is unset, empty, or whitespace-only: it states no path, so
    /// there is nothing to check. Same treatment as an empty `MERIDIAN_CONFIG`.
    Unset,
    /// The mount table binds no roots — state A or state D. The inversion has
    /// not happened on this machine, so the variable is honoured in silence.
    Unchecked,
    /// The stated path canonicalizes onto a bound root: the variable and the
    /// file agree, and agreement binds silently.
    Agrees {
        /// The canonical root name the file binds this tree to.
        mount: String,
        /// The canonical path both spellings resolve to.
        canonical: PathBuf,
    },
    /// The variable states a tree the table does not bind. The file wins:
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
    /// alike.
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

    /// The canonical root name this variable names — `Some` only on agreement:
    /// the mount table is the single authority on which roots exist.
    #[must_use]
    pub fn mount(&self) -> Option<&str> {
        match &self.state {
            BridgeState::Agrees { mount, .. } => Some(mount),
            _ => None,
        }
    }

    /// The canonical path the variable resolved onto — `Some` only on
    /// agreement.
    #[must_use]
    pub fn canonical(&self) -> Option<&Path> {
        match &self.state {
            BridgeState::Agrees { canonical, .. } => Some(canonical),
            _ => None,
        }
    }

    /// The divergence note — `Some` on the first observation of this
    /// variable's divergence in this process, `None` on every later one and on
    /// every non-diverging state. A note, never a refusal: nothing loaded is
    /// undone by it.
    #[must_use]
    pub fn report(&self) -> Option<&str> {
        self.report.as_deref()
    }
}

/// The two environment values the bridge reads, taken as data rather than from
/// the process — the same injection [`crate::Env`] uses.
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

    /// True exactly once per variable per latch: `swap` gives two racing
    /// threads one `true` between them, so "once" survives concurrency.
    fn take(&self, var: BridgeVar) -> bool {
        !self.taken[var.slot()].swap(true, Ordering::SeqCst)
    }
}

/// The process-global latch. `check` uses this one; the tests drive their own
/// so the once-property is asserted deterministically.
static REPORTED: ReportLatch = ReportLatch::new();

/// Check both bridged variables against the bound mount table.
///
/// Returns one [`Bridged`] per variable in [`BridgeVar::ALL`] order. Never
/// fails: a divergence is a note.
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

    // `by_path` canonicalizes by the same rule bind used; a literal compare
    // here would be a second canonicalization law.
    if let Some(mount) = table.by_path(Path::new(stated)) {
        return plain(BridgeState::Agrees {
            mount: mount.name().to_string(),
            // A matched mount has a canonical path by construction; rendered
            // rather than unwrapped so a widened lookup cannot panic.
            canonical: mount
                .canonical_path()
                .map_or_else(|| PathBuf::from(stated), Path::to_path_buf),
        });
    }

    // Only the detail re-reads the filesystem, and only to say why.
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
