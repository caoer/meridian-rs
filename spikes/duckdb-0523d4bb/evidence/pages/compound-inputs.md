---
title: Compound inputs
---

```sql as_of
select banner, root, built_at from meridian.as_of_root
```

> **<Value data={as_of} column=banner/>**

<Alert status=warning>
**SYNTHETIC-DEMO (spike churn)** — change rows are synthetic. Real deployments diff
root-A→root-B; seq milestones stand in for roots here.
</Alert>

This is the ccc-compound front door: "what changed in compound-relevant paths
(memos / decisions / results) between two roots?" Output shape:
**(path, change, n_changes, first_seq, last_seq)**.

```sql milestones_asc
select label, seq from meridian.compound_seq_milestones order by seq
```

```sql milestones_desc
select label, seq from meridian.compound_seq_milestones order by seq desc
```

<Dropdown data={milestones_asc} name=root_a value=seq label=label title="root-A (from seq)"/>
<Dropdown data={milestones_desc} name=root_b value=seq label=label title="root-B (to seq)"/>

```sql scoped
select path, change, count(*) as n_changes, min(seq) as first_seq, max(seq) as last_seq
from meridian.compound_changes_raw
where seq between ${inputs.root_a.value} and ${inputs.root_b.value}
group by path, change
order by n_changes desc
```

```sql scope_summary
select count(distinct path) as paths_touched, sum(n_changes) as total_changes from ${scoped}
```

<BigValue data={scope_summary} value=paths_touched title="Paths touched"/>
<BigValue data={scope_summary} value=total_changes title="Changes in range"/>

<DataTable data={scoped} rows=20/>
