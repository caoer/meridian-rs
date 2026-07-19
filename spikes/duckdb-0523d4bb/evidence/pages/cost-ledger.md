---
title: Cost ledger
---

```sql as_of
select banner, root, built_at from meridian.as_of_root
```

> **<Value data={as_of} column=banner/>**

<Alert status=negative>
**NOT RUN — MOCK DATA.** No measured step data exists yet (r2 probe queue).
Every number below is invented to show the ledger's shape only.
</Alert>

**META** — the rule-pack cost ledger: each rule declares its step budget; the probe
queue measures actual runs against it. Schema is real; rows are mock.

```sql mock_ledger
select * from (values
  ('blurb-required',      'wiki-core',    2, 41,  180, 0),
  ('tag-taxonomy',        'wiki-core',    3, 95,  410, 0),
  ('task-shape',          'sessions',     3, 60,  250, 0),
  ('backlink-integrity',  'wiki-core',    5, 210, 890, 0),
  ('frontmatter-created', 'wiki-core',    1, 12,  40,  0),
  ('receipt-attestation', 'effects',      4, 150, 620, 0)
) as t(rule, pack, declared_steps, measured_p50, measured_p99, corpus_runs)
```

<DataTable data={mock_ledger}>
  <Column id=rule/>
  <Column id=pack/>
  <Column id=declared_steps/>
  <Column id=measured_p50 title="p50 (MOCK)"/>
  <Column id=measured_p99 title="p99 (MOCK)"/>
  <Column id=corpus_runs title="corpus runs (0 = never run)"/>
</DataTable>

`corpus_runs = 0` on every row: nothing has been measured. The ledger becomes real
the first time the r2 probe queue writes a run.
