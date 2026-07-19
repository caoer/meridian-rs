---
title: Boundary
---

```sql as_of
select banner, root, built_at from meridian.as_of_root
```

> **<Value data={as_of} column=banner/>**

## The two-faces law

The engine has exactly two faces, and they never blur:

**Facts face (wire):** the facts API serves agents from engine RAM. Agents ask
questions, get facts. No SQL surface, no file handle, no snapshot — the DB is not
reachable from this face at all.

**View face (snapshot):** the DB file is cloned copy-on-write (`snap.duckdb`) and
handed to viewers — Evidence, DuckDB CLI, humans. Read-only, disposable, rebuildable.
This entire dashboard lives on the view face: every chart here reads the snapshot,
never the live file.

## Why this is structural, not discipline

> no rule or agent decision ever reads the DB — the facts API doesn't expose it, so
> actionable facts = md receipts is structural, not discipline.

The R5 line above is the whole argument: you cannot leak DB-dependence into agent
behavior because the surface agents use has no DB in it. Receipts (md files) are the
only actionable facts; the DB is a rebuildable index over them. Delete `snap.duckdb`,
re-clone, rebuild this site — nothing of record is lost. That deletion test is run as
part of this spike's reproducibility proof.
