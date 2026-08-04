# crates/wire-serve/tests/u5_4_substrate.rs — comment-cleanup notes

## Module charter / two gates (from u5_4_substrate.rs:1)

U5.4 remaining substrate debts — merge-gate acceptance suite. Two named gates, each falsification-verified:

- **Gate 1 — heading-rename round-trip preserves block ids + reddens hpath-pinned dependents correctly.** `query::plan_rename` (rung-5 gap, heading-rename op) applied through strict writer (`model::validate_batch` → `fs::apply_batch`, query charter's "model validation + fs execution"). After rename: every `^block-id` survives (`preserves_block_ids`), hpath pin reddens `red(selector-unresolved)`, body-`^block-id` pin still resolves at an unchanged rev (`model::selector::resolve_selector` — the address half; the `node_rev` compare it used to assert through was retired with `classify_edge`), renaming BACK restores original bytes → the hpath pin matches its original rev again (true round-trip). Falsification: body-DROPPING rename loses `^abc` — block-id assertion discriminates.

- **Gate 2 — general N-edit close batch lands atomically with one rev mint.** 3-edit `splice` (frontmatter `upsert` + two body `match`es) commits through `wire_serve::write::splice` in ONE root advance, one receipt line. Falsification: one invalid edit refuses the WHOLE batch — card byte-identical (valid `status: closed` never lands), no receipt, no rev minted.

## apply_plan (from u5_4_substrate.rs:87)

Apply `RenamePlan` through strict writer: validate each `(path, batch)` splice and land atomically via `fs::apply_batch`. A refusal panics loud — the plan is supposed to validate.

## Gate 1 round-trip detail (from u5_4_substrate.rs:115)

Section rev is heading-inclusive, so exact restore is exact rev (hpath pin greens again). Block pin stays green: body block byte-untouched, rev unchanged.

## Gate 1 falsification (from u5_4_substrate.rs:200)

Broken "rename" that replaces whole section with just heading line (`put at:all`) — shape `plan_rename` deliberately does NOT emit. Guard the real rename upholds is exactly this negation: body-dropping rename LOSES the block id so `preserves_block_ids` truly discriminates. hpath-pin `red` is what a broken rename would still leave green.

## Backlink fixup (from u5_4_substrate.rs:246)

Rung-5 op is heading rename WITH backlink fixup: every wikilink/embed whose fragment is the renamed heading is rewritten (`#Foo` → `#Bar`), alias-preserving, while a `^block-id` link is left untouched (anchor survives). Cross-file and sub-section links both covered.

## Gate 2 atomicity (from u5_4_substrate.rs:289)

Canonical close batch: frontmatter upsert + two body matches, mixed in ONE splice. Distinct-target, disjoint spans. ONE root advance = one rev mint; ONE delta batch at seq 1. One delta carries content card plus receipt — atomicity fact is single seq/root advance, not file count.

## Gate 2 falsification (from u5_4_substrate.rs:417)

One invalid edit (`match` with no occurrence) refuses WHOLE batch — zero partial splice, no rev minted. Load-bearing: valid `status: closed` never lands; non-atomic writer would leak it.

## Review verdict: GO
- reviewer: codex-exec
- code-invariance: pass — PASS: comment-only diff
- info-preservation: pass — Both gates, falsification logic, selector-color contracts, backlink scope, and atomicity evidence survive in comments or these notes.
- style: pass — Remaining comments are minimal and accurate; no commented-out code or unsafe blocks.
- detail: Notes preserve the strict rename application path, heading-inclusive rev rationale, discriminating body-drop case, and single-seq/root-advance meaning.
