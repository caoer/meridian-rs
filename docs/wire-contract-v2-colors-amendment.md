# Wire contract v2 amendment — the color law (§ Colors) and the composed legend

Status: normative amendment to `docs/wire-contract-v2.md` on the render / read
plane. This file is the doc home of the D2-X3 door-fact-surface sentence, the
D2-F4 four-grey enumeration, the anchor-state qualifiers, and the composed
legend. `docs/wire-contract-v2.md` is FROZEN and unedited; this file is the sole
normative text for the color law. The implementation plan names this artifact
"contract v4 § Colors" — colors are render-plane, so it rides the existing rev,
NOT a `v4` wire rev (see § Why an amendment, not a new rev). Law: D2-X3; D2-F4;
ZT "yes. no git" (`results/round2/zt-rules-plane-rulings.md`); design-lens legend
finding.

## Why an amendment, not a new negotiated rev

A color is COMPUTED per query run, never stored, and never rides a frame: the
wire serves revs, and `mrd status`, `mrd check`, and the board view derive the
color from those revs locally. A render law changes no frame shape and forces no
`hello` negotiation. The v3 amendment bumped the rev for a vocabulary rename an
old client would misread; a color law does not. A declared wire rev `"v4"` is in
fact refused LOUD as unknown (`bad_request`, `wire-contract-v3-amendment.md`
§ Negotiation mechanism). So this is a v2-riding render amendment, not a `v4`
wire rev — per the v3 and refusal-amendment precedent, a separate normative doc,
never an edit of the frozen v2.

## D2-X3 — the door cannot express origin-freshness

Verbatim-class, from the 23-07 ZT ruling ("yes. no git"):

> A door-site CHECK reads the change — before/after docs, edits, properties,
> actor, force — plus ONE hop of declared pinned evidence at pinned revs,
> fail-closed on drift. NO git facts, no clock, no search at the door — git and
> freshness questions belong to `status` and the sweep. Whole-vault sight
> belongs to the sweep site (`mrd check` / VIEW).

Consequence for colors: **origin-freshness is structurally inexpressible by a
convention at the write door** — a door-site CHECK has no git, no clock. It is a
read-plane fact, rendered only by `status` and the sweep. The door decides
representability (the convention-severity axis); the read plane decides freshness
(the anchor axis). The two axes never collapse into one.

## § Colors — the color law

A color is per edge, per query run — computed, never stored (design-2 §2.3). One
phrasing, used by every surface.

- **green** — `live_rev(to) = pinned_rev`. Node rollup = worst-of-own-edges.
- **red** — resolution fails (named reason, fail-closed) OR
  `live_rev(to) ≠ pinned_rev`.
- **grey** — the ledger cannot verify. The grey class is one of EXACTLY FOUR
  named classes (D2-F4 — the exhaustive enumeration). Grey never refuses; grey
  inputs never block a write (`wire-contract-v2-refusal-amendment.md`
  § Non-refusing renders).

| grey class | when | never renders |
|---|---|---|
| `declared-unpinned` | an `inputs:` ref is declared but `pinned_rev` is NULL — never pinned | red / green — the first pin is how grey turns green |
| `unmanaged` | the target sits outside the ledger's sight | red / green |
| `superseded-algo` | pinned under a `hash_algo` this engine does not compute — readable, unverifiable here | red / green-with-tag |
| `immutable-root` | a `session-id#seq-N` transcript hop — recognized, not verified; the address class cannot drift by construction (§2.2) | red / green |

These four are exhaustive: a grey edge is exactly one of them, named. Greys that
belong to OTHER axes — the genesis-epoch write (enforcement axis), the
between-runs directory shape (fs-frontier axis), the `test --history` class-C
(history-fidelity axis) — are NOT pin colors and are governed by their own units,
not by this enumeration.

## The anchor axis — three states (two-badge freshness)

The freshness axis is two-sided (design-2 §2.3; anchor law amended at v3
`18d3d86b9858a7d1`): tip equality is mechanical and local; the ANCHOR — local
knowledge of origin's refs — carries its own trust state. The qualifier is
mandatory unless verified.

| anchor state | renders | how it is earned |
|---|---|---|
| `verified` | bare `at-tip` / `behind` | the run itself performed the origin observation — only `realise` executing a fetch observe-class claim (net cap, customer-triggered); a moment, never a stored state |
| `as-known` | `at-tip (anchor as-known, observed <now>, ~<age>)`; AGELESS when the fetch was out-of-engine | the last origin observation is a journaled fetch-claim receipt; a restore replays it only with visibly growing age |
| `unverified` | `at-tip (anchor unverified)` | never fetched, or anchor facts absent |

`status` is cap-free and therefore NEVER renders bare `at-tip` — it cannot fetch,
so it never claims `verified`. Bare `at-tip` without a qualifier ⇔ verified in
the same run, by construction (the restore-replay renders its own epistemic edge,
never a false green).

## The composed legend — three axes on one surface

Three orthogonal axes render together on every shared surface — the `mrd status`
line and the board. They never collapse into one color; each item carries its own
value on each axis, and each axis rolls up worst-of INDEPENDENTLY.

| axis | values | question it answers | who decides | renders on |
|---|---|---|---|---|
| pin color | `green` · `red` · `grey`{`declared-unpinned`, `unmanaged`, `superseded-algo`, `immutable-root`} | what drifted / what lies — validity and freshness of my inputs | the ledger (revs) | status line · board |
| anchor state | `verified` · `as-known` · `unverified` | is origin fresh — tip plus the trust of that knowledge | the read plane (`status` / sweep) — NEVER the door (D2-X3) | status line · board |
| convention severity | `off` · `warn` · `block` | does armed law refuse this change | the door (`gate()`, the armed INDEX row — refusal-amendment §11.1) | status line (violation row) · board |

Composition law: the three axes are independent, and validity is not permission.

- A `green` pin can still be `block`ed by a convention — a fresh input does not
  grant permission to write.
- A `red` pin under an `off` (or `warn`) row renders as a finding, never a
  refusal (refusal-amendment §11.1); only `block` refuses.
- Origin-freshness (`as-known` / `unverified`) is a read-plane qualifier the door
  cannot see (D2-X3) and never gates a write.

Worst-of rolls up WITHIN an axis, never ACROSS axes — the shared surface shows
all three, side by side, one phrasing everywhere.
