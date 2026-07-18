---
title: Backlinks explorer
---

```sql asof
select * from wiki.asof
```

<Alert status=info>

**as_of_root** **{asof[0].root_hex}** — snapshot built {asof[0].root_built_at}, source `wiki-snap.duckdb` (CoW clone, read-only). **Provenance: REAL corpus facts.**

</Alert>

Find-references for any page — the r3 reference query `Q1` (`SELECT src_path FROM
links WHERE target = ? AND resolved`), made interactive. Pick a target:

```sql targets
select target, count(*) as in_deg
from wiki.links
where resolved
group by target
order by in_deg desc
limit 200
```

<Dropdown data={targets} name=target value=target label=target defaultValue=schema title="Target page (by inbound-link key)"/>

```sql backlinks
select distinct src_path
from wiki.links
where target = '${inputs.target.value}' and resolved
order by src_path
```

**{backlinks.length}** page(s) link to **{inputs.target.value}** at this root.

<DataTable data={backlinks} rows=15 search=true>
  <Column id=src_path title="Backlinking page"/>
</DataTable>
