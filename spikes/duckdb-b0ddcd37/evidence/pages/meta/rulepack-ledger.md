---
title: Rule-pack cost ledger (schema + mock)
---

```sql asof
select * from wiki.asof
```

<Alert status=negative>

🚫 **NOT RUN — MOCK DATA.** No measured rule-pack step data exists yet. The table
below is the *intended schema* with illustrative placeholder numbers, shown to
fix the contract shape only. Nothing here is a measurement. **as_of_root**
**{asof[0].root_hex}** (banner shown for surface parity; the ledger itself carries
no root).

</Alert>

The catalog's meta surface: **per-rule measured steps over the corpus** — the r2
budget discipline made visible. When the rules engine runs over the world model,
each rule reports the steps it took; this page would rank them so the r2 budget
(bounded steps per rule) has a dashboard.

## Intended schema

| column | type | meaning |
|---|---|---|
| `rule_id` | varchar | the rule pack + rule name |
| `corpus_root` | varchar | root the pass ran against (tense) |
| `steps` | bigint | measured engine steps consumed |
| `budget` | bigint | the r2 step budget for this rule |
| `over_budget` | boolean | `steps > budget` |
| `wall_us` | bigint | wall time of the pass |

## Mock rows (illustrative — NOT measured)

```sql mock
select * from (values
  ('links/rename-editset',  1250, 2000, false, 840),
  ('orphans/lint',          3510, 4000, false, 3510),
  ('tags/taxonomy-check',   1183, 1500, false, 1180),
  ('board/status-legal',    1977, 1500, true,  2104),
  ('shape/conformance',      950, 1200, false,  980)
) as t(rule_id, steps, budget, over_budget, wall_us)
order by steps desc
```

<DataTable data={mock}>
  <Column id=rule_id/>
  <Column id=steps contentType=colorscale/>
  <Column id=budget/>
  <Column id=over_budget title="over budget?"/>
  <Column id=wall_us title="wall µs"/>
</DataTable>

**Kill note:** this surface stays schema+mock until the rules engine emits real
step counts. Shipping it with fabricated numbers unlabeled would violate the
round's honest-data law — hence the NOT-RUN banner is load-bearing.
