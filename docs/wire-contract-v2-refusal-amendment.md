# Wire contract v2 amendment — the armed change plane: blocking §11.1 + refusal taxonomy

Status: normative amendment to `docs/wire-contract-v2.md` §11.1 and §8.
`docs/wire-contract-v2.md` is FROZEN and unedited; this file is the sole
normative text for the block-is-a-feature semantics and for the refusal codes
this module mints. Law: ZT ruling #2; plan unit U4.1; design-lens MAJOR.

## What this amends

Two sections of contract v2:

- **§11.1 (verdicts in write responses)** — v2 says every verdict is a *finding,
  never a decision*, and "whether an `error` *blocks* is Go policy, not engine
  behavior". This amendment makes **block a feature of the engine** for the
  armed change plane. When a workspace is armed, the engine refuses at the door;
  the Go policy layer that used to decide blocking dies with Go.
- **§8 (error taxonomy — six recovery classes)** — this amendment adds the new
  refusal codes the attestation module mints, each bound to one of the existing
  six recovery classes. No class is added, renamed, or removed.

## Why an amendment, and why now

This is the **taxonomy-first** artifact: U2.2 / U2.4 / U2.5 / U2.6 and every
refusal-minting unit in the plan depend on this table landing *before any
refusal implements*, so that every refusal speaks one closed vocabulary from the
first line of code. The named-class floor (ambiguity, dangling, `check_claim`,
convention-fault, armed-drift, binding-break, bypass classes, create/remove CAS,
journal-path) is fixed here; the full inventory below is grep'd from the plan's
§4, and the floor is confirmed complete.

## Why this is an amendment, not a new negotiated rev

Contract v3 (`wire-contract-v3-amendment.md`) bumped the rev because it renamed
wire vocabulary and forbade dual-emit — an old client would misread a new frame.
This amendment does not:

- **New §8 codes are additive by the tolerant-code law.** §8 already rules that
  "a client that doesn't recognize a code dispatches on `recovery` alone". Every
  new code below binds to an existing recovery class, so a v2 client that has
  never heard the code still recovers correctly. No new frame shape, no `hello`
  negotiation.
- **The §11.1 behavior change is arming-gated, not wire-gated.** A never-armed
  workspace is byte-identical to today (U4.2): verdicts stay advisory findings.
  Blocking appears only once a workspace holds an attested INDEX — an in-vault
  state, invisible on the wire until the workspace itself arms. A v2 consumer
  (ccc-statusd) against a never-armed workspace observes no change; against an
  armed workspace it may receive a well-formed §8 error frame (dispatched on
  `recovery`) where it previously got an advisory verdict — a new *outcome*,
  never a new frame shape and never a negotiation.

So this rides the frozen v2 rev. It is a semantics-and-codes amendment, not a
vocabulary rev. v2's §18 waiver ledger stays frozen: per the v3 precedent, this
separate amendment doc is its own normative record, not a new §18 row (which
would require editing the frozen v2).

## §11.1 amendment text — block is a feature

Replace the advisory posture of §11.1 with the following, FOR THE ARMED CHANGE
PLANE only:

1. **The engine decides, not Go.** When a workspace is armed (an attested INDEX
   is present, U1.4), the write door evaluates the change through `gate()`
   (U4.2) after CAS and before bytes land. A verdict of **block** severity, or a
   door-law violation from the closed set below, **refuses the write** — the
   bytes never land, and the refusal carries a §8 `{code, recovery}` pair. The
   INDEX row's severity governs: `block` refuses; `warn` and `off` render as
   findings and never refuse (the §11.1 advisory shape, preserved for those
   rows).

2. **`gate()` reads the workspace's own INDEX, never a caller-supplied set.**
   The armed law is loaded and verified from the workspace path inside the
   trusted write path. A caller passing an empty or forged ruleset cannot
   weaken the decision — the Go-era caller-supplies shape dies with Go (U4.2).

3. **`--force` is the only escape, and it is loud.** A `--force` write always
   escapes an armed refusal; the skip is **journaled** (a permanent receipt row)
   AND **rendered** (a visible violation row on `status` and the board). Never
   silent. This is the sanctioned bypass (decision #6) — force-on-the-index with
   the same alarm property that the refusal itself would have raised.

4. **Never-armed is unchanged.** With no attested INDEX, the door is a no-op
   bit-for-bit: verdicts remain advisory findings per the original §11.1, and
   `budget_exceeded` remains a finding (§11.3), never a wire error.

**ATTACK-034 scoping.** Refusal makes violations "unrepresentable through an
armed change plane" — never a stronger claim. The genesis epoch (pre-first-arming
writes) renders grey, never green. Refusal governs only the armed change plane:
out-of-band mutation (an offline pre-push git rewrite, a root-preserving forged
journal row) is caught by the git witness plus the receipt-engine-only write
restriction, or it is a named residual (U2.1) — it is never rendered green by
refusal.

## §8 amendment — the refusal taxonomy table

Every refusal below carries `code` + `recovery` from §8's CLOSED six-class enum
(`fix / env / refresh / retry / resync / respawn`). A code marked *(existing)*
is already in §8 and is reused, not minted; the rest are new codes, each
statically bound to one class. The **class** column ties each row to the plan's
named-class floor; **surface** names where the refusal fires (door = the wire
write door; harness = `mrd test` scenario runner; bridge = transitional Go leg).

| # | Refusal reason | Named class | Unit | `code` | `recovery` | Trigger |
|---|----------------|-------------|------|--------|------------|---------|
| 1 | selector-ambiguous | ambiguity | U2.2, U2.4 | `ambiguous_ref{candidates}` *(existing)* | fix | write at a selector matching >1 node; refusal names both candidates (n= + ^block) |
| 2 | dangling-anchor | dangling | U2.2 | `ref_not_found{stage,dest?}` *(existing)* | refresh | a pinned anchor's target vanished; nearest-candidate hint; also renders `red(dangling-anchor)` |
| 3 | selector-unresolved | dangling | U2.2 | `no_match` *(existing)* | fix | the write's own selector resolves to nothing; also renders `red(selector-unresolved)` |
| 4 | check_claim-false | check_claim | U2.4, U2.5 | `check_claim{assert}` | fix | a declared `check:` assertion is false at the pinned rev; refuses the whole pin/attest atomically |
| 5 | realised-gate-unmet | check_claim | U2.5 | `realised_gate{claim,state}` | retry | attest's realised gate fails (e.g. `pending-agent`); refusal names claim+state+last-receipt; converge, then re-attest |
| 6 | convention-fault | convention-fault | U4.2 | `convention_fault{index}` | env | armed INDEX absent-on-once-armed, corrupt, or a convention cannot evaluate; fail-closed, names the INDEX/convention |
| 7 | armed-drift | armed-drift | U1.4, U4.2 | `armed_drift{armed_rev,report_rev}` | refresh | arming or gating sees the armed law drifted (report-rev ≠ armed-rev); "re-arm or revert" |
| 8 | arming-precondition | armed-drift | U4.4 | `arming_precondition{rule}` | fix | arming refused — evidence not attested (P@R), actor == author, or `cites:` join fails (meta-convention) |
| 9 | binding-break | binding-break | U4.3 | `binding_break{side}` | fix | a one-sided file↔index change stops at the door (teaching refusal); use the ONE-act path, `--truth`, or realise |
| 10 | index-integrity | binding-break | U4.3 | `index_integrity{target}` | fix | deletion/rename of the INDEX or the once-armed marker refused, citing the floor convention |
| 11 | journal-path | journal-path | U2.1 | `journal_path{path}` | fix | an ordinary `^put`/splice targets the reserved journal path; receipt-engine-only (a bypass attempt) |
| 12 | locked-read-bypass | bypass classes | U2.9 | `locked_read{op}` | fix | `ATTACH` / `COPY` / external access from a view refused; the read face is locked |
| 13 | create-CAS | create/remove CAS | U2.6 | `cas_mismatch{expected,actual}` *(existing)* | refresh | `create` on an existing path (`if_absent` violated); re-read — it exists |
| 14 | remove-CAS | create/remove CAS | U2.6 | `cas_mismatch{expected,actual}` *(existing)* | refresh | `remove` after the target drifted (remove-what-you-read violated); refusal cites the rev |
| 15 | frontmatter-corrupt-guard | (grep-extra) | U2.11 | `would_corrupt{lost}` *(existing)* | fix | a properties patch that would silently corrupt a multi-line list value refuses instead |
| 16 | capability-ceiling | (grep-extra) | U1.3 | `capability_deferred{cap}` | fix | a convention `FIX` / `HOOK` / `VIEW` file refused in v1 (CHECK-only); deferral named |
| 17 | def-invalid | (grep-extra) | U5.3 | `def_invalid{rule}` | fix | `mrd new` / preset birth refused; the `^properties` def rule is violated, named |
| 18 | claim-CAS | (grep-extra — U4.4) | U4.4 | `claim_cas{winner}` | refresh | a contested claim; the loser is refused, naming the winner |
| 19 | reviewer-eq-owner | (grep-extra — U4.4) | U4.4 | `reviewer_owner{owner}` | fix | owner self-close refused (`change.actor == doc.owner`); a different actor must close |
| 20 | reviewer-bind | (grep-extra — U4.4) | U4.4 | `reviewer_bind{reviewer}` | fix | a close Verdict names a reviewer other than the closing actor (ATTACK-024) |
| 21 | decoy-close | (grep-extra — U4.4) | U4.4 | `decoy_close{rule}` | fix | a decoy close fires; the change does not satisfy the real close law |
| 22 | mount-escape | bypass classes | U1.2 | `bad_path` *(existing)* | fix | a scenario `^put` / `t.doc(path)` with an absolute or `..` path escapes the test mount; refused (harness) |
| 23 | unknown-attribute | (grep-extra) | U1.2 | `bad_request` *(existing)* | fix | a scenario `^expect` references an unknown `t.` API attribute; fails LOUD (harness) |
| 24 | rehash-window | (grep-extra, bridge) | U0.3 | `bad_request` *(existing)* | fix | Go attest on a v2 page refused citing the re-hash window (v1-only fence); transitional |

**Reconciliation (exact census).** The nine named-class floor classes are all
present, spanning **15 rows**: ambiguity (1), dangling (2, 3), check_claim
(4, 5), convention-fault (6), armed-drift (7, 8), binding-break (9, 10),
journal-path (11), bypass classes (12, 22), create/remove CAS (13, 14) — the
floor is confirmed complete. The grep of §4 surfaced **9 further** refusal
reasons beyond the named floor: the frontmatter-corruption guard (15, U2.11),
the capability ceiling (16, U1.3), preset birth (17, U5.3), the four U4.4 floor
conventions (18–21), the harness unknown-attribute check (23, U1.2), and the
transitional Go re-hash fence (24, U0.3). 15 + 9 = 24 rows, matching the table
exactly. Rows 22–24 fire off the meridian-rs wire (harness / bridge surfaces);
they map to §8 codes by the same recovery semantics and are listed so the
inventory is complete.

## Non-refusing renders — grey never refuses, and these reds are findings

Grep of the plan also surfaces colors that are NOT refusals. They render; they
never block a write. Listed here so the inventory is complete and the
ATTACK-034 scoping is unambiguous:

| Render | Kind | Unit | Why it does not refuse |
|--------|------|------|------------------------|
| `superseded-algo` | grey | U0.2, U3.4 | an un-recomputable hash-algo renders grey, never red/green. **NO RENDERING SITE in the shipped engine — THE SUBJECT MOVED PLANES, it was not dropped, and the non-refusal law it carried is UNCHANGED on its destination.** Under R4 every pin row carries a mandatory self-describing `fp1.…` token and NO `hash-algo:` header, so "pinned under a `hash_algo` this engine does not compute" is unrepresentable; the case is answered by the FINGERPRINT plane and spelled `unverifiable-fingerprint`, which also renders grey and also never refuses. Recorded, not retired — see § Amendment record below for the per-row citation chain |
| `immutable-root` (`session-id#seq-N`) | grey | U2.2 | transcript-plane refs are read-only; grey, never a refusal |
| `red(drifted)` | red (finding) | U2.2, U2.5 | a drifted pin renders red but **never refuses** (U2.5) — realise converges it |
| `foreign_edit` | red (finding) | U2.10 | an out-of-writer edit renders red convention-free; a check finding, not a door refusal |
| chain-discontinuity | red (finding) | U2.1 | a spliced journal row reddens `check --core` with a row cite; detection, not a door refusal |
| class-C (unreconstructable) | grey | U1.6 | `test --history` counts it grey, never guessed |
| genesis-epoch write | grey | U4.4 | the first-arming write is ungated-but-journaled; grey on the enforcement axis, never green |
| `budget_exceeded` (fuel / heap / stack) | finding | U1.5, U4.4, U1.2 | a typed finding per §8 + §11.3, never a wire error; names the exhausted budget. In armed mode, a convention that cannot COMPLETE its eval under the budget fails closed as `convention-fault` (taxonomy row 6) — the finding never itself refuses |

## Relationship to the other U4.1 artifacts

- The **policy-crate charter amendment** (`docs/laws.md`, § Amendment — the
  policy gate) states the crate-law side: `crates/policy` now owns the blocking
  `gate()` seam, carrying the same ATTACK-034 scoping.
- **`gate()`** (U4.2) is the runtime seam that mints these codes at
  `wire-serve/src/write.rs` — this table is the closed vocabulary it draws from.

## Amendment record — the `superseded-algo` row, tier BRONZE, GOVERNING NOT SETTLED

**Challengeable at U25.** This record and the reading under it are a READING, not
a fact, and are recorded as one so U25 can reject them without reconstructing
what was decided.

**What changed.** The `superseded-algo` row in § Non-refusing renders carried an
UNQUALIFIED PRESENT-TENSE claim that an un-recomputable hash-algo renders grey.
The class has no rendering site in the shipped engine. The row is QUALIFIED, not
struck: the non-refusal law it records is still true of the case, on the plane the
case moved to. **The row's `Unit` column is left as `U0.2, U3.4`** — those are the
acts that MINTED the row, and rewriting them would erase why it exists.

**Why this is record-catch-up and not a change to what the law binds.** Nothing
here alters a refusal code, a byte, or whether any case refuses. The case still
renders grey and still never blocks a write; only its NAME and its PLANE moved.
`GreyReason` lives in `crates/model/src/selector.rs` and has zero presence in
`crates/wire`, so no ratified byte contract moves.

### The citation chain — per row, and it is the condition this act was authorized on

The authorization required this act to cite, per row, the specific ruling acts
that made the row false, and to STOP and escalate if the chain of authority
bottomed out anywhere neither ZT-ruled nor already registered
ruled-and-challengeable. **It was written first and it did not bottom out.**

1. **What made the row false: R4 schema v2 removed the trigger condition.** R4's
   identity is recorded at `docs/wire-contract-v2-passenger-registry-amendment.md`
   § Provenance — *"R4 schema and the `objects:` removal — session `86449b4e`
   (08-01) and its 17:20 ruling"* — and identically at
   `docs/wire-contract-v3-amendment.md` § D12.
2. **R4's authority bottoms out at ZT, ratified explicitly.** Session
   `86449b4e`, `results/design-pin-lock-redesign.md` — *"The pin / lock redesign
   — ZT's rulings, live at step B6"*, `status: ZT-directed during live dogfood;
   ratified points marked`, and *"Marked `RATIFIED` where ZT said so explicitly."*
   Its § 1 **The target lock shape** is marked **RATIFIED**, and the ratified
   shape carries `fingerprint: "fp1.span2.b3.…"` on EVERY pin row with **no
   `hash-algo:` header anywhere in it**. Its field table makes `fingerprint`
   authoritative for *"the attestation itself"*. ZT verbatim: *"the value is the
   lock between hash + path + fingerprint."* **That is why the row's trigger
   condition is unrepresentable rather than merely unused** — a pin cannot declare
   a foreign algo, because the field it would declare it in does not exist.
3. **The act that removed the variant: `006b8ad7`**, the U9c bronze
   `Uncolourable` act — *"edge_color arms 4 and 5 collapse into one fail-closed
   arm"* — which killed `GreyReason::SupersededAlgo`,
   `GreyReason::DeclaredUnpinned` and `model::selector::classify_edge`. Registered
   ruled-and-challengeable at `docs/wire-contract-v2-colors-amendment.md`
   § Amendment record — U9c, tier BRONZE, GOVERNING NOT SETTLED, **Challengeable
   at U25.**
4. **The interpretation that act leans on**, cited because this row inherits it:
   *"SITE in this section means A REACHABLE RENDERING PATH"* — advisor-ruled,
   recorded at `docs/wire-contract-v2-colors-amendment.md` § THE INTERPRETIVE
   STEP **together with the competing reading it was ruled against.**
5. **The destination, which says so in its own words.**
   `crates/model/src/selector.rs`'s `GreyReason::UnverifiableFingerprint` doc
   comment calls itself *"The fingerprint plane's `superseded-algo`"*. That
   variant is produced by `classify_pin` and is a separate ratified class in
   § Colors, so the law was inherited, not lost.
6. **Evidence, explicitly NOT authority — the negative gates.**
   `crates/view/tests/board_pin_verdict_gates.rs` asserts the class is gone,
   including the SQL arm `WHERE il.verdict_color IS NOT NULL AND b.reason =
   'superseded-algo'` under the reason *"a judged pin never renders
   `superseded-algo` — that was the two-plane divergence"*. **A test is evidence
   that a decision shipped; it is never the decision.** Cited in that position
   only.
7. **This authorization itself** is registered ruled-and-challengeable by name for
   U25, alongside the item it authorizes.

**No link in that chain rests on an unregistered ruling.** Had step 2 bottomed
out at direction rather than ratification, this act would have escalated as
amendment-class instead of landing — and that outcome was the equal of this one.

### What this act did NOT decide

The **six malformed-refusal spellings** stay NAMED-UNSWEPT; no conclusion is
drawn from outside the opened sites. The **`unmanaged` open question stays open**.
And the **eight remaining present-tense `superseded-algo` claims** measured
elsewhere in the tree — one further normative member (`docs/norm-v2-spec.md`,
hedged as *"family"*) and six `.rs` doc comments — are **out of this act's scope
and remain open**: this act is doc-only on this file, and code comments are not
this row's to move.

### Invalidation condition

**This act is void and the unqualified row returns if the class becomes
producible again** — that is, if any engine path can render `superseded-algo`, or
if a pin row can again declare a `hash-algo` this engine does not compute. The
test: `grep -rn 'SupersededAlgo' . --exclude-dir=target --exclude-dir=.git` run
bare, with no file-type filter. At `04387a65` it returns **one** line, in
`docs/wire-contract-v2-colors-amendment.md`, recording the variant's death. **Any
hit in `crates/` falsifies this record.**
