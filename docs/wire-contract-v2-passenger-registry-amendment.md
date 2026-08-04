# RETIRED — Wire contract v2 amendment: the lock-item passenger registry

> [!WARNING] This amendment is retired (U9b, 2026-08-03). It governs a lock row
> that no longer exists.
> The normative lock-item grammar is **R4 schema v2** — `crates/lock`'s module
> documentation is its implementation home, and the ratified basis is session
> `86449b4e` (08-01), including the 17:20 ruling that removed the top-level
> `objects:` table.

This file is kept as a **knowledge-preserving deletion** (13.5): the prose is
gone, the knowledge is not. What follows is what this amendment established, and
what became of each piece — because a reader arriving from an old citation needs
to know which parts were superseded and which are still law wearing a new name.

## What it governed, and why that row is gone

The amendment described one row of the **`^inputs` lock** that `mrd pin` wrote:
an engine core of `{ref, to, rev, rev_class}` plus editor-declared keys riding
alongside. R4 replaced that row wholesale. `inputs` is dead as vocabulary **and**
as a storage key (R1.3), and the row is now
`{object, hash, path | properties, fingerprint}` plus free-form extras.

## What SURVIVED, renamed

**The engine-core / passenger split is still the law of the row** — it is simply
spelled differently. R4 states it as reserved fields versus free-form extra keys:

| this amendment said | R4 says |
|---|---|
| engine core `{ref, to, rev, rev_class}` | reserved keys `{object, hash, path, properties, fingerprint}` (`lock::PinEntry::RESERVED_KEYS`) |
| **engine-ignored passenger** | **free-form extra key** — stored, rendered, carried VERBATIM, never gates a write, never colors a pin |
| a passenger may not shadow the core | an extra key may not shadow a reserved key — refused |

The property the split protects is unchanged and is worth restating plainly:
**the engine carries what it does not understand, byte for byte.** That is why
the v1→v2 migration refuses to drop an unknown legacy key
(`crates/lockmigrate`), and it is the one rule from this file with live
consequences today.

## What was SUPERSEDED, and by whom

**The append-only registry law is gone, by ZT's ruling of 2026-08-03**, verbatim:
*"user can or can not use claim, its free to use anything"*.

This amendment required a key to land a row in the `^passengers` table **before**
it could ship, so that the set of legal passengers was enumerated and closed.
R4 opens it: any extra key is legal, unregistered, engine-ignored. There is no
registry to append to, and pre-registration is no longer a gate on anything.

Consequences for the two registered passengers:

- **`claim`** — survives as an ordinary free-form extra key. No special status,
  no registry row, engine-ignored exactly as before. `crates/lock`'s round-trip
  test still pins it by name.
- **`at:`** — retired with the row shape that carried it. Its purpose was to
  stamp the commit or tree the pinned bytes came from so `status` could tag a red
  edge `cosmetic`. R4 puts the git blob `hash` on every pin row unconditionally
  ("if hash is missing, we lost the explicit target meaning"), which is a
  stronger fact than the best-effort observation `at:` carried.

## What this file does NOT rule on

**`check:` / `check_rev`** were recorded here as *engine-read* fields — the
engine evaluates them and may refuse on them — explicitly governed by the 23-07
ruling rather than by this registry's append-only law. **Their fate under R4 is
not settled by this retirement**, and U9b does not settle it: this unit owns the
migration door, the tool, the runbook and these doc amendments, and inventing a
ruling for an engine-read field would be widening past the card
(Amendment-3, ask-don't-widen).

Stated so the next reader inherits the question rather than a silence: under R4
an unrecognised key is engine-IGNORED by definition, so an engine-read field must
be a reserved key or it is not engine-read at all. `check:` is neither today.
Whoever owns the rules plane owns that decision.

## Citations

- R4 schema and the `objects:` removal — session `86449b4e` (08-01) and its 17:20 ruling.
- The free-form ruling — ZT, 2026-08-03.
- The row's implementation and its round-trip proof — `crates/lock`.
- The v1→v2 field migration that depends on verbatim carriage — `crates/lockmigrate`.
- The original 23-07 `check:` ruling — `results/round2/zt-rules-plane-rulings.md`.
