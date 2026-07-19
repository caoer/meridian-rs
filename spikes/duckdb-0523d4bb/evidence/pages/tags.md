---
title: Tags
---

```sql as_of
select banner, root, built_at from meridian.as_of_root
```

> **<Value data={as_of} column=banner/>**

**REAL corpus data** — corpus-wide tag aggregation, top 50.

```sql top_tags
select tag, n from meridian.tags_top order by n desc
```

<BarChart
  data={top_tags.limit(25)}
  x=tag
  y=n
  swapXY=true
  title="Top 25 tags by usage"
/>

## Top 50 tags

<DataTable data={top_tags} rows=50/>
