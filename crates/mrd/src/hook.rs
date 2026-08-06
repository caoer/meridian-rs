//! The commit fence, read side: what is standing in this checkout's hook doors. Nothing here
//! writes — `mrd skill hook` emits the contract document, the agent reading it does the placing,
//! and this module reads the doors back so `mrd check` can report what a checkout is fenced by.
//!
//! The door set is [`FENCED_HOOKS`], and one body serves all three: each fires with the index
//! already holding what would be committed. A checkout carrying fewer than three is
//! `installed-partial`.
//!
//! `$GIT_DIR/hooks` is never a tracked path, so no clone, fetch or pull can transport the fence:
//! coverage is per-checkout and opt-in.

use std::fs;
use std::path::{Path, PathBuf};

/// The ownership marker, on the fence's second line. A file carrying it is one this engine's
/// document produced; a file without it belongs to another tool and is reported as
/// [`HookHere::Foreign`], never counted as coverage.
pub const HOOK_MARKER: &str = "mrd-hook-fence";

/// The generation this engine's document declares, and the datum every placed fence is judged
/// by. Bump it whenever the emitted body changes behaviour, or a stale fence reports as current.
/// `crates/mrd/tests/skill_hook_emit.rs` holds this number and the document's own
/// `mrd-hook-fence <n>` line to each other.
pub const FENCE_VERSION: u32 = 4;

/// Every door git offers a veto on for a commit it builds from a prepared index. Each is
/// veto-capable and fires with the index already holding what would be committed, so one body
/// serves them all and a checkout carrying fewer than three is partially fenced.
pub const FENCED_HOOKS: [&str; 3] = ["pre-commit", "pre-merge-commit", "pre-applypatch"];

// ---------------------------------------------------------------------------
// the version line, read
// ---------------------------------------------------------------------------

/// Read the generation a placed fence declares for itself. `None` when the line is absent or its
/// number unparseable — [`FENCE_VERSION`] is never substituted.
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

/// Why one root has no door plane to read. Every variant names the observed state, never a
/// guessed cause: an unreadable root is named, never reported as unfenced.
#[derive(Debug)]
pub enum Unfenceable {
    /// The root is not a git repository — a supported workspace state, not an error: there is
    /// nowhere for a hook to live.
    NotAGitRepo {
        /// The root asked about.
        root: PathBuf,
    },
    /// The root is a submodule of a superproject. Its hooks live at
    /// `<super>/.git/modules/<name>/hooks`, which nothing in this engine computes.
    Submodule {
        /// The root asked about.
        root: PathBuf,
        /// The superproject's working tree, as git reports it.
        superproject: PathBuf,
    },
    /// `core.hooksPath` redirects hooks away from the common dir, so the doors under
    /// `$GIT_COMMON_DIR/hooks` are not the ones git would run.
    HooksPathRedirected {
        /// The root asked about.
        root: PathBuf,
        /// Where git will actually look for hooks.
        hooks_path: PathBuf,
        /// The redirect target's own `pre-commit`, when it already has one: placing a fence
        /// anyway would write into another repository's hook directory.
        occupied_by: Option<PathBuf>,
    },
    /// The meridian workspace root is not the worktree top-level, so "this workspace" and "this
    /// repository" name different directories.
    WorkspaceNotToplevel {
        /// The meridian workspace root.
        workspace: PathBuf,
        /// The worktree top-level git reports.
        top_level: PathBuf,
    },
    /// Git could not answer. The refusal carries what failed.
    CannotAsk {
        /// The root asked about.
        root: PathBuf,
        /// What failed, verbatim.
        detail: String,
    },
}

impl Unfenceable {
    /// The reason word: one spelling per observed state. These describe a checkout's
    /// configuration, not a corpus verdict, so they never borrow `grey(...)` / `red(...)`.
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

    /// The teaching refusal: what was seen, and what the operator can do about it.
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

/// What is standing in `workspace`'s hook doors right now. Read-only: a root that is looked at
/// comes away byte-identical, including the roots that refuse.
///
/// # Errors
/// [`Unfenceable`] naming the observed state, with its teaching, when this root has no door
/// plane to read at all.
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
/// Guard order matters: a root can trip several at once, so the structural reason (no hook dir
/// is computable at all) is asked before the configured one (a hook dir exists, git looks
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
    // Canonicalize both sides: raw equality would call /var and /private/var two roots on macOS.
    let same = match (workspace.canonicalize().ok(), top_level.canonicalize().ok()) {
        (Some(a), Some(b)) => a == b,
        // Uncanonicalizable is not evidence of a mismatch — fall back to the raw comparison.
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
        // An unreadable file is not an absent one, and it is not ours either.
        return if path.exists() {
            HookHere::Foreign {
                first_line: "<unreadable>".to_owned(),
            }
        } else {
            HookHere::None
        };
    };
    if body.contains(HOOK_MARKER) {
        // The marker says whose the file is; the version line says what it is.
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
    /// A fence from this engine's document (it carries [`HOOK_MARKER`]), carrying the generation
    /// the file declares for itself. `None` when that line is absent or unparseable, never the
    /// asking engine's number.
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
/// [`FENCE_VERSION`]. Three-valued: the file can be older than, equal to, or newer than the
/// engine asking, and equality alone cannot tell older from newer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Currency {
    /// The placed generation is the one this engine's document declares.
    Current,
    /// Older than this engine emits: re-placing from `mrd skill hook` refreshes it.
    Superseded {
        /// The generation the file declares.
        installed: u32,
    },
    /// Newer than this engine. The remedy inverts: the `mrd` answering is the one that is
    /// behind, and re-placing from its document would downgrade the fence.
    Ahead {
        /// The generation the file declares.
        installed: u32,
    },
    /// The marker is there and no generation is declarable — never resolved into a direction.
    Unversioned,
}

/// The relation, computed. `None` in means [`Currency::Unversioned`] out.
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
    /// This one door's state, in the same vocabulary [`Coverage::word`] uses for the set.
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

    /// The generation this file declares — `None` when nothing is placed here or the line is
    /// undeclarable. Never the asking engine's number.
    #[must_use]
    pub fn version(&self) -> Option<u32> {
        match &self.here {
            HookHere::Ours { installed_version } => *installed_version,
            _ => None,
        }
    }
}

/// What the whole door set looks like on disk. A set's state is not any one door's state: "two
/// of three doors carry a current fence" is a distinct fact from "installed".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Coverage {
    /// One entry per [`FENCED_HOOKS`] name, in that order.
    pub doors: Vec<Door>,
}

impl Coverage {
    /// The one word for the whole set. Precedence is deliberate: a foreign file is named first,
    /// then `installed-ahead` before the other version relations — it is the only state whose
    /// remedy is the opposite of every other one's.
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

    /// The generation the placed fences declare — `None` when nothing is placed, or when the
    /// doors disagree.
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

    // ── the version line is a datum ──────────────────────────────────────────

    #[test]
    fn an_undeclarable_generation_is_never_resolved_into_the_asking_engines() {
        let good = format!("#!/bin/sh\n# {HOOK_MARKER} {FENCE_VERSION} — the fence.\n");
        assert_eq!(parse_fence_version(&good), Some(FENCE_VERSION));

        // The marker and the version share a line: spoil only the number, since deleting the
        // line would make the file foreign, a different state with a different word.
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

    // ── the door set is a claim about coverage ───────────────────────────────

    #[test]
    fn one_body_serves_every_door() {
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
        // the full set reads `installed`
        let full = Coverage {
            doors: FENCED_HOOKS.iter().map(|n| door(n, ours.clone())).collect(),
        };
        assert_eq!(full.word(), "installed");
        assert_eq!(full.teaching(), None);
        assert_eq!(full.fence_version(), Some(FENCE_VERSION));

        // a checkout carrying `pre-commit` alone is not fenced
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
        // doors that disagree are not one generation
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
