---
type: convention
id: docsys
status: standing
description: How this corpus is structured, addressed, cited and locked. The rules every docs/ file obeys.
owns: [doc-id registry, citation grammar, one-law-one-home, anchor policy, pin policy]
draws_from: []
---

# The doc system

> **This file governs the FORM of `docs/`, never its content.** No law about
> meridian lives here — only the rules that say where a law may live, how it is
> addressed, and how a reader knows it has not gone stale.
> Process and inventory: `README.md`. Wire law: `wire-contract.md`.

## §1 Why form is load-bearing

A law is trustworthy when a reader can find its ONE home and MEASURE that the
home has not drifted. This corpus grew as a history rather than a document, and
two properties were lost on the way.

**Section numbers are not unique across files.** Derive it:

```sh
# every numbered heading, by section number, across the corpus
grep -hE '^#{1,6} +(§ ?)?[0-9]+' docs/*.md
```

Several files answer to a heading numbered `4.4`, and more to `1`. So a bare
`§4.4` names a section only by convention, and a convention cannot be measured.

**A law restated outside its home is invisible.** A citation index finds a law
where it is SPELLED. A paraphrase carries no address, so nothing points at it,
nothing verifies it, and it goes stale silently while reading as authoritative.
The corpus has shipped that failure: a claim was paraphrased into prose outside
the section that spells the law, and the paraphrase outlived the law.

The rules below make both conditions structural rather than a matter of care.

## §2 The doc-id registry

Every file in this directory has a short, stable `id`. The id is declared in the
file's own frontmatter and listed here; the two must agree.

| id | File | Is the home of |
|---|---|---|
| `index` | `README.md` | process, standing corrections, inventory, reading order |
| `docsys` | `doc-system.md` | this document's own rules (§2–§6) |
| `wire` | `wire-contract.md` | the wire constitution — nouns, ops, guards, receipts, errors |
| `laws` | `laws.md` | architecture laws + crate charters |
| `release` | `release.md` | what a release promises; stamp and tag mechanics |
| `addr` | `address-grammar.md` | cross-root addressing, mounts, `addr::Addr` |
| `schema` | `meridian-md-schema.md` | `MERIDIAN.md` config parse |
| `merkle` | `node-rev-merkle-spec.md` | `node_rev` + merkle encoding |
| `fp` | `fingerprint-norm-spec.md` | the fingerprint CID token + norm-v2 |
| `armed` | `armed-plane.md` | the arming ladder + the `gate()` seam |
| `run` | `run-plane.md` | the run plane, preset and session birth |
| `base-projection` | `base-projection.md` | the `.base` projection relations, membership, `base_fold` |
| `body-projection` | `body-projection.md` | the `body` relation, the chunk law, the `body_text` cache protocol |
| `status` | `status.md` | what the binary exposes today (descriptive only) |

A new file claims a new id in the same act that creates it. An id is never
reused and never renamed — citations outlive filenames.

## §3 Citation grammar

**A citation names its document.** The form is `<id> §N`:

- `wire §4.4` — the splice law.
- `merkle §5` — the span fixture.
- `fp §2.1` — the norm-v2 step.

**Bare `§N` is deprecated for new writing and reads as `wire §N`.** That reading
is not a preference; it is what every existing bare citation in this repo and in
downstream clients already means. Declaring it is the compatible move: the tokens
resting on the wire contract keep resolving, and nothing outside this corpus has
to change. Never "fix" an existing bare citation by re-pointing it — qualify it
to `wire §N` or leave it.

**Within one file**, a citation to that same file may stay bare — the document is
its own default namespace. Crossing a file boundary requires the id.

⚠️ **A `§` number is not a dewey ordinal.** `mrd read --section` accepts a dewey
ordinal, and the two schemes disagree by one level: this document's `§2` is dewey
`1.2`, because the `#` title is dewey `1`. So `§2` and `1.2` name the same section
by two different systems. Cite the `§` form in prose, always; leave dewey ordinals
to the tool that prints them.

## §4 One law, one home

1. A law is **SPELLED** in exactly one section. That section is its home, and the
   home's document `owns` it in frontmatter.
2. Everywhere else the law is **REFERENCED** by its address (§3) and is not
   restated. A summary that a reader could act on IS a restatement.
3. A pointer table, an index row, or a reading-order line may name a law in a few
   words **only when the same line carries the law's address.** An unaddressed
   summary is the failure of §1 and is a defect in this corpus.
4. When two sections both spell a law, that is not a duplicate to tidy — it is a
   finding. One of them is stale and a reader cannot tell which. Report it; do
   not pick a winner by reading.

## §5 Anchors — the durable form of an address

A section number is an editorial artifact: inserting `§4.4` renumbers everything
after it. An anchor is not.

- A law section carries a `^block-id` slug, minted by `mrd pin`.
- The anchor, not the number, is the address a long-lived citation should use.
- Renumbering therefore stops being a breaking change.
- A citation may carry both: `wire §4.4 (^splice-law)` reads well and resolves
  durably.

Minting an anchor writes to the target's heading line, which is inside that
section's own rev span — so it changes the section's `node_rev`. On a section
that external expectations rest on, coordinate the mint with whoever owns those
expectations, never alone.

## §6 Locked, not merely written

This corpus is a meridian workspace and is attested by the tool it describes.
That is the point: trust is a measured property here, not a claim.

- `mrd resolve docs` — this directory resolves through the repo-root workspace.
  `docs/` deliberately declares **no** nested `MERIDIAN.md` root: a nested root
  would move resolution for every consumer of this repo, and the lock comes from
  pins, not from a root.
- `mrd read <file>` — the section map with a `sec_rev` per section, under the
  read's own fingerprint. This, not `grep`, is how the corpus is surveyed: a
  fixture printed inside a code fence looks exactly like a heading to `grep`, and
  `wire §0.3` prints four of them.
- `mrd pin <page> <target>#<selector>` — the drawing page records that it draws
  from that section AT that section's content fingerprint.
- `mrd check` — every pin's verdict. A law that moved under a drawer turns that
  pin red instead of leaving prose quietly wrong.
- `mrd walk <page> --down` — who draws from this page, and the blast radius. This
  is the instrument for sizing a law edit: it answers from the pin graph, where a
  grep over citations answers from a convention.

A restatement (§4) cannot be pinned, because there is nothing to draw from. So
under this system an unpinned claim about a law elsewhere is **structurally
visible** — which is why §4 is enforceable rather than aspirational.

## §7 Migration status

The corpus is mid-migration to these rules. The rules bind NEW writing
immediately; existing files are converted in dependency order — frontmatter, the
index, qualified citations, anchors, pins, then the restatement audit. Until a
file is converted, its bare citations read per §3 and its unaddressed summaries
are known debt, not licence.
