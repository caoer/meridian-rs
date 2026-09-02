---
type: task
created_at: 2026-04-18T19:55:19
updated_at: 2026-04-18T21:57:48
session: 11-04-dialect-sprint
status: done
owner: "[[b2c3d4e5]]"
tags: [type/task]
---

# Task: plan-dialect

## Objective

Author `results/dialect-sprint/round-3/dialect/impl-plan.md` — the draft implementation plan for the dialect-team schema against `/srv/repos/engine/` as it exists (gate A12).

## Context

Session dir: `/srv/vaults/example-sessions/year=2026/month=04/11-04-dialect-sprint`. Required reading: `round-3/dialect/schema.md` (the contract being implemented), `round-3/dialect/briefs/engine.md` (the seam map), `round-3/dialect/briefs/law-stack.md` (the ladder + A12 wording). Verify seams against the real repo where the brief is thin — the plan must survive a reviewer opening the repo.

HARD RULES: write ONLY inside `round-3/dialect/`. Do NOT read any other `round-3/*/` directory. Read-only on the repo. Silent mode: the board is the status channel.

## Steps

1. Sequence on the panel ladder: dialect → facts/check parity → integrity+diff → write/CAS → subscribe → policy packs.
2. Per rung: what STANDS in the engine today (crate/module/file cited), what CHANGES (concrete: new types, changed signatures, new modules), what is NET-NEW; dependency edges to earlier rungs; how the rung is verified (tests/fixtures, incl. existing harness).
3. State where the daemon-side policy ratchet meets the engine (mandatoriness lives in policy, not engine — 007).
4. Honest risk register per rung (S3 standard): what could invalidate the estimate, storage-format re-probe posture on major dependency bumps (A11).
5. A blank-slate plan fails A12 — every rung must name real code it builds on or replaces.

## Quality Gates

- All six rungs present, each with stands/changes/net-new + real citations.
- Every cited seam spot-checked against the repo (state how many you verified).
- Self-check line: A12 → section anchors.

## When Done

Record gate results in this file body, move card to review (`board task move plan-dialect review`), then wrap up (terminate by default).

## Gate Results — [[b2c3d4e5]] 2026-04-18T17:50-04:00

Deliverable: `results/dialect-sprint/round-3/dialect/impl-plan.md`.

- **Six rungs present, each stands/changes/net-new + real citations:** PASS —
  §1–§6 sequenced on the panel ladder (dialect → facts/check parity →
  integrity+diff → write/CAS → subscribe → policy packs); every rung has
  Stands / Changes / Net-new / Depends on / Verification / Estimate & risk,
  every claim cited crate + file:line. The plan's §0.1 documents the re-cut vs
  the repo's stale internal rung numbers (and why the re-cut is what makes S2
  satisfiable).
- **Cited seams spot-checked against the repo:** PASS — repo re-surveyed at
  HEAD `0a1b2c3` (matches the brief); all ten crate lib/main files read in
  full plus root Cargo.toml, tests/main.rs, wire_vocab.rs, PROVENANCE.md,
  claims.toml, engine.proto. **34 citations verified, 0 mismatches**
  (receipts table in the doc's Appendix V); dependency ABSENCES (blake3,
  notify, yaml, mlua, config crate) grep-verified too. One wording nit found:
  brief's "proto Frame field 3 reserved" is literally `reserved 3;` — same
  meaning, noted in Appendix V.
- **Daemon-side policy ratchet stated (007):** PASS — plan §7, four verified seams
  (caps discovery, in-process crate seam C1, authorize deferred per review
  C5 at policy lib.rs:18–24, HOW-as-data + receipt loop).
- **Honest risk register per rung (S3) + re-probe posture (A11):** PASS —
  per-rung invalidators (R1-a…R6-c) + cross-rung re-probe table (plan §8),
  incl. row-13/row-21 either-outcome posture.
- **Self-check A12 → anchors:** PASS — plan §9.
- **HARD RULES held:** wrote only inside `round-3/dialect/`; no other
  `round-3/*/` dir read; repo touched read-only.
