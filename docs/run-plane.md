---
type: spec
id: run
status: standing
description: The run plane — how `mrd run` executes an addressed task block and turns what it emits into governed effects, plus preset and session birth.
owns: [the run plane, preset and session birth]
---

# The run plane — `mrd run` (S1)

> Standing law: `README.md` (process and standing corrections) and `wire-contract.md` (the wire contract).

The run plane executes an addressed task block and turns what it emits into
governed effects. It is **consumer-plane, imperative, local**: a client of
the engine crates. The daemon, the wire and the serve path carry no
run-plane state across invocations.

Two entries cross the wire:

| Entry | Door | What the daemon holds |
|---|---|---|
| script | Starlark evaluation in the daemon behind the wire `script` op (wire-contract § A.7) | the evaluator and per-attempt state, for the attempt only |
| task | wire-contract § A.8: a target list through the face, or `run()` inside the script entry (live, under § Effects mode) | the § A.8 op arm (per-target loop over the unchanged `crates/run` seam, §9 identity threading) and the effects-mode live host, per invocation |

Charter: **the serve path consumes the run plane, it never re-implements
it** — one runner, one executor, one receipt convention, whichever door
invoked it. `mrd run` and `mrd script` are clients of the same plane.

**No-guard amendment (normative): `run` and script-with-effects are not
guarded.** No CAS premise, no fingerprint requiredness, no synthesized
touch-set guard, because mrd cannot bound what execution does. Guards are
pure-lane law (wire-contract §5.4–§5.7, § A.7) for markdown writes through
the pure doors. Consequences:

- **No world pin**, and no narrower pin in its place: no root-mismatch
  premise refusal, no per-target pin-and-verify before a replace-class
  effect. No refusal on this door is a premise refusal.
- **Observation honesty only.** A foreign advance re-derives and proceeds:
  reported (the named out-of-band window), never refused. A vanished
  unrelated record drops from view and fails no other target; a vanished
  addressed target is an invocation-law (addressing) refusal.
- **A task-selection pin is targeting, never CAS.** `task_rev` on the wire
  row (or any future selection pin) chooses what to execute; docs and faces
  call it targeting. A supplied guard field is rejected as inapplicable,
  never checked (wire-contract § A.8).
- **Guard-free is not fold-invisible.** Every effects write rides the one
  write choke-point and maintains the resident tree, advancing the folds
  other writers' premises compare against — tree maintenance, not a guard.
- **Beside it:** `put_live` is CAS-free by its own law; `effects` and guard
  fields are mutually exclusive at decode; the step's own out-of-band writes
  refuse phase-2 convergence (the governed-change law: one write path, not a
  world premise); `run.lock` serialization stands (a lock refusal is not a
  premise refusal).

Deliberate non-guarantees: § Accepted gaps.

## The kernel entry points

The Starlark kernel has exactly three entry points:

| Plane | Entry | Trigger |
|---|---|---|
| change | `on_change(event)` | a governed change event (the effect kernel) |
| run | `def run(ctx)` | `mrd run` on a task block; in the `run` op's fire mode, the block's declared entry (§ The run entry, amended) |
| run | script — module top level | `mrd script` / the wire `script` op (wire-contract § A.7) / MCP `script` carrying caller-supplied inline source |

- `on_change(event)` and `def run(ctx)` are **hermetic by construction**; the
  script builtins do not join their globals.
- The script entry is **hermetic by recording**: its one effectful builtin is
  `read()`, every read response is recorded into the trace, and eval is a
  pure function of `(script, args, files, read-response sequence)` (§ The
  script entry).

`RunCtx` is inert data: page, task, args, env, invocation id, root-at-eval.
Identity and time are **caller-supplied** (§9): the kernel never reads a
clock and never mints an id.

**The fold behind root-at-eval is lazy on the starlark leg: it is taken only
when this tense's output will put the token in front of a reader.** This
binds every caller of `runner::run` / `runner::rehearse`: the `mrd run` CLI,
the wire `run` op (`registry::run_op`) and `realise`.

| tense | folds when | the token's reader |
|---|---|---|
| **rehearsal** (`--dry`) | ANY effect was emitted | the `--dry --json` report and the wire rehearse row (whole effects, provenance included) |
| **live** | an **md.\*** effect was emitted | the receipt's `root_pin`, written from `observed_root` |

- **Rehearsal residual.** The human `--dry` (`mrd::run_cmd` `dry_starlark`,
  `Format::Human`) prints `kind` plus args, no provenance, yet pays the fold
  when it emitted anything: the gate ignores output format, so `--dry` and
  `--dry --json` observe the same world.
- **Live gate.** The live report prints only `kind` + `domain`
  (`run::report::EffectLine`), so a live block emitting only
  `proto.*`/`daemon.*` (the hook shape: notice / remind / send, once per
  event) has no reader for the token: no batch, no `observed_root`, no
  receipt. Its `root_at_eval` stays empty in the in-memory effect set. The
  cascade's per-generation fold has the same gate.
- **When a gate fires:** one `fs::domain_fold` after the eval, its root
  stamped onto every emitted effect's `Provenance::Run.root_at_eval` and,
  live, handed to the apply as `observed_root`. After-eval sees the same
  domain as before-eval, because this entry cannot write. The token is never
  compared (the no-guard amendment, above).

**Starlark never invokes bash on the pure path** (test-gated): the sandbox
exposes no `exec` / `subprocess` / `os` name. The composition layer is bash.
The one sanctioned exec surface is the effects path: a submission carrying
`effects:["run"]` holds a live `run()` that executes the addressed task at
call time (live, not armed/deferred: out-of-world effects cannot be refused).
The surface test asserts: pure globals stay `{read, me, put}`; effects
globals add exactly the admitted list.

## The script entry

The script entry runs caller-supplied inline source instead of an addressed
task block: same plane, same one write path. A task is a governed page's
declared behavior; a script is one caller's inline intent.

**Inputs — all caller-supplied, all inert** (the `RunCtx` precedent):

| Input | Meaning |
|---|---|
| `script` | the source; the module top level is the body, no hook lookup |
| `args` | the caller's arguments as an **inert dict**: string keys, string values |
| `files[]` | **paths only**, in call order (`files[i]` = the i-th path named); patterns expand in place; a pattern before a literal refuses at entry (`files_member_order`, wire-contract § A.7 literals-first) |
| `actor` | the caller's own identity, threaded per §9; the engine mints none |
| `now` | caller-supplied time; the kernel never reads a clock |
| budget overrides | fuel / mem / call depth / source bytes / wall clock / max reads / max armed edits |

- `args`: a dict, not a list (`args["page"]`); no callables, no host reach.
  The kernel binds it; a host never flattens or reshapes it.
- `files[]`: paths, never content; content enters only through recorded
  `read()`.
- No `glob()` builtin: enumeration is the host's; the wire serves no
  corpus-enumeration op.
- No cap grammar here: authority is the caller's identity, not a declared
  ceiling; delegation caps are a v2 feature.

**Recorded-read purity.** `read(path)` (the toc face) and
`read(path, section=…)` (the cat face) are the only effectful builtins;
`put(...)` and its siblings append to the armed list and do no I/O at call
time. **Script eval is a pure function of
`(script, args, files, read-response sequence)`**: the trace records every
read response, so re-evaluation against it is byte-identical. No exec
surface exists here.

**The read budget is 64 `read()` calls per attempt, not 64 files.**
`max_reads` counts read-builtin calls, no dedup by path, section and
whole-file reads alike: toc + N sections spends 1+N, so the file domain is
below 64 once sections are used. Example: 3 `--files` with 30 sections each
refuse at 70 section reads (`outcome fault`, `reads_used 64`); at 60 they
return `outcome no_effect`, `reads_used 60`, exit 0. Raising the budget is
refused: it buys a different wrong number, not a stated domain.

**Effects mode.** `effects: […]` beside `dry`/`files`/`args` names the
effect builtins the program may use; the closed set lives only in
wire-contract § A.7's effects paragraph (today: `run`, `token_count`). No
`mutex()` builtin: exclusivity belongs to the coordination layer. The flag
switches the execution model:

- **Absent → pure script.** Everything above holds: entry world, armed set,
  set-form law, the touch-set commit premise (below), replay.
- **Present → live program.** `read()` serves the live disk at call time: no
  pin, no overlay. `put()` applies at once through the wire splice door
  (write flock held, structural validation intact, the guard's `force`
  bypass): no rev, no snapshot, no CAS, no set-form law.
  `run(page, task=None, args=[], env={}, dry=False)` executes the addressed
  task at call time through the plane's own seam and returns its § A.8 row
  (state, exit code, stdout) as a value; refusal rows return as values too;
  only shape errors (wrong argument types) fault the program.
- **Principle.** The rev leashes an agent's stale context, not writes; a
  script's own read is the freshest. Effects cannot be refused
  (out-of-world), so no transaction promise holds.
- **Accepted tradeoff, on record.** Two effect-scripts can last-writer-wins
  each other on one section, like two shell scripts; the engine write flock
  keeps files structurally intact.
- **`token_count(text)`** returns the text's real token cost as an int,
  measured at call time through the § A.7 frame's `token_count_endpoint`;
  the engine never counts tokens. The string is measured verbatim (the tool
  face's `{text}` arm), no ref resolution; compose with `read()` to measure
  served content. No endpoint faults "unbound"; an endpoint refusal faults
  the program with its words carried whole; the dial deadline caps at the
  remaining wall clock. No trace entry; a top-level binding echoes like any
  computed name.
- **No rollback.** A mid-program fault leaves prior acts landed; the trace
  records how far the program got. A completed live program's outcome is
  `effects` (the vocabulary's one addition); `fault` keeps its meaning on
  both models.
- **Replay refuses a live program:** `replay_script` refuses an effects-mode
  context.
- **Budgets.** Eval limits and the wall clock bind over the program's own
  acts (reads, puts, compute); the read ceiling counts live reads
  identically; `put()`/`run()` are not fuel-metered. A live `run()` is
  admitted under the script clock (the pre-dispatch check); the clock then
  stops while the run plane executes under its own budget
  (`run.timeout_secs` on the root's declaration, default 5m), never charged
  to the script clock. `max_runs` (64/attempt) bounds the count: the 65th
  run refuses typed, naming the ceiling; executed runs stand.
- **Identity (§9).** `actor`/`now` thread as everywhere; run identity is
  `<invocation>-r<K>` from the submission's host-minted `invocation` base,
  K the 0-based call ordinal.

**Read alignment — both models.** In-script `read()` mirrors the read tool
and returns values, not opaque structs: `read(path)` is the wire toc face,
1:1, a dict `{"fm": {…}, "toc": […], "rev": "…", "words": N}`;
`read(path, section=…)` is the section text as a plain string
(`"x" in read(p, section=s)` is legal), its rev still riding the recording
for the threading law. `section` takes the read tool's selector grammar
(heading path, dewey ordinal, `^anchor`), every arm served in-script.

- `fm` values are decoded scalars (wire-contract § A.6, as for the composed
  read's `props[].value`): `owner: "[[x]]"` reaches a script as `[[x]]`; a
  comparison against the unquoted form arms.
- `words` is the wire's `words_total`, a delivered fact the host carries,
  never a count this plane computes: the whole file (`wc -w` parity), never
  the sum of section rows (wire-contract § A.3).
- The composed `read` (§4.1, toc mode) carries `words_total`; the `toc` op's
  body `{path, file_rev, root, nodes}` does not, so `read(path)` asks both:
  `toc` for the rev and section map, `read` for the count. Zero wire delta:
  both ops exist, and a read mints nothing (wire-contract § A.3).

**The snapshot guarantee and its composition rule.** `script` pins one entry
fingerprint and `commit` guards on it; the single-snapshot guarantee holds up
to the read budget. Above it the caller composes runs under a checkable
rule: **equal entry fingerprints across runs = one snapshot; unequal = the
world moved, re-run.** No daemon-held state; an engine-held chunk-spanning
snapshot is deliberately not ruled in (revisit trigger: compose-retry
livelock under real churn in the field). `mrd script --help` carries the
budget, so the guarantee's domain is documented.

**The trace is the read-only script's output channel, and it is contract
material.** There is no `print()`; the builtin surface is closed. A script
that only reads arms nothing and exits `no_effect`: that reports nothing was
**armed**, never that nothing happened. Under `--json` its reads come back
as `trace[]` rows of `{kind, line, path, face}`.

**Seam, named and not taken here: the guarantee's crossing (face-honesty
clause 5).** Content crossing the guarantee boundary owes a mark: the
provenance it was read under, so the caller can re-verify. Separate work: it
changes what the read face emits beside content and touches the composition
rule above. Nothing of it is implemented, stubbed, or designed around here.

**A composed read is bracketed by `file_rev`, or it refuses.** A whole-file
`read(path)` is 2+N live round trips (`toc`, one `cat` per frontmatter key,
the closing `read`), and the world may move between any two. The opening
`toc`'s `file_rev` is compared with the closing `read`'s; a difference
refuses the read, naming both revs. The count op closes the bracket (asked
anyway: no extra round trip), so every `cat` sits between two agreeing
observations; the count is asked last, never second. A single
`read(path, section=…)` is one `cat`: one op, one revision, no bracket.

**What the bracket does not catch.** `file_rev` is content-derived: an A→B→A
sequence (a write and a byte-identical restoration inside one composed read)
closes the bracket on A while the `cat` values were taken at B. Nothing
false is published (A describes those bytes too), but the face cannot say
the file did not move: the bracket is a revision-identity check, never a
mutation counter, and the wire offers none. The commit is unaffected: it
carries the entry fingerprint and §5.1 checks it first, so a world that
moved and came back still refuses there if the fingerprint moved.

**The arming surface — `put()` speaks the wire's second edit dialect.**
`put(path, props={…})` arms one `set_property` plan item per key, keys
sorted; `put(path, section="…", append="…")` arms one section-addressed
`append`. Both are `splice.plan_edits[]` items (§A.3) carried verbatim and
lowered by the existing intake (`wire-serve::plan::lower`): no third edit
grammar, wire schema untouched. One `put()` may arm several items; arm order
is execution order; each item records its source line and nesting depth.
Depth is a trace fact only: an applied effect renders at any depth
(echo/quiet governs reads, not arms).

An `append` addresses a section: `PlanEdit::Append` carries an hpath, an
empty one refuses `NotFound`, and a document-grain append has no wire
target. A bare `append=` with no `section=` refuses at arm time rather than
minting a default section; the MCP `put` face refuses the same shape in the
same words. A `props=` write needs no section (frontmatter is file-grain).
Addresses are segments: `section="Notes/Fresh"` is two segments, never a
joined string.

**One address grammar, one parser.** `section=` on `put()` is parsed by
`ReadSel::parse`, the same door as `read(path, section=…)` and the tree's
one human-string→selector door, so the three spellings mean the same on both
faces: `^id` is a block address, digits-and-dots a dewey ordinal, anything
else splits on `/` into raw heading segments. An `append` carries an hpath,
so the two non-heading spellings refuse at arm time, naming what they are: a
toc row's `^anchor` is a real address on the read face and a real refusal on
the write face, never a heading silently named `^r-…`.

**`section=` also takes the §2.1 segment array:** a list of `{h, n?}`
objects, one per heading, raw text verbatim, the wire's machine form on both
faces. The string coat still splits on `/` with no escape; the array is that
escape, the one the section-miss refusal already teaches. A heading whose
raw text carries `/` rides one entry; the occurrence index `n` rides the
structured form only. The toc face publishes each heading row's raw segments
as `hpath` beside the joined `section`, so any row feeds back into
`section=` verbatim. A bare string in the list refuses with the wire's
single-sourced text (wire-contract §2.1); the type refusal names both
accepted forms.

**The statement-position rule — echo and quiet.** Every read is recorded. A
read **echoes** exactly when its call is the whole right-hand side of a
top-level assignment or a top-level expression statement (`card = read(…)`
binds and echoes); every other position is **quiet**: comprehensions, `if`
conditions, loop bodies, function bodies. The rule is read off the parsed
module: syntactic, never a call-depth heuristic. No suppression syntax in
v1: `_ = read(…)` is rejected permanently; `quiet()` waits on elision-count
evidence.

**The bindings echo.** A successful evaluation's module top-level bindings
ride the trace as `bindings` (name → Starlark repr, name-ordered). One
carrier per fact: inert inputs and `def` bindings stay out; a name whose
last assignment is a top-level `name = read(…)` stays out (that value is the
read's own `echo` entry), but any later rebinding (reassignment, `+=`, a
loop target, an assignment inside an `if` or `for` body) returns it. A
failed or refused evaluation carries no bindings; a run that bound nothing
emits no `bindings` member.

**The grammar.** The script entry parses under the rule dialect plus
top-level statements: the module top level is the program, so top-level `if`
and `for` are ordinary. A rule or task must define a hook, so the hooked
planes keep the stricter grammar. `load` stays disabled at every entry.

**Where the budgets bind.** Fuel, memory, call depth and source bytes are
`EvalLimits`, shared with the other two entries. The read ceiling (64 per
attempt) is the script plane's own, kernel-enforced: the read past it
refuses typed, naming the ceiling, and the attempt has no result (never
truncation). The armed-edit ceiling (64 per attempt) binds in the kernel at
arm time: `put()` refuses the crossing call, typed and naming the ceiling,
no host involvement. The retry budget binds in the host, above the kernel:
the loop is the host's.

**The wall clock binds at four layers.** Fuel bounds computation but not a
blocked read, so time needs its own budget. The attempt is one § A.7 `script`
frame: the CLI holds the socket, the daemon the evaluation.

1. **Per read, in the daemon** — before every read, against the pinned entry
   world.
2. **On the socket, in the CLI** — read and write timeouts of one wall clock,
   so an unanswered frame fails the round trip. The only layer `mrd` holds.
3. **Before the commit, in the daemon** — a clock that elapsed during
   evaluation refuses **pre-commit**; nothing is issued or lands.
4. **Above the process, in the MCP host** — the child is killed by process
   group past its own bound, for a child that never reaches or ignores its
   clock.

Layers 1–3 refuse in the entry's vocabulary and answer a trace; layer 4
answers a host refusal, never a face. The budget is 7 s on both sides:
`effects::DEFAULT_WALL_CLOCK` (layers 1 and 3) and `WALL_CLOCK`
(`crates/mrd/src/script/cmd.rs`, layer 2), two literals kept equal by hand.

The write verbs (`put`, `pin`, `rm`, `retire`) dial the same `SocketDoor` but
carry no budget, so a read timeout is **not** their verdict: the door waits
for the answer (`write_ipc::call` → `SocketDoor::call_until_answered`),
printing one notice at the first tick. The wall clock still bounds their
HELLO: a daemon that will not greet is down, and nothing was sent. A
transport loss after the frame goes out is an unknown outcome (read before
any re-send), never a failed write — a timeout there once returned `os error
35` (the layer-2 `EAGAIN`) at 7 s, exit 1, while the bytes landed seconds
later.

The in-process lane (wire `script`, wire-contract § A.7, no CLI) has no round
trips and no child; the clock binds at three daemon-enforced sites: entry
before the pass, every read builtin, pre-commit. Fuel bounds computation
between checks; `catch_unwind` at the eval boundary bounds the rest: a panic
answers a `fault` trace and the daemon serves its next frame.

The host-side defaults are **§5.3 host policy**: their existence is contract,
their values tunable.

| Budget | Default | Binds | Over it |
|---|---|---|---|
| wall clock | 7s | entry (per round trip, per socket, pre-commit) | typed refusal |
| child bound | 30s | MCP host (process group kill) | host refusal, no face |
| retries | 2 attempts | host | exhaustion → resync face |
| armed edits | 64 | kernel (arm time) | typed refusal |
| live runs / attempt | 64 | kernel (run admission, effects mode) | typed refusal; executed runs stand |
| reads / attempt | 64 | kernel | typed refusal, no result |
| selector width | 256 paths | host | typed refusal, **never truncation** |

The selector cap sits above the read ceiling: it bounds enumeration (host
result size, fan-out width), not I/O. 256 covers board-wide fan-out without
letting a runaway glob return the corpus.

**What an entry costs.** What a program spends against the table above.

**The ceiling is a function of round trips, and reads are not trips.** For an
entry against `C` domain members:

```
ceiling = f(reads, corpus)

wall clock  ≥  trips(R) × pass(C)

trips(R) = 3 + Σ per read: (2) whole-file
                           (1) sectioned

pass(C)  = O(dirty) vouched; floor O(C) in `stat`s,     <- the linear term,
           O(changed) in bytes                             floor only
```

- **`pass(C)` is linear only on the floor.** The vouched pass
  (`node-rev-merkle-spec.md` §6.7: the event feed's cookie proof over the
  resident memo) costs O(dirty), constant on a quiet corpus. Any named miss
  (no live feed, an unproven cookie, a doubt collapse, an untrusted memo)
  falls to the extent-refresh floor, which `stat`s every domain member:
  doubling the corpus (23,758 → 47,477 members) multiplied the floor pass by
  **1.84×** and per-read cost by **1.91×**. Size against the floor. Cost is
  reads × pass, not reads × frontmatter × corpus. The OS event watcher
  (§6.4) carries the vouched pass.
- **`3` is the fixed frame:** `hello`, `fingerprint`, the commit.
- **A whole-file `read(path)` is two trips:** the `toc` op, and the composed
  `read` (§4.1, toc mode) that brackets it and carries `words_total` plus the
  frontmatter. A sectioned read is one (~122 ms against ~235 ms).
- **Two is the floor.** The composed read carries `file_rev`, heading rows,
  `words_total`, and `props[]` decoded per § A.6 (no `cat` per frontmatter
  key), but no rev for `^anchor` rows: `wire::ReadAnchor` is `{anchor,
  span}`, `wire::ReadRow` has no anchor field; only the `toc` op's nodes
  publish an anchor row with its own `node_rev`.

**What a pass costs.** Every trip is answered from the warm engine, proved
current first over the whole hash domain: a read is corpus-scoped, since a
poison member anywhere refuses a read of a healthy one (Law A-3c); that scope
is not an optimization target. The pass reuses the listing of any directory
whose timestamps did not move, `stat`s every member, re-reads only members
whose stat identity moved, and folds the §12.2 tree from per-member digests:
O(corpus) in `stat`s, O(changed) in bytes. Re-reading every byte per trip
would cost ~0.9 s on a 24k-file, 150 MB corpus.

**A stale engine pays a rebuild, and `pass(C)` never prices it.** The formula
assumes the resident engine agrees with disk; the currency check's other
branch is a different regime, not `pass(C)` scaled up.

- **Trigger: any fingerprint move, including the caller's own.** A splice
  commits to disk without touching the resident engine, so the next currency
  check treats the caller's own commit like a stranger's write. The host's
  retry (budget 2) fires exactly because the fingerprint moved, so a retried
  attempt always reads a stale engine; the formula does not bound a
  write-bearing program's next trip.
- **Cost: no incremental path.** A currency miss re-reads every member
  unconditionally (the leaf memo is bypassed, not widened) and re-parses every
  member, replacing the resident index and document map wholesale. One changed
  byte pays the same as a rewritten corpus.

```
rebuild(C)  =  O(corpus) in full reads  +  O(corpus) in full reparse,
               paid whole regardless of how much of C actually changed
```

A stale trip pays `rebuild(C)` in place of `pass(C)`, not in addition; the
ceiling formula holds only on a current engine, and `commit(C)` below
composes with either.

- **Measured:** nothing controlled. `fs::domain_snapshot` (the re-read half)
  has a benchmark arm, `crates/fs/examples/domain_cost.rs`, with no recorded
  run and a slope not assumed to match `pass(C)`'s; `fs::build_corpus` (the
  reparse half) has none. The only order-of-magnitude figure is the cold
  daemon start (`node-rev-merkle-spec.md` § 0): 2.2–5.2s on a 50,319-node,
  9.5 GB corpus (M4 Max), a related path.
- **Evidence:** `Registry::warm_or_build` (`crates/registry/src/registry.rs`),
  the sole site — `Reused` on a match, `fs::domain_snapshot` +
  `fs::build_corpus` (`syntax::parse` + `model::build`) on a mismatch;
  `Op::Splice` (`crates/registry/src/server.rs`) writes disk and returns.

**A write-bearing program pays a byte term the pass never does.** The
read-side formula alone is a lower bound: the commit's §5.1 world guard and
the seam roots fold **from bytes** under the write flock (`domain_snapshot`;
the digest memo never supplies them, so the guard stays byte-derived).

```
wall clock  ≥  trips(R) × pass(C)  +  commit(C)

commit(C) = 0 for a read-only program
            two byte-folds, O(C) in BYTES, for a write-bearing one
```

The folds are O(corpus **bytes**): ~1.8 s together on the 24k-file, 150 MB
corpus priced throughout, more than the fixed frame. The remaining `≥` is OS
variance, never an unpriced engine cost.

**The live-corpus term, measured** (five trials, medians):

```
commit(C)  ≈  4.46 s   process wall, live corpus, 5-trial medians
```

Process wall as an operator meets it: the folds are not decomposed (the
figure includes the rest of the commit path), the daemon round trip is not
separated from the splice, and corpus size was not varied (no slope). The
gap to ~1.8 s is no regression claim: the two were never verified to measure
the same shape. A decomposition run is not owed for v1 and opens only if v1
testing hits the term.

**The linear term has a measured slope.** Per-read cost multiplies by **~2.1×
per root doubling** (1.84× pass, 1.91× end to end; 2.35× without the leaf
memo, so the memo buys about one doubling — a constant, not a change of
shape). A 10-read program at ~3.8 s on a 2× root returns to ~7.8 s at 4×,
over budget. Capacity planning reads `ceiling = f(reads, corpus)` with that
slope.

**The in-process lane's cost shape.** Above, each trip pays `pass(C)`. On the
in-process lane (wire `script`, § A.7) the pass runs once, at entry, and
reads serve from the pinned entry state:

```
wall clock  ≥  entry_pass(C)  +  Σ reads × O(1)  +  commit(C)
               one pass, at entry   memory-speed     unchanged
```

`entry_pass(C)` keeps `pass(C)`'s shape, paid once per attempt, so per-read
cost does not move with corpus size; the slope still governs the entry and
commit terms. A stale engine pays `rebuild(C)` at entry instead. No figures
for this lane are recorded.

**The bash bracket's observations serve from the resident memo.** A bash
dispatch observes the corpus three times (pre-flock leaves fold, bracket
open, bracket close) from an injected source (`RunSpec.observations`).

- **CLI lane:** a fresh walk each time — full `read_dir` enumeration, full
  stat sweep, byte reads amortised by the per-workspace drawer memo
  (`run-digests.v1`); a separate process has no resident memo.
- **Daemon door** (§ A.8 `run` op, § A.7 in-script `run()`): the registry's
  resident `fs::DomainCache` (dir-listing memo plus leaf memo, shared with
  every currency pass and warm rebuild), locked per observation, never across
  the exec window, no drawer I/O.

Verdicts are lane-independent by gate (`crates/fs/tests/cached_observation.rs`:
same folds, residual deltas and symlink refusals, including from remembered
listings). The daemon lane skips enumeration and drawer serialization; the
stat sweep stays, because an observation is a live walk. Measured
(`crates/fs/examples/run_observation_cost.rs`, hermetic 29.5 k-doc corpus):
trio ~288–305 ms median through the drawer, ~245–248 ms resident;
enumerations per warm trio ~888 versus 0; byte reads identical, movers only.
A run leaves the shared memo warm for the next op's currency pass, and vice
versa.

**The transaction — stand-still optimistic.** The word **snapshot is banned**
here: the daemon has no MVCC and v1 must not grow one.

1. **Entry.** One currency pass pins the **entry fingerprint** (§4.7) and the
   entry world.
2. **Reads serve the entry world**, plus the program's own arms — the four
   laws below.
3. **Commit.** One splice batch, its premise the **touch set** the attempt
   recorded (below), checked first.
4. **Any interleaved write inside the touch set** ⇒ `fingerprint_mismatch`
   ⇒ nothing commits.

**The currency pass stays corpus-scoped.** The cost model changes what the
pass costs, never what it checks: the whole hash domain proves current before
the entry is served, and a poison member anywhere refuses the entry naming
the poison (Law A-3c). The pass trusts one thing: unchanged
`(device, inode, size, mtime, ctime)` means unchanged bytes, and unchanged
directory timestamps mean unchanged entries; the kernel bumps `ctime` on
every inode change and no API sets it, so a `utimes` restore is caught. This
`fs::domain_stat_signature` posture is evidence, not proof, and fails closed:
the commit's §5.1 guard folds from bytes under the write flock, so a memo
that disagrees with disk refuses `fingerprint_mismatch`.

**The entry world (the in-process lane).** On the wire `script` op
(wire-contract § A.7) the daemon evaluates the program itself. Four laws, and
only these, govern what a read sees:

1. **One pass, at entry.** Corpus-grain, as above: no doc-grain narrowing,
   no staleness window, no watcher; re-derived per attempt, never
   incremental.
2. **Reads serve the entry world, plus the program's own arms.** An unarmed
   target serves the entry bytes and rev; an armed target serves the entry
   bytes plus the program's own edits in arm order, and that content's rev
   — what you read is what is hashed (wire-contract §4.2), overlay included.
   Foreign mid-program changes are invisible: one attempt's reads span one
   state, the hash domain's. An out-of-domain path (wire-contract §12.1)
   stays addressable, served by a live single-file disk load outside the
   stand-still guarantee. Every read is recorded; eval is a pure function of
   (script, args, files, read-response sequence).
3. **Disk changes only at commit, and the commit guards the live world.** One
   splice, its premise the recorded touch set, verified entry-vs-live now
   (below).
4. **This is not the banned snapshot.** The ban is on daemon-held MVCC —
   versions retained across attempts. The entry world is attempt-scoped:
   born at entry, dropped at the answer, never retained or shared across
   connections, no as-of parameter. No daemon state survives the attempt.

**Rev threading under the entry world (the entry-rev law).** Every rev-less
row threads the target's entry rev unconditionally — file rev for a `props=`
row, section node rev for an `append` — off the pinned entry state;
recording does not gate it. An overlay rev is never a CAS token: §4.4 guards
resolve against the entry state and threading consults only the entry toc. A
target the entry state cannot name (an absent section) threads nothing and
meets the engine's target-class refusal. For every program, read-first or
not, an unmoved world commits and a moved touch set refuses whole at commit.

**The bracket is structurally satisfied on this lane.** A composed read is
bracketed by `file_rev` because 2+N trips could span states; in-process there
are no trips, so the A→B→A limit disappears. The bracket law
still binds any lane with trips.

A caller may pin its own `if_fingerprint?` guard, checked against the minted
entry fingerprint **pre-eval**: a mismatch refuses with zero evaluation,
read-class, `attempts:1`. That is a fast-fail courtesy; the commit re-checks
the same value as a widening premise (below). Before the compare, the pin
passes §5.7's grammar wall: a non-grammatical `Root`-family value —
including the reserved `absent`, §5.6 premise vocabulary (`guards[]`), never
an entry pin — refuses as a REFUSED trace (recovery `fix`) with the raw bytes
debug-quoted, so one leading space shows as a byte instead of a `conflict`
whose expected/live pair renders identical and loops a re-read. The CLI
entry and the wire `script` op refuse identically (wire-contract § A.7).

**The commit premise is the touch set; the frozen view is kept.**
Wire-contract § A.7 carries the full law:

- Authority is the **touch set** the attempt recorded (point reads, armed
  targets, pattern/selector expansions as set folds, sql provenance
  regions), verified entry-vs-live at exactly those nodes, O(touch set),
  not the whole-corpus entry fingerprint. A foreign write outside it never
  refuses; one inside refuses `fingerprint_mismatch` naming the moved
  premise's scope (wire-contract §5.7), on every script lane, MCP `script`
  included.
- Guarantee: *a committed script is consistent with exactly one state of
  everything it touched — what it read and what it wrote stood still, or the
  commit refused* (an uncommitted script still read one). Frozen-view reads
  (entry-world laws 1, 2 and 4) hold; the A.7 read-stability promise holds
  word for word.
- The caller's `if_fingerprint` widens only: strictest wins, never
  sufficient alone, never drops write coverage (the touch-set floor contains
  the armed writes). No host policy forces a token copy onto script doors.
- Retries spend only on same-subtree contention; foreign churn outside the
  touch set never refuses.

The entry is **single-attempt**: a conflict is one `fingerprint_mismatch`
with recovery `resync`. The host owns the retry loop (budget 2),
re-resolving a selector per attempt and re-running pinned `files[]` as
pinned; `attempts:N` is a host fact on the composed face, never a field of
the entry's trace.

**One commit per attempt (the set-form law).** An effect-less script's output
is a finite armed list, known before any I/O and validated whole before the
first byte moves. One armed path commits as the single §4.4 splice; N paths
as the §4.4 set form (`splice.set`): per-path plan groups in first-arm
order, one sealed validate-all-then-apply commit under the entry
fingerprint, one receipt entry naming every file, one Delta, one fingerprint
advance. A refusal anywhere lands nothing; inside a set a `no_match` on file
k is provably the program's own text (§5.2), the world guard having passed.
The receipt companion rides the same sealed set (§6.1). No arm-time
multi-file refusal, no single-path commit door; crash story: §6.5's set
paragraph (in-memory rollback, no journal, stated windows). Effects mode is
untouched (write-one was never its law): each live `put()` is one
single-path splice.

**Wire-client mode.** The script entry does its I/O as a wire client through
the one door, in one § A.7 `script` frame: the CLI sends the program; the
daemon pins the entry, evaluates in-process against the entry world, and
issues the one guarded `splice` carrying `actor`/`now`/`receipt` through the
same write choke-point. §4.4 is untouched — splice stays the only write op;
the `script` op arms one splice and embeds its response in the trace. The
only schema delta is the additive § A.7 op; one lane, one commit-premise
implementation. A daemon-side commit advances the delta ring, so the CLI
lane's missing-delta gap (wire-contract §18 row 12) does not extend to this
lane.

**The commit is guarded per row, and the consumer plane supplies the
tokens.** A wire door demands a fingerprint for every edit of existing
content, or an explicit `force` (`wire-serve::guard`): a `set_property` row
takes the **file** rev (frontmatter is file-scoped), an `append` row the
**node** rev of its section. The lane threads each token off the pinned
entry state at commit time, as the `put` face autofills. A token the entry
state cannot mint leaves the row untokened for the engine's own guard: the
degrade is loud, never a guessed token.

**No read-the-section-first ritual binds the author.** A `put()` target need
not have been read this attempt: **appends go rev-free for the author** (put
parity — append cannot clobber), **destructive rows are auto-guarded from
the entry state**, and **consistency enforcement lives at commit**. `force`
is not a script-plane door; one token law binds every door. Evidence:
`crates/mrd/tests/script_golden_live.rs` (every golden scenario against a
live daemon: each `plan_edits[]` row on the socket carries a token its own
reads published; the unread-target scenario pins the rev-free half) and
`crates/registry/tests/script_op.rs` (props and append with zero reads commit
on an unmoved world; a foreign edit after entry refuses).

**`--dry` is a rehearsal, not a commit.** The splice carries `dry: true`: the
daemon builds the whole effect set and applies none. The response
(`dry: true`, `fingerprint_after: null`) rides the trace as the commit leg;
the outcome is `no_effect` — no receipt, no fingerprint advance, workspace
unchanged, every armed entry `[not committed]`. A caller-guard refusal is
`conflict` with **no** commit leg and zero telemetry (no splice was issued,
so no §5.1 body exists); its extras ride the trace in band — `actual` is the
trace's `entry_fingerprint`, `expected` the caller's pin as
`guard_expected`, present on exactly this terminal. The face renders from
the trace alone, so `conflict` + no commit leg + `guard_expected` marks a
guard refusal, not a commit-time mismatch.

**The execution-model seam: arm, then commit.** The seam table's Authority
row names this entry's authority as *"the caller's own identity — `actor`
threaded per §9; **ownership guard + armed law**; no cap grammar."* The
ownership guard is the host's organ (the engine's splice is caller-agnostic,
§5.3), but `put(path)` computes its path inside the Starlark source, so the
write set exists only after evaluation, and one evaluate-and-commit call
never shows it to the host. Hence:

> **The MCP host runs the entry TWICE per attempt: once as an ARM (`--dry`),
> then, if and only if its own write-authorization plane admits every armed row,
> once as a COMMIT carrying `--expect-armed <the arm's armed_digest>`.**
> *(`--expect-armed` proves set identity; the touch-set verify, entry-vs-live,
> proves set freshness; a host-passed `--if-fingerprint` is widening only.)*

Four things follow, and only these four:

1. **Consumer-plane sequencing; zero wire delta.** Two ordinary invocations,
   the same five ops on the socket; the CLI gains exactly one flag,
   `--expect-armed` on the commit child (sub-amendment below), with no
   request shape change.
2. **Safe by construction, never by being fast.** A move inside the touch
   set between the legs refuses at §5.1 as an ordinary
   `fingerprint_mismatch` (host retry budget); a move outside never refuses.
   Correctness never depends on the gap being small.
3. **Recorded-read purity makes the arm's set the commit's set.** Every read
   is a touch-set member, so an unmoved touch set means an unmoved
   read-response sequence and identical arms; a move that does change the
   commit child's arms is caught pre-splice by `--expect-armed`. The arm is
   output, never a second decision.
4. **Parity with `put`, not a second policy grammar.** Same organs
   (`checkPutAuthz`, `checkContentWrite`), same per-target flock held across
   the commit child, same journal pipeline. The birth gate is not among
   them: a `put()` to a path with no file is refused at arm time
   (`file_not_found` on the rehearsal splice), before any row is classified,
   so no host birth decision exists; when this entry gains a birth door, the
   third organ gets its call site.

The CLI entry keeps its single-call shape: `mrd script` evaluates and commits
in one process, having no host identity plane to gate against. The law binds
the MCP `script` tool.

**Sub-amendment (the armed-set expectation, `--expect-armed`).** The seam
relies on reasoning that both evaluations arm identically; nothing compares
what the commit child armed with what the host gated, so the realpath the
authorization used and the addressable-vs-hash-domain gap stay unverified.
This sub-amendment makes that chain irrelevant:

> **The commit child accepts `--expect-armed <digest>` and REFUSES BEFORE THE
> SPLICE IS ISSUED when its own armed set does not hash to that digest.** The
> refusal is pre-splice: nothing is sent, nothing lands, no fingerprint advances.

Five things follow, and only these five.

**First — the digest is defined once, engine-side, and this is its whole
definition.** Let `rows` be the armed set the commit splice would carry: the
armed rows *after* rev threading, in arm order, each one an object

```
{"edit": <the plan_edits[] item>, "path": <the file it writes>}
```

whose `edit` is byte-for-byte the value of the request's `plan_edits` field
and whose `path` is the request's `path`. The digest is

> `armed-set-path-edit:` ‖ `sha256:` ‖ lowercase-hex( SHA-256( CANON(`rows`) ) )

where `CANON` is compact JSON: **object keys sorted lexicographically by
UTF-8 byte order**, no whitespace between tokens, RFC 8259-minimal string
escaping (only `"`, `\`, and control characters below `U+0020` are escaped;
every other code point is raw UTF-8). `effects::digest::armed_digest` is its
only implementation, with three callers: the arm, the commit, and the § A.7
op.

**Why the path is in the domain, not only the payload.** A `PlanEdit` carries
no path (the target rides `splice.path`), so a digest over `plan_edits[]`
alone hashes payloads, not the set: identical edits to different files hash
identically. A symlink re-pointed between the legs, or a pin covering the
hash domain while the write plane resolves through the larger addressable
set, would then pass and splice into a file nobody authorized. Pairing each
row with its target closes that by construction.

**The `armed-set-path-edit:` prefix is the digest's domain tag, and it is a
deployment organ.** A host cannot tell a payload-only digest from a set
digest by looking, and a resident daemon can run a stale engine for hours.
So the host asserts the **literal prefix by string comparison, no parsing**,
refusing an engine below the minimum by name — a capability assertion, not a
canonicalization: the host still copies one opaque string and computes
nothing (the courier property in *Second*). The tag names the domain, not a
version, so a refusal says what is missing; widening the domain means a new
tag, and hosts asserting the old one refuse loudly.

**Second — the host is a courier, not a second implementation.** The arm's
trace publishes the digest as a top-level `armed_digest` field; the host copies
that string into the commit child's `--expect-armed` and canonicalizes nothing.
Both children compute the digest with the same Rust function over the same
type, so a courier cannot invent a disagreement or pass vacuously.

**Third — the serialization is published anyway**, so an independent verifier
lands on the same bytes. `CANON` is exactly RFC 8785 (JCS) over a value whose
only number is `HpathSeg.n`. Three traps, each a silent false refusal on
ordinary markdown: a Go implementation MUST disable `SetEscapeHTML` (the engine
does not escape `<`, `>`, `&`), MUST NOT escape `U+2028`/`U+2029`, and MUST
decode with `UseNumber` so `HpathSeg.n` never round-trips through a float.

The **test vector** is pinned in `digest.rs::the_published_test_vector_holds`.
For the two-row armed set

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

Deliberate in that line: key order at **both** levels is lexicographic, not
Rust declaration order (`edit` before `path`; `body` before `hpath` before
`rev`); `<`, `>` and `&` ride raw; `HpathSeg.n` is **present**; the rows
target **different paths**. An implementation that drops or floats `n`, or
hashes the target once per set, fails here; one that reproduces this line
reproduces every digest.

**Fourth — the receipt is not an armed row, so the digest excludes it by
construction.** The receipt rides `request.receipt`, never `eval.armed`, and is
not in `plan_edits[]`. `--expect-armed` says nothing about it; its file is born
under the host's `receiptpolicy` pre-spawn gate.

**Fifth — the flag is optional; the CLI entry is unaffected.** Without
`--expect-armed`, `mrd script` behaves as before. With it, the check runs after
rev threading and before the commit is issued, beside the wall-clock pre-commit
refusal: nothing was sent. A mismatch is `refused` with fault class `refused`,
not `conflict`.

**Evidence.** `crates/mrd/tests/script_expect_armed.rs` asserts both directions
through a recording door: a matching digest commits; a planted mismatch yields
a socket census of `hello`/`fingerprint`/`toc`/`cat` and **no `splice` frame**,
so the refusal is pre-splice. The target dimension and the tag each hold both
arms there:

| Claim | Arm | Test |
|---|---|---|
| The digest reads the target | refuse | `identical_edits_to_two_targets_publish_different_digests` |
| | admit | `the_same_target_publishes_one_digest_across_runs` |
| The tag does not break the tool | admit | `an_ordinary_commit_still_commits_on_the_tagged_engine` — arm, forward verbatim, commit; the census asserts the splice was issued |
| The tag is assertable | refuse | `digest.rs::an_untagged_digest_is_distinguishable_from_this_engines` |

The first pair is also the **wire-observable capability probe**: the same
edits at two paths publish different digests. The capability is observed, not
inferred from a version constant.

The tag cannot false-refuse an ordinary commit: it is prepended inside
`armed_digest` itself, so arm and commit make the same call over the same type.

`cmd.rs`'s tests pin the rev-threading law: `guarded()` looks up a row's CAS
token **by that row's own `arm.path`**, so a child that resolved elsewhere
cannot inherit the gated file's rev.

**The trace — one commit-fact shape, and no `attempts`.** The entry returns a
`ScriptTrace`: entry fingerprint, outcome
(`committed | no_effect | conflict | fault | refused`), decision trace,
optional commit leg, optional fault, top-level `bindings`, and telemetry.

- **The commit leg is the §4.4 splice response, embedded verbatim** as raw
  bytes, never re-typed. Rev transitions, the receipt fact,
  `fingerprint_before/after` and `verdicts` (rules-as-data) ride it, so nothing
  drifts when §4.4 grows a field. A `fingerprint_mismatch` embeds through the
  same leg. Absent when no splice was issued (the read-class path).
- **No `attempts` field.** The entry is single-attempt; the retry loop and
  `attempts:N` are host facts on the composed face.
- **Telemetry is unconditional** — fuel, memory, reads, wall time — on faults
  and refusals too (the `RuleTelemetry` precedent).

The decision trace lists one entry per recorded read, in call order, then the
armed block in arm order. Entry kind is the statement-position rule: `echo` for
a top-level-statement read, `read` for every quiet position. Each armed entry
carries the wire plan-edit verbatim plus whether the commit landed it, so the
face's wrote-lines zip descriptor × result as put faces do. The fault taxonomy is closed at `parse | runtime | budget | refused`; a refusal is
not a fault, and the two must grep apart.

**A refusal carries the wire's refusal triple, typed.** The fault of a
`refused` run carries `code`, `recovery` and `reason` — the §8 error frame's
triple — and `recovery` is `wire::Recovery`, the closed six-class enum.

- **Never a fifth fault class.** Transient-vs-permanent is a property of a
  refusal, not a kind of fault; a `transient` variant would break every
  consumer matching `refused`.
- **Prose is a rendering, never the carrier.** `reason` keeps the engine's
  wording verbatim; a consumer reads the class, never the spelling.
- **One source, with a stated precedence.** The daemon's `error.recovery`
  wins; when a frame carries none, the class is the §8 frozen table's binding
  for its `code` (`ErrorCode::recovery()`), never a second copy of it. An
  unparseable code with no `recovery` yields absence.
- **Engine-minted refusals name their class explicitly**: `expect_armed_mismatch`
  is `fix` (re-arming is the caller's act); an elapsed wall clock before the
  commit is `retry` (nothing was sent). They carry no `code`, since no daemon
  could answer with an invented one.
- **The migration is additive.** `code` and `recovery` are optional, omitted
  when absent; a consumer matching `outcome: refused` plus `fault.reason` is
  byte-unaffected.

The daemon frame carries `recovery` and the put door reads it; a script path
that flattened the frame into `format!("{code}: {message}")` would destroy the
class, and **no host-side change can recover it**.

**A controlled failure exit speaks.** A run that fails before reaching
`CommitLeg` and leaves with prose on stderr and nothing on stdout would make a
deliberate refusal (the caller fixes it) indistinguishable from a process
killed mid-write (never resend).

**What is controlled — the definition, not a list.** A failure exit is
controlled when the process reaches its own exit door under its own control.
`mrd` has exactly one: `mrd::run`, whose `Err(Fail)` arm prints the diagnostic
and returns `fail.code`; there is no `std::process::exit` and no `abort` in
`crates/`. So **every** failure of `mrd` is controlled, and controllability
separates the engine from whatever killed it. What a controlled exit may say
depends on two questions:

1. **Does it hold the trace's premise?** The premise is `entry_fingerprint`,
   `ScriptTrace`'s first field (the §4.7 value). A path that failed before
   minting one may not synthesize one and may not speak a trace; its silence
   is contracted below.
2. **Does it know what the splice did?** A path holding the premise MUST speak
   and may assert only what it knows: never sent means nothing landed; sent
   with no answer means unknown, and it says so.

**The absence contract — what the survivor may rely on.**

- **Nonzero exit + a trace on stdout** — the engine answered; every claim in
  it, including `fault.recovery`, is the engine's.
- **Exit exactly 2 + empty stdout** — a controlled exit before the entry
  fingerprint existed: a bad invocation, an unreadable script, an unresolvable
  workspace, or no daemon. **Nothing was armed, no splice was issued, and the
  workspace is unchanged** — a guarantee, not a likelihood. The stderr
  diagnostic is for an operator; no consumer parses it.
- **Any other nonzero exit with an absent trace** — the engine did not choose
  this exit. The class is `resync`: a splice already on the wire is the
  daemon's to finish, so re-read, never resend. `--dry` narrows it to `retry`;
  a rehearsal writes nothing.

**A lost commit answer states its indeterminacy; it does not resolve it.** Of
the premise-holding doors, some sent nothing (`splice` refused with no error
body) — an ordinary engine-minted refusal, classed as above — and some cannot
know: `splice` never answered, a frame that would not parse, or an `ok`
carrying no body. **A reply that violates its own schema certifies nothing,
including its own `ok` bit**, so the last is unknown, not "landed but
undescribable". A door that cannot know may not use `no_effect`, `conflict` or
a bare `refused`: **all three assert that nothing landed**. It carries
`recovery: resync`, the killed-engine class, and states the indeterminacy in
band; a consumer reads the class, never the sentence.

The shape: `outcome: refused`, `fault.class: refused`,
`fault.recovery: resync` (or `retry` under `--dry`, as on the killed path), no
`fault.code`, and **`commit_unknown: true`** — present exactly when a splice
was issued and its outcome is not known. It is a field because an absent
`commit` already means "no splice was issued"; it is not a sixth outcome word
or a fifth fault class, because **committed-or-not-known is a property of a
run, not a kind of outcome**. These doors leave through the findings leg,
**exit 1** (`conflict`, `fault`, `refused`); exit 2 stays the bad-invocation
leg, which is what makes the exit-2 guarantee true.

**The shape is additive.** A consumer that reads the trace, or treats
`exit 2 + empty stdout` as "the engine did not answer", needs no change.

**The `mrd script` human-mode face is non-normative.** The MCP host owns the
normative text face, rendered from the trace; `mrd script --json` emitting the
trace is the contract between them. The CLI's human mode is an operator
convenience: two normative renderers would drift, and only the host knows
`attempts:N`.

### The two entries of the run plane — seam table

The mechanism is shared — sealed Starlark kernel, `md.*` descriptors, the one
write path. Only the entry differs.

| Axis | Task entry (`mrd run`) | Script entry (`mrd script` / MCP `script`) |
|---|---|---|
| Source | addressed fenced block in a governed page (`task.<name>: "[[#^block]]"`) — reviewable, rev-pinned, in the hash domain | inline source, caller-supplied per call; never lands in the tree |
| Authority | ambient — whoever invokes the page runs its task; caps declared in frontmatter, deny-by-default, root ceiling narrows only | the caller's own identity — `actor` threaded per §9; ownership guard + armed law; no cap grammar (caps = v2 delegation feature) |
| Entry point | `def run(ctx)` (one entry per plane) | module top level — the script is the body (kernel entry #3) |
| Languages | starlark + bash (fence dispatch) | starlark only; no exec, ever |
| Hermeticity | hermetic by construction: sealed kernel, zero I/O, `RunCtx` inert | recorded-read purity: eval is a pure function of (script, args, files, read-response sequence); the trace records every read; replay against recorded reads is byte-identical |
| Reads | none — inputs arrive as inert `RunCtx` data | one lane (§ A.7): `read()` serves in-process from the pinned entry world plus the program's own armed overlay; `mrd script` forwards the whole attempt and lowers no read of its own |
| Enumeration | page names its own targets | none in-kernel: host resolves selector (sorted) or binds caller `files[]` in call order — inert paths only |
| Commit | one atomic `if_fingerprint`-pinned batch via the local executor | one guarded commit as the caller (`actor`/`now`/`receipt` on the request): the single §4.4 splice for one armed path, the §4.4 SET form for N (§ One COMMIT per attempt) |
| Concurrency | `run.lock` `LOCK_NB`; `write.lock` bounded-wait at the door calls (§ Executor laws) | stand-still optimistic at touch-set grain: entry world pinned for reads (frozen view); commit premise = the engine-computed touch set, verified entry-vs-live — foreign churn outside it never refuses; conflict inside it ⇒ host re-resolves selector and retries (budget 2, `attempts` on the face) |
| Failure grain | one violation refuses the whole batch; bash phase-1 may stand committed and reported | one violation refuses the whole script; nothing partially lands, so a re-run never double-applies |
| Output | run record: stdout streamed + content-addressed out-of-tree log; receipt linkage via `ExecRecordSink` | `ScriptTrace` → text face: echo semantics, embedded §4.4 splice response verbatim, telemetry always present |
| Guarantee label | per block: `hermetic` (starlark) / `detected` (bash) | recorded-read + stand-still, stated as such; zero-armed outcome is read-class (`Ok(vec![])` precedent) |
| Daemon relation | local run beside a resident daemon = external change (accepted-gaps row, actor-absent) | wire client — writes arrive as governed change, actor-carrying, Delta-minted like any splice |
| Typical caller | operator / CI invoking a page's declared task | agent making a plan-shaped call over MCP (≥2 dependent steps with a decision between them) |
| Promotion | a task is already a convention — page-owned, addressable, armable | a re-sent script is a convention trying to be born: rewrite as `on_change(event)` (reads dropped, payload-only), then the arming ladder — registration ≠ activation, a reviewer arms |

Both entries converge on the one write path; neither may grow a private
executor. A task's authority comes from where it lives; a script's from who
sent it.

## The run entry, amended — load, freeze, fire

*(Wire face: `docs/wire-contract.md` § A.8. Code: `run::modes`, `effects::kernel`,
`run::blocks`, `run::caps`, `registry::run_op`, `mrd::run_cmd`.)*

The `run` op gains **`mode`**; a target without it is the shipped task target,
unchanged. Two modes join it behind the dotted caps **`run.mode`** and
**`run.input`**; an un-negotiated client is refused by name —
`` unknown field `mode` on `targets[0]` of `run` ``.

| Mode | Question it answers | Row it adds to `body.targets[]` |
|---|---|---|
| `load` | *what does this page declare?* | one row per anchored starlark block — `entry_kind` + its `declarations` |
| `fire` | *run the one block I name, with this input* | the entry's return as `value`, plus `applied[]` for the md effects it realized |

### Three phases, one globals set

**load → freeze → fire.** Load evaluates the block's top level, the module is
frozen, and a fire calls the frozen entry from a fresh evaluator.

The boundary is a **phase gate at the emission accessor**, not a difference in
bound names: `effects::kernel::hook_globals()` is one closed set for compile,
freeze and fire, because starlark-rust resolves globals at compile time.

| Phase | `declare()` · `exec()` | `bash()` · the md constructors |
|---|---|---|
| load | act — the declaration is the load's whole answer | bound, and **refuse**: `fault.class: effect_at_load`, carrying `fault.line` |
| fire | refuse: `fault.class: declare_at_fire` | act, under the page's `caps:` ceiling |

**A block declares once.** `declarations` on the load row is the uninterpreted
**dict** `declare()` collected (§ A.8): one dict, or `null`. The fire door
calls one entry, and the engine interprets no key of a declaration, so a second
`declare()` refuses **`fault.class: declared_twice`** at its own line, on that
block's row alone, siblings untouched. One anchored block per declaration.

These are typed faults downcast from starlark's `ErrorKind::Native`
(`EffectAtLoad` / `DeclareAtFire` / `DeclaredTwice`), so a phase violation is
never absorbed into `name_error`, which keeps its own meaning: an unbound
identifier. **Load purity is behavioral, not structural**: nothing effectful
happens at load, but the names are in scope.

### The consent gate — `run` executes what the page declares

> **`run` executes what the page declares: `task.<name>` in frontmatter or
> `declare()` in the block, never an undeclared block.**

- A fire naming a bare anchored fence refuses `not_declared` at the door
  (`run::modes`); a task-bound block's refusal names the task addressing
  instead, since a `task.<name>` block fires only as a task.
- A non-starlark fence addressed directly refuses `not_a_module`, naming its
  route: the starlark block that declares it with `exec(...)`.

**A prelude may not carry consent.** The `prelude` is caller source, evaluated
into the block's module at load, before its top level, where `declare()` and
`exec()` live. A prelude producing any declaration or `exec` value refuses
**`prelude_invalid`** at the mode door (`check_prelude` in `mode_row`, above
the load/fire dispatch) — before any block of the page is loaded, for load and
fire alike, whatever the page itself declares. A prelude declaration never
reaches the declaration list; `not_declared` stays reachable.

`prelude_invalid` is one class: the prelude's code faults, **or it carries
consent material — a declaration or an `exec` value — because consent is
page-authored**. The reason string names which.

The block enumerator `run::blocks` walks live anchors to their fences, beside
frontmatter `task.*` discovery; nothing re-implements addressing.

### Recording by declaration kind

**A fire row writes no receipt rows, and the fire process takes no task-path
lock** (the recording law above). Md effects go through the applier, which
takes the workspace lock unconditionally and can refuse workspace busy — the
pre-birth stage (A8; `modes.rs` → `executor::apply` acquires `WorkspaceLock`
first). A task row is unchanged; the engine reads the page, with no caller
flag. 100 declared-block fires add **zero** rows to `receipts/run.md` (anchors
`^r-<invocation>`, `^p-<invocation>`).

### Caps — the page's own ceiling

Declaring blocks have no task names, so `task.<name>.caps` cannot hold their
ceiling. `run::caps` reads a **page-level `caps:`** key — new grammar, not a
task binding — over the three md verbs `md.create` · `md.edit` · `md.delete`,
judged at the lifted `admit` choke point: a denial is `cap_denied` at the door,
nothing reaches disk, and a missing cap is never a silent no-op. The declaring
root's conventions ceiling narrows a page's caps and never widens them, as for
tasks. An exec'd entry's process is inert to it.

#### One spelling, both planes

Two planes read `caps:` with different vocabularies: the run plane's verbs
(`md.edit:tasks/*.md`) here, the policy plane's descriptor kinds (`proto.send`)
at the hook leg (`crates/policy/src/hook.rs`). **The spelling has one owner**,
`model::parse_caps_list`, read off the frontmatter block by `model::fm_caps` /
`model::fm_doc_caps`; these all declare the same two caps on **either** plane:

```yaml
caps: md.create, md.edit          # plain scalar, comma- or space-separated
caps: [md.create, md.edit]        # flow sequence, items optionally quoted
caps:                             # block sequence
  - md.create
  - md.edit
```

`caps: []` is the empty grant; a bare `caps:` declares nothing — a refusal on
the policy plane, deny-by-default on the run plane. Gate:
`crates/testsuite/tests/caps_one_grammar.rs`.

One key, one reader, or the two drift in opposite directions: on the flow
sequence the run plane faults `invalid capability '[md.create'`, on the plain
scalar the policy plane refuses `invalid type: string`, and a block sequence
reaches the run plane as the empty string — a silent read-only grant.
`model::fm_tags` holds the same law for `tags:`.

### The world a mode-bearing row runs against

`load` and `fire` take the **pinned resident snapshot** (an `Arc` clone, the
one the script op takes) and never run the `domain_snapshot` fold. The world is
a parameter (`run::modes::ModeWorld`): one implementation serves the daemon and
the CLI, so the two lanes cannot answer differently.

On a **cold workspace** the answer is per lane:

- **daemon lane** — the same § 3.2 cold gate as the script op: it **refuses
  `corpus_warming` (retry)**, neither blocking nor bypassing;
- **CLI / in-process lane** — no background substrate to warm on, so it
  **builds the drawer inline** and the caller waits.

`prelude` is one per call, cap `run.mode`, and an invalid one refuses
`prelude_invalid` (§ The consent gate). Declarations and frozen modules are
cached per block rev, keyed with the prelude's blake3, so an unchanged block is
served from cache — what the fire p95 rests on.

### Amendments this implementation carries (A7, A8)

**A7 — the world is a parameter, not a second implementation.**
`run::modes::ModeWorld` carries as **borrows** the page, the workspace root,
the declaring root, the observed corpus root, the caller's `prelude`, the
door-side facilities and the module cache; one `load_row`/`fire_row` serves
both lanes. The daemon hands the page out of its pinned snapshot with a
resident cache; the CLI hands the page it just loaded, with `cache: None`.

**A8 — the `applied[]` row vocabulary.** The wire emits
`born|edited|refused|not_applied`.

- `edited` — a `set_field` is not a birth, and the same row's rev would
  contradict `born`.
- no `exists` — an occupied path **refuses** at the create door (the note
  below spells it).
- `not_applied` — the page splice is atomic, so a sibling edit may not claim
  `born` for a record not on disk.

> **A8's touch set reaches past the engine.** A caller that needs "already
> there" to be benign keys on **`cas_mismatch`** with
> `expected` = **`absent_rev`, the `node_rev` of the
> EMPTY DOCUMENT** (`wire-serve/src/write.rs`: `AlreadyExists` →
> `cas_mismatch(&absent_rev(), &occupant)`; `absent_rev()` is the root rev of
> `model::build(String::new(), syntax::parse(""))` — a computed blake3 value,
> not a nil hash and not an empty string — measured `af1349b9f5f9a1a6`).
> `cas_mismatch` also spells the create-CAS, the drift/remove-CAS and the
> splice verdict, so **only `expected == absent_rev()` discriminates** — the
> field, never the call site.
>
> The token is **`wire::ABSENT_REV`** and the comparison
> **`ErrorBody::is_path_occupied()`** — one spelling for three Rust consumers
> (`preset::birth`, `realise`'s card mint, the wire-serve gates) and any wire
> client mirroring it. A wire-serve test asserts the constant equals the
> computed `absent_rev()`, so a domain-rule change that moves the empty
> document's rev fails the build. The guard plane's
> `AlreadyBorn` is a benign already-exists too, but carries no `expected` and
> is a splice-path refusal the create door never mints, so
> `is_path_occupied()` reads it false.

### A door refusal is that effect's row — never the fire's

**The rule.** A **door** refusing one descriptor is that descriptor's own
`applied[]` row: `result: "refused"` with the door's class and reason. The
**fire row keeps `result: "ok"` and keeps its `value`.** The page splice stays
atomic; sibling rows are positional on the refused descriptor's index, and a
create before that index reads `born` — births realize sequentially ahead of
the splice and never roll back.

That is the never-veto law made operational: a `PreToolUse` hook's
`{"deny": "…"}` still reaches the caller when its own append is refused, and a
daemon checking `result == "ok"` before reading `value` would drop it.

"Refusal" names two situations:

| A door said no → the EFFECT's row, fire row stays `ok` | The engine could not carry the batch → the FIRE row refuses |
|---|---|
| `cap_denied` · a birth the create door refused (occupied path, bad path) · an **armed-middleware veto** · a section that is not there or is there twice · an fp-claim · a verdict refusal | the workspace lock is held · I/O · a page that will not load · a non-md descriptor reaching the executor · a malformed descriptor |

The refusal names its descriptor by `ExecError::descriptor_index`.

A refusal naming **no** descriptor renders **by stage**, because the splice
runs after the birth lane:

| stage | creates | edits |
|---|---|---|
| pre-birth (the workspace lock — taken before anything runs) | `not_applied` | `not_applied` |
| post-birth, no descriptor named (page load, splice I/O) | `born` — the birth lane completed, or its refusal would carry that birth's index | `not_applied` |

`refused` is **reserved for the descriptor a door judged**; in both
engine-failure stages the fire row refuses.

### Ceilings, and what a caller can narrow

`timeout_ms` and `budget {steps, mem}` on a mode-bearing target are the
caller's **ceilings**: **effective = min(declared, ceiling)** in every axis. A
caller narrows, never raises, and an absent field leaves the engine's ceiling
standing. `budget` is the evaluator's fuel and memory limit; `timeout_ms`
reaches every process the fire starts — the exec'd entry and each `bash()`
call.

`env` on a fire is an **exec'd entry's process overlay**: the target's `env` is
the base, where a daemon's `CCC_HOOK_*` scalars ride opaque, and the declared
`exec(env=)` pairs overlay it. On an **evaluated**-entry fire it **refuses**
`bad_request` (no process to receive it) — a row, not a decode-wall refusal,
since the entry kind is a page fact.

The engine's own facts reach an exec'd entry as `MRD_RUN_PAGE`,
`MRD_RUN_BLOCK` (**the declaring block's anchor** the caller addressed, not the
fence `exec(block=)` points at) and `MRD_RUN_INVOCATION`; its working directory
is `input["cwd"]` when given, else the page's root.

**`exec(block=)` resolves at LOAD**, not at the call: the declaration is the
program. A missing fence faults `no_block`, carrying the anchor's own words; an
anchor minted twice faults the typed `ambiguous_anchor`. Both are **load-row**
faults, shown by `--load` before any fire, and both are judged again at the
fire door.

**An exec'd entry's program is a STAGED FILE.** The bytes are written once per
block rev under `.meridian/staged/<rev>[.<token>]`, and the process is
`<interpreter> <staged-file> <args…>`.

- `-c` is a **shell** convention, not a contract for a plane whose law is
  *a new language is `argv[0]`, not a concept*: `node -c '<source>'`
  reads `-c` as `--check` (`MODULE_NOT_FOUND`, exit 1), `bun -c` answers `File
  not found`, and `deno`'s `-c` is its config-file flag.
- The extension is the fence's own info-string FIRST token (the classifier
  `fence.rs` already has): bun and deno pick a loader from the file name.
- The staged file is the cache — *staged bytes cached by block rev*: a second
  fire of an unchanged block writes nothing. The cache removes the read and the
  write, never the spawn.
- `$0` is the staged path, not `mrd-task`; stdin, env, cwd and the exit
  contract are unchanged, so *the script's bytes run unchanged* holds.
- **The shipped task path keeps `-c` and `$0 = mrd-task`, byte for byte**
  (`run::exec::ExecSpec::task`).

**Recording has a ceiling.** Logs live at
`.meridian/runs/<page-path>/<invocation>-t<index>.log` for an exec'd entry and
`…-t<index>-b<n>.log` for the n-th `bash()` call of that target — a directory
**per page**, retention the last 50 per page. The TASK path's logs stay at the
top level of `.meridian/runs/`, written by `run::record` and pointed at by a
run receipt. An `exec[]` row publishes `stdout_sha256` + `bytes` +
that `log` path, never the stream inline. A `process` object is bounded the
same way: `stdout_tail`/`stderr_tail` are the **last 4096 bytes** of each
stream, `stdout_bytes`/`stderr_bytes` the full sizes, and the `log` carries all
of it. The dict the PROGRAM sees still carries `stdout`/`stderr` inline.
Nothing is staged and nothing is logged under `dry`.

### Input and answer

**Which def a fire calls.** `declare(impl = f)` names it; with no `impl` the
conventional entry is **`run`** (`effects::kernel::DEFAULT_HOOK_ENTRY`), and
whether the module defines it is the freeze's business (`missing_entry`), since
a block may declare before it defines. `impl` is resolved at the `declare()`
call while the value is still live: a callable is the evaluated entry, an
`exec(...)` value a process entry, anything else the `impl_type` fault.

`input` (cap `run.input`) is JSON, converted to starlark at the call; the
entry's return converts back to JSON as `value`. **`None` is no answer**: no
`value`, never `"value": null`. A birth row's `file_rev` names **the born
file's** rev, not the page's, and carries the born `path`.

## Addressing (§2.1 grammar, no new syntax)

A page declares tasks in frontmatter: `task.<name>: "[[#^block-id]]"` binds a
task name to a same-file fenced code block; `task.<name>.caps` / `.args` /
`.env` carry its capability declaration and input contract.

- `.args` names positional slots in order and the count is exact. The **last**
  name may carry a `...` tail (`task.fmt.args: title, rows...`): earlier names
  stay fixed slots, the tail takes every remaining arg, zero included; both
  dispatchers already consume a positional list (bash argv, starlark
  `ctx.args`).
- `.env` is supplied by name and refuses the suffix.
- Cross-file refs are a **non-goal** and refuse with a typed error.
- Every addressing fault is distinct and pre-eval: no such task, dangling
  binding, ambiguous anchor, not-a-code-block, unknown fence language,
  cross-file ref.

**A binding fault is scoped to its own row.** A binding VALUE is validated when
its own task is addressed, so `mrd run PAGE TASK` always answers TASK's fault,
never a sibling's. `--list` renders every declared row and prints a faulty
row's typed error in place of its language and caps.

The one page-eager guard is the task **NAME** charset (§2.4): a key outside
`[A-Za-z0-9-]` refuses the whole page, `--list` included. The reason is forgery:
a name is stamped verbatim into every run receipt as `task`, and as the actor
`run:<name>` when the request supplies no actor (§9).

## Capabilities — deny-by-default

An undeclared block is read-only: it computes, but no effect of its executes.
Three cap verbs answer *may this block touch files there* — `md.create` /
`md.edit` / `md.delete` (not the birth-preset *three verbs* of § 4); *how* it
touches them is the descriptor plane's (executor ops). `Create` needs
`md.create`; `SetField` and `AppendSection` need `md.edit`. `md.delete` is
reserved: it parses and resolves, so grants can be written ahead, but no
descriptor maps to it until a retire descriptor exists.

⚠️ **This section governs starlark blocks only.** Caps do not apply to bash
(`laws.md`): a bash task resolves `Authority::Unsandboxed`, its
`task.<name>.caps` is never read, and a present-but-empty declaration grants
nothing. A bash fence may rewrite any file; the engine detects that in the
exec-window bracket rather than denying it.

**The two live verbs differ in reach, and that decides your glob.** `Create`
births a file the block names: a real *where may I write* grant. `SetField` and
`AppendSection` change only the declaring page (`descriptor_surface`,
`crates/run/src/executor.rs`), so an `md.edit` scope is a **self-guard** on the
block's own coordinate: `md.edit:agents/*/CARD.md` admits nothing on a page
that is not an agent card.

A verb may carry a path glob in the one glob grammar (`policy::glob_match`,
`crates/policy/src/declaration.rs`; caps call it, never reimplement it),
matched at the choke point against the block's **declared** coordinate. Cap
scopes add one restriction: every segment non-empty, never `.` or `..`, from
letters, digits or `_ - . * =` (`bad_glob`, `crates/run/src/caps.rs`); outside
that charset a scope refuses at declare time, even where a rule or hook would
accept it.

| Descriptor | Coordinate the glob judges |
|---|---|
| `Create` | its `path` argument verbatim; the resolution base (descriptor `base` > frame `ambient` > workspace root) is a separate axis |
| `SetField` · `AppendSection` | the declaring page's path, minus the frame's `ambient` as a literal prefix when the page lies under it; otherwise the full workspace-relative path |

⛔ **A create scope constrains the shape of the declared path, not where the
bytes land.** The choke point never reads `base`, so `md.create:tasks/*.md`
alone lands `tasks/<slug>.md` under any confined directory.

- The grant admits every confined landing — measured:
  `conventions/attested/tasks/x.md`, `receipts/tasks/x.md`,
  `meridian/tasks/x.md`, `.meridian/tasks/x.md`, `.git/tasks/x.md`.
- Refused: the last four at the machinery floor below; `..`, absolute paths and
  foreign roots at the path law.
- Jailed by the glob: the tail only (`evil/tasks/x.md`, `tasks/sub/x.md` fail
  it as declared paths).

So `run.caps.fix-*: md.create:tasks/*.md` does not confine births to boards
(boundary-as-data: the engine holds no layout pattern); put content containment
in the block.

🛡 **The machinery floor.** Four names are engine substrate, not layout. The
create door refuses any birth whose **resolved landing** carries one as a path
segment, at any depth, ASCII-case-insensitively, whatever the caps admit:
`bad_path`, naming the segment, nothing written.

| Segment | What it is |
|---|---|
| `.git` | the git directory — a birth here can corrupt the repository |
| `.meridian` | engine stable state and run logs (`.meridian/runs/`) |
| `meridian` | the attestation tree (`meridian/armed-rules.md`, `meridian/attested`) |
| `receipts` | the receipt ledger (`receipts/run.md`, `receipts/realise.md`) |

**One carve-out, `meridian/domain.md`**, exempt at any depth: the hash-domain
config declaring the ignore list is authored content, and the resident write
path births it through this door. **Stated limit:** a block with a matching
`md.create` scope can reach it through its own `base` and reshape what the
workspace attests; `actor` is caller-supplied, so the door cannot tell it from
a human, and closing the hole needs a policy axis this guard lacks.

The floor judges the **landing**; caps judge the declared coordinate alone. One
door owns it: starlark `create()`, the wire `create` op, the birth preset and
the realise card mint.

**The door also serializes the newborn's frontmatter.**
`create(path=, body=, props=)` takes `props` as a dict of string keys to
strings or lists of strings and composes the block with the shared encoders
(`yaml_safe_key`, `yaml_safe_scalar`, `yaml_safe_flow`); no block carries its
own escaper.

| The block writes | Lands | Because |
|---|---|---|
| `props = {"status": "owner: [[x]] \" #now"}` | `status: "owner: [[x]] \" #now"` | it would read back as a key, a comment or a quoted scalar |
| `props = {"tags": ["type/agent"]}` | `tags: [type/agent]` | a list is one line of flow; a member with `,` or `]` quotes |
| `props = {"n": "7"}` | `n: "7"` | `props` is a string plane: anything a YAML parser would read as a number or a bool quotes, at every door (§ A.6.3 — a plain all-digit short id like `19895504` reads as an integer). No integer spelling here; `PropValue::List` is the one typed arm |
| `props = {"owner": "02146210"}` | `owner: "02146210"` | an all-digit 8-hex short id is a string; plain, PyYAML reads octal 576 648 and the join key is gone |
| `props = {"x": "[a, b]"}` | `x: "[a, b]"` | this door has a list arm, so a scalar never becomes a collection — **the one asymmetry with the patch face**, where the same string lands plain (`yaml_safe_scalar` vs `yaml_safe_value`, wire-contract § A.6.3) |
| `props = {"bad": "a\nb"}` | REFUSED, nothing born | a v1 frontmatter value is single-line; refused, never sanitized |
| `props=` plus a `body` opening `---` | REFUSED, nothing born | two spellings of one block; pass one |

Keys land sorted: a birth must replay byte for byte. Armed middleware still
stamps `created`/`session` here from the put frame's `fields`; `props` is
composed into the body first and never reaches them, so a fill-if-absent rule
sees the caller's keys. The armed artifact (`wire_serve::armed_disk`), the
receipt (batch commit) and run logs (plain I/O) never pass this door.

**Spelling an edit scope.** `md.create:tasks/*.md` covers the ambient board, a
based (`--target`) board and the root board alike; edits are not symmetric.
`ambient` is a frame field (cap `run.ambient`) the host attaches per call, so
where a host sends none an edit is judged by its full workspace-relative path:
a short `md.edit:tasks/*.md` then denies a card at
`year=…/<session>/tasks/x.md`. Spell `md.edit:**/tasks/*.md`, which holds
either way (`**` matches zero segments too). **Do not take the spelling the
denial suggests**: its `Fix:` line, built from the denied page's own path, is
session-pinned (`md.edit:year=2026/month=08/<session>/tasks/*.md`) and denies
every card next session. It guarantees legality only — every cap it prints
round-trips through `Cap::parse`. Where no scope can name the coordinate (a
rooted spelling, a segment outside the charset) it says so and offers the
unscoped verb.

Several scopes are several comma-list entries; no new syntax. A scoped cap is
strictly narrower than its bare verb (`md.edit:**/tasks/*.md` < `md.edit`).
Declare beside the binding, or by name convention. **The two examples below are
one working pair: the ceiling must carry every verb the page declares** (a
ceiling that omits a verb drops it whole):

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
name: notes
run.caps.fix-*: md.edit, md.create:tasks/*.md
# longest pattern wins — and a comment needs its OWN line (see the
# bricking hazard below)
run.caps.fix-note: md.edit:**/tasks/*.md
run.timeout_secs: 7
---
```

⛔ **One bad entry in this table bricks the whole root.** The table loads before
authority resolution, so one unparseable value refuses every run on that
root — read-only tasks, `check-*` tasks, bash tasks (otherwise ungoverned by
caps), even `mrd run <page> --list`. Three causes:

- a trailing comment: the frontmatter scanner takes no YAML crate, so
  `md.edit:… # longest pattern wins` parses `#` as a cap; <!-- caps-gate: refuses -->
- a bad verb: anything outside the three;
- a glob outside the cap-scope charset above, e.g. `md.edit:tasks/x!y/*.md`. <!-- caps-gate: refuses -->

The refusal reads `refused: <path>: <key>: <what is wrong>`, states the
whole-table blast radius, and names the trailing-comment cause. After editing a
ceiling, run `mrd run <any-page> --list`; it fails loudly on a bricked table.

**Legacy per-op spellings fold or refuse, never silently reinterpret a
target.** Bare `md.set_field` / `md.append_section` alias-fold into `md.edit`
at parse; reports and refusals then name the canonical form. Their field-grain
targeted forms (`md.set_field:status`) refuse: the target named a field or <!-- caps-gate: refuses -->
section, the target position is a path glob, and dropping it would widen the
grant. Field-grain guards live inside blocks; partition grain (parent-dir-name
match) is not a grammar.

⚠️ **The fold preserves execution and widens the op axis.** `md.set_field`
spelled a field-write-only grant; folded to `md.edit` it authorizes every
page-mutating descriptor, `md.append_section` included. Re-guard inside the
block; the cap plane has no op grain.

`caps: []` is an explicit read-only grant; no declaration and a bare `caps:`
are neither (the engine never invents `[]`). Precedence: explicit > convention
> none. Conventions **narrow only, never widen**; every cap not surviving
intact is reported in `narrowed[]`. **Scopes meet by glob containment**
(`Cap::meet` → `policy::glob_subsumes`): segment-wise `**`/`*`/literal
subsumption, conservative — an unprovable containment reads as incomparable and
drops, so the meet can only drop. Five ways:

| Page declares | Result under ceiling `md.edit:tasks/**` |
|---|---|
| a scope inside the ceiling — identical or nested (`md.edit:tasks/foo.md`, `md.edit:tasks/sub/*.md`) | survives intact |
| the bare verb, unscoped | replaced by the ceiling's scope (`md.edit:tasks/**`), in `narrowed[]`; no full reach |
| a scope containing the ceiling (`md.edit:**`) | tightened to the ceiling's scope, in `narrowed[]` |
| a scope neither inside nor containing it — disjoint (`md.edit:notes/*.md`) or overlapping without nesting (`md.edit:*/foo.md`) | dropped; overlap is not nesting |
| a verb the ceiling does not name | dropped whole — a ceiling allowlists verbs too, so `run.caps.fix-*: md.edit` kills every `md.create` on a `fix-*` task |

⛔ **Keep `md.edit` ceilings unscoped; scope `md.create` instead.** Edits are
self-guarded, so a scoped edit ceiling is an on/off switch keyed to where the
page lives: under `run.caps.fix-note: md.edit:**/tasks/*.md`, a `fix-note` task
on `rules/escalate.md` is denied whatever it declares, the bare verb included;
renamed under an unscoped `fix-*` entry it applies cleanly.

Bare `md.edit` resolves to `md.edit:tasks/**` with
`narrowed by ceiling: md.edit`. The builtin `check-*` / `verify-*` ceiling is
absolute; those names refuse a bash fence loudly at load. Caps bind at the
executor choke point before any I/O: one violation refuses the whole batch.

**The denial names the ceiling that ate the grant.** `narrowed[]` reports
narrowing on the listing face; a `capability denied` refusal names the winning
`run.caps.<pattern>` convention entry, or the builtin `check-*` / `verify-*`
ceiling.

⛔ **Only when the ceiling is measured.** A denial no ceiling caused —
deny-default, or a grant that never held the cap — names the cause and stops;
the engine attaches no remedy to an unmeasured cause.

The deny arm teaches one measured remedy, the **partition-grain respell**:
where a declared globless same-verb scope `T` would have covered the declared
coordinate as `T/*.md` (the refusal says "landing", meaning that coordinate),
it names the respell — `md.create:tasks` is a literal glob matching only the
path `tasks`, so the page is told to spell `md.create:tasks/*.md`. Taught only
on a measured match. Texts: `ExecError::CapDenied`
(`crates/run/src/executor.rs`); parse-time refusals (unknown verb, bad glob,
field-grain target) are `CapsError` in `crates/run/src/caps.rs`.

### Where the convention table lives

**The root declares.** The table is read from the root's own `MERIDIAN.md`
self-declaration (`type: meridian-root`; the config charter's *"the root
declares, `MERIDIAN.md` binds"*) through `crates/config`, which owns valid
declarations. No other marker file, no fallback.

**A rooted invocation's declaring root is the page's tree (address-grammar
§ 4.6).** `mrd run root:page` behaves as if the caller had cd'd into that root:
the table loads from that root's own `MERIDIAN.md`, the ceiling is that tree's,
the receipt lands in that workspace. The runtime cwd and the standing workspace
are not factors, which closes the ceiling-by-cd bypass for starlark. Bash holds
no cap ladder (caps do not apply to bash, `laws.md`); its only fence is the
builtin name-keyed `check-*`/`verify-*` refusal, travelling with the page.

The grammar is the page grammar reused: flat dotted frontmatter keys carrying a
cap list. The key must be flat — the pattern is the `<pattern>` in
`run.caps.<pattern>`, which a nested `run:`/`caps:` mapping could not carry.
The value is the ordinary cap list, in any of the three spellings § *One
spelling, both planes* names. A bare `run.caps.<pattern>:` declares the empty
ceiling, never an absent entry — fail-closed.

Which root answered is never silent (`ConventionSource`):

| Root situation | Conventions | |
|---|---|---|
| declares, with `run.caps.*` | that table | the ceiling is in force |
| declares, none stated | empty | `Declared` — deny-by-default stands |
| holds no `MERIDIAN.md` | empty | `Undeclared` — absent is not broken |
| present, not a valid declaration | **refuses** | an unreadable policy file never becomes *no policy* |
| no root resolved (`CwdDefault`) | empty | `NoRoot` — **no ceiling in force**, stated |

`config::mount` greys the same bad read instead — blast radius, not
strictness: a mount table isolates one bad root, while
the run plane holds one.

Resolution law and the declaration parse contract: `crates/run/src/caps.rs`;
design tests: `crates/run/tests/caps_home.rs`.

## Fence dispatch — two languages, one write path

Dispatch is by fence language: `starlark` → hermetic kernel eval; `bash` →
exec in the **invocation cwd**. The set is closed; there is
**no `Exec` EffectKind**.

**This whole section is the TASK path**: its two-phase receipts, phase-2
convergence and `OutOfBand` refusal are a `task.<name>` row's, not a fire
process's, and the lock is the one split. An **exec'd** `declare()` entry shares
**the same bracket** (`run::exec::exec` over `ExecSpec`), then parts company; an
**evaluated** entry spawns nothing unless the program calls `bash()`, so these
rows are the exec'd entry's:

| | task row (`task.<name>`) | fire row (`declare()`) |
|---|---|---|
| receipts | phase-1 + phase-2 rows in `receipts/run.md` | **none** — 100 declared-block fires add zero rows |
| `.meridian/run.lock` | taken | **the fire PROCESS takes no task-path lock**; md effects go through the applier (`executor::apply`), which takes the workspace lock unconditionally and can refuse workspace busy — a `runtime` fault (pre-birth stage, A8) |
| program | `bash -c <source> mrd-task <args…>`, `$0` = `mrd-task` | `<interpreter> <staged-file> <args…>`, `$0` = the staged path |
| stdin | `Stdio::null()` | the fire's `input`, compact JSON |
| exit | collapsed to `state: applied\|partial` | the **raw** code, 1 and 2 distinct |
| stderr | captured, read by nothing | `stderr_tail` on the row |
| record | the receipt + `.meridian/runs/<invocation-id>.log` | `.meridian/runs/<page-path>/<invocation-id>.log`, and the row |
| language set | closed (`starlark`, `bash`) | `argv[0]` — any interpreter |

A fire applying nothing — exec'd, or evaluated with no effect — and any `dry`
fire take no lock. Recording follows the **declaration kind**
(§ Recording by declaration kind); no flag and no caller selects it. A fire's
process is recorded, not receipted; its laws are § The run entry, amended.

A bash step runs where `mrd` runs; the supervisor never relocates it, and the
caller-minted out-of-tree scratch directory is only the artifact location.
`$MERIDIAN_PROJECT_ROOT` carries the project root. On the wire arm (§ A.8) the
step runs in the bound workspace root — a daemon has no meaningful cwd —
narrower than the CLI's.

A step **can** write into the tree: the bracket detects it as `OutOfBand` and
**phase 2 refuses to converge**, writing no completion receipt. Governed writes
ride the wire faces (MCP `put`) or a starlark task. Gates:
`crates/run/tests/dispatch_bash.rs`
(`an_ungoverned_tree_write_refuses_phase2_with_the_delta_named`,
`a_project_root_relative_stray_write_refuses_convergence`).

Both paths converge on the **shared executor**, the one write path:

```
md.* descriptors → block-cap validation AT THE CHOKE POINT
 → ONE atomic if_fingerprint-pinned splice batch
 → receipt in the same commit
 → apply→event synthesis (real post-apply fingerprints)
```

Executor laws:

- One violation refuses the whole batch; a refusal applies **nothing**.
- **Never roll back**: ungoverned writes persist as actor-absent external
 change (§7.1 class).
- `live_fingerprint` is the **computed** post-phase-1 fingerprint, threaded by
 the caller, never re-read around a bash step; missing at a bash choke point it
 refuses — enforcement-off is not a pass.
- Local runs serialize under `.meridian/run.lock`, `LOCK_NB`: a held run lock
 is a fast typed "workspace busy" refusal.
- **The `write.lock` leg waits, bounded.** Every write door takes
 `.meridian/write.lock` `LOCK_EX|LOCK_NB` and refuses a competing writer in
 ≤0.1 ms, with no queue and no engine retry (wire-contract § the batch bound).
 The run plane is a caller of those doors, and waiting is its policy: its two
 in-process acquires — the birth lane's `create` call, the
 delta-mint bracket — retry a `workspace_busy` refusal every 10 ms until
 `MERIDIAN_BUSY_WAIT_MS`
 (default 10 000) is spent, then surface the same typed refusal. `run.lock` is
 uncontended and gets no wait. Lock order stays `run.lock` → `write.lock`.
- **No foreign-edit gate.** No replace-class effect is compared against a prior
 receipt's after-rev, and no takeover flag exists — a per-target pin-and-verify
 is an unkeepable premise guard (the no-guard amendment at the top of this
 document). CAS covers races at the write door, nothing wider.

## Bash: two-phase receipts inside the enforcement bracket

A bash step runs as pre-exec receipt (phase 1) → exec → completion receipt
(phase 2), inside the detection bracket:

- The child runs in its own process group (`setsid`) under a wall-clock
 timeout. Timeout SIGKILLs the group and is a distinct typed state; background
 children die with the group at step end.
- Bash has **no governed-tree effect channel** — no descriptor fd, no in-band
 `md.*` records. Phase 2 commits the completion receipt only, always an empty
 batch.
- `domain_snapshot` residual-compare runs around **every** bash step: expected
 post-step root = pre-step files plus this step's governed edits; any residual
 delta refuses and is named.
- The **domain config** is hashed separately around every bash step
  (`meridian/domain.md`, or a sole legacy `mdfs_config.yaml` when that is the
  only file present). A mid-run change refuses, closing the config-widening
  attack. See `wire-contract.md` §12.
- An interrupt between the phases is a typed `partial`/`interrupted` state;
 the pre-exec receipt records phase-1's committed root so lint finds orphans.
 On exec failure phase 2 refuses, **phase 1 standing committed and
 reported**.
- An out-of-band delta is reported as *"out-of-band change during the exec
 window"*.

## The run record — stdout is data, not effects

Bash stdout never becomes a tree write: it is **streamed live** to the caller
and **stored out-of-tree, content-addressed** at
`.meridian/runs/<invocation-id>.log`, addressed by invocation id and pinned by
the full sha256 in the receipt. Tree output happens **only** via an explicit
`md.append_section` descriptor; no in-tree run journal exists.

Record ↔ receipt linkage:

- Receipt line: in-tree, in the same splice batch as its edits, carrying
 per-edit rev transitions — **attested history, compared by nothing** (the
 no-guard amendment above).
- Receipt `page` fact: the task page's **canonical workspace-relative spelling
 in the workspace that runs it** (wire-contract §2.1), resolved once at the
 admitting door, never argv bytes — one page owns ONE receipt history however
 spelled. Consumers: receipt address, page addressing; CLI ruleset empty
 (`S1_RULES`). A ROOTED ref (`root:page` — address-grammar § 4.6) runs under
 that root's tree and its receipt lands in that workspace, where the page HAS
 the spelling: one-key law unchanged.
- Exec record: **invocation id + exit code + stdout sha256 + byte size + log
 address**, joined to the receipt through the `ExecRecordSink` seam.
- **Ordering is structural:** stdout facts are minted only by sealing the log,
 which fsyncs it and its directory entry first. A crash can orphan a log (lint
 finds it), never produce a receipt naming a non-durable log.
- Env **keys, never values**: the record takes the contract-validated
 **declared** map only and emits its sorted key list. The child's real
 environment is larger (`wire-contract.md` § A.8) — daemon environment under
 the declared overlay, plus the plane's injected variables — and no inherited
 key reaches the record; the constructor discards values, so no secret can
 enter the type.

## The CLI surface (locked)

```
mrd run <PAGE> [TASK] [-- ARGS] --env K=V --dry --list --json
mrd run <PAGE>#^<id> [--input-json FILE|-] [--dry] [--json]   # fire one declared block
mrd run --load <PAGE>... [--json]                             # what the pages declare
mrd script [--json]                      # source on stdin (heredoc)
```

**The locked surface is exactly these shapes**: one verb in three shapes —
run a task, fire one declared block, read declarations off one or more pages
(§ The run entry, amended) — plus one further subcommand, `mrd script`: source
on stdin, serving the script entry above, its human-mode face non-normative
(§ The script entry). Every meaningless combination refuses BY NAME with
exit 2, never ignored: `--load` with `#^<id>`;
`--load` with `TASK` / `-- ARGS` / `--list`; `--input-json` without a block
address; `TASK` / `-- ARGS` / `--env` on a fire, whose one input channel is
`--input-json`.

No argv JSON. With TASK omitted the one declared task runs; with several, the
binding named `default` (`task.default`); with none, the CLI prints the list
and exits 2, never guessing. One owner, `run::address::resolve_task`, so CLI
live, `--dry` rehearsal and the wire arm answer the same. Contract violations
exit 2 with the declared contract shown.

`--dry` on starlark evaluates hermetically and prints the **full** effect set,
applying nothing; on bash it shows the block and its resolved caps and
**refuses to exec**, the caps display byte-identical to the choke-point caps.
It rehearses every pre-apply gate the real run enforces —
address → contract → caps, then choke-point admission over the evaluated md.*
set (`runner::rehearse`) — refusing as the real run would: same words, same
exit leg.

Exit triad: **0** clean · **1** the run plane refused or failed (eval fault,
cap refusal, workspace busy, timeout, bash nonzero) ·
**2** the invocation is wrong (usage, addressing, contract).

**A CHURN refusal carries a recovery line.** It blames nothing the caller
wrote: the ADDRESSED target is a **corpus member that vanished** mid-read
(`corpus_race`). A vanished unrelated record drops from view and fails no other
target; a foreign root advance re-derives and proceeds — the plane pins no
root, so no root-mismatch refusal exists (the no-guard amendment above). The
line is fitted, reason first, then `→ <the move> (recovery: <class>)`, the class
being §8's verbatim `retry`. The wire error frame carries `recovery`
**structurally** and states the reason alone; the line is for the text faces.

## Guarantee classes — labeled per block

| Class | Path | Claim |
|---|---|---|
| `hermetic` | starlark | proof by construction: sealed kernel, zero I/O, metered |
| `detected` | bash | root-snapshot **detection, not prevention** — ungoverned writes are detected and named, not blocked |

The label is per block and the claim ships **scoped, never unqualified**. The
guarantee labeler **refuses to emit `detected` unless the detection path is
landed**. OS-sandbox **prevention** (Landlock / sandbox-exec) is future work;
the shipped plane is detection-only.

## Accepted gaps (S1) — named, deliberate, scoped

Detection-not-prevention for bash and honor-system for out-of-tree writes and
secret reads are intended: ship the scoped claim, never the unqualified one.

| Gap | Class | Disposition |
|---|---|---|
| Bash enforcement is detection, not prevention | intended scope | an OS sandbox, future work |
| Out-of-tree writes / secret reads by bash | **honor-system** (accepted) | outside the hash domain; claim scoped |
| Non-md / `.meridian/` / dot-path writes | **accepted gap, distinct from the honor-system** | outside the snapshot hash domain, silently undetected |
| Symlink laundering (`ln -s secret notes/x.md`) | refused or named | `O_NOFOLLOW`: symlinked path components are refused in walk + snapshot; where refusal is impossible this is a **distinct named gap**. The walk COMPLETES before refusing, and the refusal is a sorted COUNT plus the first offender — `N symlinked paths refused in exec-window snapshot, first: …` — one link keeping the single-path wording. A symlink AT a domain-ignored path (`meridian/domain.md` frontmatter, e.g. `ignore: ["scratch*/", "bin/"]`) is skipped, not refused, reserved paths excepted. The refusal is DELTA-SCOPED: only a link APPEARING inside the window refuses; one pre-dating the bracket is recorded at open, subtracted at close, outside detection. Links never enter the hash domain, so nothing behind one reaches a hash/attest/receipt surface. The bracket's own instruments stay refusable pre-existing or not: a symlinked domain config, and a symlink at a RESERVED path (armed-rules artifact, attested marker). |
| Ungoverned writes are never rolled back | law, not gap | the run exits 1 with the delta named (§7.1) |
| Multi-file crash window (content committed, receipt lost) | accepted | re-derive; lint finds the missing receipt |
| Local run beside a resident daemon (§7.1) | accepted | its writes reach the daemon as external change, like any out-of-band edit |
| The script entry runs in **wire-client mode**, not pure-local | law, not gap | a script must execute AS the caller, and the row above disqualifies pure-local: its writes reach a resident daemon actor-absent. Through the daemon they arrive governed, actor-carrying and Delta-minted like any splice; the in-process lane (wire `script`, § A.7) is the shortest such path — eval inside the daemon, whose commit IS the governed write path. |

## Timing phases

Under `MRD_TIMING` (the switch, sink, line grammar and the two lanes:
`status.md` § The timing mode) a run reports where its wall clock went; the
"inside" column is the live shape.

- **Containment is this table, not the dot.** `dispatch` holds three undotted
  phases, `total` holds everything; the `us` column does not sum. Containment is
  `snapshot.walk`.
- **The floor names partition instead of nesting.** `currency.floor.*` and
  `door.floor.*` split one population by cause and by completion: counts sum, a
  prefix counts every pass beneath it. Partition is `currency.floor.no_feed`.
- **`--dry` has no `dispatch` span at all** ([`rehearse`] composes the chain
  itself): `snapshot` and `eval` sit directly inside `total`; `dispatch`,
  `apply`, `cascade` and `report.render` are absent.

| Phase | Inside | Emitted in | Covers |
|---|---|---|---|
| `total` | — | `mrd::run` | the whole process; every verb has it |
| `workspace.resolve` | `total` | `mrd::run_cmd` | `workspace::resolve` — the discovery ladder |
| `page.load` | `total` | `mrd::run_cmd` | the door's `address::load_page` — parse of the addressed page |
| `conventions.load` | `total` | `mrd::run_cmd` | `caps::load_conventions` — the root's `MERIDIAN.md` |
| `task.gate` | `total` | `mrd::run_cmd` | the door's pre-check: `resolve_task` + `contract_for` + `validate` + `resolve_authority` |
| `pre_eval` | `total` | `run::runner::pre_eval` | the plane's own address → contract → caps chain, repeating the door's early-refusal work as its own gate ([`pre_eval`], ONE owner for both tenses); measured on the chain, so `--dry` reports it too |
| `dispatch` | `total` | `run::runner` | starlark leg only: `eval` + `snapshot` + `apply` whole, in that ORDER — the fold FOLLOWS the eval that decides if it is needed (§ The run plane). **Bash is not that shape**: no `eval` span, no `snapshot*` line — it observes via phase-free `fs::domain_leaves_memoized` (the phases live in `fs::domain_snapshot_with_leaves` and its fold-only twin `fs::domain_fold`) first, under the flock, before the block. The lazy rule is the starlark leg's |
| `snapshot` | `dispatch` | `fs::domain_fold` on the run plane; `fs::domain_snapshot_with_leaves` for callers that want the bytes | the three below, whole; absent when this tense's lazy gate did not fire (§ The run plane). Both emit the same four names |
| `snapshot.walk` | `snapshot` | same | `Domain::load` + `hash_domain` — the hash-domain walk |
| `snapshot.read` | `snapshot` | same | `read_and_digest_members`, or `digest_members` on the fold-only path — read + blake3 of every member; the fold-only sweep releases each member's bytes with its digest |
| `snapshot.fold` | `snapshot` | same | leaf assembly + `served_root` |
| `eval` | `dispatch` | `run::dispatch_starlark` | hermetic evaluation of the block |
| `apply` | `dispatch` | `run::dispatch_starlark` | the executor's one md.\* batch (absent when the block emitted none) |
| `cascade` | `total` | `run::runner` | the cascade loop; vacuous under the empty `S1_RULES` ruleset, so near-zero `us` is expected |
| `report.render` | `total` | `mrd::run_cmd` | the report render |
| `currency.vouched` | — (`cmd=daemon`) | `registry::Registry::currency_refresh` | a READ-plane §6.7 currency pass on the O(1) cookie fast path: no walk, no stat, no byte read |
| `currency.floor.<cause>` | — (`cmd=daemon`) | same | a read-plane pass that MISSED the vouch and fell to the §6.2 extent-refresh floor — the full stat sweep. `<cause>` names the term that missed. **A bare `currency.floor` is never emitted**; it is the family prefix |
| `currency.floor.<cause>.refused` | — (`cmd=daemon`) | same | the same pass, ENTERED then refused by an I/O failure in the sweep |
| `door.vouched` | — (`cmd=daemon`) | `registry::Registry::door_observation` | a WRITE-door entry observation on that fast path, inside the write flock |
| `door.floor.<cause>` · `door.floor.<cause>.refused` | — (`cmd=daemon`) | same | the write plane's floor: same two shapes, same `<cause>` set |
| `door.refused.<cause>` | — (`cmd=daemon`) | same | a write-door observation refused BEFORE either arm was chosen. One cause today, `lock_contended`: the memo lock stayed held for its whole budget. **The read plane has no counterpart and that is a fact, not a gap** — the same `fs::lock_within` with a budget of `None` returns `Ok` before the wait loop (`Registry::patched_cache` asserts it) |

**The door's three families partition its population**: `door.vouched` (fast
path), `door.floor` (floor), `door.refused` (no arm) — every `door.*` name under
exactly one, counted once.

**`<cause>` is one of six** (`registry::FloorTrigger`):

- `no_feed` — no live feed: cold start, vouching impossible.
- `cookie_unproven` — the §6.4 barrier timed out or hit I/O; the only
  time-bounded cause, the only one rising with load.
- `cookie_refused` — the cookie would enter the hash domain.
- `collapse` — the applied set collapsed doubt.
- `untrusted` — the §6.2 close does not hold the memo trusted.
- `no_overlay` — the resident overlay folded no root.

**A phase reports only where it COMPLETED.** One that never ran emits no line:
nothing from `snapshot` down under `--list`, no `snapshot` for `--dry` on bash.
A FAILED phase is abandoned on the error path: `mrd run missing.md` prints
`workspace.resolve`, the refusal and `total`, no `page.load`. `total` still
reports on a refusal. Two exceptions:

- `daemon.dial` `stop()`s on the degrade path (`mrd::engine::answer_links`): the
  dial completed with a verdict (no usable daemon answer) — decision completed,
  not a daemon answer (`status.md` `daemon.dial` Covers).
- **The daemon's refusals are counted and named as refusals**:
  `currency.floor.<cause>.refused` and `door.floor.<cause>.refused` (floor pass
  ENTERED, then an error), `door.refused.<cause>` (no arm reached).

**And a phase whose GATE did not fire never ran.** Only `snapshot` is gated: the
lazy fold's trigger differs by tense (§ The run plane) — any effect under
`--dry`, an md.\* effect live. Effect-free and live `notice`-only runs reach no
`snapshot`; a `snapshot` line there is the lazy gate broken.

#### The currency pair answers "how often does the floor run"

`currency.*` and `door.*` are **pairs, and the pair is the measurement.** The
§6.2 extent-refresh floor is a second execution path behind the §6.7 vouched
one: the honest question is a RATIO, not a count.

```bash
MRD_TIMING=/var/log/mrd-timing.log   # on the daemon's environment
L=/var/log/mrd-timing.log
grep -c 'phase=currency.floor'          "$L"   # the fallback — every pass that ENTERED it
grep -c 'phase=currency.vouched'        "$L"   # the designed path
grep -cE 'phase=currency[.]floor[.][a-z_]+[.]refused( |$)' "$L"   # of those, the ones that did not finish — the `.refused` OUTCOME must be anchored as the final component, because a CAUSE name can itself end in `refused` (`cookie_refused`) and an unanchored `.*refused` counts its ENTRY line too
grep -o 'phase=currency\.floor\.[a-z_]*' "$L" | sort | uniq -c   # by cause
```

- Read `cmd=` first: only the engine emits these lines, always `cmd=daemon`.
- **Do not add the two pairs together**: one currency pass per read-plane warm
  versus one entry observation per write; unrelated denominators.
- **The floor prefix counts ENTRIES.** The error path
  returns under a `.refused` name instead of abandoning its span: prefix counts
  entries, suffix the entries that did not finish, and
  `grep -c 'phase=currency.floor'` answers how often a pass fell to the floor.
  The fallback test reads that count — zero in steady state means delete the
  path, above zero means it is not a fallback.
- **A `.refused` line still means the span reached its stop.** A pass that
  crashes or is killed reports neither name; `door.refused.lock_contended`
  covers the write plane's one pre-arm refusal (memo lock budget), so a
  contended observation stays in the denominator.

#### The `snapshot` set can repeat, and which lane you are on decides

`snapshot.*` comes from `fs`, not this plane: **every** caller of
`domain_snapshot*` lights it up, so **a `phase=snapshot` line does not imply a
run.** Folders reporting it under their own `cmd=`: `mrd sql`, `mrd check`,
`mrd walk`, `mrd repair`, `mrd retire`, `mrd links`, the daemon's resident
rebuild, its watch loop. It also repeats per mount corpus (`load_mounts_for` →
`build_docs_at`, calling `fs::domain_snapshot`): once for the workspace corpus,
once per mounted root addressed — lock-addressed on
`walk`/`check`/`status`/`walk_op`, link-addressed on `links`/`sql`/`sql_op`.

**But do not price a run by that count — some full-corpus folds emit no phase at
all.** The write doors observe through `fs::DomainCache`
(`wire_serve::write::observed_root` → `DomainCache::root`): it walks the domain,
`stat`s every member and reads every mover WITHOUT a `snapshot` span. That cache
is the process-global `wire_serve::write::WRITE_CACHES`, keyed by canonicalised
root — FIRST door call per process cold, later ones in the SAME process warm.
The CLI does one birth per process, so always cold there: the process model, not
`md.create`; batching births into one process pays it once.

An `md.create` therefore pays a second full-corpus observation on top of the
plane's fold, under one `phase=snapshot` line, so `grep -c phase=snapshot`
under-reports corpus work by more than half (37 800-member root: run-plane
`snapshot` 446 ms, door 620 ms inside `apply`; 8 002 members: 27 ms + 58 ms).

**Subtraction will not show it**: `apply` has NO nested phases — nothing under
`crates/run/src/executor.rs` opens a span — so a large `apply` beside a
`snapshot` of the same order is the door's own observation; only an instrumented
build splits it.

`corpus.build` is the same class, from `fs::build_corpus`: the same callers plus
the write-door referrer scan (`wire_serve::write::inbound_referrers`), so **a
`phase=corpus.build` line does not imply a links call** — read `cmd=` first. It
repeats per mount corpus as `snapshot` does; neither name distinguishes
workspace from mount, so count the lines.

Four fold sites, not all firing on one lane:

| Fold | Fires when |
|---|---|
| `dispatch_starlark.rs` `observe_if_emitted`, live tense (reached from `runner.rs` `dispatch`) | a live run whose block emitted an **md.\*** effect. A live `notice`-only or effect-free run folds NOTHING (§ The run plane) |
| `dispatch_starlark.rs` `observe_if_emitted`, rehearsal tense (reached from `runner.rs` `rehearse`) | `--dry` instead of the above, not as well, and only when the block emitted SOMETHING — a wider gate than live, for the dry report's provenance |
| `runner.rs` `cascade` | a generation that applies md.\* — needs a NON-EMPTY ruleset, and both doors hand `S1_RULES` (empty), so today: never |
| `executor.rs` pre-commit | only with a `DeltaSink` in reach, i.e. the WIRE arm. The CLI passes `delta: None` and returns before the fold |

On the **CLI** an md.\*-committing `mrd run` emits exactly ONE `snapshot` set
(`grep -c 'phase=snapshot '` = 1); one committing nothing emits **zero** — the
lazy gate (37 800-member root: 0/20 folded; eager 20/20). On the
**wire/daemon** arm a committing run folds again inside the executor: identical
names, no discriminator, so count them.

## Seam map (for reviewers)

| Seam | Owner |
|---|---|
| addressing / fence / contracts / caps | `crates/run` (`address`, `fence`, `contracts`, `caps`) |
| hermetic eval | `effects::eval_run` via `crates/run::dispatch_starlark` |
| bash exec + two-phase receipts | `crates/run` (`exec`, `dispatch_bash`) |
| detection bracket | `fs::guard` (+ `crates/run` snapshot integration) |
| the one write path | `crates/run::executor` → `model::validate_batch` → `fs::apply_batch` |
| stdout record | `crates/run::record` |
| CLI mount | `crates/mrd::run_cmd` — a client; charter edge `laws.md` §crates (`mrd`) |
| CLI mount — script entry | `crates/mrd::script::cmd` — same client edge; its human-mode face is non-normative |
| in-process script serve (§ A.7) | `crates/registry` (op arm: entry world, host, threading, commit) over `crates/effects` (kernel, trace, digest) |
| wire run serve (§ A.8) + script effects mode | `crates/registry` (`run_op`: per-target loop, §9 threading; `script_op`: the live host) over `crates/run` (the plane, unchanged) |
| per-phase timing (`MRD_TIMING`) | `crates/timing` (switch, sink, span); call sites `mrd::run_cmd`, `run::runner`, `run::dispatch_starlark`, `fs::domain_snapshot_with_leaves`, `fs::domain_fold`, `registry::Registry::currency_refresh`, `registry::Registry::door_observation`; § Timing phases |
| root-at-eval observation (the lazy fold) | `crates/run::dispatch_starlark::observe_if_emitted` — ONE owner of the per-tense gate; `runner::rehearse` calls it directly, the live leg inside `dispatch_starlark::dispatch`. `evaluate` returns `Unobserved`, so neither skips it; § The run plane (`RunCtx`) |

---

# Presets and session birth — the design element

> Folded into `run-plane.md` for navigation. Standing corrections: `README.md` / `wire-contract.md`.


A **preset** is a def page that declares a shape; **session birth** turns that
shape into files. The `preset` crate and the `new` / `unfold` / `reconcile`
verbs are audited against this element; where code and element disagree, the
element wins.

## 1. The premise — a shape is declared once, in a page

**The shape lives in a page**, in the markdown the engine already governs, and a
born tree **pins the def it came from at the def's rev** — the shape behind any
file is recoverable from the file forever, without the tool that wrote it.

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

- `# Properties` (`^properties`) — the rules a born record must satisfy: one
  `- key` or `- key = value` item each.
- `# Template` (`^template`) — the fenced body one record is born from.
- `# Unfold` — the declared scaffold: the paths a whole birth materializes, in
  declared order.
- `# Ephemeral` — the **allowlist** of declared-disposable paths, empty by
  construction: a def declaring nothing disposable prunes nothing.

**The parenthetical is a REQUIRED byte of the heading line.** `(^properties)`
and `(^template)` are block ids standing ON the heading line —
`# Properties ^properties`. The loader finds them by ANCHOR, never by heading
text: a `# Properties` section with no anchor id declares no `^properties`
block.

**Law 2.1 — `inputs` is read and written whole.** A multi-line block sequence:
read through the whole-value frontmatter grain, written as whole birth bytes. A
line-oriented scan stopping at the key line, or a single-line properties upsert,
corrupts it. Read and render halves are one round trip, audited as a pair.

## 3. The birth law — one door

**Law 3.1 — every byte a preset lands rides the guarded create.** No
`fs::write`, no second write path, no exception for a stub, a dry run or a
scaffold file: it carries the `if_absent` CAS, the journaled birth receipt and
the gate seam.

**Law 3.2 — a birth never clobbers.** An occupied target is the CAS's answer: a
`cas_mismatch` finding, the file left byte-untouched; a rehearsal that would have
clobbered refuses too.

**Law 3.3 — every removal rides the guarded remove**, read-then-delete under
the live rev. One exception, never widened: an empty directory has no governed
rev and no bytes, so raw `rmdir` removes it.

**Law 3.4 — the plane mints no identity and no clock.** `actor` and `now` are
caller-supplied, stamped exactly as given; absent stays absent.

**Law 3.5 — a born record names the def it was born from.** Root record and
scaffold stub alike carry `preset:` holding the DEF's page path, never their
own.

**Law 3.6 — a `^template` placeholder in a frontmatter value position is a
VALUE-PLANE WRITE.** `{{id}}`, `{{kind}}`, `{{actor}}` and `{{now}}` fill the
template BODY verbatim; in its frontmatter block the substitution goes through
the one encoder wire-contract § A.6.3a names, which `set_property` and
`put{at:"upsert"}` already speak. The value is the plain form when that form
decodes back to exactly the caller's string, the canonical double-quoted scalar
otherwise.

A value that cannot be ONE frontmatter line — one carrying `\n` or `\r` —
**refuses the birth** (`bad_request` / `fix`), naming the key, the v1
single-line rule, the body-section escape and the placeholder that carried it.
Nothing is written (Law 3.4: sanitizing the actor would falsify recorded
provenance).

Measured: `--actor $'bob\nstatus: closed'` against `owner: {{actor}}`, through a
door interpolating source bytes, mints `status:` twice — disk `closed`, every
read door `open`, no governed edit reaching it.

## 4. The three verbs

The plane offers exactly three births, differing only in **what set of paths
they act on**; they share one def loader, one renderer pair and one guarded
door, and a verb growing its own copy is wrong-design.

| Verb | Acts on | Refuses when |
|---|---|---|
| `new <kind> <id>` | ONE record, from `^template` | the def is invalid, or the target exists |
| `unfold <preset>` | EVERY declared scaffold path | any declared path already exists |
| `reconcile <preset>` | the MISSING declared paths only | — (an occupancy is not a failure here) |

**`new` validates before it writes.** The filled template is parsed and checked
against every `^properties` rule; the FIRST violation refuses `def_invalid`,
naming the rule verbatim. A def with no `^properties` block, no `^template`, or
an unparsed rule is itself invalid — the same refusal, stating the anchor rule so
the author learns which byte is absent.

**`unfold` is the first birth; `reconcile` is every birth after it.** Unfold
treats an occupied path as a finding; reconcile treats it as convergence.

## 5. The reconcile asymmetry

**Law 5.1 — reconcile is additive by set-difference, subtractive by
allowlist.** These are two operations, never to be refactored into one:

- **Materialize** every declared path missing from the tree. Set-difference.
- **Prune** only paths matching the `# Ephemeral` allowlist, plus empty
  undeclared directories. Allowlist.
- **Everything else** — undeclared content — renders as a **finding**. Never a
  prune action, never under any flag.

**Law 5.2 — "undeclared" is not "unwanted".** A user's file the def has never
heard of is a report, not garbage. The asymmetry is the safety property; making
the halves symmetric deletes the design.

**Law 5.3 — reconcile stays inside the shape's territory.** The scan scope is
the directories the declared scaffold occupies; reconcile never reads, reports
on, or prunes a path outside it. Engine and system files (dotfiles, the reserved
journal) are never "undeclared content".

A prunable **directory** lives strictly beneath a directory the scaffold itself
creates, is not an ancestor of a declared path, holds no finding, and is empty.
A scaffold of only top-level files creates none, and the workspace root is never
walked for candidates.

**Teaching row — Law 5.3 OUTRANKS Law 5.1, and the losing entry dies
silently.** An `# Ephemeral` path outside the scaffold's territory is INERT:
`--prune` walks past it with no prune row, no finding row, no refusal —
territory decides scan scope before the allowlist. Measured:
`sessions/tmp-cache.md` (allowlisted, in territory) pruned; `scratch/tmp.md`
(allowlisted, `scratch/` holding no `# Unfold` path) survived untouched and
unreported. **What the face does not do is say so.** A dead allowlist entry gets
zero disclosure at declare time (`mrd new` accepts the def) and zero at prune
time, and an ephemeral-declared file that IS present renders no row under a
no-prune reconcile — neither finding nor ephemeral.

⚠️ Whether the plane owes a declare-time or prune-time disclosure on a
territory-shadowed allowlist entry is NOT settled by this page.

**Law 5.4 — pruning is opt-in.** Without `--prune`, reconcile materializes and
reports and removes nothing.

## 6. The convention floor

A session preset's `inputs` pin the **convention floor** — the rule pages the
born session lives under — at a path and a rev.

**Law 6.1 — a floor pin is a pin, not a copy.** It records `path@rev` and never
inlines floor content into the born tree.

**Law 6.2 — the born root record carries the floor pins itself.** Its `inputs`
is one block sequence: the def pin first, then every declared floor pin, in
declared order. A def pin alone leaves the floor readable only through the def
blob.

**Law 6.3 — the floor prefix is a default the def overrides, never a validity
predicate the engine owns** (`docs/laws.md`: no hard-coded flow). `conventions/`
is the fallback, by convention; `floor: standards/` in a def is exactly as
valid. The engine reads the def's own key and only falls back to the constant —
the shape `root` / `DEFAULT_ROOT_RECORD` already had.

## 7. Refusals and exit codes

Two failure kinds, never conflated:

| Kind | Exit | Examples |
|---|---|---|
| **Finding** — the plane ran and reported | 1 | `def_invalid{rule}`, `cas_mismatch`, an undeclared-content finding |
| **Tool failure** — the plane could not run | 2 | def unreadable, page is not a def, a write faulted for a non-CAS reason |

**Law 7.1 — a refusal names the rule it enforced.** `def_invalid` carries the
source text of the violated `^properties` rule.

## 8. Boundaries — what this plane never does

- **No session policy, no liveness.** Whether a session is active, expired or
  archived belongs to the customer that dials this plane.
- **No CLI.** `mrd new` / `unfold` / `reconcile` are thin clients: argument
  parsing, workspace resolution, output shape. Every decision here lives in the
  crate.
- **No write path, no hash law, no rev noun.** It composes the shipped ones.

## 9. The user-facing surface carries no internal tags

**Law 9.1 — a verb's help text is written for the person typing the verb.**
Internal planning tags — unit and block numbers, plan-section references,
docket tags — belong in source comments, crate metadata and test names, never
in `mrd help` output. A test over real help output gates it.

---

## Appendix — the conformance audit

Every law audited against the `preset` crate and the `new` / `unfold` /
`reconcile` verbs, including trivially satisfied ones.

| Law | Verdict | Evidence |
|---|---|---|
| 2.1 whole-value `inputs` | **conformant** | `read_inputs_grain` spans the whole `FmKey("inputs")` block; `render_block_sequence` writes it back; round trip tested. |
| 3.1 one write door | **conformant** | All landing bytes: `birth` → `wire_serve::write::create`; no `fs::write` in the crate. |
| 3.2 never clobber | **conformant** | `if_absent` CAS; `BirthResult::Occupied` is a finding, not a fallback write; `opts.dry` refuses at the door. |
| 3.3 guarded remove | **conformant** | `prune_file` removes under the live rev; `rmdir` only for empty directories. |
| 3.4 no minted identity/clock | **conformant** | `actor` / `now`: `Option<String>` on `BirthOptions`, never clock-defaulted; `fill_vars` renders absent as empty. |
| 3.6 template fill is value-plane | **conformant** | `fill_template` encodes frontmatter values with `policy::defs::yaml_safe_value`, the other two § A.6.3a doors' encoder, refusing multi-line values in their uniform words: `\n` or `: ` cannot mint a second key line. Six tests, `crates/preset/tests/birth_value_plane.rs`, incl. plain-value and body-verbatim controls. |
| 4 three verbs, one door | **conformant** | All three call `load_def` and `birth`; no private write path, no second renderer. |
| 4 `new` validates first | **conformant** | Structural checks, `first_violated_rule`, birth; a def failing its `^properties` refuses before any byte moves. |
| 5.1 additive diff, subtractive allowlist | **conformant** | `reconcile_plan` is a pure fold; halves stay separate fields. |
| 5.2 undeclared is not unwanted | **conformant** | The prune path never reads `findings`. |
| 5.3 territory, files | **conformant** | `scan_scope` walks only directories holding a declared path; dotfiles and the reserved journal skipped. |
| 5.3 territory, directories | **conformant** | `prune_empty_dirs` walks beneath the scaffold's own directories, bounded so a top-level-only scaffold never reaches the workspace root; candidates and skip set from different expressions, else `pruned_dirs` dies. Three tests, one the bound. |
| 5.4 prune is opt-in | **conformant** | Gated by an existing test. |
| 6 floor pin | **conformant** | `render_root_record` writes `path@rev`; content never inlined. |
| 7 finding vs failure | **conformant** | `RefusalReason` (exit 1) and `PresetError` (exit 2) are separate types; only a non-CAS write fault crosses over. |
| 7.1 refusal names its rule | **conformant** | `def_invalid` carries `PropRule::raw` verbatim. |
| 8 no session policy, no CLI | **conformant** | No liveness state; the three `mrd` modules parse arguments and shape output only. |
| 9.1 no tags in help | **conformant** | No help description carries a planning tag; a derived test scans every printable page; comments and crate metadata keep theirs, which §9.1 permits. |

**No part of this plane warrants removal.**
