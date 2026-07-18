---
title: Activity heat map
---

```sql asof
select * from wiki.asof
```

<Alert status=warning>

⚠️ **SYNTHETIC-DEMO** — the change stream is the r3 spike's synthetic churn
(10 000 `modified` rows over 20 paths) plus one `created` row per file at build.
**Timestamps are collapsed to a single ~0.1 s instant**, so the catalog's
directory × *day/week* grain is not expressible on this data — the map below
substitutes **stream × change-kind** and awaits a live appender for real time
buckets. **as_of_root** **{asof[0].root_hex}**.

</Alert>

Activity by wiki stream (top directory under `home-wiki` / `home-wiki-sessions`)
crossed with change kind — the catalog's "activity heat map per directory/stream"
item, honest about the collapsed time axis.

```sql heat
select
  coalesce(nullif(regexp_extract(path, '/(home-wiki|home-wiki-sessions)/([^/]+)/', 2), ''), '(root)') as stream,
  change,
  count(*) as n
from wiki.changes
group by stream, change
order by stream, change
```

<Heatmap data={heat} x=change y=stream value=n title="Change events: stream × kind"/>

## Change kind totals

```sql totals
select change, count(*) as events, count(distinct path) as paths
from wiki.changes
group by change
order by events desc
```

<DataTable data={totals}>
  <Column id=change/>
  <Column id=events title="change events" fmt='#,##0' contentType=colorscale/>
  <Column id=paths title="distinct paths" fmt='#,##0'/>
</DataTable>
