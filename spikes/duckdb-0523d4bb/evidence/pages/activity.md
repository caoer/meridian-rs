---
title: Activity
---

```sql as_of
select banner, root, built_at from meridian.as_of_root
```

> **<Value data={as_of} column=banner/>**

<Alert status=warning>
**SYNTHETIC-DEMO (spike churn)** — every change row below is synthetic spike traffic
(seeds 42–45 + crash/live/pure test writers), NOT real edit history. Shapes only.
</Alert>

```sql top_changed
select regexp_extract(path, '[^/]+$') as file, n_changes
from meridian.activity_by_path order by n_changes desc
```

<BarChart data={top_changed} x=file y=n_changes swapXY=true title="Changes per path (top 20, synthetic)"/>

## Changes over time (minute buckets)

```sql timeline
select minute, change, n from meridian.activity_timeline order by minute
```

<BarChart data={timeline} x=minute y=n series=change title="Changes per minute (synthetic)"/>

## History heat — top-20 paths × minute

```sql heat
select minute, file, n from meridian.activity_heat
```

<Heatmap data={heat} x=minute y=file value=n title="Change heat (synthetic)"/>
