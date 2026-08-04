# Fingerprint-or-force — the write guard at every wire door (U10)

The law, as ruled by ZT on 2026-08-03: **content-mutating writes on every wire
door require fingerprint match or force.** Guard fields stay schema-optional;
`force` is any client's refuse→rewrite path. See § Ruled below for the
ratification act and what it superseded.

## Where the guard sits, and why not at plan lowering

The guard is mounted **per-edit, at the wire-origin splice intake,
post-lowering** — `crates/wire-serve/src/guard.rs`, called from
`write::splice` immediately after `plan::lower`.

Plan lowering was the obvious placement and it is wrong (adversarial finding
1.1/1.2, plan decision P2 revised): lowering is reachable only through the plan
face. Native `edits` reach the splice choke-point without ever being lowered, so
a guard mounted there is bypassed by a field rename. The intake is the one point both
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

## `force` is rung zero of the ladder

The ruled path is one continuous act: **write → refuse (+ladder) → rewrite**, for
any wire client. So `guard_required` is not a mechanism beside the R1.2
mismatch-recovery ladder — it is where the ladder attaches. The refusal is a
plain error envelope whose `expected`/`actual`/`message` slots are what a rung
enriches; a later unit adds rungs without replacing this.

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

## What the guard does NOT reach — scope, never trust

There are no trust classes here. Every wire door enforces identically, whoever
is behind it. Two paths are outside the ruling's reach because they are not wire
doors:

- **The in-process path** (`Origin::InProcess` — `mrd`, the run plane, the test
  harness). Behaviour unchanged. Tested so the boundary cannot rot into an
  accident: `the_in_process_path_is_outside_the_rulings_reach`.
- **The run-plane effects shim**, a different door entirely — no fingerprint
  there unless ZT re-rules. A NON-GOAL of this unit, per requirements **decision
  18**, which supersedes the earlier Q3 reference as the governing authority.

`Origin` has no `Default` on purpose: a door added later states which side of the
wire it is on, or does not compile. It is door bookkeeping and carries no trust
vocabulary — MCP is the main agent client that implements the refuse→rewrite
path, not a plane of its own.

## Ruled — ZT, 2026-08-03

This unit was escalated: it appeared to collide head-on with decision 007 (bound
probe MP-7: "guardless splices are legal wire frames forever … never wire law").
ZT ruled, verbatim:

> Content-mutating writes on every wire door require fingerprint match or force;
> guard fields stay schema-optional; force is any client's refuse→rewrite path;
> MCP is the main agent client that implements that path, not a separate trust
> plane.

The collision dissolved on an axis nobody had proposed: **frame legality vs
semantic refusal.** 007 protects the FRAME, not the write's success. Its schema
half survives untouched — guard fields stay optional, a guardless splice decodes
— and only its behavioural half is superseded. The refusal is semantic, after
decode; a path that rejected the FRAME would violate the ruling.

Consequences, all implemented here:

- **Every wire door enforces** — the resident daemon and the sidecar alike. The
  sidecar is not an MCP door, which is the point: the law binds the door, never
  the client.
- **No trust planes.** `Origin` is door bookkeeping with no trust vocabulary.
- **The in-process path is out of the ruling's reach** — not a wire door, so
  unchanged. Scope, not trust.
- **Decision 12's "via MCP" is descriptive**, not scoping.

Full record, with the ratification act quoted as such:
`docs/wire-contract-fingerprint-or-force-amendment.md`. The frozen v2 prose is
never edited (v3-amendment precedent); the amendment doc carries the change.

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
