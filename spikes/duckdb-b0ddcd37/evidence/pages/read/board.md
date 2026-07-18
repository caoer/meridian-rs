---
title: Board view — tasks by status & owner
---

```sql asof
select * from wiki.asof
```

<Alert status=info>

**as_of_root** **{asof[0].root_hex}** — snapshot built {asof[0].root_built_at}, source `wiki-snap.duckdb` (CoW clone, read-only). **Provenance: REAL corpus facts.**

</Alert>

The r3 reference query `Q3` (self-join on the EAV `frontmatter` table) — tasks
projected by `status`, then by `owner`. This is the SQL equivalent of a
`TASKS/FLEET.base` view.

## By status

```sql by_status
select trim(split_part(s.value, '#', 1)) as status, count(*) as n
from wiki.frontmatter t join wiki.frontmatter s using (path)
where t.key = 'type' and t.value = 'task' and s.key = 'status'
group by 1
order by n desc
```

<BarChart data={by_status} x=status y=n swapXY=true title="Tasks by status"/>

## By owner (top 20)

```sql by_owner
select o.value as owner, count(*) as n
from wiki.frontmatter t join wiki.frontmatter o using (path)
where t.key = 'type' and t.value = 'task' and o.key = 'owner'
group by o.value
order by n desc
limit 20
```

<DataTable data={by_owner} rows=10>
  <Column id=owner title="Owner (agent short-id)"/>
  <Column id=n title="open+closed tasks" contentType=colorscale/>
</DataTable>
