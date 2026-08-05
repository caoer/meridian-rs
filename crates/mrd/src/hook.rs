//! The commit fence, READ — what is standing in this checkout's hook doors. THE PLANE IS A
//! READER NOW, AND THE CONTRACT IS A DOCUMENT Nothing here writes. `mrd skill hook` emits
//! `skills/hook.md` — what to place, where, when to refuse to place it, how to verify — and the
//! agent reading that document does the placing. This module is the other half: it reads the
//! doors back, so `mrd check` can say what a checkout is actually fenced by (row 21). **The
//! install plane was deleted, not shimmed.** A verb that wrote into `$GIT_DIR/hooks` had to
//! carry an uninstaller that refused a file it did not write, a `flock` held across a
//! read-decide-write section spawning `git` three times inside it, a downgrade guard with its
//! own environment escape, and a partial-coverage migration state — four planes of imperative
//! machinery encoding rules that are, in the end, prose. They are now legible content of the
//! emitted document rather than code paths that have to be trusted, and the ONE thing that
//! cannot be prose — reading bytes off a disk to say what generation is standing there — is
//! what stayed. THE DOOR IS A CLASS, AND THE SET IS A CLAIM ABOUT COVERAGE (row 20)
//! `pre-commit` is not the only hook git dispatches for a commit it builds from a prepared
//! index. The set is [`FENCED_HOOKS`] — `pre-commit`, `pre-merge-commit` and `pre-applypatch` —
//! and **one body serves all three**: each fires with the index already holding what would be
//! committed, so the fence's question is the same at every one. A set of one left `git merge`
//! and `git am` landing commits past a fence that printed nothing, which is why [`Coverage`]
//! reports `installed-partial` as its own word rather than folding it into `installed`. THE
//! COVERAGE IS PER-CHECKOUT AND OPT-IN, PERMANENTLY (row 21) `$GIT_DIR/hooks` is never a
//! tracked path, so no clone, fetch or pull can transport the fence. That is git's design and
//! the fix cannot be "make the fence clonable". The automatic route — a global
//! `init.templateDir` — was measured working and is **refused on its collateral**: it fences
//! every unrelated repository the operator ever clones or inits, which abolishes the opt-in
//! premise the fence body's no-membership-test design rests on. **The defect to close was the
//! SILENCE, not the absence** — hence this reader, and the fence line in `mrd check` that
//! spends it.
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!

use std::fs;
use std::path::{Path, PathBuf};

/// The ownership marker, on the fence's second line. A file carrying it is one this engine's
/// document produced; a file without it belongs to another tool and is reported as
/// [`HookHere::Foreign`], never counted as coverage.
pub const HOOK_MARKER: &str = "mrd-hook-fence";

/// The generation this engine's document declares, and the datum every placed fence is judged
/// by. **Bump this whenever the emitted body changes behaviour**, because an unbumped body
/// change makes a stale fence report as current while doing something this engine does not do.
/// `crates/mrd/tests/skill_hook_emit.rs` holds this number and the document's own
/// `mrd-hook-fence <n>` line to each other.
///
///
pub const FENCE_VERSION: u32 = 4;

/// Every door git offers a veto on for a commit it builds from a prepared index. **Not a list
/// of names — a claim about coverage.** Each of the three is a pre-hook, is veto-capable, and
/// fires with the index already holding what would be committed, so one body serves them all
/// and a checkout carrying fewer than three is partially fenced rather than fenced.
///
///
pub const FENCED_HOOKS: [&str; 3] = ["pre-commit", "pre-merge-commit", "pre-applypatch"];

// ---------------------------------------------------------------------------
// the version line, read
// ---------------------------------------------------------------------------

/// Read the generation a placed fence declares for itself. **A report of the FILE's own
/// declaration, never of the asking engine's expectation.** `None` when the line is absent or
/// its number unparseable — the engine's own [`FENCE_VERSION`] is never substituted, because a
/// fence that cannot say what it is has not said it is current.
///
///
#[must_use]
pub fn parse_fence_version(body: &str) -> Option<u32> {
    body.lines().find_map(|line| {
        let rest = line.trim_start().strip_prefix('#')?.trim_start();
        rest.strip_prefix(HOOK_MARKER)?
            .split_whitespace()
            .next()?
            .parse()
            .ok()
    })
}

// ---------------------------------------------------------------------------
// the per-root verdict
// ---------------------------------------------------------------------------

/// Why one root has no door plane to read. **Every variant names the OBSERVED state, never a
/// guessed cause** — an unreadable root is named, never silently reported as unfenced, which is
/// the same absence wearing a false word.
#[derive(Debug)]
pub enum Unfenceable {
    /// The root is not a git repository. **Marker-beats-git makes this a SUPPORTED workspace state,
    /// not an error condition**: there is simply nowhere for a hook to live.
    ///
    NotAGitRepo {
        /// The root asked about.
        root: PathBuf,
    },
    /// The root is a submodule of a superproject (D12). Its hooks live at
    /// `<super>/.git/modules/<name>/hooks`, which nothing in this engine computes — a loud refusal
    /// beats a silent mis-read.
    Submodule {
        /// The root asked about.
        root: PathBuf,
        /// The superproject's working tree, as git reports it.
        superproject: PathBuf,
    },
    /// `core.hooksPath` redirects hooks away from the common dir, so the doors under
    /// `$GIT_COMMON_DIR/hooks` are not the ones git would run and reading them would answer about
    /// files with no effect.
    HooksPathRedirected {
        /// The root asked about.
        root: PathBuf,
        /// Where git will actually look for hooks.
        hooks_path: PathBuf,
        /// The redirect target's own `pre-commit`, when it already has one — the reason this refusal is
        /// stronger than "hooks are redirected": placing a fence anyway would write into ANOTHER
        /// repository's hook directory.
        ///
        occupied_by: Option<PathBuf>,
    },
    /// The meridian workspace root is not the worktree top-level, so "this workspace" and "this
    /// repository" name different directories and a per-root reading would be guessing which the
    /// operator meant (D11).
    WorkspaceNotToplevel {
        /// The meridian workspace root.
        workspace: PathBuf,
        /// The worktree top-level git reports.
        top_level: PathBuf,
    },
    /// Git could not answer. The refusal carries what failed; it never proceeds
    /// on a guess.
    CannotAsk {
        /// The root asked about.
        root: PathBuf,
        /// What failed, verbatim.
        detail: String,
    },
}

impl Unfenceable {
    /// The reason word — the OBSERVED state, one spelling per state. Measured free against the
    /// engine's existing reason-word set before minting (S3-R49): none collides with any word in
    /// `crates/*/src`, and none is a re-spelling of an existing concept — these describe a
    /// CHECKOUT's configuration, not a corpus verdict, so they neither borrow nor shadow
    /// `grey(...)` / `red(...)`.
    ///
    ///
    #[must_use]
    pub fn word(&self) -> &'static str {
        match self {
            Unfenceable::NotAGitRepo { .. } => "not-a-git-repo",
            Unfenceable::Submodule { .. } => "submodule",
            Unfenceable::HooksPathRedirected { .. } => "hooks-path-redirected",
            Unfenceable::WorkspaceNotToplevel { .. } => "workspace-not-toplevel",
            Unfenceable::CannotAsk { .. } => "cannot-ask-git",
        }
    }

    /// The teaching refusal: what was seen, and what the operator can do about
    /// it. It never prescribes an action already taken, and never accuses.
    #[must_use]
    pub fn teaching(&self) -> String {
        match self {
            Unfenceable::NotAGitRepo { root } => format!(
                "{} is not a git repository, so there is no hook directory to place a fence in. \
                 A meridian workspace does not have to be a git repository — this is a \
                 supported state, not a fault in the workspace.",
                root.display()
            ),
            Unfenceable::Submodule { root, superproject } => format!(
                "{} is a submodule of {}. A submodule's hooks live under \
                 <superproject>/.git/modules/<name>/hooks, which this engine does not compute — \
                 refusing rather than reporting on a directory git will not run.",
                root.display(),
                superproject.display()
            ),
            Unfenceable::HooksPathRedirected {
                root,
                hooks_path,
                occupied_by,
            } => {
                let mut line = format!(
                    "{} sets core.hooksPath = {}, so git runs hooks from there and never from \
                     this repository's own hooks directory. A fence placed here would be a file \
                     git will not run.",
                    root.display(),
                    hooks_path.display()
                );
                if let Some(existing) = occupied_by {
                    use std::fmt::Write as _;
                    let _ = write!(
                        line,
                        " That path already carries {} — placing there would write into \
                         another checkout's hook directory.",
                        existing.display()
                    );
                }
                line.push_str(" Unset core.hooksPath to fence this root.");
                line
            }
            Unfenceable::WorkspaceNotToplevel {
                workspace,
                top_level,
            } => format!(
                "the meridian workspace root is {} but the worktree top-level is {}. The fence \
                 is placed per git common dir and runs from the committing worktree, so a \
                 workspace nested below the top-level would be fenced by a commit it does not \
                 cover. Ask from {} instead.",
                workspace.display(),
                top_level.display(),
                top_level.display()
            ),
            Unfenceable::CannotAsk { root, detail } => format!(
                "cannot determine the hook directory for {}: {detail}",
                root.display()
            ),
        }
    }
}

impl std::fmt::Display for Unfenceable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} — {}", self.word(), self.teaching())
    }
}

// ---------------------------------------------------------------------------
// the read
// ---------------------------------------------------------------------------

/// What is standing in `workspace`'s hook doors right now.
///
/// **Read-only, and it leaves no souvenir.** Nothing in this engine writes into a
/// git directory any more, so a root that is merely looked at comes away
/// byte-identical — including the roots that refuse.
///
/// # Errors
/// [`Unfenceable`] naming the observed state, with its teaching, when this root
/// has no door plane to read at all.
pub fn status(workspace: &Path) -> Result<Coverage, Unfenceable> {
    let repo = git::Repo::at(workspace);
    let hooks = hooks_dir(&repo, workspace)?;
    Ok(Coverage {
        doors: FENCED_HOOKS
            .iter()
            .map(|name| {
                let path = hooks.join(name);
                Door {
                    name,
                    here: read_hook(&path),
                    path,
                }
            })
            .collect(),
    })
}

/// Where this root's doors live, or why it has none.
///
/// **Order is the design, not an accident.** A submodule may also have
/// `core.hooksPath` set, and a root refused for both would be reported by
/// whichever guard ran first — so the guard that names the *structural* reason
/// (nothing can compute this root's hook dir at all) is asked before the one
/// that names a *configured* reason (a hook dir exists, git just looks
/// elsewhere).
fn hooks_dir(repo: &git::Repo, workspace: &Path) -> Result<PathBuf, Unfenceable> {
    let cannot = |detail: String| Unfenceable::CannotAsk {
        root: workspace.to_path_buf(),
        detail,
    };

    let common_dir = repo.common_dir().map_err(|e| match e {
        git::GitFail::NotARepo { .. } => Unfenceable::NotAGitRepo {
            root: workspace.to_path_buf(),
        },
        other => cannot(other.to_string()),
    })?;

    if let Some(superproject) = repo.superproject().map_err(|e| cannot(e.to_string()))? {
        return Err(Unfenceable::Submodule {
            root: workspace.to_path_buf(),
            superproject,
        });
    }

    if let Some(hooks_path) = repo.hooks_path().map_err(|e| cannot(e.to_string()))? {
        let candidate = hooks_path.join("pre-commit");
        return Err(Unfenceable::HooksPathRedirected {
            root: workspace.to_path_buf(),
            hooks_path,
            occupied_by: candidate.exists().then_some(candidate),
        });
    }

    let top_level = repo.top_level().map_err(|e| cannot(e.to_string()))?;
    // Compare through the same canonicalization the workspace plane uses, so a
    // symlinked checkout is not reported as a mismatch it is not. Component-wise
    // equality alone would call /var and /private/var two roots on macOS.
    let same = match (workspace.canonicalize().ok(), top_level.canonicalize().ok()) {
        (Some(a), Some(b)) => a == b,
        // Uncanonicalizable is not evidence of a mismatch; fall back to the raw
        // comparison rather than refusing on a path we could not resolve.
        _ => workspace == top_level,
    };
    if !same {
        return Err(Unfenceable::WorkspaceNotToplevel {
            workspace: workspace.to_path_buf(),
            top_level,
        });
    }

    Ok(common_dir.join("hooks"))
}

/// Read what is at one hook path, without deciding anything about it.
fn read_hook(path: &Path) -> HookHere {
    let Ok(body) = fs::read_to_string(path) else {
        // An unreadable file is not an absent one — but it is also not ours,
        // and the report that follows names it rather than counting it.
        return if path.exists() {
            HookHere::Foreign {
                first_line: "<unreadable>".to_owned(),
            }
        } else {
            HookHere::None
        };
    };
    if body.contains(HOOK_MARKER) {
        // The marker says WHOSE the file is; the version line says WHAT it is.
        // Reading the marker and stopping there is what let a v1 fence answer
        // "installed" to a v2 engine's question.
        return HookHere::Ours {
            installed_version: parse_fence_version(&body),
        };
    }
    let first_line = body
        .lines()
        .find(|l| !l.trim().is_empty() && !l.starts_with("#!"))
        .unwrap_or("<empty>")
        .trim()
        .to_owned();
    HookHere::Foreign { first_line }
}

/// What sits at one hook path right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookHere {
    /// Nothing is placed.
    None,
    /// A fence from this engine's document (it carries [`HOOK_MARKER`]), carrying
    /// the generation the FILE declares for itself.
    ///
    /// # Why the generation is a reported datum (rows 23 + 26)
    /// The marker says WHOSE the file is; it cannot say WHAT it does. An older
    /// fence asks a different question of the engine than the current one, so it
    /// can pass what the current one refuses — **while a report that read only
    /// the marker calls it fenced.** A guard whose report cannot distinguish
    /// "fenced" from "fenced by a generation that misses the defect this one
    /// closes" is a green light with no lamp behind it.
    ///
    /// This is the file's own declaration and nothing else: `None` when the
    /// version line is absent or unparseable, never the asking engine's number.
    Ours {
        /// The generation parsed out of the placed bytes.
        installed_version: Option<u32>,
    },
    /// A file this engine's document did not produce.
    Foreign {
        /// Its first non-shebang, non-blank line, quoted verbatim.
        first_line: String,
    },
}

/// The observed relation between a placed fence's declared generation and this engine's
/// [`FENCE_VERSION`]. **Three-valued, because the file can be older than, equal to, or NEWER
/// than the engine asking.** A byte-equality test collapses *older* and *newer* into one
/// `false`, and a teaching built on it then asserts a direction the comparison never measured.
///
///
///
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Currency {
    /// The placed generation is the one this engine's document declares.
    Current,
    /// Older than this engine emits: re-placing from `mrd skill hook` refreshes
    /// it.
    Superseded {
        /// The generation the file declares.
        installed: u32,
    },
    /// **Newer than this engine.** The remedy inverts: the `mrd` answering is the one that is
    /// behind, and re-placing from ITS document would downgrade the fence.
    ///
    Ahead {
        /// The generation the file declares.
        installed: u32,
    },
    /// The marker is there and no generation is declarable. Not evidence of a
    /// direction, and never resolved into one.
    Unversioned,
}

/// The relation, computed. `None` in means [`Currency::Unversioned`] out — an
/// undeclarable generation is never resolved into the asking engine's own.
#[must_use]
pub fn currency(installed_version: Option<u32>) -> Currency {
    match installed_version {
        None => Currency::Unversioned,
        Some(v) if v == FENCE_VERSION => Currency::Current,
        Some(v) if v < FENCE_VERSION => Currency::Superseded { installed: v },
        Some(v) => Currency::Ahead { installed: v },
    }
}

/// One door of the set, and what is standing in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Door {
    /// The hook's git name — one of [`FENCED_HOOKS`].
    pub name: &'static str,
    /// Where it lives under `$GIT_COMMON_DIR/hooks`.
    pub path: PathBuf,
    /// What is at that path.
    pub here: HookHere,
}

impl Door {
    /// This one door's state, in the same vocabulary [`Coverage::word`] uses for
    /// the set — so a reader who learns the word once finds it at both scales.
    #[must_use]
    pub fn word(&self) -> &'static str {
        match &self.here {
            HookHere::None => "absent",
            HookHere::Foreign { .. } => "foreign-hook",
            HookHere::Ours { installed_version } => match currency(*installed_version) {
                Currency::Current => "installed",
                Currency::Superseded { .. } => "installed-superseded",
                Currency::Ahead { .. } => "installed-ahead",
                Currency::Unversioned => "installed-unversioned",
            },
        }
    }

    /// The generation THIS file declares — `None` when nothing is placed here
    /// or the line is undeclarable. Never the asking engine's number.
    #[must_use]
    pub fn version(&self) -> Option<u32> {
        match &self.here {
            HookHere::Ours { installed_version } => *installed_version,
            _ => None,
        }
    }
}

/// What the whole door set looks like on disk. **A set's state is not any one door's state.**
/// "Two of three doors carry a current fence" is a distinct fact from "installed", and R40
/// requires it be reported apart — it is exactly the state a checkout fenced from an older
/// document is in.
///
///
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Coverage {
    /// One entry per [`FENCED_HOOKS`] name, in that order.
    pub doors: Vec<Door>,
}

impl Coverage {
    /// The one word for the whole set. **Precedence is the design.** A foreign file is named first
    /// because it is the state nothing else explains. The version relations come next, and
    /// `installed-ahead` before the rest, because it is the only state whose remedy is the OPPOSITE
    /// of every other one's — every other word means "re-place the fence from `mrd skill hook`",
    /// and that word means "do not".
    ///
    ///
    #[must_use]
    pub fn word(&self) -> &'static str {
        if self.foreign().is_some() {
            return "foreign-hook";
        }
        if self
            .first_currency(|c| matches!(c, Currency::Ahead { .. }))
            .is_some()
        {
            return "installed-ahead";
        }
        if self
            .first_currency(|c| c == Currency::Unversioned)
            .is_some()
        {
            return "installed-unversioned";
        }
        if self
            .first_currency(|c| matches!(c, Currency::Superseded { .. }))
            .is_some()
        {
            return "installed-superseded";
        }
        let fenced = self.fenced_doors();
        if fenced == 0 {
            "absent"
        } else if fenced == self.doors.len() {
            "installed"
        } else {
            "installed-partial"
        }
    }

    /// What the operator can do about it — `None` when the word says it all.
    #[must_use]
    pub fn teaching(&self) -> Option<String> {
        if let Some(door) = self.foreign() {
            let first_line = match &door.here {
                HookHere::Foreign { first_line } => first_line.as_str(),
                _ => "",
            };
            return Some(format!(
                "{} is not this engine's ({first_line:?}); `mrd skill hook` says to refuse this \
                 door rather than overwrite it",
                door.name
            ));
        }
        if let Some((door, Currency::Ahead { installed })) =
            self.first_currency_door(|c| matches!(c, Currency::Ahead { .. }))
        {
            return Some(format!(
                "{} was placed from a NEWER engine's document than the one answering (fence \
                 {installed}, this engine {FENCE_VERSION}); the `mrd` first on PATH is behind the \
                 fence, so put the current engine first on PATH — do NOT re-place from \
                 `mrd skill hook` with this one, which would replace the fence with an older one",
                door.name
            ));
        }
        if let Some((door, _)) = self.first_currency_door(|c| c == Currency::Unversioned) {
            return Some(format!(
                "{} carries the marker but declares no readable generation, so its currency \
                 cannot be judged; `mrd skill hook` says to refuse this door rather than guess",
                door.name
            ));
        }
        if let Some((door, Currency::Superseded { installed })) =
            self.first_currency_door(|c| matches!(c, Currency::Superseded { .. }))
        {
            return Some(format!(
                "{} carries fence {installed} and this engine emits {FENCE_VERSION}; \
                 re-place it from `mrd skill hook`",
                door.name
            ));
        }
        let unfenced: Vec<&str> = self
            .doors
            .iter()
            .filter(|d| d.here == HookHere::None)
            .map(|d| d.name)
            .collect();
        if !unfenced.is_empty() && unfenced.len() < self.doors.len() {
            return Some(format!(
                "unfenced doors: {} — git dispatches these for commits it builds from a prepared \
                 index, so they are bypasses until `mrd skill hook`'s body is placed at them too",
                unfenced.join(", ")
            ));
        }
        None
    }

    /// The generation the placed fences declare — `None` when nothing is placed,
    /// or when the doors disagree, which is itself not one number.
    #[must_use]
    pub fn fence_version(&self) -> Option<u32> {
        let mut seen: Option<Option<u32>> = None;
        for door in &self.doors {
            if let HookHere::Ours { installed_version } = &door.here {
                match seen {
                    None => seen = Some(*installed_version),
                    Some(first) if first == *installed_version => {}
                    Some(_) => return None,
                }
            }
        }
        seen.flatten()
    }

    /// How many doors carry a fence from this engine's document.
    #[must_use]
    pub fn fenced_doors(&self) -> usize {
        self.doors
            .iter()
            .filter(|d| matches!(d.here, HookHere::Ours { .. }))
            .count()
    }

    fn foreign(&self) -> Option<&Door> {
        self.doors
            .iter()
            .find(|d| matches!(d.here, HookHere::Foreign { .. }))
    }

    fn first_currency_door(&self, pred: impl Fn(Currency) -> bool) -> Option<(&Door, Currency)> {
        self.doors.iter().find_map(|d| match &d.here {
            HookHere::Ours { installed_version } => {
                let c = currency(*installed_version);
                pred(c).then_some((d, c))
            }
            _ => None,
        })
    }

    fn first_currency(&self, pred: impl Fn(Currency) -> bool) -> Option<Currency> {
        self.first_currency_door(pred).map(|(_, c)| c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── the version line is a datum (rows 23 + 26) ───────────────────────────

    #[test]
    fn an_undeclarable_generation_is_never_resolved_into_the_asking_engines() {
        // The refusal arm for the parser: a marker-bearing fence whose generation
        // no `u32` parses must report None, NOT fall back to FENCE_VERSION. The
        // acceptance arm is the parse below; this one fails on a parser that
        // guesses, and the two disagree only on the guess.
        let good = format!("#!/bin/sh\n# {HOOK_MARKER} {FENCE_VERSION} — the fence.\n");
        assert_eq!(parse_fence_version(&good), Some(FENCE_VERSION));

        // The marker and the version share a line, so this fixture keeps the
        // marker and spoils only the number — deleting the line would make the
        // file FOREIGN, which is a different state with a different word.
        let spoiled = format!("#!/bin/sh\n# {HOOK_MARKER} next — the fence.\n");
        assert!(
            spoiled.contains(HOOK_MARKER),
            "the control: the marker survives"
        );
        assert_eq!(parse_fence_version(&spoiled), None);
        assert_eq!(currency(None), Currency::Unversioned);
    }

    #[test]
    fn the_relation_is_three_valued_and_names_the_direction_it_measured() {
        assert_eq!(currency(Some(FENCE_VERSION)), Currency::Current);
        assert_eq!(
            currency(Some(FENCE_VERSION - 1)),
            Currency::Superseded {
                installed: FENCE_VERSION - 1
            }
        );
        assert_eq!(
            currency(Some(FENCE_VERSION + 1)),
            Currency::Ahead {
                installed: FENCE_VERSION + 1
            },
            "a byte-equality test reported this state as `superseded`, asserting a direction \
             it never measured"
        );
    }

    // ── the door set is a claim about coverage (row 20) ──────────────────────

    #[test]
    fn one_body_serves_every_door() {
        // Each of the three fires with the index already holding what would be
        // committed, so the fence's question is the same at all three and there
        // is nothing to specialise per door.
        assert_eq!(FENCED_HOOKS.len(), 3);
        assert!(FENCED_HOOKS.contains(&"pre-commit"));
        assert!(
            FENCED_HOOKS.contains(&"pre-merge-commit"),
            "git dispatches this one for a merge commit it creates, and a door set \
             without it let `git merge` land past a fence that printed nothing"
        );
        assert!(FENCED_HOOKS.contains(&"pre-applypatch"), "`git am`'s door");
        let unique: std::collections::BTreeSet<_> = FENCED_HOOKS.iter().collect();
        assert_eq!(unique.len(), FENCED_HOOKS.len());
    }

    #[test]
    fn a_partly_fenced_set_is_its_own_word_and_not_installed() {
        let door = |name: &'static str, here: HookHere| Door {
            name,
            path: PathBuf::from("/x").join(name),
            here,
        };
        let ours = HookHere::Ours {
            installed_version: Some(FENCE_VERSION),
        };
        // ACCEPTANCE: the full set reads `installed`.
        let full = Coverage {
            doors: FENCED_HOOKS.iter().map(|n| door(n, ours.clone())).collect(),
        };
        assert_eq!(full.word(), "installed");
        assert_eq!(full.teaching(), None);
        assert_eq!(full.fence_version(), Some(FENCE_VERSION));

        // REFUSAL, in the same run: a checkout carrying `pre-commit` alone is the
        // claim about coverage that was false.
        let partial = Coverage {
            doors: vec![
                door("pre-commit", ours.clone()),
                door("pre-merge-commit", HookHere::None),
                door("pre-applypatch", HookHere::None),
            ],
        };
        assert_eq!(partial.word(), "installed-partial");
        let teaching = partial.teaching().expect("a partial set owes a teaching");
        assert!(
            teaching.contains("pre-merge-commit") && teaching.contains("pre-applypatch"),
            "the teaching names the open doors: {teaching}"
        );

        assert_eq!(
            Coverage {
                doors: FENCED_HOOKS
                    .iter()
                    .map(|n| door(n, HookHere::None))
                    .collect(),
            }
            .word(),
            "absent"
        );
    }

    #[test]
    fn the_version_relation_outranks_the_door_count_and_ahead_outranks_all() {
        let door = |name: &'static str, v: Option<u32>| Door {
            name,
            path: PathBuf::from("/x").join(name),
            here: HookHere::Ours {
                installed_version: v,
            },
        };
        let all = |v: Option<u32>| Coverage {
            doors: FENCED_HOOKS.iter().map(|n| door(n, v)).collect(),
        };
        assert_eq!(all(Some(FENCE_VERSION - 1)).word(), "installed-superseded");
        assert_eq!(all(None).word(), "installed-unversioned");
        // The word whose remedy is the OPPOSITE of every other word's, so it
        // must not be reachable only when nothing else applies.
        let ahead = all(Some(FENCE_VERSION + 1));
        assert_eq!(ahead.word(), "installed-ahead");
        let teaching = ahead.teaching().expect("the skew owes a teaching");
        assert!(
            teaching.contains("do NOT re-place"),
            "every other state's remedy is to re-place the fence; this one's is the reverse, \
             and a teaching that did not say so sends the operator to downgrade it: {teaching}"
        );
        // A foreign file at any door outranks a version relation at another.
        let mixed = Coverage {
            doors: vec![
                door("pre-commit", Some(FENCE_VERSION + 1)),
                Door {
                    name: "pre-merge-commit",
                    path: PathBuf::from("/x/pre-merge-commit"),
                    here: HookHere::Foreign {
                        first_line: "# husky".to_owned(),
                    },
                },
                door("pre-applypatch", Some(FENCE_VERSION)),
            ],
        };
        assert_eq!(mixed.word(), "foreign-hook");
        assert!(
            mixed
                .teaching()
                .expect("names the door")
                .contains("pre-merge-commit"),
            "a foreign hook beside an owned one must name WHICH door is foreign"
        );
        // Doors that disagree are not one generation, and none is invented.
        assert_eq!(mixed.fence_version(), None);
    }

    #[test]
    fn every_reason_word_is_distinct_and_names_an_observed_state() {
        let words = [
            Unfenceable::NotAGitRepo {
                root: PathBuf::from("/x"),
            }
            .word(),
            Unfenceable::Submodule {
                root: PathBuf::from("/x"),
                superproject: PathBuf::from("/y"),
            }
            .word(),
            Unfenceable::HooksPathRedirected {
                root: PathBuf::from("/x"),
                hooks_path: PathBuf::from("/y"),
                occupied_by: None,
            }
            .word(),
            Unfenceable::WorkspaceNotToplevel {
                workspace: PathBuf::from("/x"),
                top_level: PathBuf::from("/y"),
            }
            .word(),
            Unfenceable::CannotAsk {
                root: PathBuf::from("/x"),
                detail: String::new(),
            }
            .word(),
        ];
        let unique: std::collections::BTreeSet<_> = words.iter().collect();
        assert_eq!(
            unique.len(),
            words.len(),
            "two causes sharing one word is S3-R43 read backwards: {words:?}"
        );
    }
}
