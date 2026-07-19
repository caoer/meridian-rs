---
title: Hub / coverage
---

```sql as_of
select as_of_root, built_at_str from meridian.as_of_root
```

<Alert status=info>

**as_of_root** <Value data={as_of} column=as_of_root/> · built <Value data={as_of} column=built_at_str/> · **data: REAL corpus facts**

</Alert>

Inbound-link distribution: how the corpus's attention concentrates. A **hub** is a
target with many resolved inbound links. Coverage = the shape of the tail.

```sql hub_totals
select
  count(distinct target) as linked_targets,
  count(*) as resolved_links,
  max(inbound) as top_hub_inbound
from (
  select target, count(*) as inbound
  from meridian.links where resolved group by target
)
```

<BigValue data={hub_totals} value=linked_targets title="Distinct linked targets" fmt=num0/>
<BigValue data={hub_totals} value=resolved_links title="Resolved links" fmt=num0/>
<BigValue data={hub_totals} value=top_hub_inbound title="Top hub inbound" fmt=num0/>

## Top 25 hubs

```sql hubs
select target as page, count(*) as inbound_links
from meridian.links
where resolved
group by target
order by inbound_links desc, page
limit 25
```

<BarChart data={hubs} x=page y=inbound_links title="Top 25 hubs by inbound links" swapXY=true/>

## Inbound-degree distribution (the long tail)

```sql dist
with d as (
  select target, count(*) as inbound
  from meridian.links where resolved group by target
)
select
  case when inbound=1 then '1'
       when inbound<=3 then '2-3'
       when inbound<=10 then '4-10'
       when inbound<=50 then '11-50'
       else '50+' end as inbound_bucket,
  count(*) as targets,
  sum(inbound) as links
from d
group by 1
order by min(inbound)
```

<DataTable data={dist}/>
<BarChart data={dist} x=inbound_bucket y=targets title="Targets per inbound-degree bucket"/>

<Details title="SQL & the wire-op question (EQ2)">

```sql
select target, count(*) inbound from links where resolved
group by target order by inbound desc limit 25;
```

Hub ranking is **dashboard-only** — it orients humans, no agent decides from it.
Contrast [backlinks](/backlinks): the *set* of inbound links for one page IS a wire
op (rename edit-sets). Criterion: a projection earns a wire op when an **agent
mutates state from it** (edit-set, entry check); it stays dashboard-only when it
merely **orients** a human or workflow.

</Details>
