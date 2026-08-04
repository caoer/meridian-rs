# Which documents in `docs/` are law

**These counts change only when a file is added to or removed from `docs/`.**
They are unaffected by edits to this file or to any other — including the commit
that carries this line. Re-run the published commands (§ How to re-derive) to
confirm at any revision.

> [!IMPORTANT] The condition names the SITES it governs, not just the trigger
> **A `docs/`-file count appears in FOUR places in this file, and they must move
> together:**
>
> 1. § The enumeration — the table.
> 2. § How to re-derive — the expected values in the code block, **which are
>    controls**, not decoration.
> 3. § The fourth authority — the *"5 of N top-level documents"* denominator.
> 4. § How to re-derive prose — the MEMBERS scoped/unscoped worked example.
>
> **Measured, 2026-08-04:** the condition above fired exactly as written when the
> landing assembly brought four amendment documents in. § The enumeration was
> re-measured correctly. **Sites 2, 3 and 4 were not** — they still read the
> pre-assembly `39 / 31 / 19 / 33`, and site 2's numbers are the ones this file
> tells readers to two-arm as controls. A reader doing exactly what the document
> asks got four mismatches and no way to tell whether the tree or the document
> was wrong.
>
> **A condition is not self-enforcing.** Stating the trigger protects the author;
> enumerating the sites protects the reader — the same asymmetry § The merge
> obligation already names for membership rows. A property invites the next
> editor to re-derive the scope; a list does not.

**One count has a different condition, and it is the one in this file's own
table.** The MEMBERS row count changes when a row is added to or removed from
§ MEMBERS — which happens when a new self-declared normative document lands
(§ The merge obligation) or a member is retired. **Measured:** adding a row moves
MEMBERS 13 → 14 while the file counts stay at 37. A blanket "unaffected by edits
to this file" would be false for exactly this number.

Provenance, not the load-bearing part: the counts were last measured in the
landing assembly that discharged § The merge obligation; they read `457141ba`
before it.

> [!NOTE] Why this is an invalidation condition and not a date
> This header first named a revision, on the principle that a control must be
> dated. In a **self-describing** file that cannot converge: a file cannot cite
> its own sha, so a self-dated document goes stale on every subsequent commit,
> and each fix requires another commit that re-stales it. **Where a claim cannot
> be dated, state its invalidation condition instead.** The condition above is
> true at every revision, including revisions that do not exist yet — which is
> stronger than a date, not weaker, because it names exactly what would make the
> claim false. A date is a crude proxy for that.
>
> Three legs, and a number with all three needs no maintenance: **the revision**
> (so it cannot go stale silently), **the command** (so it cannot be re-derived
> with a different instrument), and **the invalidation condition** (so it does
> not need re-dating to stay true).

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

## The criterion — ruled, not derived here

**A document is normative if and only if one of these holds:**

1. it is a **frozen base**; or
2. it carries a **self-declaration of normative ownership** — the house pattern
   is *"this file is the sole normative text for X"*; or
3. a **ZT ruling act names it** so.

**Nothing else** — regardless of quality, accuracy, or how often it is cited.

The governing principle is ZT's scope-of-assent doctrine (DX-01, recorded in
`CLAUDE.md`): elaboration under or beside ratified content is not itself
ratified. **A document does not become law by containing true sentences.**

**A load-bearing but unlabelled document is a FINDING, never a silent
promotion.** It has exactly two exits:

- **(a)** it gains its declaration through a recorded act; or
- **(b)** its content is restated into a document that is already a member.

This criterion was ruled by the advisor. It is applied here, not invented here —
which is the same distinction the criterion is about.

## The enumeration, and what it could not have seen

| | count |
|---|---|
| entries under `docs/` at any depth, any type | **43** |
| directories | 6 |
| non-`.md` files | 2 |
| `.md` files | 35 |
| — of those, top level | 23 |
| — of those, under subdirectories | 12 |
| symlinks | 0 |

`6 + 2 + 35 = 43`, which closes against the total. **Every one of the 37 files
appears in a table below**, and the 6 directories are named.

> **Re-measured in the landing assembly, not incremented.** The table above read
> 39 / 31 / 19 / 33 when `docs-normative` was written. The four amendment
> documents § The merge obligation listed as in-flight have now landed here, so
> the top-level `.md` count moves 19 → 23 and every total moves with it. The
> directory, subdirectory-`.md`, non-`.md` and symlink counts are unchanged —
> stated because a reader who sees four numbers move is entitled to know which
> ones did not.

The counts are taken **with this file present**, and this file is classified
below like any other. A membership list that omits itself is a list with a
known hole in it, and the hole is the row a reader is standing on.

**Two file counts are both correct and they differ by one — reconciled here so
the next reader meets the answer rather than the discrepancy.** Counting files
under `docs/` gives **37**; counting the documents this list classifies *other
than itself* gives **36**. The difference is this file. A reviewer re-deriving
the count from `git ls-files docs/` at a revision before this file existed gets
36 and is right; a reader running `find docs -type f` today gets 37 and is also
right. Say which side of the self-inclusion you counted from.

**Stated rather than left to be discovered — this enumeration cannot see:**

- **Documents outside `docs/`.** `CLAUDE.md` is always-loaded project context
  and assigns status to four documents (§ The fourth authority); `decisions/`
  and the llm-wiki carry ratification records this file only points at.
- **Amendments in flight on other branches.** At the time of the original
  measurement, new amendment documents existed untracked on the `u27-keyset`
  worktree, unlisted because they were not on `main`. **That limitation has since
  been discharged, not merely restated:** the landing assembly brought all four
  in and § MEMBERS carries them. A membership list is still a snapshot — the
  point survives the instance, which is why this bullet stays.
- **Authority asserted anywhere but the file's own opening.** Classification
  below reads each document's self-declaration; a ruling that lives only in a
  session transcript is invisible here by construction.

## MEMBERS — normative

Each satisfies criterion 1 or 2. The quoted phrase is the qualifying
declaration; where there is none, the row would not be here.

| document | declares |
|---|---|
| `wire-contract-v2.md` | frontmatter `type: contract` · `status: frozen` · `version: 2`. **Frozen; never edited.** Amendments carry every change. |
| `wire-contract-v2-colors-amendment.md` | `:2` "Status: normative amendment … this file is the sole normative text for the color law." |
| `wire-contract-v2-effects-amendment.md` | `:3` "Status: normative additive amendment to `docs/wire-contract-v2.md` on the notification plane." |
| `wire-contract-v2-passenger-registry-amendment.md` | `:2` "Status: normative amendment … this file is the sole normative text for the lock-item passenger grammar." |
| `wire-contract-v2-refusal-amendment.md` | `:2` "Status: normative amendment … this file is the sole normative text for the block-is-a-feature semantics and for the refusal codes this module mints." |
| `wire-contract-v3-amendment.md` | `:4` "this file is the sole normative text for the v3 rev." |
| `wire-contract-fingerprint-or-force-amendment.md` | **member by criterion 3 — the ruling act.** Decision 18 ordered it into existence and it quotes ZT's ruling verbatim at `:7-13`. Its self-declaration at `:3-5` was added later as clerical restoration; see § The classifier that could not classify itself |
| `arming-from-zero.md` | `:3` "Status: normative for the floor-convention arming ladder." |
| `meridian-md-schema.md` | `:10` "Status: normative for the `MERIDIAN.md` parse." |
| `wire-contract-v2-armed-file-rev-amendment.md` | "Status: normative amendment to `docs/wire-contract-v2.md` §4.4 — the `armed` …". **Landed by the assembly; row owed under § The merge obligation.** |
| `wire-contract-v2-error-extras-amendment.md` | "Status: normative amendment to `docs/wire-contract-v2.md` §8." **Landed by the assembly; row owed under § The merge obligation.** |
| `wire-contract-v2-extract-frames-amendment.md` | "Status: normative amendment to `docs/wire-contract-v2.md` §4.3." **Landed by the assembly; row owed under § The merge obligation.** |
| `wire-contract-v2-cross-root-links-amendment.md` | "Status: normative amendment to `docs/wire-contract-v2.md` §4.6 on the read …". **Landed by the assembly; row owed under § The merge obligation.** |

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

**Under the criterion, `fingerprint-or-force.md` is NOT normative.** It carries
no self-declaration (0 occurrences of the word, measured), it is not a frozen
base, and no ruling act names it. It remains the exemplar of this category, and
its exit is now stateable rather than open:

- **(a)** a recorded act gives it the declaration the five other amendment
  documents carry; or
- **(b)** the four-property contract at `:60-66` is restated into
  `wire-contract-fingerprint-or-force-amendment.md`, which is where a reader
  already goes for this surface.

**Until one of those happens, `assert_guard_contract` enforces a contract whose
only written home is a non-member.** That is the finding, stated plainly.

**No second instance was confirmed.** Candidates are in § Findings, not promoted
here — the criterion classifies them out of membership, which is a different act
from promoting them into category 2.

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

So a de-facto pointer list already exists, it covers **5 of 23** top-level
documents, and it calls `node-rev-merkle-spec.md` *law* while that file's own
frontmatter says `status: spec`.

**`CLAUDE.md` is NOT a membership authority.** It is not a frozen base, its
prose is not a ruling act, and naming a document does not confer normative
ownership on it — criterion 3 requires a ruling act, and a pointer list is
elaboration. So `node-rev-merkle-spec.md` is a **spec**, and the word "law" at
`CLAUDE.md:34` is a **discrepancy on record**, not a competing classification.
Recorded as a fact; it needs no ruling to be resolved, only a wording fix if
anyone wants one.

## Findings — what the criterion classified, and what survives it

The criterion resolves membership for four of the six documents whose status was
previously undetermined. **Resolving membership is not the same as leaving no
finding:** a document can be cleanly non-normative and still be load-bearing,
and that is a finding with the two exits named.

### The classifier that could not classify itself — RESOLVED

`wire-contract-fingerprint-or-force-amendment.md` is the document whose `:73`
classifies `fingerprint-or-force.md` as a design note. It was measured as the
**only one of the six `*-amendment.md` files never to use the word "normative"**
— 0 occurrences, where the other five each declare themselves the sole normative
text. So the classifying document appeared to be unclassified itself.

**It resolved by the criterion's own mechanism, with no exception carved.**
Criterion 3 is satisfied: **decision 18 ordered this document into existence**,
and the ruling act is quoted verbatim in its own text at `:7-13`. The credential
was always real; **what was missing was the label.** That is the criterion's
exit (a) — a recorded act — discharged clerically rather than by re-ratification.

The self-declaration now stands at `:3-5`, with a note saying why it was added
late, so **membership-by-act is written down at grant time** and the next auditor
does not re-run this recursion.

> [!IMPORTANT] The closed half stands on every branch
> The natural reading of this finding is that everything downstream wobbles. **It
> does not.** `fingerprint-or-force.md` has no credential under *any* reading —
> no self-declaration and no ruling act names it — so its status as a non-member
> would be unchanged even if the amendment's own membership had been dirty. The
> recursion cleaned the root; **it never reached the fruit.** The `guard_required`
> conclusion that rests on `:73` stands on every branch of this check.

**Why this was worth the round trip:** a classification scheme that cannot
classify its own classifier fails. This one classified its own classifier, by its
own two-exit rule, without an exception. It surfaced only because six files named
`*-amendment.md` were measured one at a time instead of trusted as six of a kind.

### Findings that survive the criterion

**`address-grammar.md` — a self-declaration that contradicts itself.** `:3`
reads *"**Status: the law U10, U11 and U12 implement.** This document is spec
only."* It declares itself law and disclaims in the same sentence. It is not
merely unlabelled, which the criterion handles; it is **doubly labelled**, which
the criterion does not reach. Exit (a) or (b), and until then it is filed under
SPEC on the strength of "spec only" — the half a reader is more likely to act on.

**`laws.md` — load-bearing, unlabelled.** No status line, no frontmatter, no
ruling act. Under the criterion it is **not normative**, however often Law 3 is
cited as binding. Its own sentence — *"breaking it is a compile error, not a
review comment"* (`:4`) — is precisely the claim the criterion refuses to accept
as self-conferring. Exit (a) or (b). Worth noting exit (b) may already be
satisfied in substance: the laws are enforced by `Cargo.toml` dependency edges,
so the enforcement lives in the tree whether or not the prose is a member.

**`preset-session-birth.md` — supremacy over code, no declaration.** *"Where the
code and this element disagree, the element wins and the code is rebuilt"*
(`:10`). Under the criterion: **not normative**, because winning against code is
not a normative-ownership declaration. It is load-bearing by its own sentence,
so exit (a) or (b) applies.

### Resolved by the criterion — no longer open

- **`fingerprint-or-force.md`** — not normative. Category 2 exemplar; exits
  stated in that section.
- **`node-rev-merkle-spec.md`** — a **spec**. `CLAUDE.md:34` calling it "law" is
  a discrepancy on record, not a competing classification, because `CLAUDE.md`
  is not a membership authority.
- **`gate-byte-landing.md`** — not normative. It states a law whose home it
  cites at `:3`; restating law owned elsewhere is not carrying it, so this is
  not a category-2 case.
- **`arming-from-zero.md`** and **`meridian-md-schema.md`** — **members**, by
  criterion 2, on their own surfaces. Both are in the MEMBERS table.

## The merge obligation — this list does not grow by itself

> **WHEN A BRANCH CARRYING A NEW SELF-DECLARED NORMATIVE DOCUMENT LANDS ON
> `main`, ITS ROW IS OWED HERE. THE MERGE GATE CHECKS THIS FILE.**

The scope declaration above — *a membership list is a snapshot* — is honest and
it is not sufficient. A snapshot that nobody re-takes becomes wrong **at the
moment it matters most**, and the failure is the one this file exists to close:
a reader cannot tell that the place they are reading is incomplete. Declaring
the limit protects the author; the trigger protects the reader.

**In flight at the time of writing — measured across every worktree, not
recalled.** Four new self-declared amendment documents exist off `main` on two
branches. Each is a member the moment its branch lands, and none is in the
MEMBERS table:

| document | branch | self-declares |
|---|---|---|
| `wire-contract-v2-armed-file-rev-amendment.md` | `u27-keyset` | yes |
| `wire-contract-v2-error-extras-amendment.md` | `u27-keyset` | yes |
| `wire-contract-v2-extract-frames-amendment.md` | `u27-keyset` | yes |
| `wire-contract-v2-cross-root-links-amendment.md` | `u21-cross-vault-links` | yes |

**MEMBERS moves 9 → 13**, and the count in § The enumeration moves with it.
(The DISCHARGED note below records the same transition — they must agree.)

> **DISCHARGED — the obligation fired and this is the record of it.** The
> landing assembly merged `u27-keyset` and `u21-cross-vault-links` (the latter
> inside `s1-trigger-fix`, which contains it entire). All four documents above
> are now in `docs/` and all four carry rows in § MEMBERS. The MEMBERS row count
> moves 9 → 13 and § The enumeration was RE-MEASURED, not incremented.
>
> This clause is what made that happen. `docs-normative` was written on a branch
> that could not see the merge, named the four documents it could not yet
> classify, and stated the trigger — so the obligation was discharged by reading
> this file rather than by anyone remembering. Keep the clause: the next landing
> needs it as much as this one did.

**A modification to an existing member creates NO row obligation** — only a new
self-declared document does. `u9a-gamma-repair` and `u9b-migration` both edit
`wire-contract-v2-passenger-registry-amendment.md` and
`wire-contract-v3-amendment.md`, which are already members. A sweep of the same
kind that produced the table above returns **eight** files in flight, and four of
those eight are edits to rows that already exist here. Written down because the
next auditor runs the same sweep, sees eight, and would otherwise re-derive this
distinction — which is the cost this file exists to remove.
Named by file so the next reader meets the specific instances and not only the
rule — a rule with no instance attached is the thing everyone agrees with and
nobody executes.

Two other branches (`u9a-gamma-repair`, `u9b-migration`, `u21-cross-vault-links`)
modify `laws.md` and `address-grammar.md`, both of which are § Findings entries.
**Measured: none of those diffs touches a `Status:` or `normative` line**, so
neither finding is resolved or changed by work in flight. Stated because "a
branch is editing that file" is otherwise the obvious reason to assume a finding
has already been handled.

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

**The publisher owes the command, not only the number.** A number without its
revision goes stale silently (§ header). A number without its command makes the
next reader build an instrument that is not this one, and then compare two
different measurements as though they were one. Both halves, or the number is
not really published.

That is not hypothetical: a reviewer re-counting the MEMBERS rows from intent
rather than from this command got **28**, because the rebuilt query was unscoped
over a file with many tables. The scoped count is 13. Nothing had changed but the
instrument.

Expected values are given so each line is its own control, and they carry their
own invalidation conditions: **the file counts move only on a file added to or
removed from `docs/`; the MEMBERS count moves only on a row added to or removed
from § MEMBERS.** Neither is invalidated by the commit you are reading. Last
measured at `457141ba` — provenance, not the load-bearing part.

```sh
git rev-parse --short HEAD                          # state the revision

/usr/bin/find docs -mindepth 1 | wc -l              # 43  total entries
/usr/bin/find docs -mindepth 1 -type d | wc -l      #  6  directories
/usr/bin/find docs -type f ! -name '*.md' | wc -l   #  2  non-md files
/usr/bin/find docs -type f -name '*.md' | wc -l     # 35  md files
/usr/bin/find docs -maxdepth 1 -type f -name '*.md' | wc -l   # 23  md, top level
/usr/bin/find docs -type f | wc -l                  # 37  files (36 + this list)

# MEMBERS rows — SECTION-SCOPED. An unscoped grep over this file counts every
# table row in it and returns 28.
awk '/^## MEMBERS/,/^## RULED LAW/' docs/NORMATIVE.md | grep -c '^| `'   # 13

grep -c -i normative docs/*.md                      # who claims it
```

**`/usr/bin/find` is deliberate, not pedantry.** The interactive `find` here is a
shell wrapper that injects excludes; a search whose target is on its ignore list
returns empty output **with exit 0** — a silent false negative indistinguishable
from "not found". Absolute path, always.

**Single quotes around the MEMBERS pattern are load-bearing.** The trailing
backtick inside double quotes opens a command substitution. Measured: the
double-quoted form emits `zsh:1: unmatched "` on **stderr and exits 0**, so a
scripted loop with `2>/dev/null` prints a clean zero and reports the table empty.
The remedy and the defect emit the same bytes — which is the failure this whole
file is a response to, reproduced inside its own instructions if the quoting is
wrong.

**Two-arm any line here before trusting it,** including these:

```sh
awk '/^## MEMBERS/,/^## RULED LAW/' docs/NORMATIVE.md | grep -c '^| `'   # 13 — the real file
awk '/^## MEMBERS/,/^## RULED LAW/' docs/laws.md      | grep -c '^| `'   # 0  — a file with no MEMBERS section
```

If both arms return the same number, the instrument is not reading what you
think it is.

The counts must close: `directories + non-md + md == total`. If they do not, the
enumeration missed a file type — say which, rather than reporting the subtotal
as coverage.
