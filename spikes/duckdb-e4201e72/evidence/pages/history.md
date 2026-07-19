---
title: Per-path history window
---

```sql as_of
select as_of_root, built_at_str from meridian.as_of_root
```

<Alert status=warning>

⚠️ **SYNTHETIC-DEMO churn data** — the `changes` stream is 10,002 synthetic
`update`/`edit` rows over 21 paths, written by the round-3 `churn` harness to
exercise the append log. **This is NOT real history.** as_of_root
<Value data={as_of} column=as_of_root/> · built <Value data={as_of} column=built_at_str/>.

</Alert>

The per-path history window: the last-N `changes` rows for a chosen path, newest
first. In production the path is a bound `?`; here the dropdown lists the 21 paths
the synthetic churn actually touched.

```sql paths
select path, count(*) as changes
from meridian.changes
group by path
order by changes desc, path
```

<Dropdown data={paths} name=path value=path>
  <DropdownOption value=% valueLabel="(most-changed path)"/>
</Dropdown>

```sql window
select seq, ts, kind, change, old_rev, new_rev
from meridian.changes
where path like '${inputs.path.value}'
   or ('${inputs.path.value}' = '%' and path = (
        select path from meridian.changes group by path order by count(*) desc limit 1))
order by seq desc
limit 25
```

<DataTable data={window} rows=15/>

## Change volume per path (synthetic)

<BarChart data={paths} x=path y=changes title="Synthetic changes per path (21 paths)" swapXY=true/>

<Details title="SQL — history window">

```sql
select seq, ts, kind, change, old_rev, new_rev
from changes where path = ?
order by seq desc limit 25;
```

The append log is nearly free (round-3 measured ~13 M rows/s isolated); the 3.2 ms
per-append steady-state cost is projection re-maintenance + WAL fsync, not the log.

</Details>
