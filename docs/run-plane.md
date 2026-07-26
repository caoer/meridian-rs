# The run plane — `mrd run` (S1)

The run plane executes an addressed task block and turns what it emits into
governed effects. It is **consumer-plane, imperative, local** (plan decision
#1): a client of the engine crates, layered entirely above them — **"the
engine cannot tell run exists."** The daemon, the wire, and the serve path
carry no run-plane type and no run-plane state.

Sources of truth: the ratified plan (session `21-23-meridian-rs-md-run`,
`ccc-compound/plan.md`) and the round-1 verdict. This document states the
surface **as shipped in S1**, including what it deliberately does not
guarantee (§ Accepted gaps).

## One entry per plane

The Starlark kernel has exactly two entry points, one per plane, both
hermetic (decision #3):

| Plane | Entry | Trigger |
|---|---|---|
| change | `on_change(event)` | a governed change event (the effect kernel) |
| run | `def run(ctx)` | `mrd run` addressing a task block |

`RunCtx` is inert data: page, task, args, env, invocation id, root-at-eval.
Identity and time are **caller-supplied** (§9) — the kernel never reads a
clock and never mints an id. Starlark-invokes-bash is a **permanent no**
(decision #17, test-gated): the sandbox exposes no `exec` / `subprocess` /
`os` name. The composition layer IS bash.

## Addressing (§2.1 grammar, no new syntax)

A page declares tasks in frontmatter: `task.<name>: "[[#^block-id]]"` binds a
task name to a same-file fenced code block; `task.<name>.caps` / `.args` /
`.env` carry its capability declaration and input contract. Cross-file refs
are an S1 **non-goal** (decision #11) and refuse with a typed error. Every
addressing fault is distinct and pre-eval: no such task, dangling binding,
ambiguous anchor, not-a-code-block, unknown fence language.

## Capabilities — deny-by-default (verdict ruling 3, decision #15)

An undeclared block is read-only: it can compute, but no effect of its
executes. Caps are namespaced strings, optionally target-scoped
(`md.set_field` / `md.set_field:status`, the latter strictly narrower).
Declared two ways — beside the binding, or by name convention:

```markdown
---
task.fix-drift: "[[#^fix-1]]"
task.fix-drift.caps: md.set_field:status, md.append_section
---
```

```markdown
<!-- <root>/MERIDIAN.md — the root's own self-declaration -->
---
type: meridian-root
version: 1
name: field-notes
run.caps.fix-*: md.set_field, md.append_section
run.caps.fix-note: md.set_field:status      # longest pattern wins
run.timeout_secs: 7
---
```

A present-but-empty `caps` declaration is an EXPLICIT read-only grant, distinct
from no declaration. Precedence for the grant is explicit > convention > none;
conventions **narrow only, never widen**, and every cap that did not survive
intact is reported in `narrowed[]`. The builtin `check-*` / `verify-*` ceiling
is absolute, and those names refuse a bash fence loudly at load. Caps bind at
the executor choke point before any I/O: one violation refuses the whole batch.

### Where the convention table lives (marker-retirement ruling, 2026-07-26)

**The root declares.** The table is read from the root's own `MERIDIAN.md`
self-declaration (`type: meridian-root`) — the artifact the config charter's
*"the root declares, `MERIDIAN.md` binds"* already governs — through
`crates/config`, which owns what a valid declaration is. The retired marker
files are not read and no fallback to them ships.

The grammar is the page grammar reused: flat dotted frontmatter keys with
comma-separated cap lists. Flat is the reader's law, not a preference —
`model`'s frontmatter scanner takes no YAML crate and skips every indented
line, so a nested `run:`/`  caps:` spelling would be unreadable.

Which root answered is never silent (`ConventionSource`):

| Root situation | Conventions | |
|---|---|---|
| declares, with `run.caps.*` | that table | the ceiling is in force |
| declares, none stated | empty | `Declared` — deny-by-default stands |
| holds no `MERIDIAN.md` | empty | `Undeclared` — absent is not broken |
| present, not a valid declaration | **refuses** | an unreadable policy file never becomes *no policy* |
| no root resolved (`CwdDefault`) | empty | `NoRoot` — **no ceiling in force**, stated |

The refusal arm is load-bearing: silently reading a broken declaration as the
empty table would delete a declared ceiling on one typo, which is a widening.
`config::mount` renders the same bad read grey rather than refusing outright —
that is a **blast-radius** difference, not a strictness one: a mount table holds
many roots and isolates the bad one, while the run plane holds exactly one and
has nothing to isolate it from.

Resolution law and the declaration parse contract: `crates/run/src/caps.rs`;
design tests: `crates/run/tests/caps_home.rs`.

## Fence dispatch — two languages, one write path

The runner dispatches on the fence language (decision #13): `starlark` →
hermetic kernel eval; `bash` → exec in a scratch cwd. The language set is
closed. There is **no `Exec` EffectKind** — a replayed exec would re-run
arbitrary code, so exec never enters the effect surface.

Both paths converge on the **shared executor** (decision #4), the one write
path:

```
md.* descriptors → block-cap validation AT THE CHOKE POINT
                 → ONE atomic if_root-pinned splice batch
                 → receipt in the same commit
                 → apply→event synthesis (real post-apply fingerprints)
```

Executor laws:

- One violation refuses the whole batch; a refusal applies **nothing**.
- **Never roll back** (decision #14 / verdict ruling 2, verbatim): *"Never
  roll back ungoverned writes (rollback = second write path with invented
  authority). Ungoverned writes persist as actor-absent external change
  (§7.1 class)."*
- `live_root` is the **computed** `root_after_phase1`, threaded by the
  caller, never re-read around a bash step; a missing live root at a bash
  choke point refuses — enforcement-off is not a pass (decision #19).
- Local runs serialize under the workspace flock (decision #9); the CLI leg
  is `LOCK_NB` — a held lock is a fast typed "workspace busy" refusal.
- **Foreign-edit law** (decision #26, ZT): CAS covers only concurrent races.
  Before a replace-class effect applies to a target with a prior run
  receipt, the executor compares the target's current rev against that
  receipt's after-rev — a foreign change since is a typed `foreign_edit`
  refusal naming the target and both revs, **never a silent overwrite**.
  Overwrite requires the explicit takeover flag.

## Bash: two-phase apply inside the enforcement bracket

A bash step runs as: md.* batch (phase 1) → exec → shim batch (phase 2),
with the detection bracket around it:

- The child runs in its own process group (`setsid`) under a wall-clock
  timeout; timeout SIGKILLs the group and is a distinct typed state
  (decision #21). Background children die with the group at step end.
- Bash mutates the tree **only** through the effect-shim fd (fd 3, named to
  the child as `MD_EFFECT_FD`): length-prefixed NDJSON `md.*` descriptor
  records. A truncated or malformed stream **fails closed** — the whole
  phase-2 batch refuses (S6).
- `domain_snapshot` residual-compare runs around **every** bash step: the
  expected post-step root is the pre-step files plus this step's governed
  edits; any residual delta refuses and is named (decision #19).
- `mdfs_config.yaml` is hashed separately around every bash step; a mid-run
  change refuses — the config-widening attack (shrink the hash domain, then
  write inside the blind spot) is closed (decision #20).
- An interrupt between the phases is a typed `partial`/`interrupted` state;
  a pre-exec receipt records phase-1's committed root so lint finds orphans.
  On exec failure phase 2 refuses and **phase 1 stands committed and
  reported** (decision #22).
- An out-of-band delta is reported as *"out-of-band change during the exec
  window"* — the window is named, the block is not accused (review S4).

## The run record — stdout is data, not effects

Bash stdout never becomes a tree write. It is (verdict ruling 7):

- **streamed live** to the caller, and
- **stored out-of-tree, content-addressed** at
  `.meridian/runs/<invocation-id>.log` — addressed by invocation id, content
  pinned by the full sha256 recorded in the receipt.

Tree output happens **only** via an explicit `md.append_section` descriptor.
`.runs.md` stays dead: no in-tree run journal exists.

Record ↔ receipt linkage:

- The receipt line (in-tree, committed in the same splice batch as its
  edits) carries per-edit rev transitions — the foreign-edit anchors.
- The exec record carries **invocation id + exit code + stdout sha256 +
  byte size + log address**, joining the receipt through the
  `ExecRecordSink` seam.
- **Ordering is structural (S8):** the stdout facts are minted only by
  sealing the log, and sealing fsyncs the log file and its directory entry
  first. A crash can orphan a log (lint finds it); it can never produce a
  receipt naming a log that is not durable.
- Records carry env **keys, never values** (S7): the record type accepts
  the full child environment and can only emit the sorted key list.

## The CLI surface (locked, decision #12)

```
mrd run <PAGE> [TASK] [-- ARGS] --env K=V --dry --list --json
```

No argv JSON. TASK omitted: one declared task runs; several print the list
and exit 2 — the CLI never guesses. Contract violations exit 2 with the
declared contract shown. `--dry` on starlark evaluates hermetically and
prints the **full** effect set, applying nothing; `--dry` on bash shows the
block and its resolved caps and **refuses to exec** — running it is the only
way its effects exist, and inventing descriptors would be fiction
(decision #18). The `--dry` caps display is byte-identical to the choke-point
caps (S14).

Exit triad: **0** clean · **1** the run plane refused or failed (eval fault,
cap refusal, foreign edit, workspace busy, root mismatch, timeout, bash
nonzero) · **2** the invocation is wrong (usage, addressing, contract).

## Guarantee classes — labeled per block

| Class | Path | Claim |
|---|---|---|
| `hermetic` | starlark | proof by construction: sealed kernel, zero I/O, metered |
| `detected` | bash | root-snapshot **detection, not prevention** — ungoverned writes are detected and named, not blocked |

The label is per block, and the claim ships **scoped, never unqualified**.
The guarantee labeler **refuses to emit `detected` unless the detection path
(U6b) is landed** (decision #23) — a block is never labeled detected with
zero detection behind the label. OS-sandbox **prevention** (Landlock /
sandbox-exec) is the numbered phase-2 unit U11; S1 is detection-only.

## Accepted gaps (S1) — named, deliberate, scoped

ZT's ratified posture (plan §1, verbatim): *"detection-not-prevention for
bash and honor-system for out-of-tree writes / secret reads is INTENDED for
S1 — ship the scoped claim, never the unqualified one."*

| Gap | Class | Disposition |
|---|---|---|
| Bash enforcement is detection, not prevention | intended scope | upgrade path is U11 (OS sandbox), phase-2 |
| Out-of-tree writes / secret reads by bash | **honor-system** (accepted, ZT) | outside the hash domain by definition; the claim is scoped |
| Non-md / `.meridian/` / dot-path writes | **accepted gap, distinct from the honor-system** (decision #20) | outside the snapshot hash domain — silently undetected, named here rather than hidden |
| Symlink laundering (`ln -s secret notes/x.md`) | refused or named (decision #25) | `O_NOFOLLOW` / refuse symlinked path components in walk + snapshot; where not refusable it is a **distinct named gap** (it defeats in-domain detection, unlike plain out-of-tree) |
| Ungoverned writes are never rolled back | law, not gap (decision #14) | they persist as actor-absent external change (§7.1) and the run exits 1 with the delta named |
| Multi-file crash window (content committed, receipt lost) | accepted (decision #10) | recovery is re-derive; lint finds the missing receipt |
| Local run beside a resident daemon (§7.1) | accepted | a local run's writes reach the daemon as external change — the same class as any out-of-band edit |

## Seam map (for reviewers)

| Seam | Owner |
|---|---|
| addressing / fence / contracts / caps | `crates/run` (`address`, `fence`, `contracts`, `caps`) |
| hermetic eval | `effects::eval_run` via `crates/run::dispatch_starlark` |
| bash exec + shim + two-phase | `crates/run` (`exec`, `shim`, `dispatch_bash`) |
| detection bracket | `fs::guard` (+ `crates/run` snapshot integration) |
| the one write path | `crates/run::executor` → `model::validate_batch` → `fs::apply_batch` |
| stdout record | `crates/run::record` |
| CLI mount | `crates/mrd::run_cmd` — a client; the charter edge is `docs/laws.md` §crates (`mrd` row) |
