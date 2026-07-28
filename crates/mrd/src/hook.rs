//! The git pre-commit fence — the boundary fence, installed per
//! `$GIT_COMMON_DIR` (U15, D11/D12).
//!
//! `mrd hook install` writes a hook that calls `mrd check --staged` and rejects
//! on its exit. **Zero markdown semantics live in the hook** — it is an adapter
//! over the engine, the same law that keeps them out of ccc-statusd
//! (`2026-07-24-pin-content-anchoring-modes.md` §5, silver). Refusal's legal home
//! stays engine-side; the hook fences the one door the engine cannot see, the
//! out-of-band write (a human in Obsidian, a bash edit).
//!
//! # THE DOOR IS A CLASS, AND THE INSTALL SET IS THE CLAIM ABOUT IT (row 20)
//! `pre-commit` is not the only hook git dispatches for a commit it builds from a
//! prepared index. The set is [`FENCED_HOOKS`] — `pre-commit`, `pre-merge-commit`
//! and `pre-applypatch` — and **one body serves all three**: each fires with the
//! index already holding what would be committed, so `mrd check --staged` is the
//! correct question at every one, unchanged. An install set of one left
//! `git merge` and `git am` landing commits past a fence that printed nothing.
//!
//! **Three commit-creating paths stay open, and are DECLARED rather than papered
//! over**: `git cherry-pick`, `git revert` and `git rebase` replay dispatch no
//! veto-capable hook that can read the index. Measured: `pre-commit` never fires
//! on them at all; the one hook that does fire and can veto — `prepare-commit-msg`
//! — is overruled by a rebase, and a gate that refuses and is then ignored teaches
//! an operator to disbelieve it. **So the fence's guarantee is: no out-of-band
//! write reaches history through `commit`, `merge`, or `am`. It is NOT: no drift
//! reaches history.** Across the replay paths the engine's read-time `mrd check`
//! is the only guarantee, and `mrd check` now says whether it is standing in a
//! fenced checkout (row 21) so that absence is never silent.
//!
//! # THE FENCE COVERAGE IS PER-CHECKOUT AND OPT-IN, PERMANENTLY (row 21)
//! `$GIT_DIR/hooks` is never a tracked path, so no clone, fetch or pull can
//! transport the fence. That is git's design and the fix cannot be "make the
//! fence clonable". The automatic route — a global `init.templateDir` — was
//! measured working and is **refused on its collateral**: it fences every
//! unrelated repository the operator ever clones or inits, which abolishes the
//! opt-in premise [`HOOK_BODY`]'s no-membership-test clause rests on and
//! displaces in time the `--all` [`crate::hook_cmd`] rules out. **The defect to
//! close was the SILENCE, not the absence** — hence the fence line in `mrd check`.
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
//! declares a generation on its second line and the engine READS it: `mrd hook
//! install` is idempotent and refreshes, but nothing can prompt an operator to run
//! it if the status face calls a superseded fence "installed".
//!
//! # THE VERSION LINE IS A DATUM, NOT A COMMENT (rows 23 + 26)
//! `# mrd-hook-fence <n>` is parsed by [`parse_fence_version`] and compared
//! against this engine's [`FENCE_VERSION`]. The relation it yields is
//! [`Currency`], and it is **three-valued on purpose**: the installed bytes can be
//! older than, equal to, or NEWER than the engine asking. A byte-equality test
//! collapses *older* and *newer* into one `false`, and the teaching then asserts a
//! direction the comparison never measured — the guessed cause [`Unfenceable`]'s
//! own doc-comment forbids one type away in this file.
//!
//! **`Ahead` is the whole skew, and it is the state where the remedy inverts.**
//! An old `mrd` first on PATH answering about a fence a new engine wrote must not
//! be told to run `mrd hook install`: that resolves the OLD engine, which writes
//! the OLD fence, silently restoring the worktree-reading false green F1 removed.
//! So [`install`] REFUSES a downgrade ([`Unfenceable::FenceAhead`]) — the
//! [`Unfenceable::ForeignHook`] law on a second axis, refusing to overwrite a file
//! a later generation of this same engine wrote. The escape is the ratified one,
//! so a deliberate rollback stays possible and is never silent.
//!
//! # AND THE SKEW RUNS BOTH WAYS — measured by the re-verifier's harness
//! A fence written by a NEW engine can be run against an OLD `mrd` on `PATH`,
//! because the hook resolves the engine at commit time and never bakes one in
//! (that is what makes one file correct for N worktrees). The old engine then
//! answers `unknown flag: --staged`, **exit 2, and the fence refuses EVERY
//! commit** — measured: `mrd: unknown flag: --staged` on the deployed
//! `980008813ff69586…` under a hook this engine installed, turning a guard into a
//! blanket refusal. **This is the ordinary state of a cutover**, so the body
//! handles exit 2 with a teaching refusal that names the skew and the two commands
//! that decide it. It still fails CLOSED: falling back to `mrd check` would
//! restore exactly the false green this unit removed.
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

/// The environment escape, in its hook spelling. None of the [`FENCED_HOOKS`]
/// receives arguments, so the ratified `--force` escape reaches them through the
/// environment; `git commit --no-verify` is git's own, and skips the hook
/// entirely without this file's help. Both are named in every refusal.
pub const FORCE_ENV: &str = "MRD_HOOK_FORCE";

/// The generation this engine writes, declared on [`HOOK_BODY`]'s second line and
/// read back by [`parse_fence_version`].
///
/// **Bump this whenever `HOOK_BODY` changes behaviour**, because it is the datum
/// every already-fenced root is judged by: an unbumped body change makes a stale
/// fence report `installed` while doing something the current engine does not do.
pub const FENCE_VERSION: u32 = 3;

/// Every door git offers a veto on for a commit it builds from a prepared index.
///
/// **Not a list of names — a claim about coverage.** Each of the three is a
/// pre-hook, is veto-capable, and fires with the index already holding what would
/// be committed, so `mrd check --staged` is the correct question at all three and
/// **one body serves them all** (asserted by
/// [`tests::one_body_serves_every_door`]). The commit-creating paths that
/// dispatch none of them are declared in this module's doc-comment rather than
/// papered over with a hook a rebase would overrule.
pub const FENCED_HOOKS: [&str; 3] = ["pre-commit", "pre-merge-commit", "pre-applypatch"];

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
/// It deliberately tests for NO membership artifact before running the verb —
/// not the root's `MERIDIAN.md` self-declaration, not any other file. Two
/// reasons, and the retirement of the file this clause used to name changed
/// neither. Structurally: membership is the ladder's answer, and
/// `workspace::resolve` anchors a git root as a workspace with no file at all —
/// so an existence gate here would make the fence a silent no-op on exactly the
/// roots it is installed for (installation is keyed to `$GIT_COMMON_DIR`, i.e.
/// to being a git repository, which is the same fact the ladder reads). By
/// charter: an existence test is a membership rule, and this file holds none —
/// the verdict is `mrd check`'s exit, and refusal's legal home is engine-side.
const HOOK_BODY: &str = r#"#!/bin/sh
# mrd-hook-fence 3 — the meridian pre-commit fence.
#
# Installed by `mrd hook install`; removed by `mrd hook uninstall`.
# This file is an ADAPTER over the engine: it holds ZERO markdown semantics and
# decides nothing a verb could decide. The verdict below is `mrd check`'s exit.
#
# ONE body, THREE doors — pre-commit, pre-merge-commit, pre-applypatch. Each is a
# hook git dispatches for a commit it builds from a prepared index, so the
# question below is the same at all three. None of them takes an argument, which
# is what lets the force value be word-split below with no positional to protect.
#
# -f (no pathname expansion) is load-bearing, not hygiene: the force value is
# word-split, and a value of `*` must stay one unreadable word rather than
# becoming a list of the files in this worktree.
set -uf

# THE FORCE VALUE IS PARSED, never merely tested for non-emptiness. `[ -n ... ]`
# opens the gate on `0`, `false`, `no` and `off` — every spelling an operator
# means as "do NOT force" — because it reads whether a value was typed and never
# what it says.
#
# `set --` word-splits on IFS, which trims leading and trailing whitespace in the
# same step: `" "` is empty intent and must fence, not force.
set -- ${MRD_HOOK_FORCE:-}
mrd_force="$*"

# Three legs, and the third leg is the point: an unrecognised value REFUSES. The
# fence fails CLOSED, and a value nobody can read is not a decision — minting one
# from it would be the guess this file exists not to make.
case "$mrd_force" in
1 | [Tt][Rr][Uu][Ee] | [Yy][Ee][Ss] | [Oo][Nn])
	# RENDERED, never silent. A forced commit that printed nothing was
	# indistinguishable afterwards from one that passed the fence honestly.
	printf '%s\n' \
		"meridian fence: BYPASSED — MRD_HOOK_FORCE=${MRD_HOOK_FORCE} forced this commit past the fence." \
		'  NOTHING WAS CHECKED: no out-of-band write, no stranded anchor, no chain break was looked for.' \
		'  this commit carries no fence verdict.' >&2
	exit 0
	;;
'' | 0 | [Ff][Aa][Ll][Ss][Ee] | [Nn][Oo] | [Oo][Ff][Ff])
	: # Not a force. Fall through to the fence below.
	;;
*)
	printf '%s\n' \
		"meridian fence: refusing — MRD_HOOK_FORCE is set to \`${MRD_HOOK_FORCE}\`, which this fence does not parse." \
		'  the fence fails CLOSED: an unreadable escape is not permission, and this file will not guess at one.' \
		'  force:      MRD_HOOK_FORCE=1   (also true, yes, on — any case)' \
		'  do not:     MRD_HOOK_FORCE=0   (also false, no, off, empty, unset — the fence runs)' \
		"  git's own:  git commit --no-verify   (skips this file entirely)" >&2
	exit 1
	;;
esac

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

# Exit 2 is the verb's BAD-INVOCATION leg, and the only invocation this file makes
# is `check --staged` — so the commonest way to see a 2 here is an `mrd` on PATH
# that is OLDER than this fence and does not carry the flag. That happens during
# any cutover: a new engine installs the hook while the old one is still on PATH.
# It FAILS CLOSED, because the alternative is falling back to a check that reads
# the worktree and cannot speak about what is being committed. The message names
# the OBSERVED state and the two commands that decide the cause; it does not
# accuse, because an unreadable workspace exits 2 as well.
if [ "$mrd_status" -eq 2 ]; then
	printf '%s\n' \
		"meridian fence: refusing — \`mrd check --staged\` exited 2 (a bad invocation, or a workspace it could not read)." \
		"  the fence fails CLOSED: a commit nobody could vouch for is not a verified one." \
		"  if the \`mrd\` on PATH is OLDER than this fence it does not carry --staged. what decides it:" \
		"    command -v mrd  &&  mrd check --staged        (does this engine know the flag?)" \
		"    mrd hook status                                (is this fence the one this engine writes?)" \
		"  a version skew is fixed by putting the current engine first on PATH, or \`mrd hook install\`." \
		"  escape:  MRD_HOOK_FORCE=1 git commit ...   (or: git commit --no-verify)" \
		'  remove:  mrd hook uninstall' >&2
	exit 1
fi
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
// the version line, read
// ---------------------------------------------------------------------------

/// Read the generation an installed fence declares for itself.
///
/// **A report of the FILE's own declaration, never of the asking engine's
/// expectation.** `None` when the line is absent or its number unparseable — the
/// engine's own [`FENCE_VERSION`] is never substituted, because a fence that
/// cannot say what it is has not said it is current.
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
// the force grammar — ONE law, two spellings
// ---------------------------------------------------------------------------

/// What [`FORCE_ENV`] says, parsed.
///
/// **The value is read, never merely counted.** `[ -n ... ]` opened the gate on
/// every spelling of *"do not force"*, because non-emptiness is a fact about
/// whether a value was typed rather than about what it says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Force {
    /// `1` `true` `yes` `on`, any case, whitespace-trimmed.
    Yes,
    /// `0` `false` `no` `off`, empty, whitespace-only, or unset.
    No,
    /// **Anything else — which is not permission.** Carried verbatim so a
    /// refusal can name what it could not read instead of guessing past it.
    Unparseable(String),
}

/// The spellings that force, and the spellings that do not. These two lists are
/// the same law as [`HOOK_BODY`]'s `case`; the test module holds them to it.
const FORCE_YES: [&str; 4] = ["1", "true", "yes", "on"];
const FORCE_NO: [&str; 4] = ["0", "false", "no", "off"];

/// Parse [`FORCE_ENV`]'s value. Trim first — `" "` is empty intent — then fold
/// case, then match. An unrecognised value is [`Force::Unparseable`] and the
/// caller fails closed on it.
#[must_use]
pub fn parse_force(raw: Option<&str>) -> Force {
    let trimmed = raw.unwrap_or("").trim();
    if trimmed.is_empty() {
        return Force::No;
    }
    let folded = trimmed.to_ascii_lowercase();
    if FORCE_YES.contains(&folded.as_str()) {
        Force::Yes
    } else if FORCE_NO.contains(&folded.as_str()) {
        Force::No
    } else {
        Force::Unparseable(trimmed.to_owned())
    }
}

/// [`parse_force`] over the process environment — the escape as the CLI face sees
/// it, so `install`'s downgrade refusal honours the same escape the fence does.
#[must_use]
pub fn force_from_env() -> Force {
    parse_force(std::env::var(FORCE_ENV).ok().as_deref())
}

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
    /// A hook already exists at one of the [`FENCED_HOOKS`] paths and this engine
    /// did not write it. **Refused by default, naming the file** — never a silent
    /// overwrite of another tool's artifact.
    ForeignHook {
        /// The existing file, named so the operator can read it.
        path: PathBuf,
        /// Its first non-shebang line, quoted so the refusal says WHOSE it is
        /// without this engine guessing at a tool name.
        first_line: String,
    },
    /// **The installed fence was written by a NEWER engine than this one**, so
    /// installing would replace it with an older fence. The `ForeignHook` law on
    /// a second axis: the engine already refuses to overwrite a file another tool
    /// wrote, and this refuses to overwrite one a later generation of itself
    /// wrote.
    ///
    /// This is the state an operator reaches by following a skew refusal's
    /// advice with the wrong `mrd` on `PATH` — the remedy inverts here, and a
    /// guard that wrote anyway would silently restore the worktree-reading false
    /// green F1 removed.
    FenceAhead {
        /// The door carrying the newer fence.
        path: PathBuf,
        /// The generation that file declares.
        installed: u32,
        /// The generation this engine writes.
        engine: u32,
    },
    /// An installed fence carries the marker but declares no readable generation,
    /// so **it cannot be shown NOT to be newer than this engine.** The plane's
    /// standing rule is that an unreadable file is not an absent one; refuse,
    /// name it, do not guess.
    FenceUnversioned {
        /// The door whose generation is undeclarable.
        path: PathBuf,
        /// The generation this engine writes.
        engine: u32,
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
            Unfenceable::FenceAhead { .. } => "fence-ahead",
            Unfenceable::FenceUnversioned { .. } => "fence-unversioned",
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
            Unfenceable::FenceAhead {
                path,
                installed,
                engine,
            } => format!(
                "{} was written by a NEWER engine than this one (fence {installed}, this engine \
                 writes {engine}). The `mrd` first on PATH is behind the fence. Put the current \
                 engine first on PATH — do NOT install with this one, which would replace the \
                 fence with an older one and restore the staged-forgery false green the newer \
                 fence removes. A deliberate rollback is {FORCE_ENV}=1.",
                path.display()
            ),
            Unfenceable::FenceUnversioned { path, engine } => format!(
                "{} carries this engine's marker but declares no readable `# {HOOK_MARKER} <n>` \
                 line, so it cannot be shown to be older than the fence this engine writes \
                 ({engine}). An undeclarable generation is not a known-old one — refusing rather \
                 than overwriting on a guess. A deliberate overwrite is {FORCE_ENV}=1.",
                path.display()
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
    /// Every file this install writes, one per [`FENCED_HOOKS`] entry and in that
    /// order. **A set, not a scalar** — an install set of one was a claim about
    /// coverage that `git merge` and `git am` walked straight through.
    pub hook_paths: Vec<PathBuf>,
}

impl Fenceable {
    /// The doors and their names, paired — the shape every per-path report and
    /// refusal is built from.
    fn doors(&self) -> impl Iterator<Item = (&'static str, &PathBuf)> {
        FENCED_HOOKS.iter().copied().zip(&self.hook_paths)
    }
}

/// What an install did. **A state change, not an exit code** (R40): these are
/// different facts about the disk and are reported apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Installed {
    /// No door carried a fence; every one does now.
    Fresh,
    /// Every door already carried a fence this engine wrote; the bytes were
    /// refreshed. Install is idempotent, and says which it was.
    AlreadyInstalled,
    /// **Some doors carried a fence and some did not** — the partially-fenced
    /// root an older install set left behind. Named apart because reporting it as
    /// `already-installed` would hide the migration that just happened.
    Completed {
        /// How many doors were unfenced before this install.
        added: usize,
    },
}

/// What an uninstall did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Removed {
    /// At least one fence was there and every one is gone.
    Removed {
        /// How many doors carried a fence that this uninstall removed.
        doors: usize,
    },
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
/// # Panics
/// Never — the door set and the path set are built from one array.
pub fn install(workspace: &Path, force: &Force) -> Result<(Fenceable, Installed), Unfenceable> {
    let repo = git::Repo::at(workspace);
    let decision = locked_decision(&repo, workspace)?;
    let _lock = decision.lock;
    let fenceable = decision.fenceable;

    // Read EVERY door before writing ANY. A root that refuses at the third door
    // must not come away with two fences written on the strength of a decision
    // the third one reverses.
    let mut absent = 0usize;
    for (_, path) in fenceable.doors() {
        match read_hook(path) {
            HookHere::None => absent += 1,
            HookHere::Foreign { first_line } => {
                return Err(Unfenceable::ForeignHook {
                    path: path.clone(),
                    first_line,
                });
            }
            HookHere::Ours { installed_version } => {
                // The downgrade guard. Only `Force::Yes` is permission — an
                // unparseable escape is not one, which is the same fail-closed
                // law the body's third leg runs.
                if *force == Force::Yes {
                    continue;
                }
                match currency(installed_version) {
                    Currency::Current | Currency::Superseded { .. } => {}
                    Currency::Ahead { installed } => {
                        return Err(Unfenceable::FenceAhead {
                            path: path.clone(),
                            installed,
                            engine: FENCE_VERSION,
                        });
                    }
                    Currency::Unversioned => {
                        return Err(Unfenceable::FenceUnversioned {
                            path: path.clone(),
                            engine: FENCE_VERSION,
                        });
                    }
                }
            }
        }
    }

    let state = match absent {
        0 => Installed::AlreadyInstalled,
        n if n == fenceable.hook_paths.len() => Installed::Fresh,
        added => Installed::Completed { added },
    };

    write_hooks(&fenceable, workspace)?;
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

    // Decide over the whole set before removing anything: a foreign hook at the
    // second door refuses the uninstall, and a fence already deleted from the
    // first would be a partial teardown nobody asked for.
    let mut ours = Vec::new();
    for (_, path) in fenceable.doors() {
        match read_hook(path) {
            HookHere::None => {}
            HookHere::Foreign { first_line } => {
                return Err(Unfenceable::ForeignHook {
                    path: path.clone(),
                    first_line,
                });
            }
            HookHere::Ours { .. } => ours.push(path.clone()),
        }
    }
    if ours.is_empty() {
        return Ok((fenceable, Removed::Absent));
    }
    for path in &ours {
        fs::remove_file(path).map_err(|e| Unfenceable::CannotAsk {
            root: workspace.to_path_buf(),
            detail: format!("cannot remove {} ({e})", path.display()),
        })?;
    }
    let doors = ours.len();
    Ok((fenceable, Removed::Removed { doors }))
}

/// Is a fence installed for `workspace`'s repository, and whose is it?
///
/// # Errors
/// [`Unfenceable`] naming the observed state — but note a foreign hook is a
/// STATE here, reported as [`HookHere::Foreign`], not a refusal: reporting is
/// what this verb is for.
pub fn status(workspace: &Path) -> Result<(Fenceable, Coverage), Unfenceable> {
    let fenceable = survey(workspace)?;
    let coverage = coverage(&fenceable);
    Ok((fenceable, coverage))
}

/// Read every door read-only. Writes nothing, takes no lock — the same promise
/// [`survey`] makes, and what lets `mrd check` report the fence state of the
/// checkout it stands in without leaving a lock file as the souvenir.
#[must_use]
pub fn coverage(fenceable: &Fenceable) -> Coverage {
    Coverage {
        doors: fenceable
            .doors()
            .map(|(name, path)| Door {
                name,
                path: path.clone(),
                here: read_hook(path),
            })
            .collect(),
    }
}

/// What sits at one hook path right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookHere {
    /// Nothing is installed.
    None,
    /// A fence this engine wrote (it carries [`HOOK_MARKER`]), carrying the
    /// generation the FILE declares for itself.
    ///
    /// # Why the generation is a reported datum (F1, rows 23 + 26)
    /// The marker says WHOSE the file is; it cannot say WHAT it does. An older
    /// fence runs `mrd check` where the current one runs `mrd check --staged`, so
    /// it reads the worktree and passes a staged forgery — **while
    /// `mrd hook status` reports it as installed.** A guard whose report cannot
    /// distinguish "fenced" from "fenced by a version that misses the defect this
    /// one closes" is a green light with no lamp behind it.
    ///
    /// This is the file's own declaration and nothing else: `None` when the
    /// version line is absent or unparseable, never the asking engine's number.
    Ours {
        /// The generation parsed out of the installed bytes.
        installed_version: Option<u32>,
    },
    /// A file this engine did not write.
    Foreign {
        /// Its first non-shebang, non-blank line, quoted verbatim.
        first_line: String,
    },
}

/// The observed relation between an installed fence's declared generation and
/// this engine's [`FENCE_VERSION`].
///
/// **Three-valued, because the file can be older than, equal to, or NEWER than
/// the engine asking.** A byte-equality test collapses *older* and *newer* into
/// one `false`, and a teaching built on it then asserts a direction the
/// comparison never measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Currency {
    /// The installed generation is the one this engine writes.
    Current,
    /// Older than this engine writes: a re-install with THIS engine refreshes it.
    Superseded {
        /// The generation the file declares.
        installed: u32,
    },
    /// **Newer than this engine.** The remedy inverts: the `mrd` answering is the
    /// one that is behind, and installing with it would downgrade the fence.
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

/// One door of the install set, and what is standing in it.
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

    /// The generation THIS file declares — `None` when nothing is installed here
    /// or the line is undeclarable. Never the asking engine's number.
    #[must_use]
    pub fn version(&self) -> Option<u32> {
        match &self.here {
            HookHere::Ours { installed_version } => *installed_version,
            _ => None,
        }
    }
}

/// What the whole install set looks like on disk.
///
/// **A set's state is not any one door's state.** "Two of three doors carry a
/// current fence" is a distinct fact from "installed", and R40 requires it be
/// reported apart — it is exactly the state every root fenced by the previous
/// install set is in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Coverage {
    /// One entry per [`FENCED_HOOKS`] name, in that order.
    pub doors: Vec<Door>,
}

impl Coverage {
    /// The one word for the whole set.
    ///
    /// **Precedence is the design.** A foreign file is named first because it is
    /// the state nothing else explains. The version relations come next, and
    /// `installed-ahead` before the rest, because it is the only state whose
    /// remedy is the OPPOSITE of every other one's — every other word means "run
    /// `mrd hook install`", and that word means "do not".
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
                "{} is not this engine's ({first_line:?}); install refuses rather than \
                 overwriting it",
                door.name
            ));
        }
        if let Some((door, Currency::Ahead { installed })) =
            self.first_currency_door(|c| matches!(c, Currency::Ahead { .. }))
        {
            return Some(format!(
                "{} was written by a NEWER engine than the one answering (fence {installed}, this \
                 engine {FENCE_VERSION}); the `mrd` first on PATH is behind the fence, so put the \
                 current engine first on PATH — do NOT run `mrd hook install` with this one, \
                 which would replace the fence with an older one",
                door.name
            ));
        }
        if let Some((door, _)) = self.first_currency_door(|c| c == Currency::Unversioned) {
            return Some(format!(
                "{} carries the marker but declares no readable generation, so its currency \
                 cannot be judged; `mrd hook install` refuses it rather than guessing",
                door.name
            ));
        }
        if let Some((door, Currency::Superseded { installed })) =
            self.first_currency_door(|c| matches!(c, Currency::Superseded { .. }))
        {
            return Some(format!(
                "{} carries fence {installed} and this engine writes {FENCE_VERSION}; \
                 `mrd hook install` refreshes it (idempotent)",
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
                 index, so they are bypasses until `mrd hook install` covers them",
                unfenced.join(", ")
            ));
        }
        None
    }

    /// The generation the installed fences declare — `None` when nothing is
    /// installed, or when the doors disagree, which is itself not one number.
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

    /// How many doors carry a fence this engine wrote.
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

    let hooks = common_dir.join("hooks");
    Ok(Fenceable {
        workspace: workspace.to_path_buf(),
        hook_paths: FENCED_HOOKS.iter().map(|name| hooks.join(name)).collect(),
        common_dir,
    })
}

/// Read what is at one hook path, without deciding anything about it.
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

/// Write the fence at every door and make each executable. A hook git cannot
/// execute is a hook git skips — the chmod is the install, not a decoration on
/// it.
///
/// **The same bytes at every path**, which is what makes one version line true
/// for the whole set: a per-door body would be three fences to keep in step and
/// three generations to reconcile.
fn write_hooks(fenceable: &Fenceable, workspace: &Path) -> Result<(), Unfenceable> {
    let cannot = |detail: String| Unfenceable::CannotAsk {
        root: workspace.to_path_buf(),
        detail,
    };
    let hooks_dir = fenceable.common_dir.join("hooks");
    fs::create_dir_all(&hooks_dir)
        .map_err(|e| cannot(format!("cannot create {} ({e})", hooks_dir.display())))?;
    for path in &fenceable.hook_paths {
        fs::write(path, HOOK_BODY)
            .map_err(|e| cannot(format!("cannot write {} ({e})", path.display())))?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))
            .map_err(|e| cannot(format!("cannot make {} executable ({e})", path.display())))?;
    }
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

    // ── the version line is a datum (rows 23 + 26) ───────────────────────────

    #[test]
    fn the_fence_declares_the_generation_this_engine_writes() {
        assert_eq!(
            parse_fence_version(HOOK_BODY),
            Some(FENCE_VERSION),
            "the body's own `# {HOOK_MARKER} <n>` line and FENCE_VERSION are one fact; \
             a body change without a bump makes every stale fence report `installed`"
        );
    }

    #[test]
    fn an_undeclarable_generation_is_never_resolved_into_the_asking_engines() {
        // The refusal arm for the parser: a marker-bearing fence whose generation
        // no `u32` parses must report None, NOT fall back to FENCE_VERSION. The
        // acceptance arm above passes on a working parser; this one fails on a
        // parser that guesses, and the two disagree only on the guess.
        //
        // The marker and the version share a line, so this fixture keeps the
        // marker and spoils only the number — deleting the line would make the
        // file FOREIGN, which is a different state with a different word.
        let tagged = HOOK_BODY.replace(
            &format!("# {HOOK_MARKER} {FENCE_VERSION}"),
            &format!("# {HOOK_MARKER} next"),
        );
        assert!(
            tagged.contains(HOOK_MARKER) && tagged != HOOK_BODY,
            "the control: the marker survives and the datum actually changed"
        );
        assert_eq!(parse_fence_version(&tagged), None);
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

    // ── the force grammar: ONE law, two spellings (row 22) ───────────────────

    #[test]
    fn the_force_grammar_reads_the_value_and_not_whether_one_was_typed() {
        for yes in ["1", "true", "TRUE", "True", "yes", "on", " true ", "\tON\n"] {
            assert_eq!(parse_force(Some(yes)), Force::Yes, "{yes:?} means force");
        }
        // The arm that fails against the shipped `[ -n ... ]`: every one of these
        // opened the gate, because non-emptiness is a fact about the keystroke
        // and not about the word.
        for no in ["0", "false", "FALSE", "no", "off", "", " ", "   "] {
            assert_eq!(
                parse_force(Some(no)),
                Force::No,
                "{no:?} means do NOT force"
            );
        }
        assert_eq!(parse_force(None), Force::No, "unset fences");
        for bad in ["maybe", "2", "yolo", "t rue"] {
            assert_eq!(
                parse_force(Some(bad)),
                Force::Unparseable(bad.to_owned()),
                "{bad:?} is not a decision and may not be read as one"
            );
        }
    }

    #[test]
    fn the_two_spellings_of_the_force_grammar_are_one_law() {
        // The Rust parser and the shell `case` are the same grammar written
        // twice, and nothing but this arm keeps them in step. The shell spells
        // its case-fold as a character-class glob, so that is what is searched
        // for — `true` would also match the prose above it and prove nothing.
        for word in FORCE_YES.iter().chain(&FORCE_NO) {
            let glob: String = if word.chars().all(|c| c.is_ascii_digit()) {
                (*word).to_owned()
            } else {
                word.chars().fold(String::new(), |mut acc, c| {
                    use std::fmt::Write as _;
                    let _ = write!(acc, "[{}{c}]", c.to_ascii_uppercase());
                    acc
                })
            };
            assert!(
                HOOK_BODY.contains(&glob),
                "{word:?} is in the Rust grammar and not in the fence's `case` (looked for \
                 {glob:?}) — two spellings of one law that disagree is the defect, not the fix"
            );
        }
        assert!(
            !HOOK_BODY.contains(r#"[ -n "${MRD_HOOK_FORCE:-}" ]"#),
            "the non-emptiness test is what the grammar replaces"
        );
        assert!(
            HOOK_BODY.contains("set -uf"),
            "-f keeps a force value of `*` from being pathname-expanded into a file list \
             by the word-split that trims it"
        );
    }

    #[test]
    fn the_force_path_is_rendered_and_the_fenced_path_is_not() {
        // C2's specificity pair, asserted over the bytes: the bypass leg writes
        // to stderr, and the not-a-force leg falls through carrying no printf of
        // its own. A notice that fired on both would be no notice at all.
        let force_leg = HOOK_BODY
            .split("case \"$mrd_force\" in")
            .nth(1)
            .expect("the force case");
        let bypass = force_leg.split(";;").next().expect("the bypass leg");
        assert!(
            bypass.contains("BYPASSED") && bypass.contains(">&2"),
            "a forced commit that printed nothing was indistinguishable from an honest one"
        );
        let not_a_force = force_leg
            .split(";;")
            .nth(1)
            .expect("the fence-normally leg");
        assert!(
            !not_a_force.contains("printf"),
            "the fence-normally leg must stay silent, or the notice says nothing"
        );
        let unparseable = force_leg.split(";;").nth(2).expect("the third leg");
        assert!(
            unparseable.contains("exit 1"),
            "an unrecognised value refuses: the fence fails closed and does not guess"
        );
    }

    // ── the install set is a claim about coverage (row 20) ───────────────────

    #[test]
    fn one_body_serves_every_door() {
        // Each of the three fires with the index already holding what would be
        // committed, so `mrd check --staged` is the correct question at all
        // three and there is nothing to specialise per door.
        assert_eq!(FENCED_HOOKS.len(), 3);
        assert!(FENCED_HOOKS.contains(&"pre-commit"));
        assert!(
            FENCED_HOOKS.contains(&"pre-merge-commit"),
            "git dispatches this one for a merge commit it creates, and an install set \
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

        // REFUSAL, in the same run: the root every previous install left behind
        // carries `pre-commit` alone, and reporting it as `installed` is the
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
            teaching.contains("do NOT run `mrd hook install`"),
            "every other state's remedy is `install`; this one's is the reverse, and a \
             teaching that did not say so sends the operator to downgrade the fence: {teaching}"
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
            Unfenceable::FenceAhead {
                path: PathBuf::from("/x"),
                installed: 9,
                engine: FENCE_VERSION,
            }
            .word(),
            Unfenceable::FenceUnversioned {
                path: PathBuf::from("/x"),
                engine: FENCE_VERSION,
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
