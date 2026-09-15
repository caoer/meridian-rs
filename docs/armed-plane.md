---
type: spec
id: armed
status: standing
description: Normative spec for the floor-convention arming ladder and the `gate()` seam.
owns: [the arming ladder, the gate() seam]
---

# Armed plane — bootstrap ladder + gate seam

> Standing law: `README.md` (process and standing corrections) and `wire-contract.md` (the wire contract).

Law: `laws.md` § policy gate; `wire-contract.md` § A.2 (block-is-a-feature,
genesis-epoch grey) and its §8 (refusal taxonomy).

This page rules two things: the ladder a maintainer climbs to take a workspace
from **never-armed** to **steady state**, and the `gate()` seam that does the
enforcing. Never-armed means `gate()` is a no-op and writes land bit-for-bit.
Steady state means the write door — the seam where a write is checked before
it lands — refuses a write that breaks an armed rule. The wire shape of what
the plane emits lives in `wire-contract.md`; whether to arm a production
workspace at all is a deployment decision, not this plane's.

---

# Part A — Arming from zero

Arming is a reviewer act: `mrd arm <ID> --mode M --rev R [--at DIR]` attests
the resolved page at the rev the reviewer read. It is the attest path the
binding law's refusals name as the legal road. `--rev` is required, with no
live-rev default.

The floor suite this ladder arms: `reviewer-not-owner`, `claim-cas`,
`close-verdict`, `decoy-close`, `verdict-reviewer-bind`, `meta-convention`
(guards arming itself); each is a `rules/check` page, authored as rung 1 below
describes.

## The two states the marker separates

The once-armed marker `meridian/attested` (`crates/fs/src/domain.rs`,
`ATTESTED_MARKER_PATH`) records, by presence not bytes, that the workspace was
ever armed. The first attested arm creates it; nothing removes it. The gate
reads it to tell two states apart:

- **never-armed** (no marker): `policy::resolve_armed_law` answers
  `never_armed()`; `gate()` is a no-op; writes land bit-for-bit. The artifact
  that holds the attested rows (`meridian/armed-rules.md`, `ARMED_RULES_PATH`)
  is not read at all. A stray artifact cannot arm a never-armed workspace,
  because only an attested arm sets the marker.
- **once-armed**: the artifact MUST be present, parseable, and attest at least
  one row, or the gate fails closed — it refuses the write instead of letting
  it land (rung 5 below). Zero rows is absence of attestation; a rule
  deliberately not enforced is a row spelled `off`.

## The ladder (five rungs)

Rungs 1–3: ungated authoring. Rung 4: genesis transition. Rung 5: steady
state.

### 1. Fill the slot

Author the rule page.

- Registration: `rules/check` (a law) and/or `rules/hook` (a reaction) in
  `tags:`, plus an `id:` in the rule-id grammar
  (`crates/policy/src/registration.rs`, `RuleId`). One fenced block may carry
  both legs; entry points `check_change` / `on_change`. No folder or filename
  is load-bearing.
- `kind:` may restate the tag or be absent (derived); it never contradicts it.
- The check leg keeps its fixed refusal ceiling; the hook leg emits only its
  declared caps, pinned to `proto.send`. FIX and VIEW remain named deferrals.
- **The page must sit inside the workspace hash domain** — the set of files
  the workspace fingerprint covers (`wire-contract.md` §12.1). A rules-tagged
  page registers as nothing if it sits on a dot-segment path
  (`.hidden/rules/x.md`, anything under a dot directory), or if a
  `meridian/domain.md` ignore rule excludes it.
  The exclusion is never silent (`wire-contract.md` §12.1 enumerator clause).
  `mrd rules` lists such candidates in one bounded line; the full list is
  `not_offered.workspace_dot` in `--json`, exit-neutral. `mrd arm <ID>` on an
  id whose only carrier is domain-excluded refuses, naming the file and the
  exclusion reason. A MERIDIAN.md on a dot path resolves to the enclosing
  root; every page under it is outside that root's domain.

### 2. Author the floor

Write the predicate so its refusal always cites the passing scenario (the
legal path). A floor convention's refusal names its taxonomy rule
(`reviewer_owner`, `claim_cas`, `close_verdict`, `decoy_close`,
`reviewer_bind`, `arming_precondition`) and, where it teaches a winner or
bound reviewer, names them.

### 3. Test the tiers

Before arming, pass both `mrd test` tiers:

- **`--corpus`** (`mrd test --corpus <spec>`): fire-where-expected over a
  governed tree, zero dead rules, fuel/heap p50/p99, FIX/HOOK quiescence
  (reachable trigger graph plus bounded counterfactual chaining). Only this
  tier admits `md.*` counterfactuals; it does not widen the armed caps. A
  counterfactual descriptor passes the same canonical intent validation as an
  armed hook's. It runs through the production intent→executor adapter and
  atomic batch executor, in a throwaway proof workspace. The governed tree
  stays read-only, and the triggering write is untouched.
- **`--history`** (`mrd test --history <ws> --rule <page> [--spec <page>]`):
  reconstructs the workspace's past, reports the exact journal span examined,
  and requires zero undeclared refusals against the `golden` fence of the
  `--spec` page.

Passing both is **pre-arm qualification** (required for arming review), not
armability.

### 4. First arming write — ungated-but-journaled, permanent, genesis-grey

This rung is the act itself. A reviewer approves the page at the rev they
read (the `armed-rev`), the act writes the attested row, and the first arm
creates the marker.

Arming writes the row only while the live rev (`report-rev`) still equals the
approved `armed-rev`. Drifted law is refused, never silently armed.

**The attested rev is the page rev, uniformly:** `armed-rev = page_rev(page
bytes) = blake3(bytes)[:16]` (`crates/policy/src/registration.rs`,
`node-rev-merkle-spec.md`); hook and check pages attest on the same terms.
The `pin` axis of `mrd status` rolls up this attested page-rev drift
(`status.md` § The composed status line).

#### What the ARM act attests

One tag-indexed artifact per workspace (`meridian/armed-rules.md`,
`crates/policy/src/armed.rs`), one row per **(id, arm root)**:

| column | content |
|---|---|
| `id` | the page's frontmatter `id:` |
| `page` | workspace path of the resolved page (override winner) |
| `rev` | the page rev the row is attested at |
| `scope` | the arm root: the workspace-relative directory that resolution was narrowed to (`.` = workspace root). `layer:depth` (`workspace:0`) is refused at parse: a head segment with `:` is the address grammar's `root:` qualifier (`address-grammar.md` § 4.1 colon law), never a workspace path. A directory scope is not a page reference; it sits outside the rooted-lane door family (`address-grammar.md` § 4.6), so this refusal stands. |
| `mode` | checks `off\|warn\|block`; hooks `off\|armed` |

The act is indivisible and all-or-nothing. It narrows to the arm root's
chain, resolves through the one resolver, and pins the winner's page and rev,
so `scope` cannot drift from its resolution. Every fault is reported at once.

- **Arming freezes resolution.** A page appearing later, even a deeper
  override candidate, governs nothing until re-arm. The tag registers, only
  arming activates, so no writer can take over an armed id by dropping a file.
- **An edited pinned page reddens:** its row does not fire on the new bytes.
  A red check row refuses the write; a red hook row falls silent (a hook never
  vetoes).
- **Mode vocabulary splits by kind** (the `mode` column above): a hook row with
  `warn`/`block`, or a check row with `armed`, is refused at the act.
- **A row that will FIRE must LOAD.** Arming attests two layers:
  registration (tag + `id:`) and declaration (`severity:`, `caps:`,
  `budget:`, entry point). Each non-`off` winner is loaded through
  `policy::rule::load_rule`, the same loader the fire path runs in
  `armed_law::resolve_armed_law`. If it will not load, `ArmFault::Unloadable`
  names the loader's fault and refuses **before anything is written to
  `meridian/armed-rules.md`**. `policy` does no I/O: bytes arrive through the
  injected `PageSource` under the caller's `CheckLimits`. `off` rows are not
  loaded, so an `off` row may attest a page too broken to load. The loaded set
  is a superset of the fire path's: `resolve_armed_law` loads
  `verdict.firing()`, which is further narrowed to the write's own path and
  excludes reddened rows.
- **Drift, not a broken declaration:** `mrd arm` indexes before it takes the
  write flock (the workspace write lock), so the drift gate may see a stale
  rev. The loader's `RuleLoadError::RevMismatch` catches the race. The act
  re-labels it `ArmFault::Drift`, at the rev the loader read.

#### The disk edge (wired), and what is still deferred

The write door (check rows) and the reaction feeder (hook rows) read artifact
and marker through one shared reader, so the workspace cannot disagree with
itself about whether it is armed.

`wire_serve::armed_disk::ArmSession` is the disk edge, driven by `mrd arm`:

- Write flock held from artifact read to commit; rename-atomic landing.
- Crash order **artifact first, marker second**; the marker landing is the
  commit point. Artifact-without-marker reads as never-armed; the identical
  re-arm is a no-op.
- The edge does not ride the caller door: a direct door write to the artifact
  is `binding_break` (taxonomy row 9). On a direct write that refusal fires
  before any rule evaluation, so row 8 cannot fire there. The act's own law
  runs before the session opens: `policy::armed::arm`'s faults, the drift
  check, and a strict parse of the standing artifact. To every other process
  the landing is an external write.

Deferred: **`mrd realise --truth` convergence** over the artifact+marker pair
is a separate design.

The **first** arming write is special, permanently. It is **ungated**: no
marker exists yet, so `gate()` is a no-op. It is **permanent by the pair**:
the row pins page and rev, the marker pins the epoch. It is **grey, never
green**: a never-armed write carries no enforcement verdict, and
`t.result.verdicts` is empty. Grey is the absence of a green verdict, not a
token (§ What arming does NOT claim).

Once armed, the `meta-convention` guards this rung. An arming proposal must
pin attested evidence (P@R: a page at the rev it was attested at), declare a
structural `cites:` join, and be armed by a reviewer distinct from the
convention's `author`. Otherwise `arming_precondition` (taxonomy row 8)
refuses. The convention cannot gate its own first arming, which is grey.
**Row 8 is not yet evaluated on the attest path.** A follow-up rung, not yet
built, will wire the armed `meta-convention` into `mrd arm`'s re-arm leg.

### 5. Steady state

With the marker present and the artifact carrying `[x]` rows, the door
enforces:

- `block` rows refuse a violating write (bytes never land) with a `{code,
  recovery}` pair from the closed taxonomy of `wire-contract.md` §8; `warn`
  rows render an advisory finding and land; `off` rows are ignored.
- Missing or corrupt artifact: fails closed (`convention_fault`).
- Page drifted off its pinned `armed-rev`: fails closed (`armed_drift`);
  re-arm at the live rev, or revert the law.
- `--force` is the only escape: journaled and rendered.

## What arming does NOT claim

Refusal makes violations "unrepresentable through an armed change plane",
nothing stronger. The genesis epoch renders grey, never green. Out-of-band
mutation — an offline pre-push git rewrite, a root-preserving forged journal
row — is caught by the git witness plus the receipt-engine-only write
restriction, or it is a named residual. Refusal never renders it green.

---

# Part A2 — Middleware on the write door

Normative for the `rules/middleware` plane; wire shape `wire-contract.md`
§ A.2.1.

Beside check pages (yes/no before the door) and hook pages (after commit,
`proto.send` only), **middleware is the third kind: check plus transform on
the door itself.** One Starlark eval runs per armed in-scope middleware page,
on the caller's put (the incoming write), after CAS and batch validation,
before bytes land. Outputs:

| Output | Lands | Who applies |
|---|---|---|
| `refuse(message=, passing=)` | nothing committed | engine |
| `set_field(path=, key=, value=)` on this file | this put's own batch | engine |
| `set_field` on other files | **same sealed set** as this put | engine |
| `create(path=, body=)` | birth in the same sealed set | engine |
| `send(to=, body=)` | never disk; an **intent** on the response | **host realizes** |

- One caller put may become many disk edits. That **sealed set** is
  validate-all-then-apply: caller write, middleware edits, and births land
  together or not at all.
- Middleware `create` is **its own constructor** and takes no `props=`.
  Births ride this put's sealed set, not the create door, so the door-side
  frontmatter serializer (`run-plane.md` § the machinery floor) is
  unreachable. Middleware frontmatter is `set_field` on the born path, or
  body bytes.
- Send cannot ride `write.lock`, so it stays an intent; the engine never marks
  it delivered (`armed.intents[]`, `wire-contract.md` § A.2.1).

## Registration and arming

- Registration: `rules/middleware` in `tags:` plus an `id:` (rule-id grammar),
  as in rung 1. Required frontmatter: `paths:` (scope globs). Entry point:
  `def middleware(ctx)` in the fenced ```starlark block.
- **Mode vocabulary: `off | block`** (no `warn` tier; `armed` stays hook
  vocabulary). A red, unloadable, or unevaluable row refuses the write
  (`Mode::Block` enforces), as a check row does.
- Arming is the same act (`mrd arm <ID> --mode block --rev R`); artifact row,
  binding law over armed pages, and `armed_drift` apply unchanged.
- Eval order within one write: **`id` ascending (lexicographic)**. No
  `priority:` field; pad ids (`000-…`).

## The ctx surface

Middleware runs under the check evaluator's limits (`CheckLimits`: fuel, heap,
call-depth, source-size, nesting; no per-page `budget:` in V1) over one
injected `ctx`:

| Member | Carries |
|---|---|
| `ctx.op` | `"splice"` \| `"create"`, the caller's op |
| `ctx.before` | this file before the put: `{path, nodes, frontmatter, edges}` (the `@2` doc facts) |
| `ctx.after` | this file after the pending set so far (caller put + earlier middleware) |
| `ctx.put` | the caller's edit set `{op, actor, force, edits, fields_changed, sections_changed, targets}`; never rewritten |
| `ctx.fields` | **opaque passthrough** dict from the put frame's `fields` (`wire-contract.md` § A.2.1); keys uninterpreted; `actor`/`now` stay `wire-contract.md` §9 wire inputs |
| `ctx.sql(query)` | one read-only SELECT against the overlay world; returns rows (list of dicts); not DuckDB DML |
| `ctx.read(path)` | that path's bytes in the overlay world, or `None` |

**The world** both accessors read is the snapshot at flock time, plus this
file's pending after-state, plus every edit or birth earlier middleware
emitted (id order). A later writer is invisible. `ctx.sql` uses the
host-installed SQL backend (`wire_serve::middleware::install_sql_backend`);
`mrd` and the daemon install a `view::build_memory`-backed one. A middleware
that calls `ctx.sql` on a door with no installed backend refuses the write,
naming the gap.

## Emits, compiled

What the engine does with each output above:

- This-file `set_field` joins the caller's batch as a native frontmatter
  upsert (`SecRef::FmKey`). The augmented batch then re-runs the door pipeline
  on the final state: `@fp` strip, stored-form translation, lock-artifact
  guard, I4 conformance, check gate.
- Other-file `set_field` compiles to that file's member batch: same
  validation, same check gate at its own path.
- `create` births in the same set; an occupied path refuses the whole set
  (`cas_mismatch`, expected absent). Birth bodies get the same document-grain
  strip and guards as a `create` op's body.
- V1: the **create door** admits `refuse`, this-file `set_field`, and `send`;
  cross-file edits or births from a birth refuse as unsupported. In V1 the
  **set door** (`splice.set`) and **remove door** evaluate no middleware.
- **Delete: not built.** No `remove` emit.

## Hook scope beside the middleware door

Each rule kind fires on its own surface.

`splice`/`splice.set`/`create` evaluate no `rules/hook` pages; their responses
carry no reaction envelopes (`armed.effects` stays in the shape, empty). Send
is not an engine rule; middleware intents are the one send lane on the put
path. `rules/hook` fires on the external-change detector (`watch`).
`rules/check` is refuse-only: same vocabulary, same tests.

First-arm `meridian/attested` is a plane-wide permanent flip; arming a
production workspace is a deployment decision, not this plane's.

---

# Part B — Gate byte landing

**Measured at `7a22e00a`.** This part states a law and describes an
instrument: the one test that reads the source tree. It takes no census — no
list of every site that *lands bytes, gated or exempt* — because that is not
what the instrument derives.

## The law

`gate()` refuses an armed change **after CAS, before bytes land**
(`wire-contract.md` § A.2). Every gated site evaluates the same
`policy::gate(change, armed_set)` over a `rulepack-api@2` change surface built
from the before/after states. The armed set is loaded and verified inside the
trusted write path from the workspace's own attested `meridian/armed-rules.md`
plus marker, never from a caller:

- wire host: `armed_disk::resolve_at`
  (`crates/wire-serve/src/armed_disk.rs:78`), from the write gate
  (`crates/wire-serve/src/gate.rs:91`) and the reaction feeder
  (`crates/wire-serve/src/reaction.rs:49`);
- run plane: its own `DiskPages` page-source (`crates/run/src/gate.rs:73`).

## What is derived from source

`crates/wire-serve/tests/u12_door_enumeration.rs` is the only instrument that
reads the tree: it walks every crate's production `src/` except `model`,
truncates each file at its first `#[cfg(test)]`, skips lines beginning with
`//`, and looks for two constructor names, `candidate_of_body(` and
`candidate_of_batch(`. A file with at least one such call is recorded
**once**. The test then asserts that this **set of files** equals the set its
pinned table names.

At `7a22e00a`, **three files**:

- `crates/wire-serve/src/write.rs`
- `crates/run/src/fp.rs`
- `crates/wire-serve/src/watch.rs`

**That is the entire source-derived claim: three file names.** The assertion
fails when a candidate is minted in a file not on that list.

## What is NOT derived — do not read it as checked

The test's hand-written seven-door `file::function` table and two further
assertions are not measured against the tree:

- **Which function in a file mints.** Only the file column is compared;
  `file::function` rows are prose.
- **A new mint inside a listed file.** A file is recorded once; a ninth mint
  in `write.rs` changes nothing.
- **The door count.** The seven-row assertion measures the hand-written array
  against itself.
- **Whether any door calls the policy gate.** No assertion ties a call to a
  function; the guard-call count in `write.rs` counts lines, so a call moved
  between functions passes.

Gate coverage is **not stated on this page and not derived anywhere**; a
derivation would rot. **Re-derive or strike, no third state.**
