# Amendment — fingerprint-or-force at every wire door (2026-08-03)

Amends the wire contract's write plane. **The frozen v2 prose is never edited**
(the v3-amendment precedent): `docs/wire-contract-v2.md` keeps its wording, and
this document carries what changed.

## The ratification act

ZT ruled, typed verbatim, 2026-08-03:

> Content-mutating writes on every wire door require fingerprint match or force;
> guard fields stay schema-optional; force is any client's refuse→rewrite path;
> MCP is the main agent client that implements that path, not a separate trust
> plane.

## What it resolves

Two ratified rulings appeared to collide:

- **Requirements decision 12 / R1.1** — no content change without its
  fingerprint.
- **Decision 007**, bound as probe MP-7 and stated at `wire-contract-v2.md:330`
  — *"guardless request: legal at the wire forever … Whether a scope requires
  `if_node_rev`/`if_root`/`actor` is the Go policy ratchet (§5.3), never wire
  law."*

The ruling dissolves the collision on an axis neither reading had: **frame
legality versus semantic refusal.** 007 was read as *"a guardless splice must
SUCCEED"*. It says *"a guardless frame must be LEGAL"*. Those are different
claims, and only the second is what 007 protects.

## Decision 007, split

| Half | Status |
| -- | -- |
| **Schema** — guard fields stay optional; a guardless splice is a legal frame that decodes; requiredness never appears at the schema | **SURVIVES, untouched** |
| **Behavioural** — a guardless content-mutating write SUCCEEDS | **SUPERSEDED** by the ruling above |

A content-mutating write with neither a fingerprint nor `force` is now refused
**semantically**, after decode, with `guard_required` (recovery: fix). It is
never a frame rejection. Implementing it as one would make guards required at
the schema and resurrect precisely the ceremony 007 exists to kill — which is
why MP-7's `kills` clause was re-pointed at that failure mode rather than
retired.

## What the ruling binds

- **The DOOR, never the client.** Every wire door enforces identically: the
  resident daemon's socket and the per-workspace sidecar's stdio host alike. The
  sidecar is not an MCP path — it is `sidecar <workspace-root>` over any stdin,
  historically driven by the meridian-go bridge — and it enforces the same.
- **MCP is not a trust plane.** It is the main agent client that implements the
  refuse→rewrite path, nothing more. There are no origin-scoped trust classes at
  wire doors; `wire_serve::guard::Origin` is door bookkeeping and carries no
  trust vocabulary.
- **`force` is any client's refuse→rewrite path**, not an MCP affordance.
- **Decision 12's "via MCP" reads as DESCRIPTIVE**, not scoping — MCP was the
  client anyone had in mind, never the boundary of the law.
- **The in-process path is out of reach.** `mrd`, the run plane, and the test
  harness are not wire doors, so the ruling does not govern them and their
  behaviour is unchanged. That is SCOPE, not trust.

## Where it lives in the code

- `crates/wire-serve/src/guard.rs` — the guard, per-edit, at the splice intake
  post-lowering.
- `crates/wire-serve/tests/u10_guard.rs` — the law's own gates, including the
  frame-legality/semantic-refusal seam.
- `crates/sidecar/tests/u10_every_wire_door.rs` — enforcement asserted at the
  door that is NOT MCP.
- `crates/testsuite/data/harness/p4-regression-probes.json` — MP-7, amended to
  assert 007's surviving half.
- `docs/fingerprint-or-force.md` — the unit's design note.
