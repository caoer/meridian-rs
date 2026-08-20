---
type: spec
id: run
status: standing
updated: 2026-08-18
description: The run plane — how `mrd run` executes an addressed task block and turns what it emits into governed effects, plus preset and session birth.
owns: [the run plane, preset and session birth]
---

# The run plane — `mrd run` (S1)

> **Standing:** Design law is `wire-contract.md` (one contract). Mint addresses = segments only. Receipts = armed wire facts. DuckDB/`view_path` not agent core. **Doc correct > code correct; docs first.** See `README.md`.

The run plane executes an addressed task block and turns what it emits into
governed effects. It is **consumer-plane, imperative, local** (plan decision
#1): a client of the engine crates, layered entirely above them — **"the
engine cannot tell run exists."** The daemon, the wire, and the serve path
carry no run-plane type and no run-plane state.

**Amendment (2026-08-12, phase-2 script-plane ruling): the sentence above is
narrowed to the TASK entry.** The SCRIPT entry's executor moves into the
engine daemon — in-process Starlark evaluation behind the wire `script` op
(wire-contract § A.7) — so the daemon now carries the script entry's
evaluator and its per-attempt state, for exactly the attempt's duration and
no longer. The task entry (`mrd run`), the change kernel, and everything
else in this document stay consumer-plane as written, and the daemon still
holds no run-plane state ACROSS attempts of any kind. The subprocess script
lane (`mrd script`, wire-client mode below) stays functional; its removal is
a separate ruling this amendment does not make.

**Amendment (2026-08-13, run-crossing ruling): the task entry becomes
wire-invocable too (wire-contract § A.8).** ZT ruled `run` crossing a KEY
FEATURE — a list of targets through the face, and `run()` callable inside
the script entry (live, under § Effects mode). The daemon now carries the
§ A.8 op arm (per-target loop over the unchanged `crates/run` seam, §9
identity threading) and the effects-mode live host, still per-invocation and
never ACROSS invocations. "The engine cannot tell run exists" retires as the
charter sentence; its successor is narrower and still load-bearing: **the
serve path consumes the run plane, it never re-implements it** — one runner,
one executor, one receipt convention, whichever door invoked it. The CLI
entry stays a client as written.

**Amendment (2026-08-15, no-guard-on-effects ruling — NORMATIVE): `run` and
script-with-effects are NOT guarded.** Ruled,
`decisions/2026-08-15-no-guard-on-effects.md` (ZT, verbatim: *"make sure we
don't guard run, script with effects, no meaningless safety guard to give
false promise and cause complexity and slowness"*): no CAS premise, no
fingerprint requiredness, no synthesized touch-set guard, on execution whose
consequences mrd cannot bound — a bash script can install a timer that fires
five minutes later, and no check evaluated at commit time can prevent that.
A guard there PROMISES what it cannot keep and buys complexity and slowness
for the false promise. Guards are pure-lane law: the premise/coverage/token
machinery (wire-contract §5.4–§5.7, § A.7) applies to markdown writes
through the pure doors, where the engine CAN keep the promise. Consequences,
each stated so no implementer picks silently:

- **The plane's self-manufactured world pin is REMOVED, and no narrower pin
  replaces it.** The root-mismatch premise refusal (§ The CLI surface,
  churn paragraph) and the per-target pin-and-verify (the foreign-edit law,
  decision #26, § Fence dispatch executor laws) RETIRE. No refusal on this
  door is a premise refusal.
- **What remains is observation honesty.** A foreign advance re-derives and
  proceeds — reported (the named out-of-band window), never refused. A
  vanished unrelated record drops from view and never fails another target;
  a vanished ADDRESSED target stays an invocation-law refusal, which is
  addressing, not a premise.
- **A task-selection pin is TARGETING, never CAS.** A pin (`task_rev` on
  the wire row, or any future selection pin) chooses WHAT to execute; its
  documentation and its faces must call it targeting. A supplied guard
  field is rejected as inapplicable, never ceremonially checked
  (wire-contract § A.8).
- **Guard-free never means fold-invisible.** Every effects write rides the
  same write choke-point and maintains the resident tree — an effects write
  advances the folds other writers' premises compare against. That is tree
  maintenance, not a guard.
- **Unchanged by this ruling:** `put_live` stays CAS-free by its own
  standing ruling (2026-08-13); `effects` and guard fields stay mutually
  exclusive at decode; the step's OWN out-of-band writes still refuse
  phase-2 convergence (the governed-change law, decisions #14/#19 —
  enforcement of the one write path, not a world premise); `run.lock`
  serialization stays (a lock refusal is not a premise refusal).

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
| run | script — module top level | `mrd script` / the wire `script` op (wire-contract § A.7) / MCP `script` carrying caller-supplied inline source |

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

*Amendment (2026-08-13, script-effects ruling): #17 is OVERTURNED on the
effects path only. A submission carrying `effects:["run"]` holds a live
`run()` that executes the addressed task at call time — a sanctioned exec
surface, chosen deliberately over an armed/deferred chimera (ZT, verbatim:
"Effects cannot be refused (out-of-world), so the transaction promise is
unkeepable there — no half-promises, no chimera"). The PURE path keeps #17
word for word: no exec surface, eval a pure function of its recorded
inputs. The surface test asserts both: pure globals stay `{read, me, put}`;
effects globals add exactly the admitted list.*

## The script entry

The script entry runs **caller-supplied inline source** instead of an
addressed task block. It is the same plane and the same one write path; only
the entry differs. A task is a governed page's declared behavior; a script is
one caller's inline intent.

**Inputs — all caller-supplied, all inert** (the `RunCtx` precedent):

| Input | Meaning |
|---|---|
| `script` | the source; the module top level IS the body, no hook lookup |
| `args` | the caller's arguments as an **inert dict** — string keys, string values |
| `files[]` | **paths only**, in call order — `files[i]` is the i-th path the caller named (order-bind ruling); patterns expand in place, and a pattern standing before a literal refuses at entry (`files_member_order`, wire-contract § A.7 literals-first) |
| `actor` | the caller's own identity, threaded per §9 — the engine mints none |
| `now` | caller-supplied time — the kernel never reads a clock |
| budget overrides | fuel / mem / call depth / source bytes / wall clock / max reads / max armed edits |

`args` is a **dict, not a list** — callers name their inputs (`args["page"]`),
they do not count them. It is inert in the `RunCtx.env` sense: string keys,
string values, no callables and no host reach, so nothing in it can read or
write. The kernel binds the dict; a host never flattens or reshapes one on the
way in, which is what keeps a single args grammar in a single place.

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

**The read budget states its own domain: 64 `read()` CALLS per attempt, NOT 64
files.** The unit is the entire statement. `max_reads` counts calls to the read
builtin — the kernel holds one counter over recorded reads with **no dedup by
path**, and a section read (`read(p, section=…)`, the cat face) is pushed
identically to a whole-file read (`read(p)`, the toc face). So a file taken as
toc + N sections spends **1+N** of the budget, and **the effective FILE domain is
strictly smaller than 64 the moment sections are used.**

Measured, with its positive control beside it (published binary at `27cf2bca`,
2026-08-11): a `--files` list of **3 files** with 30 sections each refuses at 70
section reads — `outcome fault`, `reads_used 64` — while the identical corpus,
list, and addressing at 60 section reads returns `outcome no_effect`,
`reads_used 60`, exit 0. Three files exhausted a budget a reader would have
called "64 files". The control differs from the test in exactly one variable,
the call count, so the refusal is the budget and nothing else.

**Effects mode (added 2026-08-13, script-effects ruling; supersedes the
same-day armed-run design, under which no code shipped).** A submission may
carry `effects: […]` beside `dry`/`files`/`args`: the list declares which
effect builtins the program may use. The closed set's one home is the wire
contract's § A.7 effects paragraph (today: `run`, `token_count`); a
`mutex()` builtin mirroring fleet make-mutex semantics is recorded
DO-NOT-BUILD, for later. **The flag switches the execution model:**

- **Absent → pure script.** Everything above, word for word: entry world,
  armed set, set-form law, the touch-set commit premise (the 2026-08-15
  amendment below), replay. A script is provably pure by default.
- **Present → LIVE PROGRAM.** `read()` serves the live disk at call time —
  no pin, no overlay, its own read is the world. `put()` applies
  IMMEDIATELY through the wire splice door: write flock held, structural
  validation intact, the guard's `force` bypass — no rev, no snapshot, no
  CAS; the set-form law does not apply (it is the pure
  TRANSACTION's law, and there is no transaction here). `run(page,
  task=None, args=[], env={}, dry=False)` executes the addressed task at
  call time through the plane's own seam and RETURNS its § A.8 row as a
  value — state, exit code, stdout observable in-program; run-then-decide
  works. Refusal rows return as values too (branchable); only shape errors
  (wrong argument types) fault the program.
- **Principle (ZT, ruled verbatim):** *"the rev is a leash for the agent
  stale context, not a property of writes. A script reads at execution
  time — its own read is the freshest possible; guarding a millisecond gap
  means nothing. Effects cannot be refused (out-of-world), so the
  transaction promise is unkeepable there — no half-promises, no
  chimera."*
- **Accepted tradeoff ON RECORD, not a warning:** two effect-scripts can
  last-writer-wins each other on one section, same as two shell scripts;
  the engine write flock keeps files structurally intact; exclusivity
  belongs to the coordination layer.
- **`token_count(text)` (token_count ruling leg B, 2026-08-13)** answers
  the text's real token cost as an int, measured at call time through the
  harness endpoint the § A.7 frame binds (`token_count_endpoint`) — a
  socket call wearing a function; the engine never counts tokens. ONE
  measurement law: the string is measured verbatim (the tool face's
  `{text}` arm) — no ref resolution, so the tool face's stored-vs-served
  split cannot enter; compose with `read()` to measure served content. A
  lane with no endpoint faults "unbound"; the endpoint's refusal faults
  the program with its words carried whole; the dial deadline caps at the
  remaining wall clock. A measurement, not an act: no trace entry — a
  top-level binding echoes like any computed name.
- **No rollback.** A mid-program fault leaves every prior act landed; the
  trace records how far the program got. The outcome word for a completed
  live program is `effects` (the vocabulary's one addition); `fault` keeps
  its meaning on both models.
- **Replay refuses a live program.** Eval-as-pure-function holds for the
  pure model only; `replay_script` refuses an effects-mode context rather
  than forging a world that was live.
- **Budgets, and where a run's cost is charged (amended, dogfood r2 F8):**
  eval limits and the wall clock bind unchanged over the program's OWN acts;
  the read ceiling counts live reads identically; `put()`/`run()` are not
  fuel-metered. A live `run()` is ADMITTED under the script clock (the
  pre-dispatch check), and then **the clock stops while the run plane
  executes**: the plane's walks and its child are bounded by the plane's own
  budget — `run.timeout_secs` on the root's declaration, default 5m — and
  the run's elapsed is never charged to the caller's script clock. What the
  script clock prices is the program: its reads, its puts, its compute. The
  COUNT of runs is bounded by the kernel's run ceiling (`max_runs`,
  64/attempt): the 65th run refuses typed, naming the ceiling, and the runs
  already executed stand — a live program has no rollback.
- **Identity (§9):** `actor`/`now` thread as everywhere; run identity
  derives from the submission's host-minted `invocation` base
  (`<invocation>-r<K>`, K the 0-based call ordinal).

**Read alignment (2026-08-13, same ruling — BOTH models).** In-script
`read()` mirrors the read TOOL interface, and its results are VALUES, not
opaque structs (the Mathematica principle: *"read() returns actual VALUES
the agent computes with"*; resolves dogfood F-SC1/F-S1 by interface
alignment): `read(path)` answers the toc face as a DICT — `{"fm": {…},
"toc": […], "rev": "…", "words": N}` — and `read(path, section=…)` answers
the section TEXT as a plain string (`"x" in read(p, section=s)` is a
legal program; the section's rev still rides the recording, where the
threading law reads it). The `section` string speaks the read tool's own
selector grammar — heading path, dewey ordinal, `^anchor` — and the dewey
arm is now served (the prior in-script refusal is retired).

This is why **RAISING the budget is refused**: it is cost policy wearing a fix,
and since sections count, a raise is a treadmill that buys a different wrong
number rather than a stated domain.

**The snapshot guarantee is stated WITH its composition rule.** `script` pins one
entry fingerprint and `commit` guards on it, so the single-snapshot guarantee
holds **up to the budget above**. Above it the caller composes runs under a law
the caller can CHECK: **equal entry fingerprints across runs = one snapshot;
unequal = the world moved, re-run.** That converts the limitation into a
protocol and keeps the guarantee composable **without daemon-held state**. An
engine-HELD chunk-spanning snapshot is deliberately **not** ruled in — it is
daemon state across attempts for a need the dogfood has not shown. Revisit
trigger, named so it is not a matter of taste: **compose-retry livelock under
real churn in the field.**

A guarantee whose domain appears in no help text stops holding SILENTLY above a
boundary the caller cannot learn exists until crossing it — a contract claim
with an undocumented domain, not an inconvenient budget. That is why this
paragraph exists and why `mrd script --help` carries the number. **It was never
a missing help page; it was a missing sentence on a page that already existed.**

**The trace IS the read-only script's output channel, and that makes it
CONTRACT MATERIAL.** There is no `print()` — the builtin surface is closed, so a
script that only reads arms nothing and exits `no_effect`. **That outcome
reports that nothing was ARMED, never that nothing HAPPENED**, and the reads are
returned as `trace[]` rows of `{kind, line, path, face}` under `--json`. The
face-honesty law reaches this face from the opposite side of `links`: not a
subset withheld, but **a capability withheld** — the help said `--json` "emits
the trace" and never that the trace carries what you read, so the only
documented reading of `script` was that it is write-only. Every measurement in
the 2026-08-10 dogfood rode this echo; **an echo that load-bearing is contract
material, not an implementation detail**, which is the argument for documenting
it rather than a complaint about it.

**Seam, named and NOT taken here: the guarantee's crossing (face-honesty clause
5).** A face that hands content ACROSS the guarantee boundary owes a mark on the
crossing — content leaving accompanied by the provenance it was read under, so
the caller can RE-VERIFY rather than inherit a guarantee that silently
evaporated. The crossing is legitimate and constant; the silence at handover is
the defect. It is a separate card because it changes what the read face emits
ALONGSIDE content and interacts with the composition protocol above. Recorded
here as a seam so the next lane inherits a pointer rather than a gap; no part of
it is implemented, stubbed, or designed around in this change.

**`read(path)` IS the wire toc face, 1:1.** The recorded toc face is
`{rev, fm, toc, words}`. `fm` values are DECODED scalars — the frontmatter
scalar law (wire-contract § A.6) governs this plane exactly as it governs the
composed read's `props[].value`, so `owner: "[[x]]"` reaches a script as
`[[x]]` and a comparison against the unquoted form arms
*(amended 2026-08-07, dogfood-season-1 finding 1)*. `words` is the wire's own
`words_total` — a
delivered fact the host carries, never a count the consumer plane computes.
It names the FILE (fields over the whole document, `wc -w` parity), never the
sum of the section rows *(amended 2026-08-13, the counting law: wire-contract
§ A.3)*. A
script sees `t.words` for the same reason it sees `t.rev`: the wire answered
it. **Which op answers it:** the composed `read` (§4.1, toc mode) carries
`words_total`; the `toc` op's body is `{path, file_rev, root, nodes}` and
carries none, so a whole-file `read(path)` asks both — `toc` for the rev and
the section map, `read` for the count. Zero wire delta: both ops are already
declared, and a read mints nothing (reads are side-effect-free engine-wide
since pin proof moved onto the request, wire-contract § A.3), so the second
ask costs nothing but the call (ruling 2026-08-07, `words:` on the read face).

**A composed read is BRACKETED by `file_rev`, or it refuses.** A whole-file
`read(path)` is 2+N round trips — `toc`, one `cat` per frontmatter key, then the
closing `read` — and reads are LIVE, so the world may move between any two of
them. Composing one face from two revisions would hand the script a state that
never existed, and the stand-still guarantee is about exactly that. So the
`file_rev` the opening `toc` answered is compared against the `file_rev` the
closing `read` answers, and a difference **refuses the read** naming both revs.
The closing op is the count op deliberately: it already had to be asked, so the
bracket costs **no additional round trip**, and every intermediate `cat` sits
inside two agreeing observations of the same revision. Sequence order is
therefore load-bearing — the count is asked LAST, never second. A single
`read(path, section=…)` is one `cat` and needs no bracket: one op is one
revision by construction.

**What the bracket does NOT catch, stated beside the guarantee it bounds.** A
`file_rev` is content-derived, so an A→B→A sequence — a write and a
byte-identical restoration, both landing inside one composed read — closes the
bracket with two agreeing observations of A while the `cat` values in between
were taken at B. The face is then internally consistent with A and the values
came from a state that A also describes byte-for-byte, so nothing false is
published; what is lost is the ability to SAY that the file did not move. The
bracket is a revision-identity check, never a mutation counter, and the wire
offers no mutation counter to check instead. The commit is unaffected either
way: it carries the entry fingerprint and §5.1 checks it first, so a world that
moved and came back still refuses there if the fingerprint moved. Naming this
limit is the point — a guarantee stated without its known limit reads as a
stronger claim than the mechanism makes.

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

**One address grammar, one parser.** `section=` on `put()` is parsed by
`ReadSel::parse` — the same door `read(path, section=…)` goes through, and the
one human-string→selector door in the tree. The three spellings it decides are
therefore the same on both faces: `^id` is a block address, digits-and-dots is a
dewey ordinal, anything else splits on `/` into raw heading segments. An
`append` carries an **hpath**, so the two non-heading spellings **refuse at arm
time** naming what they are — the `^anchor` a toc row publishes is a real
address on the read face and a real refusal on the write face, never a heading
silently named `^r-…`. A face that parsed its own addresses would mint a second
grammar for the one thing both faces call `section=`.

**`section=` also takes the §2.1 segment array** *(dogfood r7 F1, card
script-slash-heading-addressing)*: a list of `{h, n?}` objects, one per
heading, raw text taken verbatim — the wire's own machine form, on both faces.
This is not a widening of the string coat (D-1: the coat splits on `/` and C2
stays reserved); it is the escape the engine's section-miss refusal already
taught, made real on the plane that prints it. A heading whose raw text
carries `/` rides one array entry, and the occurrence index `n` — which the
joined spelling cannot spell — rides the structured form only. The toc face
publishes each heading row's raw segments as `hpath` beside the joined
`section`, so any row feeds back into `section=` verbatim. Out-of-grammar
members refuse at the boundary in the D-1 line's first arm — a bare string in
the list is the retired v1 spelling and refuses with the wire's own
single-sourced text (v2 §2.1), and the type refusal names both accepted forms.

**The statement-position rule — echo and quiet.** Every read is recorded; the
face renders only the ones the reader wrote as a decision. A read **echoes**
exactly when its call is the whole right-hand side of a top-level assignment,
or a top-level expression statement — `card = read(…)` binds and echoes. Every
other position is **quiet**: comprehensions, `if` conditions, loop bodies,
function bodies. The kernel reads the positions off the parsed module, so the
rule is syntactic and stable, never a call-depth heuristic. Suppression syntax
does not exist in v1 (`_ = read(…)` is rejected permanently; `quiet()` waits on
elision-count evidence).

**The bindings echo — "bind it to a name to echo it", made true** (result-echo
ruling 2026-08-13, F-S1+F-S3). A successful evaluation's module top-level
bindings ride the trace as `bindings` — name → Starlark repr, name-ordered —
so every value the script computes is observable by binding it, Mathematica
style, and learning a face's fields costs a `dry` run instead of a committed
write. The capture law keeps one carrier per fact: the inert inputs stay out
(inputs are not results), function bindings stay out (a `def` is not a value
the run computed), and a name whose LAST assignment is a top-level
`name = read(…)` stays out — that value is the read's own `echo` entry. Any
later rebinding of such a name — reassignment, `+=`, a loop target, an
assignment inside an `if` or `for` body — returns it to the bindings, because
the name no longer holds what the echo carries. A failed or refused
evaluation carries no bindings: its namespace is not a result. Absence stays
absence — a run that bound nothing emits no `bindings` member at all.

**The grammar.** The script entry parses under the rule dialect plus top-level
statements — its module top level IS the program, so `if` and `for` at the top
level are the ordinary case there. A rule or a task must define a hook, so the
hooked planes keep the stricter grammar. `load` stays disabled at every entry.

**Where the budgets bind.** Fuel, memory, call depth, and source bytes are
`EvalLimits`, shared with the other two entries. The read ceiling (64 per
attempt) is the script plane's own — the I/O-amplification axis the hermetic
entries do not have — and the kernel enforces it: the read past the ceiling
refuses typed, naming the ceiling, and the attempt has no result at all. Never
truncation. The armed-edit ceiling (64 per attempt) is the kernel's too, and it
binds at **arm time**: `put()` refuses the call that would cross it, typed and
naming the ceiling, with no host involvement at all. The retry budget binds in
the host, above the kernel, because the loop is the host's.

**The wall clock binds at four layers, and every one of them is load-bearing.**
The entry does its I/O against a daemon, so time is the one budget evaluation
cannot bound itself: fuel bounds computation, and a blocked read spends none.

1. **Per round trip, above the socket.** The clock is checked before **every**
   wire call the host makes — not once per script-level read. A whole-file
   `read(path)` is 2+N round trips, so a per-read check would bound the number
   of checks and not the time.
2. **On the socket itself.** The connection carries read and write timeouts of
   one wall clock, so a daemon that accepts a frame and never answers fails the
   round trip instead of parking the process forever. Without it the check in
   (1) never runs again.
3. **Before the commit.** The commit leg is a wire call like any other and is
   checked like one: a run whose clock elapsed during evaluation refuses
   **pre-commit**, so nothing is issued and nothing lands.
4. **Above the process, in the MCP host.** The child is spawned under a bound of
   its own and killed by process group if it passes it. That layer exists for
   the failures the first three cannot see — a child that never reaches its own
   clock, or one that ignores it.

Layers 1–3 refuse in the entry's own vocabulary and answer a trace; layer 4 is
the backstop and answers a host refusal, never a face. A hung child that reached
none of them would hang the tool with nothing bounding it.

*(Amended 2026-08-12, the in-process lane.)* The four layers above price a
lane whose reads are round trips. On the in-process lane (wire `script`,
wire-contract § A.7) there are no round trips and no child, so the wall
clock binds at three sites, all daemon-enforced: at entry before the pass,
at every read builtin, and pre-commit. Fuel bounds computation between wall
checks; `catch_unwind` at the eval boundary bounds everything else — a
panic answers a `fault` trace and the daemon serves its next frame. The
socket timeouts and the MCP child bound stay on the lanes that have sockets
and children.

The host-side defaults are **§5.3 host policy** — their existence is contract,
their values are tunable:

| Budget | Default | Binds | Over it |
|---|---|---|---|
| wall clock | 7s | entry (per round trip, per socket, pre-commit) | typed refusal |
| child bound | 30s | MCP host (process group kill) | host refusal, no face |
| retries | 2 attempts | host | exhaustion → resync face |
| armed edits | 64 | kernel (arm time) | typed refusal |
| live runs / attempt | 64 | kernel (run admission, effects mode) | typed refusal; executed runs stand |
| reads / attempt | 64 | kernel | typed refusal, no result |
| selector width | 256 paths | host | typed refusal, **never truncation** |

The selector cap sits **above** the read ceiling on purpose: it bounds
ENUMERATION — host result size and fan-out width — while the read ceiling bounds
actual I/O, and a program is free to read only a few of the paths it was handed.
256 covers S2-class fan-out over a large board without letting a runaway glob
return the corpus.

**What an entry costs** *(added 2026-08-08)*. The table above bounds a program;
this states what the program spends against it, so a caller can compute its own
ceiling instead of discovering it as a refusal.

**The ceiling is a function of round trips, and reads are not trips.** For an
entry against a corpus of `C` domain members:

```
ceiling = f(reads, corpus)

wall clock  ≥  trips(R) × pass(C)

trips(R) = 3 + Σ per read: (2) whole-file
                           (1) sectioned

pass(C)  = O(C) in `stat`s, O(changed) in bytes      <- the linear term
```

**Both arguments matter, and the second one is linear.** `pass(C)` is not a
constant: a currency pass `stat`s every domain member, so it grows with the
corpus even though it no longer reads it. Measured on this engine, doubling the
corpus (23,758 → 47,477 members) multiplies the pass by **1.84×**, and with
trips minimised a program's per-read cost converges on the pass and moves with
it — **1.91×** measured end to end.

So a program that fits today does not automatically fit on a corpus twice the
size. What the trip collapse removed is the *amplification*: cost no longer
scales with reads × frontmatter × corpus, only with reads × corpus. Making
`pass(C)` itself constant would need the root maintained incrementally rather
than re-derived — an OS event watcher or a persistent index — which v1 does not
have and this contract does not promise.

`3` is the fixed frame — `hello`, `fingerprint`, and the commit. A whole-file
`read(path)` is **two** trips: the `toc` op, and the composed `read` (§4.1, toc
mode) that brackets it and carries `words_total` plus the frontmatter.

**The whole-file 2 / sectioned 1 split is confirmed by an unasked prediction**
*(added 2026-08-09)*. The split is not read off the code alone: the model
predicted a figure nobody requested. A sectioned read was expected to cost about
one trip where a whole-file read of the same page would cost two — ~122 ms
against ~235 ms — and the measurement landed on the predicted side. A whole-file
figure near the sectioned one would have falsified `trips(R)` outright. A
formula that predicts a number nobody asked it for is the strongest form that
evidence takes, and it is the reason this line is stated as law rather than as
an inspection of the dispatch code.

**Two is the floor, not one** *(amended 2026-08-08)*. The composed read alone
almost suffices — it already carries `file_rev`, the heading rows,
`words_total`, and `props[]` with every value decoded per § A.6. What it does
not carry is a rev for `^anchor` rows: `wire::ReadAnchor` is `{anchor, span}`
and `wire::ReadRow` has no anchor field, while the `toc` op's nodes publish an
anchor row with its own `node_rev`. Collapsing to one op would silently drop
anchor rows from the face a script sees, so the `toc` trip stays.

Until this amendment the frontmatter cost one `cat` **per key** on top, making a
whole-file read `2+N` trips — seven to ten for an ordinary session artifact, so
a program's ceiling was set by its pages' frontmatter rather than by its own
read count. The per-key fan-out was justified in-code by the composed read
having "no frontmatter plane"; that stopped being true when `props[]` was
added, and the fan-out outlived its reason.

**What a pass costs.** Every trip is answered from the warm engine, and the
engine is proved current first over the WHOLE hash domain: a read is
corpus-scoped, not file-scoped, because a poison member anywhere refuses a read
of a healthy one (Law A-3c). That scope is unchanged and is not an optimization
target. What changed is the price. The pass walks the domain reusing the listing
of any directory whose own timestamps did not move, `stat`s every member,
re-reads only the members whose stat identity moved, and folds the §12.2 tree
from per-member digests — O(corpus) in `stat`s, O(changed) in bytes.

It used to re-read and re-fold every domain byte on every trip. On a 24k-file,
150 MB corpus that is ~0.9s per trip, so a single script read spent several
seconds of the 7s budget and a two-read program did not fit at all. The budget
was never the defect.

**A stale engine pays a rebuild, and `pass(C)` never prices it** *(added
2026-08-08 — fable F8, merged-verdict.md § C7 disposition)*. The formula above
prices the read side on the assumption the resident engine already agrees
with disk — that agreement is exactly what the currency check tests, and the
check's other branch is not `pass(C)` scaled up, it is a different regime the
formula does not name.

**The trigger is any fingerprint move, including one the caller itself just
made.** A splice commits straight to disk and returns without touching the
resident engine at all — the engine finds out only on its own next currency
check, and that check treats a caller's own just-landed commit exactly like
any stranger's concurrent write. The clean case is a retry: the host's own
retry loop (budget 2, above) fires precisely because the fingerprint moved,
so the retried attempt's reads land on a stale engine by construction — a
write-bearing program's ceiling on its next trip is not bounded by the
formula above at all.

**What the rebuild costs, and why it does not compose with `pass(C)` by
scaling.** A currency miss re-reads every domain member unconditionally — the
leaf memo `pass(C)` consults is bypassed on this path, not merely widened —
and then re-parses every member to replace the resident index and document
map wholesale. There is no incremental engine update: one changed byte
anywhere in the corpus pays the same full re-read and full re-parse as a
rewritten corpus, because the rebuild does not know which byte moved, only
that the fingerprint disagrees.

```
rebuild(C)  =  O(corpus) in full reads  +  O(corpus) in full reparse,
               paid whole regardless of how much of C actually changed
```

A trip landing on a stale engine pays `rebuild(C)` in place of `pass(C)`, not
in addition to it — the ceiling formula above (`wall clock ≥ trips(R) ×
pass(C)`) holds only for a trip that lands on an already-current engine; the
write-bearing `commit(C)` term below composes with either.

**What is and is not measured here.** `rebuild(C)`'s re-read half is
`fs::domain_snapshot`, the unconditional-read arm `crates/fs/examples/domain_cost.rs`
exists to benchmark directly against a real root — but no run of it is
recorded in this tree, and its scaling is not assumed to match `pass(C)`'s
measured slope, which was measured against the leaf-memo path `pass(C)`
actually ships as. The reparse half (`fs::build_corpus`) carries no dedicated
benchmark at all. The only order-of-magnitude figure on record for the
combined read-plus-reparse rebuild is the cold-daemon-start case
(`node-rev-merkle-spec.md` § 0): 2.2–5.2s on a 50,319-node, 9.5 GB corpus
(M4 Max) — the same functions, a related path, not a controlled measurement
of this specific mid-session trigger.

Evidence: `Registry::warm_or_build` (`crates/registry/src/registry.rs`) is
the sole site of this branch — a fingerprint match returns `Reused` at
`pass(C)`'s cost, a mismatch calls `fs::domain_snapshot` (unconditional
full re-read) and `fs::build_corpus` (`syntax::parse` + `model::build` over
every member), and no partial-rebuild path exists. `Op::Splice`
(`crates/registry/src/server.rs`) writes disk and returns without touching
the resident engine; its own comment states the consequence: the warm engine
rebuilds on next read, because the fingerprint moved.

**A write-bearing program pays a byte term the pass never does** *(amended
2026-08-08 — the review battery's cross-arm cost-model finding)*. The formula
above prices the read side; taken alone as the computable ceiling it is a
lower bound, because the commit's §5.1 world guard and the seam roots fold
**from bytes** under the write flock (`domain_snapshot` — the digest memo
never supplies them; that is what keeps the commit guard byte-derived).
Priced:

```
wall clock  ≥  trips(R) × pass(C)  +  commit(C)

commit(C) = 0 for a read-only program
            two byte-folds, O(C) in BYTES, for a write-bearing one
```

The folds are O(corpus **bytes**), not `pass(C)`: **on the 24k-file, 150 MB
corpus this section prices throughout**, they cost ~1.8 s together — more than
the whole fixed frame. A read-only program's `commit(C)` is zero and its ceiling
is the first term alone; a write-bearing program budgets both. With every
engine-side spend a named term, the `≥` that remains is measurement honesty —
the OS may always be slower — never an unpriced structural cost.

**The live-corpus term, measured** *(added 2026-08-09)*. Against the live
corpus, five trials, medians:

```
commit(C)  ≈  4.46 s   process wall, live corpus, 5-trial medians
```

That figure is **process wall as an operator meets it**, and the door seat that
produced it fenced it in exactly those terms — *"it prices the commit as an
operator meets it, not as the model decomposes it"*. Read it with the fence
attached:

- The two byte-folds are **not decomposed** — the number is their sum plus
  whatever else the commit path spends.
- The daemon round trip is **not separated** from the splice itself.
- **Corpus size was not varied**, so this term carries no slope of its own.

**No regression claim attaches to the gap between ~1.8 s and ~4.46 s.** The two
figures were never verified to measure the same shape — one is an engine-side
fold pair on a stated corpus, the other an operator-side process wall on the
live one — so the difference is not evidence of anything having become slower.
A decomposition run would settle it. That run is **not owed for v1**; it opens
only if v1 testing hits the term.

**The linear term has a measured slope, and the slope is the honest headline**
*(amended 2026-08-08 — the memo design's own disclosure, the ratios above,
independently confirmed by the review battery's three-point curve)*. Per-read
cost multiplies by **~2.1× per root doubling** as independently measured (the
design disclosed 1.84× pass / 1.91× end-to-end before anyone re-measured; the
pre-memo engine's slope was 2.35×, so the memo bought about one root
doubling — a constant, not a change of shape). The consequence
stated rather than implied: a 10-read program clearing ~3.8 s at a 2× root
returns to roughly ~7.8 s at 4× on the same slope — over the budget again.
Capacity planning must read `ceiling = f(reads, corpus)` WITH that slope;
a flattened impression is exactly what this section exists to prevent.

**The in-process lane's cost shape** *(added 2026-08-12, the entry-world
ruling)*. Every formula above prices the wire-client lane, where each trip
pays `pass(C)`. On the in-process lane (wire `script`, § A.7) the pass runs
once, at entry, and reads serve from the pinned entry state:

```
wall clock  ≥  entry_pass(C)  +  Σ reads × O(1)  +  commit(C)
               one pass, at entry   memory-speed     unchanged
```

`entry_pass(C)` keeps `pass(C)`'s shape — O(corpus) in `stat`s, O(changed)
in bytes — and the linear term is paid ONCE per attempt instead of once per
trip, so a program's per-read cost no longer moves with corpus size at all.
The slope above still governs the entry term and the commit term; what the
lane removes is the multiplication by `trips(R)`. A stale engine still pays
`rebuild(C)` at entry in place of `entry_pass(C)`, exactly as above.
Measured figures for this lane ride the change that lands it, in its
delivery record — this section states the shape.

**The bash bracket's observations, unified onto the resident memo** *(added
2026-08-15, card run-observation-unification — engine-warm-cost design § 5)*.
A bash dispatch observes the corpus three times — the pre-flock leaves fold,
the bracket open, the bracket close — and each observation used to run its own
fresh walk: full `read_dir` enumeration, full stat sweep, byte reads amortised
by the per-workspace drawer memo (`run-digests.v1`, F8). The observation
source is now injected (`RunSpec.observations`). The CLI lane keeps exactly
that instrument — a separate process has no resident memo in reach. When the
door is the daemon (the § A.8 `run` op and the § A.7 in-script `run()`), the
observations serve from the registry's resident `fs::DomainCache` — the
dir-listing memo plus the leaf memo every currency pass and warm rebuild
already run on — locked per observation, never across the exec window, with
no drawer I/O at all. The verdicts are lane-independent by gate
(`crates/fs/tests/cached_observation.rs`: same folds, same residual deltas,
same symlink refusals — including from remembered listings). What the
daemon lane stops paying is the enumeration and the drawer serialization; the
stat sweep stays, deliberately — the walk and the stats are live, that is
what an observation IS. Measured on a 29.5 k-doc synthetic corpus (hermetic,
`crates/fs/examples/run_observation_cost.rs`): the three-observation trio
went from ~288–305 ms (drawer) to ~245–248 ms (resident) median, with
enumerations per warm trio dropping ~888 → 0 and byte reads identical at
movers-only; a run now also leaves the shared memo warm for the next op's
currency pass, and vice versa.

**The transaction — stand-still optimistic.** The word **snapshot is
banned** here: the daemon has no MVCC and v1 must not grow one.

1. **Entry.** One `fingerprint` call (§4.7) pins the **entry fingerprint**.
2. **Reads are LIVE.** If the world moves mid-script, reads may span states.
3. **Commit.** ONE splice batch carrying `if_fingerprint` = the entry
   fingerprint, and §5.1 checks that guard **first**.
4. **Any interleaved write** ⇒ `fingerprint_mismatch` ⇒ nothing commits.

**Reads stay live, and stay corpus-scoped** *(amended 2026-08-08)*. The cost
model above changes what a currency pass COSTS and never what it CHECKS. Every
read op still proves the whole hash domain current before it answers, so a
poison member anywhere still refuses a read of a healthy member and still names
the poison (Law A-3c). Nothing is served out of a picture taken earlier, no
staleness window is introduced, and no version is retained — the banned
snapshot stays banned, and point 2 above is untouched.

The one thing the pass now takes on trust is that a file whose
`(device, inode, size, mtime, ctime)` is unchanged has unchanged bytes, and
that a directory whose own timestamps are unchanged has the same entries —
which is what a directory's timestamps mean. `ctime` is what puts the first out
of reach in practice: the kernel bumps it on every inode change and no API sets
it, so even a deliberate `utimes` restore is caught. This is the standing
`fs::domain_stat_signature` posture — evidence, not proof — and it is bounded
underneath: the commit's §5.1 guard folds **from bytes** under the write flock,
so a memo that ever disagreed with disk makes the commit refuse
`fingerprint_mismatch` rather than land. It fails closed, in the vocabulary the
transaction already speaks.

**Amendment to § The script entry (the entry world — the in-process lane,
ruled 2026-08-12).** The transaction above states, point 2: *"Reads are
LIVE. If the world moves mid-script, reads may span states."* That point is
SUPERSEDED on the in-process lane — the wire `script` op
(wire-contract § A.7), where the daemon itself evaluates the program. The
subprocess lane (`mrd script`, wire-client mode) keeps the live-reads law
above unchanged. Four laws replace point 2 on the in-process lane, and only
these four:

1. **One pass, at entry.** The currency pass keeps its corpus-grain scope —
   the whole hash domain proves current, and a poison member anywhere
   refuses the ENTRY naming the poison (Law A-3c unchanged in scope, moved
   in time). What changed is WHEN, never WHAT: no doc-grain narrowing, no
   staleness window at entry, and no watcher — the pass is re-derived per
   attempt, never maintained incrementally.
2. **Reads serve the entry world, plus your own arms.** A read of a target
   the program has not armed serves the entry bytes and the entry rev. A
   read of a target the program ITSELF armed serves the ARMED content — the
   entry bytes with the program's own armed edits applied, in arm order —
   and that content's own rev: what you read is exactly what is hashed
   (wire-contract §4.2), on the overlay too. Foreign mid-program changes
   are invisible; the reads of one attempt span ONE state by construction.
   The ONE state is the hash domain's: an out-of-domain path
   (wire-contract §12.1) stays addressable on this lane too and serves
   from a live single-file disk load — the stand-still guarantee is the
   fingerprint's surface and nothing wider, exactly as the entry
   fingerprint never covered those bytes.
   Recorded-read purity is unmoved — every read, entry-served or
   overlay-served, is recorded, and eval stays a pure function of
   (script, args, files, read-response sequence).
3. **Disk changes only at commit, and the commit guards the LIVE world.**
   §5.1 is untouched: ONE splice carrying `if_fingerprint` = the entry
   fingerprint, checked first, against the world as it is NOW. Any
   interleaved foreign write ⇒ `fingerprint_mismatch` ⇒ nothing commits.
   The guarantee strengthens: *a committed script read and wrote exactly
   one workspace fingerprint — and an uncommitted one still read exactly
   one.*
4. **This is not the banned snapshot.** The ban above is on daemon-held
   MVCC — versions retained across attempts. The entry world is
   attempt-scoped: born at entry, dropped at the answer, never retained,
   never shared across connections, no as-of parameter. Zero daemon state
   survives the attempt.

**Rev threading under the entry world (the entry-rev law).** *(Amended
2026-08-13, the CAS relaxation — the license clause is dissolved.)* Every
rev-less row threads the target's ENTRY rev, unconditionally: the file rev
for a `props=` row, the section's node rev for an `append`, read off the
pinned entry state. No recording gates it — the read ritual is gone, and
the recording is a trace fact only. An overlay rev is never a CAS token —
the pre-batch state the §4.4 guards resolve against is the entry state,
and threading consults only the entry toc, so a token naming bytes no disk
ever carried cannot be minted. A target the entry state cannot name (an
absent section) threads nothing and meets the engine's own target-class
refusal. Behavioural parity with the wire-client lane holds for every
program: read-then-arm, arm-then-read, and never-read all commit on an
unmoved world, and a moved world refuses whole on both lanes — at the
commit's own guard, which is where consistency enforcement lives.

**The bracket is structurally satisfied on this lane.** A composed read is
bracketed by `file_rev` because its 2+N trips could span states; in-process
there are no trips and one state, so the bracket's purpose is met by
construction and the A→B→A limit disappears with the window that created
it. The bracket law itself stays, for the lane that has trips.

A caller may also pin its own `if_fingerprint?` guard. It is checked against
the minted entry fingerprint **pre-eval** — mismatch refuses immediately with
zero evaluation, read-class, and the run is `attempts:1` by construction. That
pre-eval check is a fast-fail courtesy, **not** the authoritative one: the
commit's splice still carries the guard and §5.1 still checks it first, which
is what catches a world that moves *during* eval. Two checks, one value.

*(Amended 2026-08-16 — the malformed arm; dogfood break #7, script door.)*
Before the compare, the pin passes §5.7's grammar wall: a value that is not
a grammatical `Root`-family token — the reserved `absent` included, which is
§5.6 premise vocabulary (`guards[]`), never an entry pin — refuses as a
REFUSED trace (recovery `fix`) with the raw bytes debug-quoted, so invisible
damage (one leading space, the measured case) shows as a byte. Comparing a
damaged spelling instead would answer `conflict` with an expected/live pair
that can render character-identical and teach a re-read that loops. Both
lanes — this CLI entry and the wire `script` op — refuse identically
(wire-contract § A.7).

The guarantee, stated exactly: *a committed script is consistent with exactly
one workspace fingerprint — the world stood still, or the commit refused.*

**Amendment (2026-08-15, fingerprint-grain plan §4.6 — the commit premise
narrows to the TOUCH SET; frozen view KEPT, pre-merge ruling 2).**
Wire-contract § A.7 carries the full law; what it supersedes HERE, named:

- Point 3 above, the pre-eval caller-guard paragraph, the set-form commit
  below, the 2026-08-13 CAS-relaxation's premise restatements
  ("auto-guarded by the entry-fingerprint snapshot", "consistent with
  exactly one workspace fingerprint"), and — named and retired, bounce-2
  closure 2026-08-15 — the execution-model seam's commit leg below (the
  arm→commit blockquote's `--if-fingerprint <the arm's entry fingerprint>`
  pin and its "any movement of the world between the two refuses" claim,
  both rewritten in place) plus the seam table's Concurrency cell
  ("commit `if_fingerprint` = entry", rewritten in place), and — named and
  DELETED, round-3 closure 2026-08-15, ZT ruling: the touch-set law covers
  ALL script lanes (S1), same product as MCP `script` — the per-row-guard
  paragraph's "not peers / enforcement point" close (wire-client mode),
  which still made the entry `if_fingerprint` the commit's enforcement
  point — are AMENDED at
  the premise: the commit's authority is no
  longer the whole-corpus entry fingerprint — it is the **touch set** the
  attempt itself recorded (point reads, armed targets, pattern/selector
  expansions as set folds, sql provenance regions), verified entry-vs-live
  at exactly those nodes, O(touch set). A foreign write OUTSIDE the touch
  set no longer refuses the commit; a foreign write INSIDE it refuses
  exactly as before — `fingerprint_mismatch` naming the moved premise's
  scope (wire-contract §5.7).
- The guarantee restates at full strength on the premise's own surface:
  *a committed script is consistent with exactly one state of everything
  it touched — what it read and what it wrote stood still, or the commit
  refused.* Frozen-view reads (points 1, 2 and 4 of the entry-world
  amendment) are UNTOUCHED — ruling 2 keeps the A.7 read-stability
  promise word for word, and existing tests keep their meaning.
- The caller's own `if_fingerprint` — pre-eval fast-fail and commit check
  alike — stays legal as a WIDENING premise: strictest wins, never
  sufficient alone, never able to drop write coverage (the touch-set
  floor always contains the armed writes). The host-policy ratchet that
  forced the token copy onto script doors (`require_if_fingerprint`) is
  RETIRED (R3, `decisions/2026-08-15-plan-rulings-final.md`).
- The single-attempt law and the host retry budget are unchanged; retries
  now spend only on genuine same-subtree contention, because foreign
  churn outside the touch set never refuses at all.

The entry itself is **single-attempt**. A conflict at the entry is one
`fingerprint_mismatch` with recovery `resync`; the retry loop belongs to the
host (budget 2), which re-resolves a selector per attempt and re-runs pinned
`files[]` as-pinned. `attempts:N` is therefore a host fact, stamped on the
composed face, never a field of the entry's own trace.

**One COMMIT per attempt (the set-form law — replaces "One CONTENT path per
commit", ruled 2026-08-14).** The armed list may span N content paths: an
effect-less script's entire output is a finite armed list, known in full
before any I/O, so the commit can validate the WHOLE set against the world
before the first byte moves. One armed path commits as the single §4.4
splice, byte-identical to before; N paths commit as the §4.4 SET form
(`splice.set`) — per-path plan groups in first-arm order, one sealed
validate-all-then-apply commit under the entry fingerprint, one receipt
entry naming every file, one Delta, one fingerprint advance. All-or-nothing
holds by measurement rather than by fencing: a refusal anywhere in the set
lands nothing (the §5.2 diagnosis even sharpens inside a set — the world
guard passed, so a `no_match` on file k is provably the program's own text,
never a moved world). The receipt companion rides the same sealed set
(§6.1), as it always did. The arm-time `multi_file_write_set` refusal and
the single-path commit door are retired in the same change that made the
set commit real — the old rule was the fence while the machinery did not
exist; the crash story is §6.5's set paragraph (in-memory rollback, no
journal, stated windows). Effects mode is untouched: write-one was never
its law, and each live `put()` stays one single-path splice.

**Wire-client mode.** When a daemon is resident the script entry does its I/O
**as a wire client through the one door**: reads lower to `toc`/`cat`, and the
commit lowers to ONE guarded `splice` carrying `actor`/`now`/`receipt`. §4.4
is untouched — splice remains the only write op and the script executor is
just another client — and the daemon still carries no run-plane type and no
run-plane state. The whole wire cost of this entry is **zero schema delta**.

*(Amended 2026-08-12.)* The paragraph above is the CLI lane (`mrd script`),
unchanged. The entry's SECOND lane is the wire `script` op
(wire-contract § A.7): the caller submits the program in one frame, the
daemon evaluates it in-process against the entry world, and the commit is
the SAME one guarded splice, issued daemon-side through the same write
choke-point with `actor`/`now`/`receipt` threaded verbatim. §4.4 stays
untouched on this lane too — splice remains the only write op; the `script`
op ARMS one and embeds its response in the trace. One schema delta exists
and it is the § A.7 op itself, additive. A daemon-side commit advances the
delta ring like any wire splice — the CLI lane's missing-delta gap
(wire-contract §18 row 12) does not extend to this lane.

**The commit is guarded per row, and the consumer plane supplies the tokens.**
A wire door demands a fingerprint for every edit that changes existing content,
or an explicit `force` (`wire-serve::guard`), and the two grains differ: a
`set_property` row takes the **file** rev, because frontmatter semantics are
file-scoped, and an `append` row takes the **node** rev of the section it lands
in. The consumer plane threads each row's token itself — out of the recording
when the script's own reads cover the target (the LAST read of that target,
since this lane's reads are live), and from ONE bare `toc` trip per armed path
when they do not: the same host autofill the `put` face performs, spoken by
this lane at commit time. A mint the daemon refuses leaves the row untokened
and the engine's own guard answers — degrade is loud, never a guessed token.

**Amendment to § The script entry (the write-follows-read law — DISSOLVED,
2026-08-13).** The paragraph above originally created a behavioural law: *a
`put()` row's target must have been READ this attempt, or the row carries no
token and the wire door refuses the whole batch* — the consumer plane declined
to mint, and an unread target met `guard_required` (the Advisor's 2026-08-07
golden-v8 ruling pointed the same way). ZT's CAS-relaxation ruling
(2026-08-13, dogfood F-S2) supersedes both: **appends go rev-free for the
author** (put parity — append cannot clobber), **destructive rows are
auto-guarded by the entry-fingerprint snapshot the engine already holds**, and
**consistency enforcement lives at COMMIT** — the world-moved refusal — never
as a read-the-section-first ritual on the author. What remains of the old law
is its per-attempt consistency guarantee, which the entry fingerprint carries
alone: a committed script is consistent with exactly one workspace
fingerprint, or the commit refused. `force` is still not a script-plane door,
and the wire guard itself is unchanged — one token law for every door; the
lanes satisfy it for the author.

Evidence, and where it is held: `crates/mrd/tests/script_golden_live.rs` runs
every golden scenario (`inbox/run-golden.html` v9) through the real entry against
a **live daemon** and asserts that every `plan_edits[]` row on the socket carries
a token its own reads published — the conforming half. The relaxation's half is
pinned by the same suite's unread-target scenario (the lane mints the token, the
engine accepts the batch) and, engine-side, by `crates/registry/tests/script_op.rs`
(props and append with zero reads commit on an unmoved world) beside the
module-grain moved-world pin (a foreign edit after entry refuses the commit).

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

**Amendment to § The script entry (the execution-model seam: arm, then commit).**
The seam table's Authority row above already names this entry's authority as *"the
caller's own identity — `actor` threaded per §9; **ownership guard + armed law**;
no cap grammar."* The ownership guard is the HOST's organ — the engine's splice is
caller-agnostic by §5.3 and enforces no ownership — and a host cannot gate a write
set it has not seen. `put(path)` takes an arbitrary path, computed inside the
Starlark source, so the write set does not exist until evaluation has run. The
paragraphs above describe a single call that evaluates and commits in one child,
which leaves no point at which the host can read the armed set. This amendment
states the execution model that gives it one:

> **The MCP host runs the entry TWICE per attempt: once as an ARM (`--dry`),
> then, if and only if its own write-authorization plane admits every armed row,
> once as a COMMIT carrying `--expect-armed <the arm's armed_digest>`.**
> *(Commit leg amended 2026-08-15, bounce-2 closure — the touch-set amendment
> above, fingerprint-grain plan §4.6: the commit's world premise is the
> engine-computed touch set, verified entry-vs-live; the former
> `--if-fingerprint <the arm's entry fingerprint>` pin is retired as premise —
> a host-passed token stays legal as WIDENING only (R3,
> `decisions/2026-08-15-plan-rulings-final.md`). `--expect-armed` proves set
> identity; the touch-set verify proves set freshness.)*

Four things follow, and only these four. First, **this is a consumer-plane
sequencing law, and the wire contract carries zero delta** — the split is two
ordinary invocations of the entry, and the ops on the socket are the same five.
The CLI surface is NOT untouched, and saying so was the gap the sub-amendment
below closes: the commit child gains `--expect-armed`, which is consumer-plane
too and changes no request shape. Second, the split
is **safe by construction, never by being fast** *(amended 2026-08-15 with the
commit leg)*: the commit verifies its touch set entry-vs-live, so movement of
the world INSIDE the touch set between the two legs refuses at §5.1 as an
ordinary `fingerprint_mismatch` naming the moved premise's scope, which the
host's retry budget already handles; movement OUTSIDE the touch set never
refuses at all (plan §4.6 — foreign churn stops causing retries). Correctness
never depends on the gap being small. Third,
**recorded-read purity is what makes the arm's set the commit's set**: eval is a
pure function of (script, args, files, read-response sequence), and an unmoved
touch set means an unmoved read-response sequence — every read is itself a
touch-set member — so the two evaluations arm identically; a between-legs move
that DOES change what the commit child arms is caught pre-splice by
`--expect-armed` (sub-amendment below). The arm is therefore OUTPUT, never a
second decision. Fourth, the
gate is **parity with `put`, not a second policy grammar** — the same organs
(`checkPutAuthz`, `checkContentWrite`), the same per-target flock held across the
commit child, and the same journal pipeline. A script commit that took no flock
and wrote no audit line was the broadest-reach write face in the host having
neither. The **birth gate is not among them, and that is measured**: a `put()` to
a path carrying no file is refused by the engine at ARM time (`file_not_found` on
the rehearsal splice), so the trace is a terminal before any row is classified and
no host birth decision exists to make. The day this entry gains a birth door, the
third organ gets its call site.

The CLI entry keeps its single-call shape: an operator running `mrd script`
directly evaluates and commits in one process, because there is no host identity
plane in that path to gate against. The law binds the MCP `script` tool.

**Sub-amendment (the armed-set expectation, `--expect-armed`).** The amendment
above gates the ARM's rows and then runs a SECOND child to commit. It states why
the two evaluations arm identically — recorded-read purity plus an unmoved
fingerprint — and then *relies* on that reasoning holding. Reasoning is not
measurement. Nothing in the sequence above compares what the commit child armed
against what the host actually gated, so every link in that chain (the realpath
the authorization was decided on, the addressable-vs-hash-domain gap) is load
bearing and unverified. This sub-amendment makes the chain not matter:

> **The commit child accepts `--expect-armed <digest>` and REFUSES BEFORE THE
> SPLICE IS ISSUED when its own armed set does not hash to that digest.** The
> refusal is pre-splice: nothing is sent, nothing lands, no fingerprint advances.

Five things follow, and only these five.

**First — the digest is defined ONCE, engine-side, and this is its whole
definition.** Let `rows` be the armed set the commit splice would carry: the
armed rows *after* rev threading, in arm order, each one an object

```
{"edit": <the plan_edits[] item>, "path": <the file it writes>}
```

whose `edit` halves are byte-for-byte the value of the request's `plan_edits`
field and whose `path` halves are the request's `path`. The digest is

> `armed-set-path-edit:` ‖ `sha256:` ‖ lowercase-hex( SHA-256( CANON(`rows`) ) )

where `CANON` is compact JSON with **object keys sorted lexicographically by
UTF-8 byte order**, no whitespace between tokens, and RFC 8259-minimal string
escaping — only `"`, `\`, and the control characters below `U+0020` are escaped;
every other code point is emitted as raw UTF-8. There is no second spelling of
this anywhere in the tree: `effects::digest::armed_digest` is the only function
that computes it *(module home moved from `mrd::script::digest` on 2026-08-12 so
the in-process lane reaches the same call — one function, now three callers)*,
and the arm, the commit, and the § A.7 op all reach it through that one call.

**Why the path is in the domain, and not only the payload.** A `PlanEdit`
carries no path — the target rides `splice.path` — so a digest over
`plan_edits[]` alone is a total function of the armed **payloads**, not of the
armed **set**: two sets writing identical edits to *different files* hash
identically. That is exactly the dimension the arm/commit gap turns on. A host
gates the rows of one file; a commit child that resolved somewhere else (a
symlink re-pointed between the legs, or a pin covering the hash domain while the
write plane resolves through the larger addressable set) produces a **matching**
digest and splices into a file nobody authorized. Pairing each row with its
target closes that dimension by construction rather than by argument.

**The `armed-set-path-edit:` prefix is the digest's DOMAIN TAG, and it is a
deployment organ.** A host cannot tell a narrow digest from a wide one by looking
at it — an engine hashing payloads only publishes a perfectly well-formed value,
both children agree, the guard passes, and the class claim above degrades to one
that holds only on a pinned tree. Host/engine skew is measured rather than
theoretical: a resident daemon ran a stale engine for hours on 2026-08-06. So the
digest names what it covers, and a host asserts that **literal prefix with a
string comparison and no parsing whatsoever**, refusing an engine below the
minimum BY NAME. That is a capability assertion, not a canonicalization — the
courier property in *Second* survives it intact, because the host still copies
one opaque string and computes nothing. The tag names the DOMAIN rather than a
version number, so a refusal can say what is missing; widening the domain again
means a new tag, and every host still asserting the old one refuses loudly
instead of gating the wrong thing quietly.

**Second — the host is a COURIER, not a second implementation.** The arm's trace
publishes the digest as a top-level `armed_digest` field. The host copies that
string into the commit child's `--expect-armed` and never canonicalizes anything
itself. This is the load-bearing property: a host that re-serialized the trace's
armed rows would be a second canonicalization, and two canonicalizations give
either a refusal on every call or — far worse — a **vacuous pass**, a comparison
that agrees because both sides computed something equally wrong. The digest is
computed twice by the *same Rust function* over the *same type*, once per child.
A courier cannot invent a disagreement.

**Third — the serialization is published anyway**, precisely so that a host which
someday wants to verify independently lands on the same bytes instead of guessing.
Three traps are named because each one produces a silent false refusal on ordinary
markdown rather than on a test fixture: a Go implementation MUST disable
`SetEscapeHTML` (Go escapes `<`, `>`, `&` by default and the engine does not), MUST
NOT escape `U+2028`/`U+2029` (Go's marshaller does so unconditionally), and MUST
decode with `UseNumber` so `HpathSeg.n` never round-trips through a float. `CANON`
is otherwise exactly RFC 8785 (JCS) over a value whose only number is that `n`.

The **test vector** an independent implementation checks itself against, before
trusting itself, is pinned in `digest.rs::the_published_test_vector_holds` so
these bytes and this document cannot drift apart. For the two-row armed set

- `set_property{key:"owner", value:"8ab41c02", rev:"7c40e1a8b2f9d356"}` at
  `cards/one.md`, then
- `append{hpath:[{h:"Goals", n:2}], body:"a <b> & c\n", rev:"a6665baff294bd04"}`
  at `cards/two.md`

`CANON` is exactly

```json
[{"edit":{"set_property":{"key":"owner","rev":"7c40e1a8b2f9d356","value":"8ab41c02"}},"path":"cards/one.md"},{"edit":{"append":{"body":"a <b> & c\n","hpath":[{"h":"Goals","n":2}],"rev":"a6665baff294bd04"}},"path":"cards/two.md"}]
```

and the digest is
`armed-set-path-edit:sha256:37c4d09eb84d1e902b887a0b13cc90f67d5888e0bd5ebf9148ac0031ccdcde4a`.

Four things in that line are deliberate. The key order at **both** levels is
lexicographic, not the declaration order the Rust types use — `edit` before
`path`, and inside the edit `body` before `hpath` before `rev`. The `<`, `>` and
`&` ride raw. `HpathSeg.n` is **present**, because it is the one field named as a
trap above and no earlier vector reached it, so an implementation that dropped or
floated it passed every published check. And the two rows target **different
paths**, which the armed law does not permit in a single commit: that is on
purpose, because a vector whose rows shared a path would be reproducible by an
implementation that hashes the target once for the whole set and then diverges on
something no published bytes could catch. An implementation that reproduces this
line reproduces every digest; one that does not would have refused real markdown
while passing an ASCII fixture.

**Fourth — the receipt is NOT an armed row, so the digest excludes it, and the
exclusion is structural rather than a rule to remember.** The receipt rides
`request.receipt`, never `eval.armed`; it is not a member of `plan_edits[]` and so
it is outside `CANON`'s input by construction. The armed-set comparison therefore
says nothing about the receipt, and must not be read as covering it — the receipt
births a file under its own pre-spawn gate (the host's `receiptpolicy` leg), which
is a different door with a different organ. A reader who assumed `--expect-armed`
covered the receipt would believe a write was gated that this flag never sees.

**Fifth — the flag is optional and the CLI entry is unaffected.** Absent
`--expect-armed`, the entry behaves exactly as before, so an operator's direct
`mrd script` is unchanged. Present, it is checked after rev threading and before
the commit is issued, alongside the wall-clock's own pre-commit refusal — the same
position, the same "nothing was sent" guarantee. A refusal is `refused` with fault
class `refused`, not `conflict`: a mismatched armed set is not the world moving.

Evidence, and where it is held: `crates/mrd/tests/script_expect_armed.rs` drives
the real entry through a recording door and asserts both directions — a matching
digest commits, and a planted mismatch produces a socket census containing
`hello`/`fingerprint`/`toc`/`cat` and **no `splice` frame at all**, which is what
makes the refusal pre-splice rather than a detection after landing.

The target dimension and the tag are held there too, and each has both arms,
because a gate proven only to refuse cannot be told apart from one that refuses
everything:

| Claim | Arm | Test |
|---|---|---|
| The digest reads the target | refuse | `identical_edits_to_two_targets_publish_different_digests` |
| | admit | `the_same_target_publishes_one_digest_across_runs` |
| The tag does not break the tool | admit | `an_ordinary_commit_still_commits_on_the_tagged_engine` — arm, forward verbatim, commit, and the census asserts the splice WAS issued |
| The tag is assertable | refuse | `digest.rs::an_untagged_digest_is_distinguishable_from_this_engines` |

The first pair is also the **wire-observable capability probe**: a caller holding
nothing but the entry and a door establishes that this engine's digest covers the
target by running the same edits at two paths and comparing the published values.
The capability is observed, never inferred from a version constant.

The tag cannot false-refuse an ordinary commit, and that is structural rather
than tested-for: it is prepended inside `armed_digest` itself, so the value the
arm publishes and the value the commit recomputes are the same call over the same
type. There is no side on which it could be stripped or re-added.

`cmd.rs`'s own tests pin the rev-threading law the target dimension leans on:
`guarded()` looks a row's CAS token up **by that row's own `arm.path`**, so a
child that resolved elsewhere cannot inherit the gated file's rev. That was an
unstated accident until it was pinned, and an unstated accident either becomes
law or becomes a regression.

**The trace — one commit-fact shape, and no `attempts`.** The entry returns a
`ScriptTrace`: the entry fingerprint, the outcome
(`committed | no_effect | conflict | fault | refused`), the decision trace, an
optional commit leg, an optional fault, the top-level `bindings`, and
telemetry. Three laws hold it together:

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

**A refusal carries the wire's refusal triple, TYPED** (docs-first, 2026-08-10).
The fault of a `refused` run carries `code`, `recovery` and `reason` — the same
triple the §8 error frame carries — and `recovery` is `wire::Recovery`, the
closed six-class enum, taken from the **one** source the wire field's vocabulary
comes from. Five clauses hold it:

- **Never a fifth fault class.** Transient-vs-permanent is a PROPERTY of a
  refusal, not a KIND of fault. A `transient` variant beside
  `parse | runtime | budget | refused` would conflate two axes and silently
  break every consumer that matches `refused`.
- **Prose is a rendering, never the carrier.** `reason` keeps the engine's own
  wording verbatim and the face keeps rendering it; a consumer that needs the
  class reads the class. A downstream that pins a refusal's SPELLING is the
  name-promise gap's manufacture channel, and this shape is what retires it.
- **One source, with a stated precedence.** The daemon's own `error.recovery`
  wins; when a frame carries none, the class is the §8 frozen table's binding
  for its `code` (`ErrorCode::recovery()`) — the same table, never a second copy
  of it. A code the engine cannot parse with no `recovery` beside it yields
  absence, and absence stays absence.
- **Engine-minted refusals name their class explicitly**, because no frame
  minted one for them: an `expect_armed_mismatch` is `fix` (the armed set is not
  the one authorized — re-arming is the caller's act), and an elapsed wall clock
  before the commit is `retry` (nothing was sent, so the same request may
  succeed). They carry no `code`: no wire code was minted, and inventing one
  would put a value on the §8 surface that no daemon can answer with.
- **The migration is ADDITIVE.** `code` and `recovery` are optional and omitted
  when absent, so a consumer matching `outcome: refused` plus `fault.reason` is
  byte-unaffected by a frame that carries neither.

Why the class must cross here and not above: the engine KNOWS the refusal is
transient — the daemon frame carries `recovery` first-class and the put door
reads it — while the script path flattened it into `format!("{code}: {message}")`
and lost it. **No host-side change can recover a class the engine destroyed
before the boundary**; a face left with prose can only match strings. One engine,
one refusal vocabulary: a door that reads it and a door that destroys it is the
asymmetry this closes.

**A controlled failure exit SPEAKS** (docs-first, 2026-08-10). The clause above
gives a refusal that reaches `CommitLeg` a typed class. A run can fail without
ever reaching one, and until now those exits left through `mrd::run`'s `Err(Fail)`
arm with prose on stderr and **nothing at all on stdout**. That is the same
disease one door over: a consumer saw a nonzero exit and an absent trace, and
could not tell a deliberate, fully-understood refusal from a process killed
mid-write. The two need different remedies — one is the caller's to fix, the
other must never be resent — so a surface that cannot separate them is not an
inconvenience, it is a correctness hole at the seam.

**What is CONTROLLED — the definition, not a list.** A failure exit is
controlled when the process reaches its own exit door under its own control.
`mrd` has exactly one such door — `mrd::run`, whose `Err(Fail)` arm prints the
diagnostic and returns `fail.code`; there is no `std::process::exit` and no
`abort` anywhere in `crates/`. So inside this engine controllability is not a
discriminator between paths: **every** failure of `mrd` is controlled, and
controllability discriminates the engine from whatever killed it. What a
controlled exit may SAY is then decided by two further questions, both
answerable at the site by a reader writing a new path:

1. **Does it hold the trace's premise?** `ScriptTrace`'s first field is
   `entry_fingerprint`, the §4.7 value the whole run is consistent with. A path
   that failed before minting one has no premise, and a synthesized premise would
   mint a fact — the thing this module's assembler is built never to do. Such a
   path may not speak a trace, and its silence is contracted below.
2. **Does it know what the splice did?** A path holding the premise MUST speak,
   and what it may assert about the workspace is bounded by what it knows: a
   request that was never sent knows nothing landed; a request sent whose answer
   never arrived knows nothing either way, and must say so.

**The absence contract — what the survivor may rely on.** The obituary belongs
to the survivor, so the contract states what absence MEANS rather than leaving a
consumer to mint a convention:

- **Nonzero exit + a trace on stdout** — the engine answered. Every claim in it
  is the engine's, including `fault.recovery`.
- **Exit exactly 2 + empty stdout** — a controlled exit taken before the entry
  fingerprint existed: a bad invocation, an unreadable script, an unresolvable
  workspace, or no daemon to answer. **Nothing was armed, no splice was issued,
  and the workspace is unchanged** — this is a guarantee, not a likelihood. The
  diagnostic on stderr is a rendering for an operator, and no consumer needs to
  parse it to act.
- **Any other nonzero exit with an absent trace** — the engine did not choose
  this exit. It cannot promise its own obituary, so the consumer classifies from
  the observable pair and the class is `resync`: a splice already on the wire is
  the daemon's to finish, so re-read, never resend. `--dry` narrows it to
  `retry`, because a rehearsal writes nothing and could not have committed.

The second bullet is what the change BUYS, and it is worth stating as the
reason: today `exit 2 + empty stdout` spans both a provably-nothing-sent refusal
and a possibly-landed commit whose answer was lost, so it licenses no conclusion
at all. Once the premise-holding doors speak, that pair means exactly one thing
and the guarantee in it becomes true.

**A lost commit answer states its indeterminacy; it does not resolve it.** Of the
premise-holding doors, some sent nothing (`splice` refused with no error body) and
some cannot know (`splice` never answered; a frame that would not parse; an `ok`
carrying no body). The last of those is the one a later reader is most likely to
"correct": an `ok` bit looks like knowledge. It is not, and the ground is the
reply itself — **a reply that violates its own schema certifies nothing,
including its own `ok` bit.** Filing it as "landed but undescribable" would
over-trust a malformed answer; knowledge at this boundary IS a well-formed
answer, so this door is genuinely unknown rather than merely unexplained. The
first kind is an ordinary engine-minted refusal and takes
its class from the reading above. The second kind may not use `no_effect`,
`conflict` or a bare `refused`, because **all three assert that nothing landed**,
and that assertion would be a fabrication aimed at the caller's own file. It
carries `recovery: resync` — the class the consumer already dispatches on for a
killed engine — and, because `refused` alone reads as "nothing was applied", the
trace states the indeterminacy in band rather than in prose. Prose stays a
rendering here too: a consumer that needs to know whether it may resend reads the
class, never the sentence. `--dry` is `retry` on the same reading as the killed
path — a rehearsal that lost its answer provably committed nothing.

The shape, spelled: `outcome: refused` with `fault.class: refused`,
`fault.recovery: resync` (or `retry` under `--dry`), no `fault.code` — no frame
minted one — and **`commit_unknown: true`**, a boolean present exactly when a
splice was issued and its outcome is not known. The field exists because
`commit`'s ABSENCE is already spoken for: it means no splice was issued, so a
lost answer that merely omitted the leg would read as the read-class path. It is
a field and not a sixth outcome word, and not a fifth fault class, on the
preceding clause's own reasoning one axis over: **committed-or-not-known is a
PROPERTY of a run, not a KIND of outcome.** A sixth word would break every
consumer matching the closed five; a fifth class would break every consumer
matching `refused`. These doors leave through the findings leg — **exit 1**, with
`conflict`, `fault` and `refused` — which is also what makes the exit-2 guarantee
above true, since exit 2 is documented as the bad-invocation leg and a lost
commit answer is not a bad invocation.

**The migration is ADDITIVE.** A consumer that reads a trace when stdout carries
one is unaffected; a consumer that treated `exit 2 + empty stdout` as "the engine
did not answer" keeps that reading, now with a guarantee behind it.

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
| Reads | none — inputs arrive as inert `RunCtx` data | CLI lane: `read()` lowering to `toc`/`cat` — live, as a wire client through the one door. In-process lane (§ A.7): `read()` serving from the entry world plus the program's own armed overlay |
| Enumeration | page names its own targets | none in-kernel: host resolves selector (sorted) or binds caller `files[]` in call order — inert paths only |
| Commit | one atomic `if_fingerprint`-pinned batch via the local executor | ONE guarded commit as the caller (`actor`/`now`/`receipt` on the request): the single §4.4 splice for one armed path, the §4.4 SET form for N (§ One COMMIT per attempt) |
| Concurrency | workspace flock, `LOCK_NB` (decision #9) | stand-still optimistic at touch-set grain (amended 2026-08-15, plan §4.6): entry world pinned for reads (frozen view); commit premise = the engine-computed touch set, verified entry-vs-live — foreign churn outside it never refuses; conflict inside it ⇒ host re-resolves selector and retries (budget 2, `attempts` on the face) |
| Failure grain | one violation refuses the whole batch; bash phase-1 may stand committed and reported (decision #22) | one violation refuses the whole script; nothing ever partially lands (the sealed set keeps retry sound: a refusal lands nothing, so a re-run never double-applies) |
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
`.env` carry its capability declaration and input contract. `.args` names
positional slots in order and the count is exact, with one form for
variable-length input: the LAST name may carry a `...` tail
(`task.fmt.args: title, rows...`), which keeps the earlier names as fixed slots
and takes every remaining arg, zero included. Both dispatchers already consume
a positional list (bash argv, starlark `ctx.args`), so a tail changes no supply
surface; `.env` is supplied by name and refuses the suffix. Cross-file refs
are an S1 **non-goal** (decision #11) and refuse with a typed error. Every
addressing fault is distinct and pre-eval: no such task, dangling binding,
ambiguous anchor, not-a-code-block, unknown fence language, cross-file ref.

**A binding fault is scoped to its own row.** A binding VALUE is validated when
its own task is addressed, so `mrd run PAGE TASK` always answers TASK's fault —
a sibling's malformed or cross-file binding never masks it. `--list` renders
every declared row and prints a faulty row's typed error in place of its
language and caps, so one broken declaration neither hides the page nor
vanishes from it.

The one page-eager guard is the task **NAME** charset (§2.4, ruling 011): a key
outside `[A-Za-z0-9-]` refuses the whole page, including `--list`. Its reason is
not addressing but forgery — a name is stamped verbatim into every run receipt
(`task`, and the actor `run:<name>`), and listing it would print the forged
bytes it exists to keep out.

## Capabilities — deny-by-default (verdict ruling 3, decision #15)

An undeclared block is read-only: it can compute, but no effect of its
executes. The cap plane speaks THREE CAP VERBS — `md.create` / `md.edit` /
`md.delete` — answering one question: may this block touch files there
(caps-redesign ruling, 2026-08-19; distinct from the birth-preset *three
verbs* of § 4). HOW it touches them is the descriptor plane's (executor
ops), extensible without growing this grammar: `Create` needs `md.create`;
`SetField` and `AppendSection` need `md.edit`; `md.delete` is reserved — it
parses and resolves so grants can be written ahead, but no descriptor maps
to it until a retire descriptor exists.

⚠️ **Everything in this section is STARLARK's.** Caps do not apply to bash
(`laws.md` § Amendment): a bash task resolves `Authority::Unsandboxed`, its
`task.<name>.caps` is never read, and a present-but-empty declaration grants
it nothing — a bash fence rewrites any file it likes and the engine DETECTS
that in the exec-window bracket rather than denying it. Read every rule below
as governing starlark blocks.

**The two live verbs do not have the same reach, and the difference decides
your glob.** `Create` births a file the block names, so `md.create` is a
genuine *where may I write* grant. `SetField` and `AppendSection` change the
DECLARING PAGE and nothing else (`descriptor_surface`,
`crates/run/src/executor.rs`) — a starlark block has no descriptor that edits
a second file — so an `md.edit` scope is a SELF-GUARD narrowing the block
against its own coordinate, never a reach. An `md.edit:agents/*/CARD.md`
declared on a page that is not an agent card can admit nothing, ever.

A verb is optionally scoped by a PATH GLOB in the system's one glob grammar
(`policy::glob_match`, defined in `crates/policy/src/declaration.rs` — caps
call it, never reimplement it), matched at the choke point against the block's DECLARED
coordinates. Cap scopes carry one restriction the shared matcher does not:
every segment must be non-empty, never `.` or `..`, and built from letters,
digits or `_ - . * =` (`bad_glob`, `crates/run/src/caps.rs`) — a scope
outside that charset refuses at declare time, even where the same string
would be a legal glob for a rule or hook.

| Descriptor | Coordinate the glob judges |
|---|---|
| `Create` | its `path` argument VERBATIM — the resolution base (descriptor `base` > frame `ambient` > workspace root) is a separate axis, never glued into the matched string |
| `SetField` · `AppendSection` | the declaring page's path with the frame's `ambient` directory stripped as a LITERAL prefix when the page lies under it; with no `ambient` on the frame — or a page that does not lie under it — the page's full workspace-relative path unchanged |

⛔ **A create scope constrains the SHAPE of the declared path, not where the
bytes land.** `base` is an ordinary argument the block chooses, and the choke
point never reads it, so a block granted exactly `md.create:tasks/*.md` can
land `tasks/<slug>.md` under ANY confined directory in the workspace.
Measured 2026-08-19, all from that one grant: `conventions/attested/tasks/x.md`,
`receipts/tasks/x.md`, `meridian/tasks/x.md`, `.meridian/tasks/x.md`, and
**`.git/tasks/x.md`** — the reach included the receipt ledger, the attestation
tree, the engine's own reserved dirs, and the git directory, not just a wrong
content folder. The last four now refuse at the machinery floor below; `..`,
absolute paths and foreign roots refuse where they always did, and every other
confined landing is still reachable. The tail jail is real (`evil/tasks/x.md`
and `tasks/sub/x.md` both fail the glob as declared paths); the location is
jailed at the machinery floor and nowhere else. A root ceiling
like `run.caps.fix-*: md.create:tasks/*.md` reads as *births are confined to
boards* and does not mean it. That is the boundary-as-data ruling working as
designed — the engine holds no layout pattern to confine against — so treat
a create scope as a shape contract, and put content containment, if you need
it, in the block.

🛡 **The machinery floor (2026-08-20).** Four names are engine substrate rather
than layout, so the CREATE DOOR refuses any birth whose RESOLVED landing
carries one as a path segment — at any depth, ASCII-case-insensitively,
whatever the capabilities admit. The refusal is `bad_path`, it names the
offending segment, and nothing is written.

| Segment | What it is |
|---|---|
| `.git` | the git directory — a birth here can corrupt the repository |
| `.meridian` | engine stable state and run logs (`.meridian/runs/`) |
| `meridian` | the attestation tree (`meridian/armed-rules.md`, `meridian/attested`) |
| `receipts` | the receipt ledger (`receipts/run.md`, `receipts/realise.md`) |

**One carve-out: `meridian/domain.md`.** The hash-domain config sits beside the
attestation artifacts but is not one of them — it is AUTHORED content
declaring the ignore list, deliberately inside its own hash domain, and the
resident write path births it through this same door. Exempt at any depth.
Measured, not reasoned: the floor's first CI run refused it and took down three
door tests. **Stated limit:** the carve-out is a hole in the floor. A run block
granted a matching `md.create` scope can reach `meridian/domain.md` through its
own `base` and reshape which files the workspace attests. The door cannot tell
that block from a human authoring the same page — `actor` is caller-supplied —
so closing it needs a policy axis this guard does not have.

**At any depth**, because a nested root's machinery is machinery too:
`results/ws/.git/x.md` corrupts a repository exactly as `.git/x.md` does.
Measured over the live sessions corpus before the rule landed — every non-root
occurrence of these four names was a nested root's OWN machinery, never
content — so the depth rule refuses no legitimate birth. Case-insensitively,
because a case-insensitive filesystem lands `.GIT/x.md` inside `.git/` and a
guard a spelling defeats is not a guard.

This is a DOOR guard on the LANDING — deliberately the one axis capabilities
do not judge, so caps still read the DECLARED coordinate alone and the two
grains stay separate. It is also the one owner: both run-plane lanes (starlark
`create()` and the bash shim's `md.create`) converge on that door, as do the
wire `create` op, the birth preset and the realise card mint. It costs the
engine nothing — the armed artifact is written by `wire_serve::armed_disk`,
the receipt rides the batch commit, and run logs use plain I/O; none of them
passes this door.

The engine holds no layout pattern (boundary-as-data ruling, 2026-08-19 #2),
so `md.create:tasks/*.md` covers the ambient board, a based (`--target`)
board, and the root board alike. **That symmetry does not carry to edits**,
and the asymmetry is the one that bites: `ambient` is a frame field (cap
`run.ambient`) the calling host attaches per call, so on any lane whose host
sends none, an edit is judged by its FULL workspace-relative path — and a
short `md.edit:tasks/*.md` then denies a card sitting at
`year=…/<session>/tasks/x.md`. Spell an edit scope to span the depth,
`md.edit:**/tasks/*.md`, which holds either way: `**` matches zero segments
as readily as five. **Do not take the spelling the denial suggests** — its
`Fix:` line is built from the denied page's own path, so it hands you a
session-pinned scope (`md.edit:year=2026/month=08/<session>/tasks/*.md`) that
works today and denies every card in the next session.

Several scopes = several entries in the existing comma list; no new
list syntax. A scoped cap is strictly narrower than its bare verb
(`md.edit:**/tasks/*.md` < `md.edit`). Declared two ways — beside the binding,
or by name convention. **The two examples below are one working pair: the
ceiling must carry every verb the page declares** (see the verb-allowlist rule
under Precedence — a ceiling that omits a verb drops it whole):

```markdown
---
task.fix-drift: "[[#^fix-1]]"
task.fix-drift.caps: md.edit:**/tasks/*.md, md.create:tasks/*.md
---
```

```markdown
<!-- <root>/MERIDIAN.md — the root's own self-declaration -->
---
type: meridian-root
version: 1
name: field-notes
run.caps.fix-*: md.edit, md.create:tasks/*.md
# longest pattern wins — and a comment needs its OWN line (see the
# bricking hazard below)
run.caps.fix-note: md.edit:**/tasks/*.md
run.timeout_secs: 7
---
```

⛔ **One bad entry in this table bricks the whole root.** The table is loaded
before authority resolution, so an unparseable value refuses EVERY run on that
root — read-only tasks, `check-*` tasks, **bash** tasks (which caps otherwise
never govern), and even `mrd run <page> --list`, which is pure discovery. All
three causes are the same hazard, and none of the refusals names the file or
the key you broke — you get `invalid capability '#'` and no pointer to
`MERIDIAN.md`:

- a trailing comment: the frontmatter scanner takes no YAML crate and strips
  none, so `md.edit:… # longest pattern wins` parses `#` as a cap;
- a bad verb: anything outside the three;
- a glob outside the cap-scope charset above, e.g. `md.edit:tasks/x!y/*.md`.

After editing a ceiling, run `mrd run <any-page> --list` once — it is the
cheapest possible smoke test, and it fails loudly on a bricked table.

**Migration (the ruled split, caps-redesign 2026-08-19).** Retired per-op
spellings fold or refuse, never silently REINTERPRET a target: bare
`md.set_field` / `md.append_section` ALIAS-FOLD into `md.edit` at parse, and
the canonical form is what every report and refusal then names; their
field-grain targeted forms (`md.set_field:status`) REFUSE with the
retirement teaching — the old target named a field or section, the new
target position is a path glob, and dropping the target would widen the
grant. Field-grain guards live inside blocks. Partition grain
(parent-dir-name match) is retired with them.

⚠️ **The fold preserves execution and WIDENS the op axis** — say this out
loud, because a page that keeps running looks like a page that did not
change. `md.set_field` used to authorize field writes alone; folded to
`md.edit` it authorizes every page-mutating descriptor, `md.append_section`
included (measured 2026-08-19: a block declaring only bare `md.set_field`
applies an `append_section` descriptor). Every live bare legacy grant was
widened this way at the cutover. A page that relied on the OP grain as a
guard has lost it and must re-guard inside the block; the cap plane has no
op grain left to express it with.

A present-but-empty `caps` declaration is an EXPLICIT read-only grant, distinct
from no declaration. Precedence for the grant is explicit > convention > none;
conventions **narrow only, never widen**, and every cap that did not survive
intact is reported in `narrowed[]`. **Scopes meet by STRING EQUALITY, not by
glob containment** (`Cap::meet`, `crates/run/src/caps.rs`). Under a scoped
ceiling a page's cap meets it three ways, and only one of them keeps what the
page asked for:

| Page declares | Result under ceiling `md.edit:tasks/**` |
|---|---|
| the identical scope string | survives intact |
| the bare verb, unscoped | REPLACED by the ceiling's scope (`md.edit:tasks/**`), reported in `narrowed[]` — the page does not keep full reach |
| any other scope, however narrow | DROPPED — the grant is gone |
| a verb the ceiling does not name at all | DROPPED WHOLE — a ceiling is an allowlist of VERBS as well as scopes, so `run.caps.fix-*: md.edit` kills every `md.create` on a `fix-*` task, however the page declares it |

⛔ **Keep `md.edit` ceilings UNSCOPED; scope `md.create` instead.** Because
edits are self-guarded to the declaring page, a SCOPED edit ceiling is not a
narrowing a page can comply with — it is an on/off switch keyed to where the
page happens to live. Measured: under `run.caps.fix-note: md.edit:**/tasks/*.md`,
a `fix-note` task on `rules/escalate.md` is denied no matter what it declares,
including the bare verb, because the ceiling's own glob does not cover that
page; renaming the task so it falls to an unscoped `fix-*` entry applies
cleanly. The engine's "aim the effect inside what it leaves" is unfollowable
there — an edit has no second page to aim at.

Measured 2026-08-19: `md.edit:tasks/sub/*.md` under that ceiling is denied
though it sits plainly inside it, and the refusal's "aim the effect inside
what it leaves" misreads there, since the effect already was inside; a page
declaring bare `md.edit` resolves to `md.edit:tasks/**` with
`narrowed by ceiling: md.edit`. So under a scoped ceiling, spell the page's
scope byte-for-byte, or declare the bare verb and accept the ceiling's scope
as yours. The builtin `check-*` / `verify-*` ceiling
is absolute, and those names refuse a bash fence loudly at load. Caps bind at
the executor choke point before any I/O: one violation refuses the whole batch.

**The denial names the ceiling that ate the grant (2026-08-09, dogfood s12-50).**
`narrowed[]` reports the narrowing on the LISTING face; the refusal itself is a
second face and must stand alone. A block whose own frontmatter declares
`md.edit` and is nevertheless denied `md.edit` reads, at the denial,
as an engine ignoring a grant that is plainly on the page — and the remedy the
caller derives (declare the cap) is already in place. So a `capability denied`
refusal names **which ceiling removed the cap**: the winning `run.caps.<pattern>`
convention entry, or the builtin `check-*` / `verify-*` ceiling.

⛔ **Only when the ceiling is measured.** A denial that no ceiling caused —
deny-default, an explicit grant that never held the cap — names the cause and
STOPS. The engine never attaches a fixed remedy string to a cause it did not
measure: a remedy that may misdiagnose is worse than none.

**The one measured remedy the deny arm does teach is the retired-partition
respell** (caps-redesign 2026-08-19): where a declared GLOBLESS same-verb
scope `T` would have covered the DECLARED COORDINATE as `T/*.md` (the same
string every cap glob judges — the engine's own refusal text says "landing"
here, which is the coordinate, not the resolved destination), the denial names that
exact respell — `md.create:tasks` is now a literal glob matching only the
path `tasks`, and the page that used it under partition grain is told to
spell it `md.create:tasks/*.md`. It is taught only when the match is
measured, never guessed. Texts: `ExecError::CapDenied`
(`crates/run/src/executor.rs`); parse-time refusals — unknown verb, bad
glob, retired field-grain target — are `CapsError` in `crates/run/src/caps.rs`.

### Where the convention table lives (marker-retirement ruling, 2026-07-26)

**The root declares.** The table is read from the root's own `MERIDIAN.md`
self-declaration (`type: meridian-root`) — the artifact the config charter's
*"the root declares, `MERIDIAN.md` binds"* already governs — through
`crates/config`, which owns what a valid declaration is. The retired marker
files are not read and no fallback to them ships.

**A rooted invocation's declaring root is the PAGE's tree (rooted-refs-everywhere, ZT
2026-08-18 — address-grammar § 4.6).** `mrd run root:page` behaves exactly as if the caller had
cd'd into the named root: the convention table above loads from THAT root's own `MERIDIAN.md`,
the caps ceiling is that tree's, and the receipt lands in that workspace. The standing
workspace contributes nothing to the ceiling — *"the runtime cwd should not be a factor to
decide the behavior"* (ZT's motive, receipt at § 4.6). This closes the ceiling-by-cd bypass on
the plane where a declared ceiling exists — STARLARK: the `run.caps.*` table governing a
task's `md.*` writes is now always the page's own tree's, never a looser table chosen by where
the caller stood. (Bash holds no cap ladder at all — caps do not apply to bash, `laws.md`
§ Amendment; its only fence is the builtin name-keyed `check-*`/`verify-*` refusal, which
travels with the page whatever tree resolves it. Corrected 2026-08-18: an earlier spelling of
this sentence illustrated the bypass with bash tasks — address-grammar § 4.6, second editorial
note.)

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
(convenience, decision P6). *Amendment (2026-08-13, § A.8): U16's sentence
was written for a local entry whose cwd is the caller's context. A daemon
has no meaningful cwd, so on the WIRE arm the step's working directory is
the bound workspace root — stated in § A.8, deterministic, narrower than
the CLI. The CLI entry is unchanged.*

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
 Overwrite requires the explicit takeover flag. *(RETIRED 2026-08-15 —
 the no-guard amendment at the top of this document: this was the
 per-target pin-and-verify, a premise guard on a door whose promise is
 unkeepable. Replace-class effects are no longer gated on a prior
 receipt's rev, and the takeover flag gates nothing. The bullet stays as
 the record of the superseded law.)*

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
 edits) carries per-edit rev transitions — **attested history, compared by
 nothing** (the former decision-#26 foreign-edit scan is RETIRED — the
 2026-08-15 no-guard-on-effects amendment, § the no-guard amendment above;
 the code states the same at its seam).
- The receipt's `page` fact is the task page's **canonical
 workspace-relative spelling in the workspace that runs it**, resolved once
 at the door that admitted the ref — never the invocation's argv bytes
 (wire-contract §2.1: every ref-carrying surface speaks that grammar and no
 other). One page therefore owns ONE receipt history however a caller
 spelled it. Its consumers today are the receipt address and the run
 plane's page addressing — the scan that used to match on this key is
 retired (above), and the CLI ruleset is empty (`S1_RULES`). A ROOTED ref
 (`root:page` — address-grammar § 4.6, 2026-08-18) resolves to the named
 root's workspace, where the page HAS that canonical spelling: the run
 executes under the page's tree and the receipt lands in the page's
 workspace, so the one-key law holds there unchanged. *(Superseded wording,
 kept as the record: "…and the foreign-edit scan matches on that one key. A
 ref that resolves outside the workspace has no such spelling and rides
 verbatim — refusing it is the path-law door family's business, not the
 receipt's." Both halves are dead: the scan is retired, and a rooted ref
 now resolves instead of riding verbatim.)*
- The exec record carries **invocation id + exit code + stdout sha256 +
 byte size + log address**, joining the receipt through the
 `ExecRecordSink` seam.
- **Ordering is structural (S8):** the stdout facts are minted only by
 sealing the log, and sealing fsyncs the log file and its directory entry
 first. A crash can orphan a log (lint finds it); it can never produce a
 receipt naming a log that is not durable.
- Records carry env **keys, never values** (S7): the record receives the
 contract-validated **declared** map only, and emits its sorted key list —
 receipts name declared keys, nothing else. The child's real environment
 is larger since the run-env ruling (2026-08-16 — `wire-contract.md`
 § A.8): it inherits the daemon environment under the declared overlay,
 plus the plane's own injected variables, and none of those inherited
 keys reach the record. The constructor still discards values, so the
 type cannot carry a secret whatever it is fed.

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
and exit 2 — the CLI never guesses. *(Amended 2026-08-19 — the default-task
election, ZT directive: with several tasks declared, a binding named
`default` (`task.default`) runs instead of the list-and-refuse. This is not
a guess — the page's author elected it by name. The list-and-exit-2 leg
stands wherever no `default` binding exists. One owner:
`run::address::resolve_task`, so every door — CLI live, `--dry` rehearsal,
the wire arm — answers the same.)* Contract violations exit 2 with the
declared contract shown. `--dry` on starlark evaluates hermetically and
prints the **full** effect set, applying nothing; `--dry` on bash shows the
block and its resolved caps and **refuses to exec** — running it is the only
way its effects exist, and inventing descriptors would be fiction
(decision #18). The `--dry` caps display is byte-identical to the choke-point
caps (S14). `--dry` rehearses every pre-apply gate the real run enforces —
address → contract → caps, then the choke-point admission over the evaluated
md.* set (`runner::rehearse`) — and refuses exactly as the real run would,
same words, same exit leg (dogfood r2 F2: a rehearsal that passes what live
refuses predicts nothing).

Exit triad: **0** clean · **1** the run plane refused or failed (eval fault,
cap refusal, workspace busy, timeout, bash nonzero — the foreign-edit and
root-mismatch legs are RETIRED, 2026-08-15 no-guard amendment above) ·
**2** the invocation is wrong (usage, addressing, contract).

**A CHURN refusal carries a recovery line.** Two refusals here blame nothing
the caller wrote — the corpus moved under an unrelated writer while the call
was in flight: a **root mismatch** (the plane pinned a root for itself and the
world advanced past it) and a **corpus member that vanished** mid-read. Both
end with one fitted line — reason first, then `→ <the move> (recovery:
<class>)` — because a caller who reads only the reason has no way to know the
call was never wrong. The class is §8's, verbatim: `resync` for the root
mismatch (a merkle root is not invertible, so the door cannot say which files
drifted and cannot promise one re-read is enough) and `retry` for the vanished
member (`corpus_race` — the next snapshot serves the corpus as it now is). A
face that carries `recovery` **structurally** — the wire error frame — states
the reason alone; the line is for the text faces, which have nowhere else to
put the class. Receipts: dogfood r9 prober § F3. Re-deriving instead of
refusing is the churn-grain design, not this wording.

*(Amended 2026-08-15 — the no-guard amendment at the top of this document.
The root-mismatch leg above is RETIRED with the plane's self-pin: a foreign
advance re-derives and proceeds, which is exactly the churn-grain design the
previous sentence already named. `corpus_race` survives only where the
vanished record is the ADDRESSED target; a vanished unrelated record drops
from view and never fails another target. The recovery-line law — reason
first, then the move with its class — stands for the refusals that remain.)*

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
| Symlink laundering (`ln -s secret notes/x.md`) | refused or named (decision #25) | `O_NOFOLLOW` / refuse symlinked path components in walk + snapshot; where not refusable it is a **distinct named gap** (it defeats in-domain detection, unlike plain out-of-tree). *(Amended, dogfood r2 F2: the walk COMPLETES before refusing and the refusal is a COUNT plus the first offender, sorted — `N symlinked paths refused in exec-window snapshot, first: …`; one link keeps the established single-path wording. A symlink AT a path the domain's own ignore rules exclude — a stranger's venv `bin` dir, a `scratch*` entry — is outside detection like the ignored directories always were: skipped, not refused, reserved paths excepted. A sessions-shaped root declares the exclusion in `meridian/domain.md` frontmatter, e.g. `ignore: ["scratch*/", "bin/"]` — scratch is ungoverned by definition.)* |
| Ungoverned writes are never rolled back | law, not gap (decision #14) | they persist as actor-absent external change (§7.1) and the run exits 1 with the delta named |
| Multi-file crash window (content committed, receipt lost) | accepted (decision #10) | recovery is re-derive; lint finds the missing receipt |
| Local run beside a resident daemon (§7.1) | accepted | a local run's writes reach the daemon as external change — the same class as any out-of-band edit |
| The script entry runs in **wire-client mode**, not pure-local | law, not gap | a script must execute AS the caller, and the row above disqualifies the pure-local leg by this plane's own table: its writes reach a resident daemon actor-absent. Through the daemon, a script's writes arrive as governed, actor-carrying change, Delta-minted like any splice. *(Amended 2026-08-12: the in-process lane — wire `script`, § A.7 — satisfies this row's reason by a shorter path: eval runs inside the daemon and its commit IS the governed write path, actor-carrying and Delta-minted. The law stands; it gains a second conforming lane.)* |

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
| CLI mount — script entry | `crates/mrd::script::cmd` — the same client edge; its human-mode face is non-normative |
| in-process script serve (§ A.7) | `crates/registry` (the op arm: entry world, host, threading, commit) over `crates/effects` (kernel, trace, digest) — added 2026-08-12 |
| wire run serve (§ A.8) + script effects mode | `crates/registry` (`run_op`: per-target loop, §9 threading; `script_op`: the live host) over `crates/run` (the plane, unchanged) — added 2026-08-13 |

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
| `floor` | the workspace prefix this def's floor pins live under | `conventions/` |

Its body declares, in named sections:

- `# Properties` (`^properties`) — the rules a born record must satisfy. One
  `- key` or `- key = value` list item per rule.
- `# Template` (`^template`) — the fenced body one record is born from.
- `# Unfold` — the declared scaffold: the file paths a whole birth materializes,
  in declared order.
- `# Ephemeral` — the **allowlist** of declared-disposable paths. Empty by
  construction, so a def that declares nothing disposable can prune nothing.

**The parenthetical is a REQUIRED byte of the heading line (2026-08-09, dogfood
s13-20).** `(^properties)` and `(^template)` are block ids that must stand ON
the heading line — `# Properties ^properties`. The loader finds these blocks by
ANCHOR, never by heading text, so a def carrying a visually complete
`# Properties` section with no anchor id declares no `^properties` block at all.
The two spellings are indistinguishable to a reader and total opposites to the
loader, so both the element above and the `def_invalid` refusal below state the
anchor rule outright.

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

**Law 3.5 — a born record names the def it was born from.** Every born record —
the root record and each scaffold stub alike — carries `preset:` holding the
DEF's page path. A root record that names itself there makes its own provenance
line false and gives one key two meanings inside a single birth.

**Law 3.6 — a `^template` placeholder in a frontmatter value position is a
VALUE-PLANE WRITE.** `{{id}}`, `{{kind}}`, `{{actor}}` and `{{now}}` fill the
template BODY verbatim; inside the template's frontmatter block the same
substitution goes through the one encoder wire-contract § A.6.3a names, the
encoder `set_property` and `put{at:"upsert"}` already speak. The emitted value
is the plain form when the plain form decodes back to exactly the caller's
string, and the canonical double-quoted scalar otherwise.

A value that cannot be ONE frontmatter line — one carrying `\n` or `\r` —
**refuses the birth** (`bad_request` / `fix`), naming the key, the v1
single-line rule, the body-section escape, and the placeholder that carried it.
Nothing is written. This is Law 3.4 holding, not bending: sanitizing the
caller's actor would falsify the provenance the birth records, and an
escaped-scalar workaround leaks, so the door refuses instead of rewriting.

Measured (dogfood pass 1, f03): before this law the door interpolated source
bytes, and `--actor $'zt\nstatus: closed'` born against `owner: {{actor}}`
minted a record carrying `status:` twice — disk `closed`, every read door
`open`, and no governed edit able to reach the shadow line. One law missing at
a third door, where two doors already carried it.

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
that cannot state its own contract cannot birth a record that satisfies it. The
missing-block refusal states the anchor rule (Law 2.1's neighbour above): the
block is found by its `^` id on the heading line, so an author staring at a
visible `# Properties` heading is told which byte is absent, not merely that the
block "does not exist".

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

**Teaching row — Law 5.3 OUTRANKS Law 5.1, and the losing entry dies
silently.** An `# Ephemeral` path that lies outside the scaffold's territory is
INERT: `--prune` walks past it with no prune row, no finding row and no
refusal, because territory decides scan scope before the allowlist is ever
consulted. Measured 2026-08-09 on v1.0.0: `sessions/tmp-cache.md` (allowlisted,
in territory) pruned, while `scratch/tmp.md` (allowlisted, `scratch/` holding
no `# Unfold` path) survived untouched and unreported. The precedence is
correct — it is Law 5.2's safety property, which must not weaken because a def
ASKED for a deletion outside the shape. **What the face does not do is say
so.** A def author gets a dead allowlist entry with zero disclosure at declare
time (`mrd new` accepts the def) and zero at prune time, so the only way to
learn the entry is dead is to notice the file that should be gone is still
there. An ephemeral-declared file that IS present renders no row under a
no-prune reconcile either — neither finding nor ephemeral.

⚠️ Recorded here as the scoping fact it is. Whether the plane owes a
declare-time or prune-time disclosure on a territory-shadowed allowlist entry
is NOT settled by this page.

**Law 5.4 — pruning is opt-in.** Without `--prune`, reconcile materializes and
reports and removes nothing.

## 6. The convention floor

A session preset's `inputs` pin the convention floor — the rule pages the born
session lives under — at a path and a rev. The root record is born carrying that
pin, so the law a session was born under is readable from the session itself
long after the def has moved on.

**Law 6.1 — a floor pin is a pin, not a copy.** The preset records `path@rev`;
it never inlines the floor's content into the born tree.

**Law 6.2 — the born root record carries the floor pins itself.** Its `inputs`
is one block sequence holding the def pin first, then every floor pin the def
declared, in declared order. A root record carrying the def pin alone leaves the
floor readable only transitively — def@rev, then the def's content — which is
one indirection weaker than "readable from the session itself" and survives only
while the def blob does.

**Law 6.3 — the floor prefix is a default the def overrides, never a validity
predicate the engine owns** *(no-hard-coded-flow amendment,
`docs/laws.md` § Amendment — no hard-coded flow; ZT ruling 2026-08-15)*.
`conventions/` is where the U4.4 floor suite lives by convention, so it is the
fallback; a def spelling `floor: standards/` pins its floor there and is exactly
as valid. The engine reads the def's own key and only falls back to the
constant — the shape `root` / `DEFAULT_ROOT_RECORD` already had. A user who
files their convention suite elsewhere is served, not refused.

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
| **3.6 template fill is a value-plane write** | **WAS ABSENT — added 2026-08-09** | The door interpolated source bytes into the born frontmatter, so a caller value carrying `\n` or `: ` minted a second key line (dogfood pass 1, f03). `fill_template` now routes a frontmatter value position through `policy::defs::yaml_safe_value` — the encoder the other two § A.6.3a doors use — and refuses a multi-line value in their uniform words. Gated by `crates/preset/tests/birth_value_plane.rs`, six tests including the plain-value control and the body-verbatim control. |
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
