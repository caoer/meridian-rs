---
title: Compound inputs — root A → root B, scoped
---

```sql asof
select * from wiki.asof
```

<Alert status=warning>

⚠️ **SYNTHETIC-DEMO** — computed over the r3 spike's change stream; the root pair
is the snapshot's single real `batches` row (genesis → head). **as_of_root**
**{asof[0].root_hex}**, source `wiki-snap.duckdb` (CoW clone, read-only).

</Alert>

The ccc-compound **front door** (EQ3): *everything that changed between root A and
root B, scoped to `memos/decisions/results`.* Output shape =
`{path, change_kind, old_rev, new_rev}` — the input contract a compound run reads
before it opens a single file.

## The root pair (from `batches`)

```sql pair
select
  coalesce(root_before, 'genesis (∅)') as root_a,
  root_after as root_b,
  n_rows as batch_rows
from wiki.batches
order by batch_seq desc
limit 1
```

<DataTable data={pair}/>

## Scope summary

```sql scope_summary
select
  case
    when path like '%/memos/%' then 'memos'
    when path like '%/decisions/%' then 'decisions'
    when path like '%/results/%' then 'results'
  end as scope,
  change as change_kind,
  count(*) as n
from wiki.changes
where path like '%/memos/%' or path like '%/decisions/%' or path like '%/results/%'
group by scope, change_kind
order by scope, change_kind
```

<DataTable data={scope_summary}>
  <Column id=scope/>
  <Column id=change_kind/>
  <Column id=n title="changed paths" contentType=colorscale/>
</DataTable>

## The change set (md path list + change kinds)

```sql compound
select
  case
    when path like '%/memos/%' then 'memos'
    when path like '%/decisions/%' then 'decisions'
    when path like '%/results/%' then 'results'
  end as scope,
  path,
  change as change_kind,
  old_rev::varchar as old_rev,
  new_rev::varchar as new_rev
from wiki.changes
where path like '%/memos/%' or path like '%/decisions/%' or path like '%/results/%'
order by scope, path
```

**{compound.length}** changed paths in scope between root A and root B.

<DataTable data={compound} rows=15 search=true>
  <Column id=scope/>
  <Column id=path title="md path"/>
  <Column id=change_kind/>
  <Column id=old_rev title="old_rev (opaque id)"/>
  <Column id=new_rev title="new_rev (opaque id)"/>
</DataTable>
