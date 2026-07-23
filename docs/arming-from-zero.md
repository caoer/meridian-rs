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

Author the convention folder `conventions/<slug>/`: `CHECK.md` (the `paths:`
scope + the fenced `def check_change(change)` predicate), `base/` (the
before-world fixtures), `scenarios/` (the firing + passing teaching pages). The
CHECK's power ceiling is fixed at v1 — CHECK only, `refuse(message, passing)` the
one builtin; a `FIX`/`HOOK`/`VIEW` file is refused with a named deferral (U1.3).

### 2. Author the floor

Write the predicate so a refusal always cites its passing scenario (the legal
path). A floor convention's refusal message names its taxonomy rule
(`reviewer_owner`, `claim_cas`, `close_verdict`, `decoy_close`, `reviewer_bind`,
`arming_precondition`) and, where it teaches a winner or a bound reviewer, names
them.

### 3. Test the tiers

Before arming, prove the convention against all three `mrd test` tiers — the
pre-arming gate:

- **scenarios** (`mrd test <dir>`, U1.2) — the named firing + passing scenarios
  hold through the production write path.
- **`--corpus`** (`mrd test --corpus <spec>`, U1.5) — fire-where-expected over a
  governed tree, **zero dead rules**, and a fuel/heap p50/p99 budget.
- **`--history`** (`mrd test --history <ws> --convention <slug>`, U1.6) —
  reconstruct the workspace's own past; every would-refuse row is either fixed
  or declared in `conventions/<slug>/GOLDEN.md` with a reason.

A convention that has not passed the tiers is not ready to arm.

### 4. First arming write — ungated-but-journaled, permanent, genesis-grey

Arming attests a convention: a reviewer approves it AT the evidence rev they read
(`armed-rev = blake3(CHECK.md)[:16]`), then writes the attested INDEX row and,
on the FIRST arm, creates the once-armed marker. `policy::arm` admits the
attestation only when the live swept rev (`report-rev`) still equals the approved
`armed-rev` — a law that drifted since approval is refused, never silently armed
(`ArmError::Drift`).

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
CHECK.md drifts off its pinned `armed-rev` fails CLOSED (`armed_drift`) — re-arm
at the live rev, or revert the law. `--force` is the only escape, and it is loud:
journaled AND rendered.

## What arming does NOT claim (ATTACK-034 scoping)

Refusal makes violations "unrepresentable through an armed change plane" — never
a stronger claim. The genesis epoch renders grey, never green. Out-of-band
mutation (an offline pre-push git rewrite, a root-preserving forged journal row)
is caught by the git witness plus the receipt-engine-only write restriction, or
it is a named residual — it is never rendered green by refusal.
