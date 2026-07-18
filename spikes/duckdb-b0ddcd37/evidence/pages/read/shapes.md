---
title: Shape conformance — frontmatter key coverage by type
---

```sql asof
select * from wiki.asof
```

<Alert status=info>

**as_of_root** **{asof[0].root_hex}** — snapshot built {asof[0].root_built_at}, source `wiki-snap.duckdb` (CoW clone, read-only). **Provenance: REAL corpus facts.**

</Alert>

Shapes are data. For each document `type`, what fraction of its docs carry each
frontmatter key? A conformance projection straight out of the EAV table — the
view columns ARE the shape. Cell = % of that type's docs carrying that key.

```sql shapes
with typed as (
  select path, value as type from wiki.frontmatter where key = 'type'
),
tc as (select type, count(*) as n_docs from typed group by type),
top_types as (select type from tc order by n_docs desc limit 10),
top_keys as (select key, count(*) c from wiki.frontmatter group by key order by c desc limit 16)
select t.type, f.key,
  round(100.0 * count(distinct f.path) / tc.n_docs, 0) as coverage_pct
from typed t
join tc using (type)
join top_types using (type)
join wiki.frontmatter f on f.path = t.path
join top_keys tk on tk.key = f.key
group by t.type, f.key, tc.n_docs
```

<Heatmap data={shapes} x=type y=key value=coverage_pct valueFmt='0"%"' title="Key coverage % by type (100 = every doc of that type has the key)"/>

## Doc counts per type

```sql typecounts
select value as type, count(*) as n_docs
from wiki.frontmatter
where key = 'type'
group by value
order by n_docs desc
limit 10
```

<DataTable data={typecounts} rows=10>
  <Column id=type/>
  <Column id=n_docs title="documents" contentType=colorscale/>
</DataTable>
