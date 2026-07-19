---
title: Parquet export — datalake convergence
---

```sql as_of
select as_of_root, built_at_str from meridian.as_of_root
```

<Alert status=info>

**as_of_root** <Value data={as_of} column=as_of_root/> · built <Value data={as_of} column=built_at_str/> · this page: **mixed provenance** (row counts real; `changes`/`batches` synthetic-demo)

</Alert>

**Evidence's architecture *is* the meta surface.** Evidence doesn't query the
DuckDB file at page-load — during `evidence sources` it **extracts each source
query to a Parquet file**, then the browser queries those Parquet files with
DuckDB-WASM. That extract step is literally the *"Parquet export converging with
the harvest datalake"* catalog item: **one pipeline, two consumers** — the
dashboards on this site, and any datalake that reads the same Parquet.

## The extract manifest (row counts are live from the projection)

```sql manifest
select table_name, rows, provenance from meridian.extract_manifest order by rows desc
```

<DataTable data={manifest}>
  <Column id=table_name/>
  <Column id=rows fmt=num0/>
  <Column id=provenance/>
</DataTable>

## The extract directory — the receipt

Each table above lands as one Parquet file under the project's static data dir.
This is the convergence point: point a lakehouse (DuckLake, an Iceberg catalog,
or `harvest`'s datalake) at this directory and the same bytes feed it.

```sql
evidence/.evidence/template/static/data/meridian/
├── files/files.parquet                     (~484 KiB)   22,343 rows   real
├── frontmatter/frontmatter.parquet         (~1.16 MiB) 188,401 rows   real
├── links/links.parquet                     (~184 KiB)   26,642 rows   real
├── changes/changes.parquet                 (~181 KiB)   10,002 rows   synthetic-demo
├── roots/roots.parquet                     (~0.4 KiB)         2 rows   real
├── batches/batches.parquet                 (~0.7 KiB)         1 row    synthetic-demo
├── as_of_root/as_of_root.parquet           (~0.6 KiB)         1 row    derived
└── extract_manifest/extract_manifest.parquet (~0.7 KiB)      6 rows   derived
```
<Alert status=info>Sizes captured at the extract that built this page; row counts above are re-queried live each build.</Alert>

## EQ4 partial — is this THE face, or a reference impl?

Evidence earns rung 5 as the **human/orientation face** because its extract-to-Parquet
step is the datalake bridge for free — a plainer renderer would still need that
export. The *agent* face is not Evidence; it is the wire fact-op API (see
[boundary](/boundary)). So: Evidence is the ratified **view-face reference**, and its
Parquet extract is the non-negotiable structural win, not the chart library.

<Details title="Source query — sources/meridian/extract_manifest.sql">

```sql
select 'files' as table_name, count(*) as rows, 'real' as provenance from files
union all select 'links', count(*), 'real' from links
union all select 'frontmatter', count(*), 'real' from frontmatter
union all select 'nodes', count(*), 'real' from nodes
union all select 'roots', count(*), 'real' from roots
union all select 'batches', count(*), 'synthetic-demo' from batches
union all select 'changes', count(*), 'synthetic-demo' from changes;
```

</Details>
