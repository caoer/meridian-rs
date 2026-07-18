---
title: Tag aggregation (corpus-wide)
---

```sql asof
select * from wiki.asof
```

<Alert status=info>

**as_of_root** **{asof[0].root_hex}** — snapshot built {asof[0].root_built_at}, source `wiki-snap.duckdb` (CoW clone, read-only). **Provenance: REAL corpus facts.**

</Alert>

The r3 reference query `Q2`, verbatim shape over the 200k-row frontmatter table.
This page is the **V4 authoring-cost exemplar**: one existing SQL query → one
rendered page, zero custom JS.

```sql tags
select value as tag, count(*) as n
from wiki.frontmatter
where key = 'tags'
group by value
order by n desc
limit 30
```

<BarChart data={tags} x=tag y=n swapXY=true title="Top 30 tags corpus-wide"/>

<DataTable data={tags} rows=15 search=true>
  <Column id=tag/>
  <Column id=n title="pages" contentType=colorscale/>
</DataTable>
