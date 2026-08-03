# Fingerprint-or-force — the wire-origin write guard (U10)

The standing law, recalled from the win tournament (requirements decision 12 /
R1.1): **via the wire, no content change reaches disk without its fingerprint.
`force` is the only bypass.**

## Where the guard sits, and why not at plan lowering

The guard is mounted **per-edit, at the wire-origin splice intake,
post-lowering** — `crates/wire-serve/src/guard.rs`, called from
`write::splice` immediately after `plan::lower`.

Plan lowering was the obvious placement and it is wrong (adversarial finding
1.1/1.2, plan decision P2 revised): lowering is an MCP-only layer. Native
`edits` reach the splice choke-point without ever being lowered, so a guard
mounted there is bypassed by a field rename. The intake is the one point both
faces have already reached. `tests/u10_guard.rs::the_field_rename_bypass_is_closed`
is that finding as a regression test.

Per-edit scope is deliberate: an empty batch — `mrd pin` — has nothing to
demand and passes through untouched.

## The rules

| Change | Guard | Grain |
| -- | -- | -- |
| Any edit mutating existing content | `if_node_rev` | node |
| `set_properties` (frontmatter) | `set_property.rev`, the doc-root token | file |
| A birth (whole file, or a plan `create`) | absence | — |

Frontmatter takes the file grain because frontmatter semantics are file-scoped;
a key-line rev would guard a grain the meaning does not live at (plan decision
P3).

The law is **content-change-scoped, not replace-shaped** — security finding S3.
An append changes existing content and is guarded like every other change; the
replace-shaped reading is exactly what let append escape.
`tests/u10_guard.rs::s3_append_on_existing_content_is_guarded` holds that line.

## `force` and CAS are now one mechanism

Before this unit `force: bool` existed on the write path but was never wired to
CAS: it escaped the armed gate only. `guard_batch` joins them — a forced write
drops its node-grain tokens, so a stale fingerprint lands instead of refusing at
a rung `force` never reached.

Every forced write **names the planes it bypassed** (plan decision P15), as
`fingerprint-or-force` verdicts on the rendered response. The journal is dead by
ruling; the rendered surface is where a caller reads this.

## The refusal teaches

`guard_required` (recovery: fix) carries a message built from the house refusal
primitives (`NO_PARTIAL_WRITE_CLAUSE`, the `mrd config` four-property shape):
subject · cause at its grain · partial state · a runnable fix — plus the one
negative, never an internal mode name. It has its **own** contract assertion
(`assert_guard_contract`), added rather than loosening a shared one, following
the `assert_both_planes_contract` precedent.

## Two exemptions, both deliberate

- **The CLI in-process door is exempt** (`Origin::Cli`) — local-operator trust,
  following the 5.7 `read_mint_required` precedent where the bare CLI carries no
  session actor. A reader who finds `mrd put` writing without a fingerprint is
  looking at a trust-plane split, not a hole. Tested, so it cannot rot into an
  accident: `the_cli_in_process_door_is_exempt`.
- **The run-plane shim door is a NON-GOAL of this unit.** ZT ruled it directly
  (Q3): it is a different trust plane — declared caps plus a detection bracket.
  It is not guarded here and must not be.

`Origin` has no `Default` on purpose: a door added later states its trust plane
or does not compile.

## ⚠ ESCALATED, UNRESOLVED — this unit collides with decision 007

**Two ratified rulings say opposite things, and this unit cannot satisfy both.**

- **Requirements decision 12 / R1.1** (this unit): via the wire, no content
  change without its fingerprint.
- **Decision 007**, bound as conformance probe **MP-7** in
  `crates/testsuite/data/harness/p4-regression-probes.json`: *"requests never
  require revs; guardless splices are legal wire frames forever (mandatoriness =
  Go ratchet, never wire law)"* — and its `kills` field names the failure mode
  verbatim: *"safety ceremony reappearing as wire-level requiredness."*

U10 IS wire-level requiredness. MP-7 fails against this implementation and was
**left failing on purpose**: it is a bound probe of a standing decision, so
editing it to pass would erase the conflict instead of surfacing it. The same
statement appears as prose at `docs/wire-contract-v2.md` line 330.

No frozen v2 **type or byte** moves — `pf_frozen_sweep` is green, and
`if_node_rev` is itself a frozen v2 field, so a v2 client can comply without
learning a new key. What changed is v2 wire BEHAVIOR.

Reconciliation is a ZT question, per the amendment-3 ask-don't-widen law. The
options, none of them a worker's to pick:

1. Amend decision 007 — fingerprint-or-force supersedes it at the wire door.
2. Scope the guard to v3 sessions. **This leaves a bypass**: a client declaring
   `contract: v2` writes unguarded, which defeats the unit.
3. Move the demand out of the wire decode plane into the armed/policy plane, so
   007's "mandatoriness = ratchet, never wire law" survives intact. This
   contradicts P2's placement ruling and would be a re-plan of the unit.

## Named residuals

- **The native face has no file-grain slot.** A native `edits` payload that
  upserts a frontmatter key is guarded at node grain, not file grain, because
  `wire::Edit` is a FROZEN v2 type and adding a field to it is an
  amendment-3 question, not this unit's to answer. The plan face — where P3
  authorized the additive field — carries the file-grain token.
- **The plan `append` shape carries no rev field**, so a wire-origin plan append
  can only proceed with `force` or through the native face with `if_node_rev`.
  The refusal names both paths. Giving `append` its own token is a contract
  amendment beyond this unit's authorization.
