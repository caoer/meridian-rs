---
title: Task board
---

```sql as_of
select as_of_root, built_at_str from meridian.as_of_root
```

<Alert status=info>

**as_of_root** <Value data={as_of} column=as_of_root/> · built <Value data={as_of} column=built_at_str/> · **data: REAL corpus facts**

</Alert>

Kanban board over the corpus: a **task** is any page whose frontmatter declares
`type=task` or carries the `type/task` tag. Status is taken from the `status` key
(inline `# comments` stripped). Note: session **agent files** also carry a free-form
`status:` one-liner — those are excluded here by the task filter, which is exactly
why the board must scope by `type`, not read `status` blindly.

```sql board
with tasks as (
  select distinct path from meridian.frontmatter
  where (key='type' and value='task') or (key='tag' and value='type/task')
)
select trim(split_part(s.value,'#',1)) as status, count(*) as n
from meridian.frontmatter s
where s.key='status' and s.path in (select path from tasks)
group by 1
order by n desc
limit 15
```

<BarChart data={board} x=status y=n title="Task cards by status" swapXY=true/>

<DataTable data={board} rows=15/>

## Cards by owner (top 20)

```sql by_owner
with tasks as (
  select distinct path from meridian.frontmatter
  where (key='type' and value='task') or (key='tag' and value='type/task')
)
select
  coalesce(nullif(trim(o.value),''), '(unassigned)') as owner,
  count(*) as cards
from tasks t
left join meridian.frontmatter o on o.path=t.path and o.key='owner'
group by 1
order by cards desc
limit 20
```

<DataTable data={by_owner} rows=12 search=true/>

<Details title="SQL — board scoped by type, not by raw status">

```sql
with tasks as (
  select distinct path from frontmatter
  where (key='type' and value='task') or (key='tag' and value='type/task')
)
select trim(split_part(value,'#',1)) as status, count(*) n
from frontmatter
where key='status' and path in (select path from tasks)
group by 1 order by n desc;
```

</Details>
