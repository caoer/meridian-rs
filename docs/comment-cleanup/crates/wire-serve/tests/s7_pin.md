# crates/wire-serve/tests/s7_pin.rs — comment-cleanup notes

## Module charter (from s7_pin.rs:1)

S7: `mrd pin` mints a real `meridian-lock` pin — the read-mint gate (D16), rev-neutral slug promotion (D15), and content+lock in ONE `commit_batch` (D7).

The pin rides `Op::Splice` as a sibling field, so `args.path` is the PINNING page and the lock block is a content edit on it. Every test here drives the production choke-point (`wire_serve::write::splice`) against a real on-disk workspace, and the gate tests are SINGLE-SESSION IN-PROCESS: one `ReadMintStore` held across a read call and then a pin call, which is the only shape that models a daemon session (two CLI processes could not).

## Fixtures (from s7_pin.rs:15)

PINNER: the pinning page (drawing end). No lock block yet, so the first pin BIRTHS one at EOF.

TARGET: the pinned page. `Leader's Guideline` exercises the D15 slug (apostrophe dropped, not separating): the derived id is `leaders-guideline`.

`live_fingerprint`: computes the live fingerprint of a lock `ref` exactly the way the VERIFY plane does — parse the ref through the normative selector grammar, resolve against the live document, hash that span. Every drift assertion goes through here; a pin whose minted token does not equal this value is a pin that reads red the moment it lands.

## GATE 1+3: bare CLI pin (from s7_pin.rs:98)

A pin with NO actor (the local-operator-trusted CLI door, D16) births a canonical lock block carrying `pins:` AND `objects:`, promotes the slug anchor into the target, and mints a fingerprint the verify plane recomputes identically. Placement law: one blank line before the block, one terminator after, page's own bytes untouched. Promotion writes exactly one own-line slug marker, leaving the heading text (and therefore the section's address) untouched.

## GATE 4a: rev-neutral promotion (from s7_pin.rs:177)

Promotion is **rev-neutral** — the pinned section's fingerprint is byte-identical before and after the slug marker lands. Honesty claim behind D14 (promoting into a target this actor may not own is permitted BECAUSE it cannot move that target's fingerprint): if it moved, every OTHER page pinning the same section would redden on somebody else's pin. norm-v2 removes the marker plus its one leading space, so the promoted section hashes identically.

## GATE 4b: idempotent re-pin (from s7_pin.rs:210)

A re-pin recomputes the SAME slug, sees it already present, and promotes nothing. That keeps a benign orphan from accumulating (a counter or random id would leave a new marker per pin, each one a fingerprint-neutral but permanent wart). `upsert_pin` unions in place — still exactly ONE pin.

## GATE 5a: refusal leaves no orphan (from s7_pin.rs:244)

A REFUSAL leaves no orphan. This test used to assert the opposite, and the assertion was the defect (review finding 12): it simulated the G3 crash residual with a deterministic REFUSAL — a corrupt lock block on the pinning page — and then asserted the marker survived it. A refusal is not a crash. A crash is survivable and heals; that refusal repeated identically on every re-pin, so the bytes it left in a page the request does not even name could never heal, and the promotion is now ordered after every refusal rung.

The vehicle is worth keeping precisely because it refuses at a DIFFERENT rung from the ref-grammar repro in `s2fix_promotion` (`lock_engine_edit`, inside the batch composition rather than the prologue): between them they show it is the ORDERING that holds, not one patched rung.

## GATE 5b: crash orphan heals (from s7_pin.rs:304)

The pin's two inodes are two renames (residual G3, unchanged and still accepted), and the promotion's rename is ordered first — so the one survivable failure mode is a CRASH between them: "anchor written, lock not". A refusal can no longer produce that state; the aftermath is staged the way a crash leaves it — the marker on disk, no claim — and the claim under test is what a later pin does with it. The orphan is fingerprint-neutral while unclaimed.

## GATE 2: read-mint gate (from s7_pin.rs:347)

One engine session: a `ReadMintStore` held across a read call and then a pin call — the H1 shape (the ledger is NOT the engine's, so a write cannot evaporate it). Serving the read through the production arm is the point: the receipt under test is the one a real read mints.

Agent path refuses an UNREAD pin — "you cannot attest content that was never in your context" — and the same request succeeds once a covering read has minted its receipt. Gate is keyed on all three parts of the receipt: a FOREIGN actor's read does not authorize mine, and a SIBLING section's read does not authorize a pin into a section nobody saw (D6's grain — the whole reason the receipt is selector-grained rather than doc-level).

A host with NO session layer (the per-request sidecar) cannot know what an actor read, so a pin carrying an actor refuses — and the message NAMES that reason (architecture, not a bug).

## GATE 7: rev-recheck (from s7_pin.rs:480)

A receipt answers "was it read", never "is it current" — so a target edited between the read and the pin yields `write_conflict`, never a silent pin over bytes the actor never saw. Ordered before promotion.

## Edge cases (from s7_pin.rs:542)

- `^id` selector needs no promotion; read face projects anchor rows from LIST ITEMS only (Go parity); pin path inherits that addressability.
- Missing page/selector → `pin_target_missing` rather than minting a claim the drift plane could only render red(dangling).
- `--vibe` writes the blob EAGERLY into the object store; a normal pin only computes the oid.
- Outside git: claim plane lands, retrieval plane ABSENT — honest degradation (D5), never a fabricated sha.
- Dry pin rehearses everything and writes NOTHING while still reporting the plan.
- Second pin UNIONS into existing lock; sole writer mints exactly one block.
- Pin alongside caller edits: both content edit and lock land in SAME sealed batch; armed facts stay 1:1 with REQUEST edits — engine-minted lock reported through pin fact, never as phantom armed row.
- Self-pin of own section: promotion and lock on same file, so pre-image must be re-read between them; pinned section must not be the one the lock lands in.
- Self-pin of the section holding the lock refuses at mint (would be permanently red).
- Taken slug refuses rather than minting a duplicate id.
- Trailing-whitespace heading still promotes rev-neutrally: marker never touches that line (norm-v2 R2 removes whole marker line including terminator). A tail marker would have had to reason about padding.
- Stale `if_root` refuses BEFORE promotion.
- Fresh world guard survives the pin's own root advance: guard honored against the root the client pinned; batch re-guards on the current one. Reported `root_before` is post-promotion.

## Review verdict: GO
- reviewer: codex-exec
- code-invariance: pass — PASS: comment-only diff
- info-preservation: pass — Deleted gate contracts, crash/refusal distinctions, promotion invariants, and edge-case rationale survive in the notes or concise comments.
- style: pass — Remaining comments are minimal and accurate; no narration, commented-out code, or unsafe blocks remain.
- detail: Single-session read-mint gating, rev-neutral and idempotent promotion, refusal-before-promotion ordering, crash-orphan healing, rev recheck, self-pin behavior, and world-guard semantics are preserved.
