---
type: spec
id: body-projection
status: standing
description: How section body text projects into the sql face — the exclusive-chunk law, the split body relation, and the content-addressed cache protocol that keeps the append-only file from re-storing unchanged text.
owns: [the body relation, the exclusive-chunk law, the body_text content-address protocol]
---

# body projection

> Standing law: `README.md` (process, standing corrections; correction C: the sql face is not agent core) and `wire-contract.md`.

Normative for the `body` relation in both sql lanes (`:memory:` build, `sql.duckdb` cache). Also law: `node-rev-merkle-spec.md` (node_rev, content span), `laws.md` § crate charters, `view::store` module docs (cache protocol).

## §1 The grain

One row per **chunk**: a section's own text (children excluded) or the document **preamble** (text before its first heading). Heading lines are in **no** chunk (`section.heading` serves them). Whole-document text is not a relation; the out-of-engine `mrd read` (measured 0.52 s corpus-wide) serves it.

## §2 The chunk law

- **Section chunk:** content start (the byte after the heading line's terminator; `model`'s content-span law) to the first child's span start, or the section's span end when childless. Only a heading closes a section, and any heading opens a child, a sibling, or closes it, so exclusive content is one contiguous run: every body byte lands in exactly one chunk.
- **Preamble chunk:** frontmatter node's span end (0 without frontmatter) to the first section's span start (file end without sections); emitted only when non-empty.
- **Section chunks always emit**, even empty: `COUNT(*) FROM body WHERE section_seq IS NOT NULL` equals `COUNT(*) FROM section`.
- `text` is the raw chunk bytes, untrimmed and unnormalized; boundaries are line-aligned, so the slice is valid UTF-8.

## §3 The relation

Both lanes serve the same face; the DDL is the contract (`crates/view/src/schema.rs` mirrors it):

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

The shape is `task`'s. `node_rev` is the owning section's CAS token, not a chunk hash; there is no chunk splice door.

## §4 The cache lane

The cache is append-only (`hist.*` is its only storage): each re-projection appends the document's full row set at the next generation, so identity and content are split:

- **`hist.body`** — the narrow per-generation row: `(path, gen, seq, section_seq, hpath, span_start, span_end, node_rev, body_key)`.
- **`hist.body_text (body_key, text)`** — content-addressed text, insert-if-absent (staged anti-join at append; INSERT-only, keeping the never-edit law). `body_key` is the **full 64-hex blake3** of the chunk bytes; `node_rev`'s 16-hex truncation is for per-node CAS racing only (merkle spec).
- **`main.body`** — the latest generation via `hist.doc_latest` (the standing semi-join), joined to `hist.body_text` on `body_key`, serving exactly §3's columns; `body_key` never appears on the face.

Only `hist.body_text` rows dedup, across generations and paths: an edit appends text rows only for chunks new to the file's history. Orphaned `body_text` rows compact only at rebuild-and-swap. The pin protocol is unchanged: one file, one transaction, one fingerprint. `as_of_fingerprint` covers body bytes, so no second witness exists; `base_fold` has one only because `.base` bytes are outside the fingerprint.

## §5 Alternatives

- **`body` column on `section`** — rejected: kilobyte cells fatten the toc-shaped workload (`SELECT * FROM section`); the preamble has no section row.
- **Second file (ATTACH)** — rejected: breaks the single-file single-transaction pin invariant (§4); DuckDB has no cross-file transaction.
- **Lazy body materialization** — rejected: forks the pin into a second freshness dimension inside the fingerprint's domain (§4).
- **FTS index in the cache file** — rejected: `PRAGMA create_fts_index` / `drop_fts_index` per pin is edit-shaped work in a never-vacuumed file, and the measured 3.36–5.83 s rebuild would ride the daemon's per-save append path. Search on the face is LIKE/regexp over `body.text`; a caller may build a per-call FTS index in the rollback lane at its measured cost. A persistent side-artifact index is a future design, taken only if per-call friction shows.

## §6 Costs

- **Cache file, cold:** ~25 MB structure + ~34 MB deduped chunk text + ~5 MB narrow rows ≈ **65 MB** (derived) on the measuring corpus (6,686 docs / 51,374 sections; raw bodies 35.9 MB), vs 174 MB measured for the naive body+FTS file.
- **Cache append:** one staged anti-join insert per append; text grows only by new chunk bytes, not by documents touched.
- **`:memory:` lane, per query:** ~51k chunk slices (~34 MB transient) on the measuring corpus; `mrd sql` stays a slow operator tool by design.
- **Schema:** `SCHEMA_VERSION`, `CACHE_SCHEMA_VERSION`, and `SCHEMA_SALT` each advance by one; delete-don't-migrate covers both lanes (a mismatched cache file cold-rebuilds).

## §7 Rollout

### §7.1 Red tests to pin (red-first, with the code)

1. **The gate:** a fixture with preamble, nested sections, an empty-content section, and CJK text answers §8's SELECT byte-exact.
2. **No phantoms:** a doc with no preamble emits no preamble row; a frontmatter-only doc emits zero body rows.
3. **Dedup:** the same body in two docs yields one `hist.body_text` row; editing doc A with section S unchanged adds no text row for S.
4. **Rename survives:** a heading-only rename moves `node_rev`; the `hist.body_text` count is unchanged (§1's heading exclusion).
5. **Tombstone:** a removed doc's chunks leave `main.body`.
6. **Face parity:** `main_face_columns_match_the_ephemeral_build_exactly` and the surface digests gain `body`; the cache equals a fresh build after cold build and after append.
7. **Version:** a version-6 cache file cold-rebuilds under 7.

### §7.2 Doc deltas

This spec authorizes: `crates/view/src/schema.rs` DDL + `SCHEMA_VERSION` note; `crates/view/src/store.rs` cache DDL, append path, module doc (hist tables, dedup protocol) + `CACHE_SCHEMA_VERSION` note; `crates/cache` `SCHEMA_SALT`; `docs/README.md` index row.

### §7.3 Served-face note

Landing adds relation `body` (`hist.body` / `hist.body_text` in the catalog), new `information_schema` rows, and `SCHEMA_VERSION` 7. Conformance re-records ride the same landing; downstream clients' teaching surfaces follow the served face.

### §7.4 Out of scope

- FTS/BM25 index persistence (§5, future design).
- Whole-document text reconstruction (§1: `mrd read`).
- Wire surface: none; the sql face is a non-wire operator face.

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

`SELECT path, hpath, node_rev FROM body WHERE text LIKE '%sub body%'` returns the address and CAS token a `splice` needs.
