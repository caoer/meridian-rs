---
type: spec
id: merkle
status: standing
updated: 2026-08-16
description: Normative hash law for `node_rev` and the workspace merkle fingerprint (two law versions, radix-256 from the cutover), plus the resident tree that serves it, with worked examples.
owns: [node_rev, merkle encoding, resident tree, event feed]
---

# node_rev + workspace fingerprint (merkle) spec

> **Standing:** Design law is `wire-contract.md` (one contract). Mint addresses = segments only. Receipts = armed wire facts. DuckDB/`view_path` not agent core. **Doc correct > code correct; docs first.** See `README.md`.

**Scope note:** this document is **node_rev and workspace merkle (fingerprint) hash law**, plus the **resident tree** — the engine-held instrument that serves that law (§6: structure, stable-read protocol, stamps, event feed, checkpoint). It does not define section address grammar; mint-plane hpath remains segment form only. Worked-example generator assets stay under `node-rev-merkle-spec.assets/`.

**Ruling basis (2026-08-15):** the sections marked RULED below cite the fingerprint-grain decision files (`decisions/2026-08-15-*.md`, session `15-14-fingerprint-grain`) and the merged plan of record (`results/pins-deliverable/merged-plan.md`, same session). Every such ruling is PROVISIONAL per ZT's standing reclassification ("anything I agreed on this session is not rule … any agent can discuss with me") — build against it, challenge it with evidence, directly with him.

The hash scheme behind the wire nouns `node_rev` and **`fingerprint`** (workspace content hash — design noun **`fingerprint`**; the wire's default v2 vocabulary spells the field `root`, re-keyed to `fingerprint` once a client negotiates `contract:"v3"` — `wire-contract.md` §1). Binds: what bytes are hashed, tree composition to the 32-byte workspace fingerprint, incremental update on `splice`, and the resident tree with its feed. Integrity surface on the wire is **`fingerprint` + `if_fingerprint` + `diff`** — there is no separate `guard` op (`wire-contract.md` §4.7); the scoped-premise surface (`scope`, `guards[]`, `scope_bytes`, `absent`) is the wire-law card's to spell, against §7's grain ladder. Under the three laws: no snapshot files (the §6.5 checkpoint is a disposable index, not a snapshot — its own section states why), no second database, Rust memory disposable.

## 0. Design inheritance — the merkle-root-spike, absorbed

An earlier merkle-root prototype was folded into this spec. Its scheme is adopted with one primitive swap (blake3-256 for xxhash64) and its persistence layer dropped; the taken and rejected elements are restated below as standalone decisions.

**Taken:**
- **Injective interior encoding** — interior hash over sorted child entries `varint(len(name)) ‖ name ‖ typeByte ‖ childHash`; the length prefix + type byte prevent sibling-boundary reinterpretation (`{"ab"}` vs `{"a","b…"}`).
- **Names hash into the PARENT** — a rename surfaces as remove+add at the parent, leaves untouched.
- **Root node's own name never hashed** — identical content ⇒ identical root regardless of the top folder's name.
- **Empty dirs pruned, git-style** — nested empty chains are invisible to the root; the tree root itself survives so an empty tree still has a root.
- **File modes don't count** — content + names only.
- **Byte-sorted child names** for deterministic walks.
- **Diff shape** — roots equal ⇒ 1 comparison (the commit fast path); unequal ⇒ descend only unequal branches, naming every drifted path in one pass, remove+add on species change, whole-subtree enumeration on add/remove.
- **Pluggable leaf hasher seam** — kept as a seam, though §3 rejects section-grain leaves for composition.
- **Measured envelope** (M4 Max): session dir 4,141 nodes → root 74ms; whole corpus 9.5GB/50,319 nodes → 2.2s warm / 5.2s cold. These numbers sized §6's original no-persistence stance — superseded 2026-08-15 by the resident tree + disposable checkpoint (§6; the supersession record is at §6's head).

**Rejected (with reasons):**
- **Snapshot persistence** (a `Save`/`Load` binary format, requiring the snapshot to live OUTSIDE the hashed dir). That design served an offline-subscriber demo. In meridian-rs it violates law 2 (Rust memory is disposable; disk = markdown only — "no snapshot files") — and the recovery numbers make it unnecessary: cold rebuild is 2.2s, "daemon death is a blip". The subscriber holds a 32-byte root, never a tree; the daemon holds no trees — the root re-derives on demand (§6–7 re-derive the drift-naming capability the snapshot existed for). *Scoped 2026-08-15 (`decisions/2026-08-15-restart-index-allowed.md`):* the ban this bullet records targets TRUSTED snapshots — objects that can serve stale AS truth. The disposable, checksummed, identity-bound checkpoint of §6.5 is the allowed opposite (it cannot serve stale by construction); markdown stays the sole truth.
- **xxhash64 width.** 64-bit is a race detector, not collision-resistant; the vision's cursor is 32 bytes (law 2) and the wire example is `b3:`-prefixed. §1 rules blake3-256.
- **mtime+size leaf cache** — rejected for v1: its lie window (same mtime+size, different bytes) buys warm-rebuild speed we don't need once the tree is memory-resident and event-updated (§6). Cold start eats the 2–5s honestly. *Superseded in part 2026-08-15:* the resident tree keeps a `StatKey`-keyed memo, and the lie window this bullet feared is closed by law, not by abstinence — the §6.2 watermark + stable-read protocol (a same-instant same-size in-place edit still refuses; merged plan §7's hermetic gate row).

## 1. One hash family: BLAKE3-256

Every hash in this spec — node_rev, file leaf, interior, workspace fingerprint — is BLAKE3 with 256-bit output. One primitive, one implementation, no mixed families.

- Chooses crypto-grade at xxhash-class speed (blake3 saturates memory bandwidth on these sizes), so the integrity cursor is strong for free, and the 32-byte fingerprint matches law 2's "32-byte cursor" verbatim.
- Wire spellings: **`fingerprint`** = `"b3:" + 64 lowercase hex chars` (algorithm- and domain-prefixed per `wire-contract.md` §1 / §12; short `"b3:88d2aa"` forms in old examples are abbreviations, non-normative). `node_rev` = first **16 lowercase hex chars** (64 bits) of the node hash, unprefixed. Both remain opaque to clients — equality only.
- Why truncate node_rev but not fingerprint: node_rev is a CAS race detector scoped to one node's edit history — 64 bits is enough for that job at half the wire noise; the workspace fingerprint is the integrity cursor and keeps full width.

## 2. node_rev — what bytes are hashed

`node_rev = hex(blake3(node_span_bytes))[:16]` where `node_span_bytes = raw_file_bytes[span.start : span.end)` — the node's **span bytes exactly as issued** under the wire span laws (`wire-contract.md` §1 span sub-laws: raw disk bytes, UTF-8-valid files only, leaf block spans exclude the final line terminator).

- For a **section** (heading ref via `resolve`): the span is heading-inclusive (`wire-contract.md` §1 span sub-laws — heading line through end of subtree), so `node_rev` covers the heading too. Consequence, deliberate: a heading rename invalidates the section's CAS token — a writer composing against `#Alpha` must notice `#Alpha` became `#Alpha2`. The content-only span (`content_span`, `wire-contract.md` §1 rev sub-laws) is a write-target convenience and mints no separate rev.
- For **frontmatter**: the whole-block span (`---`…`---` inclusive, `wire-contract.md` §18 row 3 — span-lawed with the section family). Per-key spans mint their own rev at the `fm_key` grain, spelled `prop_rev` — §2.1 is its law. What remains future-only is the delta `keys:[{key, change, value_rev}]` sub-array (`wire-contract.md` §7.4).
- For any other node kind (`toc`/`extract` nodes — `wire-contract.md` §4.1/§4.3): its `wire-contract.md` §1 span, verbatim.
- **No normalization of content.** No newline canonicalization, no trailing-space trim, no NFC. The span law already guarantees disk bytes = string bytes (UTF-8 refusal); hashing anything but the raw bytes would let two "equal" revisions denote different disk states — the exact corruption CAS exists to prevent.

### 2.1 `prop_rev` — the per-key frontmatter CAS token

`prop_rev = hex(blake3(fm_key_grain_span_bytes))[:16]` — same hash family (§1), same 16-hex width, same equality-only opacity as `node_rev`. It is not a second hash scheme; it is §2 applied at a second grain.

**The grain.** `fm_key_grain_span` = the key line, extended over every indented continuation line of a block value. The key name is inside the span, the end excludes the last content line's terminator (§1 leaf law), and a blank line joins the grain only when a later indented line extends past it — trailing blanks belong to the inter-key gap. The scan stops at the next column-0 non-blank line or at the block end.

**Why it exists beside the block-grain `node_rev`.** A frontmatter node's `node_rev` covers the whole `---`…`---` span, so **every key of a document shares one token** — measured over a 6586-document corpus, 6586/6586 multi-key documents. A `splice` that guards a single key with that token therefore refuses `cas_mismatch` whenever any *other* key moved, which on a live corpus is the common case: block grain answers "did this document's frontmatter move", never "did THIS key move". The two revs are additive and neither replaces the other. Reading the wrong one is a diagnosis error, not a race: a guarded miss at key grain is a real drift; a miss at block grain says nothing about the key.

**One owner, three faces.** The token is computed in exactly one place — `model::resolve(doc, Ref::FmKey(key))`, which the write door already compares `if_node_rev` against. Every face **serves** that value and none recomputes it:

| Face | Spelling |
|---|---|
| the write door's guard | `if_node_rev` on an `fm_key` target (`wire-contract.md` §4.7) |
| the composed read | `props[].prop_rev` (`wire-contract.md` § A.3) |
| the corpus projection | the `frontmatter.prop_rev` column (`mrd sql`) |

A face that recomputed the hash would be a second owner of one fact, and the two would drift where they disagree — silently, because both answer 16 plausible hex characters. The projection's `frontmatter.node_rev` column keeps its block-grain meaning unchanged; `prop_rev` is additive next to it.

**Stored form, never decoded.** `prop_rev` hashes source bytes under `wire-contract.md` § A.6.2: a guard token must distinguish `owner: ""` from `owner:`, and the value-plane decode (§ A.6.1) would collapse those two states into one.

## 3. File leaf hash — and why there is no per-file sub-merkle

`leaf(file) = blake3(raw_file_bytes)` — full 32 bytes, whole file, for every path in the **hash domain** (`wire-contract.md` §12: md-only floor + default/custom ignore via `meridian/domain.md`). Non-domain paths never enter the tree.

Section-grain leaves (the pre-wired seam of §0's inherited design) are **rejected for tree composition**: node spans index the raw file bytes, so the whole-file hash already changes iff any node's bytes change — a file-internal merkle adds hashing work and tree surface while buying only sub-file drift-naming, which the parser gives us for free (`toc`/`extract` diff, or `DiffSections`-style rev-table comparison at ~0.2ms/file). Node-grain integrity lives in `node_rev` (CAS, §2); file-and-above integrity lives in the tree (§4). Two grains, one boundary, no overlap.

Files that are **not valid UTF-8** still get leaf hashes (blake3 needs no UTF-8) and participate in the root; they simply serve no spans/nodes (wire `invalid_utf8` law). Integrity coverage and span service are independent properties.

## 4. Tree composition — leaves to the 32-byte workspace fingerprint

Two interior laws exist. **Merkle law 1** (§4.1, the flat encoding) is the
shipped law; it RETIRES at the one-time cutover. **Merkle law 2** (§4.2, the
fixed-256 radix child map) is the law of the first scoped-token version.
Exactly one law is current per workspace at any time — there is no dual-hash
serving window. Rulings: `decisions/2026-08-15-width-sharding-now.md`
(sharding-now, deferral rejected); cutover GO per
`decisions/2026-08-15-plan-rulings-final.md` R1 (priced protocol,
pre-cutover code blockers B-01 / B-02 / B-04 first — B-03 is not a
blocker — pay-once accepted in its `B_cutover` amendment).

### 4.1 Merkle law 1 — the flat interior encoding (retiring at the cutover)

The inherited scheme with blake3-256 in place of xxhash64:

```
interior(dir) = blake3( concat over children sorted by name-bytes:
 varint(len(name)) ‖ name_bytes ‖ type_byte ‖ child_hash_32B )
type_byte: 0x00 = file, 0x01 = dir
```

- Children sorted by raw name bytes (§9 for the unicode caveat). Symlinks skipped (§9). Empty dirs pruned bottom-up (a dir whose children all pruned is itself pruned); the workspace tree root always exists.
- **`name_bytes` are the exact on-disk bytes of the child's name** — on Unix, the `OsStr` bytes verbatim. Never a lossy decode (`to_string_lossy` on the hash path is a spec violation: two distinct non-UTF-8 names that decode to one replacement string would collapse to one leaf, letting content changes leave the fingerprint unmoved), and never a separator rewrite (a `\` inside a name is a name byte, not a path separator). §9 rules this.
- The workspace directory's own name is not hashed (no parent to hold it).
- **`fingerprint` (wire noun)** = the workspace tree's interior hash, spelled `b3:<64hex>` (prefix may advance with domain `version` — `wire-contract.md` §12.3). A file-scope leaf hash is that file's leaf; it is not a second wire “root” op.
- **Why it retires:** re-folding a directory re-encodes ALL its children, so a
  flat 100,000-file folder stays O(100,000) however well cached. The width
  term is a real cost law; the byte-identity deferral ("keep this encoding,
  shard later") was rejected as a priced less-worse
  (`decisions/2026-08-15-width-sharding-now.md`), and the cutover is taken
  while no scoped tokens exist in the wild.

### 4.2 Merkle law 2 — the fixed-256 radix child map (the new hash-law version)

RULED (`decisions/2026-08-15-width-sharding-now.md`): each directory's child
list becomes a canonical radix map with fanout fixed at 256 — one child slot
(the ruling's "bucket") per byte value — so one change re-hashes a bounded
number of vertices, never every sibling. The requirement spine is the codex
design §2.1–2.2 (a bounded-fanout authenticated map, canonical: history never
affects the result), instantiated under the ruled fixed-256 shape. The cost
law lives in the data structure, not in a benchmark: updating one entry
touches the vertices on that entry's key path (bounded by the name's byte
length) plus one directory node per filesystem ancestor — sibling count
appears nowhere.

**Carried over from law 1 unchanged:** the hash family (§1, blake3-256); the
leaf law (§3, `leaf(file) = blake3(raw_file_bytes)` — deliberately untagged,
so a leaf stays the plain blake3 of the file: externally checkable with any
b3 tool, and reusable across the cutover from the `StatKey` memo without
re-reading content; cross-kind confusion is prevented structurally by the
kind byte beside every hash, as in law 1); raw name bytes, byte-order sort,
zero normalization (§9); symlinks skipped (§9); empty directories pruned
bottom-up; the workspace root's own name never hashed; file modes never
hashed.

**Definitions.** A directory's child set `C` is a set of entries
`(name, kind, hash)`: `name` = the child's exact on-disk name bytes (§9),
`kind` = file or dir, `hash` = the child's 32-byte value (file → its §3 leaf;
dir → that directory's §4.2.3 value). Names are unique within `C`; one name
reaching the map as BOTH kinds is the collision case, representable and lawed
in §4.4. Every varint in this law is unsigned LEB128 in **minimal-length
form** — a non-minimal varint is not a legal encoding (canonicality). Byte
order comparisons are on unsigned byte values.

#### 4.2.1 The canonical radix trie over `C`

The child map is the radix trie over the key set `{name}` produced by this
recursion, and no other shape is legal for a given `C` — the shape is a pure
function of the current entry set, so insertion and deletion history never
affect the result:

```
build(S, pos):                      # S = entries; pos = name bytes consumed
  ext      = longest common prefix of { name[pos..] : entries in S }
  pos'     = pos + len(ext)
  terminal = the entry whose len(name) == pos'
             # at most one NAME; a file+dir collision at that name is one
             # terminal with two values (§4.4)
  groups   = partition of the remaining entries by the byte name[pos']
  children = { b → build(groups[b], pos' + 1) : each byte value b present }
  return vertex(ext, terminal, children)

child map of C = build(C, 0)        # the root vertex
```

Invariants the recursion forces (an encoder that emits anything else is out
of law): a vertex with no terminal has ≥ 2 children — otherwise `ext` was not
longest; a vertex with no children has a terminal; vertex count ≤ 2·|C| − 1;
fanout ≤ 256.

#### 4.2.2 Vertex hash — bucket layout and the empty-bucket rules

```
vhash(v) = blake3( "mrk2.vtx" ‖ varint(len(ext)) ‖ ext
                   ‖ terminal_frame ‖ children_frame )

terminal_frame — exactly one of three markers:
  0x00                                   no terminal at this vertex
  0x01 ‖ kind_byte ‖ hash_32B            one terminal
                                         (kind_byte: 0x00 file, 0x01 dir)
  0x02 ‖ file_hash_32B ‖ dir_hash_32B    the §4.4 collision terminal —
                                         both kinds, fixed order, no
                                         kind bytes (the order spells them)

children_frame:
  varint(n) ‖ n × ( slot_byte ‖ vhash_32B )   slot bytes strictly ascending
```

Domain tags are the literal 8 ASCII bytes shown, no terminator, no length
prefix — the tag table and its prefix-freedom argument are §4.3.

**Empty-bucket rules.** An unoccupied slot contributes NOTHING — no
placeholder byte, no zero hash; only occupied slots encode, strictly
ascending. A vertex with no terminal and no children is unrepresentable (its
subtree would be empty). On delete, the map re-canonicalizes as if the entry
had never existed: a vertex left with one child and no terminal merges into
that child (the prefix re-extends), and a slot left empty vanishes from its
parent's frame — no tombstone slots, no retained split points.

#### 4.2.3 Directory value and the workspace fingerprint

```
dir(d) = blake3( "mrk2.dir" ‖ vhash(child map of C) )    # C nonempty
dir(workspace root with C = ∅) = blake3( "mrk2.dir" )    # the empty tree
```

- The wrap gives a directory ONE value whatever its trie shape, and
  domain-separates directory values from map vertices (§4.3).
- Non-root empty directories stay pruned and never reach this rule; only the
  workspace root may be empty.
- The **workspace fingerprint** = `dir(workspace root)`. A child directory's
  `dir()` value is the `hash` in its parent's child map; a file-scope value
  is its §3 leaf, unchanged.

#### 4.2.4 The cost law, stated

A one-entry change re-hashes: the entry's key-path vertices (bounded by the
name's byte length; with compression typically 1–3), each vertex pre-image at
most `8 + varint + len(ext) + 65 + varint + 33·256` bytes ≈ 8.5 KiB at full
fanout — bounded by the 256 fanout, never by directory width — then the
`mrk2.dir` wrap, then the same again per filesystem ancestor. Under law 1 the
same change re-encoded every sibling — O(width) bytes, unbounded. The
flat-100k acceptance gate publishes this as operation counts (merged plan
§7(c): per-commit unrelated-member stat/read/hash counters = 0, plus the
flat-100k directory case with its same-vertex-count discipline). Width today,
for calibration: live-corpus maximum directory width 428 (k3 census, merged
plan §4.2) — the cutover is cheap NOW and a cliff later.

#### 4.2.5 Versioning — one law current, typed retirement

- The hash-law version is a dimension of its own, ORTHOGONAL to the
  workspace-domain version (blocker B-02, merged plan §6 step 5): the
  `mrk2.` domain tags bind the law version into every interior value, so law-2
  and any future law-3 values can never collide even over identical
  structure. The wire prefix advance (today `b3:`) is `wire-contract.md`
  §12.3's to spell, amended by the wire-law card of this wave.
- Every TOKEN value changes at the cutover (leaf values survive but re-spell
  under the new version prefix). A held old-law token refuses
  **`fingerprint_version_retired`** with re-mint teaching — never
  `fingerprint_mismatch`, which would lie: the premise did not move, the LAW
  moved. A token from an unknown FUTURE family is a distinct
  unsupported-version refusal. Only a current-family unequal digest is the
  scoped mismatch. Three errors, three facts, never flattened (§7).
- No dual-hash serving window: maintaining two trees to keep old tokens valid
  is permanent waste against requirement 6. The cutover is paid once per
  workspace (`decisions/2026-08-15-plan-rulings-final.md` R1 + its
  `B_cutover` amendment, ZT verbatim: "that's OK. only pay once. not every
  read."); `sub` re-baselines at a labeled epoch boundary, never a silent
  chain break.
- **The cutover boundary, stated as law (R1: GO — the priced protocol;
  amended 2026-08-15, bounce-1 closure; amended 2026-08-16, B-03
  activation retired — see below).** The cutover runs behind the
  pre-cutover code blockers — B-01 cursor identity, B-02 this section's
  second version dimension, B-04 durable cutover authority state — then
  walks `OLD_SERVING → DRAINING → no-return boundary → NEW_BUILDING →
  NEW_COMMITTED → reopen`. The no-return boundary is crossed by ONE
  durable act: making the B-04 cutover record's `NEW_BUILDING` state
  durable. Retirement precedence rides the same boundary: before
  no-return the old law is still serving and answers guards normally —
  nothing refuses `fingerprint_version_retired` yet; once the record is
  durable, old-family tokens refuse exactly that, with re-mint teaching.
  (`decisions/2026-08-15-plan-rulings-final.md` R1; merged plan §6 steps
  5–7.)
- **Amendment 2026-08-16 — the B-03 tombstone never activates.** This
  section previously read: "The downgrade-fence tombstone (B-03's tested
  mechanism) ACTIVATES at the no-return boundary and ONLY there;
  activating it earlier is the no-rollback error class." That sentence is
  SUPERSEDED by ZT's standing law (2026-08-15, verbatim: "B-03 standing
  law: not a cutover blocker. No old-binary users. Mechanism may land if
  already green; no more matrix; never activate for this reason. Leftover
  bin = delete it. Broken tool = bash. Do not hold radix/L2 for B-03.").
  The fence mechanism (`crates/fs/src/fence.rs`) landed dormant —
  `activate` has no production caller — and is never activated on
  downgrade grounds: its threat model is empty (no old-binary users; a
  leftover old binary is deleted, not fenced). The axis that died is
  old-BINARY fencing; the axis that lives is old-TOKEN refusal
  (`fingerprint_version_retired`, above). If a future implementer finds a
  non-downgrade reason the fence must activate, that is a new ZT
  question — no such reason exists today.
- **The shadow-build fallback is NOT BUILT (`B_cutover` answered).** ZT
  accepted the one-time per-workspace pause, so the bounded NON-SERVING
  shadow — whose only purpose was to shrink that pause — stays on paper,
  verdict-passed, and is not implemented: no second tree is built beside
  the serving one, even as preparation. Acceptance MEASURES and RECORDS the
  real pause; it no longer gates on a budget. The bullet above bans a
  dual-hash SERVING window; this one says the non-serving BUILD does not
  exist either — together they leave an implementer no shadow to reach for.
  (`decisions/2026-08-15-plan-rulings-final.md` § Amendment — B_cutover
  answered.)

### 4.3 Domain tags — one table

Every interior hash in law 2 begins with an 8-byte ASCII domain tag. The
three tags are equal length and differ at byte 5, so they are prefix-free and
no tagged pre-image of one kind can parse as another. Content leaves are
deliberately untagged (§4.2's carry-over list states why).

| tag (8 ASCII bytes) | over |
|---|---|
| `mrk2.vtx` | a radix-map vertex (§4.2.2) — hash-law INTERNAL, see below |
| `mrk2.dir` | a directory's child map (§4.2.3) — the scoped directory value |
| `mrk2.fst` | a forest fold (§4.3.1) — a derived match set, never a directory |

**Vertices are hash-law internals.** A caller premise names PATH nodes only —
the workspace root, a folder, a file leaf, or `absent`. Radix vertices and
their slots are hashed, never addressable as a `scope`, and no wire surface
mints or compares a vertex token (merged plan §4.2, consistent with the codex
premise algebra: nothing below a path node holds a scope).

#### 4.3.1 The forest fold — its own domain tag

A set premise ("all files matching `a/*.md`"; "the rows this query actually
scanned") guards a DERIVED match set, not a directory. Its fold carries its
own domain tag, distinct from directory folds, so the two can never collide —
this subsection is the merged plan §4.5's named spec residue, discharged:

```
forest(M) = blake3( "mrk2.fst" ‖ varint(n)
                    ‖ n × ( varint(len(path)) ‖ path_bytes ‖ leaf_32B ) )
```

- `M` = exactly the matching members: workspace-relative paths in raw bytes,
  segments joined by `/` (0x2f — unambiguous: a POSIX name cannot contain
  it), strictly ascending by path bytes; `leaf` = each member's §3 leaf hash;
  `n` = |M|.
- Computed from the resident listings at O(dir width), zero byte I/O. A new
  MATCHING sibling joins the re-expansion and moves the fold — the membership
  hole stays closed. A non-matching sibling never moves it — the false
  conflict dies. Deletes and renames are caught by two-point set comparison
  (entry expansion vs live expansion).
- `n = 0` is legal: `blake3("mrk2.fst" ‖ 0x00)` is the fold of "nothing
  matches" — a mintable premise that guards a match set's CONTINUED
  emptiness.
- The subtree fold (`mrk2.dir` at a folder scope) remains the conservative
  fallback and the explicit directory-premise form. Consistency law (merged
  plan §4.5): every set premise — pattern root, selector root, sql
  provenance — validates against the TREE, the same instrument as every
  other guard; no premise anywhere consults the journal.

### 4.4 Name collisions — lint loud, refuse at address time

RULED shape (merged plan §4.1; Bazel unique-segment precedent). Law 1 ignored
file/dir name collisions both ways, which left some path's bytes OUTSIDE the
fold. Law 2 closes both halves:

- **Both kinds enter the fold.** A name reaching one child map as both a file
  and a directory (however composed — e.g. through the write overlay) is ONE
  key whose terminal carries both values (`0x02`, §4.2.2). No bytes sit
  outside the integrity surface; ancestor folds above a collision stay
  honest.
- **The tree build LINTS the collision loudly** — a named diagnostic on every
  build that sees it, never silence — and serving continues; one odd name
  must not take a workspace down.
- **Collision paths refuse addressing.** The colliding path, and every path
  through it, refuses **`scope_unresolved`** (fix class) at mint and at
  guard: `scope: "x.md"` cannot say WHICH kind it premises, and an ambiguous
  premise is no premise. This is also the stated precondition of the forest
  digest (§4.3.1).
- Integrity-covered but unaddressable is the deliberate posture for
  collision paths — kept here even as §9's non-UTF-8 name posture retires
  behind `scope_bytes` (§7): a byte-exact path arm can say WHICH BYTES, but
  no premise arm can say WHICH KIND, and an ambiguous premise is no premise.

## 5. Worked example (real values — law 1; law-2 shape in §5.1)

Workspace: two entries. `notes.md` = `"# Notes\n\nhello\n"`; `tasks/x.md` (64 bytes) =

```
---\ntitle: demo\n---\n\n# Alpha\n\nbody line one\n\n## Beta\n\nbeta body\n
```

Node spans (wire laws — block spans exclude the final line terminator):

| node | span | node_rev = blake3(span bytes)[:16] |
|---|---|---|
| frontmatter | `[0,20)` (`---\ntitle: demo\n---\n`) | `c93f2c5ca47ac0a0` |
| section `#Alpha` (resolve, heading-inclusive) | `[21,64)` | `3d5903c3604ee3ac` |
| section `#Alpha/Beta` | `[45,64)` | `780d2fb4cf68f60f` |

> **Frontmatter row receipt** (regenerated 2026-08-08 through the deployed
> engine, `mrd 0.0.0 (git fdcf0d2562fa765ea9000b4e8e83bdd49b4c88e3)`). The
> frontmatter node is terminator-INCLUSIVE: a fence-to-fence container,
> span-lawed with the section (newline-inclusive) family, not the leaf-block
> family — `wire-contract.md` §18 row 3 (waived, declared). This row carried
> the pre-waiver `[0,19)` / `22c54c415778475e` values; the engine never
> served them. Regeneration (fixture workspace = this section's two files,
> byte-exact):
>
> ```
> $ printf '%s\n' '{"id":1,"op":"hello","proto":1,"client":"p2-regen/0.1","workspace":"/private/tmp/p2-spec-regen.YfNM"}' \
>                 '{"id":2,"op":"toc","path":"tasks/x.md"}' \
>   | nc -U "$SOCKET"   # the daemon's short hash-keyed sock — `hash(cache_root)` under $XDG_RUNTIME_DIR/mrd (Linux) or ~/.cache/mrd-run (short-sock law, 2026-08-20; was ~/.cache/meridian/registry/daemon.sock)
> {"id":2,"ok":true,"body":{"path":"tasks/x.md","file_rev":"1e56548abcd43053",
>  "root":"b3:807b69c693ad2c65e290422a1123198f22be6161c2caa43d71fab029fa4763cd","nodes":[
>  {"kind":"frontmatter","span":[0,20],"node_rev":"c93f2c5ca47ac0a0","text_prefix_16b":"---\ntitle: demo\n","keys":["title"]},
>  {"kind":"heading","level":1,"hpath":[{"h":"Alpha"}],"span":[21,64],"content_span":[29,64],"node_rev":"3d5903c3604ee3ac","text_prefix_16b":"# Alpha\n\nbody li"},
>  {"kind":"heading","level":2,"hpath":[{"h":"Alpha"},{"h":"Beta"}],"span":[45,64],"content_span":[53,64],"node_rev":"780d2fb4cf68f60f","text_prefix_16b":"## Beta\n\nbeta bo"}]}}
> ```
>
> (Response reflowed for line width only; values verbatim. The two section
> rows and the fingerprint match this section's pinned values, confirming the
> divergence was isolated to the frontmatter row.)

Leaves (blake3 over whole raw file):

```
leaf(tasks/x.md) = 1e56548abcd43053053ef8f06b68c3261a7d29aa2a03aaa80b0a2f204d213d7e
leaf(notes.md) = 96c26935d00a13398c39887a29adeb554d351b6863ec776c31d4a7f7f93f1875
```

Interior `tasks/` — pre-image is one child entry, 38 bytes: `04` (varint len 4) ‖ `x.md` ‖ `00` ‖ leaf:

```
pre-image: 04 78 2e 6d 64 00 1e56…3d7e
interior(tasks/) = f7a2e4b1af9ef2aa9d57abaa4375e6cff8c474c2f6dd788bc6a9d2543f0277fe
```

Workspace root — two entries sorted by name (`notes.md` < `tasks`), 81-byte pre-image `08‖notes.md‖00‖leaf ‖ 05‖tasks‖01‖interior`:

```
fingerprint = b3:807b69c693ad2c65e290422a1123198f22be6161c2caa43d71fab029fa4763cd
```

**Incremental update:** splice Beta's body `beta body\n` → `beta body v2\n`. Recompute exactly one path:

```
node_rev(#Alpha/Beta): 780d2fb4cf68f60f → f34813be3889438e
leaf(tasks/x.md) : 1e56…3d7e → b78aa71202f4273e830ace6c7844b8943a53c04d1bab719586af2c3a307907ef
interior(tasks/) : f7a2…77fe → 234267c9a1b642b751e50dabed092664a0013fce2c1b22738f6279ac99075a4f
fingerprint : b3:807b… → b3:a1f7bb8e46227d0c44df8c993fa1ab066b299d275d01d81e5dd6c40ba665b7c2
leaf(notes.md) : unchanged (96c26935d00a1339…)
```

(All values computed by a reference implementation of this spec, blake3-256, 2026-07-18; the generator is `node-rev-merkle-spec.assets/worked-example-gen.go` — 127 lines of Go, with `node-rev-merkle-spec.assets/go.mod` — and should land as the fixture seed for the rung-3 test suite. The interior/fingerprint values above are LAW 1 values; `node_rev` and leaf values are law-independent and survive the cutover.)

### 5.1 The same workspace under law 2 — exact pre-images, symbolic hashes

The two keys `notes.md` and `tasks` share no first byte (`n` = 0x6e, `t` =
0x74), so the child map is one root vertex fanning to two leaf vertices:

```
v_n  ext="otes.md"  terminal=(file, leaf(notes.md))  children=∅
     pre-image: "mrk2.vtx" ‖ 07 ‖ 6f 74 65 73 2e 6d 64 ‖ 01 ‖ 00 ‖ leaf(notes.md) ‖ 00
v_t  ext="asks"     terminal=(dir, dir(tasks/))      children=∅
     pre-image: "mrk2.vtx" ‖ 04 ‖ 61 73 6b 73 ‖ 01 ‖ 01 ‖ dir(tasks/) ‖ 00
root ext=""  terminal=none  children={ 0x6e → v_n, 0x74 → v_t }
     pre-image: "mrk2.vtx" ‖ 00 ‖ 00 ‖ 02 ‖ 6e ‖ vhash(v_n) ‖ 74 ‖ vhash(v_t)

dir(tasks/)  = blake3("mrk2.dir" ‖ vhash( "mrk2.vtx" ‖ 04 ‖ 78 2e 6d 64 ‖ 01 ‖ 00 ‖ leaf(tasks/x.md) ‖ 00 ))
fingerprint  = blake3("mrk2.dir" ‖ vhash(root))
```

Reading the frames back against §4.2.2: `07 ‖ "otes.md"` is the varint-framed
`ext`; `01 ‖ 00 ‖ hash` is a one-terminal frame of kind file; the trailing
`00` is `children_frame` with n = 0; the root's `00 ‖ 00 ‖ 02 ‖ …` is empty
`ext`, no terminal, then two ascending `(slot, vhash)` pairs. Compression at
scale: 100,000 names sharing the prefix `2026-08-1` collapse that run into
one vertex's `ext`; divergence fans out below it, so the key path stays a few
vertices deep.

Real values (pinned 2026-08-15, card radix-map — this section's byte layout
is the law; these numbers are that card's receipt). Generated by the law-2
arm of `node-rev-merkle-spec.assets/worked-example-gen.go` and re-derived
byte-identically by the engine encoder (`crates/fs/src/radix.rs`; the gate is
`crates/fs/tests/radix_gate.rs::spec_worked_example_law2_byte_identity` —
two implementations, two languages, one encoding):

```
vhash(v_x, tasks/ map) = 2ca0edd90ba490f03108cd25dd5d12ab21ecb51950a904466588ffffda6588e8
dir(tasks/)            = ef0e7e2eca3cacfcc3bf8fded1454d65645a5a20359c770d6e2dea009d285bd2
vhash(v_n)             = 267b393de0d71194cf17376fef3017d11951b31fab29f70c0637730d0859910c
vhash(v_t)             = a7cbe077445b77bc24873902ce9e896e91f5dd9fcce5d224e4180f4b9bd0e7d9
vhash(root)            = de4f14de1fe5206850e917db2e5ea95306b6b7cfc5935da496a4c92c979fb952
fingerprint (law 2)    = d53c447167825d40f442c65b10f5ae2c6176a49e1e2d8237902d7eaa3008319e
```

(Bare 64-hex here on purpose: the wire spelling of a law-2 fingerprint rides
the §12.3 prefix ladder, which is `wire-contract.md`'s to advance — this
section pins values, not wire spellings.) The §5 incremental splice under
law 2 — `beta body\n` → `beta body v2\n` — recomputes exactly the `x.md`
key path:

```
leaf(tasks/x.md)    : 1e56…3d7e → b78a…07ef   (law-independent, as §5)
dir(tasks/)         : ef0e…5bd2 → e4f51f04970d9feb5c680de5534e1824b27d2660577395e5fadcd9d82fb8a967
fingerprint (law 2) : d53c…319e → 6aab1dd1ef89648508430e0ded866c6ad964b1074fc9b624d025f5c27d10fc58
vhash(v_n), leaf(notes.md): unchanged
```

## 6. The resident tree — memory-held, event-fed, checkpointed

> **Supersession record (2026-08-15).** This section previously ruled
> derive-on-demand: no tree in memory, a full stat sweep + full fold per
> currency pass, no persistence, cold rebuild on every daemon start. That
> stance paid two flock-held full-corpus reads per guarded write (~1.5 s of
> corpus reading, measured) to keep 32 bytes, and made every world guard
> root-grain. The merged fingerprint-grain plan replaces it
> (`results/pins-deliverable/merged-plan.md` §4.1/§4.3/§4.8/§4.9, session
> `15-14-fingerprint-grain`, citing the rulings named below). The old text is
> in git history at `504bcce80`.

### 6.1 The resident structure and the own-write overlay

The engine holds the merkle tree RESIDENT, evolved from `fs::DomainCache` —
the cache that already holds the two hard halves: per-file leaf digests keyed
by `StatKey` (device, inode, size, mtime, ctime) and per-directory listings —
never a second subsystem beside it. Per node it keeps: the §4.2 child map,
the cached 32-byte fold, a dirty bit, and a `last_seq` stamp (§6.3).

- **Own writes update the tree synchronously.** A commit knows the exact
  bytes it wrote: it replaces those leaves and re-folds the ancestor chain
  (measured: one leaf + 13-level refold in 13.6–20.6 µs, median 14.0 µs —
  merged plan lane D). `root_after` / `fingerprint_after` derives from this
  overlay, NEVER from a second corpus read — the overlay is MORE correct than
  a re-read, because a foreign write racing the commit never silently enters
  the folded baseline (`DomainLeaves::overlay`'s own doc law). The overlay's
  own bytes include the receipt append the engine composed (never a
  post-apply reload). A domain-config write applies
  `DomainCache::overlay_membership`: the new `Domain` is parsed from the
  commit's own config bytes and imposed on the overlay's current leaves —
  departed members drop, version and ignore rules update, no disk walk, no
  newly admitted member is read. A remove calls `overlay_remove` then
  `overlay_root`. There is no config/remove exception that re-observes.
  Both exclusion-held `ambient_root` corpus reads leave the write path. The
  splice response keeps its `wire-contract.md` §4.4 transition fields.
- **Write doors ride the same `DomainCache` the feed patches.** The daemon
  passes `Registry::domain_cache` into every write door (an argument, not a
  process-wide hook) so a splice and a currency pass lock one address — and
  the door-entry observation is the registry's, made inside the door's
  flock on that same memo: the §6.4 cookie barrier first, take-and-apply
  second, and the overlay serves as `root_before` only on `Seen` + no
  doubt collapse + `Trusted` — the same vouch `currency_refresh` demands.
  A drained dirty set alone is never a completeness proof: `Trusted` says
  the last observation landed whole and no loss is unabsorbed, not that
  the stream has delivered everything disk holds (without the cookie, a
  silent-dead watcher and a sticky failed feed are indistinguishable from
  a quiet corpus). ANY miss — no live feed, cookie `Unproven`/`Refused`,
  a doubt collapse, an untrusted memo — degrades to the full observation
  that absorbs the loss (§6.2 row 6). In-process callers with no registry
  keep a process-local fallback map and still live-observe every door
  entry (no feed covers their gap). The watcher already advances and notes
  loss on that cache's `FeedGen` cell, so a mid-read fence can fire.
- **Foreign changes arrive through the feed (§6.4)** and mark the touched
  nodes dirty; folds recompute lazily on demand — maintenance cost follows
  change, never corpus size (requirement 1).
- **Memory:** ~10 MB at today's corpus (ESTIMATED, never measured — flagged);
  full fold from already-known leaves 12.1 ms (measured, lane C);
  build-from-scratch stays the 1.45 s cold baseline (§6.5 makes it rare).

### 6.2 The watermark trust close — the full stable-read protocol

`StatKey` alone can miss a same-instant, same-size, in-place write. The close
is git's racy-clean rule, adopted as LAW and shipped as a full protocol, not
a sentence (fable F-11, adopted by merged plan §4.1):

1. The leaf memo carries an **observation watermark**; any leaf whose mtime
   is at or after the watermark is re-read before its digest is trusted — or
   its memo entry is deliberately spoiled.
2. A **per-backend timestamp-granularity calibration probe** runs at
   workspace open: the watermark's comparison unit is measured, never
   assumed.
3. Reads **open without following links**; an identity check (fstat) runs
   BEFORE and AFTER the byte read — identity moved mid-read ⇒ discard and
   re-read. *(Anchored, 2026-08-22, card mac-devhost-snapshot-canonicalization:
   the no-follow walk is a directory-fd `openat(O_NOFOLLOW)` per component
   **from the workspace root down** — every component BELOW the root refuses
   a symlink; the root itself opens as the caller named it, symlinks in its
   own prefix followed. The root is the trust anchor, not a member: every
   production root is canonical at bind (`workspace::canonicalize`), so the
   prefix-follow is a no-op there, while a fixture root under macOS's
   `/var` → `/private/var` `$TMPDIR` stops refusing its own tree. Before
   this the walk began at `/` and a default-`$TMPDIR` mac was red across the
   workspace.)*
4. An **event-generation fence** brackets the read: a feed event landing
   during the read re-classifies it rather than letting a torn observation
   into the memo.
5. A **still-open in-place writer** is classified SUSPECT until its identity
   settles.
6. **Unknown capability or event loss puts guard currency in a LOUD untrusted
   state** — never silent trust.

With this clause the resident tree is at least as strict as BOTH of today's
instruments on every path; the acceptance row is merged plan §7's "same-tick
same-size in-place edit still refuses" gate.

### 6.3 Stamps — `last_seq`, instance-bound

Merged plan §4.9, stated as law:

- Each resident node carries `last_seq` = the highest journal seq beneath it,
  maintained by the SAME guarded write path that maintains digests — the hash
  instrument audits the stamp instrument, so the two cannot drift silently
  (the ZFS hole_birth lesson).
- Fast path: "subtree untouched iff the node's `last_seq` ≤ the token's
  seq" — one node read, O(1), legal only while the event stream can vouch
  (§6.4); the §6.2-governed extent refresh is the floor when it cannot.
- **Stamps are instance-bound** (kimi D3, adopted unconditionally): ring seq
  is per-daemon-epoch and rings are idle-reaped, so a stamp compare across a
  reap or restart could FALSE-PASS "untouched". Stamps and stamp-bearing
  tokens carry the tree instance id; instance mismatch degrades to the
  content-fold compare — always available, epoch-free, history-free.
- **Hash tokens are epoch-free; cursors are not.** A content-hash guard token
  survives daemon restart — exactly the demand on disposable memory; cursors
  stay confined to the delta plane. Advisory `{instance, seq}` hints may ride
  premises; the engine answers IDENTICALLY with them absent.
- **Stamps never answer for the dead:** a deleted or renamed-away path has no
  current node to carry its stamp. Delete visibility while the journal lives
  comes from journal frames; past the ring's horizon it is cursor-too-old —
  re-derive, never a stamped guess.

### 6.4 Feeding the tree — the event feed and the rescan ladder

The tree must learn about changes the engine did not make itself. The
event-source question was adjudicated ENGINE-SIDE, five lanes unanimous
(merged plan §4.3):

- **The engine owns its senses.** One kernel file watcher (FSEvents on macOS,
  inotify on Linux) per workspace, owned by the engine process. Strongest
  reasons first: a dead cross-process feed is indistinguishable from a quiet
  corpus; the engine holds workspaces the daemon never tracks (CLI lane,
  ad-hoc repos, CI fixtures); the currency proof must ride the SAME stream
  that feeds the tree; and the daemon (ccc-statusd) is the engine's CUSTOMER
  by charter — the engine's freshness must not depend on its customer's
  uptime.
- **The daemon journal is a legal ADDITIONAL feed** where it already
  watches — an opportunistic dirty-path hint, never the instrument any guard
  or currency answer depends on.
- **Guard correctness consults neither journal nor watcher.** The guard is a
  live fold over the named premise through the watermarked memo (§6.2). The
  journal's three vacuous windows — unsubscribed external edits never
  journaled; idle-reap deletes subscriber-less rings; the RAM-only ring
  resets its seq on restart — are irrelevant to guard honesty by
  construction. No premise anywhere consults the journal (§4.3.1's
  consistency law).
- **Watcher lifecycle** (kimi D1, adopted): the watcher's lifetime is the
  workspace REGISTRATION, not engine warmth. An idle-reaped engine keeps its
  watcher; events accumulate into a dirty set held by the registry; the next
  warm applies the dirty set — O(dirty), never O(corpus).
- **The currency barrier (the cookie).** A guard-grade currency question
  writes a sentinel at `.meridian/cookie` and waits to see it return through
  the ordered event stream — an O(1) proof that everything before it is
  already folded in (watchman's shape). `Seen` proves ordered delivery of
  what the kernel captured, AND that no capture-gap doubt is open. A
  new directory (or any path that is not a member candidate but can hide
  members) is a missed-event: the kernel may not have armed a watch before
  a child landed, so those children can be invisible forever. `Seen` is
  illegal while that doubt is open — the barrier returns unproven
  immediately and does not spend the cookie timeout. The path is LAW, not
  convention: dot-prefixed segments are outside the hash domain by the
  standing `wire-contract.md` §12.1 floor, so the cookie can never move
  the root or break a held token. A cookie inside the hash domain is
  refused by construction (merged plan §7's gate row).
- **The rescan ladder — every cause NAMED, throttled:** kernel event overflow
  → mark-all-dirty → the next pass is the full stat sweep (lane B, 160 ms
  warm, measured); a new directory (or other hideable non-member) →
  mark-all-dirty under missed-event → the same sweep (the child that
  landed before the sub-watch armed is recovered by the walk, not by the
  lost event); watcher instance change → one LABELED re-baseline (1.45 s
  cold / 160 ms warm, measured); the watcher never restarts across a rescan;
  a rebuilt index commits by swap. Self-echo — the engine's own writes
  re-arriving through the watcher — is deduped as a cost saving only: overlay
  idempotence is the correctness, masking is never load-bearing.
- **Idle re-check — RULED: SUSPICIOUS-ONLY, NO TIMER**
  (`decisions/2026-08-15-pre-merge-rulings.md` ruling 3). The engine
  re-checks files only on a named reason for doubt — a missed event
  (including a new directory, or any non-member path, that can hide
  members before its watch arms), watcher overflow, an instance change, a
  failed spot check (vouch failure), a cookie timeout — and may piggyback
  on guard-path touches. Zero background work when healthy (requirement 6,
  "no waste anywhere"). The periodic idle sweep is DECLINED, its price on
  the record: a silent event-stream loss with no named trigger goes
  uncaught until a guard touches its scope.

### 6.5 Restart — the disposable checkpoint

RULED (`decisions/2026-08-15-restart-index-allowed.md`, superseding the
memory-only direction): requirement 2 — "the engine knows, it does not
re-ask" — binds ACROSS ordinary daemon restarts. The engine may hold a
checksummed, DISPOSABLE derived index outside the hash domain (git-index
class): the checkpoint.

- **Identity tuple:** `(workspace_uuid, domain_version, tree_root,
  journal_instance, journal_seq, parse-cache generation)`. ANY field
  mismatching forces exactly one loud, labeled re-baseline. Without
  `domain_version` a checkpoint would survive a hash-law change (the §4.2
  cutover changes the interior encoding) and serve old-law nodes as current;
  without the journal cursor pair, O(changes-while-down) has no replay point.
- **Restart replay** (gate 11 re-cut by gap class —
  `decisions/2026-08-16-gate11-stat-floor.md`, session
  15-14-fingerprint-grain, as amended post-enactment): where a QUALIFYING
  journal covers the whole gap — qualifying means all four conditions hold
  for THIS gap: (1) coverage is complete from gap start to gap end; (2) the
  instrument carries a loss signal and raised none — silence from an
  instrument without loss signaling qualifies nothing; (3) per-file
  granularity was in effect; (4) the instrument's own contract treats a
  clean, loss-signal-free window as definitive — an instrument whose
  documentation directs rescans regardless fails. Then: one journaled
  change = exactly one file read and hashed, zero unchanged members
  statted, no cold rebuild. Today exactly one instrument qualifies: the
  live §6.4 watcher across an engine-cold, process-alive gap with no
  overflow raised. No persisted instrument known qualifies — FSEvents fails
  (2) and (4) by Apple's own guide; btrfs/ZFS fail portability and
  enumeration. A journal qualifies by meeting the standard, never by being
  called a journal.
- **Where no qualifying journal covers the gap** (today, every process
  death), a sound checkpoint restores every row UNTRUSTED; no restored row
  may serve, and no answer may be derived from one, until exactly one
  §6.2-governed verification pass has completed over the full member set.
  The pass is: one stat per member as the floor — the 160 ms lane-B figure
  at 29.7k measures THIS STAT PASS ALONE — plus the watermark law's
  mandatory re-reads: any row racily clean at save (recorded mtime within
  one calibrated granularity unit of the checkpoint's saved observation
  watermark) is re-read or restored pre-spoiled, NEVER trusted on
  stat-match; §6.2's identity-fence and suspect rules govern those reads.
  Zero-byte cost holds for every row outside the watermark window; the
  watermark-window re-reads are additional and their count is published.
  The pass is a pre-serve BARRIER — lazy, deferred or post-first-serve
  verification is refused by construction. Counter equation: reads = hashes
  = movers + watermark-window re-reads; stats = member count, exactly once,
  all before first serve. **Parses are NOT gated by this law:** the
  checkpoint carries leaf digests and the tree, never parsed documents, so
  these counters and the 160 ms govern the RESIDENT-TREE restore — the
  guard/currency plane. The document plane's restart cost is a named open
  residue awaiting a parse-cache persistence object, whose identity slot
  this tuple's parse-cache generation field already reserves.
- A soundness mismatch = one loud labeled cold re-baseline; a cursor that
  cannot anchor = one labeled warm re-baseline — replay forfeited, object
  retained. The residual stat term is recorded UNFIXED in the decision
  record; requirement 1 is PARTIALLY SATISFIED there, never here.
- **Markdown stays the sole truth, always.** The §0 ban on trusted snapshots
  is UNTOUCHED — that ban targets objects that can serve stale AS truth; this
  object is the opposite (it cannot serve stale by construction). The caution
  rides with the allowance: the distinction is real only while loud-discard
  and identity-binding are ENFORCED — a checkpoint trusted after a mismatch,
  or outliving a domain-version change, is the banned object wearing the
  allowed object's name.
- **Format ordering:** the checkpoint format is downstream of the §4.2
  encoding — never persist what the next step replaces (merged plan §6
  step 10).
- **Storage site:** one file per workspace in that workspace's cache drawer
  (beside `sql.duckdb` and the run plane's digest memo), written atomically
  and read whole. The drawer is outside every hash domain by construction, so
  the checkpoint can never move a root or break a held token — the same floor
  the §6.4 cookie stands on.
- **Two questions, two instruments** (ruled: `results/checkpoint-design-ruling.md`
  § I, session `15-14-fingerprint-grain`). A checkpoint is an observation record
  from a dead epoch — a claim about the past, never about the present — and its
  trust splits in two. **Lawfulness:** may these rows enter as HYPOTHESES?
  Decided once at restore by the soundness fields — `workspace_uuid`,
  `domain_version`, hash law, parse-cache generation, and the `tree_root`
  binding. A mismatch means the rows are not lawful statements about this world
  under today's law, so the object is discarded WHOLE, loudly, labeled per
  field: the **COLD** re-baseline. **Currency:** may a row SERVE? Never
  answered by the checkpoint, and never wholesale — every restored row serves
  only through the §6.2 watermark trust close, the same live protocol that
  governs a RAM-resident row. A restored row has exactly the trust class of a
  resident row whose stat evidence is stale: none, until the live instrument
  confirms it. That is "it cannot serve stale by construction", said
  mechanically.
- **The journal pair is neither — it is a CURSOR**, governed by §6.3's landed
  cursor law ("hash tokens are epoch-free; cursors are not"). A cursor that
  anchors buys REPLAY; a cursor that cannot anchor forfeits replay ONLY, and
  the rows still enter as hypotheses — the **WARM** re-baseline, which
  re-establishes currency from zero TRUST, not from zero BYTES. Trust never
  rides the cursor in either direction. The two flavors carry distinct labels
  on purpose: the cold event is rare and meaningful, while the warm event fires
  on **every** ordinary process restart by design, because §7.1 persists no
  epoch fact. Reading the pair as an identity FIELD collapses the two and makes
  the object discard on every restart — the vacuity this naming exists to
  prevent; the git-index class this section names discards nothing at a gap, it
  re-verifies by stat.

### 6.6 Fingerprint history ring

`diff(from_fingerprint, to_fingerprint)` needs the frames behind old
fingerprints; clients hold only the token. The daemon keeps a bounded
in-memory ring of recent `DeltaFrame`s (`RootRing`, `wire-serve/src/ring.rs`),
which `diff` replays between two roots. A fingerprint not in the ring answers
`fingerprint_unknown` → full resync — re-derive, never wrong data. Under the
resident tree the detect side becomes event-fed (the poll survives as the
fallback clock); frames are unchanged on the wire and the ring keeps its
bound. At the §4.2.5 cutover the ring re-baselines at a labeled epoch
boundary, never a silent chain break.

### 6.7 The serve-path currency consumers — one instrument, vouch first

**Docs-first (2026-08-20).** This section closes a code-lags-doc gap, not a
new direction: §6.1/§6.4 already rule the vouched currency answer ("O(dirty),
zero when quiet — never O(corpus)") and the write door already rides it
(`Registry::door_observation`). What was never stated is WHICH standing
consumers the law binds — and in the unstated gap every one of them kept the
§6.2 extent-refresh floor unconditionally. Measured on the live fleet corpus
(37.8k members, 2026-08-20, mrd 1.0.0 @ d9035428): the daemon burned ~66–85
CPU-seconds per 60 wall-seconds at steady state, nearly all of it floor
passes — the domain walk + one `lstat` per member — on request threads and
timers, while the feed, the cookie, and the overlay sat landed and unused on
the read side.

> **The law.** Every standing currency question in the daemon is answered by
> the workspace's ONE resident memo (§6.1, card run-observation-unification)
> through the §6.4 vouch, at the grade the question needs; the §6.2
> extent-refresh floor answers only a NAMED miss — no live feed, cookie
> `Unproven`/`Refused`, a doubt collapse, an untrusted memo, no baseline. The
> floor never runs on a timer against a healthy feed. A stat signature or any
> other evidence-grade instrument may still gate work that is pure latency
> (G11's standing license), and may never stand in for the content root: what
> a served answer is stamped with is always a root folded from content
> digests — the overlay's own read-and-hashed leaves, or the floor's.

Two grades, both already ruled, now bound to their consumers:

| Consumer | Question | Grade | Instrument |
|---|---|---|---|
| warm read pass — the read family, `sql`, the `script` entry (`Registry::warm_or_build`, cheap half) | is the resident engine current NOW? | current-as-of-the-question | cookie barrier → take-and-apply → `Trusted` → overlay fold (`Registry::currency_refresh`); floor on any named miss |
| § A.11 post-result `live` | did the corpus move past the rows? | current-as-of-the-question | same call, same vouch |
| write door `root_before` | §6.1 door-entry observation | guard | `Registry::door_observation` — unchanged, was already lawful |
| G11 prewarm quiet check | may this sweep be SKIPPED? | latency-only | O(1), no cookie: nothing pending after take-and-apply, memo `Trusted`, cached served fold == the engine's stamp. The `domain_stat_signature` walk survives ONLY where no live feed exists (`FeedSlot::Failed`), under the existing quiet backoff |
| §4.7 detect pre-check (`WorkspaceRing::detect`) | did the root move since baseline? | latency-only + fallback clock | the same O(1) quiet check through the SHARED memo; the private fold memo serves `prime` and the miss path only. §6.6's surviving poll: even under a continuously quiet vouch the floor pre-check still runs once per `DETECT_FLOOR_CADENCE` (30 s) — the push plane has no guard to catch a silent capture loss, so the poll is its bounded backstop |

**Why the read pass carries the cookie.** A read's answer mints ambient
premise tokens (§5.4; wire-contract §2 mint law) — `fingerprint`, revs — that
the caller's next write spends. A vouch without the barrier proves only "the
memo folded everything DELIVERED", and a silent-dead watcher is
indistinguishable from a quiet corpus (§6.1's own sentence). The barrier is
what makes the served stamp current-as-of-the-question — the same grade the
floor gave it, at O(dirty) instead of O(corpus). Latency: the sentinel write
plus delivery, bounded by the door cookie budget; against the floor's
~160 ms-class sweep this is a strict improvement on both axes.

**The cookie holdoff (posture, both doors).** A `CookieTimeout` collapse is
sticky doubt until the next take; the take converts it to a full sweep and
clears it — so under a dead-but-running watcher every OTHER question re-paid
the full timeout. Ruled posture: after a `CookieTimeout`, the barrier answers
`Unproven` immediately for `COOKIE_HOLDOFF` (60 s) — callers take the floor
without stalling, exactly the pre-feed cost — and one probe per holdoff
window re-tests the stream. Self-healing, and the stall a dead watcher can
add is bounded at one timeout per window per workspace.

**Foreign domain-config edits collapse the vouch (correctness, closed in the
same act).** `meridian/domain.md` governs membership and version, and the
overlay fold serves BOTH from the last observation (`domain_seen`). A foreign
edit of the config arriving as an ordinary dirty path would move the config's
own leaf but fold it under the SUPERSEDED membership — a root no true corpus
state ever had. The feed apply therefore escalates a dirty path equal to
`fs::domain::DOMAIN_CONFIG_PATH` to the Sweep rung (§6.4 ladder: memo kept,
loss noted, the next observation is the full walk under the freshly loaded
config). The governed write path is untouched — its own-config commit imposes
`overlay_membership` synchronously (§6.1) and never pays this rung. Config
edits are ruling-class rare; one floor pass each is the honest price.

**The watch plane classifies off the resident memo (the §6.6 direction,
finished — landed as its own unit with frame-parity gates; measured live
before it: one full corpus snapshot — every member's BYTES — per emitted
external batch, ~1/s under fleet writes with a subscriber).** When the
detect pre-check finds the root moved, the cycle takes the workspace flock
and makes the §6.1 door-grade observation through the SHARED memo — the same
cookie-barrier → take-and-apply → overlay observation the write doors make,
the extent-refresh floor on any named miss — then hands the classifier the
memo's leaf set and root. The classifier diffs those digests against the
watcher's retained baseline (each entry now carries its leaf digest beside
its bytes), reads bytes ONLY for movers, and mints the same frames: rename
pairing compares digests (byte-equality's exact proxy), removed and
`unattested` rows parse retained baseline bytes, modified rows diff
retained-old against read-new. A mover whose re-read digest disagrees with
the observed leaf is a mid-cycle race: the cycle emits nothing and holds its
baseline — the racing write's own event re-fires detection and the next
cadence tells the whole truth once. The frame's `root_after` is the memo's
root — the very value the read plane stamps, so the push and read planes
cannot disagree about the current world. Priming (the subscribe-time
baseline) keeps its one full snapshot. The ring's private fold memo is
DELETED — the shared memo is the one instrument on the watch plane too
(card run-observation-unification), a plain mutex keeps the single-flight
gate, and the `DETECT_FLOOR_CADENCE` backstop forces a true floor pass
through that shared memo once per window. Non-UTF-8-NAMED members stay
baseline-invisible exactly as the snapshot kept them (their leaves still
fold; a `wire::Path` cannot spell them) — §52 covers non-UTF-8 CONTENT,
which classifies normally.

**What does NOT change.** The floor pass itself (walk semantics, §6.2 trust
close, refusal shapes) is untouched — every consumer keeps falling to it on
its named misses, and the cold path (no baseline) is the floor by
construction. `Reused` keeps its zero-parse proof; a rebuild is still
`fs::update_corpus` against the memo's leaf set; nothing served is ever
stamped with a root its own fold did not derive from content digests. The
run-plane bracket observations (guard grade, locked-window law) are out of
scope and keep their live floors.

### 6.8 The absorb path — deriving the answer at the cost of the change

**Docs-first (2026-08-21).** Like §6.7 this closes a code-lags-doc gap:
§6.1 already rules the maintenance law ("folds recompute lazily on demand —
maintenance cost follows change, never corpus size") and §4.2 puts the cost
law in the data structure. What was never stated is that the law binds
DERIVING the served answer, not only deciding whether it may serve. In the
unstated gap the served fold rebuilt a FRESH radix tree over the whole leaf
memo on every recompute — §4.2's per-change bound honored inside an
instrument reconstructed at O(corpus) per question — and the incremental
corpus pass deep-cloned every carried document. Measured (37.8k members,
2026-08-21, mrd 1.0.0 @ 149cf428d, 15 s sample under 2 foreign writes/s):
~0.25 CPU-s of pure memory work per absorbed change, ~26.6 CPU-s per 60 s —
attributed to the two flat rebuilds (the overlay fold and the rebuild's
tail fold) and the carried-document clones.

> **The law.** The resident tree IS the serving instrument. The served
> workspace root derives from the ONE memo's incrementally maintained fold
> (§6.1) — never from a second tree rebuilt over the leaf set the memo
> already carries. An incremental corpus pass carries an unmoved member's
> parsed document by shared reference, never by copy. And such a pass folds
> nothing when the leaf set it built is byte-equal to the set its snapshot
> served — its stamp is then the snapshot's own root, taken with the
> snapshot under one lock. On the serve path the flat build over a leaf set
> survives in exactly three roles: the cold observation (no resident state
> yet), the divergence tail of an incremental pass (a mover vanished or
> changed between snapshot and read — the built set differs, and the pass
> folds what it actually built), and the equivalence gate's oracle. The
> run-plane bracket observations stay out of scope with their own
> instrument, exactly as §6.7 left them.

Why this is lawful, stated as the gates that hold it:

- **§4.2.1 purity is the equivalence.** The canonical trie shape is a pure
  function of the current entry set — insertion, deletion, and refold
  history never affect the result — so the incrementally maintained fold
  and a fresh build over the same leaves cannot differ. The resident
  structure's own property gate (any op history vs fresh build, §4.4
  collision keys included) is the standing proof.
- **Lockstep is the invariant, gated at the memo grain.** The leaf memo and
  the resident tree advance in the same guarded act at every mutation
  site — the observation generation, the own-write overlay (leaf, remove,
  membership), the §6.5 restore (which verifies the rebuilt tree against
  the stored `tree-root` before adopting any row). The absorb-path gate
  asserts served fold == flat oracle after each mutation class, so no
  future path can advance one half alone.
- **One instrument, one truth.** The floor pass keeps its §6.2 walk, stat,
  and read semantics unchanged; its FOLD is the same resident fold — the
  §6.7 consumers and the floor can never disagree about the root of one
  memo state (card run-observation-unification, extended to the fold).
- **The §6.3 audit edge is untouched.** Stamps are maintained by the same
  guarded write path that maintains digests; this section moves only where
  the served value is READ from. No write path changes.
- **The stamp law holds.** Nothing served is stamped with a root its own
  fold did not derive from content digests (§6.7): the resident fold IS
  the fold of the memo's content digests, and the fold-free rebuild stamp
  rides input-equality — a fold of byte-equal inputs is the same value by
  purity, so the pass's own leaf set is still exactly what its stamp
  folds.
- **Sharing changes ownership, never content.** A parsed document is
  immutable once built (`model` law: derived, disposable); carrying it by
  shared reference changes who frees it, not what it says. The resident
  engine is already shared whole across concurrent readers — per-document
  sharing extends the same posture one level down, so a rebuild allocates
  movers only.

The memo's fold counter keeps its semantics — zero on a quiet vouched
pass, one per advance — counting served-fold recomputes; the instrument
behind the count is now O(dirty vertices), never O(corpus).

## 7. Integrity surface + CAS — the grain ladder

The premise grains, one instrument: the resident tree (§6) serves every row.
Legality is D-04 — "it can be any allowable legal token in the tree": every
ADDRESSABLE node is a legal premise. Sufficiency (the coverage law), field
spellings (`scope`, `guards[]`, `scope_bytes`), and requiredness geography
are wire-side law — `wire-contract.md` §5.3 and the wire-law card of this
wave. **No separate `guard` op** (dropped; integrity = mint + premise +
`diff`).

| Premise / op | Grain | Question | Failure |
|---|---|---|---|
| `if_node_rev` (on `splice`) | one node | "is THIS section still what I read?" | `cas_mismatch` {expected, actual} — re-read, re-plan |
| `if_node_rev` on an `fm_key` target (`prop_rev`, §2.1) | one frontmatter key | "did THIS key move?" | `cas_mismatch` at key grain |
| **scoped fingerprint** `{scope, fingerprint}` | any PATH node: root, folder, file leaf, or `absent` | "is THIS subtree still what I planned against?" | `fingerprint_mismatch` {expected, actual, scope} — re-read that scope, re-plan |
| **forest fold** (pattern / selector / sql-provenance premise, §4.3.1) | a derived match set | "is the SET I derived still exactly this?" | `fingerprint_mismatch` naming the set premise |
| workspace token (root scope — the old `if_fingerprint`) | the world | "is the WORLD still what I planned against?" | `fingerprint_mismatch` — resync, re-plan |
| `fingerprint {scope}` op | any PATH node | mint the current token at a scope (root default) | `scope_unresolved` — fix the path |
| `diff` | range of fingerprints | batches of Delta between two cursors | `fingerprint_unknown` outside retained history |

**The scope rows, their law (merged plan §4.4):**

- **`absent` is a value, not an error.** A lawful path with no node — never
  created, emptied, pruned — mints the reserved non-hex spelling `absent`,
  and the chain law holds: absence of the whole prefix is still `absent`
  (`a/b/c` with `a/` itself missing mints `absent` — creation-guard plans
  stand on exactly this). A path is unlawful — `scope_unresolved`, fix
  class — only where it escapes the root, conflicts in kind with an EXISTING
  entry along its prefix, or names a §4.4 collision.
- **Raw-byte names are addressable** through the opaque canonical byte-path
  arm (`scope_bytes`: base64url raw segments; the wire card spells it) beside
  the UTF-8 `scope` convenience — mint and guard serve both. §9's former
  "integrity-covered but unaddressable" posture for names is RETIRED (§9, as
  amended); the UTF-8 read-face serving limits stated there stand.
- **Guard-path freshness:** at check time the engine refreshes the named
  premise's own extent through the watermarked memo (§6.2) — guard one file,
  pay one stat; guard a folder, pay the folder; guard the world, pay the
  world. Refusals narrow because the PREMISE narrows, never because the
  engine looked less hard. The fast path is the instance-bound stamp compare
  (§6.3), O(1) while the event stream can vouch; the extent refresh is the
  floor when it cannot.
- **Radix vertices hold no scope** (§4.3): nothing below a path node is
  addressable, so the grain ladder bottoms out at the file leaf and at
  `absent`.

**`fingerprint` op** — mint the current cursor (root default; scoped form per
the wire card). Bare is the world mint. Under `scoped-guards`:

```jsonc
→ {"id":7,"op":"fingerprint"}
← {"id":7,"ok":true,"body":{"fingerprint":"b3:807b69c6…","seq":N}}
→ {"id":8,"op":"fingerprint","scope":"a/target.md"}
← {"id":8,"ok":true,"body":{"fingerprint":"b3:…","seq":N,"scope":"a/target.md"}}
```

A lawful empty path answers `fingerprint: "absent"` and still echoes the
scope pair. Both `scope` and `scope_bytes` on one request refuse `bad_request`.

**Three errors, three facts, never flattened** (merged plan §4.2/§4.4; the
register-law refusal texts are drafted in the plan's Appendix C):

| refusal family | the fact | the recovery |
|---|---|---|
| `fingerprint_mismatch` {expected, actual, scope} | the premise MOVED | re-read that scope, re-plan |
| `scope_unresolved` | the premise cannot be evaluated at that path | fix the path (`absent` is NOT this — a lawful empty path mints `absent`) |
| cursor family: `fingerprint_unknown`, dead instance, **`fingerprint_version_retired`** | the reference is TOO OLD — a seq past the ring, a reaped instance, a retired hash law (§4.2.5) | re-derive and resume; with the resident tree the re-derivation degrades to a scope-fold compare, never a full relist. `fingerprint_version_retired` teaches re-mint: the premise did not move — the LAW moved |

**Ordering when several premises ride one write:** widest first — root token,
then folder scopes, then per-edit `if_node_rev`; a failing wider premise
skips narrower work (the old two-guard ordering, generalized).

**What each layer never does:** the engine never decides *when* a guard is
**required** (host policy / geography — `wire-contract.md` §5.3, as amended
by this wave's rulings); hosts never compute hashes (node_rev / fingerprint
are opaque equality tokens).

**Consistency with the three laws:** disk stays the only durable truth (memo,
ring, and tree are memory; the checkpoint is disposable, §6.5); recovery is
re-derive; the engine answers “what changed”, policy decides what to do about
it.

## 8. Interaction with the write plane

- `splice` request: optional `if_fingerprint`. Response: fingerprint transition fields per `wire-contract.md` §4.4; `fingerprint_after` is the own-write overlay's fold (§6.1), never a re-read.
- The scoped-premise fields (`scope`, `guards[]`, `scope_bytes`) are capability-advertised; a frozen v2 session using them un-negotiated refuses `bad_request` loudly, never silence (merged plan §4.4). Field law: the wire-law card.
- Node objects / `resolve`: node_rev algorithm (§1–2) is the hash law for CAS tokens.
- Caps advertise `fingerprint`, `diff`, `splice.if_fingerprint` (and related) — not a `guard` op.
- Error codes: `fingerprint_mismatch`, `fingerprint_unknown`, `scope_unresolved`, `fingerprint_version_retired` (not `root_*`); the three-family split is §7's law.
- Routine writes route through the daemon (RULED B, daemon-routed — `decisions/2026-08-15-pre-merge-rulings.md` ruling 4): the CLI rides the daemon's resident tree over IPC; direct writing retires; `LOCK_EX` on `write.lock` becomes takeover/recovery only. The write plane's own law (lease, intents, parallel disjoint commits, the ruled plain-fsync class) is the authority contract's, not this spec's. Construction step 6's publication half — reservation algebra, checksummed `O_EXCL` intents, the one state owner's contiguous root chain — lives in `crates/wire-serve/src/publish.rs` (disk primitives in `crates/fs/src/intent.rs`); the lease half is `authority.rs`. The live write door keeps its interim flock until the cutover flips routing. The `apply_batch` pre-image verify is the in-process second-writer refusal under B (`docs/laws.md` Amendment — the one state owner).
- The effects lane (`run`, script-with-effects) is UNGUARDED by ruling (`decisions/2026-08-15-no-guard-on-effects.md`; the normative paragraph lives in `run-plane.md`) — but guard-free never means fold-invisible: every effects write rides the same write choke-point and maintains the resident tree (leaf update, chain refold, §6.2 watermark discipline). That is tree maintenance, not a guard.
- **File death mints no terminal hash (RULED, ZT 2026-08-15, card `engine-delete-door`: "No tombstone — death Delta is the record").** A guarded `remove` (`wire-contract.md` § A.3) unlinks the leaf; the next fold composes the tree without it under the current §4 encoding — removal is already in the diff shape (§0, "whole-subtree enumeration on add/remove") and no new hash law exists for it. Under law 2 this is the §4.2.2 delete rule: the child map re-canonicalizes as if the entry never existed. A rev is a function of bytes (§2); absent bytes mint nothing: the death's terminal facts are the removed file's LAST rev (`file_rev_before`, confirmed by the remove-what-you-read CAS) and the workspace fingerprint transition, both carried by the death Delta (`change:"deleted"`, `wire-contract.md` §7.1). No tombstone leaf, no on-disk marker — disk stays markdown only, and history past the ring re-derives to a world where the path is simply absent. The emptied path itself now mints `absent` (§7) — a legal premise, not an error.

## 9. Normalization rulings (closed for v1)

- **Names: raw bytes, byte-order sort, no unicode normalization.** The fingerprint is a per-host daemon↔client cursor, never a cross-host sync token — the NFD/NFC divergence (macOS vs Linux) cannot bite a cursor that never crosses hosts. Revisit only if fingerprints ever travel between machines (then: NFC at hash time, flagged as proto-visible).
- **Name truthfulness (RULED, ZT 2026-08-08, "6b"):** *"we don't have conversion. the content for markle hashing is always truthful. if we have conversion, that's a flaw. if we are talking about per session display layer conversion, such layer is not possible to drift. in defined snapshot, the conversion is two way convertible with zero lose."* (verbatim, unparaphrased). Operationally: every fold — the workspace fingerprint, the exec-guard bracket folds, any derivation that claims to be the corpus root — carries the raw on-disk name bytes end to end. Conversion is legal only in display layers (error prose, listings), and there only where it is two-way convertible with zero loss for the servable set; an escaped rendering (`\xNN` for invalid bytes, `\\` for a literal backslash) is the display form for a name that has no UTF-8 spelling.
- **Non-UTF-8 names: hashed truthfully, addressable via `scope_bytes`, unservable on the UTF-8 read faces — AMENDED 2026-08-15 (bounce-1 closure; the "integrity-covered but unaddressable" posture is RETIRED).** A domain member whose NAME is not valid UTF-8 still gets its leaf and enters the root with its exact name bytes (blake3 and the interior encoding need no UTF-8). The integrity plane now serves it: mint and guard take the opaque raw-byte arm (`scope_bytes`, §7; field law `wire-contract.md` §5.4), so a premise can name it and coverage can count it — the plan rules this explicitly (merged plan §4.4, "Raw-byte names are addressable"). What remains true is the READ-face limit: wire paths on the serving surface are JSON strings (UTF-8 by construction), and no injective UTF-8 spelling exists that also keeps every valid name fixed — so such a member serves no spans, the serving snapshot (`DomainFiles`) carries only the UTF-8-named members, and a watch delta cannot name such a path; the frame's fingerprints stay truthful and the §6/§7 resync law covers what the delta cannot spell. Reachability: macOS refuses creating such names (errno 92); Linux is the reachable platform.
- **Symlinks: skipped silently** — consistent with the addressing jail's stance that symlinks are resolved and confined at the addressing layer; a symlink's target, if in-tree, is hashed at its real path. Cost: retargeting an in-tree symlink alone doesn't move the fingerprint — accepted for now, listed below.
- **Content: raw bytes always** (§2, §3). CRLF, trailing whitespace, BOM — all hash as written.
- **File/dir name collisions: no longer ignored.** Law 2 hashes both kinds (§4.4 — nothing sits outside the fold), lints the build loudly, and refuses `scope_unresolved` at mint and guard on the colliding path. All the rulings above carry into law 2 unchanged: raw name bytes end to end are the trie's key bytes, byte-order sort is the slot and forest ordering (§4.2.2, §4.3.1).

## 10. Open questions for architecture review

1. **node_rev width** — 16 hex (64-bit) per §1, vs the contract examples' 6. Examples are non-normative, but the amendment that freezes `resolve` should state the width; any objection to 16?
2. **Ring bound** — settled at **256 roots** in `wire-contract.md` §13: older ranges answer `fingerprint_unknown` → full resync — re-derive, never wrong data.
3. **Hash domain** — settled for design in `wire-contract.md` §12 (md-only + `meridian/domain.md`). This spec’s leaf rule must stay aligned with that domain filter.
4. **Symlink retarget invisibility** (§9) — acceptable for now, or hash the target path as a pseudo-leaf?
5. **Multi-file atomic batch** — limit stated in `wire-contract.md` §6.5; vocabulary is `if_fingerprint` + batch `splice`.
6. **`diff` payload cap** — a wildly stale fingerprint can name thousands of paths; cap + `truncated:true`, or force `fingerprint_unknown` past a threshold? (The mismatch `changed` field was struck — `wire-contract.md` §18 row 2 — leaving only the `diff` half open.)

