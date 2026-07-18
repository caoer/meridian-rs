---
title: Rung-5 View Face — meridian-rs wiki projection
---

```sql asof
select * from wiki.asof
```

<Alert status=info>

**as_of_root** **{asof[0].root_hex}** — snapshot built {asof[0].root_built_at}, file mtime {asof[0].snapshot_mtime_local}, source `wiki-snap.duckdb` (CoW APFS clone, read-only). **Provenance: REAL corpus facts.**

</Alert>

The **view face** of ZT's rung-5 surface catalog: DuckDB → Evidence over a
copy-on-write snapshot of the round-3 spike DB. The agent/wire face never reads
this DB — see the [boundary page](/boundary). Every page carries its own
`as_of_root` banner and a data-provenance label.

## Corpus at this root

```sql corpus
select
  (select count(*) from wiki.files)       as files,
  (select count(*) from wiki.links)       as links,
  (select count(*) from wiki.frontmatter) as frontmatter_rows,
  (select count(*) from wiki.changes)     as change_rows
```

<BigValue data={corpus} value=files title="files" fmt='#,##0'/>
<BigValue data={corpus} value=links title="links" fmt='#,##0'/>
<BigValue data={corpus} value=frontmatter_rows title="frontmatter (EAV)" fmt='#,##0'/>
<BigValue data={corpus} value=change_rows title="changes journal" fmt='#,##0'/>

## Provenance legend

- <Alert status=info>**REAL**</Alert> — computed from live corpus facts (files, links, frontmatter). The read surfaces.
- <Alert status=warning>**SYNTHETIC-DEMO**</Alert> — computed from the r3 spike's 10 000 synthetic churn rows (20 hot paths, timestamps collapsed to one instant). The history surfaces, until a live appender exists.

## Read surfaces (real)

- [Backlinks explorer](/read/backlinks) — find-references for any page (parameterized)
- [Orphans & dangling links](/read/orphans) — anti-joins over the link graph
- [Tag aggregation](/read/tags) — corpus-wide tag counts
- [Board view](/read/board) — tasks by status & owner from frontmatter
- [Shape conformance](/read/shapes) — frontmatter key coverage by `type` (shapes are data)
- [Hub / coverage](/read/hubs) — inbound-link distribution

## History surfaces (synthetic-demo)

- [Per-path history window](/history/path-history) — the changes stream for one path
- [Activity heat map](/history/heatmap) — stream × change-kind
- [Compound inputs](/history/compound-inputs) — everything changed between root A and root B, scoped to memos/decisions/results

## Meta surfaces

- [Parquet export](/meta/parquet-export) — Evidence's own extract as the datalake convergence
- [Rule-pack cost ledger](/meta/rulepack-ledger) — schema + mock (NOT RUN — no measured step data exists yet)

## Boundary

- [The two faces of one projection](/boundary) — the R5 line, verbatim from the round-4 catalog
