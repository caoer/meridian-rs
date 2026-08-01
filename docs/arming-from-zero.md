# Arming from zero — the manual bootstrap ladder (U4.4)

Status: normative for the floor-convention arming ladder. Law: plan §4 Block 4,
U4.4; `docs/laws.md` § Amendment — the policy gate (ATTACK-034 scoping);
`docs/wire-contract-v2-refusal-amendment.md` (§11.1 block-is-a-feature, the §8
refusal taxonomy, the genesis-epoch grey render).

Arming a workspace's law is a **documented manual bootstrap, not tooling.** There
is no `mrd arm` command and no arming automation — arming is a reviewer act,
recorded through the ordinary write door. This page is the ladder a maintainer
climbs to take a workspace from **never-armed** (the gate is a bit-for-bit
no-op) to **steady state** (the door enforces the armed floor).

The floor conventions this ladder arms are the U4.4 suite:
`reviewer-not-owner`, `claim-cas`, `close-verdict`, `decoy-close`,
`verdict-reviewer-bind`, and the `meta-convention` (which guards arming itself).
Each is a real convention folder under `conventions/<slug>/` (`CHECK.md` +
`base/` + `scenarios/`).

## The two states the marker separates

The once-armed marker (`meridian/attested`, defined U4.2) is the pivot. Its
**presence** — not its bytes — records that a workspace has EVER been armed
(`crates/fs/src/domain.rs`, `ATTESTED_MARKER_PATH`). The gate reads it to tell
two worlds apart:

- **never-armed** — no marker, no INDEX. `policy::resolve_armed_set` returns
  `ArmedSet::NeverArmed`; `gate()` is a no-op and every write lands bit-for-bit
  as with no gate at all. The reserved INDEX path (`conventions/INDEX.md`,
  `RESERVED_INDEX_PATH`) is not even read — a stray INDEX cannot arm a
  never-armed workspace.
- **once-armed** — the marker is present. Now the attested INDEX MUST be present
  and valid, or the gate fails CLOSED (`convention_fault`). The marker is created
  on the first arm and never removed.

## The ladder (five rungs)

Climb these in order. Rungs 1–3 are ungated authoring; rung 4 is the genesis
transition; rung 5 is steady state.

### 1. Fill the slot

Author the convention folder `conventions/<slug>/`: `CHECK.md` for a law and/or
`HOOK.md` for a reaction, `base/` for the before-world fixtures, and `scenarios/`
for the firing + passing teaching pages. CHECK keeps its fixed refusal ceiling.
HOOK may emit only its declared caps, with slice 1 pinned to `proto.send`.
`FIX.md` and `VIEW.md` remain named deferrals (U1.3).

### 2. Author the floor

Write the predicate so a refusal always cites its passing scenario (the legal
path). A floor convention's refusal message names its taxonomy rule
(`reviewer_owner`, `claim_cas`, `close_verdict`, `decoy_close`, `reviewer_bind`,
`arming_precondition`) and, where it teaches a winner or a bound reviewer, names
them.

### 3. Test the tiers

Before arming, prove the convention against all three `mrd test` tiers — the
pre-arming gate:

- **scenarios** (`mrd test <dir>`, U1.2) — every named Given/When/Then scenario
  runs through the production write path. A HOOK asserts its emitted effect set
  as `t.result.effects` through the same `^expect` Starlark surface.
- **`--corpus`** (`mrd test --corpus <spec>`, U1.5) — fire-where-expected over a
  governed tree, **zero dead rules**, fuel/heap p50/p99, and FIX/HOOK quiescence
  by a reachable trigger graph plus bounded counterfactual chaining. This tier
  alone may admit `md.*` counterfactuals; it does not widen the armed caps.
  A counterfactual descriptor passes the SAME canonical intent validation an armed
  HOOK's does, and is executed through the production intent→executor adapter and
  the production atomic batch executor. **The isolation is the corpus, not the
  code:** every counterfactual generation lands in a throwaway proof workspace, so
  the governed tree is read-only and the triggering write is never touched.
- **`--history`** (`mrd test --history <ws> --convention <slug>`, U1.6) —
  reconstruct the workspace's own past, report the exact journal span examined,
  and require zero UNDECLARED refusals against the pinned
  `conventions/<slug>/GOLDEN.md` list.

Passing all three tiers is **pre-arm qualification**, not armability. C6a proves
the reaction; it does not invent the attestation contract.

That contract is now settled, and rung 4 states it: attestation pins the PAGE
rev, so a hook page is attestable on exactly the same terms as a check page, and
the activation field is `off|armed`. What remains open is not the contract but
the WIRING — no armed row of either kind reaches the write door yet. The legacy
`conventions/<slug>/` surface still pins `blake3(CHECK.md)`, so a HOOK-only
FOLDER convention remains fail-closed there until that surface is retired.

A convention that has not passed the tiers is not qualified for arming review.

### 4. First arming write — ungated-but-journaled, permanent, genesis-grey

Arming attests a rule: a reviewer approves it AT the rev they read (the
`armed-rev`), then writes the attested row and, on the FIRST arm, creates the
once-armed marker. Arming admits the attestation only when the live rev
(`report-rev`) still equals the approved `armed-rev` — a law that drifted since
approval is refused, never silently armed.

**The attested rev is the PAGE rev, uniformly.** `armed-rev = page_rev(page
bytes) = blake3(bytes)[:16]` (`crates/policy/src/registration.rs`,
`docs/node-rev-merkle-spec.md`). There is no `blake3(CHECK.md)` special case and
no per-kind fingerprint: check pages and hook pages are attested by the same
function. This is the grain that closed the original attestation blocker — a
HOOK page had no `CHECK.md` to hash, so under the old special-casing it could
not be attested at all.

#### What the ARM act attests (the INDEX successor)

The tag-indexed artifact (`crates/policy/src/armed.rs`, written to
`meridian/armed-rules.md`) is the INDEX's successor. One artifact per workspace,
one row per **(id, arm root)**:

| column | content |
|---|---|
| `id` | the page's frontmatter `id:` |
| `page` | workspace path of the RESOLVED page — the override winner |
| `rev` | the page rev the row is attested at |
| `scope` | the ARM ROOT: the root the resolution was narrowed to |
| `mode` | checks `off\|warn\|block` · hooks `off\|armed` |

The act is one indivisible step — narrow to the arm root's chain, resolve through
the one resolver, pin the winner's page and rev — so `scope` cannot drift from
the resolution it describes. It is all-or-nothing and reports every fault at
once: a partial artifact would silently drop a rule the reviewer meant to arm.

Three properties the runbook depends on:

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

#### What is NOT yet wired (read before trusting this rung)

The artifact and its ARM act are landed and tested as a pure policy surface. The
write door has NOT been re-keyed onto them: `policy::resolve_armed_set` still
reads `conventions/INDEX.md` through the folder loader, and no walk yet feeds
discovery from disk. So today, **arming through this artifact enforces nothing at
the door for either kind** — HOOK arming in particular is attestable but not yet
firing. Re-keying the door and retiring the folder surface is the loader-cutover
card's; feeding the guarded write path is C3's.

The **first** arming write is special, and its specialness is permanent:

- **ungated** — at the moment it lands, the workspace is still never-armed (the
  marker does not yet exist), so `gate()` is a no-op. The first-arming write
  therefore lands UNGATED — it cannot be gated by the law it is installing.
- **journaled** — it is still a guarded write, so it appends its row to the
  receipt journal (`op=create path=conventions/INDEX.md … ^r-NNNNNN`). The
  genesis act is present and permanent in the ledger.
- **grey on the enforcement axis, never green** — a never-armed write carries NO
  enforcement verdict (`t.result.verdicts` is empty). Grey is the ABSENCE of a
  green enforcement verdict, not a token. The genesis epoch renders grey; refusal
  never retroactively makes it green (ATTACK-034 scoping; refusal-amendment §
  non-refusing renders — "genesis-epoch write … grey on the enforcement axis,
  never green").

The `meta-convention` guards this rung once it is itself armed: an arming
proposal must pin attested evidence (P@R), declare a structural `cites:` join,
and be armed by a reviewer distinct from the convention's `author` — or the
arming is refused (`arming_precondition`, taxonomy row 8). But the meta-convention
cannot gate its OWN first arming (nothing is armed yet); that genesis write is
grey, exactly as above.

### 5. Steady state

Once the marker exists and the INDEX carries `[x]` rows, the door enforces:
`block` rows refuse a violating write (the bytes never land) with a `{code,
recovery}` pair from the closed §8 taxonomy; `warn` rows render an advisory
finding and land; `off` rows are ignored. A missing or corrupt INDEX on a
once-armed workspace fails CLOSED (`convention_fault`). An armed convention whose
attested page drifts off its pinned `armed-rev` fails CLOSED (`armed_drift`) —
re-arm at the live rev, or revert the law. `--force` is the only escape, and it
is loud: journaled AND rendered.

## What arming does NOT claim (ATTACK-034 scoping)

Refusal makes violations "unrepresentable through an armed change plane" — never
a stronger claim. The genesis epoch renders grey, never green. Out-of-band
mutation (an offline pre-push git rewrite, a root-preserving forged journal row)
is caught by the git witness plus the receipt-engine-only write restriction, or
it is a named residual — it is never rendered green by refusal.
