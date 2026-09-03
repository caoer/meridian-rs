---
type: spec
id: move
status: standing
description: The move door — how `mrd move` renames or moves a page or a directory inside one root and rewrites every reference the resolver says would break, in-process, never on the wire.
owns: [the move door]
---

# The move door — `mrd move`

> **Standing:** Design law is `wire-contract.md` (one contract). Mint addresses = segments only. Receipts = armed wire facts. DuckDB/`view_path` not agent core. **Doc correct > code correct; docs first.** See `README.md`.

## §1 What the door is

```text
! mrd move <OLD> <NEW> [--dry] [--immutable PREFIX]... [--json]
```

A tip-level rename of one page or one directory inside ONE root, plus the
rewrite of every reference held by that root which the address owner says would
break. It is the composition `wire §16` names for `mv`: the link plane
(`wire §4.6`) decides *would break*, §4 below decides the new spelling, and one
whole-file replace per referring page lands it. It is a CLI door that runs
in-process (§8) and is never a wire op.

The verb is a page-taking door of the rooted family (`addr §4.6`): both
operands take the `[root:]path` spelling. A rooted OLD selects the root exactly
as a `cd` into it would; a rooted NEW must name the same root, and a different
root refuses `cross_root` with nothing moved. The §1 path law of `addr`
(no absolute path, no `.`/`..`/empty segment) binds both operands at exit 2.

## §2 Operands

- **OLD** is an existing file or directory of the root.
- **NEW** is the destination path. Two forms: a full path (`a/X.md` → `b/Y.md`,
  `a/dir` → `b/dir2`), or the **into-form** — NEW ends in `/`, or names an
  existing directory — which lands OLD under it by its own basename
  (`mrd move a/X.md b/` lands `b/X.md`; `mrd move a/dir b/` lands `b/dir`).
  Missing parent directories of NEW are created.
- A directory OLD moves its whole subtree: every file under it, markdown or
  not, in or out of the hash domain (`wire §12.1`). References are computed for
  the corpus members; the other files move with their directory and are
  counted (`moved_outside_domain`), never rewritten toward.

Refusals, each exit 1 with nothing written: OLD absent (`file_not_found`); NEW
occupied — a file already there, or the into-form landing on an existing path
(`bad_path`); NEW inside OLD, or equal to it (`bad_path`); OLD or NEW under an
immutable prefix (§6, `bad_path`); the ambiguity class (§5); the workspace
write lock held by another writer (`workspace_busy`).

## §3 The rename

The rename is ONE `rename(2)` of OLD to NEW, after every rewrite of §4 has
landed (§9 states the order and why). History is never rewritten and the git
index is never touched — the operator stages the result; git detects the rename
by content. The moved files' bytes do not change, so their blob oids are
unchanged by construction: a lock row's `hash:` and `fingerprint:` stay valid
across a move, and only its `object:` can go stale — which is why §4 rewrites
exactly that field and nothing else.

## §4 What is rewritten, and the form-preserving rule

Four reference classes, all inside the caller's root:

1. **Body wikilinks and embeds** — `[[t]]`, `[[t#Heading]]`, `[[t#^block]]`,
   `[[t|alias]]`, `![[t…]]`: the link nodes the parse yields. Only the bytes
   of the target slot change; fragment and alias are byte-untouched. Inside a
   markdown table cell the alias pipe is written `\|` — that backslash is the
   table's byte, not the address's, so `syntax` keeps it out of the target
   (`laws.md`, the `syntax` charter) and the rewrite lands on the path slot
   before it, leaving the escape and the alias as the author wrote them.
2. **Frontmatter wikilinks** — every `[[…]]` inside the frontmatter block, in a
   scalar value (`owner: "[[USER]]"`), a flow list or a block list. The body
   parse yields no node for these (the ground-truth law keeps frontmatter
   link-free), so the door scans the frontmatter span for the wikilink
   grammar and resolves each hit through the same resolver as class 1.
3. **Frontmatter rooted strings** — a value token `ROOT:path` (the plain form
   `source: "home-wiki:domains/x/PAGE.md"`) whose ROOT names the caller's own
   root (its declared name or its bound alias) and whose path lies under OLD:
   the path part is rewritten, the root prefix kept. A token naming another
   root is that root's fact and is not touched.
4. **`meridian-lock` rows** — a row whose `object:` resolves to a moved page
   is rewritten to the page's new full path (the form `mrd pin` mints);
   `hash:`, `path:`, `fingerprint:` and every extra key stay byte-identical.
   The block is re-rendered by the lock crate's canonical writer, which is
   byte-stable for every engine-written block. Pins made FROM a moved page
   travel with it inside its own bytes; anchors minted IN a moved page are
   untouched. This is the one class an immutable prefix does not freeze (§6).

**The would-break law.** A reference is rewritten iff the address owner's
answer changes: `CorpusIndex::resolve_linkpath` (`wire §4.5` stage 1; the
three-rule owner for a rooted spelling) evaluated against the post-move corpus
— every moved path re-keyed, the referring page at its own post-move path — no
longer names the file it named before. Everything else is byte-untouched: a
bare link whose basename did not change, a partial path none of whose segments
moved, a dangling link (there is nothing to protect), a link into a fence or
inline code (not a link, by the parse's own law).

**The new spelling is minted at the class the author wrote:**

| the author wrote | the door writes |
|---|---|
| the full path (case-insensitively, with or without `.md`) | the new full path |
| a partial path (carries `/`, not the full path) | the shortest suffix of the new path of at least two segments that resolves uniquely — never a longer path than needed |
| a bare name whose basename the move changed | the new basename, when unique after the move; otherwise §5 |
| a bare name whose basename did not change | never rewritten — an answer that flipped is §5, not a retarget |

*Uniquely* means the resolver's candidate set for that spelling, read off the
post-move index, is exactly the moved file — no source-relative tie-break
decides it. A `.md` in the spelling is kept; rewritten segments carry the
on-disk case. A lock `object:` is an engine-minted full path and always takes
the full-path row.

## §5 The ambiguity refusal

Two files of one basename make a bare link a coin the resolver decides by rule
(`pick_source_relative`: the source's own directory, then the shortest path).
This door never lets the move flip that coin silently. It refuses — exit 1,
nothing written — when after the move a bare spelling's candidate set would
hold more than one file AND the move changed that set or that spelling's
answer: a page renamed onto a basename another page carries, a page moved so
that a bare link now lands elsewhere, a bare link whose source moved out of the
directory that decided the pick. Every pair is named — `(source, linkpath,
candidates[])` — under `ambiguous_ref`, in the `--json` frame beside the error
and on the human face. A collision the move neither creates nor moves into is
the corpus's standing state, not this door's finding.

## §6 `--immutable PREFIX` (repeatable)

A file under an immutable prefix keeps every word its author wrote. No body
wikilink or embed, no frontmatter wikilink, no rooted string in it is
rewritten; each one that would break is reported instead — path, line, kind,
the spelling as written, the spelling the door would have minted — and left as
written, so the operator can file it where their own law keeps such things.
OLD or NEW under an immutable prefix refuses: a move into, out of, or across a
frozen tree is a write to it.

**A `meridian-lock` row's `object:` is the one thing the prefix does not
freeze.** A lock row is engine bookkeeping, not authored content: the claim it
carries is `hash:` + `fingerprint:`, both content facts a move leaves intact by
construction (§3), and `object:` is a pointer this engine mints and this engine
owns. So a row naming a moved page is repointed under an immutable prefix
exactly as anywhere else — the path only; `hash:`, `path:`, `fingerprint:` and
every extra key stay byte-identical, and nothing is re-pinned. A frozen page
whose only breakage is a lock row is therefore written, and it is written with
that one slot changed. The alternative was to leave the operator a red
`file-not-found` row inside a tree their own law forbids them to repair —
`mrd check` reading red about a page that moved correctly is not a truth worth
keeping.

The prefix set is the operator's declaration on each run. The engine reads no
workspace list of frozen paths — not `meridian/domain.md` (the hash domain is
a different question), not a wiki contract's layer table — because a door that
froze content the operator did not name would be exercising a judgment the
operator owns.

## §7 Plan and receipt

`--dry` prints the whole plan and writes nothing. The real run prints the same
plan as its receipt, plus the reading back from disk. The plan is: the renames
(one row per corpus member), per-file rewrite counts by class, every immutable
skip, the out-of-domain count, and the link census — links that resolve and
links that dangle, before and after. After a real run the `after` census is
re-read from disk, never copied from the plan; a delta larger than the reported
immutable skips is printed as such and exits 1 — the bytes landed, the plan was
wrong, and the receipt says so.

`--json` is one frame: `{workspace, move: {dry, old, new, kind, renames[],
rewrites[], immutable[], counts{}, links{before{resolved, dangling},
after{resolved, dangling}}}}`. A refusal answers `{workspace, error}` with the
§5 pairs beside it under `move.ambiguous` — the `--json` envelope law of the
face (`status.md` § Teaching rows).

## §8 Why the daemon is not in the path

`wire §16` rules `mv` above-wire: the move is a composed consumer of the link
plane and the write doors, not a fact op — a wire op would have to carry the
emission algebra of §4, which is a corpus convention, not a wire noun. The door
therefore runs in-process, on the same guarded write functions the daemon
serves (`wire_serve::relocate::relocate`, beside `create` and `remove` that the
preset lane calls), under the workspace write flock: cooperating writers
serialize, and a resident daemon reads the changed files back and reports the
rename on its watch plane as a `renamed` change once the origin has left the
disk.

The in-process consequences are stated, not hidden: there is no seq sink, so
no Delta is minted and `changes_seq` does not move (the fingerprint does —
`status.md` § Known gaps); the armed plane does not fire on this door, in the
ambient spelling or the rooted one — which is why the rooted spelling adds no
bypass the ambient spelling lacks, and why `move` is rooted-capable where the
preset lane still refuses (`addr §4.6`): it selects the workspace the way a
`cd` would and has no daemon route to wait for.

## §9 Crash honesty

Order: every referring page is rewritten first — one atomic replace each, tmp
+ fsync + rename — and the rename of OLD is the last act. A crash before the
last act leaves rewritten links naming a NEW that does not exist yet: visible
as dangling, repaired by running the same command again, whose plan finds
nothing left to rewrite and performs the rename. A crash mid-rewrite leaves
some pages rewritten; the re-run rewrites the rest. Multi-file atomicity is
absent, as `wire §6.5` says of every multi-file write, and a rename-first order
would have left the origin gone with no command that converges.

## §10 Jurisdiction and stated limits

- **One root.** References held by other roots — a rooted string in another
  wiki's frontmatter, a cross-root pin — are not enumerated; enumerating them
  costs a corpus parse per bound root. `mrd walk OLD --down` before the move
  lists the pinners across roots; afterwards those roots' own `mrd check`
  reads their stale rows red.
- **Non-markdown targets** (`![[diagram.png]]`) and markdown outside the hash
  domain move with their directory; their references are not computed — the
  link projection is md-only (`wire §12.1`).
- **Markdown-link URLs** (`[t](root:path)`, `addr §9` position 2) and prose
  mentions of a path are not addresses this door reads.
- **Performance.** One corpus parse for the plan, one for the read-back; a
  directory move over a ten-thousand-file corpus is bounded by those two
  parses, never by the reference count.

## §11 Exit triad

0 moved, or a dry plan that would move · 1 refused — every engine refusal, the
engine's verbatim message (`file_not_found`, `bad_path`, `ambiguous_ref`,
`workspace_busy`), and a read-back delta the plan did not predict · 2 bad
invocation — flags, a missing operand, the §1 path law, before anything is read.
