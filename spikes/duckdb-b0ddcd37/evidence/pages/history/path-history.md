---
title: Per-path history window
---

```sql asof
select * from wiki.asof
```

<Alert status=warning>

⚠️ **SYNTHETIC-DEMO** — the change rows are the r3 spike's 10 000 synthetic churn
events (20 hot paths, timestamps collapsed to one instant). Not a real edit
history. **as_of_root** **{asof[0].root_hex}**, source `wiki-snap.duckdb` (CoW clone, read-only).

</Alert>

The r3 reference query `Q4` (`... FROM changes WHERE path = ? ORDER BY seq DESC
LIMIT 50`) made interactive over the 20 synthetically-churned paths.

```sql hotpaths
select path, count(*) as n_changes
from wiki.changes
where change = 'modified'
group by path
order by n_changes desc
```

<Dropdown data={hotpaths} name=hotpath value=path defaultValue="/Users/Shared/projects/home-wiki/LLM_WIKI.md" title="Churned path"/>

```sql window
select seq, ts, change, old_rev::varchar as old_rev, new_rev::varchar as new_rev
from wiki.changes
where path = '${inputs.hotpath.value}'
order by seq desc
limit 50
```

**{window.length}** change rows in the last-50 window for **{inputs.hotpath.value}**.

<DataTable data={window} rows=15>
  <Column id=seq fmt='#,##0'/>
  <Column id=ts title="timestamp (synthetic)"/>
  <Column id=change/>
  <Column id=old_rev title="old_rev (opaque id)"/>
  <Column id=new_rev title="new_rev (opaque id)"/>
</DataTable>
