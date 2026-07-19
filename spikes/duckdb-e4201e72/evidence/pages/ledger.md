---
title: Rule-pack cost ledger — NOT RUN
---

```sql as_of
select as_of_root, built_at_str from meridian.as_of_root
```

<Alert status=negative>

🚫 **NOT RUN — no data.** as_of_root <Value data={as_of} column=as_of_root/> · built <Value data={as_of} column=built_at_str/>

</Alert>

This surface has **no honest numbers to show**: the round-3 spike measured
query/append performance, not per-rule step counts over the corpus. **Blocker:**
there is no rule-execution trace table yet — a `rule_steps(rule_id, path, steps,
ms, root)` stream must exist before this ledger can carry a single measured figure.
Building it now would mean **inventing numbers, which the round forbids**.

**What this page will be** (the r2 budget discipline as a meta surface): per-rule
measured cost — how many projection steps each rule-pack rule spends over the whole
corpus, so a rule's budget is *measured*, not asserted.

## The schema it needs (mock — not populated)

```sql mock_schema
select * from (values
  ('rule_id',   'varchar', 'e.g. tag-taxonomy-enforcement'),
  ('path',      'varchar', 'page the rule evaluated'),
  ('steps',     'bigint',  'projection ops the rule consumed'),
  ('ms',        'double',  'wall time for the evaluation'),
  ('root',      'varchar', 'as_of_root the run was measured at')
) as t(column_name, type, note)
```

<DataTable data={mock_schema}/>

## The query it would answer (against that future table)

<Details title="Planned SQL — DO NOT interpret as results">

```sql
-- per-rule cost ledger over the corpus, at a given root
select
  rule_id,
  count(*)            as pages_evaluated,
  sum(steps)          as total_steps,
  round(avg(steps),1) as steps_per_page,
  round(sum(ms),1)    as total_ms
from rule_steps
where root = :as_of_root
group by rule_id
order by total_steps desc;
```

Until `rule_steps` is emitted by the rule engine, this page stays NOT-RUN. No
mock rows are rendered as if measured — the table above is *schema*, labeled.

</Details>

## Why it's honest to ship a NOT-RUN page

A surface you cannot build honestly gets a NOT-RUN page with the blocker stated,
not a fabricated chart. This page is the placeholder that names exactly what
instrumentation rung 5 still owes: a rule-step trace stream.
