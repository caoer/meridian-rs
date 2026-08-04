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
- **grey** — the ledger cannot verify. The grey class is one of the named
  classes enumerated below (D2-F4). Grey never refuses; grey inputs never block
  a write (`wire-contract-v2-refusal-amendment.md` § Non-refusing renders).

| grey class | when | never renders |
|---|---|---|
| `declared-unpinned` | an `inputs:` ref is declared but `pinned_rev` is NULL — never pinned. **NO RENDERING SITE in the shipped engine** — R1.3 retired `inputs` as vocabulary AND as storage key, so the trigger condition is unrepresentable and `model::selector::GreyReason` has no `DeclaredUnpinned` variant. Recorded, not retired: the name is ratified and R1.3 is what removed its subject, not this act | red / green — the first pin is how grey turns green |
| `ambiguous` | the pinned selector resolves to MORE than one node, so no single node's rev can answer the compare | red / green |
| `superseded-algo` | pinned under a `hash_algo` this engine does not compute — readable, unverifiable here. **NO RENDERING SITE in the shipped engine — THE SUBJECT MOVED PLANES, it was not dropped.** Under R4 every pin carries a self-describing `fp1.…` token, so the foreign-algo case is answered by the FINGERPRINT plane and spelled `unverifiable-fingerprint`, which names WHICH triple member is unknown. The destination says so in its own words: `GreyReason::UnverifiableFingerprint`'s doc comment calls itself *"The fingerprint plane's `superseded-algo`"*. Recorded, not retired | red / green-with-tag |
| `immutable-root` | a `session-id#seq-N` transcript hop — recognized, not verified; the address class cannot drift by construction (§2.2) | red / green |
| `unverifiable-fingerprint` | a `meridian-lock` pin whose fingerprint token PARSES but whose `version.codec.hashfn` triple names a member this build does not implement. The render names WHICH member is unknown, never the live-looking triple | red / green |
| `malformed-fingerprint` | the pinned value is not a fingerprint token at all — wrong field count, empty field, or an out-of-charset digest | red / green |
| `lock-refused` | the page's `meridian-lock` block itself is unreadable (malformed, unsupported version, or more than one block on the page), so the pins it declares are outside sight. The row carries the refusal reason verbatim, because a corrupt lock must never read as "no pins" | red / green |
| `unmanaged` | the target sits outside the ledger's sight. **NO RENDERING SITE in the shipped engine** (`model::selector::GreyReason` has no `Unmanaged` variant). The clause that said *"this case renders `declared-unpinned` today"* was TRUE WHEN WRITTEN and then went stale: cross-root addressing gave outside-sight its own variants, `Unmounted { root }` and `PathUnseeable { .. }`, and nobody re-ran the line. It is struck here rather than repaired, because `DeclaredUnpinned` no longer exists to render. **`unmanaged`'s open question STAYS OPEN** — it stopped blocking a sweep; it was not answered, and those are not the same | red / green |
| `uncolourable` | a lock row carrying NEITHER a fingerprint NOR a refusal — it names no evidence and reports no failure to read any, so no compare on either plane can answer it. **Fail-closed sentinel**: unreachable from live input under R4, guarded at ONE point — the parser rule carried by `lock::a_pin_row_missing_a_mandatory_field_refuses_at_parse`, one rule with one test on it. **If this class ever renders, the render is ITSELF the finding**, and the rendered line says so in its own words | red / green — it is not a fact about the target |

A grey edge is exactly one of these, named. Three rows above —  `unmanaged`,
`declared-unpinned` and `superseded-algo` — are ratified contract names with NO
rendering site; each row says which decision removed its subject.

**This section states no COUNT of the rendered classes, deliberately.** It has
carried a wrong one twice (see the correction record below), and a count here is
a second, hand-maintained copy of something the code already owns:
`view::walk::color_reason` is an exhaustive match over the colour enum with no
wildcard arm, so a class cannot exist without a label and cannot be added
without breaking the build. A number in this prose can only ever go stale
against it. Greys
that belong to OTHER axes — the genesis-epoch write (enforcement axis), the
between-runs directory shape (fs-frontier axis), the `test --history` class-C
(history-fidelity axis) — are NOT pin colors and are governed by their own units,
not by this enumeration.

**EXTENSION RULE.** The pin-axis grey enumeration is OPEN and is extended by
ratified unit decisions, each recorded in this table when it lands — so a later
reader extends the list here instead of re-litigating a stale exhaustive claim.

### Amendment record — U9c, tier BRONZE, GOVERNING NOT SETTLED

**Challengeable at U25.** This row and the interpretive ruling under it are a
READING, not a fact, and they are recorded as one so U25 can reject them without
having to reconstruct what was decided.

**What landed.** `edge_color`'s two trailing arms — the foreign-algo
short-circuit and the `node_rev`-compare fall-through — collapsed into ONE
explicit fail-closed arm returning the new grey `uncolourable`. Both arms landed
in a single change deliberately: deleting the first alone would have widened the
fall-through before anything decided what the fall-through IS. `DeclaredUnpinned`,
`SupersededAlgo` and `classify_edge` died inside that collapse.

**The class.** A fail-closed sentinel for a lock row carrying neither
fingerprint nor refusal. Unreachable from live input under R4, **guarded at ONE
point** — the parser rule carried by
`lock::a_pin_row_missing_a_mandatory_field_refuses_at_parse`, one rule with one
test on it. That is a single point of failure stated as one, not a claim of
impossibility. **If the class ever renders, the render is itself the finding**,
and the rendered line says so in its own words rather than leaving it here.

**THE INTERPRETIVE STEP, stated so it can be challenged as itself.** Advisor
ruling: **SITE in this section means A REACHABLE RENDERING PATH.** The section
enumerates RENDERED classes, so a variant plus an arm that no input can reach
cannot produce a render and is therefore not a site in the sense this
enumeration counts. On that reading `declared-unpinned` and `superseded-algo`
were already false at the revision this act began, for stated reasons — R1.3
retired the first's trigger plane, R4 moved the second's subject — so the act is
the record catching up to decisions already ratified, which is this tier's
charter. **On the competing reading — that a variant plus an arm IS a site
regardless of reachability — those two rows are open.** That reading was put and
ruled against, on measured evidence; it is recorded here because a ruling whose
alternative is unrecorded cannot be re-examined.

**What this act did NOT decide.** `unmanaged`'s open question stays open. Its
stale clause is struck because its referent was deleted, which is not the same as
answering it.

### Correction record (stage-2, tier BRONZE)

This section previously asserted the enumeration was EXACTLY FOUR classes and
that "these four are exhaustive". That claim was false, and it stayed false
after three ratified decisions shipped:

- **S9's charter** — map ALL four `ContentVerdict` arms, with `Unverifiable` →
  grey and `Malformed` → grey — added `unverifiable-fingerprint` and
  `malformed-fingerprint`.
- **The Advisor's `lock-refused` ruling** — a refused lock projects zero rows,
  so the page must project a visible grey row rather than reading as "no pins" —
  added `lock-refused`.
- `ambiguous` and the `unmanaged` gap are older than stage 2 and are recorded
  here for the first time.

The amendment's own carve-out ("greys that belong to OTHER axes are NOT pin
colors") does not save the old claim: all three new classes ARE pin colors on
the pin axis, squarely inside the enumeration's scope.

The Advisor classified this correction **BRONZE**, on three grounds a later
reader should find rather than reconstruct. (1) The decisions being recorded
were already ratified inside this milestone, so nothing new is decided — the
doc was simply behind. (2) This amendment documents **view-plane vocabulary, not
wire bytes**: `GreyReason` lives in `crates/model/src/selector.rs` and has ZERO
presence in `crates/wire`, so no ratified byte contract moves and the v2
byte-identity exit criterion is untouched. (3) A ratified document asserting a
false exhaustive claim is ALREADY broken — correcting it restores what its
authors meant, and leaving it is the governance failure.

Elevation test: silver or gold is for changes to what a law MEANS or binds. Had
this correction redefined grey semantics — letting grey cover a case the model
calls red, say — it would return to the Advisor. It does not: every row above
is a case the shipped engine already renders grey, and `unmanaged` is carried
forward unchanged.

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
| pin color | `green` · `red` · `grey`{the § Colors enumeration} | what drifted / what lies — validity and freshness of my inputs | the ledger (revs) | status line · board |
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
