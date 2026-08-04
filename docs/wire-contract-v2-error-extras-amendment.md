# Wire contract v2 amendment — error-frame extras, per code

Status: normative amendment to `docs/wire-contract-v2.md` §8.
`docs/wire-contract-v2.md` is FROZEN and unedited; this file is the sole
normative text for the error-frame extras below. Advisor ruling, 2026-08-04, on
the U27b sweep.

## What this amends

§8 writes each code's extras in `code{extras}` notation — `cas_mismatch{expected,actual}`,
`stale_view{required,as_of_root,live_root}`, and so on. **That notation is
exhaustive**, and three codes demonstrate it rather than assert it: those two
plus `ref_not_found{stage,dest?}` each predict the served key set exactly,
optional member included.

Read exhaustively, four §8 codes serve extras the table does not name, and one
amendment code serves extras nothing names. This amendment declares them.

**The wire is not wrong here.** The extras implement ruled refusal behaviour —
the §8 recovery binding and the house refusal convention — so the documents
lagged the engine rather than the engine departing from them. Nothing in this
amendment changes what any door serves.

**Six keys, on five codes.** The U27b sweep first reported one of the six as
already declared elsewhere; that citation rested on a non-normative document and
does not stand (advisor ruling, 2026-08-04). All six are declared here.

## The declared extras

| code | §8 / prior declaration | extras declared here | why |
|---|---|---|---|
| `bad_path` | fix class, no extras | `path` | the offending path, echoed, so the caller sees which spelling was refused |
| `file_not_found` | env class, no extras | `path` | the requested path, echoed — the env fault names its subject |
| `unsupported_proto` | respawn class, no extras | `supported` | the protos this engine speaks; a respawn-class refusal that does not say what would work is not actionable |
| `guard_required` | code + recovery only, by `wire-contract-fingerprint-or-force-amendment.md` — no frame, no extras | `message`, `path` | the refusal must teach; the content convention it follows is described below |
| `bad_request` | fix class; extras declared per call site (`unknown_kinds` §4.3, `id_raw` §3.1, `overlap` §4.4) | `message`, OPTIONAL ON THE CODE | see normalization, below |

## `message` is optional on the code, never on the call site

Before this amendment `message` rode `bad_request` at some call sites (the §4.4
disjointness refusal) and not at others (`unknown_kinds`, `id_raw`) — one code,
inconsistent with itself. A declared field rides the CODE, not the call site, so
this amendment declares it that way:

> **`message` is an OPTIONAL human-readable slot on every error frame.** It
> carries teaching, never dispatch data: a client dispatches on `code` and
> `recovery` (§8), and MUST NOT parse `message`. Its absence is never
> meaningful — a refusal without one is not a lesser refusal.

Whether the serving sites backfill a message uniformly is an implementation
question with measurement behind it, not something this document assumes. The
declaration makes a message legal everywhere and required nowhere, so a site
that gains one later needs no further amendment.

## `guard_required` — the keys are declared HERE, and only here

**This amendment is the first normative text to declare either key.** Neither is
inherited, and the distinction matters because it was tested: no normative
member prints a `guard_required` frame at all. `wire-contract-fingerprint-or-force-amendment.md`
declares the code and its recovery class in one sentence and prints no frame;
`crates/testsuite/data/harness/p4-regression-probes.json` asserts
`"error_code": "guard_required"` and no key beside it.

The message's CONTENT follows the house refusal convention described in
`docs/fingerprint-or-force.md` — a message built from the refusal primitives:
**subject · cause at its grain · partial state · a runnable fix**, plus the one
negative, never an internal mode name — enforced by `assert_guard_contract`.

That document is the unit's DESIGN NOTE and is **not normative** (advisor
ruling, 2026-08-04, on ZT's scope-of-assent doctrine DX-01: elaboration beside
ratified content is not itself ratified, and a document does not become law by
containing true sentences). It is cited here as the source of a convention the
engine already follows, never as the authority for the key. The authority for
the key is this amendment.

`path` carries the subject at frame grain, so a consumer that renders the code
without the prose still names the file. Worked:

```json
{"id":27,"ok":false,"error":{"code":"guard_required","recovery":"fix",
 "path":"notes/plan.md",
 "message":"section \"Goals/Q3\" in notes/plan.md changes existing content with no fingerprint — …"}}
```

## The unchanged remainder of §8

Every other code's extras stand exactly as §8 prints them. In particular this
amendment adds NO extras to `unknown_op` (`{code, recovery}` and nothing else),
and does not touch the closed six-class recovery enum, which gains no class and
loses none.

**`root_mismatch` is deliberately out of scope.** §8 and the §18 ledger both
spell `{expected, actual, changed}` while the engine serves `changed` never —
doc and fixture agreeing against the wire, the opposite polarity from everything
above. It is carded separately and nothing here should be read as settling it.

## Why an amendment, not a new negotiated rev

§8 already rules that "a client that doesn't recognize a code dispatches on
`recovery` alone", and §3.2's tolerant-client law ignores unknown response
fields. Every extra above is additive on an existing code bound to an existing
recovery class, so no client misreads a frame, and no `hello` negotiation or
proto bump is implied. Per the v3, refusal, effects, passenger-registry and
armed-file-rev precedent, this separate document is its own normative record
rather than a new §18 waiver row — which would require editing the frozen v2.

## Executable record

| pin | asserts |
|---|---|
| `crates/sidecar/tests/u27_v2_key_set_pins.rs::remaining_frozen_error_key_sets_are_pinned` | `bad_path`, `file_not_found`, `unsupported_proto`, `unknown_op`, and the three `bad_request` forms, each an exhaustive key set from the wire |
| `crates/sidecar/tests/u27_v2_key_set_pins.rs::guard_required_error_key_set_is_pinned` | `{code, message, path, recovery}` |
| `crates/wire/tests/u27_frozen_key_sets.rs::error_body_key_set_is_frozen_plus_the_v3_ladder_extras` | the `ErrorBody` type's full admitted key set |

## Provenance

Found by the U27b silent-shapes sweep (worker `d8260642`): the five rows above
were the shapes whose key sets no fixture had ever asserted, so the divergence
had never been visible to a gate. The `bad_request` self-inconsistency is what
identified the cluster as accretion rather than five separate decisions.
