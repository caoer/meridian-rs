---
type: result
status: spec
created: 2026-07-24
tags: [type/result, domain/ccc-mdfs, topic/meridian-rs, topic/merkle, milestone-1]
---

# norm-v2 + fingerprint CID-token spec (M1 U-SPEC)

The hash-domain law for the **fingerprint plane** — the attestation content
identity that decision `2026-07-24-fingerprint-cid-representation` (ratified,
silver; "#4" below) rules. Binds: the self-describing fingerprint token
(#4 §1), the norm-v2 canonicalization rule set (#4 §4), the byte domains each
node kind hashes, and what this spec deliberately does NOT change. Golden
fixtures: `crates/model/tests/norm_v2_fixtures.rs` — the fixtures and this doc
are one artifact; a divergence is a defect in whichever moved last.

Sibling inputs: `2026-07-24-claim-link-grammar` (#6 — anchor promotion at pin,
rev-neutral by construction), `2026-07-24-meridian-lock-block` (#8 —
`meridian-lock` carries full-length fingerprints; lock-is-content). This spec
is written BEFORE any hashing code (hash-domain-defining, plan U-SPEC);
`crates/model/src/compose.rs` migrates to it in U10.

## 1. The three hash planes — one family, three domains

Every hash is BLAKE3-256 (node-rev-merkle-spec §1: one primitive, no mixed
families). The planes differ on **domain** and **job**, never on family:

| Plane | Byte domain | Normalization | Spelling | Job |
|---|---|---|---|---|
| `node_rev` | node span bytes (contract §1) | **none — raw** | bare 16 lowercase hex | CAS race detector (`if_node_rev`) |
| workspace merkle (`leaf`/`root`) | whole raw file bytes → tree | **none — raw** | `b3:` + 64 lowercase hex | guard/freshness cursor (`if_root`, `guard`) |
| **fingerprint** (this spec) | node span bytes | **norm-v2 (§4)** | CID-token (§2) | attestation content identity (pins, locks, receipts) |

`node_rev` and the workspace merkle are UNCHANGED by this spec — their laws
stay `node-rev-merkle-spec.md` §2–§4. The CAS plane hashes raw bytes on
purpose: an anchor promotion (#6 §2) genuinely moves bytes on disk, so a
concurrent writer's CAS token MUST invalidate (spans shifted), while the same
promotion must NOT move the fingerprint (no false drift — #4 §4). One physical
edit, two planes, opposite obligations — both fixtured (§5).

## 2. The fingerprint token

### 2.1 Grammar

One token, four dot-joined fields, all lowercase (#4 §1 `{version, codec,
hash-fn, digest}`):

```
token   = version "." codec "." hashfn "." digest
version = 1*(a-z / 0-9)        ; token-grammar version; "fp1" is the only LIVE value
codec   = 1*(a-z / 0-9)        ; WHAT was hashed and HOW normalized (§2.2)
hashfn  = 1*(a-z / 0-9)        ; hash family + width (§2.3)
digest  = lowercase hex, length fixed by hashfn
```

Example (the only M1-live prefix; this is the fixtures' `X0` golden token —
the fingerprint of `"# A\nintro\n\n# B\nbody\n"`):

```
fp1.span2.b3.40b167ed9b42a2beadb7c441b214efdc93069ef443a1cc2b5ae2ccda4cf03152
```

- Exactly four fields; every field non-empty; charset `[a-z0-9]` for the first
  three fields; digest is lowercase hex only. Anything else is **malformed**
  (not a fingerprint token at all).
- The prefix (`version.codec.hashfn`) fully determines interpretation. Hash or
  normalization migration = a NEW prefix; old tokens stay verifiable forever —
  migration lives inside the identifier, never beside it (#4 §1).
- The token is a plain YAML scalar (dots only), so `meridian-lock` `pins:`
  entries (#8 §2) carry it unquoted. Full-length tokens appear only in lock
  blocks and receipts (pin-count objects, #4 §5); render planes abbreviate.
- Display short form (render/wire view, #6 §4): `@` + a digest prefix
  (e.g. `@40b167ed`, 8 hex). Non-normative here — the `@fp` grammar belongs to
  the claim-link view plane; it always abbreviates the DIGEST field.

### 2.2 Codec registry

The codec names the byte domain AND its normalization version — one slot, one
registry entry, per #4 §1 ("raw span bytes at a named normalization version, a
composed tree node, or a receipt envelope"):

| Codec | Status | Domain |
|---|---|---|
| `span2` | **live (M1)** | the node's span bytes (contract §1 span laws, selector axis §3), canonicalized by norm-v2 (§4) |
| `node1` | reserved | composed dag-node encoding (own-hash + ordered child `(ref, fingerprint)` list) — the #4 §5 upgrade path, stage 2+ |
| `rcpt1` | reserved | receipt envelope — stage 2+ |
| `tree1` | reserved | workspace file-tree merkle domain — reserved for the future migration of the wire `fingerprint`/root spelling off `b3:` |

`span2`'s "2" is the norm version: norm-v1 is the raw-bytes non-normalization
the `node_rev` plane keeps; norm-v2 is §4. A future grammar change that alters
anchor recognition (§4.1) is a codec bump (`span3`), never a silent
reinterpretation of `span2`.

### 2.3 Hash-fn registry

| hashfn | Status | Meaning |
|---|---|---|
| `b3` | live | BLAKE3, 256-bit output, digest = exactly 64 lowercase hex |

### 2.4 Parse vs verify

- **Parse** is grammar-only and codec-agnostic: any 4-field token with valid
  charsets parses into `{version, codec, hashfn, digest}` — including codecs
  or hash-fns this build does not implement (self-describing survives its
  implementations). Digest LENGTH is validated only when the hashfn is known.
- **Verify** (recompute + compare) requires an implemented `(version, codec,
  hashfn)` triple. An unknown member — version included: a future `fp2` token
  parses under this grammar — is **unverifiable**, a distinct outcome from
  malformed and from red; it renders grey (`superseded-algo` family), never
  green, never red.
- Not tokens (never parse as fingerprints): bare 16-hex (`node_rev`),
  `b3:` + 64hex (the workspace-merkle wire spelling — unchanged in M1; its
  migration onto `tree1` is a stage-2 wire amendment).

## 3. What bytes enter the hash — the selector axis

The selector axis (WHICH span) and the canonicalization axis (HOW bytes become
a hash) never conflate (#4 §4). `span2` composes with any selector:

- **fingerprint(node)** = `b3( norm2( raw[span.start..span.end) ) )` where
  `span` is the node's contract-§1 span exactly as the model mints it
  (sections heading-inclusive and newline-inclusive; leaf blocks
  terminator-exclusive; frontmatter fence-to-fence; document = whole file).
- **own-hash(node)** (#4 §2) = `b3( norm2( own bytes ) )`:
  - Section → its heading LINE (the heading leaf span, terminator-exclusive);
  - Document → its frontmatter block span when present, else empty;
  - any leaf → its own span. **For a leaf, own-hash = fingerprint** (#4 §2).
  Same codec `span2` — own-hash differs from fingerprint on the selector axis
  only.
- **No embed expansion.** `span2` hashes the span's own bytes: an `![[embed]]`
  contributes its LINK bytes, never the embedded content. Cross-document
  transitivity is carried by lock-is-content (#8 §5): A's fingerprint covers
  A's `meridian-lock` block (it is inside A's span), which holds B's
  fingerprint — drift propagates by construction, at pin-update time, not at
  hash time. This supersedes `compose_rev`'s hash-time embed expansion (§6).
- Structural descendants need no explicit fold: a section's span contains
  every descendant's bytes, so any descendant edit moves the section
  fingerprint — span-hashing is fingerprint-v1's composition (#4 §5).

## 4. norm-v2 — the exact rule set

norm-v2 is the identity transform except for **anchor-token removal**. It
performs NO other change: no newline canonicalization (CRLF stays CRLF), no
trailing-whitespace trim, no NFC/NFD, no case folding, no BOM handling. Two
inputs differing in any non-anchor byte hash differently. (U2's Go-exact
heading sanitization is the ADDRESSING plane — selector derivation — and has
no bearing on hashed bytes.)

### 4.1 What is an anchor token

The one normative grammar is the syntax crate's block-anchor lexer
(`syntax::parse` → `DialectKind::Anchor`, ruling 011 / contract §2.4),
restated:

- The marker is `^` + id, id = 1+ chars of `[A-Za-z0-9-]` (app-exact; `_` is
  outside the charset, so `^b_1` is NOT an anchor).
- Line-tail only: after the id, only spaces/tabs and an optional `\r` may
  precede the line's `\n` (or EOF). At most one anchor per line.
- The byte before `^` must be a space, a tab, or the line start.
- Not inside fenced code or inline code (mask-exact per the parser).
- The **marker span** is `^` through the id end — trailing whitespace and the
  preceding separator are OUTSIDE the marker span. (The model's
  `NodeKind::Anchor` host-line re-span is an addressing fact; norm-v2 consumes
  the SYNTAX marker span.)

Anchor identification runs over the **whole file** (the document parse), never
over a slice re-parse — a slice can never change what is or is not an anchor.
Parser-grammar evolution that changes recognition is a codec bump (§2.2).

### 4.2 Removal rules

For each anchor marker `M` (file coordinates), classify by its line: let
`line_start` be the byte after the previous `\n` (or 0).

- **R1 — tail anchor** (any non-whitespace byte between `line_start` and
  `M.start`): remove `[M.start − 1, M.end)` — the marker plus exactly ONE
  immediately-preceding space or tab. Bytes after the id (trailing
  spaces/tabs, `\r`) are untouched. Exactly one separator is removed because
  promotion (#6 §2) inserts exactly one: `text` → `text ^goal` → `text`. A
  hand-written `text  ^goal` normalizes to `text ` — deterministic; only the
  promotion path carries a neutrality obligation.
- **R2 — own-line anchor** (only spaces/tabs, possibly none, between
  `line_start` and `M.start`): remove the ENTIRE line including its
  terminator: `[line_start, end_of_terminator)` where the terminator is the
  line's `\n` (a preceding `\r` of a CRLF pair is part of the removed line).
- **R2b — own-line anchor with no terminator** (the file's or slice's last
  line): remove the line AND the terminator of the PRECEDING line instead:
  `[t, line_end)` where `t` starts at the `\n` (or `\r\n`) immediately before
  `line_start`; when no preceding terminator exists, remove `[line_start,
  line_end)`. This is what makes own-line promotion at EOF neutral for
  terminator-exclusive slices: `…|rows|` → promote → `…|rows|\n^tbl` →
  norm-v2 → `…|rows|`.

Overlapping removal ranges (only constructible by hand, e.g. two consecutive
anchor-only lines at EOF) merge by union — deterministic for all inputs.

### 4.3 Application to a slice

`norm2(node)` = the node's span bytes with every removal range **intersected
with the span** applied. Removals are computed once, file-level (§4.1);
slicing never re-classifies. A removal range partially outside the span
removes only the intersection — a determinism guard; it cannot arise for the
grains pin promotes to (file, section, block), because marker + separator sit
inside the host block's span and an own-line anchor's line sits inside the
section/document span that contains it.

Grain consistency, by construction: a table's block span excludes a following
own-line anchor (the anchor line is outside the block's span → block
fingerprint trivially neutral); the section and document spans include the
anchor line → R2/R2b remove it there. Every grain of the same edit is
neutral, each through its own path.

### 4.4 Noted edge (parser-governed)

The current lexer does not mask frontmatter, so a caret-tail line INSIDE
frontmatter (`title: x ^fm`) mints an anchor and norm-v2 removes it from the
hashed bytes. Obsidian's own frontmatter has no block-id concept — this is a
recognized parser/app divergence, pinned by fixture (`frontmatter_caret`) so a
future parser fix is a VISIBLE codec decision (§2.2), not silent drift.

## 5. Rev-neutrality — the theorem the fixtures pin

For any pin promotion per #6 §2 (insert one ` ^id` at a block's line tail, or
one `^id` own line after a block, id in charset):

1. `fingerprint(node)` is UNCHANGED for every node whose span contains the
   promotion site, at every grain (block, section, document) — no false drift
   (#4 §4, the honesty doctrine).
2. `node_rev(node)` MOVES for every such node, and the workspace root moves —
   the CAS/guard planes see the real byte change (§1).

The neutrality obligation binds the promotion path exactly; hand-authored
anchor variants (extra separators, trailing whitespace) normalize
deterministically but claim no inverse-image guarantee.

## 6. Supersedes — the compose_rev scheme (U10 executes)

The `crates/model/src/compose.rs` scheme this spec replaces (the pre-marathon
22-01 design): leaf = `blake3("L" ‖ node_rev-hex-string)` (a hash of a hash's
hex spelling), hash-time `![[embed]]` expansion with cycle sentinel and
dangling refusal, 16-hex truncated `ComposeRev`, bare un-prefixed spelling.
Replaced by `fp1.span2.b3.<64hex>` over norm-v2 span bytes: no hash-of-hex
indirection, no hash-time graph walk (no cycles or dangling composes can
exist — the span is always complete), full-width digest, self-describing
prefix. `RevClass` maps: `Content` → fingerprint-token verify (parse →
codec dispatch → recompute → compare); `Object` → git-oid equality, unchanged
(git remains the only second family, never computed by the engine).

## 7. Fixture manifest

`crates/model/tests/norm_v2_fixtures.rs` — reference implementation of §4
(spec-verbatim, consumed by U10 as the conformance target) + golden table.
Canonical-bytes goldens: tail anchor; neutrality pair (§5 both directions);
own-line anchor (mid-file, at EOF terminated, at EOF unterminated — R2/R2b);
mid-line caret kept; fenced-code caret kept; inline-code line-tail; CRLF tail;
unicode id kept (`^ünïcode`); underscore id kept (`^b_1`); heading-line
anchor; two-spaces / tab separators; trailing-ws after id; frontmatter caret
(§4.4); empty doc; anchor-only doc. Section-grain: neutrality at section and
document grain incl. the EOF R2b case, plus the node_rev-moves contrast.
Token: mint/parse round-trip, malformed rejections, unknown-codec
parseability, golden digest + full-token literals.
