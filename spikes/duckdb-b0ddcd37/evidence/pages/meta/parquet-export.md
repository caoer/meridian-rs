---
title: Parquet export — the datalake convergence
---

```sql asof
select * from wiki.asof
```

<Alert status=info>

**as_of_root** **{asof[0].root_hex}** — snapshot built {asof[0].root_built_at}, source `wiki-snap.duckdb` (CoW clone, read-only). **Provenance: REAL — these are Evidence's own build artifacts.**

</Alert>

Evidence's source step extracts every source query to **Parquet**, then queries it
in-browser via DuckDB-WASM. That extract *is* the catalog's "Parquet export
converging with the harvest datalake" — **one pipeline, two consumers**: these
dashboards, and any datalake reader that globs the same files. No separate export
job. The files below were written by `npm run sources` at this root:

| parquet file | rows | bytes |
|---|---:|---:|
| `wiki/files/files.parquet` | 22,344 | 655,798 |
| `wiki/frontmatter/frontmatter.parquet` | 200,449 | 1,393,934 |
| `wiki/links/links.parquet` | 26,331 | 179,064 |
| `wiki/changes/changes.parquet` | 32,344 | 539,254 |
| `wiki/roots/roots.parquet` | 1 | 409 |
| `wiki/batches/batches.parquet` | 1 | 764 |
| `wiki/asof/asof.parquet` | 1 | 1,233 |
| **total** | **281,470** | **≈ 2.77 MB** |

Source snapshot `wiki-snap.duckdb` is **30.8 MB**; the Parquet projection of the
queried columns is **≈ 2.77 MB** (0.09×) — columnar + dictionary encoding, ready
to land in the harvest datalake unchanged.

**Convergence contract.** The datalake reader and the dashboard read the *same*
Parquet files under `static/data/wiki/`; a rebuild (`npm run sources`) at a new
root overwrites them atomically per source. The root stamp travels in the `asof`
extract, so a datalake consumer knows the tense of the Parquet it just globbed.
