---
type: convention
id: docsys
status: standing
description: How this corpus is structured, addressed, cited and locked. The rules every docs/ file obeys.
owns: [doc-id registry, citation grammar, one-law-one-home, anchor policy, pin policy]
draws_from: []
---

# The doc system

> This file governs the **form** of `docs/`, never its content. Process and
> inventory: `README.md`. Wire law: `wire-contract.md`.

## §1 Why form is load-bearing

A law is trustworthy when a reader can find its one home and measure that it
has not drifted.

**Section numbers are not unique across files.** Verify:

```sh
# every numbered heading, by section number, across the corpus
grep -hE '^#{1,6} +(§ ?)?[0-9]+' docs/*.md
```

Several files have a `4.4`; more have a `1`. So a bare `§4.4` resolves only by
convention, and a convention cannot be measured.

**A law restated outside its home is invisible.** A paraphrase has no address,
so nothing verifies it and it goes stale unnoticed.

## §2 Doc-id registry

Every file in this directory declares a short, stable `id` in its
frontmatter; the table below repeats it. The two must agree.

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

A new file claims a new id when created; an id is never reused or renamed, so
citations outlive filenames.

## §3 Citation grammar

A citation names its document: `<id> §N`, as in `wire §4.4`, `merkle §5`,
`fp §2.1`.

- **Bare `§N` is deprecated for new writing and reads as `wire §N`.** Never
  re-point an existing one; qualify it to `wire §N` or leave it.
- **Within one file**, a citation to that same file may stay bare; crossing a
  file boundary needs the id.
- ⚠️ **A `§` number is not a dewey ordinal.** `mrd read --section` takes
  dewey ordinals, and the two schemes differ by one level: the `#` title is
  dewey `1`, so this document's `§2` is `1.2`. Cite `§` in prose; leave dewey
  to the tool.

## §4 One law, one home

1. A law is **spelled** in exactly one section, its home; that document `owns`
   it in frontmatter.
2. Elsewhere it is **referenced** by address (§3), never restated; a summary a
   reader could act on is a restatement.
3. A pointer table, index row or reading-order line may name a law in a few
   words **only when the same line carries its address**; an unaddressed
   summary is a defect (§1).
4. Two sections spelling one law are a finding, not a duplicate: one is stale
   and a reader cannot tell which. Report it; pick no winner.

## §5 Anchors

A section number is editorial (inserting `§4.4` renumbers all after it); an
anchor is not.

- A law section carries a `^block-id` slug, minted by `mrd pin`.
- Long-lived citations use the anchor, not the number, so renumbering breaks
  nothing; both may appear: `wire §4.4 (^splice-law)`.
- Minting edits the heading line, inside the section's rev span, so its
  `node_rev` changes; if external expectations rest on the section, coordinate
  the mint with whoever owns them, never alone.

## §6 Locked

This corpus is a meridian workspace, attested by the tool it describes.

- `mrd resolve docs` — resolves through the repo-root workspace. `docs/`
  declares **no** nested `MERIDIAN.md` root: that would move resolution for
  every consumer, and the lock comes from pins.
- `mrd read <file>` — the section map, a `sec_rev` per section, under the
  read's fingerprint. Survey with this, not `grep`: a fixture inside a
  code fence looks like a heading to `grep`, and `wire §0.3` prints four such
  lines.
- `mrd pin <page> <target>#<selector>` — records that the page draws from that
  section, at that section's content fingerprint.
- `mrd check` — every pin's verdict; a law that moved under a drawing page
  turns its pin red.
- `mrd walk <page> --down` — who draws from this page: the blast radius of a
  law edit, from the pin graph.

A restatement (§4) has nothing to pin, so a claim about a law outside its home
is visibly unpinned; that makes §4 enforceable.

## §7 Migration status

These rules bind new writing now. Existing files convert in dependency order:
frontmatter, index, qualified citations, anchors, pins, restatement audit.
Until a file is converted, its bare citations read per §3 and its
unaddressed summaries are known debt, not licence.
