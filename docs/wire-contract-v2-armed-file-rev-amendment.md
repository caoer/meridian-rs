# Wire contract v2 amendment — the armed file-rev fact

Status: normative amendment to `docs/wire-contract-v2.md` §4.4 — the `armed`
object of the splice response. `docs/wire-contract-v2.md` is FROZEN and
unedited; this file is the sole normative text for `armed.file_rev_after`.
Law: requirements decision 21 (ZT, 2026-08-04; personal freeze authority per
v2 §18). The act that added the field is ZT's own commit `9365455a`
(2026-07-21, W-5).

## What this amends

The §4.4 worked response prints `armed` as `{path, edits}`. It also carries a
third key, `file_rev_after`, on every committed splice. This amendment records
that key as v2 law and states its semantics; the printed frame in the frozen
document is not edited.

## The ruling

Decision 21 ratifies the field ON V2 — intentional law, never a leak. ZT's
semantics, recorded verbatim-grade:

> `body.armed.file_rev_after` is the whole-file rev AFTER a committed splice,
> so a client learns the new file rev WITHOUT A FOLLOW-UP TOC. Latency only;
> correctness stays fingerprint and `root_after`. ABSENT ON DRY, because
> nothing was written. Same family as `DeltaFile.file_rev_after` and a
> subsequent `toc` `file_rev`.

## The field

`armed.file_rev_after` is the whole-file rev of the spliced path after the
batch commits — same family and width as [`DeltaFile.file_rev_after`] (§7.1)
and as the `file_rev` a subsequent `toc` serves (§4.1). One fact, three frames,
non-drifting: a consumer that reads it and a consumer that re-`toc`s learn the
same value.

It is a LATENCY fact and never a correctness one. The CAS law (§5.1) and the
world-grain `root_after` (§4.4) are unchanged and remain the only guards; a
client that ignores this field is exactly as correct as before and pays one
extra round trip.

## Absent on dry — the law is a bracket, not a single rule

A dry run writes nothing, so the post-write file rev does not exist and the key
does not ride. This mirrors `root_after`'s contractual dry-`null` at world
grain (§4.4), one grain down: at file grain the fact is ABSENT rather than
null, because a dry batch has no post-write file at all to hash.

So the ratified law has two halves and both are load-bearing:

| batch | `armed.file_rev_after` |
|---|---|
| committed (`dry` absent or false) | present — the post-batch whole-file rev |
| dry (`dry:true`) | ABSENT — nothing was written |

## Why an amendment, not a negotiated rev

The field is purely additive on a response, so the §3.2 evolution law covers a
consumer that has never heard of it — "tolerant client — unknown response
fields … are ignored". No `hello` negotiation, no proto bump, no cap string.
The v3 precedent bumped the rev because a RENAME forbade dual-emit; an old
client would misread a renamed frame. This does not.

Per the v3, refusal, effects and passenger-registry precedent, this separate
amendment document is its own normative record rather than a new §18 waiver row
— which would require editing the frozen document.

## Vintage, not provenance: what this field is NOT

The field is v2 law from decision 21 forward. Explicitly:

- **No `rev::demote_v2` row.** A v2 session receives it; that is the point.
- **No v3 split.** One field, one spelling, both revs.
- **No v2-reserved-field registry row.** The registry is for post-v2 vintage;
  this field is not post-v2 vintage, it is v2.

The reaction-plane sibling on the same object, `armed.effects`, is governed by
`docs/wire-contract-v2-effects-amendment.md` and is unaffected by this
amendment.

## Why this record exists

Recording the ruling only in code would have been mechanically sufficient and
practically wrong, and the cost is already measured rather than assumed. The
absence of a `docs/` record for this exact field cost a full escalation:
a reader diffing §4.4 against the live wire found a discrepancy with no record
at the place they were reading, could not resolve it, and escalated; a
frozen-surface sweep sized it; the advisor raised it; ZT ruled. Without this
file the next reader repeats that escalation in full.

A v2 reader must be able to learn the current law from `docs/` alone. That is
what the amendment documents are for.

## Executable record

The shape is pinned on both planes, and the two pins bracket the ruling — the
field rides a committed write and only a committed write:

| pin | asserts |
|---|---|
| `crates/sidecar/tests/u27_v2_key_set_pins.rs::armed_key_set_as_served_on_v2` | a committed v2 splice serves exactly `{edits, file_rev_after, path}` |
| `crates/sidecar/tests/u27_v2_key_set_pins.rs::splice_dry_body_key_set_is_frozen` | a dry v2 splice serves exactly `{edits, path}` |
| `crates/wire/tests/u27_frozen_key_sets.rs::armed_key_set_is_frozen_plus_two_passengers` | the `Armed` type admits exactly the frozen pair plus the two declared passengers |
| `crates/sidecar/tests/splice_e2e.rs`, `crates/wire/tests/contract_v2.rs` | the frozen worked E3/E4 frames, unchanged — their independent agreement is what the ruling confirms |

Both key-set pins are exhaustive `assert_eq!` over the full sorted key list, so
a further addition to `armed` reddens them rather than riding along.
