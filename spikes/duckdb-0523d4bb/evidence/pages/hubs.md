---
title: Hubs
---

```sql as_of
select banner, root, built_at from meridian.as_of_root
```

> **<Value data={as_of} column=banner/>**

**REAL corpus data** — inbound resolved-link count per file (stem resolution).

```sql coverage
select
  count(*) as total_files,
  count(*) filter (where inbound > 0) as with_inbound,
  round(100.0 * count(*) filter (where inbound > 0) / count(*), 1) as pct_covered
from meridian.hubs
```

<BigValue data={coverage} value=total_files title="Total files" fmt='#,##0'/>
<BigValue data={coverage} value=with_inbound title="Files with ≥1 inbound" fmt='#,##0'/>
<BigValue data={coverage} value=pct_covered title="Coverage %" fmt='#,##0.0"%"'/>

```sql linked_only
select inbound from meridian.hubs where inbound > 0
```

<Histogram data={linked_only} x=inbound title="Inbound-link distribution (linked files only)"/>

## Deciles (linked files)

```sql deciles
select decile, count(*) as files, min(inbound) as min_inbound, max(inbound) as max_inbound
from (select inbound, ntile(10) over (order by inbound) as decile
      from meridian.hubs where inbound > 0)
group by decile order by decile
```

<DataTable data={deciles}/>

## Top hub pages

```sql top_hubs
select path, inbound from meridian.hubs order by inbound desc limit 20
```

<DataTable data={top_hubs} rows=20/>
