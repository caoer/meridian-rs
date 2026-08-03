# crates/wire-serve/src/decode.rs — comment-cleanup notes

## Module charter (from decode.rs:1)

The hand-rolled strict-decode pass (v2 §3.2 server law, the repo's own recorded obligation — risk R5: fixture-driven because manual).

serde's default silently IGNORES unknown request fields, and `deny_unknown_fields` does not compose with `flatten` — a silently dropped CAS guard corrupts data. So every armed op's field set is validated here by hand, loudly: unknown field → `bad_request` naming it; unknown enum value → `bad_request` (with `unknown_kinds` echoed for `extract`, D-C5); mistyped value → `bad_request`. Ops the wire doesn't arm at this rung answer `unknown_op` (§3.2: an op is in `caps` or answers `unknown_op`).

## SPLICE_V2_FIELDS / SPLICE_V3_FIELDS (from decode.rs:21)

`splice`'s FROZEN v2 field set (wire v2 §4.4). Never grows — a v2 session's field wall is byte-identical for the life of the contract.

`SPLICE_V3_FIELDS` is the ONE owner of "which splice fields exist under v3"; the v3-era amendments are exactly `SPLICE_V3_FIELDS \ SPLICE_V2_FIELDS`. R23: each of those amendments MUST be advertised by the v3 caps projection as `splice.<field>` — enforced by the enumeration test `crate::rev::tests::v3_splice_amendments_are_all_advertised`, which derives its expected set from these two arrays rather than from a hand-copied list.

## decode entry (from decode.rs:48)

Both hosts call this — the sidecar over stdio and the resident daemon over its socket — so the strict pass is one implementation.

`rev` is the session's negotiated contract rev, threaded in by BOTH hosts (M1 U8b rider 1): the ONE rev-dependent decode surface is `splice`'s field list — `plan_edits` decodes under v3 and hits the frozen unknown-field wall under v2 (fixture-pinned). Every other op decodes rev-agnostically (v3-only ops like `read`/`check_write` gate at DISPATCH, unchanged).

Errors: `bad_request` for unknown field/enum/mistype/malformed path/anchor/now/unknown declared contract rev; `bad_path` for path-law violation; `unsupported_proto` for hello whose proto this server does not speak; `unknown_op` for unrecognized op name.

## Per-op notes (from decode.rs arms)

- **hello**: v3-amendment negotiation — unknown DECLARED rev refused LOUD, never silent fallback (`docs/wire-contract-v3-amendment.md`).
- **root**: v2 §4.7 — no parameters; root is world-grain.
- **links**: v2 §4.6 — both fields optional; path absent = whole-corpus edge map; `require_root` is §10.2 opt-in.
- **sub**: v2 §4.7 push path — reserved shape, live at T5-SUB.
- **view_path**: V2 §Q2 view-organ path forwarder. `cwd` is a RAW host path (absolute) the daemon resolves to a workspace — NOT a workspace-relative wire path, so it takes `req_str`, never `req_path` (path-law would reject a leading `/`). `fresh` is the optional bounded-rebuild knob (§Q3).
- **default**: §3.2 discovery honesty — every op is armed as of T5-SUB; only genuinely unknown names land here.

## decode_read (from decode.rs:176)

M1 U4a2 composed read op (v3-only at DISPATCH — decode is rev-agnostic; a v2 session's dispatch answers `unknown_op`, §3.2 discovery honesty against the frozen v2 caps). `actor` is §9 read-provenance slot (D-Actor/B): opaque string, same law as splice's actor — a wire input, never ambient.

## decode_check_write (from decode.rs:227)

M1 U8c I4 def-conformance verdict op (v3-only at DISPATCH). `target` is a RAW host path string (caller's absolute spelling — labels refusal strings + anchors def-layer discovery), so `req_str`, never `req_path`. `edits` is the put-plan vocabulary.

## CREATE_FIELDS / decode_create (from decode.rs:265)

A v3-era op, so unlike `splice` there is no frozen v2 twin: the whole op ships as ONE cap (`create`), exactly like `read` and `check_write`, and no dotted `create.<field>` cap exists. This array is the one owner of "which create fields exist".

`force` is deliberately ABSENT: the guarded door has no forced-birth escape (`write::create`), so admitting the key would advertise a bypass that does not exist.

Rev-agnostic here (like `read`/`check_write`) — the v3 gate is at DISPATCH. `now` takes the SAME RFC 3339 validation `splice` applies: this op writes a journal row stamped with the caller's clock (§9 — engine validates time, never generates it).

## decode_splice (from decode.rs:305)

v2 §4.4: the only write op UNDER V2, batch-only (v3 adds `create`, the birth door; `splice` stays the only op that EDITS an existing file). §9: `now` is RFC 3339, format-VALIDATED never generated.

M1 U8b (rider 1): under a v3 session the field list additionally admits `plan_edits` — the plan-level batch, mutually exclusive with `edits`. Under v2 the list is FROZEN, so a v2 `plan_edits` refuses on the existing unknown-field wall byte-for-byte (fixture-pinned negative).

S7: a `pin` is itself a write, so a pin-only splice is a complete batch. Decoded BEFORE the edits gate because that gate reads its presence. The frozen v2 refusal "missing `edits`" is verbatim for a plan-less, edit-less splice (C note 6: never a serde accident).

## decode_pin (from decode.rs:370)

Strict-decode `splice.pin` (S7): `{target, selector, vibe?}` and nothing else. There is deliberately no `actor` key — a pin's mint identity IS the splice's own daemon-derived actor (D13), so admitting one here would let a caller forge a pin as another actor.

## plan_edits / edits / shapes (from decode.rs:392)

`plan_edits` (v3-only; caller gated on rev): externally tagged items, exactly one tag each, every shape's field set validated by hand — same strict wall as the native edit union. Empty array refuses like the native empty batch (a batch IS its edits).

Empty `edits` array: derived reading (no frozen empty-batch form exists) — a batch IS its edits, and an edit-less commit would mint a Delta with no root advance — unrepresentable under §7.1 "one Delta = one batch = one root advance". Refused loud, recorded as derived-data.

## decode_anchor S10 (from decode.rs:713)

Stage-2 S10: the `@fp` strip is ordered BEFORE the mint-guard here, and this is the FIRST of the two guard sites a wire address meets (the other is `read::to_model_ref`, the belt for in-process callers that build a `SecRef` directly). Decoding to the STORED spelling is what makes the decorated address agent-plane rather than display-only: an agent that read `[[guide#^goal@green.b3af12cd]]` can address `^goal@green.b3af12cd` and reach exactly the node `^goal` names.

Additive on the frozen v2 plane by construction: the only inputs whose outcome moves are shaped tokens, which every pre-S10 build refused and no v2 client can produce (the grammar did not exist). An `@` the shape does NOT recognize still refuses, verbatim, below.

## decode_kinds (from decode.rs:788)

`extract.kinds` against the closed 11-kind enum: any unknown value is `bad_request{unknown_kinds}`, loud (D-C5 — the typo-silently-returns-nothing trap, killed). The valid names are the wire enum's own serde spellings — never a duplicated list.

## Review verdict: GO
- reviewer: codex-exec
- code-invariance: pass — PASS: comment-only diff
- info-preservation: pass — Deleted strict-decode contracts, revision gates, refusal laws, and S10 ordering survive in the notes or concise comments.
- style: pass — Remaining comments are concise and accurate; no narration, commented-out code, or unsafe blocks remain.
- detail: Strict field walls, v2/v3 splice ownership, D13 pin identity, empty-batch/root-advance law, raw-host-path handling, and S10 strip-before-mint-guard behavior are preserved.
