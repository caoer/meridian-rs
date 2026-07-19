---
title: Compound front door
---

```sql as_of
select as_of_root, built_at_str from meridian.as_of_root
```

<Alert status=warning>

⚠️ **SYNTHETIC-DEMO churn data** — history rows are synthetic (see
[history](/history)). Root A → root B values come from the synthetic `batches`
rootstamp step. as_of_root <Value data={as_of} column=as_of_root/> · built
<Value data={as_of} column=built_at_str/>.

</Alert>

**The ccc-compound redesign's front door.** Compound's entry check becomes:
*"everything that changed between root A and root B, scoped to
memos/decisions/results — then read those files."* Pick a batch (a root transition);
the query returns the changed md paths in that window.

```sql batch_pick
select
  seq,
  root_before || ' → ' || root_after as transition,
  root_before, root_after, ts
from meridian.batches
order by seq
```

<Dropdown data={batch_pick} name=batch value=root_before defaultValue={batch_pick[0].root_before}/>

Selected transition — root A → root B:

```sql selected
select root_before as root_a, root_after as root_b, ts as batch_ts
from meridian.batches
where root_before = '${inputs.batch.value}'
```

<DataTable data={selected}/>

## Scoped result — paths matching `memos/ | decisions/ | results/`

```sql scoped_count
select count(*) as n from meridian.changes
where ts <= (select ts from meridian.batches where root_before = '${inputs.batch.value}')
  and regexp_matches(path, '/(memos?|decisions?|results?)/')
```

```sql scoped
select path, count(*) as changes, max(seq) as last_seq
from meridian.changes
where ts <= (select ts from meridian.batches where root_before = '${inputs.batch.value}')
  and regexp_matches(path, '/(memos?|decisions?|results?)/')
group by path
order by changes desc
```

<Alert status=info>

The scoped query returns **<Value data={scoped_count} column=n/> rows**. On this
snapshot that is **0** — the synthetic churn only edited 21 domain/inbox `README.md`
paths, none under memos/decisions/results. **This is the honest empty set, not a
bug:** the *query shape* is the deliverable; real compound inputs will populate it.

</Alert>

<DataTable data={scoped}/>

## Mechanism proof — the same query, unscoped (all paths in the window)

```sql unscoped
select
  path,
  count(*) as changes,
  min(seq) as first_seq,
  max(seq) as last_seq
from meridian.changes
where ts <= (select ts from meridian.batches where root_before = '${inputs.batch.value}')
group by path
order by changes desc
limit 25
```

<DataTable data={unscoped} rows=10 search=true/>

<Details title="EQ3 — the compound input contract">

Output shape = **`(md_path, change_kinds, last_seq)`** — a path list plus the change
kinds per path, keyed to a root range. That suffices as compound's input contract.
What's missing before "query the range, read the files" is fully turnkey:

1. a **path→session** resolver so compound scopes to *its own* session subtree, and
2. `change` granularity richer than `edit` (create / delete / rename) so compound
   knows whether to ingest a new file or reconcile a moved one.

The query itself — `changes` filtered by root range + path glob — is already
expressible in flat SQL with no extension.

```sql
select path, list(distinct change) as kinds, max(seq) as last_seq
from changes
where seq between :root_a_seq and :root_b_seq
  and regexp_matches(path, '/(memos|decisions|results)/')
group by path;
```

</Details>
