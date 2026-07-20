---
type: result
status: spec
created: 2026-07-18
tags: [type/result, domain/ccc-mdfs, topic/meridian-rs, topic/merkle]
---

# node_rev + root spec — rung 3 (`root`, `guard`)

The hash scheme behind the wire nouns `node_rev` and `root` (wire contract §2), specified so rung 3 is implementation-only. Binds: what bytes are hashed, normalization, tree composition to the 32-byte root, incremental update on `splice`, and `guard` op semantics — all under the three laws (vision §2: no snapshot files, no second database, Rust memory disposable).

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
- **Snapshot persistence** (a `Save`/`Load` binary format, requiring the snapshot to live OUTSIDE the hashed dir). That design served an offline-subscriber demo. In meridian-rs it violates law 2 (Rust memory is disposable; disk = markdown only — "no snapshot files") — and the recovery numbers make it unnecessary: cold rebuild is 2.2s, "daemon death is a blip". The subscriber holds a 32-byte root, never a tree; the daemon holds trees in memory only (§6–7 re-derive the drift-naming capability the snapshot existed for).
- **xxhash64 width.** 64-bit is a race detector, not collision-resistant; the vision's cursor is 32 bytes (law 2) and the wire example is `b3:`-prefixed. §1 rules blake3-256.
- **mtime+size leaf cache** — rejected for v1: its lie window (same mtime+size, different bytes) buys warm-rebuild speed we don't need once the tree is memory-resident and event-updated (§6). Cold start eats the 2–5s honestly.

## 1. One hash family: BLAKE3-256

Every hash in this spec — node_rev, file leaf, interior, root — is BLAKE3 with 256-bit output. One primitive, one implementation, no mixed families.

- Chooses crypto-grade at xxhash-class speed (blake3 saturates memory bandwidth on these sizes), so the integrity cursor is adversary-proof for free, and the 32-byte root matches law 2's "32-byte cursor" verbatim.
- Wire spellings: `root` = `"b3:" + 64 lowercase hex chars` (algorithm-prefixed per §2 of the wire contract; the contract's `"b3:88d2aa"` examples are abbreviations, non-normative). `node_rev` = first **16 lowercase hex chars** (64 bits) of the node hash, unprefixed. Both remain opaque to clients — equality only.
- Why truncate node_rev but not root: node_rev is a CAS race detector scoped to one node's edit history — 64 bits gives collision-safety margins (~2³² revisions to birthday) at half the wire noise; the root is the fleet's integrity cursor and keeps full width. The frozen wire role ("opaque, equality only") means widening later is an amendment, not a break.

## 2. node_rev — what bytes are hashed

`node_rev = hex(blake3(node_span_bytes))[:16]` where `node_span_bytes = raw_file_bytes[span.start : span.end)` — the node's **span bytes exactly as issued** under the wire span laws (§2 laws 1–5: raw disk bytes, UTF-8-valid files only, block spans exclude the final line terminator).

- For a **section** (heading ref via `resolve`): the span is heading-inclusive (wire §6.1 — heading line through end of subtree), so `node_rev` covers the heading too. Consequence, deliberate: a heading rename invalidates the section's CAS token — a writer composing against `#Alpha` must notice `#Alpha` became `#Alpha2`. The content-only span (`content_span`, migration-map A2) is a write-target convenience and mints no separate rev.
- For **frontmatter**: the whole-block span (`---`…`---` inclusive, wire §5.2). Per-key value spans (migration-map A8) mint their own node_rev over the value bytes once that amendment lands.
- For any other node kind (`toc`/`extract` nodes with A1): its §5.2 span, verbatim.
- **No normalization of content.** No newline canonicalization, no trailing-space trim, no NFC. The span law already guarantees disk bytes = string bytes (UTF-8 refusal); hashing anything but the raw bytes would let two "equal" revisions denote different disk states — the exact corruption CAS exists to prevent.

## 3. File leaf hash — and why there is no per-file sub-merkle

`leaf(file) = blake3(raw_file_bytes)` — full 32 bytes, whole file, every regular file under the workspace root (not just `.md`; an asset drift must move the root too).

Section-grain leaves (the pre-wired seam of §0's inherited design) are **rejected for tree composition**: node spans index the raw file bytes, so the whole-file hash already changes iff any node's bytes change — a file-internal merkle adds hashing work and tree surface while buying only sub-file drift-naming, which the parser gives us for free (`toc`/`extract` diff, or `DiffSections`-style rev-table comparison at ~0.2ms/file). Node-grain integrity lives in `node_rev` (CAS, §2); file-and-above integrity lives in the tree (§4). Two grains, one boundary, no overlap.

Files that are **not valid UTF-8** still get leaf hashes (blake3 needs no UTF-8) and participate in the root; they simply serve no spans/nodes (wire `invalid_utf8` law). Integrity coverage and span service are independent properties.

## 4. Tree composition — leaves to the 32-byte root

The inherited scheme with blake3-256 in place of xxhash64:

```
interior(dir) = blake3( concat over children sorted by name-bytes:
                          varint(len(name)) ‖ name_bytes ‖ type_byte ‖ child_hash_32B )
type_byte: 0x00 = file, 0x01 = dir
```

- Children sorted by raw name bytes (§9 for the unicode caveat). Symlinks skipped (§9). Empty dirs pruned bottom-up (a dir whose children all pruned is itself pruned); the workspace root always exists.
- The workspace root's own name is not hashed (no parent to hold it).
- `root` (wire noun) = the workspace root's interior hash, spelled `b3:<64hex>`. A subtree root (per-file or per-dir scope, §7 `root` op) is that node's hash in the same spelling — a file-scope root is the file's leaf hash.

## 5. Worked example (real values)

Workspace: two entries. `notes.md` = `"# Notes\n\nhello\n"`; `tasks/x.md` (64 bytes) =

```
---\ntitle: demo\n---\n\n# Alpha\n\nbody line one\n\n## Beta\n\nbeta body\n
```

Node spans (wire laws — block spans exclude the final line terminator):

| node | span | node_rev = blake3(span bytes)[:16] |
|---|---|---|
| frontmatter | `[0,19)` (`---\ntitle: demo\n---`) | `22c54c415778475e` |
| section `#Alpha` (resolve, heading-inclusive) | `[21,64)` | `3d5903c3604ee3ac` |
| section `#Alpha/Beta` | `[45,64)` | `780d2fb4cf68f60f` |

Leaves (blake3 over whole raw file):

```
leaf(tasks/x.md) = 1e56548abcd43053053ef8f06b68c3261a7d29aa2a03aaa80b0a2f204d213d7e
leaf(notes.md)   = 96c26935d00a13398c39887a29adeb554d351b6863ec776c31d4a7f7f93f1875
```

Interior `tasks/` — pre-image is one child entry, 38 bytes: `04` (varint len 4) ‖ `x.md` ‖ `00` ‖ leaf:

```
pre-image: 04 78 2e 6d 64 00 1e56…3d7e
interior(tasks/) = f7a2e4b1af9ef2aa9d57abaa4375e6cff8c474c2f6dd788bc6a9d2543f0277fe
```

Workspace root — two entries sorted by name (`notes.md` < `tasks`), 81-byte pre-image `08‖notes.md‖00‖leaf ‖ 05‖tasks‖01‖interior`:

```
root = b3:807b69c693ad2c65e290422a1123198f22be6161c2caa43d71fab029fa4763cd
```

**Incremental update:** splice Beta's body `beta body\n` → `beta body v2\n`. Recompute exactly one path:

```
node_rev(#Alpha/Beta): 780d2fb4cf68f60f → f34813be3889438e
leaf(tasks/x.md)     : 1e56…3d7e → b78aa71202f4273e830ace6c7844b8943a53c04d1bab719586af2c3a307907ef
interior(tasks/)     : f7a2…77fe → 234267c9a1b642b751e50dabed092664a0013fce2c1b22738f6279ac99075a4f
root                 : b3:807b… → b3:a1f7bb8e46227d0c44df8c993fa1ab066b299d275d01d81e5dd6c40ba665b7c2
leaf(notes.md)       : unchanged (96c26935d00a1339…)
```

(All values computed by a reference implementation of this spec, blake3-256, 2026-07-18; the generator is `node-rev-merkle-spec.assets/worked-example-gen.go` — 90 lines of Go, with `node-rev-merkle-spec.assets/go.mod` — and should land as the fixture seed for the rung-3 test suite.)

## 6. Incremental update on splice — memory-only, event-fed

The daemon holds ONE current tree in Rust memory (the world model). On a successful `splice(path, …)`:

1. New file bytes are already in hand (the splice output) → new leaf = blake3(bytes). Cost: ~15µs for a 46KB file (blake3 ≥3GB/s).
2. Walk parent chain to the workspace root re-hashing interiors: O(depth) hashes, each over ~30–40 bytes × fanout. Session trees are ≤6 deep; total incremental root update is well under 1ms.
3. The splice response gains an additive `root` field (wire §6.2 sketch already reserves this) — the caller's next cursor.

Out-of-band changes (hand edits, git operations, the `md` CLI during transition) arrive via the watch feed → same per-file update path (re-read, re-leaf, re-chain). Watch latency is a freshness window, not a correctness hole: the recovery law (§7) catches anything missed, and splice re-reads the file under the sidecar flock before writing, so CAS never trusts the tree — the tree serves reads (`root`, `guard`, diff), never writes.

**No persistence.** The tree is never written to disk — no snapshot files, no cache. Daemon start = full rebuild (measured 2.2–5.2s corpus-wide, §0); per the vision, "the moment memory can't be thrown away, the architecture has been violated". The inherited snapshot format is dropped here (§0 rejected).

**Root history ring (the one new mechanism).** `diff(my_root, live_root)` — the recovery law — needs a tree BEHIND the old root, and clients hold only 32 bytes. So the daemon keeps a bounded in-memory ring of recent `(root, tree)` snapshots (structurally shared — an updated tree reuses every unchanged subtree node, so N snapshots cost O(changes), not O(N·tree)). Ring bounded by count/age (knob, §10). A root not in the ring is answerable only as "full resync" — which degrades to law 3's re-derive, never to wrong data.

## 7. `guard` semantics + CAS interaction (rung-3 op surface: `root`, `guard`)

Two guards, two grains, composable:

| Guard | Grain | Question | Failure |
|---|---|---|---|
| `if_node_rev` (on `splice`, rung 2) | one node | "is THIS section still what I read?" | `cas_mismatch` {expected, actual} — retryable: re-resolve, re-derive |
| `if_root` (on `splice`/batch, rung 3, additive field) | file / subtree / workspace | "is the WORLD still what I planned against?" (TODO-6: commit guards ONE thing) | `root_mismatch` {expected, actual, scope} — retryable: re-plan |

**`root` op** — read the current cursor:

```jsonc
→ {"id":7,"op":"root"}                      // workspace scope
← {"id":7,"ok":true,"root":"b3:807b69c6…"}
→ {"id":8,"op":"root","path":"tasks"}       // subtree scope; a file path yields its leaf
← {"id":8,"ok":true,"root":"b3:f7a2e4b1…"}
```

**`guard` op** — validate a held root and, on mismatch, name the drift (the inherited Diff, answered from the ring):

```jsonc
→ {"id":9,"op":"guard","root":"b3:807b69c6…"}
← {"id":9,"ok":true,"root":"b3:807b69c6…"}                       // match: 1 comparison, the fast path
← {"id":9,"ok":false,"error":"root_mismatch",
   "expected":"b3:807b69c6…","actual":"b3:a1f7bb8e…",
   "changed":[{"path":"tasks/x.md","kind":"modified"}]}          // O(changed) descent, ring-served
← {"id":9,"ok":false,"error":"root_unknown","message":"root evicted; full resync"}  // ring miss
```

- `changed` kinds: `modified` / `added` / `removed`; species change = remove+add; whole added/removed subtrees enumerate their files (the §0 Diff semantics). `path` optionally scopes a guard to a subtree.
- New error codes (amendment, per wire §4 additive rule): `root_mismatch` (retryable — re-plan), `root_unknown` (retryable only via full resync — treat as "cursor expired").
- **Ordering when both guards ride one splice:** `if_root` checked first (1 comparison, cheapest, fails the whole plan), then `if_node_rev` per edit. A passing root implies unchanged node_revs everywhere — the node check is then a free formality; a failing root skips all node work.
- **What each layer never does:** Rust never decides WHEN a guard is required (that's Go's rev-ladder policy, migration map §4); Go never computes hashes (node_rev/root are opaque — it compares equality only). The pipe txn's CAS-vs-T0 + DRY-all commit is re-expressed at rung 3 as: `guard(root@plan)` → batch `splice` with `if_root` — multi-file all-or-nothing shape is rung-3 detail, flagged in the migration map §8.

**Consistency with the three laws:** disk stays the only durable truth (law 2 — tree and ring are memory); recovery at every layer is re-derive (law 3 — cold start rebuilds, ring miss resyncs, events remain a latency optimization since `diff(my_root, live_root)` is always available); Rust answers "what changed", Go decides what to do about it (law 1).

## 8. Interaction with rung 2 (what rung 3 adds, nothing it changes)

- `splice` request: `if_root` (optional, additive). `splice` response: `root` (additive).
- Node objects/`resolve`: unchanged — node_rev algorithm (§1–2) is the amendment the wire contract §2 promised ("algorithm and length are a rung-2 decision").
- New ops `root`, `guard` appear in `hello.caps`.
- New error codes `root_mismatch`, `root_unknown`.

## 9. Normalization rulings (closed for v1)

- **Names: raw bytes, byte-order sort, no unicode normalization.** The root is a per-host daemon↔client cursor, never a cross-host sync token — the NFD/NFC divergence (macOS vs Linux) cannot bite a cursor that never crosses hosts. Revisit only if roots ever travel between machines (then: NFC at hash time, flagged as proto-visible).
- **Symlinks: skipped silently** — consistent with the statusd jail's stance that symlinks are resolved and confined at the addressing layer; a symlink's target, if in-tree, is hashed at its real path. Cost: retargeting an in-tree symlink alone doesn't move the root — accepted for v1, listed below.
- **Content: raw bytes always** (§2, §3). CRLF, trailing whitespace, BOM — all hash as written.

## 10. Open questions for architecture review

1. **node_rev width** — 16 hex (64-bit) per §1, vs the contract examples' 6. Examples are non-normative, but the amendment that freezes `resolve` should state the width; any objection to 16?
2. **Ring bound** — count (e.g. 64 roots) vs age (e.g. 10min) vs both. Determines how stale a subscriber can be and still catch up without a full resync. Cheap either way (structural sharing); needs a number.
3. **Root scope of non-watched files** — §3 says every regular file under the workspace root. Should ignore globs (`.git/`, `.ccc/events.ndjson` journals, `*.lock` sidecars) be excluded from the root? Recommend YES for the daemon's own write-side artifacts (journal/locks — otherwise every guarded write moves the root it just guarded) with the ignore set frozen in the contract amendment; needs sign-off on the exact set. **This is the one place §3's "every file" needs a carve-out — a root that self-invalidates on every splice is useless as a commit guard.**
4. **Symlink retarget invisibility** (§9) — acceptable for v1, or hash the target path as a pseudo-leaf?
5. **Multi-file atomic batch at rung 3** (pipe-txn successor) — same open question as migration map §8; the guard vocabulary here is designed to serve it (`if_root` + batch splice) but the wire shape is undecided.
6. **guard `changed` payload cap** — a wildly stale root can name thousands of paths; cap + `truncated:true`, or force `root_unknown` past a threshold? Recommend cap at 1000 with truncation flag.

