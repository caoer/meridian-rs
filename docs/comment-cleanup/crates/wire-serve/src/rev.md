# crates/wire-serve/src/rev.rs — comment-cleanup notes

## Module charter (from rev.rs:1)

Contract-rev negotiation state + the v3 vocabulary projection (`docs/wire-contract-v3-amendment.md`), lifted so BOTH hosts — per-workspace sidecar and resident registry daemon — project v3 through ONE implementation (arch map A6: "lift, don't duplicate").

The frozen v2 `wire` types serialize BYTE-FOR-BYTE as contract v2 forever — nothing in this module touches them. v3 is a pure projection at the envelope layer: outgoing v2-shaped frames are re-keyed `root` → `fingerprint` on the way out, and incoming v3 requests are re-keyed `fingerprint` → `root` on the way in, so the strict decoder (`super::decode`) and every arm stay v2-only. One rename table, applied in two directions.

**Why a projection, not a serde rename:** A serde attribute change on the `wire` types would break the frozen v2 goldens (`crates/wire/tests/contract_v2.rs`, `crates/testsuite/tests/wire_vocab.rs`) and the byte-identical guarantee live v2 consumers pin via `hello`. The projection keeps v2 emission on the untouched typed path (each host serializes `wire::Response` directly for v2) and only re-shapes when the session negotiated v3.

**No dual-emit (the amendment's hard rule):** A v2 session emits `root` and never `fingerprint`; a v3 session emits `fingerprint` and never `root`. One epoch, one rev.

## Rename table design (from rev.rs:52)

Fields (the fingerprint-VALUED slots): the plain value and every compound name that spells the concept. `expected`/`actual`/`required`/`changed` keep their names — already vocabulary-neutral, they never spell "root".

- RESPONSE_KEYS: response/notification key renames (v2 → v3).
- REQUEST_KEYS: request key renames (v3 → v2). These ride only requests (splice/diff/links), all at the flattened top level — no arbitrary-key map sits there, so a top-level rename is collision-free.
- ERROR_CODES: recovery class unchanged; only the spelling follows fingerprint vocabulary.
- CAP_STRINGS: hello `caps` string renames — the op name and the two dotted field-amendment caps that spell the concept.

## rename_request (from rev.rs:105)

Re-key a v3 request into its v2 form BEFORE strict decode: the `fingerprint` op tag becomes `root`, and the four fingerprint request fields become their `root` names. A no-op for keys the frame does not carry, so a request that happens to use the v2 spelling passes through untouched (input leniency, amendment §"Input acceptance"). Never called for a v2 session.

## project_response (from rev.rs:123)

Re-shape one already-serialized v2 response frame into v3 in place. Touches only the known fingerprint slots under `body`/`error` — never descends into the arbitrary-key maps (`files`, `resolved`, `unresolved`), where a corpus path or raw linkpath could legitimately be the string `"root"`.

Hello body: rewrite the fingerprint caps + echo the negotiated rev. `server` is present on the hello body alone. v3-only caps pushed here so frozen v2 caps lists never carry them and a v2 session stays byte-identical / answers `unknown_op` for those ops (§3.2 discovery honesty):

- `read` (M1 U4a2 composed)
- `check_write`
- `splice.plan_edits` (M1 U8b rider 2 — ONE composite cap for the plan-level splice batch)
- `splice.pin` (S3 U1 / R23: field-only amendment as dotted `op.field` cap; frozen v2 never carries it because a v2 session REFUSES `pin` at the field wall — advertising there would be a false advertisement)
- `create` (BIRTH op — whole v3-era OP, ships as one bare cap like `read`/`check_write`, never dotted `create.<field>`; dotted form names a FIELD amendment to an op that already exists under v2)

diff response: each batch is a `{"delta":{…}}` frame, projected via `project_delta_frame`.

## attach_meta (from rev.rs:189)

Insert the in-band timing block `meta: {duration_us}` beside `body`/`error` on an outgoing frame — the U7 enforcement channel for the ratified sub-second budget (per-op engine duration, integer µs).

v3-ONLY by call discipline: both hosts invoke this on their v3 branch alone, AFTER `project_response`, so the frozen v2 path stays the direct typed serialization, byte-for-byte. For v3 consumers the new top-level key is a safe additive slot under the tolerant-client law (unknown response fields are ignored). Callers convert with `u64::try_from(elapsed.as_micros())` — checked, never a lossy `as`.

## project_delta_frame (from rev.rs:209)

Re-shape one live notification frame (`{"delta":{…}}`) into v3: the two Delta fingerprint slots only, never the `files` array beneath.

## Test: R23 splice amendments advertised (from rev.rs:301)

Every v3-era `splice` field amendment is advertised by the v3 caps projection as `splice.<field>`. Expected set is DERIVED from the decoder's own field arrays — `SPLICE_V3_FIELDS \ SPLICE_V2_FIELDS` — never hand-copied. A second hand-written list would be a second owner for one fact and would drift; deriving it means a splice field added to the v3 decoder without a matching `caps.push` fails this test before it can ship.

v2-era fields are deliberately NOT asserted: `force` is honoured and unadvertised, and advertising it would require changing the FROZEN v2 `CAPS` constant (whole-frame equality, stage-2 criterion 7). It is a stated exception in `docs/wire-contract-v3-amendment.md`.

Anti-vacuity: an empty difference would pass the loop without asserting anything.

## Test: create at op grain (from rev.rs:382)

§3.2 discovery honesty for the BIRTH op, both directions. v3 advertises `create` at OP grain as one bare cap. The dotted `op.field` form names a FIELD amendment to an op that already exists under v2; `create` has no v2 twin to amend, so a `create.<field>` cap would be a category error. Pinned so the next reader does not "complete" the caps list by dotting the birth op's fields.

The projection is the ONLY place the cap is added, so an unprojected (v2) hello body never carries it and a v2 `create` answers `unknown_op` at dispatch.

## Review verdict: GO
- reviewer: codex-exec
- code-invariance: pass — PASS: comment-only diff
- info-preservation: pass — Deleted projection contracts, frozen-v2 rationale, capability laws, and test invariants survive in the notes or concise comments.
- style: pass — Remaining comments are concise and accurate; no narration, commented-out code, or unsafe blocks remain.
- detail: Projection-not-serde rationale, no-dual-emit law, known-slot-only renaming, v3-only cap derivation, attach-meta call discipline, R23 anti-drift test, and create-at-op-grain rule are preserved.
