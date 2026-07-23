# Wire contract v2 amendment — the lock-item passenger registry

Status: normative amendment to `docs/wire-contract-v2.md` §16 (the above-wire
`pin` / `attest` effects layer) and to the 23-07 ZT-ratified `check:` ruling
(`results/round2/zt-rules-plane-rulings.md`). `docs/wire-contract-v2.md` is
FROZEN and unedited; this file is the sole normative text for the lock-item
passenger grammar. Law: D2-X1; decision #15; ZT amendment 23-07.

## What this amends

The **lock item** — one row of the `^inputs` lock a `pin` writes — has a fixed
engine core and an open passenger set. Contract v2 places `pin` / `attest` above
the wire (§16) and never spells the lock-item grammar; the 23-07 ruling added
`check:` / `check_rev` to that row. Passenger keys were therefore about to
accrete with no single owner. This amendment makes THIS file the owner: one
append-only registry anchor enumerates every passenger, so a new passenger is one
appended row, never a reopening of contract prose (decision #15).

## Why an amendment, not a new negotiated rev

A passenger is a frontmatter key on an in-vault lock, above the wire (§16). It
never rides a frame, so it forces no `hello` negotiation and no rev bump. The v3
precedent bumped the rev because a rename forbade dual-emit — an old client would
misread a new frame; this does not. A consumer that has never heard of a
passenger reads the lock row and ignores the extra key, exactly as before. So
this rides the frozen v2 rev. Per the v3 and refusal-amendment precedent, this
separate amendment doc is its own normative record, not a new §18 waiver row
(which would require editing the frozen v2).

## The lock item — engine core vs passengers

`mrd pin` resolves each declared `inputs:` ref and writes one lock row through the
strict writer. The row's ENGINE CORE is fixed and engine-authored:

`{ref, to, rev, rev_class}` — selector, resolved address (`path#(hpath | ^block-id)`),
composed rev, and rev class.

Every other key on the row is a **passenger**: a key the pinning editor declares
and the engine carries verbatim. Passengers split into two classes, and the split
is normative:

- **engine-ignored** — the engine stores and renders the value but never decides
  on it: it never gates a write and never colors a pin. The append-only registry
  below enumerates these. They are the passengers proper.
- **engine-read** — the engine evaluates the value and may refuse on it. These
  are NOT ordinary passengers; §_engine-read fields_ lists them apart so the
  boundary is never blurred.

## The passenger registry (append-only)

Append-only law, binding on this anchor: a passenger is only ADDED to the
`^passengers` table below — never renamed, never removed (one name per thing,
contract-wide). Growth appends a row; it never reopens the prose above or beside
it. A lock-item key absent from this table is not a passenger: the engine treats
no unknown lock-item key as load-bearing, and a second passenger MUST land its
row here before it ships (D2-X1).

| passenger | writes it | reads it | semantics |
|---|---|---|---|
| `claim` | any editor (declared in `inputs:`); `pin` carries it verbatim | humans; `status` / board render | free-text assertion of WHY the ref is drawn from. Engine-ignored — never gates a write, never colors a pin. Its optional machine twin is the engine-read `check:` field (§ engine-read fields), which is a distinct key, not this passenger. |
| `at:` | `pin`, when it can name the tree it read | `status` (cosmetic-change tag, D3) | optional observation stamping the commit / tree the pinned bytes came from. Engine-ignored, best-effort: `status` norm-compares pinned vs live bytes to tag a red edge `cosmetic`; a missing `at:` degrades to an untagged red, never a wrong verdict. Distinct from the `put{at}` write-slot selector (§4.4) — same spelling, unrelated grammar. |

^passengers

## Engine-read fields (NOT passengers) — `check:` / `check_rev`

`check:` / `check_rev` ride the same lock row, but the engine READS them. They are
recorded here so the passenger boundary is explicit; they are governed by the
23-07 ruling, not by this registry's append-only law.

- **`check:`** — declared beside `claim:` (any editor). An ordinary selector
  naming a starlark `def check_claim(t)` predicate that asserts the claim over the
  pinned content at the pinned revs. In-tree only (dot-dirs sit outside the hash
  domain — existing law).
- **`check_rev`** — recorded by `mrd pin` beside `check:` when it resolves the
  predicate target: the rev at which the predicate text was pinned. The predicate
  is one more pinned edge under the three-question color derivation; editing the
  predicate text renders `red check-drifted`, and re-pin resolves.

Evaluation sites (23-07 ruling, unchanged): `pin` — the predicate runs over the
content being pinned before the splice; a false claim refuses the whole pin
atomically, and `dry` reports `would_refuse` naming the failed assert. `attest` —
inherits pin's evaluation; the refuse predicate gains class (c): a false
`check_claim` refuses (a false claim is not staleness — re-pinning cannot fix it,
so it is refuse-class, never record-class). `check` / sweep — speculative on
`content-drifted` only, annotating `claim-holds` (re-pin candidate) vs
`claim-broken` (genuine re-review); a candidate, never auto-green. Bare
`mrd status` never evaluates a predicate (the <1s budget holds).

The refusal code `check_claim{assert}` and its recovery class live in
`wire-contract-v2-refusal-amendment.md` (row 4). This file owns the passenger and
field GRAMMAR; that file owns the refusal CODE.

## The worked lock item (verbatim, 23-07 ruling)

Declared in frontmatter by any editor:

```yaml
# declared (frontmatter, any editor)
inputs:
  - ref: "B#Anchor-law"
    claim: "freshness law this section builds on"
    check: "A#^claim-anchor"
```

Pinned into the `^inputs` lock by the strict writer, engine core + passengers +
engine-read fields on one row:

```yaml
# pinned (^inputs lock, strict writer)
items:
  - {ref: 'B#Anchor-law', to: 'B#Anchor-law', rev: 'b49f62b1…', rev_class: content,
     claim: 'freshness law this section builds on',
     check: 'A#^claim-anchor', check_rev: '7c01d4e2…'}
```

`ref` / `to` / `rev` / `rev_class` are the engine core; `claim` is a registered
passenger (engine-ignored); `check` / `check_rev` are engine-read fields, not
passengers. An `at:` observation, when `pin` can name the tree it read, rides the
same row as the second registered passenger.
