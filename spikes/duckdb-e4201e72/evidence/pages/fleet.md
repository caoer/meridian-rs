---
title: Fleet view
---

```sql as_of
select as_of_root, built_at_str from meridian.as_of_root
```

<Alert status=info>

**as_of_root** <Value data={as_of} column=as_of_root/> · built <Value data={as_of} column=built_at_str/> · **data: REAL corpus facts**

</Alert>

The **TASKS.base equivalent as SQL** — task cards across the whole sessions tree,
each row a card with its frontmatter pivoted wide. This is the projection that
"feeds dashboards beyond one vault": the same `frontmatter` melt, pivoted per page.

```sql fleet
with tasks as (
  select distinct path from meridian.frontmatter
  where (key='type' and value='task') and path like '%home-wiki-sessions%'
)
select
  regexp_extract(t.path, 'month=[0-9]{2}/([^/]+)/', 1) as session,
  regexp_extract(t.path, '([^/]+)\.md$', 1) as card,
  trim(split_part(max(case when f.key='status' then f.value end),'#',1)) as status,
  max(case when f.key='owner' then f.value end) as owner
from tasks t
join meridian.frontmatter f on f.path = t.path
group by 1,2
order by session desc, card
```

```sql fleet_by_session
with tasks as (
  select distinct path from meridian.frontmatter
  where (key='type' and value='task') and path like '%home-wiki-sessions%'
)
select
  regexp_extract(path, 'month=[0-9]{2}/([^/]+)/', 1) as session,
  count(*) as cards
from tasks
group by 1
order by cards desc
limit 20
```

<BigValue data={fleet_totals} value=total_cards title="Task cards (sessions tree)" fmt=num0/>
<BigValue data={fleet_totals} value=sessions title="Sessions with tasks" fmt=num0/>

```sql fleet_totals
with tasks as (
  select distinct path from meridian.frontmatter
  where (key='type' and value='task') and path like '%home-wiki-sessions%'
)
select
  count(*) as total_cards,
  count(distinct regexp_extract(path, 'month=[0-9]{2}/([^/]+)/', 1)) as sessions
from tasks
```

## Cards per session (top 20)

<BarChart data={fleet_by_session} x=session y=cards title="Task cards per session" swapXY=true/>

## The fleet, card by card

<DataTable data={fleet} rows=20 search=true>
  <Column id=session/>
  <Column id=card wrap=true/>
  <Column id=status/>
  <Column id=owner/>
</DataTable>

<Details title="Why this is one SQL statement, not a base file per vault">

`TASKS.base` / `FLEET.base` are per-vault Obsidian view files. As SQL over the
projection, the *same* pivot serves every vault the projection ingests — the fleet
view stops being a file you copy into each vault and becomes a query. That is the
"feeding dashboards beyond one vault" catalog item.

</Details>
