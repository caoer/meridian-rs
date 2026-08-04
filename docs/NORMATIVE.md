# Which documents in `docs/` are law

**Measured at `2d466dd7` (`main`).** A membership list is a claim about a
directory at a revision; the directory moves. Re-derive rather than trust this
prose if the tree has advanced — § How to re-derive states the commands.

This file **classifies; it does not ratify.** Listing a document here changes
nothing about its authority. Where a document's status could not be determined
from the tree, it is in § Undetermined and it stays there until somebody with
the authority rules — not until somebody with a spare minute assigns.

## Why this exists

A reader could not tell which documents are places to read. The measured cost:
the law that dissolved one of U27's reported disagreements —
`guard_required`'s four-property teaching contract — lives in
`docs/fingerprint-or-force.md`, which the normative amendment for that same unit
labels *"the unit's design note"*
(`docs/wire-contract-fingerprint-or-force-amendment.md:73`). U27 treated it as
carrying ruled law because the advisor did, and flagged that as a judgement it
made rather than a fact it found. That flag is why this file exists.

## The enumeration, and what it could not have seen

| | count |
|---|---|
| entries under `docs/` at any depth, any type | **39** |
| directories | 6 |
| non-`.md` files | 2 |
| `.md` files | 31 |
| — of those, top level | 19 |
| — of those, under subdirectories | 12 |
| symlinks | 0 |

`6 + 2 + 31 = 39`, which closes against the total. **Every one of the 33 files
appears in a table below**, and the 6 directories are named.

The counts are taken **with this file present**, and this file is classified
below like any other. A membership list that omits itself is a list with a
known hole in it, and the hole is the row a reader is standing on.

**Stated rather than left to be discovered — this enumeration cannot see:**

- **Documents outside `docs/`.** `CLAUDE.md` is always-loaded project context
  and assigns status to four documents (§ The fourth authority); `decisions/`
  and the llm-wiki carry ratification records this file only points at.
- **Amendments in flight on other branches.** At the time of measurement two
  new amendment documents existed untracked on the `u27-keyset` worktree. They
  are not on `main` and are not listed. A membership list is a snapshot.
- **Authority asserted anywhere but the file's own opening.** Classification
  below reads each document's self-declaration; a ruling that lives only in a
  session transcript is invisible here by construction.

## NORMATIVE — the document is law for its surface

Each declares this in its own opening. The quoted phrase is the declaration.

| document | declares |
|---|---|
| `wire-contract-v2.md` | frontmatter `type: contract` · `status: frozen` · `version: 2`. **Frozen; never edited.** Amendments carry every change. |
| `wire-contract-v2-colors-amendment.md` | `:2` "Status: normative amendment … this file is the sole normative text for the color law." |
| `wire-contract-v2-effects-amendment.md` | `:3` "Status: normative additive amendment to `docs/wire-contract-v2.md` on the notification plane." |
| `wire-contract-v2-passenger-registry-amendment.md` | `:2` "Status: normative amendment … this file is the sole normative text for the lock-item passenger grammar." |
| `wire-contract-v2-refusal-amendment.md` | `:2` "Status: normative amendment … this file is the sole normative text for the block-is-a-feature semantics and for the refusal codes this module mints." |
| `wire-contract-v3-amendment.md` | `:4` "this file is the sole normative text for the v3 rev." |
| `arming-from-zero.md` | `:3` "Status: normative for the floor-convention arming ladder." |
| `meridian-md-schema.md` | `:10` "Status: normative for the `MERIDIAN.md` parse." |

**Two of these are not amendment files.** `arming-from-zero.md` and
`meridian-md-schema.md` are self-declared normative on their own surfaces. A
reader who learned "the amendments are the normative ones" would miss both.

## RULED LAW IN A NON-NORMATIVE HOME

**This category is the point of the file. It is not a tidiness problem and it
must not be collapsed into either neighbour.** A document here carries law a
normative document depends on, while being labelled something else.

### `fingerprint-or-force.md`

- **What it carries.** The `guard_required` refusal's four-property teaching
  contract: *"subject · cause at its grain · partial state · a runnable fix —
  plus the one negative, never an internal mode name"*
  (`docs/fingerprint-or-force.md:60-66`). This is the contract
  `assert_guard_contract` enforces, and the shape U23's own refusal sweep was
  re-authored from.
- **Which normative document points at it.**
  `docs/wire-contract-fingerprint-or-force-amendment.md:73` —
  ``- `docs/fingerprint-or-force.md` — the unit's design note.``
- **The contradiction, stated exactly.** The amendment calls it a design note.
  The document's own first sentence is *"The law, as ruled by ZT on
  2026-08-03"* (`docs/fingerprint-or-force.md:3`). One of the two is wrong about
  what it is, and **neither file uses the word "normative" anywhere** — measured,
  0 occurrences in each.

**No second instance was confirmed.** Two candidates were considered and are in
§ Undetermined instead, because promoting them here would have been the
assignment this card forbids.

## SPEC — states a law, ships no engine code

| document | self-declaration |
|---|---|
| `address-grammar.md` | `:3` "**Status: the law U10, U11 and U12 implement.** This document is spec only; it ships no engine code." |
| `node-rev-merkle-spec.md` | frontmatter `status: spec` |
| `norm-v2-spec.md` | frontmatter `status: spec` |

`address-grammar.md` and `node-rev-merkle-spec.md` are **also in
§ Undetermined** — see there for why a `spec` label is not a settled answer.

## DESIGN NOTE / DESIGN ELEMENT

| document | self-declaration |
|---|---|
| `fingerprint-or-force.md` | so labelled by the amendment; **see category 2 — the label is contested** |
| `preset-session-birth.md` | `:3` "the design element the `preset` crate … are audited against"; also in § Undetermined |
| `gate-byte-landing.md` | `:3` "Status: enforcement doc for the U4.2 `gate()` seam"; also in § Undetermined |

## DESCRIPTIVE — describes what is, binds nothing

| document | self-declaration |
|---|---|
| `run-plane.md` | "This document states the surface **as shipped in S1**" |
| `status.md` | "A snapshot of what is built and verified today. Numbers here are reproducible from the commands shown — prefer running them over trusting this prose." |
| `laws.md` | none — see § Undetermined; it is listed here **only** because it makes no self-declaration, not because descriptive is the answer |
| `NORMATIVE.md` | this file. `Status: descriptive.` It classifies and does not ratify, so it binds nothing; listing a document here changes no authority. If it ever starts *deciding* status rather than *recording* it, that is a defect, not a promotion |

## GENERATED — machine output, not authored prose

`docs/comment-cleanup/` — 12 `.md` files, generated by
`.claude/workflows/comment-cleanup.js` (`docs/comment-cleanup/README.md:3`).
Each mirrors a `.rs` file and holds comment substance moved out of code plus a
reviewer verdict. Not law; not a design note; **regenerated, so an edit here is
lost.**

**Listed one per line, not brace-collapsed.** `src/{decode,lib,…}.md` is shorter
and it is not checkable: a coverage check greps for a filename, and no filename
in this directory is spelled that way. The compressed form reads as coverage
without being verifiable as coverage — which is the whole failure this file
exists to stop, in miniature.

```
comment-cleanup/README.md
comment-cleanup/crates/wire-serve/src/decode.md
comment-cleanup/crates/wire-serve/src/lib.md
comment-cleanup/crates/wire-serve/src/plan.md
comment-cleanup/crates/wire-serve/src/reaction.md
comment-cleanup/crates/wire-serve/src/rev.md
comment-cleanup/crates/wire-serve/src/write.md
comment-cleanup/crates/wire-serve/tests/s10_fp_decorate.md
comment-cleanup/crates/wire-serve/tests/s2fix_promotion.md
comment-cleanup/crates/wire-serve/tests/s2fix_rule_id_journal.md
comment-cleanup/crates/wire-serve/tests/s7_pin.md
comment-cleanup/crates/wire-serve/tests/u5_4_substrate.md
```

## NON-DOCUMENT FILES

`docs/node-rev-merkle-spec.assets/go.mod` and
`docs/node-rev-merkle-spec.assets/worked-example-gen.go` — the generator for
that spec's worked example. Code, not prose; carries no status.

## Directories

`comment-cleanup`, `comment-cleanup/crates`, `comment-cleanup/crates/wire-serve`,
`comment-cleanup/crates/wire-serve/src`, `comment-cleanup/crates/wire-serve/tests`,
`node-rev-merkle-spec.assets`.

## The fourth authority, and it disagrees with the tree

`CLAUDE.md:32-34` is always-loaded project context and assigns status to four
documents:

> Docs: `docs/laws.md` (crate charters), `docs/wire-contract-v2.md` +
> `docs/wire-contract-v3-amendment.md` (the client seam),
> `docs/node-rev-merkle-spec.md` (**rev/hash law**), `docs/status.md` (mrd CLI).

So a de-facto pointer list already exists, it covers **5 of 18** top-level
documents, and it calls `node-rev-merkle-spec.md` *law* while that file's own
frontmatter says `status: spec`. Recorded as a fact, not resolved here.

## Undetermined — escalate, do not assign

**This is the valuable output of this card.** Each entry states what makes it
ambiguous and what would settle it. None is a failure to look.

1. **`laws.md` — normative in practice, declares nothing.** It has no status
   line and no frontmatter. Yet "Law 3" is cited as binding in unit designs, and
   `CLAUDE.md` names it. Its own claim is stronger than a convention — *"breaking
   it is a compile error, not a review comment"* (`docs/laws.md:4`). **Settles
   it:** a ruling on whether an architectural law enforced by dependency edges is
   normative doc-plane text, or a description of enforcement that lives in
   `Cargo.toml`.

2. **`wire-contract-fingerprint-or-force-amendment.md` — an amendment that never
   claims to be normative.** Measured: **0 occurrences of "normative"** in the
   file. The other five amendments each declare themselves "the sole normative
   text" for their surface; this one says only *"Amends the wire contract's
   write plane"* (`:3`). It is treated as normative by naming convention and by
   carrying ZT's verbatim ratification act. **This is the amendment for the very
   unit whose design note is category 2**, so the ambiguity sits on both halves
   of the same surface. **Settles it:** the author adding the declaration
   sentence the other five carry — or a ruling that the ratification act is
   itself sufficient and the sentence is decoration.

3. **`address-grammar.md` — "the law" and "spec only" in one sentence.**
   `:3` reads *"**Status: the law U10, U11 and U12 implement.** This document is
   spec only."* It then says *"Where it rules, the implementer has no design
   decision left"* — normative force — while disclaiming that it ships code,
   which no document does. **Settles it:** whether "spec only" scopes authority
   or merely scopes deliverables.

4. **`node-rev-merkle-spec.md` — `status: spec` in the file, "law" in
   `CLAUDE.md`.** Two authorities, two words, and the file is the definition of
   `node_rev` and `root` that every crate hashes against. **Settles it:** one of
   the two changing, deliberately.

5. **`preset-session-birth.md` — a "design element" that overrides the code.**
   *"Where the code and this element disagree, the element wins and the code is
   rebuilt"* (`:10`). Supremacy over implementation is normative force under
   another name. It may be a second instance of category 2; it is here rather
   than there because promoting it would be the assignment this card forbids.
   **Settles it:** a ruling on whether a design element that wins against code is
   normative for its plane.

6. **`gate-byte-landing.md` — "enforcement doc" that states a law it cites
   elsewhere.** *"This page states a law and describes an instrument"* (`:5`),
   while `Law:` points at the plan and the refusal amendment (`:3`). Whether it
   restates law owned elsewhere or carries any itself is not determinable from
   the file. **Settles it:** the same ruling as 5, most likely.

## The rule for future documents

A document in `docs/` **declares its own status in its opening**, before any
body text, in one of these forms:

- `Status: normative for <surface>.` — it is law. Say what surface, and if it
  amends a frozen document, say *"the sole normative text for X"* as five of the
  six amendments already do.
- `Status: spec for <surface>.` — it states a law owned elsewhere; name the
  owner on the same line.
- `Status: design note.` / `Status: design element.` — not law. **If ruled law
  nonetheless lands in it, that is category 2 and it belongs in this file** with
  the law named and the normative document that points at it.
- `Status: descriptive.` — describes what is; binds nothing.
- Generated output declares its generator, as `comment-cleanup/README.md` does.

**A new document with no status line is a review comment, not a merge blocker —
and it lands in § Undetermined here until it grows one.** That is the cheap
outcome; the expensive one is the reader who cannot tell and guesses.

## How to re-derive

```
cd <repo>
git rev-parse --short HEAD                         # state the revision
/usr/bin/find docs -mindepth 1 | wc -l             # total entries
/usr/bin/find docs -mindepth 1 -type d             # directories
/usr/bin/find docs -type f ! -name '*.md'          # non-md files
/usr/bin/find docs -type f -name '*.md' | wc -l    # md files
grep -c -i normative docs/*.md                     # who claims it
```

The counts must close: `directories + non-md + md == total`. If they do not,
the enumeration missed a file type — say which, rather than reporting the
subtotal as coverage.
