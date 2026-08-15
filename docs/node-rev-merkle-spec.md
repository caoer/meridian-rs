---
type: spec
id: merkle
status: standing
updated: 2026-08-08
description: Normative hash law for `node_rev` and the workspace merkle fingerprint, with worked examples.
owns: [node_rev, merkle encoding]
---

# node_rev + workspace fingerprint (merkle) spec

> **Standing:** Design law is `wire-contract.md` (one contract). Mint addresses = segments only. Receipts = armed wire facts. DuckDB/`view_path` not agent core. **Doc correct > code correct; docs first.** See `README.md`.

**Scope note:** this document is **node_rev and workspace merkle (fingerprint) hash law**. It does not define section address grammar; mint-plane hpath remains segment form only. Worked-example generator assets stay under `node-rev-merkle-spec.assets/`.

The hash scheme behind the wire nouns `node_rev` and **`fingerprint`** (workspace content hash — design noun **`fingerprint`**; the wire's default v2 vocabulary spells the field `root`, re-keyed to `fingerprint` once a client negotiates `contract:"v3"` — `wire-contract.md` §1). Binds: what bytes are hashed, tree composition to the 32-byte workspace fingerprint, and incremental update on `splice`. Integrity surface on the wire is **`fingerprint` + `if_fingerprint` + `diff`** — there is no separate `guard` op (`wire-contract.md` §4.7). Under the three laws: no snapshot files, no second database, Rust memory disposable.

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
- **Measured envelope** (M4 Max): session dir 4,141 nodes → root 74ms; whole corpus 9.5GB/50,319 nodes → 2.2s warm / 5.2s cold. These numbers justify §6's no-persistence stance.

**Rejected (with reasons):**
- **Snapshot persistence** (a `Save`/`Load` binary format, requiring the snapshot to live OUTSIDE the hashed dir). That design served an offline-subscriber demo. In meridian-rs it violates law 2 (Rust memory is disposable; disk = markdown only — "no snapshot files") — and the recovery numbers make it unnecessary: cold rebuild is 2.2s, "daemon death is a blip". The subscriber holds a 32-byte root, never a tree; the daemon holds no trees — the root re-derives on demand (§6–7 re-derive the drift-naming capability the snapshot existed for).
- **xxhash64 width.** 64-bit is a race detector, not collision-resistant; the vision's cursor is 32 bytes (law 2) and the wire example is `b3:`-prefixed. §1 rules blake3-256.
- **mtime+size leaf cache** — rejected for v1: its lie window (same mtime+size, different bytes) buys warm-rebuild speed we don't need once the tree is memory-resident and event-updated (§6). Cold start eats the 2–5s honestly.

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

## 5. Worked example (real values)

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
>   | nc -U ~/.cache/meridian/registry/daemon.sock
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

(All values computed by a reference implementation of this spec, blake3-256, 2026-07-18; the generator is `node-rev-merkle-spec.assets/worked-example-gen.go` — 127 lines of Go, with `node-rev-merkle-spec.assets/go.mod` — and should land as the fixture seed for the rung-3 test suite.)

## 6. Incremental update on splice — memory-only, event-fed

The daemon holds no merkle tree in memory. The workspace root is derived on demand through `fs::DomainCache` — a `StatKey`-keyed leaf-digest memo plus remembered directory listings — by a full stat sweep and a full fold (`merkle_root_of_leaves`) on every currency pass. On a successful `splice(path, …)`:

1. The splice writes the new bytes to disk under the write flock (`wire_serve::write::splice`); it hands no leaf to any tree.
2. The next currency pass re-reads the moved file (identity changed → re-read → re-hash) and re-folds: one `stat` per domain member, O(changed) bytes read, then all entries re-encoded (uleb128‖name‖type‖hash) into one buffer and one blake3 — O(corpus) in stats, O(changed) in bytes.
3. The splice response carries ambient **`fingerprint_before` / `fingerprint_after`** (and related fields per `wire-contract.md` §4.4) — the caller's next world cursor.

Out-of-band changes (hand edits, git operations) arrive via the watch feed → the next currency pass re-reads and re-hashes the moved members. Watch latency is a freshness window, not a correctness hole: recovery (§7 below / `wire-contract.md` §4.7 `diff`) catches anything missed, and splice re-reads under the write flock before writing, so CAS never trusts the memo alone — the memo serves integrity reads, never silent write authority.

**No persistence.** Nothing is persisted to disk — no snapshot files, no on-disk cache. Daemon start = full rebuild (measured 2.2–5.2s corpus-wide, §0). The inherited snapshot format is dropped here (§0 rejected).

**Fingerprint history ring.** `diff(from_fingerprint, to_fingerprint)` needs the frames behind old fingerprints; clients hold only the token. The daemon keeps a bounded in-memory ring of recent `DeltaFrame`s (`RootRing`, `wire-serve/src/ring.rs`), which `diff` replays between two roots. A fingerprint not in the ring answers `fingerprint_unknown` → full resync — re-derive, never wrong data.

## 7. Integrity surface + CAS (wire vocabulary)

Two grains, composable — **no separate `guard` op** (dropped; integrity = `fingerprint` + `if_fingerprint` + `diff`):

| Guard / op | Grain | Question | Failure |
|---|---|---|---|
| `if_node_rev` (on `splice`) | one node | "is THIS section still what I read?" | `cas_mismatch` {expected, actual} — refresh: re-read, re-plan |
| `if_fingerprint` (on `splice`, optional) | workspace | "is the WORLD still what I planned against?" | `fingerprint_mismatch` {expected, actual} — resync / re-plan |
| `fingerprint` op | workspace | read the current content-hash cursor | — |
| `diff` | range of fingerprints | batches of Delta between two cursors | `fingerprint_unknown` if outside retained history |

**`fingerprint` op** — read the current cursor:

```jsonc
→ {"id":7,"op":"fingerprint"}
← {"id":7,"ok":true,"body":{"fingerprint":"b3:807b69c6…","seq":N}}
```

**Ordering when both guards ride one splice:** `if_fingerprint` first (cheapest world check, fails the whole batch), then per-edit `if_node_rev`. A failing world guard skips node work.

**What each layer never does:** the engine never decides *when* a guard is **required** (host policy / geography — `wire-contract.md` §5.3); hosts never compute hashes (node_rev / fingerprint are opaque equality tokens). Multi-file all-or-nothing commit remains a named limit (`wire-contract.md` §6.5).

**Consistency with the three laws:** disk stays the only durable truth (memo and ring are memory); recovery is re-derive; the engine answers “what changed”, policy decides what to do about it.

## 8. Interaction with the write plane

- `splice` request: optional `if_fingerprint`. Response: fingerprint transition fields per `wire-contract.md` §4.4.
- Node objects / `resolve`: node_rev algorithm (§1–2) is the hash law for CAS tokens.
- Caps advertise `fingerprint`, `diff`, `splice.if_fingerprint` (and related) — not a `guard` op.
- Error codes: `fingerprint_mismatch`, `fingerprint_unknown` (not `root_*`).
- **File death mints no terminal hash (RULED, ZT 2026-08-15, card `engine-delete-door`: "No tombstone — death Delta is the record").** A guarded `remove` (`wire-contract.md` § A.3) unlinks the leaf; the next fold composes the tree without it under the existing §4 encoding — removal is already in the diff shape (§0, "whole-subtree enumeration on add/remove") and no new hash law exists for it. A rev is a function of bytes (§2); absent bytes mint nothing: the death's terminal facts are the removed file's LAST rev (`file_rev_before`, confirmed by the remove-what-you-read CAS) and the workspace fingerprint transition, both carried by the death Delta (`change:"deleted"`, `wire-contract.md` §7.1). No tombstone leaf, no on-disk marker — disk stays markdown only, and history past the ring re-derives to a world where the path is simply absent.

## 9. Normalization rulings (closed for v1)

- **Names: raw bytes, byte-order sort, no unicode normalization.** The fingerprint is a per-host daemon↔client cursor, never a cross-host sync token — the NFD/NFC divergence (macOS vs Linux) cannot bite a cursor that never crosses hosts. Revisit only if fingerprints ever travel between machines (then: NFC at hash time, flagged as proto-visible).
- **Name truthfulness (RULED, ZT 2026-08-08, "6b"):** *"we don't have conversion. the content for markle hashing is always truthful. if we have conversion, that's a flaw. if we are talking about per session display layer conversion, such layer is not possible to drift. in defined snapshot, the conversion is two way convertible with zero lose."* (verbatim, unparaphrased). Operationally: every fold — the workspace fingerprint, the exec-guard bracket folds, any derivation that claims to be the corpus root — carries the raw on-disk name bytes end to end. Conversion is legal only in display layers (error prose, listings), and there only where it is two-way convertible with zero loss for the servable set; an escaped rendering (`\xNN` for invalid bytes, `\\` for a literal backslash) is the display form for a name that has no UTF-8 spelling.
- **Non-UTF-8 names: hashed truthfully, unservable — the §3 analog for names.** A domain member whose NAME is not valid UTF-8 still gets its leaf and enters the root with its exact name bytes (blake3 and the interior encoding need no UTF-8). It cannot be SERVED: wire paths are JSON strings (UTF-8 by construction), and no injective UTF-8 spelling exists that also keeps every valid name fixed — so such a member is integrity-covered but unaddressable, exactly as a non-UTF-8-CONTENT file is integrity-covered but serves no spans. Stated limits that follow: the serving snapshot (`DomainFiles`) carries only the UTF-8-named members, and a watch delta cannot name such a path — the frame's fingerprints stay truthful and the §6/§7 resync law covers what the delta cannot spell. Reachability: macOS refuses creating such names (errno 92); Linux is the reachable platform.
- **Symlinks: skipped silently** — consistent with the addressing jail's stance that symlinks are resolved and confined at the addressing layer; a symlink's target, if in-tree, is hashed at its real path. Cost: retargeting an in-tree symlink alone doesn't move the fingerprint — accepted for now, listed below.
- **Content: raw bytes always** (§2, §3). CRLF, trailing whitespace, BOM — all hash as written.

## 10. Open questions for architecture review

1. **node_rev width** — 16 hex (64-bit) per §1, vs the contract examples' 6. Examples are non-normative, but the amendment that freezes `resolve` should state the width; any objection to 16?
2. **Ring bound** — settled at **256 roots** in `wire-contract.md` §13: older ranges answer `fingerprint_unknown` → full resync — re-derive, never wrong data.
3. **Hash domain** — settled for design in `wire-contract.md` §12 (md-only + `meridian/domain.md`). This spec’s leaf rule must stay aligned with that domain filter.
4. **Symlink retarget invisibility** (§9) — acceptable for now, or hash the target path as a pseudo-leaf?
5. **Multi-file atomic batch** — limit stated in `wire-contract.md` §6.5; vocabulary is `if_fingerprint` + batch `splice`.
6. **`diff` payload cap** — a wildly stale fingerprint can name thousands of paths; cap + `truncated:true`, or force `fingerprint_unknown` past a threshold? (The mismatch `changed` field was struck — `wire-contract.md` §18 row 2 — leaving only the `diff` half open.)

