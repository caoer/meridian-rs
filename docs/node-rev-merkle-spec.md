---
type: spec
id: merkle
status: standing
description: Normative hash law for `node_rev` and the workspace merkle fingerprint (two law versions, radix-256 from the cutover), plus the resident tree that serves it, with worked examples.
owns: [node_rev, merkle encoding, resident tree, event feed]
---

# node_rev + workspace fingerprint (merkle) spec

> Standing law: `README.md` (process and standing corrections) and `wire-contract.md` (the wire contract).

**Scope note:** hash law for `node_rev` and the workspace merkle (fingerprint), plus the **resident tree** — the engine-held instrument that serves that law (§6: structure, stable-read protocol, stamps, event feed, checkpoint). It does not define section address grammar; mint-plane hpath stays segments only. Generator assets: `node-rev-merkle-spec.assets/`.

**Binds:** what bytes are hashed (§2, §3), how file leaves compose into the 32-byte workspace fingerprint (§4), how one `splice` updates that fingerprint incrementally (§5), and the resident tree with its event feed (§6).

The design noun is **`fingerprint`**, the workspace content hash; the wire spells the field `root` in v2 vocabulary, re-keyed to `fingerprint` under `contract:"v3"` (`wire-contract.md` §1). Wire integrity is `fingerprint` + `if_fingerprint` + `diff`, with no `guard` op (`wire-contract.md` §4.7). The scoped-premise surface — `scope`, `guards[]`, `scope_bytes`, `absent` — is `wire-contract.md` §5.4's to spell, against §7's grain ladder. Three laws bound the design: no snapshot files (the §6.5 checkpoint is a disposable index), no second database, Rust memory disposable.

## 0. Design inheritance — the merkle-root-spike, absorbed

An earlier merkle-root prototype was folded in: scheme adopted, blake3-256 for xxhash64, persistence layer dropped.

**Taken:**

- the injective **interior encoding** — a directory's hash over its sorted child entries — whose length prefix and type byte block sibling-boundary reinterpretation;
- names hashing into the **parent**, so a rename is remove+add there and leaves untouched;
- the **diff shape** — equal roots ⇒ one comparison (the commit fast path), unequal ⇒ descend only unequal branches, naming every drifted path in one pass, remove+add on species change, whole-subtree enumeration on add/remove;
- the **pluggable leaf hasher seam** (§3);
- the **measured envelope** (M4 Max): a 4,141-node tree → root 74ms, a 9.5GB/50,319-node corpus → 2.2s warm / 5.2s cold.

§4.1 and §4.2 carry the rest as law: git-style empty-dir pruning, the unhashed root name, unhashed file modes, byte-sorted child names.

**Rejected:**
- **Snapshot persistence** (a `Save`/`Load` format outside the hashed dir) — violates law 2 (Rust memory is disposable; disk stays markdown only); the root re-derives on demand (§6–7). The ban covers **trusted** snapshots; §6.5's disposable, checksummed, identity-bound checkpoint is the allowed opposite.
- **xxhash64 width** — 64-bit is a race detector, not collision-resistant; §1 rules blake3-256.
- **mtime+size leaf cache** — its lie window (same mtime+size, different bytes) buys warm-rebuild speed the resident, event-fed tree (§6) does not need; cold start eats the 2–5s. Under §6 the memo — the cache of per-file leaf digests — is keyed by `StatKey` (device, inode, size, mtime, ctime), and §6.2's watermark closes that window.

## 1. One hash family: BLAKE3-256

Every hash here — `node_rev`, file leaf, interior, workspace fingerprint — is BLAKE3-256: one primitive, one implementation, no mixed families. It is crypto-grade at xxhash-class speed, and the 32-byte fingerprint is law 2's cursor.

- **`fingerprint`** = `"b3:" + 64 lowercase hex chars`, algorithm- and domain-prefixed (`wire-contract.md` §1 / §12). Short `"b3:88d2aa"` forms in old examples are non-normative abbreviations.
- **`node_rev`** = the first **16 lowercase hex chars** (64 bits) of the node hash, unprefixed: enough to detect races in one node's edit history. The fingerprint is the integrity cursor and keeps full width.
- Both are opaque to clients: equality only.

## 2. node_rev — what bytes are hashed

`node_rev = hex(blake3(node_span_bytes))[:16]`, `node_span_bytes = raw_file_bytes[span.start : span.end)` — the node's **span bytes exactly as issued** (`wire-contract.md` §1 span sub-laws; leaf block spans exclude the final line terminator).

- **Section** (heading ref via `resolve`): heading-inclusive (heading line through end of subtree), so `node_rev` covers the heading and a rename invalidates the token. `content_span` (`wire-contract.md` §1 rev sub-laws) is a write-target convenience and mints no rev.
- **Frontmatter**: the whole-block span, `---`…`---` inclusive (`wire-contract.md` §18 row 3, span-lawed with the section family). The per-key grain mints its own rev, `prop_rev` (§2.1). The delta sub-array `keys:[{key, change, value_rev}]` (`wire-contract.md` §7.4) is future-only.
- **Other node kinds** (`toc`/`extract` — `wire-contract.md` §4.1/§4.3): their `wire-contract.md` §1 span, verbatim.
- **No normalization of content**: no newline canonicalization, no trailing-space trim, no NFC. Raw bytes only: otherwise two equal revisions could denote different disk states.

### 2.1 `prop_rev` — the per-key frontmatter CAS token

`prop_rev = hex(blake3(fm_key_grain_span_bytes))[:16]` — §2 applied at a finer grain, one frontmatter key instead of the whole block: same family (§1), same 16-hex width, same equality-only opacity.

**The grain.** `fm_key_grain_span` = the key line plus every indented continuation line of a block value. The key name is inside the span; the end excludes the last content line's terminator (§1 leaf law). A blank line joins the grain only if a later indented line extends past it; trailing blanks belong to the inter-key gap. The scan stops at the next column-0 non-blank line or the block end.

**Why it exists beside the block-grain `node_rev`.** A frontmatter node's `node_rev` covers the whole block, so every key shares one token (6586/6586 multi-key documents in a 6586-document corpus). Guarding one key with that token refuses `cas_mismatch` when any other key moves. Block grain says whether the frontmatter moved, never whether a single key moved. Both revs are additive.

**One owner, three faces.** Only `model::resolve(doc, Ref::FmKey(key))` computes the token, the value the write door (where a write enters the engine) compares `if_node_rev` against; every face **serves** it.

| Face | Spelling |
|---|---|
| the write door's guard | `if_node_rev` on an `fm_key` target (`wire-contract.md` §4.7) |
| the composed read | `props[].prop_rev` (`wire-contract.md` § A.3) |
| the corpus projection | the `frontmatter.prop_rev` column (`mrd sql`) |

The projection's `frontmatter.node_rev` column keeps block-grain meaning.

**Stored form, never decoded.** `prop_rev` hashes source bytes (`wire-contract.md` § A.6.2): a guard token must distinguish `owner: ""` from `owner:`, which the value-plane decode (`wire-contract.md` § A.6.1) collapses into one state.

## 3. File leaf hash — and why there is no per-file sub-merkle

`leaf(file) = blake3(raw_file_bytes)` — full 32 bytes, whole file, for every path in the **hash domain** (`wire-contract.md` §12: md-only floor, default/custom ignores via `meridian/domain.md`). Non-domain paths never enter the tree.

Section-grain leaves (§0's seam) are **rejected for tree composition**: the whole-file hash already changes iff any node's bytes change, and sub-file drift-naming comes free from the parser (`toc`/`extract` diff, or `DiffSections`-style rev-table comparison at ~0.2ms/file). `node_rev` holds node-grain integrity (§2), the tree file-and-above (§4).

Files that are **not valid UTF-8** still get leaf hashes and enter the root but serve no spans or nodes (wire `invalid_utf8` law): integrity coverage and span service are independent.

## 4. Tree composition — leaves to the 32-byte workspace fingerprint

Two interior laws exist — two encodings for folding a directory's children
into one hash. **Merkle law 1** (§4.1, the flat encoding) is the shipped law,
retiring at the one-time cutover; **Merkle law 2** (§4.2, the fixed-256 radix child map) is the law of the first scoped-token version. Exactly one law is current per workspace; no dual-hash serving window. The cutover is a priced protocol, paid once, behind the pre-cutover code blockers §4.2.5 names.

### 4.1 Merkle law 1 — the flat interior encoding (retiring at the cutover)

The inherited scheme, blake3-256 for xxhash64:

```
interior(dir) = blake3( concat over children sorted by name-bytes:
 varint(len(name)) ‖ name_bytes ‖ type_byte ‖ child_hash_32B )
type_byte: 0x00 = file, 0x01 = dir
```

- Children sorted by raw name bytes (§9 for the unicode caveat); symlinks skipped (§9); empty dirs pruned bottom-up (a dir with all children pruned is pruned too); the workspace tree root always exists.
- **`name_bytes` are the exact on-disk bytes of the child's name** — on Unix, the `OsStr` bytes verbatim (§9). `to_string_lossy` on the hash path is a spec violation: two non-UTF-8 names decoding to one replacement string would collapse to one leaf. A `\` inside a name is a name byte, not a separator.
- The workspace directory's own name is not hashed (no parent to hold it), so identical content gives an identical root.
- **`fingerprint` (wire noun)** = the workspace tree's interior hash, `b3:<64hex>`; the prefix may advance with the domain `version` (`wire-contract.md` §12.3). A file-scope leaf hash is that file's leaf, not a second wire "root" op.
- **Why it retires:** re-folding a directory re-encodes every child, so a flat 100,000-file folder stays O(100,000). Keeping this encoding and sharding later is rejected; the cutover is taken while no scoped tokens exist in the wild.

### 4.2 Merkle law 2 — the fixed-256 radix child map (the new hash-law version)

Each directory's child list is a canonical radix map, fanout fixed at 256 — one slot ("bucket") per byte value — so one change re-hashes a bounded number of vertices (the nodes of that map), never every sibling. Canonical: history never affects the result. Updating one entry touches the vertices on its key path (bounded by the name's byte length) plus one directory node per ancestor; sibling count appears nowhere.

**Carried over from law 1 unchanged:**

- the hash family (§1);
- the untagged leaf law (§3) — a leaf stays the plain blake3 of the file, checkable with any b3 tool and reusable across the cutover from the `StatKey` memo, while the kind byte beside every hash prevents cross-kind confusion;
- raw name bytes, byte-order sort and zero normalization (§9);
- symlinks skipped (§9);
- empty directories pruned bottom-up;
- the workspace root's own name never hashed;
- file modes never hashed.

**Definitions.** A child set `C` holds entries `(name, kind, hash)`: `name` = the child's exact on-disk name bytes (§9), `kind` = file or dir, `hash` = its 32-byte value (file → §3 leaf; dir → §4.2.3 value). Names are unique in `C`; one name arriving as **both** kinds is the collision case (§4.4). Every varint is unsigned LEB128 in **minimal-length form**; a non-minimal varint is illegal. Byte comparisons are unsigned.

#### 4.2.1 The canonical radix trie over `C`

The child map is the radix trie over `{name}` built by this recursion; no other shape is legal for a given `C`.

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

Invariants forced (an encoder emitting anything else is out of law): a vertex with no terminal has ≥ 2 children (else `ext` was not longest); a vertex with no children has a terminal; vertex count ≤ 2·|C| − 1; fanout ≤ 256.

#### 4.2.2 Vertex hash — bucket layout and the empty-bucket rules

A vertex hashes its own prefix (`ext`), its terminal entry if it has one, and
its occupied child slots:

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

Domain tags are the literal 8 ASCII bytes shown: no terminator, no length prefix (§4.3).

- An unoccupied slot contributes **nothing** — no placeholder byte, no zero hash; occupied slots only.
- A vertex with no terminal and no children is unrepresentable.
- On delete the map re-canonicalizes as if the entry never existed: a vertex left with one child and no terminal merges into it (prefix re-extends), and an empty slot vanishes from its parent's frame. No tombstones, no retained split points.

#### 4.2.3 Directory value and the workspace fingerprint

```
dir(d) = blake3( "mrk2.dir" ‖ vhash(child map of C) )    # C nonempty
dir(workspace root with C = ∅) = blake3( "mrk2.dir" )    # the empty tree
```

- The `mrk2.dir` wrap gives a directory one value whatever its trie shape, and
  keeps directory values apart from vertex values (§4.3).
- Only the workspace root may be empty; non-root empty directories stay pruned.
- The **workspace fingerprint** = `dir(workspace root)`; a child directory's value is the `hash` in its parent's child map, and a file-scope value is its §3 leaf.

#### 4.2.4 The cost law, stated

A one-entry change re-hashes its key-path vertices (bounded by the name's byte length; typically 1–3 with compression), the `mrk2.dir` wrap, then the same per filesystem ancestor. Each vertex pre-image is at most `8 + varint + len(ext) + 65 + varint + 33·256` bytes, ≈ 8.5 KiB at full fanout — bounded by the 256 fanout, never directory width. Law 1 re-encoded every sibling: O(width), unbounded.

The flat-100k acceptance gate publishes operation counts: per-commit unrelated-member stat/read/hash counters = 0, plus the flat-100k directory case, held to the same vertex count. Calibration: a live corpus's maximum directory width was 428.

#### 4.2.5 Versioning — one law current, typed retirement

- The hash-law version is its own dimension, **orthogonal** to the workspace-domain version: the `mrk2.` tags bind it into every interior value, so law-2 and any future law-3 values never collide over identical structure. The wire prefix advance (today `b3:`) is `wire-contract.md` §12.3's.
- At the cutover every **token** value changes; leaf values survive but re-spell under the new version prefix.
- Three errors, three facts, never flattened (§7): an old-law token refuses **`fingerprint_version_retired`** with re-mint teaching, never `fingerprint_mismatch`; an unknown **future** family refuses a distinct unsupported-version error; only a current-family unequal digest is the scoped mismatch.
- **No dual-hash serving window** — keeping two trees for old tokens is waste (requirement 6). The cutover is paid once per workspace, never on every read; `sub` re-baselines at a labeled epoch boundary, never a silent chain break.
- **The cutover walk** runs behind three pre-cutover code blockers (cursor identity, the hash-law version dimension, durable cutover authority state): `OLD_SERVING → DRAINING → no-return boundary → NEW_BUILDING → NEW_COMMITTED → reopen`. One durable act crosses the no-return boundary: making the cutover record's `NEW_BUILDING` state durable. Before it the old law answers guards; after it, old-family tokens refuse `fingerprint_version_retired`.
- **The downgrade fence never activates.** `crates/fs/src/fence.rs` is dormant (`activate` has no production caller) and the threat model is empty, so it is no cutover blocker. This law's axis is old-**token** refusal, not old-**binary** fencing.
- **The shadow build is not built.** The one-time per-workspace pause is accepted; no second tree beside the serving one, even as preparation. Acceptance records the real pause and does not gate on a budget.

### 4.3 Domain tags — one table

Every interior hash in law 2 starts with an 8-byte ASCII domain tag, and the three are equal length differing at byte 5, so they are prefix-free. Content leaves are untagged (§4.2).

| tag (8 ASCII bytes) | over |
|---|---|
| `mrk2.vtx` | a radix-map vertex (§4.2.2) — hash-law internal, see below |
| `mrk2.dir` | a directory's child map (§4.2.3) — the scoped directory value |
| `mrk2.fst` | a forest fold (§4.3.1) — a derived match set, never a directory |

**Vertices are hash-law internals.** A caller premise (the claim a guard carries, §7) names **path** nodes only:
workspace root, folder, file leaf, or `absent` — a lawful path with no node. Vertices and slots are hashed, never addressable as a `scope`, and no wire surface mints or compares one.

#### 4.3.1 The forest fold — its own domain tag

A set premise ("all files matching `a/*.md`"; "the rows this query scanned") guards a **derived match set**, not a directory, so its fold carries its own tag:

```
forest(M) = blake3( "mrk2.fst" ‖ varint(n)
                    ‖ n × ( varint(len(path)) ‖ path_bytes ‖ leaf_32B ) )
```

- `M` = exactly the matching members: workspace-relative paths in raw bytes, segments joined by `/` (0x2f, which a POSIX name cannot contain), strictly ascending; `leaf` = each member's §3 leaf hash; `n` = |M|.
- Computed from the resident listings at O(dir width), zero byte I/O. A **matching** new sibling moves the fold; a non-matching one never does. Deletes and renames are caught by two-point set comparison (entry vs live expansion).
- `n = 0` is legal: `blake3("mrk2.fst" ‖ 0x00)` is the fold of "nothing matches", a mintable premise guarding continued emptiness.
- The subtree fold (`mrk2.dir` at a folder scope) stays the conservative fallback and the explicit directory-premise form.
- Consistency law: every set premise — pattern root, selector root, sql provenance — validates against the **tree**; no premise consults the journal.

### 4.4 Name collisions — lint loud, refuse at address time

Law 1 ignored file/dir name collisions both ways, which left some path's bytes outside the fold. Law 2 closes both halves:

- **Both kinds enter the fold.** A name reaching one child map as both file and directory (however composed, e.g. via the write overlay) is one key whose terminal carries both values (`0x02`, §4.2.2); no bytes sit outside the integrity surface.
- **The build lints the collision loudly** — a named diagnostic on every build, never silence — and serving continues: one odd name must not take a workspace down.
- **Collision paths refuse addressing.** The colliding path, and every path through it, refuses **`scope_unresolved`** (fix class) at mint and at guard: `scope: "x.md"` cannot say which kind it premises, and an ambiguous premise is no premise — also the forest digest's precondition (§4.3.1).
- Collision paths stay integrity-covered but unaddressable, even as §9's non-UTF-8 posture retires behind `scope_bytes` (§7): a byte-exact arm says which bytes, no premise arm says which kind.

## 5. Worked example (law 1)

Workspace: two entries. `notes.md` = `"# Notes\n\nhello\n"`; `tasks/x.md` (64 bytes) =

```
---\ntitle: demo\n---\n\n# Alpha\n\nbody line one\n\n## Beta\n\nbeta body\n
```

Node spans (wire law: block spans exclude the final line terminator):

| node | span | node_rev = blake3(span bytes)[:16] |
|---|---|---|
| frontmatter | `[0,20)` (`---\ntitle: demo\n---\n`) | `c93f2c5ca47ac0a0` |
| section `#Alpha` (resolve, heading-inclusive) | `[21,64)` | `3d5903c3604ee3ac` |
| section `#Alpha/Beta` | `[45,64)` | `780d2fb4cf68f60f` |

> **Frontmatter row receipt.** Frontmatter is terminator-inclusive: a
> fence-to-fence container span-lawed with the section (newline-inclusive)
> family, not the leaf-block family (`wire-contract.md` §18 row 3, declared
> waiver). The engine never serves the leaf-law `[0,19)`. Regenerated over
> these two files, byte-exact:
>
> ```
> $ printf '%s\n' '{"id":1,"op":"hello","proto":1,"client":"spec-regen/0.1","workspace":"/path/to/fixture-workspace"}' \
>                 '{"id":2,"op":"toc","path":"tasks/x.md"}' \
>   | nc -U "$SOCKET"   # the daemon's short hash-keyed sock — `hash(cache_root)` under $XDG_RUNTIME_DIR/mrd (Linux) or ~/.cache/mrd-run
> {"id":2,"ok":true,"body":{"path":"tasks/x.md","file_rev":"1e56548abcd43053",
>  "root":"b3:807b69c693ad2c65e290422a1123198f22be6161c2caa43d71fab029fa4763cd","nodes":[
>  {"kind":"frontmatter","span":[0,20],"node_rev":"c93f2c5ca47ac0a0","text_prefix_16b":"---\ntitle: demo\n","keys":["title"]},
>  {"kind":"heading","level":1,"hpath":[{"h":"Alpha"}],"span":[21,64],"content_span":[29,64],"node_rev":"3d5903c3604ee3ac","text_prefix_16b":"# Alpha\n\nbody li"},
>  {"kind":"heading","level":2,"hpath":[{"h":"Alpha"},{"h":"Beta"}],"span":[45,64],"content_span":[53,64],"node_rev":"780d2fb4cf68f60f","text_prefix_16b":"## Beta\n\nbeta bo"}]}}
> ```
>
> Response reflowed for line width only; values verbatim, matching the rows
> and fingerprint pinned above.

Leaves (blake3 over the raw file):

```
leaf(tasks/x.md) = 1e56548abcd43053053ef8f06b68c3261a7d29aa2a03aaa80b0a2f204d213d7e
leaf(notes.md) = 96c26935d00a13398c39887a29adeb554d351b6863ec776c31d4a7f7f93f1875
```

Interior `tasks/` — 38-byte pre-image, one child entry: `04` (varint len 4) ‖
`x.md` ‖ `00` ‖ leaf:

```
pre-image: 04 78 2e 6d 64 00 1e56…3d7e
interior(tasks/) = f7a2e4b1af9ef2aa9d57abaa4375e6cff8c474c2f6dd788bc6a9d2543f0277fe
```

Root — entries sorted by name (`notes.md` < `tasks`), 81-byte pre-image
`08‖notes.md‖00‖leaf ‖ 05‖tasks‖01‖interior`:

```
fingerprint = b3:807b69c693ad2c65e290422a1123198f22be6161c2caa43d71fab029fa4763cd
```

**Incremental update:** splice Beta's body `beta body\n` → `beta body v2\n`;
exactly one path recomputes:

```
node_rev(#Alpha/Beta): 780d2fb4cf68f60f → f34813be3889438e
leaf(tasks/x.md) : 1e56…3d7e → b78aa71202f4273e830ace6c7844b8943a53c04d1bab719586af2c3a307907ef
interior(tasks/) : f7a2…77fe → 234267c9a1b642b751e50dabed092664a0013fce2c1b22738f6279ac99075a4f
fingerprint : b3:807b… → b3:a1f7bb8e46227d0c44df8c993fa1ab066b299d275d01d81e5dd6c40ba665b7c2
leaf(notes.md) : unchanged (96c26935d00a1339…)
```

From `node-rev-merkle-spec.assets/worked-example-gen.go` +
`node-rev-merkle-spec.assets/go.mod` (blake3-256), the test-suite fixture
seed. Interior and fingerprint values are law 1; `node_rev` and leaf values
are law-independent, surviving the cutover.

### 5.1 The same workspace under law 2

`notes.md` and `tasks` share no first byte (`n` = 0x6e, `t` = 0x74): the
child map is one root vertex over two leaves:

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
`ext`, `01 ‖ 00 ‖ hash` a one-terminal frame of kind file, and the trailing `00`
a `children_frame` with n = 0.

At scale, 100,000 names sharing `2026-08-1` collapse into one vertex's `ext`;
a key path stays a few vertices deep.

Law-2 arm of the same generator; re-derived byte-identically by the engine
encoder `crates/fs/src/radix.rs`, gate
`crates/fs/tests/radix_gate.rs::spec_worked_example_law2_byte_identity`:

```
vhash(v_x, tasks/ map) = 2ca0edd90ba490f03108cd25dd5d12ab21ecb51950a904466588ffffda6588e8
dir(tasks/)            = ef0e7e2eca3cacfcc3bf8fded1454d65645a5a20359c770d6e2dea009d285bd2
vhash(v_n)             = 267b393de0d71194cf17376fef3017d11951b31fab29f70c0637730d0859910c
vhash(v_t)             = a7cbe077445b77bc24873902ce9e896e91f5dd9fcce5d224e4180f4b9bd0e7d9
vhash(root)            = de4f14de1fe5206850e917db2e5ea95306b6b7cfc5935da496a4c92c979fb952
fingerprint (law 2)    = d53c447167825d40f442c65b10f5ae2c6176a49e1e2d8237902d7eaa3008319e
```

Bare 64-hex: the law-2 wire spelling rides the `wire-contract.md` §12.3
prefix ladder, which that contract advances. The §5 splice under law 2
recomputes exactly the `x.md` key path:

```
leaf(tasks/x.md)    : 1e56…3d7e → b78a…07ef   (law-independent, as §5)
dir(tasks/)         : ef0e…5bd2 → e4f51f04970d9feb5c680de5534e1824b27d2660577395e5fadcd9d82fb8a967
fingerprint (law 2) : d53c…319e → 6aab1dd1ef89648508430e0ded866c6ad964b1074fc9b624d025f5c27d10fc58
vhash(v_n), leaf(notes.md): unchanged
```

## 6. The resident tree — memory-held, event-fed, checkpointed

The resident tree is the engine-held instrument that serves this hash law.
Derive-on-demand — full sweep and fold per pass, no persistence, cold rebuild
per start — was rejected: two flock-held full-corpus reads per guarded write
(~1.5 s) for 32 bytes, and root-grain world guards.

### 6.1 The resident structure and the own-write overlay

The tree lives in `fs::DomainCache`, which already holds per-file leaf digests
keyed by `StatKey` (device, inode, size, mtime, ctime) and per-directory
listings — no second subsystem beside it. Per node: the §4.2 child map, the
cached 32-byte fold, a dirty bit, a `last_seq` stamp (§6.3).

- **Own writes update the tree synchronously.** A commit replaces the leaves it
  wrote and re-folds the ancestor chain (one leaf + 13 levels: 13.6–20.6 µs,
  median 14.0 µs, measured). `root_after` / `fingerprint_after` derives from
  that overlay, never from a second corpus read (`DomainLeaves::overlay`'s own
  doc law); the overlay includes the receipt append the engine composed, with
  no post-apply reload. A domain-config write applies
  `DomainCache::overlay_membership`: the new `Domain` from the commit's own
  config bytes replaces membership on the overlay's leaves — departed members
  drop, version and ignore rules update, no disk walk, no new member read.
  A remove calls `overlay_remove`, then `overlay_root`. No config or remove
  exception re-observes; both exclusion-held `ambient_root` corpus reads leave
  the write path. The splice response keeps its `wire-contract.md` §4.4
  transition fields.
- **Write doors ride the same `DomainCache` the feed patches.** The daemon
  passes `Registry::domain_cache` into each write door — an argument, not a
  process-wide hook. The registry makes the door-entry observation inside the
  door's flock on that memo: §6.4 cookie barrier, then take-and-apply — the
  pending dirty set taken and applied. The overlay serves as `root_before`
  only on `Seen` (the cookie came back through the ordered event stream,
  proving earlier events are folded in) + no doubt collapse + `Trusted` — the
  vouch `currency_refresh` demands. `Trusted` says the last observation landed
  whole with no unabsorbed loss, not that the stream delivered everything on
  disk; without the cookie, a silent-dead watcher or a sticky failed feed
  looks like a quiet corpus. A drained dirty set is no
  completeness proof, and any named miss degrades to the full observation that
  absorbs the loss (§6.2 row 6). In-process callers with no registry fall back
  to a process-local map, still live-observing every door entry. The watcher
  notes loss on that cache's `FeedGen` cell, so a mid-read fence can fire.
- **Foreign changes arrive through the feed (§6.4)** and mark touched nodes
  dirty; folds recompute lazily on demand — maintenance cost follows change,
  never corpus size (requirement 1).
- **Memory:** ~10 MB at 30k members (estimated, never measured); full fold from
  known leaves 12.1 ms; build-from-scratch stays the 1.45 s cold baseline
  (rare, §6.5).

### 6.2 The watermark trust close — the full stable-read protocol

The engine trusts a cached leaf digest only where this protocol says it may.
`StatKey` alone can miss a same-instant, same-size, in-place write. The close
is git's racy-clean rule, adopted as law; acceptance: a same-tick same-size
in-place edit still refuses.

1. The leaf memo carries an **observation watermark**; a leaf with mtime at or
   after it is re-read before its digest is trusted, or its memo entry is
   deliberately spoiled.
2. A **per-backend timestamp-granularity calibration probe** runs at workspace
   open: the comparison unit is measured, never assumed.
3. Reads **open without following links**, with an fstat identity check before
   and after the byte read; identity moved mid-read ⇒ discard and re-read. The
   no-follow walk is a directory-fd `openat(O_NOFOLLOW)` per component **from
   the workspace root down**: every component below the root refuses a symlink.
   The root — trust anchor, not member — opens as the caller named it, symlinks
   in its own prefix followed; production roots are canonical at bind
   (`workspace::canonicalize`).
4. An **event-generation fence** brackets the read: a feed event landing during
   the read re-classifies it instead of admitting a torn observation.
5. A **still-open in-place writer** is suspect until its identity settles.
6. **Unknown capability or event loss puts guard currency in a loud untrusted
   state** — never silent trust.

### 6.3 Stamps — `last_seq`, instance-bound

A stamp answers "did anything under this node change?" without re-folding the
subtree.

- Each node carries `last_seq` = the highest journal seq beneath it, kept by
  the same guarded write path as the digests: the hash instrument audits the
  stamps.
- Fast path: a subtree is untouched iff its `last_seq` ≤ the token's seq — one
  node read, O(1). Legal only while the event stream can vouch (§6.4); else the
  §6.2 extent refresh is the floor.
- **Stamps are instance-bound:** ring seq is per-daemon-epoch and rings are
  idle-reaped, so a compare across a reap or restart could false-pass
  "untouched". Stamps and stamp-bearing tokens carry the tree instance id; a
  mismatch degrades to the content-fold compare — epoch-free, history-free.
- **Hash tokens are epoch-free; cursors are not.** A content-hash guard token
  survives daemon restart; cursors stay in the delta plane. Advisory
  `{instance, seq}` hints may ride premises, and the engine answers identically
  without them.
- **Stamps never answer for the dead:** a deleted or renamed-away path has no
  node to carry a stamp. Delete visibility comes from journal frames while the
  journal lives; past the ring's horizon it is cursor-too-old — re-derive, not
  a stamped guess.

### 6.4 Feeding the tree — the event feed and the rescan ladder

The tree must learn about changes the engine did not make; the event source is
engine-side.

- **The engine owns its senses.** One kernel file watcher per workspace
  (FSEvents on macOS, inotify on Linux), owned by the engine process, never a
  client. The reasons: a dead cross-process feed is indistinguishable from a
  quiet corpus; the engine holds workspaces the daemon never tracks (CLI lane,
  ad-hoc repos, CI fixtures); and the currency proof must ride the same stream
  that feeds the tree.
- **A client daemon's journal is a legal additional feed** where it already
  watches — an opportunistic dirty-path hint, never an instrument a guard or
  currency answer depends on.
- **Guard correctness consults neither journal nor watcher.** The guard is a
  live fold over the named premise through the watermarked memo (§6.2), so the
  journal's vacuous windows — unjournaled external edits, idle-reaped rings, a
  seq reset on restart — cannot touch it. No premise consults the journal
  (§4.3.1's consistency law).
- **Watcher lifecycle:** the watcher lives with the workspace registration,
  bounded by the resident budget. An idle-reaped engine keeps it; events
  accumulate in a registry-held dirty set that the next warm applies —
  O(dirty), never O(corpus). The registry holds one parsed corpus per warm
  workspace, with a resident budget (`MRD_MAX_RESIDENT_BYTES`, estimated
  resident bytes — a fixed multiplier over the warm set's raw markdown
  bytes). LRU eviction under budget pressure drops the whole warm state,
  watcher included; the next warm is a full walk, not O(dirty). Bounded
  residency takes priority over gap coverage for the least-recently-used
  workspace. Only a live subscription exempts a workspace from the budget
  sweep; the registration survives either way.
- **The currency barrier (the cookie).** A guard-grade currency question writes
  a sentinel at `.meridian/cookie` and waits for it to return through the
  ordered event stream. `Seen` — ordered delivery of all the kernel captured,
  no capture-gap doubt open — proves in O(1) that earlier events are folded in.
  A new directory, or any non-member-candidate path that can hide members, is a
  missed-event: `Seen` is then illegal, and the barrier answers unproven at
  once, without spending the cookie timeout. The dot-prefixed cookie path is
  outside the hash domain by the standing `wire-contract.md` §12.1 floor, so it
  can never move the root or break a held token; a cookie inside the hash
  domain is refused.
- **The rescan ladder — every cause named, throttled:**

| Cause | Response |
|---|---|
| kernel event overflow | mark-all-dirty; the next pass is the full stat sweep (160 ms warm, measured) |
| a new directory (or other hideable non-member) | mark-all-dirty under missed-event; the same sweep recovers the child that landed before the sub-watch armed |
| watcher instance change | one labeled re-baseline (1.45 s cold / 160 ms warm, measured) |

  The watcher never restarts across a rescan; a rebuilt index commits by swap.
  Self-echo dedupe is a cost saving only: overlay idempotence is the
  correctness.
- **Idle re-check — suspicious-only, no timer.** Re-checks run only on a named
  doubt — a missed event (including a path that can hide members before its
  watch arms), watcher overflow, an instance change, a failed spot check (vouch
  failure), a cookie timeout — and may ride a guard-path touch. Zero background
  work when healthy (requirement 6, "no waste anywhere"). The periodic idle
  sweep is declined at a price: silent event-stream loss with no named trigger
  waits for a guard.

### 6.5 Restart — the disposable checkpoint

Requirement 2 — "the engine knows, it does not re-ask" — binds across ordinary
daemon restarts. So the engine may hold a checksummed, disposable derived index
outside the hash domain (git-index class): the checkpoint.

- **Identity tuple:** `(workspace_uuid, domain_version, tree_root,
  journal_instance, journal_seq, parse-cache generation)`. Any field mismatch
  forces exactly one loud, labeled re-baseline. `domain_version` stops a
  checkpoint outliving a hash-law change (the §4.2 cutover changes the interior
  encoding); the journal cursor pair is the replay point.
- **Restart replay** needs a journal that qualifies for this gap: (1) coverage
  complete, gap start to end; (2) the instrument carries a loss signal and
  raised none — silence from an instrument without loss signaling qualifies
  nothing; (3) per-file granularity; (4) its own contract treats a clean,
  loss-signal-free window as definitive, so an instrument whose docs direct
  rescans regardless fails. Then one journaled change = one file read and
  hashed, zero unchanged members statted, no cold rebuild. One instrument
  qualifies today: the live §6.4 watcher across an engine-cold, process-alive
  gap with no overflow raised. No persisted instrument known qualifies:
  FSEvents fails (2) and (4) by Apple's own guide, btrfs/ZFS fail portability
  and enumeration. An instrument qualifies by meeting these four conditions,
  never by being called a journal.
- **Where no qualifying journal covers the gap** (today, every process death) a
  sound checkpoint restores every row **untrusted**: no row serves, and no
  answer derives from one, until one §6.2-governed pass has covered the full
  member set — a pre-serve barrier, so lazy, deferred or post-first-serve
  verification is refused. The pass is one stat per member as the floor (the
  160 ms figure at 29.7k members measures this stat pass alone) plus the
  watermark law's re-reads. A row racily clean at save — recorded mtime within
  one calibrated granularity unit of the checkpoint's saved watermark — is
  re-read or restored pre-spoiled, never trusted on stat-match, under §6.2's
  identity-fence and suspect rules. The re-read count is published; rows
  outside the window cost zero bytes. Counter equation: reads = hashes = movers +
  watermark-window re-reads; stats = member count, once, before first serve.
- **Parses are not gated by this law:** the checkpoint carries leaf digests and
  the tree, never parsed documents, so those counters and the 160 ms govern the
  resident-tree restore (the guard/currency plane). The separate disposable
  document cache (§6.9) removes unchanged-document parsing; it does not remove
  this barrier.
- A soundness mismatch forces one loud, labeled cold re-baseline. A cursor that
  cannot anchor forces one labeled warm re-baseline: replay is forfeited, the
  object retained. The residual stat term remains: requirement 1 is only
  partially satisfied at restart, never on the warm path.
- **Markdown stays the sole truth, always.** The §0 ban on trusted snapshots
  stands: this object is allowed only while loud-discard and identity-binding
  hold.
- **Format ordering:** the checkpoint format is downstream of the §4.2
  encoding — never persist what the next step replaces.
- **Storage site:** one file per workspace in its cache drawer (beside
  `sql.duckdb` and the run plane's digest memo), written atomically, read whole.
  The drawer is outside every hash domain — the §6.4 cookie's floor — so the
  checkpoint cannot move a root or break a held token.
- **Two questions, two instruments.** A checkpoint records the past, never the
  present, so its trust splits in two. *Lawfulness* (may these rows enter as
  hypotheses?) is decided once at restore by the soundness fields —
  `workspace_uuid`, `domain_version`, the hash law, parse-cache generation and
  the `tree_root` binding, never the journal cursor pair. A mismatch discards
  the object whole, loudly, labeled per field: the **cold** re-baseline.
  *Currency* (may a row serve?) the checkpoint never answers, and never
  wholesale: only the §6.2 trust close does, row by row.
- **The journal pair is neither — it is a cursor** (§6.3's landed cursor law),
  never an identity field. A cursor that anchors buys replay; one that cannot
  anchor forfeits replay only: the object is kept and its rows still enter as
  hypotheses — the **warm** re-baseline, which rebuilds currency from zero
  trust, not zero bytes. The warm event fires on **every** ordinary process
  restart, since `wire-contract.md` §7.1 persists no epoch fact; the git-index
  class discards nothing at a gap, it re-verifies by stat.

### 6.6 Fingerprint history ring

`diff(from_fingerprint, to_fingerprint)` needs the frames behind old
fingerprints; clients hold only the token. The daemon keeps a bounded in-memory
ring of recent `DeltaFrame`s (`RootRing`, `wire-serve/src/ring.rs`); `diff`
replays between two roots, and a fingerprint outside the ring answers
`fingerprint_unknown` → full resync (re-derive, never wrong data). Detection is
event-fed, with the poll surviving as fallback clock; frames and the ring's
bound are unchanged. At the §4.2.5 cutover the ring re-baselines at a labeled
epoch boundary, never a silent chain break.

### 6.7 The serve-path currency consumers — one instrument, vouch first

**Motive, measured.** A consumer left on the §6.2 floor unconditionally burns
~66–85 CPU-seconds per 60 wall-seconds on a 37.8k-member corpus, nearly all of
it floor passes — the domain walk plus one `lstat` per member.

> **The law.** Every standing currency question in the daemon is answered by
> the workspace's one resident memo (§6.1) through the §6.4 vouch, at the grade
> the question needs. The §6.2 extent-refresh floor answers only a named miss —
> no live feed, cookie `Unproven`/`Refused`, a doubt collapse, an untrusted
> memo, no baseline — and never on a timer against a healthy feed. A stat
> signature or other evidence-grade instrument may gate pure-latency work (the
> prewarm's standing license), never the content root: a served answer is
> always stamped with a root folded from content digests — the overlay's
> read-and-hashed leaves, or the floor's.

Two grades, bound to their consumers:

| Consumer | Question | Grade | Instrument |
|---|---|---|---|
| warm read pass — read family, `sql`, `script` entry (`Registry::warm_or_build`, cheap half) | engine current now? | current-as-of-the-question | cookie barrier → take-and-apply → `Trusted` → overlay fold (`Registry::currency_refresh`); floor on a named miss. The barrier makes the stamp current for the ambient premise tokens a read mints (`wire-contract.md` §5.4 and its §2 mint law); it costs one sentinel write plus delivery, bounded by the door cookie budget |
| `wire-contract.md` § A.11 post-result `live` | did the corpus move past the rows? | current-as-of-the-question | same call, same vouch |
| write door `root_before` | §6.1 door-entry observation | guard | `Registry::door_observation`, unchanged |
| prewarm quiet check | may this sweep be skipped? | latency-only | O(1), no cookie: nothing pending after take-and-apply, memo `Trusted`, cached served fold == the engine's stamp. `domain_stat_signature` walk survives only with no live feed (`FeedSlot::Failed`), under the quiet backoff |
| `wire-contract.md` §4.7 detect pre-check (`WorkspaceRing::detect`) | root moved since baseline? | latency-only + fallback clock | the same O(1) quiet check through the shared memo; the private fold memo serves `prime` and the miss path only. §6.6's poll survives: even under a quiet vouch the floor pre-check runs once per `DETECT_FLOOR_CADENCE` (30 s) — the push plane's backstop against silent capture loss |

**The cookie holdoff (posture, both doors).** A `CookieTimeout` collapse is
sticky doubt until the next take, which converts it to a full sweep and clears
it. Ruled posture: the barrier answers `Unproven` at once for `COOKIE_HOLDOFF`
(60 s) and one probe per window re-tests the stream, so a dead watcher costs at
most one timeout per window per workspace.

**Foreign domain-config edits collapse the vouch.** `meridian/domain.md`
governs membership and version, and the overlay fold serves both from the last
observation (`domain_seen`). The feed apply therefore escalates a dirty path
equal to `fs::domain::DOMAIN_CONFIG_PATH` to the Sweep
rung (§6.4 ladder: memo kept, loss noted, next observation the full walk under
the fresh config). The governed write path never pays it: its own-config
commit imposes `overlay_membership` synchronously (§6.1).

**The watch plane classifies off the resident memo** — the §6.6 direction,
frame-parity gated. The alternative — one full corpus snapshot of every
member's bytes per external batch — measured ~1/s under foreign writes.

- On a moved root the cycle takes the workspace flock, makes the §6.1
  door-grade observation through the shared memo, and hands the classifier that
  leaf set and root.
- The classifier diffs those digests against the watcher's retained baseline
  (entries carry a leaf digest beside their bytes), reads bytes only for
  movers, and mints the same frames. Renames pair by digest (byte-equality's
  proxy); removed and `unattested` rows parse retained baseline bytes; modified
  rows diff retained-old against read-new.
- A mover whose re-read digest disagrees with the observed leaf is a mid-cycle
  race: the cycle emits nothing and holds its baseline; the racing write's event
  re-fires detection.
- The frame's `root_after` is the memo's root — what the read plane stamps, so
  push and read planes cannot disagree.
- Priming (the subscribe-time baseline) keeps its one full snapshot. The ring
  holds no private fold memo, and a plain mutex keeps the single-flight gate.
- Non-UTF-8-**named** members stay baseline-invisible as the snapshot kept
  them (their leaves still fold; a `wire::Path` cannot spell them); non-UTF-8
  **content** classifies normally.

**What does not change.** The floor pass — walk semantics, §6.2 trust close,
refusal shapes — is untouched. `Reused` keeps its zero-parse proof; a rebuild is
still `fs::update_corpus` against the memo's leaf set. Run-plane bracket
observations (guard grade, locked-window law) keep their live floors.

### 6.8 The absorb path — deriving the answer at the cost of the change

**Motive, measured.** On 37.8k members over 15 s at 2 foreign writes/s:
~0.25 CPU-s per absorbed change, ~26.6 CPU-s per 60 s — two flat rebuilds
(overlay fold, rebuild tail fold) plus carried-document clones.

> **The law.** The resident tree is the serving instrument. The served workspace
> root derives from the one memo's incrementally maintained fold (§6.1) — never
> from a second tree rebuilt over the leaf set the memo already carries. An
> incremental pass carries an unmoved member's parsed document by shared
> reference, never by copy, and folds nothing when its built leaf set is
> byte-equal to its snapshot's — its stamp is then the snapshot's own root,
> taken with the snapshot under one lock. On the serve path the flat build
> over a leaf set survives in exactly three roles: the cold observation
> (no resident state yet), the divergence tail of an incremental pass (a mover
> vanished or changed since the snapshot, so the pass folds what it built),
> and the equivalence gate's oracle. Run-plane bracket observations stay out of
> scope (§6.7).

Gates:

- **§4.2.1 purity is the equivalence.** The canonical trie shape is a pure
  function of the entry set, so the maintained fold and a fresh build over the
  same leaves cannot differ; the property gate (op history vs fresh build, §4.4
  collision keys included) proves it.
- **Lockstep, gated at the memo grain.** Leaf memo and resident tree advance in
  one guarded act at every mutation site: the observation generation, the
  own-write overlay (leaf, remove, membership), the §6.5 restore (which checks
  the rebuilt tree against the stored `tree-root` before adopting a row). The
  absorb-path gate asserts served fold == flat oracle per mutation class.
- **One instrument, one truth.** The floor pass keeps its §6.2 walk, stat and
  read semantics; its fold is the resident fold.
- **The §6.3 audit edge is untouched.** Stamps stay on the guarded write path;
  only the source of the served value moves.
- **The stamp law holds** (§6.7): the resident fold is the fold of the memo's
  content digests, and the fold-free rebuild stamp rides input-equality — by
  purity, byte-equal inputs fold to the same value.
- **Sharing changes ownership, never content.** A parsed document is immutable
  once built (`model` law: derived, disposable): a shared reference changes who
  frees it, and a rebuild allocates movers only.

The fold counter keeps its semantics — zero on a quiet vouched pass, one per
advance, counting served-fold recomputes — and now runs O(dirty vertices),
never O(corpus).

### 6.9 Durable document reuse

A process restart or resident-budget eviction loses ownership of parsed
documents, not their content identity. The engine may persist a disposable,
per-workspace document cache outside the hash domain. Markdown remains the
sole authority. Cache absence, contention, incompatibility, corruption, or a
failed save changes cost only; none can make a workspace fail or serve stale
bytes.

**Three independent objects.** The §6.5 observation checkpoint establishes
file identities and leaf digests under the normal pre-serve barrier. The
document cache maps a content digest to the immutable document derived from
those bytes (or its invalid-UTF-8 condition). The SQL projection is a separate
consumer. None is reconstructed from another plane's lossy projection.

**Compatibility.** The document format and semantic parser generation are
independent of the daemon build SHA, package release number, and SQL schema
salt. The semantic generation is derived at build time from the parser,
governed-model, address, and codec sources and their locked dependency set.
A changed input conservatively invalidates parse reuse; the covered crates'
full Rust sources and manifests are hashed, so even a comment or other
metadata-only edit in those inputs may invalidate reuse. An unrelated daemon
implementation change does not. The cache uses a separate `parsed-v1`
drawer under the existing workspace bucket, with the same registration,
locking, last-use, clean, and GC rules as other drawers.

**Restore.** First establish current membership and digests through the
existing currency instrument. Then decode only cache entries whose digest
is wanted by that current leaf set. Check the format/generation, record
checksum, raw-content digest, UTF-8/span bounds, tree depth, and node revs.
Malformed records are misses. A broken framing boundary ends adoption at
that boundary; already verified records remain usable. Bound all allocations
by the available record bytes and a cache-entry size ceiling. Large documents
that cannot be cached still parse and serve normally.

Each record's checksum includes the semantic generation, type, content digest,
and payload, so a verified prefix cannot adopt an old-generation record under
a different header. The generation and checksum bind the derived representation produced by this
engine; the cache is private local derived data, not an authenticated remote
input. Restoring does not re-run the parser to prove every semantic field.

Map verified objects onto current paths, including renamed or duplicate
content, and pass this transient prior corpus to the SAME `fs::update_corpus`
used by resident rebuilds. Only misses and movers read/parse source. Rebuild
the corpus name index from the final documents; never deserialize an index,
persist an epoch cursor as authority, or retain a second decoded corpus.
The existing witness check controls publication, so an older concurrent
build cannot replace a newer engine. `WarmOutcome::Built.docs` continues to
count actual parses, including zero when a cold engine restores completely.

**Storage and saving.** A streaming, checksummed snapshot keeps restore to
one sequential file instead of one file per document. Entries are keyed by
content digest and deduplicated within the workspace. Temporary writes are
atomically replaced; a torn temporary file is never a restore candidate.
Encode one document at a time so the snapshot size is not also a transient
RAM allocation. A first completed cold build, eviction, and graceful shutdown
are save opportunities. Skip a snapshot already saved at the same engine
fingerprint. Do not periodically rewrite the whole parsed corpus for routine
edits. An older snapshot is useful: the verified leaf set selects its unchanged
members and ordinary incremental reconciliation absorbs later edits.

The smaller observation checkpoint may be coalesced in the background, only
when dirty. Capture its journal cursor before its consistent memo snapshot;
encode and write after releasing the memo and registry map locks. Background
saves skip contended state. All cache writes are best effort and observable;
a failed save never prevents eviction or publication. Persistence I/O must
not run under a lock that a hook fire or corpus lookup needs.

Acceptance covers unchanged restart (zero parses), one mover, add/remove/
rename, duplicate content, invalid UTF-8, incompatible generations, corrupt
records, truncated saves, unavailable storage, concurrent publication, eviction,
and cache GC. A representative corpus measurement must publish encoded size,
encode/decode cost, and save memory behavior before relying on this format
for a large workspace.

## 7. Integrity surface + CAS — the grain ladder

One instrument serves every grain: the resident tree (§6). A **premise** is
what a guard claims — this scope still holds this token — and the engine
re-checks it before the write lands. Every addressable node is a legal
premise; each row below is one grain, with the refusal it gives. Coverage
sufficiency, field spellings and guard requiredness are wire-side law
(`wire-contract.md` §5.3–§5.4).
**No separate `guard` op** — integrity = mint + premise + `diff`.

| Premise / op | Grain | Question | Failure |
|---|---|---|---|
| `if_node_rev` (on `splice`) | one node | is this section still what I read? | `cas_mismatch` {expected, actual} — re-read, re-plan |
| `if_node_rev` on an `fm_key` target (`prop_rev`, §2.1) | one frontmatter key | did this key move? | `cas_mismatch` at key grain |
| **scoped fingerprint** `{scope, fingerprint}` | any PATH node: root, folder, file leaf, or `absent` | is this subtree still what I planned against? | `fingerprint_mismatch` {expected, actual, scope} |
| **forest fold** (pattern/selector/sql-provenance premise, §4.3.1) | a derived match set | is the set I derived still exactly this? | `fingerprint_mismatch` naming the set premise |
| workspace token (root scope — `if_fingerprint`) | the world | is the world still what I planned against? | `fingerprint_mismatch` — resync |
| `fingerprint {scope}` op | any PATH node | mint the current token at a scope (root default) | `scope_unresolved` |
| `diff` | range of fingerprints | Delta batches between two cursors | `fingerprint_unknown` outside retained history |

**Scope rows, their law:**

- **`absent` is a value, not an error.** A lawful path with no node mints the
  reserved non-hex spelling `absent`; so does a path whose prefix is gone
  (`a/b/c` with `a/` missing), which creation guards plan on. `scope_unresolved`
  (fix class) is refused only where a path escapes the root, conflicts in kind
  with an existing prefix entry, or names a §4.4 collision.
- **Raw-byte names are addressable** through the `scope_bytes` arm (base64url
  raw segments) beside UTF-8 `scope`; mint and guard serve both (§9;
  `wire-contract.md` §5.4).
- **Guard-path freshness:** at check time the engine refreshes the premise's
  extent through the watermarked memo (§6.2), so cost follows that extent.
  Fast path: the stamp compare (§6.3); extent refresh is the floor.
- **Radix vertices hold no scope** (§4.3): nothing below a path node is
  addressable, so the ladder bottoms out at the file leaf and `absent`.

**`fingerprint` op** — bare mints the world cursor; scoped form per
`wire-contract.md` §5.4. Under `scoped-guards`:

```jsonc
→ {"id":7,"op":"fingerprint"}
← {"id":7,"ok":true,"body":{"fingerprint":"b3:807b69c6…","seq":N}}
→ {"id":8,"op":"fingerprint","scope":"a/target.md"}
← {"id":8,"ok":true,"body":{"fingerprint":"b3:…","seq":N,"scope":"a/target.md"}}
```

A lawful empty path answers `fingerprint: "absent"` and still echoes the scope
pair. `scope` with `scope_bytes` refuses `bad_request`.

**Three errors, three facts, never flattened:**

| refusal family | the fact | the recovery |
|---|---|---|
| `fingerprint_mismatch` {expected, actual, scope} | the premise moved | re-read that scope, re-plan |
| `scope_unresolved` | the premise cannot be evaluated there | fix the path |
| cursor family: `fingerprint_unknown`, dead instance, **`fingerprint_version_retired`** | too old: a seq past the ring, a reaped instance, a retired hash law (§4.2.5) | re-derive and resume, with the resident tree a scope-fold compare, never a full relist; on `fingerprint_version_retired` the law moved, not the premise: re-mint |

**Ordering on one write:** widest first — root token, folder scopes, then
per-edit `if_node_rev`; a failing wider premise skips narrower work.

**What each layer never does:** the engine never decides *when* a guard is
required (host policy — `wire-contract.md` §5.3); hosts never compute hashes
(node_rev and fingerprint are opaque equality tokens).

**Consistency with the three laws:** disk stays the only durable truth (memo,
ring and tree are memory; the checkpoint is disposable, §6.5); recovery is
re-derive; the engine answers "what changed", policy decides what to do about
it.

## 8. Interaction with the write plane

- `splice`: optional `if_fingerprint`; transition fields per
  `wire-contract.md` §4.4; `fingerprint_after` is the own-write overlay's fold
  (§6.1), never a re-read. `scope`, `guards[]` and `scope_bytes` are
  capability-advertised; un-negotiated use by a frozen v2 session refuses
  `bad_request` (`wire-contract.md` §5.4).
- Node objects and `resolve`: node_rev (§1–2) is the hash law for CAS tokens.
- Caps advertise `fingerprint`, `diff`, `splice.if_fingerprint` and related,
  not a `guard` op. Errors: `fingerprint_mismatch`, `fingerprint_unknown`,
  `scope_unresolved`, `fingerprint_version_retired` (not `root_*`); §7 splits
  them three ways.
- Routine writes route through the daemon: the CLI rides its resident tree
  over IPC; `LOCK_EX` on `write.lock` is takeover and recovery only.
  Lease, intents, parallel disjoint commits and the plain-fsync class are
  authority-contract law; publication (step 6: reservation algebra,
  checksummed `O_EXCL` intents, the one state owner's contiguous root chain)
  is `crates/wire-serve/src/publish.rs`, disk primitives
  `crates/fs/src/intent.rs`, lease half `authority.rs`. The live write door
  keeps its interim flock until the cutover flips routing. `apply_batch`'s
  pre-image verify is the in-process second-writer refusal under daemon
  routing (`docs/laws.md` Amendment).
- The effects lane (`run`, script-with-effects) is unguarded by rule
  (`run-plane.md` holds the paragraph), yet every effects write rides the same
  write choke-point and maintains the resident tree (leaf update, chain
  refold, §6.2 watermark) — maintenance, not a guard.
- **File death mints no terminal hash — the death Delta is the record.** A
  guarded `remove` (`wire-contract.md` § A.3) unlinks the leaf; the next fold
  composes the tree without it under the §4 encoding, and law 2's child map
  re-canonicalizes as if the entry never existed (§4.2.2). The terminal facts
  are the removed file's last rev (`file_rev_before`, confirmed by the
  remove-what-you-read CAS) and the workspace fingerprint transition, both
  carried by the death Delta (`change:"deleted"`, `wire-contract.md` §7.1).
  No tombstone leaf or on-disk marker: disk stays markdown only, and history
  past the ring re-derives to a world where the path is absent (§7).

## 9. Normalization rulings (closed for v1)

- **Names: raw bytes, byte-order sort, no unicode normalization.** The
  fingerprint never crosses hosts, so NFD/NFC divergence cannot bite it.
  Revisit only if fingerprints travel between machines: then NFC at hash time,
  flagged proto-visible.
- **Name truthfulness:** every fold carries the raw on-disk name bytes end to
  end. Conversion is legal only in display layers (error prose, listings), and
  only where two-way convertible with zero loss for the servable set. The
  display form for a name with no UTF-8 spelling is escaped: `\xNN` for
  invalid bytes, `\\` for a literal backslash.
- **Non-UTF-8 names: hashed truthfully, addressable via `scope_bytes`,
  unservable on the UTF-8 read faces.** Such a member's leaf enters the root
  with the exact name bytes; mint and guard take the raw-byte arm
  (`scope_bytes`, §7).
  Wire paths are JSON strings: the member serves no spans, the serving
  snapshot (`DomainFiles`) holds only UTF-8-named members, and a watch delta
  cannot name it — frame fingerprints stay truthful and §6/§7 resync covers
  the gap. macOS refuses such names (errno 92); Linux is
  the reachable platform.
- **Symlinks: skipped silently** — the addressing jail confines them; an
  in-tree target is hashed at its real path. Accepted cost: retargeting one
  does not move the fingerprint.
- **Content: raw bytes always** (§2, §3). CRLF, trailing whitespace and BOM
  all hash as written.
- **File/dir name collisions: both kinds hashed.** Law 2 hashes both (§4.4 —
  nothing sits outside the fold), lints the build loudly, and refuses
  `scope_unresolved` at mint and guard on the colliding path. Raw name bytes
  are the trie's key bytes; byte-order sort is slot and forest ordering
  (§4.2.2, §4.3.1).

## 10. Open questions for architecture review

1. **node_rev width** — 16 hex (64-bit) per §1; the contract's 6-hex examples
   are non-normative. The `resolve` freeze amendment should state it —
   objection to 16?
2. **Ring bound** — **256 roots**, `wire-contract.md` §13; older ranges answer
   `fingerprint_unknown` → full resync, never wrong data.
3. **Hash domain** — `wire-contract.md` §12 (md-only + `meridian/domain.md`);
   the leaf rule here must stay aligned with that domain filter.
4. **Symlink retarget invisibility** (§9) — accept, or hash the target path as
   a pseudo-leaf?
5. **Multi-file atomic batch** — limit in `wire-contract.md` §6.5; vocabulary
   is `if_fingerprint` + batch `splice`.
6. **`diff` payload cap** — a stale fingerprint can name thousands of paths:
   cap + `truncated:true`, or `fingerprint_unknown` past a threshold?
   (`wire-contract.md` §18 row 2 struck the mismatch `changed` field; only the
   `diff` half is open.)
