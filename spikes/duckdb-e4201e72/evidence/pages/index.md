---
title: Meridian — rung-5 surface catalog
---

```sql as_of
select as_of_root, built_at_str from meridian.as_of_root
```

<Alert status=info>

**as_of_root** <Value data={as_of} column=as_of_root/> · built <Value data={as_of} column=built_at_str/> · source: CoW APFS snapshot of the round-3 spike `wiki.duckdb` (read-only; never the live file) · this page: **REAL corpus facts**

</Alert>

**Speak in Evidence.** This project mines the round-3 DuckDB projection of the
home-wiki + sessions corpus into the rung-5 surface catalog — the *view face* of
the two-faces law. The *agent face* never touches this DB (see [Boundary](/boundary)).

Every page carries an **as_of_root banner** and a **provenance label**
(REAL corpus facts · SYNTHETIC-DEMO churn). Snapshots are disposable by
construction: `cp -c` clone → read-only → delete = re-clone.

```sql corpus
select
  (select count(*) from meridian.files) as files,
  (select count(*) from meridian.links) as links,
  (select count(*) from meridian.links where resolved) as resolved_links,
  (select count(*) from meridian.frontmatter) as frontmatter_rows,
  (select count(distinct value) from meridian.frontmatter where key='tag') as distinct_tags
```

<BigValue data={corpus} value=files title="Files (md)" fmt=num0/>
<BigValue data={corpus} value=links title="Wikilinks" fmt=num0/>
<BigValue data={corpus} value=resolved_links title="Resolved" fmt=num0/>
<BigValue data={corpus} value=frontmatter_rows title="Frontmatter rows" fmt=num0/>
<BigValue data={corpus} value=distinct_tags title="Distinct tags" fmt=num0/>

## Read surfaces — REAL data
- [Backlinks explorer](/backlinks) — find-references for any page (default `LLM_WIKI`)
- [Orphans &amp; dangling links](/orphans) — anti-join both directions
- [Tag aggregation](/tags) — corpus-wide tag frequency
- [Task board](/board) — tasks by frontmatter status / owner
- [Shape conformance](/shapes) — required-key conformance for a chosen shape (`type=task`)
- [Hub / coverage](/hubs) — inbound-link distribution, top hubs

## History surfaces — SYNTHETIC-DEMO churn
- [Per-path history window](/history) — last-N changes for a path
- [Compound front door](/compound) — changes between root A → root B, scoped to memos/decisions/results

## Meta surfaces
- [Fleet view](/fleet) — task cards across the sessions tree (TASKS.base as SQL)
- [Parquet export](/parquet) — the datalake convergence receipt
- [Rule-pack cost ledger](/ledger) — **NOT-RUN** (schema + mock; no measured step data exists)

## Boundary
- [The two-faces law + the R5 line](/boundary)
