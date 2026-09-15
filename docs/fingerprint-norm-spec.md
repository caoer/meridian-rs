---
type: result
id: fp
status: spec
created: 2026-07-24
tags: [type/result, domain/meridian-rs, topic/meridian-rs, topic/merkle]
owns: [the fingerprint CID token, norm-v2]
---

# Fingerprint CID-token + normalization spec

> **Naming:** **norm-v2** (codec `span2` in `fp1.span2.b3.…`) canonicalizes span
> bytes before hashing — an algorithm name, not a wire-contract version.

> Standing law: `README.md` (process and standing corrections) and `wire-contract.md` (the wire contract).

**Scope note:** hash/fingerprint/norm law, not address grammar; mint-plane hpath
stays segment form. The golden fixtures and this doc are one artifact; a
divergence is a defect in whichever moved last.

## 1. The three hash planes

Every hash is BLAKE3-256 (node-rev-merkle-spec §1); planes differ on **domain**
and **job**, never family:

| Plane | Byte domain | Normalization | Spelling | Job |
|---|---|---|---|---|
| `node_rev` | node span bytes (contract §1) | **none — raw** | bare 16 lowercase hex | CAS race detector (`if_node_rev`) |
| workspace merkle (`leaf` → fingerprint) | whole raw file bytes in **hash domain** → tree | **none — raw** | `b3:` + 64 lowercase hex | world cursor (`if_fingerprint`, `fingerprint` op / `diff`) |
| **fingerprint** (this spec) | node span bytes | **norm-v2 (§4)** | CID-token (§2) | attestation content identity (pins, locks, receipts) |

`node_rev` and the workspace merkle stay as `node-rev-merkle-spec.md` §2–§4
defines them.

## 2. The fingerprint token

### 2.1 Grammar

Four dot-joined lowercase fields: `{version, codec, hash-fn, digest}`.

```
token = version "." codec "." hashfn "." digest
version = 1*(a-z / 0-9) ; token-grammar version; "fp1" is the only LIVE value
codec = 1*(a-z / 0-9) ; WHAT was hashed and HOW normalized (§2.2)
hashfn = 1*(a-z / 0-9) ; hash family + width (§2.3)
digest = lowercase hex, length fixed by hashfn
```

Example — the only live prefix, the fixtures' `X0` golden token for
`"# A\nintro\n\n# B\nbody\n"`:

```
fp1.span2.b3.40b167ed9b42a2beadb7c441b214efdc93069ef443a1cc2b5ae2ccda4cf03152
```

- All four fields non-empty; `[a-z0-9]` for the first three; lowercase hex
  digest — anything else is **malformed**.
- The prefix `version.codec.hashfn` fixes interpretation: a hash or
  normalization migration mints a new prefix, and old tokens stay verifiable
  forever.
- No YAML escaping needed; the lock quotes it anyway (`fingerprint: "<CID>"`,
  `crates/lock` render law).
- Full-length tokens appear only in lock blocks (`laws.md` lock crate) and
  receipts (pin-count objects); elsewhere the short form is `@` + digest prefix
  (`@40b167ed`, 8 hex), non-normative here and owned by the claim-link view
  plane's `@fp` grammar, always the digest.

### 2.2 Codec registry

A codec names domain + normalization version.

| Codec | Status | Domain |
|---|---|---|
| `span2` | **live** | the node's span bytes (contract §1 span laws, selector axis §3), canonicalized by norm-v2 (§4) |
| `props1` | **live** | frontmatter property bytes — the canonical keyed map (sorted keys, length-prefixed `len:key`, three-state values `=A` absent / `=N` null / `=S` scalar), domain-separated by the `props1\n` prefix (wire-contract §A.6.2) |
| `node1` | reserved | composed dag-node encoding: own-hash + ordered child `(ref, fingerprint)` list; the composition upgrade path |
| `rcpt1` | reserved | receipt envelope |
| `tree1` | reserved | workspace file-tree merkle domain — reserved for migrating the wire `fingerprint`/root spelling off `b3:` |

`span2`'s "2" is the norm version; norm-v1 is the raw bytes `node_rev` keeps.
Changing anchor recognition (§4.1) is a codec bump (`span3`), never a silent
reinterpretation of `span2`.

### 2.3 Hash-fn registry

| hashfn | Status | Meaning |
|---|---|---|
| `b3` | live | BLAKE3, 256-bit output, digest = exactly 64 lowercase hex |

### 2.4 Parse vs verify

- **Parse**: grammar-only, codec-agnostic. Any valid 4-field token parses into
  `{version, codec, hashfn, digest}`, unknown codecs and hash-fns included;
  digest length is checked only for a known hashfn.
- **Verify** (recompute + compare) needs an implemented `(version, codec,
  hashfn)` triple. An unknown member — a future `fp2` still parses — is
  **unverifiable**: not malformed, not red, rendered grey (`superseded-algo`
  family), never green.
- Never tokens: bare 16-hex (`node_rev`); `b3:` + 64hex (workspace-merkle wire
  spelling; its move onto `tree1` is a future wire amendment).

## 3. What bytes enter the hash — the selector axis

`span2` composes with any selector; selector (which span) and canonicalization
(how bytes hash) stay separate axes.

- **fingerprint(node)** = `b3( norm2( raw[span.start..span.end) ) )` on the
  node's contract-§1 span as minted: sections heading- and newline-inclusive,
  leaf blocks terminator-exclusive, frontmatter fence-to-fence, document =
  whole file.
- **own-hash(node)** = `b3( norm2( own bytes ) )`, same codec, selector only:
  section → heading line (heading leaf span, terminator-exclusive); document →
  frontmatter block span, else empty; leaf → own span (**leaf: own-hash =
  fingerprint**).
- **No embed expansion.** `![[embed]]` contributes link bytes, never embedded
  content. Transitivity is lock-is-content: A's span covers A's `meridian-lock`
  block holding B's fingerprint, so drift propagates at pin-update time, not
  hash time (§6).
- **No descendant fold.** A section's span holds every descendant's bytes; any
  descendant edit moves its fingerprint.

## 4. norm-v2 — the exact rule set

norm-v2 is the identity transform except for **anchor-token removal**: no
newline canonicalization (CRLF stays CRLF), no whitespace trim, no NFC/NFD, no
case folding, no BOM handling. Any non-anchor byte difference changes the hash.
Heading sanitization is addressing (selector derivation), not hashing.

### 4.1 What is an anchor token

Normative grammar: the syntax crate's block-anchor lexer (`syntax::parse` →
`DialectKind::Anchor`, contract §2.4).

- `^` + id, id = 1+ of `[A-Za-z0-9-]` (app-exact: `_` excluded, so `^b_1` is not
  an anchor).
- Line-tail only: after the id only spaces/tabs and an optional `\r`, then `\n`
  or EOF; one anchor per line.
- Byte before `^`: space, tab, or line start.
- Never inside fenced or inline code (mask-exact per the parser).
- **Marker span** = `^` through id end; separator and trailing whitespace lie
  outside it. norm-v2 consumes this marker span, not the model's
  `NodeKind::Anchor` host-line re-span.

Classification runs on the whole-file parse, never a slice re-parse.

### 4.2 Removal rules

Per anchor marker `M` (file coordinates); `line_start` = the byte after the
previous `\n` (or 0):

- **R1 — tail anchor** (non-whitespace between `line_start` and `M.start`):
  remove `[M.start − 1, M.end)` — the marker plus exactly one preceding space or
  tab; bytes after the id (spaces/tabs, `\r`) untouched. Promotion inserts one
  separator: `text` → `text ^goal` → `text`; hand-written `text ^goal` →
  `text `.
- **R2 — own-line anchor** (only spaces/tabs, possibly none, before `M.start`):
  remove the entire line, `[line_start, end_of_terminator)`; the terminator is
  the line's `\n` with any preceding `\r`.
- **R2b — own-line anchor on an unterminated last line** (of file or slice):
  remove `[t, line_end)`, `t` at the `\n` (or `\r\n`) immediately before
  `line_start`; with no preceding terminator, `[line_start, line_end)`. EOF
  promotion stays neutral for terminator-exclusive slices: `…|rows|` →
  `…|rows|\n^tbl` → norm-v2 → `…|rows|`.
- Overlapping ranges (hand-made only, e.g. two anchor-only lines at EOF) merge
  by union, deterministically.

### 4.3 Application to a slice

`norm2(node)` = the span bytes with every removal range **intersected with the
span** applied; removals are computed once, file-level (§4.1). A range partly
outside the span removes only the intersection. So a block span excludes a
following own-line anchor (trivially neutral), while section and document spans
include it and R2/R2b remove it.

### 4.4 Noted edge (parser-governed)

The lexer does not mask frontmatter, so a caret-tail line there
(`title: x ^fm`) mints an anchor and norm-v2 removes it from the hashed bytes.
Fixture `frontmatter_caret` pins this parser/app divergence, so a parser fix is
a visible codec decision (§2.2), not silent drift.

## 5. Rev-neutrality — the theorem the fixtures pin

For any pin promotion — ` ^id` at a block's line tail, or `^id` own line after
a block, id in charset:

1. `fingerprint(node)` is unchanged at every grain (block, section, document)
   for every node whose span contains the site: no false drift.
2. `node_rev(node)` moves for every such node, and the workspace root moves:
   the CAS and guard planes see the real byte change (§1).

Only the promotion path is bound; hand-authored variants (extra separators,
trailing whitespace) normalize deterministically with no inverse-image
guarantee.

## 6. Supersedes — the compose_rev scheme

The content fingerprint `fp1.span2.b3.<64hex>` covers norm-v2 span bytes: no
hash-of-hex indirection, no hash-time graph walk (the span is always complete:
no cycles, no dangling composes). `RevClass`: `Content` → fingerprint-token
verify (parse → codec dispatch → recompute → compare); `Object` → git-oid
equality (git, the only second family, never computed by the engine).

## 7. Fixture manifest

`crates/model/tests/norm_v2_fixtures.rs` — spec-verbatim reference
implementation of §4, the conformance target, plus the golden table.

- Canonical bytes: tail anchor; neutrality pair (§5, both directions); own-line
  anchor (mid-file, EOF terminated, EOF unterminated — R2/R2b); mid-line caret
  kept; fenced-code caret kept; inline-code line-tail; CRLF tail; unicode id
  kept (`^ünïcode`); underscore id kept (`^b_1`); heading-line anchor;
  two-spaces / tab separators; trailing-ws after id; frontmatter caret (§4.4);
  empty doc; anchor-only doc.
- Section grain: neutrality at section and document grain incl. EOF R2b, plus
  the node_rev-moves contrast.
- Token: mint/parse round-trip, malformed rejections, unknown-codec
  parseability, golden digest + full-token literals.
