---
title: Shape conformance
---

```sql as_of
select as_of_root, built_at_str from meridian.as_of_root
```

<Alert status=info>

**as_of_root** <Value data={as_of} column=as_of_root/> · built <Value data={as_of} column=built_at_str/> · **data: REAL corpus facts**

</Alert>

**Shapes are data → view columns.** A "shape" is a declared set of required
frontmatter keys for a page `type`. Conformance = the fraction of pages of that
type that carry each required key. Here the shape under test is **`type=task`**,
required keys `{ type, status, session, owner }` (observed convention).

```sql shape_pages
select count(distinct path) as task_pages
from meridian.frontmatter
where key='type' and value='task'
```

<BigValue data={shape_pages} value=task_pages title="Pages of type=task (the shape population)" fmt=num0/>

## Per-key coverage vs the required set

```sql coverage
with t as (
  select distinct path from meridian.frontmatter where key='type' and value='task'
), tc as (select count(*) n from t),
req as (
  select unnest(['type','status','session','owner']) as key
)
select
  r.key as required_key,
  count(distinct f.path) as pages_with_key,
  (select n from tc) as total_task_pages,
  round(100.0*count(distinct f.path)/(select n from tc),1) as conformance_pct
from req r
left join meridian.frontmatter f
  on f.key=r.key and f.path in (select path from t)
group by r.key
order by conformance_pct desc
```

<DataTable data={coverage}>
  <Column id=required_key/>
  <Column id=pages_with_key/>
  <Column id=conformance_pct contentType=colorscale/>
</DataTable>

<BarChart data={coverage} x=required_key y=conformance_pct title="Required-key conformance (%)" yMax=100/>

## Fully-conformant pages (all 4 required keys present)

```sql conformance_rollup
with t as (
  select distinct path from meridian.frontmatter where key='type' and value='task'
),
key_hits as (
  select path, count(distinct key) filter (where key in ('type','status','session','owner')) as req_present
  from meridian.frontmatter
  where path in (select path from t)
  group by path
)
select
  count(*) filter (where req_present = 4) as fully_conformant,
  count(*) filter (where req_present < 4) as partial,
  count(*) as total
from key_hits
```

<BigValue data={conformance_rollup} value=fully_conformant title="Fully conformant (4/4 keys)" fmt=num0/>
<BigValue data={conformance_rollup} value=partial title="Partial (< 4 keys)" fmt=num0/>
<BigValue data={conformance_rollup} value=total title="Total task pages" fmt=num0/>

<Details title="Why this is a projection, not a linter">

Conformance is a `group by` over the same `frontmatter` table every other read
surface uses. No separate schema store, no validation pass — the "shape" is a
`req` list unnested against the corpus. Add a type, add a row.

</Details>
