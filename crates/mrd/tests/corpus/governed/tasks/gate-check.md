---
type: task
created_at: 2026-04-18T19:55:19
updated_at: 2026-04-18T22:20:21
session: 11-04-dialect-sprint
status: done
owner: "[[c3d4e5f6]]"
tags: [type/task]
---

# Task: gate-check

## Objective

Execute last-gate Part A (A1–A14) against the four dialect-team deliverables as an adversarial reviewer, producing `results/dialect-sprint/round-3/dialect/briefs/gate-report.md` with per-item PASS/FAIL + receipts, and a concrete fix list for every FAIL.

## Context

Session dir: `/srv/vaults/example-sessions/year=2026/month=04/11-04-dialect-sprint`. The gate: `results/dialect-sprint/gate-3/last-gate.md`. The deliverables: `round-3/dialect/schema.md`, `impl-plan.md`, `skill/SKILL.md` + `skill/docs-outline.md`, `index.html`. Law sources reachable via `round-3/dialect/briefs/law-stack.md` citations — verify against SOURCES when a pass/fail hinges on exact wording.

HARD RULES: write ONLY inside `round-3/dialect/`. Do NOT read any other `round-3/*/` directory. Silent mode: the board is the status channel.

## Steps

1. Per A-item: quote the deliverable line(s) that answer it (file + section anchor as receipt), verdict PASS/FAIL. Adversarial posture: an asserted answer without structure is a FAIL — gate-3 reviewers will treat it that way.
2. A4 spot-verification: recompute at least 3 worked values (blake3 etc.) from the page/schema; show the commands and outputs.
3. A5: cross-check the coverage table's pattern count against `results/usage-mining.md` directly.
4. Conditional items (A6/A9/A10): verify BOTH outcomes of decision rows 12/13/21 are addressed, not one assumed.
5. Cross-deliverable coherence: schema ↔ page ↔ skill tell one story (same noun names, same verb semantics, same values). Any drift is a finding.
6. Write the fix list: per FAIL, the minimal concrete change and which file owns it.

## Quality Gates

- 14 verdict rows, every verdict carrying a quoted receipt.
- ≥3 recomputed values shown with commands.
- Zero verdicts based on the deliverable's own self-check table (independent evidence only).

## When Done

Record gate results in this file body, move card to review (`board task move gate-check review`), then report and await next-step instructions — do NOT self-terminate.

## Gate Results (recorded 2026-04-18T18:40-04:00, worker c3d4e5f6)

**Tally: 14 PASS / 0 FAIL.** Full report with receipts:
`results/dialect-sprint/round-3/dialect/briefs/gate-report.md`.

- A1–A14 all PASS; every verdict carries quoted deliverable receipts; zero verdicts sourced from the deliverables' self-check tables.
- A4: 12 values recomputed independently from deliverable text (own script, not the team's) — incl. full merkle reconstruction of R0/R1/R2 and the 295-byte receipt file; all match. Team script corroborates ("all 11 pins reproduced byte-exact").
- A5: 40/40 confirmed directly against `results/usage-mining.md` (31 tool rows + M1 + C1 + 7 friction items); all 31 invocation counts byte-identical; the P5 "61 heading-fragment reads" correction verified at walker r2:369.
- A6/A9/A10: rows 12/13/21 BOTH outcomes verified addressed (schema §4.4 / §11 + plan R6-a/b / §10).
- A12: 10 of 34 Appendix-V repo citations independently re-verified @ `0a1b2c3`, 0 mismatches (incl. absence claims).
- A14: own tag-balance walker clean, zero real external refs, all anchors resolve.

**Findings (fix list, none verdict-flipping):** F-1 **P1** — worked session serves `diff` (id:10) while `hello.caps` omits it, violating the schema's own caps law (fix: add `"diff"` to §6.1 example, schema + page) · F-2 P2 — `edit.dry` in plan caps spine but absent from schema caps example · F-3 P3 — plan lists `hello` as a cap · F-4 P3 — §8 delta-vs-e-table note omits `no_match` as an addition · F-5 nit — description char count 583 vs claimed 599. Owners + minimal changes in the report.
