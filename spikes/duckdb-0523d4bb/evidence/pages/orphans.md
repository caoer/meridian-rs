---
title: Orphans
---

```sql as_of
select banner, root, built_at from meridian.as_of_root
```

> **<Value data={as_of} column=banner/>**

**REAL corpus data** — files no resolved wiki-link points at (stem-resolution anti-join).

```sql orphan_counts
select
  count(*) as orphans_total,
  count(*) filter (where not is_bulk_capture) as orphans_pages,
  count(*) filter (where is_bulk_capture) as orphans_bulk
from meridian.orphans
```

<BigValue data={orphan_counts} value=orphans_total title="Orphan files (all)" fmt='#,##0'/>
<BigValue data={orphan_counts} value=orphans_pages title="Orphan pages (excl. assets/scraped)" fmt='#,##0'/>
<BigValue data={orphan_counts} value=orphans_bulk title="Bulk captures (assets + scraped)" fmt='#,##0'/>

Bulk captures (`home-wiki/assets/`, `home-wiki/inbox/scraped/`) are data drops never
meant to be linked — they are counted separately, not hidden.

## Sample orphan pages (largest first)

```sql orphan_sample
select path, bytes
from meridian.orphans
where not is_bulk_capture
order by bytes desc
limit 25
```

<DataTable data={orphan_sample} rows=25/>
