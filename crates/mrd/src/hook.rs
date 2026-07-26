//! The git pre-commit fence — the boundary fence, installed per
//! `$GIT_COMMON_DIR` (U15, D11/D12).
//!
//! `mrd hook install` writes a `pre-commit` hook that calls `mrd check --staged`
//! and rejects on its exit. **Zero markdown semantics live in the hook** — it is
//! an adapter over the engine, the same law that keeps them out of ccc-statusd
//! (`2026-07-24-pin-content-anchoring-modes.md` §5, silver). Refusal's legal home
//! stays engine-side; the hook fences the one door the engine cannot see, the
//! out-of-band write (a human in Obsidian, a bash edit).
//!
//! # THE INTERVAL THE FENCE ASKS ABOUT (F1)
//! **git commits the INDEX**, so the fence asks `mrd check --staged` — the verb's
//! interval-bearing question — and not `mrd check`, which answers about the
//! worktree the hook happens to be standing in. The two part company on
//! `git add` + restore, `git add -p`, `git commit <pathspec>`, `git stash`, and any
//! concurrent writer between `git add` and hook fire; the shipped fence read the
//! worktree, answered green over bytes no commit would record, and let forged
//! bytes into history.
//!
//! **An OLDER installed fence therefore misses this**, which is why the body
//! carries a version and [`HookHere::Ours`] reports whether the installed bytes
//! are current: `mrd hook install` is idempotent and refreshes them, but nothing
//! can prompt an operator to run it if the status face calls a superseded fence
//! "installed".
//!
//! # Why this module is public
//! The R19 anti-vacuity harness has to drive [`HookLock`] across a fork window
//! it holds open BY HAND — a raw `fork(2)` whose child parks on `pause()`. A
//! test driving only the binary cannot do that: `Command::spawn` returns after
//! the child has exec'd, and exec closes the `O_CLOEXEC` lock fd, so the window
//! the hazard lives in is already shut by the time the test could look. The lock
//! type is therefore reachable from an integration test, and the CLI face
//! ([`crate::hook_cmd`]) is the thin wrapper over it.
//!
//! # The three senses of "root" meet here, and this module names them apart
//! - the **meridian workspace root** — what `mrd check` resolves and reads;
//! - the **worktree top-level** — `git rev-parse --show-toplevel`;
//! - the **common git dir** — `git rev-parse --git-common-dir`, where `hooks/`
//!   actually lives.
//!
//! **RULED (D11): install per common dir, and refuse-with-teaching when the
//! meridian workspace root is not the worktree top-level.** N linked worktrees
//! are N meridian workspaces sharing ONE hook directory, so the hook is written
//! once and reads its worktree from git's working directory at commit time — it
//! never bakes a path in. That is what makes one file correct for N workspaces.

use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

/// The ownership marker, on the hook's second line. `uninstall` removes a file
/// carrying it and **refuses one that does not** — the engine never deletes a
/// file it did not write, the read side of "never silently overwrite a file the
/// engine does not own".
pub const HOOK_MARKER: &str = "mrd-hook-fence";

/// The environment escape, in its hook spelling. A `pre-commit` hook receives no
/// arguments, so the ratified `--force` escape reaches it through the
/// environment; `git commit --no-verify` is git's own, and skips the hook
/// entirely without this file's help. Both are named in every refusal.
pub const FORCE_ENV: &str = "MRD_HOOK_FORCE";

/// The lock file, beside git's own lock files in the common dir. Its scope IS
/// the common dir: two worktrees of one repository are two meridian workspaces
/// racing for ONE `hooks/pre-commit`, so a lock keyed by workspace root would
/// let exactly that race through.
const LOCK_FILE: &str = "mrd-hook.lock";

/// The fence, verbatim as installed.
///
/// Every line here is adapter logic. There is no markdown semantics, no
/// selector, no rev, no colour — the verdict is `mrd check`'s exit and nothing
/// in this file may second-guess it.
///
/// It deliberately does NOT test for a `.meridian.toml` marker before running
/// the verb. Measured: none of the four operator roots carries one, and
/// `workspace::resolve` accepts a git root as a workspace without it — a marker
/// gate here would make the fence a silent no-op on exactly the roots it is
/// installed for.
const HOOK_BODY: &str = r#"#!/bin/sh
# mrd-hook-fence 2 — the meridian pre-commit fence.
#
# Installed by `mrd hook install`; removed by `mrd hook uninstall`.
# This file is an ADAPTER over the engine: it holds ZERO markdown semantics and
# decides nothing a verb could decide. The verdict below is `mrd check`'s exit.
set -u

# The ratified escape in its hook spelling: a pre-commit hook takes no
# arguments, so --force reaches it through the environment.
if [ -n "${MRD_HOOK_FORCE:-}" ]; then
	exit 0
fi

# git runs a hook with the working directory set to the worktree that is
# committing. N worktrees share this ONE file (it lives in the common git dir),
# so the worktree is read from here and never baked in at install time.
if ! command -v mrd >/dev/null 2>&1; then
	printf '%s\n' \
		'meridian fence: refusing — `mrd` is not on PATH, so this commit could not be checked.' \
		'  the fence fails CLOSED: a commit nobody could vouch for is not a verified one.' \
		"  escape:  MRD_HOOK_FORCE=1 git commit ...   (or: git commit --no-verify)" \
		'  remove:  mrd hook uninstall' >&2
	exit 1
fi

# --staged is the whole point of running here: git commits the INDEX, so the
# fence asks about the interval the commit spans and not about the worktree it
# happens to be standing in. `mrd check` alone answers a true question about the
# wrong bytes — staged forgery, restored worktree, forged bytes in history.
mrd check --staged
mrd_status=$?
if [ "$mrd_status" -ne 0 ]; then
	printf '%s\n' \
		"meridian fence: refusing this commit — \`mrd check --staged\` exited ${mrd_status}; its lines above say why." \
		"  escape:  MRD_HOOK_FORCE=1 git commit ...   (or: git commit --no-verify)" \
		'  remove:  mrd hook uninstall' >&2
	exit 1
fi
exit 0
"#;

// ---------------------------------------------------------------------------
// the R19 lock
// ---------------------------------------------------------------------------

/// The install lock: an exclusive advisory `flock(2)` on
/// `$GIT_COMMON_DIR/mrd-hook.lock`, held across the read-decide-write critical
/// section — the guards' `git` queries, the existing-hook read, and the write.
///
/// `LOCK_NB`: a held lock is [`io::ErrorKind::WouldBlock`] immediately, never a
/// wait, so a hung holder cannot make a second installer hang.
#[derive(Debug)]
pub struct HookLock {
    // Held open for its fd; released by the explicit `flock(LOCK_UN)` in Drop.
    file: fs::File,
}

/// Release the lock EXPLICITLY, before the fd closes (R19).
///
/// # Why the fd close is not enough, and why this path in particular
/// A `flock` lock belongs to the open file DESCRIPTION, and `fork` duplicates
/// every descriptor. **This module spawns subprocesses inside its critical
/// section by definition** — the submodule query, the `core.hooksPath` query and
/// the top-level query are three `git` processes forked while this lock is held,
/// and each one transiently holds a copy of this fd between its fork and its exec
/// (`FD_CLOEXEC` acts at exec, not at fork). Releasing by fd close would leak
/// the lock into any of them, and the next installer would refuse for a critical
/// section that had already finished.
///
/// `LOCK_UN` acts on the description itself, so one unlock releases the lock no
/// matter how many copies of the fd exist. Proven by
/// `crates/mrd/tests/u15_hook_lock_release.rs`, whose control holds the fork
/// window open by hand rather than racing for it.
impl Drop for HookLock {
    fn drop(&mut self) {
        // SAFETY: flock on a valid open fd we own; the fd outlives the call.
        unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_UN) };
    }
}

impl HookLock {
    /// Try to take the install lock for `common_dir`, creating the lock file on
    /// first use. Never blocks.
    ///
    /// # Errors
    /// [`io::ErrorKind::WouldBlock`] when another installer holds it; any other
    /// I/O failure creating or locking the file.
    pub fn acquire(common_dir: &Path) -> io::Result<Self> {
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(common_dir.join(LOCK_FILE))?;
        // SAFETY: flock on a valid open fd; the fd outlives the call.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { file })
    }
}

// ---------------------------------------------------------------------------
// the per-root verdict
// ---------------------------------------------------------------------------

/// Why one root cannot carry the fence. **Every variant names the OBSERVED
/// state, never a guessed cause**, and every one is reported at install time —
/// an unreachable root is named, never silently skipped.
#[derive(Debug)]
pub enum Unfenceable {
    /// The root is not a git repository. **Marker-beats-git makes this a
    /// SUPPORTED workspace state, not an error condition**: there is simply
    /// nowhere to install a hook.
    NotAGitRepo {
        /// The root asked about.
        root: PathBuf,
    },
    /// The root is a submodule of a superproject (D12). Its hooks live at
    /// `<super>/.git/modules/<name>/hooks`, which nothing in this engine
    /// computes — a loud refusal beats a silent mis-install.
    Submodule {
        /// The root asked about.
        root: PathBuf,
        /// The superproject's working tree, as git reports it.
        superproject: PathBuf,
    },
    /// `core.hooksPath` redirects hooks away from the common dir, so anything
    /// written to `$GIT_COMMON_DIR/hooks` is a silent no-op.
    HooksPathRedirected {
        /// The root asked about.
        root: PathBuf,
        /// Where git will actually look for hooks.
        hooks_path: PathBuf,
        /// The redirect target's own `pre-commit`, when it already has one —
        /// the reason this refusal is stronger than "hooks are redirected":
        /// installing anyway would write into ANOTHER repository's hook
        /// directory.
        occupied_by: Option<PathBuf>,
    },
    /// The meridian workspace root is not the worktree top-level, so "this
    /// workspace" and "this repository" name different directories and a
    /// per-root install would be guessing which the operator meant (D11).
    WorkspaceNotToplevel {
        /// The meridian workspace root.
        workspace: PathBuf,
        /// The worktree top-level git reports.
        top_level: PathBuf,
    },
    /// A `pre-commit` hook already exists and this engine did not write it.
    /// **Refused by default, naming the file** — never a silent overwrite of
    /// another tool's artifact.
    ForeignHook {
        /// The existing file, named so the operator can read it.
        path: PathBuf,
        /// Its first non-shebang line, quoted so the refusal says WHOSE it is
        /// without this engine guessing at a tool name.
        first_line: String,
    },
    /// Git could not answer, or the hook directory could not be written. The
    /// refusal carries what failed; it never proceeds on a guess.
    CannotAsk {
        /// The root asked about.
        root: PathBuf,
        /// What failed, verbatim.
        detail: String,
    },
}

impl Unfenceable {
    /// The reason word — the OBSERVED state, one spelling per state.
    ///
    /// Measured free against the engine's existing reason-word set before
    /// minting (S3-R49): none of these five collides with any of the ~110 words
    /// in `crates/*/src`, and none is a re-spelling of an existing concept —
    /// these describe an INSTALL refusal, not a corpus verdict, so they neither
    /// borrow nor shadow `grey(...)` / `red(...)`.
    #[must_use]
    pub fn word(&self) -> &'static str {
        match self {
            Unfenceable::NotAGitRepo { .. } => "not-a-git-repo",
            Unfenceable::Submodule { .. } => "submodule",
            Unfenceable::HooksPathRedirected { .. } => "hooks-path-redirected",
            Unfenceable::WorkspaceNotToplevel { .. } => "workspace-not-toplevel",
            Unfenceable::ForeignHook { .. } => "foreign-hook",
            Unfenceable::CannotAsk { .. } => "cannot-ask-git",
        }
    }

    /// The teaching refusal: what was seen, and what the operator can do about
    /// it. It never prescribes an action already taken, and never accuses.
    #[must_use]
    pub fn teaching(&self) -> String {
        match self {
            Unfenceable::NotAGitRepo { root } => format!(
                "{} is not a git repository, so there is no hook directory to install into. \
                 A meridian workspace does not have to be a git repository — this is a \
                 supported state, not a fault in the workspace.",
                root.display()
            ),
            Unfenceable::Submodule { root, superproject } => format!(
                "{} is a submodule of {}. A submodule's hooks live under \
                 <superproject>/.git/modules/<name>/hooks, which this engine does not compute — \
                 refusing rather than installing where git will not look.",
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
                     this repository's own hooks directory. Installing would write a file git \
                     will not run.",
                    root.display(),
                    hooks_path.display()
                );
                if let Some(existing) = occupied_by {
                    use std::fmt::Write as _;
                    let _ = write!(
                        line,
                        " That path already carries {} — installing there would write into \
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
                 is installed per git common dir and runs from the committing worktree, so a \
                 workspace nested below the top-level would be fenced by a commit it does not \
                 cover. Install from {} instead.",
                workspace.display(),
                top_level.display(),
                top_level.display()
            ),
            Unfenceable::ForeignHook { path, first_line } => format!(
                "{} already exists and this engine did not write it (its first line reads {:?}). \
                 Refusing rather than overwriting a file the engine does not own. Move or remove \
                 that hook and run install again.",
                path.display(),
                first_line
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

/// A root the fence CAN reach, with the three directories named apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fenceable {
    /// The meridian workspace root (== the worktree top-level, checked).
    pub workspace: PathBuf,
    /// `git rev-parse --git-common-dir` — where `hooks/` lives.
    pub common_dir: PathBuf,
    /// The `pre-commit` file this install writes.
    pub hook_path: PathBuf,
}

/// What an install did. **A state change, not an exit code** (R40): `Fresh` and
/// `AlreadyInstalled` are different facts about the disk and are reported apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Installed {
    /// The hook was written where none was before.
    Fresh,
    /// A fence written by this engine was already there; its bytes were
    /// refreshed. Install is idempotent, and says which it was.
    AlreadyInstalled,
}

/// What an uninstall did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Removed {
    /// The fence was there and is gone.
    Removed,
    /// There was no fence to remove.
    Absent,
}

// ---------------------------------------------------------------------------
// the operations
// ---------------------------------------------------------------------------

/// Survey `workspace` read-only: can the fence reach this root, and where would
/// it land? Writes nothing, takes no lock — this is the report an operator gets
/// for a root before anything touches it.
///
/// # Errors
/// [`Unfenceable`] naming the observed state, with its teaching.
pub fn survey(workspace: &Path) -> Result<Fenceable, Unfenceable> {
    let repo = git::Repo::at(workspace);
    let common_dir = common_dir_of(&repo, workspace)?;
    guards(&repo, workspace, common_dir)
}

/// Install the fence for `workspace`'s repository.
///
/// The lock is taken as soon as the common dir is known and is held across every
/// remaining git query, the existing-hook read, and the write — the whole
/// read-decide-write section (R19: it releases explicitly, never by fd close).
///
/// # Errors
/// [`Unfenceable`] naming the observed state.
pub fn install(workspace: &Path) -> Result<(Fenceable, Installed), Unfenceable> {
    let repo = git::Repo::at(workspace);
    let fenceable = locked_decision(&repo, workspace)?;
    let _lock = fenceable.lock;
    let fenceable = fenceable.fenceable;
    let state = match read_hook(&fenceable.hook_path) {
        HookHere::None => Installed::Fresh,
        HookHere::Ours { .. } => Installed::AlreadyInstalled,
        HookHere::Foreign { first_line } => {
            return Err(Unfenceable::ForeignHook {
                path: fenceable.hook_path.clone(),
                first_line,
            });
        }
    };

    write_hook(&fenceable, workspace)?;
    Ok((fenceable, state))
}

/// Remove a fence this engine wrote. **Refuses a `pre-commit` it did not
/// write** — an uninstall that deleted a foreign hook would be the overwrite
/// defect wearing the other sign.
///
/// # Errors
/// [`Unfenceable`] naming the observed state.
pub fn uninstall(workspace: &Path) -> Result<(Fenceable, Removed), Unfenceable> {
    let repo = git::Repo::at(workspace);
    let decision = locked_decision(&repo, workspace)?;
    let _lock = decision.lock;
    let fenceable = decision.fenceable;
    match read_hook(&fenceable.hook_path) {
        HookHere::None => Ok((fenceable, Removed::Absent)),
        HookHere::Foreign { first_line } => Err(Unfenceable::ForeignHook {
            path: fenceable.hook_path.clone(),
            first_line,
        }),
        HookHere::Ours { .. } => {
            fs::remove_file(&fenceable.hook_path).map_err(|e| Unfenceable::CannotAsk {
                root: workspace.to_path_buf(),
                detail: format!("cannot remove {} ({e})", fenceable.hook_path.display()),
            })?;
            Ok((fenceable, Removed::Removed))
        }
    }
}

/// Is a fence installed for `workspace`'s repository, and whose is it?
///
/// # Errors
/// [`Unfenceable`] naming the observed state — but note a foreign hook is a
/// STATE here, reported as [`HookHere::Foreign`], not a refusal: reporting is
/// what this verb is for.
pub fn status(workspace: &Path) -> Result<(Fenceable, HookHere), Unfenceable> {
    let fenceable = survey(workspace)?;
    let here = read_hook(&fenceable.hook_path);
    Ok((fenceable, here))
}

/// What sits at the `pre-commit` path right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookHere {
    /// Nothing is installed.
    None,
    /// A fence this engine wrote (it carries [`HOOK_MARKER`]), with whether its
    /// bytes are the ones this engine writes TODAY.
    ///
    /// # Why the currency of the bytes is a reported state (F1)
    /// The marker says WHOSE the file is; it cannot say WHAT it does. An older
    /// fence runs `mrd check` where the current one runs `mrd check --staged`, so
    /// it reads the worktree and passes a staged forgery — **while
    /// `mrd hook status` reports it as installed.** A guard whose report cannot
    /// distinguish "fenced" from "fenced by a version that misses the defect this
    /// one closes" is a green light with no lamp behind it, and the operator has
    /// no way to know a re-install is owed.
    Ours {
        /// The installed bytes are byte-for-byte what [`install`] writes now.
        current: bool,
    },
    /// A file this engine did not write.
    Foreign {
        /// Its first non-shebang, non-blank line, quoted verbatim.
        first_line: String,
    },
}

// ---------------------------------------------------------------------------
// the pieces
// ---------------------------------------------------------------------------

/// A root that passed the guards, plus the lock held over the decision.
struct LockedDecision {
    fenceable: Fenceable,
    lock: HookLock,
}

/// Survey UNLOCKED, then take the lock and survey again — the shape both write
/// verbs share.
///
/// # Why the guards run twice, and why that is not theatre
/// The lock file lives in `$GIT_COMMON_DIR`, so taking it is itself a write into
/// the repository. **A root the fence cannot reach must not be written to at
/// all** — a refused `ccc-statusd` or a refused submodule may not come away with
/// an `mrd-hook.lock` in its git dir as the souvenir of being told no. So the
/// cheap read-only pass decides whether to touch the root, and the second pass —
/// under the lock, spawning git inside the critical section — is the decision the
/// write actually rides on.
///
/// That second pass is also where R19 has its instance: three `git` processes are
/// forked while this lock is held, each transiently holding a copy of its fd.
fn locked_decision(repo: &git::Repo, workspace: &Path) -> Result<LockedDecision, Unfenceable> {
    let common_dir = common_dir_of(repo, workspace)?;
    // Pass one: read-only. A refusal here leaves the root byte-identical.
    guards(repo, workspace, common_dir.clone())?;

    let lock = HookLock::acquire(&common_dir).map_err(|e| Unfenceable::CannotAsk {
        root: workspace.to_path_buf(),
        detail: format!(
            "cannot take the install lock in {} ({e}) — another `mrd hook` holds it",
            common_dir.display()
        ),
    })?;

    // Pass two: the decision the write rides on, taken while nothing else can be
    // deciding it. A root that acquired `core.hooksPath` between the two passes
    // refuses here, having written only a lock file into a repository that was
    // fenceable when we asked.
    let fenceable = guards(repo, workspace, common_dir)?;
    Ok(LockedDecision { fenceable, lock })
}

/// The one git query that has to happen before the lock exists: the lock lives
/// IN the common dir, so the common dir has to be known to take it. Everything
/// that DECIDES anything runs inside the lock ([`guards`]).
fn common_dir_of(repo: &git::Repo, workspace: &Path) -> Result<PathBuf, Unfenceable> {
    repo.common_dir().map_err(|e| match e {
        git::GitFail::NotARepo { .. } => Unfenceable::NotAGitRepo {
            root: workspace.to_path_buf(),
        },
        other => Unfenceable::CannotAsk {
            root: workspace.to_path_buf(),
            detail: other.to_string(),
        },
    })
}

/// Every guard that decides whether this root is fenceable, in the order a
/// refusal is most useful: submodule (D12) → `core.hooksPath` (D11) → the
/// workspace/top-level mismatch (D11).
///
/// **Order is the design, not an accident.** A submodule may also have
/// `core.hooksPath` set, and a root refused for both would be reported by
/// whichever guard ran first — so the guard that names the *structural* reason
/// (nothing can compute this root's hook dir at all) is asked before the one
/// that names a *configured* reason (a hook dir exists, git just looks
/// elsewhere).
fn guards(
    repo: &git::Repo,
    workspace: &Path,
    common_dir: PathBuf,
) -> Result<Fenceable, Unfenceable> {
    let cannot = |detail: String| Unfenceable::CannotAsk {
        root: workspace.to_path_buf(),
        detail,
    };

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

    Ok(Fenceable {
        workspace: workspace.to_path_buf(),
        hook_path: common_dir.join("hooks").join("pre-commit"),
        common_dir,
    })
}

/// Read what is at the hook path, without deciding anything about it.
fn read_hook(path: &Path) -> HookHere {
    let Ok(body) = fs::read_to_string(path) else {
        // An unreadable file is not an absent one — but it is also not ours,
        // and the refusal that follows names it rather than overwriting it.
        return if path.exists() {
            HookHere::Foreign {
                first_line: "<unreadable>".to_owned(),
            }
        } else {
            HookHere::None
        };
    };
    if body.contains(HOOK_MARKER) {
        return HookHere::Ours {
            current: body == HOOK_BODY,
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

/// Write the hook and make it executable. A hook git cannot execute is a hook
/// git skips — the chmod is the install, not a decoration on it.
fn write_hook(fenceable: &Fenceable, workspace: &Path) -> Result<(), Unfenceable> {
    let cannot = |detail: String| Unfenceable::CannotAsk {
        root: workspace.to_path_buf(),
        detail,
    };
    let hooks_dir = fenceable
        .hook_path
        .parent()
        .expect("the hook path is always <common-dir>/hooks/pre-commit");
    fs::create_dir_all(hooks_dir)
        .map_err(|e| cannot(format!("cannot create {} ({e})", hooks_dir.display())))?;
    fs::write(&fenceable.hook_path, HOOK_BODY).map_err(|e| {
        cannot(format!(
            "cannot write {} ({e})",
            fenceable.hook_path.display()
        ))
    })?;
    fs::set_permissions(&fenceable.hook_path, fs::Permissions::from_mode(0o755)).map_err(|e| {
        cannot(format!(
            "cannot make {} executable ({e})",
            fenceable.hook_path.display()
        ))
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_installed_hook_carries_its_marker_and_names_both_escapes() {
        assert!(
            HOOK_BODY.contains(HOOK_MARKER),
            "uninstall recognises the fence by its marker; a body without it can never be removed"
        );
        assert!(HOOK_BODY.contains(FORCE_ENV), "the --force escape");
        assert!(HOOK_BODY.contains("--no-verify"), "git's own escape");
        assert!(
            HOOK_BODY.starts_with("#!/bin/sh\n"),
            "git execs the hook: the shebang is what makes chmod+x mean anything"
        );
    }

    #[test]
    fn the_hook_holds_no_markdown_semantics() {
        // The ratified bound (silver §5), asserted rather than promised. A hook
        // that grew a selector, a rev or a colour word would be a second gate.
        for forbidden in [
            "meridian-lock",
            "^inputs",
            "fingerprint",
            "blake3",
            "grey(",
            "red(",
            "frontmatter",
        ] {
            assert!(
                !HOOK_BODY.contains(forbidden),
                "the fence is an adapter over the engine; {forbidden:?} is engine semantics"
            );
        }
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
            Unfenceable::ForeignHook {
                path: PathBuf::from("/x"),
                first_line: String::new(),
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
