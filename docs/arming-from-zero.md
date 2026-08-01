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

The floor rules this ladder arms are the U4.4 suite: `reviewer-not-owner`,
`claim-cas`, `close-verdict`, `decoy-close`, `verdict-reviewer-bind`, and the
`meta-convention` (which guards arming itself). Each is a real rule PAGE that
registers by carrying `rules/check` in its `tags:` and an `id:` — it lives
wherever its author put it, and no folder name or filename is load-bearing
(registration ruling § 1; the `conventions/<slug>/` folder loader is retired).

## The two states the marker separates

The once-armed marker (`meridian/attested`, defined U4.2) is the pivot. Its
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
`rules/hook` (a reaction) in its `tags:`, plus an `id:` per the § 2 grammar; one
page may carry both legs, sharing one fenced block distinguished by entry point
(`check_change` / `on_change`). A `kind:` key may restate the tag and may be
absent — absent DERIVES from the tag — but it may never contradict it. The check
leg keeps its fixed refusal ceiling; the hook leg may emit only its declared
caps, with slice 1 pinned to `proto.send`. FIX and VIEW remain named
deferrals.

### 2. Author the floor

Write the predicate so a refusal always cites its passing scenario (the legal
path). A floor convention's refusal message names its taxonomy rule
(`reviewer_owner`, `claim_cas`, `close_verdict`, `decoy_close`, `reviewer_bind`,
`arming_precondition`) and, where it teaches a winner or a bound reviewer, names
them.

### 3. Test the tiers

Before arming, prove the convention against all three `mrd test` tiers — the
pre-arming gate:

- **`--corpus`** (`mrd test --corpus <spec>`, U1.5) — fire-where-expected over a
  governed tree, **zero dead rules**, fuel/heap p50/p99, and FIX/HOOK quiescence
  by a reachable trigger graph plus bounded counterfactual chaining. This tier
  alone may admit `md.*` counterfactuals; it does not widen the armed caps.
  A counterfactual descriptor passes the SAME canonical intent validation an armed
  HOOK's does, and is executed through the production intent→executor adapter and
  the production atomic batch executor. **The isolation is the corpus, not the
  code:** every counterfactual generation lands in a throwaway proof workspace, so
  the governed tree is read-only and the triggering write is never touched.
- **`--history`** (`mrd test --history <ws> --rule <page>`, U1.6) — reconstruct
  the workspace's own past, report the exact journal span examined, and require
  zero UNDECLARED refusals against the page's `.golden.md` sibling.

> The **scenario** tier retired with the folder loader. Its atomic unit was a
> convention FOLDER's `scenarios/` directory, and a rule page has no folder to
> hold one. Its coverage was not dropped: each scenario either ports to a
> corpus-tier spec case or is named redundant against a specific surviving test,
> in the accounting delivered at the cutover's gate.

Passing both tiers is **pre-arm qualification**, not armability. C6a proves
the reaction; it does not invent the attestation contract.

That contract is settled and wired: attestation pins the PAGE rev, so a hook page
is attestable on exactly the same terms as a check page, the activation field is
`off|armed`, and armed rows of both kinds now reach their surface — check rows the
write door, hook rows the reaction feeder. The legacy surface that pinned
`blake3(CHECK.md)` and left a HOOK-only convention permanently fail-closed is
retired.

A rule that has not passed the tiers is not qualified for arming review.

### 4. First arming write — ungated-but-journaled, permanent, genesis-grey

Arming attests a rule: a reviewer approves it AT the rev they read (the
`armed-rev`), then writes the attested row and, on the FIRST arm, creates the
once-armed marker. Arming admits the attestation only when the live rev
(`report-rev`) still equals the approved `armed-rev` — a law that drifted since
approval is refused, never silently armed.

**The attested rev is the PAGE rev, uniformly.** `armed-rev = page_rev(page
bytes) = blake3(bytes)[:16]` (`crates/policy/src/registration.rs`,
`docs/node-rev-merkle-spec.md`). There is no per-kind fingerprint: check pages and
hook pages are attested by the same function. This is the grain that closed the
original attestation blocker — under the retired special-casing the pinned rev was
a specific FILE's hash, so a reaction-only convention had nothing to hash and could
not be attested at all.

#### What the ARM act attests

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

The door and the reaction feeder are BOTH re-keyed onto the artifact, and the
folder loader is gone — that was the loader-cutover card. Both armed-law surfaces
now pivot on the `meridian/attested` marker through one shared reader, so the
workspace cannot disagree with itself about whether it is armed.

What is still unwired is the ARM act's DISK EDGE: nothing in production writes
`meridian/attested` and `meridian/armed-rules.md`, and it must write **both
atomically** — an artifact without the marker arms nothing, and a marker without
the artifact fails every write closed. Until that lands, the rungs below are
literally manual: a maintainer writes both files. Deferred by ruling and re-owed
in `[[arm-disk-edge]]`, together with the redesigned `mrd realise --truth`
convergence over the artifact+marker pair.

The **first** arming write is special, and its specialness is permanent:

- **ungated** — at the moment it lands, the workspace is still never-armed (the
  marker does not yet exist), so `gate()` is a no-op. The first-arming write
  therefore lands UNGATED — it cannot be gated by the law it is installing.
- **journaled** — it is still a guarded write, so it appends its row to the
  receipt journal (`op=create path=meridian/armed-rules.md … ^r-NNNNNN`). The
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
