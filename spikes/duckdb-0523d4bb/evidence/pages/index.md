---
title: meridian spike — snapshot views
---

```sql as_of
select banner, root, built_at from meridian.as_of_root
```

> **<Value data={as_of} column=banner/>**

All views read the CoW snapshot `snap.duckdb`, never the live file.

## Read (REAL corpus data)

- [Backlinks](/backlinks) — most-linked targets + inbound sources
- [Orphans](/orphans) — files with zero inbound resolved links
- [Tags](/tags) — corpus-wide tag aggregation
- [Board](/board) — tasks by frontmatter status
- [Shapes](/shapes) — task shape conformance (shapes-are-data)
- [Hubs](/hubs) — inbound-link distribution

## History (SYNTHETIC-DEMO spike churn)

- [Activity](/activity) — history heat over synthetic change rows
- [Compound inputs](/compound-inputs) — root-A→root-B change scan (front door)

## Meta

- [Fleet](/fleet) — per-vault/partition stats + Parquet datalake export
- [Cost ledger](/cost-ledger) — rule-pack cost schema (MOCK, not run)

## Boundary

- [Boundary](/boundary) — the two-faces law
