---
type: spec
id: body-projection
status: standing
updated: 2026-08-15
description: How section body text projects into the sql face — the exclusive-chunk law, the split body relation, and the content-addressed cache protocol that keeps the append-only file from re-storing unchanged text.
owns: [the body relation, the exclusive-chunk law, the body_text content-address protocol]
---

# body projection — section text as a first-class relation

> **Standing:** Design law is `wire-contract.md` (one contract). DuckDB / SQL
> boards are **not** agent core (README standing correction C). **Doc correct >
> code correct; docs first.** See `README.md`.

Status: normative for the `body` relation of the sql projection (both lanes:
the ephemeral `:memory:` build and the `sql.duckdb` cache). Law also:
`node-rev-merkle-spec.md` (node_rev, content span), `laws.md` § crate charters,
`view::store` module docs (cache protocol).

Mandate (ZT, 2026-08-15, verbatim): *"do 3. if you look at other sql card.
body is one of the most important field we need to add"* — the unpark-and-build
ruling on the parked body-in-projection row. The 2026-08-14 measurements stay
on record and shape this HOW: structure-only projection 25 MB; naive
body-plus-FTS file 174 MB; FTS index build over doc+section bodies 5.83 s;
raw bodies 35.9 MB; out-of-engine full-corpus body read 0.52 s.

## §1 The grain — chunks, not documents

The projection serves body text at the grain questions arrive at: *which
section says X*. One row per **chunk**, where a chunk is a section's own text
(children excluded) or a document's preamble (text before its first heading).
Heading lines are in **no** chunk: heading text is already served on
`section.heading`, and §4's dedup must survive a heading rename.

Whole-document text is deliberately not a relation: it re-stores every chunk
byte a second time, and the out-of-engine read (`mrd read`, measured 0.52 s
corpus-wide) already serves it.

## §2 The chunk law

- **Section chunk:** from the section's content start (the byte after its
  heading line's terminator — `model`'s content-span law, one owner) to its
  first child section's span start, or its own span end when childless. A
  section's exclusive content is provably one contiguous run: only a heading
  closes a section, and any heading either opens a child, a sibling, or closes
  the section itself. Every body byte therefore lands in exactly one chunk.
- **Preamble chunk:** from the frontmatter node's span end (0 when no
  frontmatter) to the first section's span start (file end when the document
  has no sections). Emitted only when non-empty.
- **Section chunks always emit**, empty text included:
  `COUNT(*) FROM body WHERE section_seq IS NOT NULL` equals
  `COUNT(*) FROM section` by invariant.
- `text` is the chunk's raw bytes verbatim — no trimming, no normalization.
  Chunk boundaries are line-aligned by construction, so the slice is valid
  UTF-8 (files are UTF-8-valid by the wire span laws).

## §3 The relation

Both lanes serve the same face. DDL is the contract
(`crates/view/src/schema.rs` mirrors this section):

```sql
CREATE TABLE body (
    path        TEXT     NOT NULL REFERENCES doc(path),
    seq         UBIGINT  NOT NULL,  -- 0-based document order of chunks
    section_seq UBIGINT,            -- owning section's node_seq; NULL = preamble
    hpath       TEXT,               -- owning section's machine address (as section.hpath); ADVISORY; NULL on preamble
    text        TEXT     NOT NULL,  -- the chunk bytes, verbatim (§2)
    span_start  UBIGINT  NOT NULL,  -- the chunk's own byte range (C1: slice raw bytes and get text)
    span_end    UBIGINT  NOT NULL,
    node_rev    TEXT,               -- the OWNING SECTION's CAS token (search hit → guarded splice); NULL on preamble
    PRIMARY KEY (path, seq),
    FOREIGN KEY (path, section_seq) REFERENCES section(path, node_seq)
);
```

The shape is `task`'s (own seq space, nullable governing section, advisory
hpath) because the population is the same: rows that usually belong to a
section but legally may not. `node_rev` is the owning section's token, not a
chunk hash: it is what a caller who found text needs next (a guarded `splice`
on that section), and a chunk-grain rev would teach an affordance that refuses
— there is no chunk splice door.

## §4 The cache lane — content-addressed, append-only

Every column served on `main.*` is physically a `hist.*` column (the cache's
only storage), and every re-projection of a document appends its full row set
at the next generation. Naive body columns would therefore re-store a
document's whole text on every edit — including the link-resolution
re-projections that touch documents whose bytes did not move. The cache
protocol splits identity from content instead:

- **`hist.body`** — the narrow per-generation row:
  `(path, gen, seq, section_seq, hpath, span_start, span_end, node_rev,
  body_key)`;
- **`hist.body_text (body_key, text)`** — content-addressed text,
  insert-if-absent (a staged anti-join at append; INSERT-only, so the
  never-edit law holds). `body_key` is the **full 64-hex blake3** of the chunk
  bytes: `node_rev`'s 16-hex truncation is scoped by the merkle spec to
  per-node CAS racing, and a corpus-wide content address that serves the wrong
  body on collision needs the full width;
- **`main.body`** — the latest-generation pick through `hist.doc_latest`
  (the standing semi-join pattern) joined to `hist.body_text` on `body_key`,
  serving exactly §3's column list. `body_key` never appears on the face.

Unchanged chunks dedup across generations AND across paths (template-stamped
corpora collapse hard). An edit appends narrow rows for the re-projected
document plus text rows only for chunks whose bytes are new to the file's
history. Orphaned `body_text` rows compact only at rebuild-and-swap, like all
hist rows. The pin protocol is untouched: one file, one transaction, one
fingerprint — body bytes are md-corpus bytes, so `as_of_fingerprint` already
covers them and no second witness exists (contrast `base_fold`, which exists
precisely because `.base` bytes are outside the fingerprint).

## §5 Alternatives held, and why they lose

- **A `body` column on `section`.** Fattens the toc-shaped workload
  (`SELECT * FROM section`) with kilobyte cells, and the preamble has no
  section row to ride. The split face keeps structure queries at their
  measured cost.
- **A second file (ATTACH).** Breaks the single-file single-transaction pin
  invariant — "the file is always at exactly one fingerprint" — and DuckDB
  offers no cross-file transaction to replace it.
- **Lazy body materialization.** A partially-projected file forks the pin into
  a second freshness dimension inside a domain the fingerprint already covers.
  `base_fold` earned its second witness because its bytes are outside the
  fingerprint; body bytes are inside it, so laziness buys an inconsistency,
  not a witness.
- **An FTS index in the cache file.** `PRAGMA create_fts_index` /
  `drop_fts_index` per pin is edit-shaped work in a never-vacuumed file, and
  the measured 3.36–5.83 s rebuild would ride the daemon's per-save append
  path. Search on the face today is LIKE/regexp over `body.text`; a caller may
  build a per-call FTS index in the rollback lane at its measured cost. A
  persistent, disposable side-artifact index is a future design with its own
  card, taken only if per-call friction shows.

## §6 Costs, named

- **Cache file, cold:** ~25 MB structure + ~34 MB deduped chunk text + ~5 MB
  narrow rows ≈ **65 MB** on the measuring corpus (6,686 docs / 51,374
  sections) — labeled derived-from-measurement, vs 174 MB measured for the
  naive body+FTS file.
- **Cache append:** one staged anti-join insert per append; text growth
  bounded by chunks whose bytes are new, not by documents touched.
- **`:memory:` lane, per query:** ~51k chunk slices (~34 MB transient) on the
  measuring corpus — a small fraction of the lane's standing full-corpus
  fold+parse; `mrd sql` remains a slow operator tool by design.
- **Schema:** `SCHEMA_VERSION`, `CACHE_SCHEMA_VERSION`, and `SCHEMA_SALT`
  each advance one past the wave-2 chain's previously landed value
  (editset-n-column → base-projection → body); delete-don't-migrate covers
  both lanes (a mismatched cache file cold-rebuilds).

## §7 Rollout

### §7.1 Red tests to pin (each lands with the code, red-first)

1. **The gate:** a fixture with preamble, nested sections, an empty-content
   section, and CJK text answers §8's SELECT — exact chunk rows, bytes
   verbatim.
2. **No phantoms:** a doc with no preamble emits no preamble row; a
   frontmatter-only doc emits zero body rows.
3. **Dedup:** an identical section body in two docs yields ONE
   `hist.body_text` row; editing doc A while section S is unchanged appends no
   new text row for S.
4. **Rename survives:** a heading-only rename moves `node_rev` and leaves the
   `hist.body_text` count unchanged (§1's heading-exclusion law, pinned).
5. **Tombstone:** a removed doc's chunks leave `main.body`.
6. **Face parity:** `main_face_columns_match_the_ephemeral_build_exactly` and
   the surface digests gain `body`; the cache equals a fresh build after
   cold build and after append.
7. **Version:** a version-6 cache file cold-rebuilds under 7.

### §7.2 Doc deltas riding the code card (docs-first: this spec authorizes them)

`crates/view/src/schema.rs` DDL + `SCHEMA_VERSION` note;
`crates/view/src/store.rs` cache DDL, append path, module doc (hist tables,
dedup protocol) + `CACHE_SCHEMA_VERSION` note; `crates/cache` `SCHEMA_SALT`;
`docs/README.md` index row.

### §7.3 Served-face note

Landing the code changes the sql face's answer set: new relation `body` (and
`hist.body` / `hist.body_text` in the catalog), new `information_schema` rows,
`SCHEMA_VERSION` 7. Conformance re-records ride the daemon's pin bump
(deploy single-writer), armed by the landing's mandatory-tier flag. Downstream
teaching surfaces outside this repo (the sql tool's skill, the daemon's
refusal text) follow the served face after landing.

### §7.4 Out of scope, named

- FTS/BM25 index persistence (§5, future card).
- Whole-document text reconstruction (out-of-engine `mrd read` stays the
  answer).
- Wire surface: none. The sql face is a non-wire operator face; no door
  changes.

## §8 The worked gate

Fixture (`a.md`):

```markdown
---
title: Alpha
---
preamble line

# Top
intro

## Sub
sub body
```

```sql
SELECT seq, section_seq, hpath, text FROM body WHERE path = 'a.md' ORDER BY seq;
```

| seq | section_seq | hpath | text |
|---|---|---|---|
| 0 | NULL | NULL | `preamble line\n\n` |
| 1 | 0 | `[{"h":"Top"}]` | `intro\n\n` |
| 2 | 1 | `[{"h":"Top"},{"h":"Sub"}]` | `sub body\n` |

And the workflow the relation exists for — search hit to guarded write in one
row: `SELECT path, hpath, node_rev FROM body WHERE text LIKE '%sub body%'`
returns the address and the CAS token a `splice` needs.
