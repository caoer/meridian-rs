---
title: Orphans & dangling links
---

```sql as_of
select as_of_root, built_at_str from meridian.as_of_root
```

<Alert status=info>

**as_of_root** <Value data={as_of} column=as_of_root/> · built <Value data={as_of} column=built_at_str/> · **data: REAL corpus facts**

</Alert>

Two anti-joins, opposite directions:

- **Orphans** — files whose stem is never the target of a *resolved* link (nothing points in).
- **Dangling links** — link targets that resolve to no file (something points at nothing).

```sql counts
select
  (select count(*) from meridian.files) as total_files,
  (select count(*) from meridian.files f
     where lower(regexp_extract(f.path, '([^/]+)\.md$', 1)) not in
       (select distinct target from meridian.links where resolved)) as orphan_files,
  (select count(*) from meridian.links where not resolved) as dangling_link_rows,
  (select count(distinct target) from meridian.links where not resolved) as dangling_names
```

<BigValue data={counts} value=orphan_files title="Orphan files (no inbound resolved link)" fmt=num0/>
<BigValue data={counts} value=total_files title="Total files" fmt=num0/>
<BigValue data={counts} value=dangling_link_rows title="Dangling link refs" fmt=num0/>
<BigValue data={counts} value=dangling_names title="Distinct dangling names" fmt=num0/>

## Orphans by top-level area

```sql orphans_by_area
with orphans as (
  select f.path
  from meridian.files f
  where lower(regexp_extract(f.path, '([^/]+)\.md$', 1)) not in
    (select distinct target from meridian.links where resolved)
)
select
  regexp_extract(path, 'home-wiki(-sessions)?/([^/]+)/', 2) as area,
  count(*) as orphans
from orphans
where regexp_extract(path, 'home-wiki(-sessions)?/([^/]+)/', 2) <> ''
group by 1
order by orphans desc
limit 20
```

<BarChart data={orphans_by_area} x=area y=orphans title="Orphan files by area (top 20)" swapXY=true/>

## Top dangling names (most-referenced missing targets)

```sql dangling
select
  target as missing_name,
  count(*) as refs,
  count(distinct src_path) as from_pages
from meridian.links
where not resolved
group by target
order by refs desc, target
limit 30
```

<DataTable data={dangling} rows=15 search=true/>

<Details title="SQL — the two anti-joins">

```sql
-- orphans: stem never a resolved target
select f.path from files f
where lower(regexp_extract(f.path,'([^/]+)\.md$',1))
      not in (select distinct target from links where resolved);

-- dangling: link target that resolves to no file
select target, count(*) from links where not resolved group by target;
```

</Details>
