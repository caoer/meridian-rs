# crates/wire-serve/tests/s2fix_rule_id_journal.rs — comment-cleanup notes

## Module charter / door (from s2fix_rule_id_journal.rs:1)

R44 P0-1 (R45 shape B), re-keyed by the registration cutover — a rule IDENTIFIER cannot carry a forged `@fp` claim into the RESERVED JOURNAL, because it cannot enter the engine at all.

`force_journal_write` strips the trailing anchor off a line `receipt::journal::render_row` already returned, interpolates `forced_rule={token_safe(&skip.rule)}`, and re-attaches the anchor. `token_safe` is `split_whitespace().join("_")`: it removes no `[`, `]`, `@`, `#`, `^`. So any identifier reaching `ForcedSkip.rule` lands verbatim in the ledger the chain-continuity detector reads — **a forged claim in the journal forges the chain detector's own input.**

What changed is only WHICH identifier that is. It was an armed convention's FOLDER NAME, guarded by `policy::validate_slug`; the folder loader is gone, and `policy::gate` now builds `GateFinding.rule` from the armed row's `RuleId`. So the intake that must hold is `RuleId::parse`'s § 2 grammar (`[a-z0-9][a-z0-9-]*(\.[a-z0-9][a-z0-9-]*)*`), which is strictly NARROWER than the slug charset it replaces.

R45's ruling survives the re-key unchanged: hostile bytes are made **unrepresentable rather than removable**, at INTAKE, so every renderer added later inherits the guard. The id has three intakes and the grammar sits at all three — page `id:` frontmatter (`register_page`), ARM act (`ArmRequest.id`), and reading an attested page back (`parse_artifact`'s `parse_row`). The hostile leg drives the THIRD, because a hand-edited artifact row is the only way a string that never passed the other two can present itself as armed.

THE ASSERT IS THE ARTIFACT (R26): `syntax::fp_removals` over the journal bytes ON DISK, never a string that looks right. Absence is carried by a CONTROL leg that drives the SAME force through the SAME fixture with a legal id and proves the journal row is really written — so "no token" can never be "no row".

## arm fixture (from s2fix_rule_id_journal.rs:62)

Artifact rendered by ENGINE through LEGAL_ID then re-keyed to `id`, so hostile leg reads exact row grammar `ArmedArtifact::render` emits. BOTH files written (artifact + marker): artifact alone leaves workspace marker says was never armed; marker alone is `ArmedFault::Missing`. Hostile leg presents page under hostile id too so row and page agree.

## Gate test reason (from s2fix_rule_id_journal.rs:178)

Id grammar refused the row, so artifact never parsed and nothing was armed under that name. `--force` does NOT escape this — an armed-law fault refuses before the gate reads `change.force`, which makes the absence unconditional. Refusal writes nothing (R32 (1)).

## Control leg (from s2fix_rule_id_journal.rs:199)

Assertion of absence passes on empty world; control drives SAME force through SAME fixture with legal id: armed law refuses unforced, `--force` escapes, journal gains `op=force` row naming `forced_rule=`. Gate measures a door that is open, not machinery that never ran.

## Second ForcedSkip.rule origin (from s2fix_rule_id_journal.rs:226)

`ForcedSkip.rule` has exactly two origins in `policy::gate`: armed row's `RuleId` (closed by intake) and ENGINE-DERIVED `binding-break:{index|file}` of a forced one-sided change. Second is a constant the engine writes, not caller bytes — but undriven door of a named shape is enumeration not closing. Driven: row lands, carries no claim token. Break is now direct edit of the ARMED PAGE (page-side binding break, re-keyed from folder generation's `conventions/<slug>/CHECK.md` to membership in what the workspace attested).

## Review verdict: GO
- reviewer: codex-exec
- code-invariance: pass — PASS: comment-only diff
- info-preservation: pass — Journal injection threat, RuleId intake grammar, three intake sites, on-disk assertion, and both ForcedSkip origins survive in comments or these notes.
- style: pass — Remaining comments are concise and accurate; no commented-out code or unsafe blocks.
- detail: The notes retain why force cannot bypass corrupt attestation, why the legal-id control prevents vacuity, and why artifact plus marker are both required.
