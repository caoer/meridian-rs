---
title: Shapes
---

```sql as_of
select banner, root, built_at from meridian.as_of_root
```

> **<Value data={as_of} column=banner/>**

**REAL corpus data** — shapes-are-data: conformance is a SQL projection over frontmatter,
not a validator. Required task keys: `status`, `stream`, `owner`.

```sql conformance
select
  count(*) as task_files,
  round(100.0 * avg(has_status), 1) as pct_status,
  round(100.0 * avg(has_stream), 1) as pct_stream,
  round(100.0 * avg(has_owner), 1) as pct_owner,
  round(100.0 * avg(case when has_status + has_stream + has_owner = 3 then 1 else 0 end), 1) as pct_full_shape
from meridian.shapes
```

<BigValue data={conformance} value=task_files title="Task files"/>
<BigValue data={conformance} value=pct_status title="% with status" fmt='#,##0.0"%"'/>
<BigValue data={conformance} value=pct_stream title="% with stream" fmt='#,##0.0"%"'/>
<BigValue data={conformance} value=pct_owner title="% with owner" fmt='#,##0.0"%"'/>
<BigValue data={conformance} value=pct_full_shape title="% full shape" fmt='#,##0.0"%"'/>

```sql key_conformance
select 'status' as key, round(100.0 * avg(has_status), 1) as pct from meridian.shapes
union all select 'stream', round(100.0 * avg(has_stream), 1) from meridian.shapes
union all select 'owner', round(100.0 * avg(has_owner), 1) from meridian.shapes
```

<BarChart data={key_conformance} x=key y=pct swapXY=true title="% of task files carrying each required key"/>

```sql combos
select has_status, has_stream, has_owner, count(*) as tasks
from meridian.shapes group by 1, 2, 3 order by tasks desc
```

<DataTable data={combos}/>
