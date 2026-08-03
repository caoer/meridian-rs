# crates/wire-serve/tests/s2fix_promotion.rs — comment-cleanup notes

## Module charter (from s2fix_promotion.rs:1)

fix2b — promotion correctness: anchor promotion is **rev-neutral**, **gated**, and **genuinely last** (review findings 7, 9, 12).

Every test drives production choke-point (`wire_serve::write::splice`) against a real on-disk workspace. Claims asserted AS the claim: byte-identity of both files for a refusal (never a walk reading), FINGERPRINT equality for rev-neutrality (never a rendered colour). Each comparison of two document states carries a CONTROL proving the two states hash DIFFERENTLY when bytes really differ — a rev-neutrality gate whose hash cannot move proves nothing.

## Finding 12 — refused pin byte-unchanged (from s2fix_promotion.rs:114)

Heading whose TEXT carries `/`: lock ref grammar joins heading chain with `/`, so `A/B` cannot round-trip and pin must refuse. Control: promotion this refusal must not leave behind is a REAL byte change (marker is a byte change + different content ids). Refusal is DETERMINISTIC (heading still carries `/`), so re-pin refuses identically and still writes nothing — unlike accepted G3 crash orphan, this shape never had a healing path.

## Finding 7 — rev-neutrality at unterminated EOF (from s2fix_promotion.rs:193)

Target whose LAST LINE is a heading with NO terminator. Promoting into `Guide/Omega` puts marker at EOF, where a terminator gets appended — that byte lies OUTSIDE the marker line, so it moves every enclosing span's canonical bytes.

First pin is whole-page section (heading ref, never bare `#^anchor`: an anchor's span is its host line and would hash the same in every document). Its span runs to EOF, so it is the span an appended terminator moves.

Marker at unterminated EOF must still READ as an anchor, or D15 idempotency would break where the neutrality fix applies: re-pin would fail to see it and try to promote a second one.

Control / reason for the fix: norm-v2 masks an own-line marker, it does NOT mask a bare EOF terminator. Two documents differing by exactly one `\n` at EOF fingerprint DIFFERENTLY — so a promotion that appends one moves the hash. Marker line added the way correct promotion adds it at EOF (no terminator of its own, so norm-v2's R2b takes the one before it) is masked whole.

## Finding 9 — promotion passes armed gate (from s2fix_promotion.rs:287)

Convention pack scoped to pin TARGET only: refuses any change to `guide.md`, says nothing about pinning page. That scoping is DISTINGUISHING — pinning page's own batch passes armed law, so a refusal can only come from gating the promotion itself. Arm through same surface door reads: rule PAGE on disk, attested artifact pinned to live rev, once-armed marker. BOTH files written; artifact alone leaves workspace marker says never armed.

Control: same pin, same fixture, UNARMED — commits. So refusal above is armed law's answer about the target, not a pin that cannot work in this fixture.

Dry pin rehearses the promotion's gate too: armed refusal fires where real write would refuse (§4.4), and rehearsal writes nothing either way.

## Review verdict: GO
- reviewer: codex-exec
- code-invariance: pass — PASS: comment-only diff
- info-preservation: pass — Byte-identity, rev-neutrality controls, EOF normalization, armed-gate discrimination, idempotency, and dry-run contracts survive in comments or these notes.
- style: pass — Remaining comments are concise and accurate; no commented-out code or unsafe blocks.
- detail: Notes retain why bare EOF newline is unmasked, why the whole-page heading pin is the distinguishing span, and why artifact plus marker are required.
