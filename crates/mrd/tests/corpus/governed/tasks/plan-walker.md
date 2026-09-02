---
type: task
created_at: 2026-04-18T19:58:30
updated_at: 2026-04-18T21:36:46
session: 11-04-dialect-sprint
status: done
owner: "[[d4e5f6a7]]"
tags: [type/task]
---

# Task: plan-walker

## Objective

<!-- What done looks like. Identity-free: no "You are a WORKER", no ids —
     the roster carries who; this file carries only the work. -->

## Context

<!-- Authoritative reading, wikilinks to specs/decisions. -->

## Steps

## Quality Gates

## When Done

Move to review (`board task move <slug> review`) — a reviewer, not the
owner, gates review → done.

## Objective

Author `results/dialect-sprint/round-3/walker/impl-plan.md` — the draft implementation plan for the walker team's contract-v2 against `/srv/repos/engine/` AS IT EXISTS (gate item A12).

## Inputs

1. `round-3/walker/schema.md` + `round-3/walker/notes/design-stance.md` — the design you are planning the build of (follow the stance; do not re-litigate decisions)
2. `round-3/walker/notes/repo-map.md` — repo reality per rung
3. `round-3/walker/notes/law-digest.md` — the ladder + constraints
4. Spot-verify seams in `/srv/repos/engine/` directly wherever the plan touches them — reviewers will check citations against real code (A12: blank-slate spec fails)

## Deliverable shape

Sequenced on the panel ladder — one section per rung, in order: dialect → facts/check parity → integrity+diff → write/CAS → subscribe → policy packs. Per rung:
- What STANDS (existing engine code carried, real crate/module/type citations)
- What CHANGES (modified seams, new modules — named and placed in the real tree)
- Contract surface delivered at that rung (which schema.md ops/nouns go live)
- Verification story (how the rung proves itself — parity checks, fixtures, oracle)
- Dependencies on prior rungs

Plus: daemon-side policy-ratchet story (where mandatoriness lives, informed by the daemon's patterns per repo-map) and honest risk notes (A11 posture).

## Rules

- Write ONLY under `round-3/walker/`. Do NOT read any other `round-3/*/` directory.
- Silent mode: report to your leader only (envelope sender), never the advisor (a1b2c3d4).

## Quality gates

- Every rung cites ≥2 real repo paths; zero invented modules (spot-check yourself before review)
- Ladder order exact; each rung's contract surface traces to schema.md sections

When done: record gate results in this card, move it to review (`board task move plan-walker review`), send your leader a 5-line report, then wrap up (terminate).

## Gate results (owner self-check, d4e5f6a7, 2026-04-18T17:50-04:00)

- **Deliverable:** `round-3/walker/impl-plan.md` authored — 6 rungs in exact panel-ladder order (dialect → facts/check parity → integrity+diff → write/CAS → subscribe → policy packs), each with stands / changes / contract surface / verification / dependencies; plus daemon ratchet story (§7) and 10-row risk register (§8).
- **≥2 real repo paths per rung:** PASS — every cited crate/module/type/line was read directly from `/srv/repos/engine/` this session (wire, model, fs, wire-map, transport, policy, query, syntax, testsuite tree, perfsuite tree) and `/srv/repos/daemon/internal/registry/` (resolve.go, slot.go) + hooks.go refs per repo-map. Per-rung path list in impl-plan §9.
- **Zero invented modules:** PASS — all existing paths verified by direct read; new code explicitly marked NEW with placement (`model/src/walk.rs`, `model/src/delta.rs`, `fs/src/domain.rs`, wire types/ops, one new crate `receipt` with growth-rule rationale).
- **Ladder order exact + contract surface traces to schema.md:** PASS — numbering declared once (§0, repo-rung skew flagged per schema §14); every rung's "Contract surface delivered" cites schema §-numbers.
- **Stance compliance:** followed design-stance §D — sequenced against schema §14 table; sealed-ValidatedSplice + wire-agreement drift pin leaned on as mechanical allies; both deliberate contradictions (blake3, match-based SpliceRequest) cite the repo's own deferred-decision notes verbatim.
