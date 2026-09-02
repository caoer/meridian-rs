---
type: task
created_at: 2026-04-18T20:08:40
updated_at: 2026-04-18T21:40:06
session: 11-04-dialect-sprint
status: done
owner: "[[e5f6a7b8]]"
tags: [type/task]
---

# Task: plan-index

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

Author `results/dialect-sprint/round-3/index/impl-plan.md` — the draft implementation plan for contract-v2 against the engine as it exists, sequenced on the panel ladder (A12).

## Context

Session dir: `/srv/vaults/example-sessions/year=2026/month=04/11-04-dialect-sprint`. Read first:
1. `results/dialect-sprint/round-3/index/schema.md` — the contract this plan implements (authoritative for op shapes).
2. `results/dialect-sprint/round-3/index/notes/repo-seams.md` — the seam map: ladder→crate mapping (§2), CAS/edit seam homes (§3), honest gaps (§5). Its ground truth stands: the engine is a skeleton with frozen types; "stands" = seams/types/tests, not implementations.
3. `results/dialect-sprint/round-3/index/notes/design-decisions.md` — DD-1 (two floors), DD-12 (diff donor), DD-13 (actor as input; authorize stays deferred), DD-16 (lock story, no Rust journal).
4. `results/dialect-sprint/round-3/index/notes/law-digest.md` — L23 (geography), L24 (ladder), §3 (decision gates 12/13/21 conditionals).
Verify seams against `/srv/repos/engine/` directly wherever you build on them (reviewers spot-check paths; a wrong cite fails A12).

## Content requirements

- Six rungs, panel ladder order: dialect → facts/check parity → integrity+diff → write/CAS → subscribe → policy packs. Per rung: what stands (real paths) / what changes (real signatures) / what's new (homes named per repo-seams §3, not invented), acceptance gate (test/bench that proves the rung), and which schema sections it realizes.
- The 10 honest gaps of repo-seams §5 each land in a rung or are explicitly deferred with rationale.
- Daemon-side story per rung where the daemon is a consumer: policy ratchet (007 geography; daemon precedents from repo-seams §4), transitional two-writers-one-lock (DD-16), actor derivation (DD-13, identity parity).
- Decision-gate conditionals: rung 3 diff under both row-12 outcomes; rung 6 evaluator-neutral under row 13; rung 5/views under row 21.
- Perf claims join `perfsuite` `claims.toml` as rows, not prose (repo-seams §1 test/perf layout).
- Honest-tense: no schedule promises; sequencing + dependency edges only.

## Quality gates (record in task body at review)

1. Every cited path/signature verified against the repo this session (record: N cites verified).
2. Ladder completeness: 6 rungs × stands/changes/new/gate, no rung empty.
3. Gaps ledger: 10/10 repo-seams §5 gaps dispositioned.
4. DRAFT banner.

## Rules

- Write ONLY inside `round-3/index/`. Do NOT read any other `round-3/*/` directory.
- DD conflict with repo reality → card blocked + message leader (f6a7b8c9); never silently deviate.
- Silent mode. Done: gates in task body, card → review, one-line report to leader, wrap up (terminate by default).

## Gate results (worker e5f6a7b8, 2026-04-18T18:00-04:00)

Deliverable: `results/dialect-sprint/round-3/index/impl-plan.md` (§Gates self-check appended in-doc).

1. **Cites verified: 47** distinct path/line cites re-verified this session against `engine @ 0a1b2c3` + `daemon @ 1b2c3d4` (full enumeration in the deliverable's §Gates). One repo-seams correction surfaced, not a conflict: `pipe_exec.go`/`tasks_sync.go` are at the daemon repo root (repo-seams cited them directory-less, so its ground truth stands). PASS.
2. **Ladder completeness:** 6 rungs (§1–§6), each with stands (real paths) / changes (real signatures) / new (homes per repo-seams §3) / daemon-side story / acceptance gate / schema sections realized. No rung empty. Decision conditionals placed: row 12 at R3, row 21 at R5, row 13 at R6, both outcomes each. PASS.
3. **Gaps ledger: 10/10** repo-seams §5 gaps dispositioned (§7): gaps 1→R2, 2/3/5/10→R4, 7→R3, 4→R5, 8→R6, 9→spread R1–R6 by design, 6 (Rust journal) explicitly DEFERRED with DD-16/L23 rationale. PASS.
4. **DRAFT banner:** present at top; honest-tense (no dates, dependency edges only); perf claims as proposed `claims.toml` rows with lag-SLA row deliberately excluded per L18. PASS.

No DD-vs-repo conflict found — no block needed.
