# crates/wire-serve/tests/s10_fp_decorate.rs — comment-cleanup notes

## Module charter / SAFETY net (from s10_fp_decorate.rs:1)

S10: claim-link `@fp` decorate-on-read / strip-on-put transform (D10). Two SAFETY tests the unit was designed around (plan §8: S10 is the plan's lowest-confidence unit because the grammar is greenfield):

1. **Round-trip, disk clean** — agent reads DECORATED face, copies decorated link into an edit, puts it. Stored bytes carry no `@fp` token, asserted byte-level over the whole file.
2. **Heading-`@` intact** — `[[Page#Q@Home]]` is a legitimate heading fragment containing `@`. Shaped grammar (D10) must not see an fp token there, on render or put.

Production surfaces: `wire_serve::read::composed_read` (decorated face) and `wire_serve::write::splice` (put).

## Gate 1 round-trip vector (from s10_fp_decorate.rs:167)

Agent context is `rendered_text`, so the link it copies into its next edit is the DECORATED spelling; if put did not strip, the `@fp` token would be authored into the file claiming a fingerprint no engine minted. Assertion is byte-level over the WHOLE file.

RAW face is never decorated — put reads `sections[].content`; a decorated view feeding a write is the A-K1 data-loss class.

Plan batch halves both carry decorated spelling:
- `old` is a NEEDLE matched against STORED bytes (never carry a token). Unstripped, agent could never match the link it just read.
- `new` is the PAYLOAD. Unstripped, token lands on disk.

Native edit vocabulary strips at the same intake.

## Gate 2 heading-`@` (from s10_fp_decorate.rs:291)

`[[Page#Q@Home]]` is a heading fragment that happens to contain `@`. A fully-opaque "anything after `@`" rule would eat it (D10's reason for a SHAPED token). Must survive both planes: put stores verbatim, decorated read renders verbatim.

`put at:end` not `at:content`: minted lock block sits at EOF inside this section; whole-section rewrite would DELETE it. R25 artifact guard refuses that (`s2fix_artifact_guard::a_whole_section_rewrite_that_would_delete_the_lock_refuses`).

## I4 ladder dual intake (from s10_fp_decorate.rs:345)

I4 ladder has ONE owner and TWO entry points (S4a/D4): host's `check_write` PRE-FLIGHT and flocked `splice`. They must judge the same bytes, so strip runs at BOTH intakes — otherwise an agent's decorated `find` would miss in pre-flight and hit in write (two-answers-to-one-question).

## Token colour / false-green trap (from s10_fp_decorate.rs:384)

Token's tone is the PIN's colour; pin identity is the SECTION its `ref` resolves to — never the `^slug` the link addresses.

False-green trap S7 named: slug marker's model span is its HOST LINE, so a decorator that took the slug for identity would recompute an unchanged one-line span and render **green on every body edit**. Here only the pinned section's BODY moves — heading/slug/link untouched — token must go red.

Grey never renders green: pin whose token names a version this build does not implement is `Unverifiable` — decoration says so rather than attested-looking green.

## Gate 3 malformed / decoder strip (from s10_fp_decorate.rs:464)

Malformed `@fp` on an ADDRESS refuses `bad_request` and writes nothing: block-id charset (§2.4) has no `@`, so anything the shaped strip does not recognize dies at `model::Ref::anchor` validation. That refusal is the backstop the strip's ordering makes unreachable for well-formed tokens.

SAME law at the wire's own guard: `decode_anchor` runs `Ref::anchor` before any arm, so strip is ordered there too — otherwise decorated address would be display-only, refused by decoder before the bridge that strips it. Shaped token decodes to STORED spelling; unshaped `@` refuses verbatim.

Well-formed `@fp` on an address is stripped BEFORE `model::Ref::anchor` sees it, so agent-plane decorated address addresses exactly what its stored spelling addresses.

## Review verdict: GO
- reviewer: codex-exec
- code-invariance: pass — PASS: comment-only diff
- info-preservation: pass — SAFETY cases, raw/decorated-plane contracts, dual intake, false-green rationale, and decoder ordering survive in comments or these notes.
- style: pass — Remaining comments are minimal and accurate; no commented-out code or unsafe blocks.
- detail: The notes retain the non-obvious needle/payload distinction, R25 append rationale, malformed-token backstop, and section-versus-slug identity rule.
