# MERIDIAN-HOOK.md — the meridian commit fence

Emitted verbatim by `mrd skill hook`. **This document is the contract.** It
declares what to place, where, when to refuse to place it, and how to verify
what was placed. The engine emits it and does nothing else: no file is written,
no git directory is read, no repository is touched. **You** place the file.

The engine has no installer, by design. A verb that wrote into `$GIT_DIR` had to
carry an uninstaller, a lock, a downgrade guard and an ownership check — four
planes of imperative machinery encoding rules that are, in the end, six
paragraphs an agent can read. These are those paragraphs.

## What the fence is

The one door the engine cannot see is the **out-of-band write** — a human
editing in Obsidian, a `sed` over a page, anything that changes governed bytes
without going through the write path. `mrd check` catches it at read time, which
is after the fact. The fence catches it at commit time, which is before.

The fence holds **zero markdown semantics**. It parses no selector, reads no
rev, spells no colour word. It runs `mrd check --commit-gate` and rejects on its
exit; refusal's legal home stays engine-side.

## Place it at three doors

`pre-commit` is not the only hook git dispatches for a commit it builds from a
prepared index. The set is:

- `pre-commit`
- `pre-merge-commit`
- `pre-applypatch`

Each is veto-capable and each fires with the index already holding what would be
committed, so `mrd check --commit-gate` is the correct question at all three and
**one body serves them all**. A set of one lets `git merge` and `git am` land
commits past a fence that printed nothing.

Place it per **`$GIT_COMMON_DIR`**, not per worktree:

1. Ask git where the hooks live — `git rev-parse --git-common-dir` — and append
   `hooks/`. This is the common dir on purpose: N linked worktrees are N
   meridian workspaces sharing ONE `hooks/` directory. Per `--git-dir` writes N
   files of which git runs one; per worktree top-level overwrites the same file
   N times.
2. Write the body below, **byte-for-byte and identically**, to all three names
   in that directory. The body reads its worktree from git's working directory
   at commit time and bakes no path in — that is what makes one file correct for
   N workspaces.
3. `chmod +x` each one. A hook git cannot execute is a hook git skips silently,
   so the chmod is part of placing it, not a decoration on it.

To remove the fence, delete those three files. Nothing else was written.

## Refuse to place it when

Each of these is a state where placing the file does damage or nothing, and in
both cases the operator must be told rather than left with a fence that is not
there. **Name what you observed; do not guess at a cause.**

| Observed | Why refuse |
|---|---|
| the root is a **submodule** (`git rev-parse --show-superproject-working-tree` answers) | its hooks live at `<superproject>/.git/modules/<name>/hooks`; write into `$GIT_COMMON_DIR/hooks` here and git looks elsewhere |
| **`core.hooksPath` is set** (`git config --get core.hooksPath`) | git runs hooks from there and never from this repository's own directory, so anything written here is a silent no-op — and if that path already carries a `pre-commit`, writing there would write into **another checkout's** hook directory |
| a door already carries a **hook this engine did not write** (its bytes lack `mrd-hook-fence`) | never silently overwrite another tool's artifact — name the file, quote its first line, stop |
| a door carries a fence declaring a **HIGHER generation** than the `mrd` you are placing for | the `mrd` you are holding is behind the fence. Placing would replace it with an older one and silently restore whatever defect the newer one closes. Put the current engine first on `PATH` instead — the remedy inverts here, and it is the one state where "re-place the fence" is the wrong advice |
| a door carries the `mrd-hook-fence` marker but **no readable generation** | an undeclarable generation is not a known-old one; refuse rather than overwrite on a guess |
| the **meridian workspace root is not the worktree top-level** (`git rev-parse --show-toplevel`) | "this workspace" and "this repository" name different directories; the fence is placed per common dir and runs from the committing worktree, so a workspace nested below the top-level would be fenced by a commit it does not cover. Place from the top-level instead |
| the root is **not a git repository at all** | a meridian workspace does not have to be one — `MERIDIAN_WORKSPACE` anchors a non-git tree and `cwd-default` accepts one. This is a supported workspace state with simply nowhere to put a hook, not a fault |

## The body

Exactly one fenced block follows, and it is the file.

```sh
#!/bin/sh
# mrd-hook-fence 4 — the meridian commit fence.
#
# Emitted by `mrd skill hook`; placed by the reader of that document. Removed by
# deleting this file. This file is an ADAPTER over the engine: it holds ZERO
# markdown semantics and decides nothing a verb could decide. The verdict below
# is `mrd check`'s exit.
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
		'  NOTHING WAS CHECKED: no out-of-band write, no stranded anchor, no unaccounted interval was looked for.' \
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
# so the worktree is read from here and never baked in at placement time.
if ! command -v mrd >/dev/null 2>&1; then
	printf '%s\n' \
		'meridian fence: refusing — `mrd` is not on PATH, so this commit could not be checked.' \
		'  the fence fails CLOSED: a commit nobody could vouch for is not a verified one.' \
		"  escape:  MRD_HOOK_FORCE=1 git commit ...   (or: git commit --no-verify)" \
		'  remove:  rm "$0"   (this file, and its sibling doors)' >&2
	exit 1
fi

# --commit-gate is the whole point of running here, on BOTH axes.
#
# The INTERVAL: it implies --staged, because git commits the INDEX. A check that
# reads the worktree answers a true question about the wrong bytes — stage a
# forgery, restore the worktree, and an honest `mrd check` passes it into history.
#
# The QUESTION: it asks "were these bytes produced by a governed write?", which is
# per-commit. The unscoped verb asks instead whether the whole write history is
# true, and past the first chain break that answer is permanently 1 — a fence
# whose verdict no longer varies with what is staged carries zero information
# about it, and the only remaining ways past are the two escapes below.
mrd check --commit-gate
mrd_status=$?

# Exit 2 is the verb's BAD-INVOCATION leg, and the only invocation this file makes
# is `check --commit-gate` — so the commonest way to see a 2 here is an `mrd` on
# PATH that is OLDER than this fence and does not carry the flag. That happens
# during any cutover: a new engine emits the document while the old one is still
# on PATH. It FAILS CLOSED, because the alternative is falling back to a check
# that reads the worktree and cannot speak about what is being committed. The
# message names the OBSERVED state and the two commands that decide the cause; it
# does not accuse, because an unreadable workspace exits 2 as well.
if [ "$mrd_status" -eq 2 ]; then
	printf '%s\n' \
		"meridian fence: refusing — \`mrd check --commit-gate\` exited 2 (a bad invocation, or a workspace it could not read)." \
		"  the fence fails CLOSED: a commit nobody could vouch for is not a verified one." \
		"  if the \`mrd\` on PATH is OLDER than this fence it does not carry --commit-gate. what decides it:" \
		"    command -v mrd  &&  mrd check --commit-gate   (does this engine know the flag?)" \
		"    mrd check                                     (its fence: line reports this door's generation)" \
		"  a version skew is fixed by putting the current engine first on PATH, or re-placing this file" \
		"  from \`mrd skill hook\` run through THAT engine." \
		"  escape:  MRD_HOOK_FORCE=1 git commit ...   (or: git commit --no-verify)" \
		'  remove:  rm "$0"   (this file, and its sibling doors)' >&2
	exit 1
fi
if [ "$mrd_status" -ne 0 ]; then
	printf '%s\n' \
		"meridian fence: refusing this commit — \`mrd check --commit-gate\` exited ${mrd_status}; its lines above say why." \
		"  escape:  MRD_HOOK_FORCE=1 git commit ...   (or: git commit --no-verify)" \
		'  remove:  rm "$0"   (this file, and its sibling doors)' >&2
	exit 1
fi
exit 0
```

## The generation line is a datum, not a comment

`# mrd-hook-fence <n>` on the body's second line is parsed by the engine and
compared against the generation that engine writes. The relation is
**three-valued on purpose** — the placed file can be older than, equal to, or
NEWER than the engine asking — because a byte-equality test collapses *older*
and *newer* into one `false` and then asserts a direction it never measured.

| Relation | What it means | What to do |
|---|---|---|
| equal | the placed fence is the one this engine emits | nothing |
| placed **older** | the fence predates this engine and misses whatever the newer one closes | re-place from this document |
| placed **NEWER** | the `mrd` you are asking is behind the fence | put the current engine first on `PATH`; **do not re-place** — see the refusal table |
| marker present, generation unreadable | the file cannot say what it is | refuse; an undeclarable generation is not a known-old one |

Whatever changes the body's behaviour bumps that number. An unbumped body change
makes every already-placed fence report as current while doing something this
engine does not do.

## The escapes

Both are named in every refusal the fence prints.

- `MRD_HOOK_FORCE=1 git commit …` — the ratified `--force`, in the spelling a
  hook that receives no arguments can carry.
- `git commit --no-verify` — git's own, which skips the file entirely and needs
  nothing from it.

`MRD_HOOK_FORCE` is a **two-sided grammar with a loud third leg**. The value is
parsed, never merely tested for non-emptiness: `[ -n … ]` opens the gate on every
spelling of *"do not force"*, because it reads whether a value was typed and
never what it says.

| `MRD_HOOK_FORCE` (trimmed, any case) | Verdict |
|---|---|
| `1` `true` `yes` `on` | **bypass** — printed on stderr, naming the value and stating that nothing was checked |
| `0` `false` `no` `off`, empty, whitespace-only, unset | **fence normally**, silently |
| anything else | **refuse the commit**, exit 1, naming the value it could not parse |

The bypass is rendered and the ordinary pass is not. That asymmetry is the
notice: a forced commit that printed nothing was indistinguishable afterwards
from one that passed the fence honestly.

## What the fence does NOT cover

Three commit-creating paths stay open, and they are **declared here rather than
papered over**: `git cherry-pick`, `git revert` and `git rebase` replay dispatch
no veto-capable hook that can read the index. Measured: `pre-commit` never fires
on them at all, and the one hook that does fire and can veto —
`prepare-commit-msg` — is overruled by a rebase. A gate that refuses and is then
ignored teaches an operator to disbelieve it.

So the fence's guarantee is: **no out-of-band write reaches history through
`commit`, `merge`, or `am`.** It is NOT: no drift reaches history. Across the
replay paths the engine's read-time `mrd check` is the only guarantee.

**Coverage is per-checkout and opt-in, permanently.** `$GIT_DIR/hooks` is never
a tracked path, so no clone, fetch or pull can transport the fence, and a fresh
clone being unfenced is a supported state rather than a fault. The automatic
route — a global `init.templateDir` — is refused on its collateral: it fences
every unrelated repository the operator ever clones or inits, which abolishes
the opt-in premise the body's no-membership-test design rests on. The defect to
close is the SILENCE, not the absence — hence the next section.

## Verify what you placed

`mrd check` reports the local checkout's fence coverage on its own line, beside
the verdict and never part of it:

- `fence:` — one word for the whole set (`installed`, `installed-partial`,
  `installed-superseded`, `installed-ahead`, `installed-unversioned`,
  `foreign-hook`, `absent`, or the reason word for a root that cannot carry a
  fence at all), the count of doors carrying the marker, and a teaching.
- `fence doors:` — each door by name with its own word, so a disagreement can be
  located rather than merely reported.

Under `--json` the same reading is the `fence` object, carrying `doors[]`,
`fenced_doors`, `total_doors`, `engine_version`, and `gates_the_exit: false`.

**That line never moves `mrd check`'s exit code.** Fence coverage is a property
of a local checkout, not of the corpus; colouring the verdict on it would make
governance unreachable in every fresh clone.

Placement succeeded when all three doors read `installed` at the generation this
document declares.
