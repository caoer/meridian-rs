---
title: Backlinks
---

```sql as_of
select banner, root, built_at from meridian.as_of_root
```

> **<Value data={as_of} column=banner/>**

**REAL corpus data** — 22,343 md files; resolved wiki-links only.

```sql targets
select target, inbound from meridian.backlink_targets order by inbound desc
```

## Most-linked targets

<Dropdown data={targets} name=picked value=target label=target title="Pick a target"/>

<DataTable data={targets} rows=15>
  <Column id=target/>
  <Column id=inbound/>
</DataTable>

## Inbound sources for <Value data={inbound_sources} column=target/>

```sql inbound_sources
select target, src_path, n as links
from meridian.backlink_sources
where target = '${inputs.picked.value}'
order by links desc
```

<DataTable data={inbound_sources} rows=20>
  <Column id=src_path/>
  <Column id=links/>
</DataTable>
