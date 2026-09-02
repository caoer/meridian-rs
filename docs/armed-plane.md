---
type: spec
id: armed
status: standing
description: Normative spec for the floor-convention arming ladder and the `gate()` seam.
owns: [the arming ladder, the gate() seam]
---

# Armed plane — bootstrap ladder + gate seam

> **Standing:** Design law is `wire-contract.md` (one contract). Mint addresses = segments only. Receipts = armed wire facts. DuckDB/`view_path` not agent core. **Doc correct > code correct; docs first.** See `README.md`.

Status: normative for the floor-convention arming ladder and the `gate()` seam.
Law also: `laws.md` § policy gate; refusal taxonomy in `wire-contract.md` § A.2 / §8.

---

# Part A — Arming from zero

Status: normative for the floor-convention arming ladder. Law:
`laws.md` § policy gate; `wire-contract.md` § A.2 (block-is-a-feature, refusal
taxonomy, genesis-epoch grey).

Arming is a **reviewer act with a verb**: `mrd arm <ID> --mode M --rev R
[--at DIR]` is the attest path — the legal road the binding law's refusals
name. The reviewer reads the resolved page, and the act admits the attestation
only at the rev they read (`--rev` is required and has no live-rev default — a
default would attest bytes nobody read). This page is the ladder a maintainer
climbs to take a workspace from **never-armed** (the gate is a bit-for-bit
no-op) to **steady state** (the door enforces the armed floor).

The floor rules this ladder arms are the floor suite: `reviewer-not-owner`,
`claim-cas`, `close-verdict`, `decoy-close`, `verdict-reviewer-bind`, and the
`meta-convention` (which guards arming itself). Each is a real rule PAGE that
registers by carrying `rules/check` in its `tags:` and an `id:` — it lives
wherever its author put it, and no folder name or filename is load-bearing.

## The two states the marker separates

The once-armed marker (`meridian/attested`) is the pivot. Its
**presence** — not its bytes — records that a workspace has EVER been armed
(`crates/fs/src/domain.rs`, `ATTESTED_MARKER_PATH`). The gate reads it to tell
two worlds apart:

- **never-armed** — no marker. `policy::resolve_armed_law` answers
  `never_armed()`; `gate()` is a no-op and every write lands bit-for-bit as with
  no gate at all. The artifact (`meridian/armed-rules.md`, `ARMED_RULES_PATH`) is
  not even read — a stray artifact cannot arm a never-armed workspace, because
  only an attested arm sets the marker.
- **once-armed** — the marker is present. Now the artifact MUST be present,
  parseable, and attest at least one row, or the gate fails CLOSED. Zero rows is
  the ABSENCE of attestation, not an attestation of absence: a rule deliberately
  not enforced is a row spelled `off`. The marker is created on the first arm and
  never removed.

## The ladder (five rungs)

Climb these in order. Rungs 1–3 are ungated authoring; rung 4 is the genesis
transition; rung 5 is steady state.

### 1. Fill the slot

Author the rule PAGE. It registers by carrying `rules/check` (a law) and/or
`rules/hook` (a reaction) in its `tags:`, plus an `id:` in the rule-id grammar
(`crates/policy/src/registration.rs`, `RuleId`); one
page may carry both legs, sharing one fenced block distinguished by entry point
(`check_change` / `on_change`). A `kind:` key may restate the tag and may be
absent — absent DERIVES from the tag — but it may never contradict it. The check
leg keeps its fixed refusal ceiling; the hook leg may emit only its declared
caps, pinned to `proto.send`. FIX and VIEW remain named
deferrals.

**The page must sit inside the workspace hash domain** (`wire-contract.md`
§12.1): a rules-tagged page on a dot-segment path (`.hidden/rules/x.md`,
anything under a dot directory), or one excluded by a `meridian/domain.md`
ignore rule, registers as NOTHING — law that cannot be hashed cannot be
attested. The exclusion is never silent (§12.1 enumerator clause):
`mrd rules` voices such registration candidates
in one bounded line (complete list on the `not_offered.workspace_dot` key of
its `--json`, exit-neutral), and `mrd arm <ID>` on an id whose only carrier is
domain-excluded refuses naming the file and the exclusion reason. Watch the
ENCLOSING-ROOT shape: a workspace whose own MERIDIAN.md sits on a dot path
resolves to the enclosing root, and every page under it — not just the
obviously hidden ones — is outside that root's domain.

### 2. Author the floor

Write the predicate so a refusal always cites its passing scenario (the legal
path). A floor convention's refusal message names its taxonomy rule
(`reviewer_owner`, `claim_cas`, `close_verdict`, `decoy_close`, `reviewer_bind`,
`arming_precondition`) and, where it teaches a winner or a bound reviewer, names
them.

### 3. Test the tiers

Before arming, prove the convention against both `mrd test` tiers — the
pre-arming gate:

- **`--corpus`** (`mrd test --corpus <spec>`) — fire-where-expected over a
  governed tree, **zero dead rules**, fuel/heap p50/p99, and FIX/HOOK quiescence
  by a reachable trigger graph plus bounded counterfactual chaining. This tier
  alone may admit `md.*` counterfactuals; it does not widen the armed caps.
  A counterfactual descriptor passes the SAME canonical intent validation an armed
  HOOK's does, and is executed through the production intent→executor adapter and
  the production atomic batch executor. **The isolation is the corpus, not the
  code:** every counterfactual generation lands in a throwaway proof workspace, so
  the governed tree is read-only and the triggering write is never touched.
- **`--history`** (`mrd test --history <ws> --rule <page> [--spec <page>]`) — reconstruct
  the workspace's own past, report the exact journal span examined, and require
  zero UNDECLARED refusals against the `golden` fence of the spec page named by
  `--spec`.

Passing both tiers is **pre-arm qualification**, not armability. The tiers prove
the reaction; they do not invent the attestation contract.

That contract: attestation pins the PAGE rev, so a hook page
is attestable on exactly the same terms as a check page, the activation field is
`off|armed`, and armed rows of both kinds reach their surface — check rows the
write door, hook rows the reaction feeder. The composed `pin` axis of `mrd status`
(`status.md` § The composed status line) rolls up exactly this PAGE-rev drift.

A rule that has not passed the tiers is not qualified for arming review.

### 4. First arming write — ungated-but-journaled, permanent, genesis-grey

Arming attests a rule: a reviewer approves it AT the rev they read (the
`armed-rev`), then writes the attested row and, on the FIRST arm, creates the
once-armed marker. Arming admits the attestation only when the live rev
(`report-rev`) still equals the approved `armed-rev` — a law that drifted since
approval is refused, never silently armed.

**The attested rev is the PAGE rev, uniformly.** `armed-rev = page_rev(page
bytes) = blake3(bytes)[:16]` (`crates/policy/src/registration.rs`,
`node-rev-merkle-spec.md`). There is no per-kind fingerprint: check pages and
hook pages are attested by the same function. A reaction-only convention is
therefore attestable on exactly the same terms as a check.

#### What the ARM act attests

The tag-indexed artifact (`crates/policy/src/armed.rs`, written to
`meridian/armed-rules.md`) is the attestation record. One artifact per workspace,
one row per **(id, arm root)**:

| column | content |
|---|---|
| `id` | the page's frontmatter `id:` |
| `page` | workspace path of the RESOLVED page — the override winner |
| `rev` | the page rev the row is attested at |
| `scope` | the ARM ROOT: the root the resolution was narrowed to — a workspace-relative DIRECTORY, `.` for the workspace root. A resolver-style `layer:depth` spelling (`workspace:0`) is refused at parse with a teaching: a head segment carrying `:` is the address grammar's `root:` qualifier (`address-grammar.md` § 4.1 colon law), never a workspace path. The arm root is a DIRECTORY scope, not a page reference, so it sits outside the rooted-lane door family (`address-grammar.md` § 4.6) and this refusal stands. |
| `mode` | checks `off\|warn\|block` · hooks `off\|armed` |

The act is one indivisible step — narrow to the arm root's chain, resolve through
the one resolver, pin the winner's page and rev — so `scope` cannot drift from
the resolution it describes. It is all-or-nothing and reports every fault at
once: a partial artifact would silently drop a rule the reviewer meant to arm.

Four properties the runbook depends on:

- **Arming freezes resolution.** A page that appears LATER — including a deeper
  override candidate that live resolution would now prefer — governs nothing
  until a re-arm. This is the cap-escape guardrail: the tag registers, only ARM
  activates, so no writer can take over an armed id by dropping a file.
- **A pinned page that is edited reddens.** Its row does not fire on its new
  bytes. A red CHECK row refuses the write; a red HOOK row falls silent, because
  a hook may never veto — refusing a write on a reaction's behalf would hand it
  exactly the power it is denied.
- **Mode vocabulary splits by kind.** A hook has no severity axis: it is `off` or
  it fires. A hook row carrying `warn`/`block`, or a check row carrying `armed`,
  is refused at the act, so no artifact can render one.
- **A row that will FIRE must LOAD.** Registration (tag + `id:`) and declaration
  (`severity:`, `caps:`, `budget:`, the block's entry point) are two layers, and
  arming attests both: the act loads the winner
  through `policy::rule::load_rule` — the same loader `armed_law::resolve_armed_law`
  runs on the fire path — and refuses `ArmFault::Unloadable`, naming the loader's
  own fault, **before anything is written to `meridian/armed-rules.md`**. A page
  missing a declaration key therefore never pins a row that could not fire.
  `policy` performs no I/O: the winner's bytes arrive through the injected
  `PageSource`, under the caller's `CheckLimits`.

The load gate runs on the modes that FIRE, never on `off`. Attesting a page `off`
is the reviewer's record that they read it at this rev and chose not to activate
it — including a page too broken to load, which is precisely a state worth
attesting. That set is a SUPERSET of what the fire path loads, not a mirror of
it: `resolve_armed_law` loads `verdict.firing()`, which is additionally narrowed
to the write's own path and excludes reddened rows. The divergence is on the safe
side — arming demands loadability of every non-`off` winner, whatever path it
will later govern.

A page edited between the corpus walk and the act is **drift, not a broken
declaration.** `mrd arm` builds its index before it takes the write flock, so the
drift gate compares the request against a rev that may already be stale; the
loader's own rev law (`RuleLoadError::RevMismatch`) is what catches the race, and
the act re-labels it `ArmFault::Drift` at the rev the loader actually read. Those
bytes are precisely NOT the ones attested, and reporting them as unloadable would
send an operator hunting a declaration bug in a healthy page.

#### The disk edge (wired), and what is still deferred

The door and the reaction feeder are BOTH keyed onto the artifact. Both armed-law
surfaces pivot on the `meridian/attested` marker through one shared reader, so the
workspace cannot disagree with itself about whether it is armed.

The ARM act's DISK EDGE is `wire_serve::armed_disk::ArmSession` (`mrd arm`
drives it): the workspace write flock held from artifact read to commit,
rename-atomic landing, and the crash order **artifact first, marker second** —
a crash between the two leaves artifact-without-marker, which reads as
never-armed: the safe, re-runnable state (the identical re-arm is a no-op).
The marker landing is the act's commit point. The edge deliberately does not
ride the caller door: a direct door write to the artifact is `binding_break`
(row 9) precisely because arming is an attestation, and the act's own law —
`policy::armed::arm`'s faults, the drift check, the strict parse of the
standing artifact — is discharged before the session opens. To every other
process the landing is an external write, observed exactly as an editor's save.

Two deferrals stay open, by name:

- **`arming_precondition` (taxonomy row 8) is not yet evaluated ON the attest
  path.** It never could be: row-9 binding refusal fires before rule evaluation
  on direct writes, and nothing could arm the meta-convention before the verb
  existed. Wiring the armed `meta-convention` into `mrd arm`'s re-arm leg is
  the follow-up rung.
- **The `mrd realise --truth` convergence** over the artifact+marker pair is a
  separate design.

The **first** arming write is special, and its specialness is permanent:

- **ungated** — at the moment it lands, the workspace is still never-armed (the
  marker does not yet exist), so `gate()` is a no-op. The first-arming write
  therefore lands UNGATED — it cannot be gated by the law it is installing.
- **its permanence is the pair itself** — the attested row pins the page and
  rev, and the marker pins the epoch, permanently.
- **grey on the enforcement axis, never green** — a never-armed write carries NO
  enforcement verdict (`t.result.verdicts` is empty). Grey is the ABSENCE of a
  green enforcement verdict, not a token. The genesis epoch renders grey; refusal
  never retroactively makes it green (§ What arming does NOT claim).

The `meta-convention` guards this rung once it is itself armed: an arming
proposal must pin attested evidence (P@R), declare a structural `cites:` join,
and be armed by a reviewer distinct from the convention's `author` — or the
arming is refused (`arming_precondition`, taxonomy row 8). But the meta-convention
cannot gate its OWN first arming (nothing is armed yet); that genesis write is
grey, exactly as above.

### 5. Steady state

Once the marker exists and the armed-rules artifact carries `[x]` rows, the door enforces:
`block` rows refuse a violating write (the bytes never land) with a `{code,
recovery}` pair from the closed §8 taxonomy; `warn` rows render an advisory
finding and land; `off` rows are ignored. A missing or corrupt armed-rules
artifact on a once-armed workspace fails CLOSED (`convention_fault`). An armed convention whose
attested page drifts off its pinned `armed-rev` fails CLOSED (`armed_drift`) —
re-arm at the live rev, or revert the law. `--force` is the only escape, and it
is loud: journaled AND rendered.

## What arming does NOT claim

Refusal makes violations "unrepresentable through an armed change plane" — never
a stronger claim. The genesis epoch renders grey, never green. Out-of-band
mutation (an offline pre-push git rewrite, a root-preserving forged journal row)
is caught by the git witness plus the receipt-engine-only write restriction, or
it is a named residual — it is never rendered green by refusal.

---

# Part A2 — Middleware on the write door

Status: normative for the `rules/middleware` plane. Wire shape:
`wire-contract.md` § A.2.1.

CHECK is yes/no in front of the door. HOOK reacts after commit and can only
`proto.send`. **Middleware is the third kind: check plus transform on the
door itself.** One Starlark eval per armed in-scope middleware page, after CAS
and batch validation, before bytes land. Its outputs:

| Output | Lands | Who applies |
|---|---|---|
| `refuse(message=, passing=)` | nothing committed | engine |
| `set_field(path=, key=, value=)` on THIS file | this put's own batch | engine |
| `set_field` on OTHER files | **same sealed set** as this put | engine |
| `create(path=, body=)` | birth in the same sealed set | engine |
| `send(to=, body=)` | never disk — an **intent** on the response | **host realizes** |

The middleware `create` is **its own constructor** and takes no `props=`: its births ride this put's sealed set, not the create door, so the door-side frontmatter serializer (`run-plane.md` § the machinery floor) is not reachable from here. Middleware frontmatter is `set_field` on the born path, or body bytes.

One caller put may become many disk edits: that is middleware compiling a
batch, not the caller folding payloads. The set is **validate-all-then-apply**
— the caller's write, every middleware edit, and every birth land together or
nothing does. Send cannot ride `write.lock`, so it stays an intent; the engine
never marks it delivered (`armed.intents[]`, § A.2.1).

## Registration and arming

- A page registers by carrying `rules/middleware` in `tags:` plus an `id:`
  (the rule-id grammar) — exactly like the other two kinds; no folder or filename is
  load-bearing. Required frontmatter: `paths:` (scope globs). The leg's entry
  point is `def middleware(ctx)` in the fenced ```starlark block.
- **Mode vocabulary: `off | block`.** Middleware is door law — it can refuse
  and it can transform, so its activation word is `block` (a middleware has no
  `warn` tier, and `armed` stays hook vocabulary). This buys the fail-closed
  law structurally: a red, unloadable, or unevaluable middleware row REFUSES
  the write (`Mode::Block` enforces), exactly as a check row does — a drifted
  transformer silently skipped would be law bypassed.
- Arming is the same attest act (`mrd arm <ID> --mode block --rev R`); the
  artifact row, the binding law over armed pages, and `armed_drift` all apply
  unchanged.
- Eval order within one write: in-scope armed middleware, **`id` ascending
  (lexicographic)**. There is no `priority:` field — pad ids (`000-…`).

## The ctx surface

Middleware evaluates under the CHECK evaluator's limits (`CheckLimits`: fuel,
heap, call-depth, source-size, nesting — no per-page `budget:` in V1) over one
injected `ctx`:

| Member | Carries |
|---|---|
| `ctx.op` | `"splice"` \| `"create"` — the caller's op |
| `ctx.before` | this file before the put (`{path, nodes, frontmatter, edges}` — the `@2` doc facts) |
| `ctx.after` | this file after the pending set SO FAR (caller put + earlier middleware transforms) |
| `ctx.put` | the caller's own edit set: `{op, actor, force, edits, fields_changed, sections_changed, targets}` — what this put asked for, never rewritten by middleware |
| `ctx.fields` | **opaque passthrough** dict from the put frame's `fields` (§ A.2.1). Engine does not interpret keys; `actor`/`now` stay §9 wire inputs |
| `ctx.sql(query)` | ONE read-only SELECT against the current overlay world; returns rows (list of dicts). Not DuckDB DML — writes are `set_field`/`create` emits |
| `ctx.read(path)` | that path's bytes in the same overlay world, or `None` |

**The world** those two accessors read is: the workspace snapshot at flock
time, overlaid with the pending after-state of this file and every edit/birth
middleware already emitted (id order). MW2 reads the world as MW1 left it,
plus this put. The overlay is snapshot-scoped — a later writer is invisible.
`ctx.sql` builds its projection through the host-installed SQL backend
(`wire_serve::middleware::install_sql_backend`; `mrd` and the resident daemon
install a `view::build_memory`-backed one). A middleware that calls `ctx.sql`
on a door with no backend fails CLOSED — the write refuses naming the gap.

## Emits, compiled

- `set_field` on this file joins the caller's own batch as a native
  frontmatter upsert (`SecRef::FmKey`), then the whole augmented batch re-runs
  the door pipeline: `@fp` strip, stored-form translation, lock-artifact
  guard, I4 conformance, and the CHECK gate all judge the FINAL state — a
  middleware cannot smuggle bytes past an armed check.
- `set_field` on another file compiles to that file's own member batch; the
  member runs the same validation and the same CHECK gate at its own path.
- `create` births the page in the same set; an occupied path refuses the whole
  set (`cas_mismatch`, expected absent). Birth bodies pass the same
  document-grain strip and guards a `create` op's body does.
- V1 limit, stated: the **create door** admits `refuse`, this-file
  `set_field`, and `send` — a middleware firing cross-file edits or births
  from a birth refuses loudly as unsupported. The **set door**
  (`splice.set`) and the **remove door** evaluate no middleware in V1.
- **Delete: not built.** No `remove` emit exists.

## Hook scope beside the middleware door

`splice`/`splice.set`/`create` evaluate no `rules/hook` pages and their
responses carry no reaction envelopes (`armed.effects` stays in the shape,
empty on this path) — send is not an engine rule; middleware intents are the
one send lane on the put path. `rules/hook` fires on the external-change
detector (`watch`), where there is no caller to answer. `rules/check` is
refuse-only, same vocabulary, same tests.

`mrd arm` is the attester. First-arm `meridian/attested` is a plane-wide
permanent flip — arming a production workspace is a deployment decision, not
this plane's.

---

# Part B — Gate byte landing

Status: enforcement doc for the `gate()` seam. Law: `wire-contract.md` § A.2; `laws.md` § the policy gate.

**Measured at `7a22e00a`.** This page states a law and describes an
instrument. It contains no census: the predicate a census would need — *lands
bytes, gated or exempt* — is not the predicate the instrument derives.

## The law

`gate()` refuses an armed change **after CAS, before bytes land**
(`wire-contract.md` § A.2). Every gated site evaluates the SAME
`policy::gate(change, armed_set)` over a `rulepack-api@2` change surface built
from the before/after states. The armed set is loaded and verified from the
workspace's OWN attested `meridian/armed-rules.md` artifact + once-armed marker
inside the trusted write path (the wire host loads it through
`armed_disk::resolve_at` at `crates/wire-serve/src/armed_disk.rs:78` — called
from the write gate at `crates/wire-serve/src/gate.rs:91` and the reaction
feeder at `crates/wire-serve/src/reaction.rs:49` — and the run plane resolves
it through its own `DiskPages` page-source at `crates/run/src/gate.rs:73`),
never a caller-supplied set — so no caller can weaken the decision at any gated site.

## What is derived from source

`crates/wire-serve/tests/u12_door_enumeration.rs` is the only instrument that
reads the tree. Stated exactly, because the difference matters:

It walks every crate's production `src/` except `model`, truncates each file at
its first `#[cfg(test)]`, skips lines beginning with `//`, and looks for two
constructor names — `candidate_of_body(` and `candidate_of_batch(`. A file
carrying at least one such call is recorded **once**. The test then asserts that
this **set of FILES** equals the set its pinned table names.

At `7a22e00a` that derived set is **three files**:

- `crates/wire-serve/src/write.rs`
- `crates/run/src/fp.rs`
- `crates/wire-serve/src/watch.rs`

**That is the entire source-derived claim: three file names.** It fails when a
candidate is minted in a file not on that list — which is a real and useful
guarantee, and is the whole of it.

## What is NOT derived — do not read it as checked

The same test carries a hand-written table classifying seven doors by
`file::function`, and two further assertions. None of the following is measured
against the tree:

- **Which function in a file mints.** The set comparison keeps the file column
  and discards the function column, so every `file::function` row is prose. It
  is accurate prose; it is not a check.
- **A new mint inside a file already on the list.** The scan records a file once
  and stops reading it. A ninth mint added to `write.rs` changes the derived set
  not at all.
- **The door count.** The assertion that the table holds seven rows measures the
  hand-written array against itself.
- **Whether any door calls the policy gate.** A guard is a call, not a type, and
  no assertion attributes a call to a function. The test that counts guard calls
  in `write.rs` counts lines in a file; moving a call between functions in that
  file does not fail it.

Gate coverage is therefore **not stated on this page and not derived anywhere**.
Determining it is a source-reading exercise whose result rots, so it is not
restated here as prose nobody checks. **Re-derive or strike, no third state:**
what this page carries is the law, and an honest description of what one test
checks.
