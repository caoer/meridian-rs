//! The env-var bridge — the two llm-wiki environment variables checked against
//! what the file declares for each.
//!
//! Owns: the two bridged variable names, the check of each stated path against
//! its authority, the four bridge states, the file-wins rule, and the
//! once-per-process divergence report.
//!
//! Never does: evaluate the `^config` block, decide what a root is called, or
//! change an exit code. The caller hands the declared `repos_root` in as data
//! ([`ReposRootDecl`]); a divergence is a note, not a refusal.
//!
//! Each variable has its own authority. `CCC_LLM_WIKI_PATH` names a wiki root,
//! so it is checked against the bound mount table. `CCC_LLM_WIKI_REPOS_ROOT`
//! names no root — a repos root is a fact about a wiki, and a mount over it
//! would widen every serving face to every repository — so it is checked
//! against `repos_root.<wiki>` from the `^config` block, `<wiki>` being the root
//! the wiki path agrees with.
//!
//! When a variable and the file disagree, the divergence is reported once per
//! process, never silently; for the wiki path the file wins. [`check`] has no `Err` variant,
//! and [`Bridged::mount`] is `Some` only on a wiki-path agreement — a variable
//! stating a tree the table does not bind names no root at all.
//!
//! Both checks compare through one canonicalization,
//! [`crate::mount::canonical_or_literal`], so a symlinked spelling, a
//! trailing-slash spelling and the real path are one lookup with one answer; a
//! second comparison here would be a second canonicalization law.
//!
//! A variable with nothing to be checked against is [`BridgeState::Unchecked`]
//! and carries the reason: an empty mount table (the inversion has not happened
//! on that machine), or no `repos_root` value for the wiki. The latch is per
//! variable, taken at check time, with the report carried in the returned
//! value.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::CONFIG_FILENAME;
use crate::mount::{MountTable, canonical_or_literal};

/// The llm-wiki path variable — rung 1 of the bridge (schema §5's first mount).
pub const WIKI_PATH_ENV_VAR: &str = "CCC_LLM_WIKI_PATH";

/// The repo-root variable — the one the llm-wiki's address layer resolves
/// through (`$CCC_LLM_WIKI_REPOS_ROOT/<slug>`).
pub const REPOS_ROOT_ENV_VAR: &str = "CCC_LLM_WIKI_REPOS_ROOT";

/// Which bridged variable. Closed: a third variable is a new declaration in the
/// file, not a new arm here.
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
    /// The file declares nothing to check this variable against — an empty
    /// mount table for the wiki path, no `repos_root` value for the repos
    /// root. The variable is honoured in silence; the detail says what is
    /// missing, so a misspelled key reads as missing, never as agreement.
    Unchecked {
        /// What the file lacks, in the operator's terms.
        detail: String,
    },
    /// The stated path canonicalizes onto what the file declares: the
    /// variable and the file agree, and agreement binds silently.
    Agrees {
        /// What it agreed with: the root name for the wiki path, the config
        /// key (`repos_root.<wiki>`) for the repos root.
        against: String,
        /// The canonical path both spellings resolve to.
        canonical: PathBuf,
    },
    /// The variable states a tree the file does not declare. The variable
    /// names no root, and the divergence is reported once per process.
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
            BridgeState::Unchecked { .. } => "unchecked",
            BridgeState::Agrees { .. } => "agrees",
            BridgeState::Diverges { .. } => "diverges",
        }
    }

    /// Why the variable was not checked, or why it diverges; `None` when it is
    /// unset or agrees.
    #[must_use]
    pub fn detail(&self) -> Option<&str> {
        match self {
            BridgeState::Unchecked { detail } | BridgeState::Diverges { detail, .. } => {
                Some(detail)
            }
            BridgeState::Unset | BridgeState::Agrees { .. } => None,
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

    /// The canonical root name this variable names — `Some` only on a wiki-path
    /// agreement: the mount table is the single authority on which roots exist,
    /// and the repos root is never one.
    #[must_use]
    pub fn mount(&self) -> Option<&str> {
        match (&self.state, self.var) {
            (BridgeState::Agrees { against, .. }, BridgeVar::WikiPath) => Some(against),
            _ => None,
        }
    }

    /// What the variable agreed with — the root name, or the `repos_root` key —
    /// `Some` only on agreement.
    #[must_use]
    pub fn against(&self) -> Option<&str> {
        match &self.state {
            BridgeState::Agrees { against, .. } => Some(against),
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

/// What the `^config` block declares for `repos_root`, read by the caller and
/// handed in as data: the bridge evaluates nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReposRootDecl {
    /// No value to check against, and why — no config file, no `^config`
    /// block, the block refused, or no `repos_root` key.
    Undeclared(String),
    /// `repos_root` is one string: a machine-wide value that needs no wiki.
    Machine(String),
    /// `repos_root` keyed by wiki, `repos_root.<wiki>` → path.
    ByWiki(BTreeMap<String, String>),
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

/// Check both bridged variables: the wiki path against the bound mount table,
/// the repos root against `repos` for the wiki the wiki path names.
///
/// Returns one [`Bridged`] per variable in [`BridgeVar::ALL`] order. Never
/// fails: a divergence is a note.
#[must_use]
pub fn check(env: &BridgeEnv, table: &MountTable, repos: &ReposRootDecl) -> Vec<Bridged> {
    check_in(env, table, repos, &REPORTED)
}

fn check_in(
    env: &BridgeEnv,
    table: &MountTable,
    repos: &ReposRootDecl,
    latch: &ReportLatch,
) -> Vec<Bridged> {
    let wiki = check_wiki_path(env.stated(BridgeVar::WikiPath), table, latch);
    let repos = check_repos_root(env.stated(BridgeVar::ReposRoot), wiki.mount(), repos, latch);
    vec![wiki, repos]
}

/// A variable's stated path, or `None` when it states none: unset, empty and
/// whitespace-only are one case.
fn stated_path(stated: Option<&str>) -> Option<&str> {
    stated.map(str::trim).filter(|s| !s.is_empty())
}

fn check_wiki_path(stated: Option<&str>, table: &MountTable, latch: &ReportLatch) -> Bridged {
    let var = BridgeVar::WikiPath;
    let Some(stated) = stated_path(stated) else {
        return Bridged::plain(var, BridgeState::Unset);
    };
    if table.mounts().is_empty() {
        return Bridged::plain(
            var,
            BridgeState::Unchecked {
                detail: "the mount table binds no roots, so there is nothing to check it against"
                    .to_string(),
            },
        );
    }

    if let Some(mount) = table.by_path(Path::new(stated)) {
        return Bridged::plain(
            var,
            BridgeState::Agrees {
                against: mount.name().to_string(),
                // A matched mount has a canonical path by construction; rendered
                // rather than unwrapped so a widened lookup cannot panic.
                canonical: mount
                    .canonical_path()
                    .map_or_else(|| PathBuf::from(stated), Path::to_path_buf),
            },
        );
    }

    // Only the detail re-reads the filesystem, and only to say why.
    let why = if std::fs::canonicalize(stated).is_ok() {
        "which resolves here but is not a root the file binds"
    } else {
        "which does not resolve on this machine, so it cannot be a bound root"
    };
    let name = var.name();
    let detail = format!(
        "{name} states {stated}, {why} — the file declares the roots and this variable is checked against it, so the FILE WINS and {name} names no root."
    );
    let report = latch.take(var).then(|| {
        format!(
            "note: {detail} Honoured for the bridge period and reported once per process. Fix: add a meridian-mount block for {stated} to {CONFIG_FILENAME}, or unset {name}."
        )
    });
    Bridged::diverges(var, stated, detail, report)
}

fn check_repos_root(
    stated: Option<&str>,
    wiki: Option<&str>,
    repos: &ReposRootDecl,
    latch: &ReportLatch,
) -> Bridged {
    let var = BridgeVar::ReposRoot;
    let unchecked = |detail: String| Bridged::plain(var, BridgeState::Unchecked { detail });
    let Some(stated) = stated_path(stated) else {
        return Bridged::plain(var, BridgeState::Unset);
    };

    let (key, declared) = match repos {
        ReposRootDecl::Undeclared(why) => {
            return unchecked(format!(
                "{why}, so there is no repos_root to check it against"
            ));
        }
        ReposRootDecl::Machine(path) => ("repos_root".to_string(), path.as_str()),
        ReposRootDecl::ByWiki(by_wiki) => {
            let Some(wiki) = wiki else {
                return unchecked(format!(
                    "{WIKI_PATH_ENV_VAR} names no bound wiki, so there is no repos_root.<wiki> to check it against"
                ));
            };
            let Some(path) = by_wiki.get(wiki) else {
                let keys: Vec<&str> = by_wiki.keys().map(String::as_str).collect();
                return unchecked(format!(
                    "the ^config block declares no repos_root.{wiki} (it declares [{}]), so there is nothing to check it against",
                    keys.join(", ")
                ));
            };
            (format!("repos_root.{wiki}"), path.as_str())
        }
    };

    let canonical = canonical_or_literal(Path::new(stated));
    if canonical == canonical_or_literal(Path::new(declared)) {
        return Bridged::plain(
            var,
            BridgeState::Agrees {
                against: key,
                canonical,
            },
        );
    }

    let name = var.name();
    let unresolved = if std::fs::canonicalize(stated).is_ok() {
        ""
    } else {
        ", which does not resolve on this machine"
    };
    let detail = format!(
        "{name} states {stated}{unresolved}, but the ^config block declares {key} = {declared} — two different trees."
    );
    let report = latch.take(var).then(|| {
        format!(
            "note: {detail} Readers of {name} look for repositories under {stated}; `mrd config get {key}` answers {declared}. Reported once per process. Fix: correct whichever is wrong — {name} where your shell profile exports it, or {key} in the ^config block of {CONFIG_FILENAME}."
        )
    });
    Bridged::diverges(var, stated, detail, report)
}

impl Bridged {
    fn plain(var: BridgeVar, state: BridgeState) -> Bridged {
        Bridged {
            var,
            state,
            report: None,
        }
    }

    fn diverges(var: BridgeVar, stated: &str, detail: String, report: Option<String>) -> Bridged {
        Bridged {
            var,
            state: BridgeState::Diverges {
                stated: stated.to_string(),
                detail,
            },
            report,
        }
    }
}

#[cfg(test)]
mod tests;
