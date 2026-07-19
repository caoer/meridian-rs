---
title: Board
---

```sql as_of
select banner, root, built_at from meridian.as_of_root
```

> **<Value data={as_of} column=banner/>**

**REAL corpus data** — task files (`*/tasks/*`) grouped by frontmatter `status`.

```sql status_counts
select coalesce(status, '(no status)') as status, count(*) as tasks
from meridian.board_tasks
group by 1
order by tasks desc
```

<BarChart data={status_counts} x=status y=tasks swapXY=true title="Tasks by status"/>

<DataTable data={status_counts}/>

## Tasks with owner / stream, grouped by status

```sql tasks_detail
select coalesce(status, '(no status)') as status, path, owner, stream
from meridian.board_tasks
order by status, path
```

<DataTable data={tasks_detail} rows=15 groupBy=status groupsOpen=false>
  <Column id=path/>
  <Column id=owner/>
  <Column id=stream/>
</DataTable>
