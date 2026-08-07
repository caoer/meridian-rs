# The run plane — `mrd run` (S1)

> **Standing:** Design law is `wire-contract.md` (one contract). Mint addresses = segments only. Receipts = armed wire facts. DuckDB/`view_path` not agent core. **Doc correct > code correct; docs first.** See `README.md`.

The run plane executes an addressed task block and turns what it emits into
governed effects. It is **consumer-plane, imperative, local** (plan decision
#1): a client of the engine crates, layered entirely above them — **"the
engine cannot tell run exists."** The daemon, the wire, and the serve path
carry no run-plane type and no run-plane state.

Sources of truth: the ratified plan (session `21-23-meridian-rs-md-run`,
`compound plan page (workspace content)`) and the round-1 verdict. This document states the
surface **as shipped in S1**, including what it deliberately does not
guarantee (§ Accepted gaps).

## The kernel entry points

The Starlark kernel has exactly three entry points (decision #3, as amended
immediately below):

| Plane | Entry | Trigger |
|---|---|---|
| change | `on_change(event)` | a governed change event (the effect kernel) |
| run | `def run(ctx)` | `mrd run` addressing a task block |
| run | script — module top level | `mrd script` / MCP `script` carrying caller-supplied inline source |

**Amendment to decision #3 (the script entry).** Decision #3 read: *exactly
two entry points, one per plane, both hermetic.* It is amended in two places
and only these two. First, the run plane carries a **second entry** — one
plane, two entries — so the count is three, not two. Second, the third entry
is **not hermetic by construction; it is hermetic by recording.** Its one
effectful builtin is `read()`; every read response is recorded into the
trace, and the law that replaces construction-hermeticity is stated in
§ The script entry: eval is a pure function of
`(script, args, files, read-response sequence)`. Replayability — the
property construction-hermeticity bought — is preserved by recording it
instead. The other two entries are untouched: `on_change(event)` and
`def run(ctx)` remain hermetic by construction, and the script builtins do
not join their globals.

`RunCtx` is inert data: page, task, args, env, invocation id, root-at-eval.
Identity and time are **caller-supplied** (§9) — the kernel never reads a
clock and never mints an id. Starlark-invokes-bash is a **permanent no**
(decision #17, test-gated): the sandbox exposes no `exec` / `subprocess` /
`os` name. The composition layer IS bash.

## The script entry

The script entry runs **caller-supplied inline source** instead of an
addressed task block. It is the same plane and the same one write path; only
the entry differs. A task is a governed page's declared behavior; a script is
one caller's inline intent.

**Inputs — all caller-supplied, all inert** (the `RunCtx` precedent):

| Input | Meaning |
|---|---|
| `script` | the source; the module top level IS the body, no hook lookup |
| `args` | the caller's arguments, inert data |
| `files[]` | **paths only**, sorted, pre-enumerated by the host |
| `actor` | the caller's own identity, threaded per §9 — the engine mints none |
| `now` | caller-supplied time — the kernel never reads a clock |
| budget overrides | fuel / mem / call depth / source bytes / wall clock / max reads / max armed edits |

`files[]` carries paths and never content: all content enters through
`read()` and is recorded, and inline content would bypass the recorded-read
purity law below and break replay. There is no `glob()` builtin — enumeration
is the host's, and the wire serves no corpus-enumeration op. There is no cap
grammar at this entry: authority is the caller's identity, not a declared
ceiling, and the delegation caps grammar is a v2 feature.

**Recorded-read purity.** `read(path)` (the toc face) and
`read(path, section=…)` (the cat face) are the **only** effectful builtins;
`put(...)` and its siblings are pure — they append to the armed list and
perform no I/O at call time. The law: **script eval is a pure function of
`(script, args, files, read-response sequence)`.** The trace records every
read response, so re-evaluating against the recorded responses is
deterministic and byte-identical. Decision #17 (no exec) stands unchanged.

**The arming surface — `put()` speaks the wire's second edit dialect.**
`put(path, props={…})` arms one `set_property` plan item per key, keys sorted;
`put(path, section="…", append="…")` arms one section-addressed `append`. These
are `splice.plan_edits[]` items (§A.3) carried **verbatim** — the armed list IS
`plan_edits[]`, lowered by the engine's existing intake
(`wire-serve::plan::lower`), so no third edit grammar is minted and the wire
schema is untouched. One `put()` call may arm several items; arm order is
execution order, and each armed item records its source line and nesting depth.
Depth is a trace fact only: **an applied effect renders at any depth** — the
echo/quiet rule governs reads, not arms.

An `append` **addresses a section**. `PlanEdit::Append` carries an hpath and an
empty one refuses `NotFound`, so a document-grain append has no wire target; a
bare `append=` with no `section=` refuses at arm time rather than minting a
default section, and the MCP `put` face refuses the same shape in the same
words. A `props=` write needs no section — frontmatter is file-grain.
Addresses are segments: `section="Notes/Fresh"` is two segments, never a joined
string.

**The statement-position rule — echo and quiet.** Every read is recorded; the
face renders only the ones the reader wrote as a decision. A read **echoes**
exactly when its call is the whole right-hand side of a top-level assignment,
or a top-level expression statement — `card = read(…)` binds and echoes. Every
other position is **quiet**: comprehensions, `if` conditions, loop bodies,
function bodies. The kernel reads the positions off the parsed module, so the
rule is syntactic and stable, never a call-depth heuristic. Suppression syntax
does not exist in v1 (`_ = read(…)` is rejected permanently; `quiet()` waits on
elision-count evidence).

**The grammar.** The script entry parses under the rule dialect plus top-level
statements — its module top level IS the program, so `if` and `for` at the top
level are the ordinary case there. A rule or a task must define a hook, so the
hooked planes keep the stricter grammar. `load` stays disabled at every entry.

**Where the budgets bind.** Fuel, memory, call depth, and source bytes are
`EvalLimits`, shared with the other two entries. The read ceiling (64 per
attempt) is the script plane's own — the I/O-amplification axis the hermetic
entries do not have — and the kernel enforces it: the read past the ceiling
refuses typed, naming the ceiling, and the attempt has no result at all. Never
truncation. Wall clock, retry budget, and the armed-edit ceiling bind in the
host, above the kernel.

**The transaction — stand-still optimistic.** The word **snapshot is
banned** here: the daemon has no MVCC and v1 must not grow one.

1. **Entry.** One `fingerprint` call (§4.7) pins the **entry fingerprint**.
2. **Reads are LIVE.** If the world moves mid-script, reads may span states.
3. **Commit.** ONE splice batch carrying `if_fingerprint` = the entry
   fingerprint, and §5.1 checks that guard **first**.
4. **Any interleaved write** ⇒ `fingerprint_mismatch` ⇒ nothing commits.

A caller may also pin its own `if_fingerprint?` guard. It is checked against
the minted entry fingerprint **pre-eval** — mismatch refuses immediately with
zero evaluation, read-class, and the run is `attempts:1` by construction. That
pre-eval check is a fast-fail courtesy, **not** the authoritative one: the
commit's splice still carries the guard and §5.1 still checks it first, which
is what catches a world that moves *during* eval. Two checks, one value.

The guarantee, stated exactly: *a committed script is consistent with exactly
one workspace fingerprint — the world stood still, or the commit refused.*

The entry itself is **single-attempt**. A conflict at the entry is one
`fingerprint_mismatch` with recovery `resync`; the retry loop belongs to the
host (budget 2), which re-resolves a selector per attempt and re-runs pinned
`files[]` as-pinned. `attempts:N` is therefore a host fact, stamped on the
composed face, never a field of the entry's own trace.

**One CONTENT path per commit (v1 law).** §4.4 splice carries one `path`, so
a one-splice commit exists only for a single-file write set. State it as one
CONTENT path — the **receipt companion still rides the same batch** (§6.1:
one fingerprint advance covering both files), so this law must not be read as
contradicting the two-file receipt commit or as reopening the §6.5 crash
window. A script arming a second content path refuses `multi_file_write_set`
**pre-commit** — consumer-plane typed and face-rendered, never a §8 code; the
closed taxonomy stays closed. Multi-file atomicity remains the §6.5 rung-3
candidate.

**Wire-client mode.** When a daemon is resident the script entry does its I/O
**as a wire client through the one door**: reads lower to `toc`/`cat`, and the
commit lowers to ONE guarded `splice` carrying `actor`/`now`/`receipt`. §4.4
is untouched — splice remains the only write op and the script executor is
just another client — and the daemon still carries no run-plane type and no
run-plane state. The whole wire cost of this entry is **zero schema delta**.

**The commit is guarded per row, by the read the script itself made.** A wire
door demands a fingerprint for every edit that changes existing content, or an
explicit `force` (`wire-serve::guard`), and the two grains differ: a
`set_property` row takes the **file** rev, because frontmatter semantics are
file-scoped, and an `append` row takes the **node** rev of the section it lands
in. A script already holds both — `read(path)` recorded the file rev and the
section map, `read(path, section=…)` recorded that section's rev — so the
consumer plane threads each row's token out of the recording, using the LAST
read of that target, since reads are live. This is the read-then-write CAS the
wire exists for, not a token minted to satisfy a check: **a row whose target the
script never read carries no token and meets the engine's own refusal**, which
is the honest answer to writing what you did not read. The two guards compose —
`if_fingerprint` says the world stood still, each row's `rev` says the thing it
edits is still what the script saw.

**Amendment to § The script entry (the write-follows-read law).** The paragraph
above records a mechanism — where each row's token comes from. It is amended to
also state the **behavioural law** that mechanism creates, because a face built
on the mechanism without the law would promise writes the engine refuses:

> **A `put()` row's target must have been READ this attempt, or the row carries
> no token and the wire door refuses the whole batch.**

Three things follow, and only these three. First, the law binds **per attempt**,
not per session: the recording is the run's own, so a retry that re-reads is
guarded on what it re-read, and a run that reads nothing writes nothing. Second,
it binds **at the row's grain** — a `props={…}` write needs the file read
(`read(path)`), an `append` needs its section read (`read(path)` for the section
map, or `read(path, section=…)`), so reading a file's toc licenses both and
reading one section licenses only that section. Third, the refusal is the
**engine's**, not the client's: the consumer plane mints nothing to satisfy the
guard, it declines to, and `guard_required` comes back with the engine's own
teaching text. `force` is not a script-plane door.

Evidence, and where it is held: `crates/mrd/tests/script_golden_live.rs` runs
every golden scenario (`inbox/run-golden.html` v9) through the real entry against
a **live daemon** and asserts that every `plan_edits[]` row on the socket carries
a token its own reads published. All of them conform. The same suite pins the
law's other direction with a counter-example — an append to a target the attempt
never read must be refused whole by the engine, not silently accepted.

**`--dry` is a rehearsal, not a commit.** The splice carries `dry: true`, so the
daemon builds the whole effect set and applies none of it; the response — with
its own `dry: true` and `fingerprint_after: null` — rides the trace as the commit
leg, and the outcome is `no_effect`, because nothing landed: no receipt, no
fingerprint advance, workspace unchanged. Every armed entry stays
`[not committed]`. A caller-guard refusal is `conflict` with **no** commit leg
and zero telemetry: no splice was issued, so no §5.1 body exists to embed. Both
extras tokens still ride the trace in band — `actual` IS the trace's
`entry_fingerprint`, and `expected` is the caller's pinned value carried as
`guard_expected`, present on exactly this terminal. The face renders from the
trace and nothing else, and `conflict` + no commit leg + `guard_expected` is
what tells a guard refusal apart from a commit-time mismatch.

**The trace — one commit-fact shape, and no `attempts`.** The entry returns a
`ScriptTrace`: the entry fingerprint, the outcome
(`committed | no_effect | conflict | fault | refused`), the decision trace, an
optional commit leg, an optional fault, and telemetry. Three laws hold it
together:

- **The commit leg IS the §4.4 splice response, embedded verbatim** — carried as
  raw bytes, never re-typed. The rev transitions, the receipt fact,
  `fingerprint_before/after`, and `verdicts` (rules-as-data) all ride it, so no
  second commit-fact shape exists and none can drift when §4.4 grows a field. A
  `fingerprint_mismatch` embeds through the same leg: it is the splice's own
  response. Absent when no splice was issued — the read-class path.
- **There is no `attempts` field.** The entry is single-attempt; the retry loop
  is the host's, so `attempts:N` is a host fact stamped on the composed face.
- **Telemetry is unconditional** — fuel, memory, reads, wall time, reported on
  faults and refusals too (the `RuleTelemetry` precedent).

The decision trace is one entry per recorded read, in call order, then the armed
block in arm order. A read's entry kind IS the statement-position rule — `echo`
for a top-level-statement read, `read` for every quiet position — and each armed
entry carries the wire plan-edit verbatim plus whether the commit landed it, so
the face's wrote-lines zip descriptor × result exactly as put faces do today.
The fault taxonomy is CLOSED at `parse | runtime | budget | refused`: a refusal
is not a fault, and the two must grep apart.

**The `mrd script` human-mode face is non-normative.** The MCP host owns the
normative text face, rendered from the trace; `mrd script --json` emitting the
trace is the contract between them. The CLI's human mode is an operator
convenience. Two normative renderers in two languages would drift, and only
the host knows `attempts:N`.

### The two entries of the run plane — seam table

The mechanism is shared — sealed Starlark kernel, `md.*` descriptors, the one
write path. Everything that differs is at the entry.

| Axis | Task entry (`mrd run`) | Script entry (`mrd script` / MCP `script`) |
|---|---|---|
| Source | addressed fenced block in a governed page (`task.<name>: "[[#^block]]"`) — reviewable, rev-pinned, in the hash domain | inline source, caller-supplied per call; never lands in the tree |
| Authority | ambient — whoever invokes the page runs its task; caps declared in frontmatter, deny-by-default, root ceiling narrows only (decision #15) | the caller's own identity — `actor` threaded per §9; ownership guard + armed law; no cap grammar (caps = v2 delegation feature) |
| Entry point | `def run(ctx)` (decision: one entry per plane) | module top level — the script IS the body (kernel entry #3) |
| Languages | starlark + bash (fence dispatch, decision #13) | starlark only; no exec, ever (decision #17 stands) |
| Hermeticity | hermetic by construction: sealed kernel, zero I/O, `RunCtx` inert | recorded-read purity: eval is a pure function of (script, args, files, read-response sequence); trace records every read; replay against recorded reads is byte-identical (decision #3 amendment) |
| Reads | none — inputs arrive as inert `RunCtx` data | `read()` lowering to `toc`/`cat` — live, as a wire client through the one door when a daemon is resident |
| Enumeration | page names its own targets | none in-kernel: host resolves selector → inert **sorted** `files[]`, paths only |
| Commit | one atomic `if_fingerprint`-pinned batch via the local executor | ONE guarded splice as the caller (`actor`/`now`/`receipt` on the request); **write set = one file (v1 law)**; `multi_file_write_set` refuses pre-commit |
| Concurrency | workspace flock, `LOCK_NB` (decision #9) | stand-still optimistic: entry fingerprint pinned, commit `if_fingerprint` = entry; conflict ⇒ host re-resolves selector and retries (budget 2, `attempts` on the face) |
| Failure grain | one violation refuses the whole batch; bash phase-1 may stand committed and reported (decision #22) | one violation refuses the whole script; nothing ever partially lands (single-write-file keeps retry sound) |
| Output | run record: stdout streamed + content-addressed out-of-tree log; receipt linkage via `ExecRecordSink` | `ScriptTrace` → text face: echo semantics, embedded §4.4 splice response verbatim, telemetry always present |
| Guarantee label | per block: `hermetic` (starlark) / `detected` (bash, U6b) | recorded-read + stand-still, stated as such; zero-armed outcome is read-class (`Ok(vec![])` precedent) |
| Daemon relation | local run beside a resident daemon = external change (accepted-gaps row, actor-absent) | wire client — writes arrive as governed change, actor-carrying, Delta-minted like any splice |
| Typical caller | operator / CI invoking a page's declared task | agent making a plan-shaped call over MCP (≥2 dependent steps with a decision between them) |
| Promotion | a task is already a convention — page-owned, addressable, armable | a re-sent script is a convention trying to be born: REWRITE as `on_change(event)` (reads dropped, payload-only), then the U4.4 ladder — registration ≠ activation, a reviewer arms |

The collision is at the entry, not the plane — both entries converge on the
one write path, and neither may grow a private executor.

A task's authority comes from where it lives; a script's authority comes from
who sent it. Every row above is that sentence applied to one axis.

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
run.caps.fix-note: md.set_field:status # longest pattern wins
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
line, so a nested `run:`/` caps:` spelling would be unreadable.

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
hermetic kernel eval; `bash` → exec in the **invocation cwd**. The language set
is closed. There is **no `Exec` EffectKind** — a replayed exec would re-run
arbitrary code, so exec never enters the effect surface.

A bash step runs where `mrd` runs (U16, requirements row E1 — *"DO NOT CHANGE
THE RUNNING PATH"*). The supervisor does not relocate the process; the
caller-minted out-of-tree scratch directory stays, as the artifact location
only. The project root reaches the step as `$MERIDIAN_PROJECT_ROOT`
(convenience, decision P6).

Say the consequence plainly: a step CAN write into the tree, and such a write is
neither tolerated nor merely reported. The U6b exec bracket detects it as
`OutOfBand` and **phase 2 refuses to converge** — nothing the step emitted
applies, no completion receipt is written, and the ungoverned write is never
rolled back (ruling 2). Governed change reaches the tree only over the shim fd.
Gates: `crates/run/tests/dispatch_bash.rs`
(`an_ungoverned_tree_write_refuses_phase2_with_the_delta_named`,
`a_project_root_relative_stray_write_refuses_convergence`).

Both paths converge on the **shared executor** (decision #4), the one write
path:

```
md.* descriptors → block-cap validation AT THE CHOKE POINT
 → ONE atomic if_fingerprint-pinned splice batch
 → receipt in the same commit
 → apply→event synthesis (real post-apply fingerprints)
```

Executor laws:

- One violation refuses the whole batch; a refusal applies **nothing**.
- **Never roll back** (decision #14 / verdict ruling 2, verbatim): *"Never
 roll back ungoverned writes (rollback = second write path with invented
 authority). Ungoverned writes persist as actor-absent external change
 (§7.1 class)."*
- `live_fingerprint` is the **computed** fingerprint after phase 1, threaded by
 the caller, never re-read around a bash step; a missing live fingerprint at a
 bash choke point refuses — enforcement-off is not a pass (decision #19).
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
- The **domain config** is hashed separately around every bash step
  (`meridian/domain.md`, or legacy sole `mdfs_config.yaml` if that is the only
  file present — do not teach yaml as standing). A mid-run change refuses —
  the config-widening attack (shrink the hash domain, then write inside the
  blind spot) is closed (decision #20). See `wire-contract.md` §12.
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
mrd script [--json]                      # source on stdin (heredoc)
```

**Amendment to decision #12 (the locked surface gains `mrd script`).**
Decision #12 locked the CLI surface at `mrd run` and its flags. It is
amended: the surface gains exactly one subcommand, `mrd script`, which takes
the script source on stdin and serves the script entry above. The amendment
is named here rather than slipped into the synopsis, because "locked" means a
new verb is a stated change, not an addition nobody has to notice. Everything
below about `mrd run` is unchanged by it, and `mrd script`'s own human-mode
face is non-normative (§ The script entry).

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
| The script entry runs in **wire-client mode**, not pure-local | law, not gap | a script must execute AS the caller, and the row above disqualifies the pure-local leg by this plane's own table: its writes reach a resident daemon actor-absent. Through the daemon, a script's writes arrive as governed, actor-carrying change, Delta-minted like any splice |

## Seam map (for reviewers)

| Seam | Owner |
|---|---|
| addressing / fence / contracts / caps | `crates/run` (`address`, `fence`, `contracts`, `caps`) |
| hermetic eval | `effects::eval_run` via `crates/run::dispatch_starlark` |
| bash exec + shim + two-phase | `crates/run` (`exec`, `shim`, `dispatch_bash`) |
| detection bracket | `fs::guard` (+ `crates/run` snapshot integration) |
| the one write path | `crates/run::executor` → `model::validate_batch` → `fs::apply_batch` |
| stdout record | `crates/run::record` |
| CLI mount | `crates/mrd::run_cmd` — a client; the charter edge is `laws.md` §crates (`mrd` row) |
| CLI mount — script entry | `crates/mrd::script_cmd` — the same client edge; its human-mode face is non-normative |

---

# Presets and session birth — the design element

> Folded into `run-plane.md` for navigation. Standing corrections: `README.md` / `wire-contract.md`.


A **preset** is a def page that declares a shape. **Session birth** is the act
of turning that declared shape into files. This document is the design element
the `preset` crate and the `new` / `unfold` / `reconcile` verbs are audited
against: it states what the plane is FOR, the laws it may not break, and the
boundaries it may not cross.

It is not a description of the current code. Where the code and this element
disagree, the element wins and the code is rebuilt.

## 1. The premise — a shape is declared once, in a page

The alternative this plane exists to replace is a scaffolding script: a
generator that knows the shape in code, births files nobody can re-derive, and
drifts from the shape the day someone edits the tree by hand.

The premise here is the opposite. **The shape lives in a page**, in the same
markdown the engine already governs, and a born tree **pins the def it came
from at the def's rev** — so the shape that produced any file is recoverable
forever, from the file, without the tool that wrote it.

Everything below follows from that premise.

## 2. The def grammar

A preset def is a page carrying `type: def`. Its frontmatter declares:

| Key | Meaning | Absent |
|---|---|---|
| `type` | must be `def` — anything else is not a preset | refuse (tool failure) |
| `defines` | the kind this def births (`session`, `task`, …) | empty kind |
| `root` | the root record the scaffold pins the preset into | `SESSION.md` |
| `births` | the `{{id}}`-filled target path template for one record | `{{kind}}/{{id}}.md` |
| `inputs` | the convention-floor pins — a **block sequence** | no floor pinned |

Its body declares, in named sections:

- `# Properties` (`^properties`) — the rules a born record must satisfy. One
  `- key` or `- key = value` list item per rule.
- `# Template` (`^template`) — the fenced body one record is born from.
- `# Unfold` — the declared scaffold: the file paths a whole birth materializes,
  in declared order.
- `# Ephemeral` — the **allowlist** of declared-disposable paths. Empty by
  construction, so a def that declares nothing disposable can prune nothing.

**Law 2.1 — `inputs` is read and written whole.** It is a multi-line block
sequence, read through the whole-value frontmatter grain and written as whole
birth bytes. A line-oriented scan that stops at the key line, or a single-line
properties upsert, corrupts it. The read half and the render half are one round
trip and are audited as a pair.

## 3. The birth law — one door

**Law 3.1 — every byte a preset lands rides the guarded create.** No
`fs::write`, no second write path, no exception for a stub, a dry run, or a
scaffold file the author thinks is uninteresting. The guarded create carries
three things a raw write cannot: the `if_absent` CAS, the journaled birth
receipt, and the gate seam.

**Law 3.2 — a birth never clobbers.** An occupied target is the CAS's answer,
not the plane's decision. It surfaces as a `cas_mismatch` finding and the file
on disk is left byte-untouched. This holds on a dry run too: a rehearsal that
would have clobbered still refuses.

**Law 3.3 — every removal rides the guarded remove**, read-then-delete under
the live rev. The one exception is an empty directory, which carries no
governed rev and no bytes to protect; it is removed with a raw `rmdir` and that
exception is stated here so it cannot be widened silently.

**Law 3.4 — the plane mints no identity and no clock.** `actor` and `now` are
caller-supplied and stamped exactly as given. Absent stays absent. A crate that
reads the wall clock cannot be tested against a fixture and cannot be replayed.

## 4. The three verbs

The plane offers exactly three births, and they differ only in **what set of
paths they act on**. They share one def loader, one renderer pair, and one
guarded door — a verb that grows its own copy of any of those is wrong-design.

| Verb | Acts on | Refuses when |
|---|---|---|
| `new <kind> <id>` | ONE record, from `^template` | the def is invalid, or the target exists |
| `unfold <preset>` | EVERY declared scaffold path | any declared path already exists |
| `reconcile <preset>` | the MISSING declared paths only | — (an occupancy is not a failure here) |

**`new` validates before it writes.** The filled template is parsed and checked
against every `^properties` rule; the FIRST violation refuses `def_invalid`
naming the rule verbatim. A def with no `^properties` block, no `^template`, or
a rule that did not parse is itself invalid — the same refusal, because a def
that cannot state its own contract cannot birth a record that satisfies it.

**`unfold` is the first birth; `reconcile` is every birth after it.** That is
the whole difference: unfold treats an occupied path as a finding because it
expected to create the world, and reconcile treats it as convergence because it
expected the world to be partly there.

## 5. The reconcile asymmetry (ZT ruling #3)

**Law 5.1 — reconcile is additive by set-difference, subtractive by
allowlist.** These are not two spellings of one operation and must never be
refactored into one:

- **Materialize** every declared path missing from the tree. Set-difference.
- **Prune** only paths matching the `# Ephemeral` allowlist, plus empty
  undeclared directories. Allowlist.
- **Everything else** — undeclared content — renders as a **finding**. Never a
  prune action, never under any flag.

**Law 5.2 — "undeclared" is not "unwanted".** The tempting symmetry (delete
whatever the def does not declare) is the one thing this design forbids. A user's
file that the def has never heard of is a report to the user, not garbage. The
asymmetry is the safety property; a change that makes the two halves symmetric
has deleted the design, whatever the tests say.

**Law 5.3 — reconcile stays inside the shape's territory.** The scan scope is
the set of directories the declared scaffold occupies. Reconcile never reads,
reports on, or prunes a path outside it. Engine and system files (dotfiles, the
reserved journal) are never "undeclared content".

A prunable **directory** is one that lives strictly beneath a directory the
scaffold itself creates, is not an ancestor of a declared path, holds no
finding, and is empty. A scaffold declaring only top-level files creates no
directory and therefore prunes none: the workspace root is never walked for
directory candidates, because every empty directory in a user's workspace is not
this shape's territory.

**Law 5.4 — pruning is opt-in.** Without `--prune`, reconcile materializes and
reports and removes nothing.

## 6. The convention floor

A session preset's `inputs` pin the convention floor — the rule pages the born
session lives under — at a path and a rev. The root record is born carrying that
pin, so the law a session was born under is readable from the session itself
long after the def has moved on.

**Law 6.1 — a floor pin is a pin, not a copy.** The preset records `path@rev`;
it never inlines the floor's content into the born tree.

## 7. Refusals and exit codes

The plane distinguishes two failure kinds and never conflates them:

| Kind | Exit | Examples |
|---|---|---|
| **Finding** — the plane ran and reported | 1 | `def_invalid{rule}`, `cas_mismatch`, an undeclared-content finding |
| **Tool failure** — the plane could not run | 2 | the def is unreadable, the page is not a def, a write faulted for a reason other than the CAS |

**Law 7.1 — a refusal names the rule it enforced.** `def_invalid` carries the
source text of the violated `^properties` rule. A refusal that says only "the
def is invalid" makes the author guess, and this plane's whole value is that the
shape is stated in a page they can read.

## 8. Boundaries — what this plane never does

- It holds **no session policy and no liveness**. Whether a session is active,
  expired, or archived belongs to the customer that dials this plane.
- It owns **no CLI**. `mrd new` / `unfold` / `reconcile` are thin clients:
  argument parsing, workspace resolution, output shape. Every decision this
  document states lives in the crate, so a second host reaches the same
  behaviour without re-deriving it.
- It invents **no write path, no hash law, no rev noun**. It composes the
  shipped ones.

## 9. The user-facing surface carries no internal tags

**Law 9.1 — a verb's help text is written for the person typing the verb.**
Internal planning identifiers — unit numbers, block numbers, plan-section
references, the tags a docket uses to track its own work — are project
bookkeeping. They are legitimate in source comments, crate metadata, and test
names, where the reader is a contributor holding the plan. They must not appear
in `mrd help` output, where the reader is a user who has never seen the docket
and for whom `(U5.3)` is noise that reads as a version, a flag, or an error
code.

This is gated by a test over the real help output, not by review discipline.

---

## Appendix — the conformance audit (2026-08-03)

The first audit of the `preset` crate and the `new` / `unfold` / `reconcile`
verbs against this element. Every law is listed, including the ones the code
already satisfied — an audit that reports only its failures cannot be checked by
the next reader, who has no way to tell an unexamined law from a passing one.

| Law | Verdict | Evidence |
|---|---|---|
| 2.1 whole-value `inputs` | **conformant** | `read_inputs_grain` resolves `FmKey("inputs")` and spans the whole block; `render_block_sequence` is its writing half. The round trip is gated by an existing test. |
| 3.1 one write door | **conformant** | Every landing byte in the crate goes through `birth` → `wire_serve::write::create`. No `fs::write` exists in the crate. |
| 3.2 never clobber | **conformant** | `if_absent` CAS; `BirthResult::Occupied` is a finding, never a fallback write. `opts.dry` is passed to the door, so a dry run refuses too. |
| 3.3 guarded remove | **conformant** | `prune_file` reads the live rev and removes under it. The `rmdir` exception is the empty-directory case, now stated in the element rather than only in a comment. |
| 3.4 no minted identity or clock | **conformant** | `actor` / `now` are `Option<String>` on `BirthOptions` and are never defaulted from a clock; `fill_vars` renders an absent one as empty. |
| 4 three verbs, one door | **conformant** | All three call `load_def` and `birth`; none carries a private write path or a second renderer. |
| 4 `new` validates before writing | **conformant** | Structural def checks, then `first_violated_rule`, then birth. A def that cannot satisfy its own `^properties` refuses before any byte moves. |
| 5.1 additive by diff, subtractive by allowlist | **conformant** | `reconcile_plan` is a pure fold and keeps the two halves as separate fields. |
| 5.2 undeclared is not unwanted | **conformant** | `findings` is never read by the prune path. |
| 5.3 territory — file half | **conformant** | `scan_scope` walks only the directories directly holding a declared path; dotfiles and the reserved journal are skipped. |
| **5.3 territory — directory half** | **WRONG-DESIGN — deleted and rebuilt** | `prune_empty_dirs` drew its candidates from `scope_dirs(declared)` and its skip set from `declared.flat_map(ancestors_of)` — the same expression. Every candidate matched the skip set, so the function returned an empty vector for every possible input and `pruned_dirs` was dead. Rebuilt to walk the live tree beneath the scaffold's own directories, bounded so a top-level-only scaffold never reaches the workspace root. Gated by three new tests, one of which is the bound. |
| 5.4 prune is opt-in | **conformant** | Gated by an existing test. |
| 6 floor pin is a pin | **conformant** | `render_root_record` writes `path@rev`; the floor's content is never inlined. |
| 7 finding vs tool failure | **conformant** | `RefusalReason` (exit 1) and `PresetError` (exit 2) are separate types; only a non-CAS write fault crosses into the latter. |
| 7.1 a refusal names its rule | **conformant** | `def_invalid` carries `PropRule::raw`, the source text verbatim. |
| 8 no session policy, no CLI ownership | **conformant** | The crate holds no liveness state; the three `mrd` modules parse arguments and shape output only. |
| **9.1 no internal tags in help** | **VIOLATION — fixed** | Four help descriptions opened with `(U5.3)` or `(U3.5b; ZT ruling #3)`. Stripped. A derived test now scans every page the CLI can print. Source comments and crate metadata keep their tags, which §9.1 permits. |

Two documentation rows were also found stale against §4 and §8 and corrected:
the `docs/laws.md` crate charter omitted `reconcile` entirely, and
`crates/mrd/src/preset_cmd.rs` described itself as serving "the two preset
verbs" while three verbs dial its `def_path` and five dial its `resolve_root`.

**No part of this plane was found to warrant removal.** The audit's one
wrong-design finding is a bounded rebuild of a single function, and the ZT
ruling to KEEP the feature is untouched by it.
