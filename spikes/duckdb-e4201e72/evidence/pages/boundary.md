---
title: The boundary — two faces & the R5 line
---

```sql as_of
select as_of_root, built_at_str from meridian.as_of_root
```

<Alert status=info>

**as_of_root** <Value data={as_of} column=as_of_root/> · built <Value data={as_of} column=built_at_str/> · this page: **design law, no corpus data**

</Alert>

## The R5 line (ZT's surface catalog, verbatim)

> **And the boundary: no rule or agent decision ever reads the DB — the facts API
> doesn't expose it, so the R5 line (actionable facts = md receipts) is structural,
> not discipline.**

## The two faces of one projection

Rung 5 is **one projection with two faces**, rendering the *same* surfaces for
different consumers:

| | **Facts API (agent face)** | **View face (this site)** |
|---|---|---|
| Consumer | rules, agents | humans, workflow orientation |
| Transport | wire op, engine RAM | DuckDB → Evidence → browser |
| Reads DB directly? | **never** — API exposes fact ops, not SQL | yes, but only a **CoW snapshot** |
| Example | `backlinks(page)` → edit-set | [backlinks explorer](/backlinks) |
| Writes? | the wire face writes | never — view is read-only |

Because the agent face **never touches SQL**, R5 is *structural*: an agent
physically cannot make a decision from the DB, because the DB is not on its wire.
An actionable fact reaches an agent only as an **md receipt** through the facts
API. This is not discipline an agent must remember — it is a wall it cannot cross.

## Why the view face is safe (round-3 receipts)

- **Writer exclusivity is real** (round-3 Axis 3): a second process cannot open the
  live `.duckdb` — *not even read-only* — while a writer holds it (3/3 lock-error
  probes). So the view face **must** read a CoW snapshot, never the live file.
- **Snapshots are instant + disposable** (host preflight): `cp -c live.duckdb
  snap.duckdb` is an APFS clone; opens read-only with correct counts; delete =
  re-clone.
- **Every page is root-stamped** (`as_of_root` banner, from `roots` inside the
  snapshot) — the honest-tense law made visible: a page never claims to be newer
  than the root it was built from.

## EQ answers (committed)

- **EQ1 — Snapshot contract.** Clone **on root-change** (a new `roots` row), not
  per-build or per-query. Retention: keep the latest; older snapshots are garbage
  the instant a newer root exists. Staleness UX: **banner-only** while lag is
  within one root; **refuse-to-render** past a configurable lag so a dashboard
  never silently serves a stale root. Minimal honest contract: *render only what
  the snapshot's own `as_of_root` proves.*
- **EQ2 — What earns a wire op.** A projection earns a wire (RAM) fact op when an
  **agent mutates state from it** — [backlinks](/backlinks) drives rename
  edit-sets, the [compound front door](/compound) drives the ingest entry check.
  It stays dashboard-only when it merely **orients** ([hubs](/hubs),
  [tags](/tags)). The R5 catch: if an agent is about to *decide* from a view, that
  projection was mis-classified — promote it to a fact op or the decision is a
  violation.
- **EQ3 — Compound front door.** Output shape `(md_path, change_kinds, last_seq)`
  over a root range suffices as compound's input; missing before turnkey: a
  path→session resolver and richer `change` granularity (create/delete/rename).
  See [compound](/compound).
- **EQ4 — Evidence verdict.** Evidence is the **ratified view-face reference**, not
  a throwaway: its extract-to-Parquet step is the datalake bridge a plainer face
  would still have to build ([parquet](/parquet)). The agent face is *not* Evidence
  — it is the wire fact-op API. Kill criterion (pre-registered): if the human face
  ever needed the *live* DB (not a snapshot) to be correct, Evidence would fail —
  it did not, so it passes.
