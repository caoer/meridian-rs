---
title: The boundary — two faces of one projection
---

```sql asof
select * from wiki.asof
```

<Alert status=info>

**as_of_root** **{asof[0].root_hex}** — snapshot built {asof[0].root_built_at}, source `wiki-snap.duckdb` (CoW clone, read-only). **Provenance: design statement (no query drives this page's claims).**

</Alert>

## The R5 line — verbatim from ZT's round-4 catalog

> **And the boundary: no rule or agent decision ever reads the DB — the facts API
> doesn't expose it, so the R5 line (actionable facts = md receipts) is
> structural, not discipline.**

## Two faces of one projection

Everything you see in this Evidence site is the **view face**: DuckDB → Parquet →
DuckDB-WASM, for humans and workflow orientation. It renders the *same*
projections as the **facts API** (wire, engine RAM, agents) — but the two faces
never share a handle to the live file. One writer, one projection, two readers
with different laws:

| | **Agent / facts face** | **View face** *(this site)* |
|---|---|---|
| Consumer | agents · wire · engine RAM | humans · workflow orientation |
| Interface | engine fact ops (backlinks, diff, orphans) — **no SQL** | DuckDB → Parquet → DuckDB-WASM |
| Reads from | the live DB (held under an **exclusive** writer lock) | **only** a CoW snapshot, never the live file |
| Authoritative output | an **md receipt** (R5) | nothing — it is a mirror |
| Staleness posture | refuse-if-stale (`require_root`) | banner-only (`as_of_root`) |

The pipeline is linear and one-way: *world-model bytes → sole writer → engine
(live DB) →* either *wire fact ops → agents (RAM, no SQL)* **or** *`cp -c` CoW
snapshot → `npm run sources` → Parquet → this view face → humans*. Agents act on
facts and write md receipts back to the world model; the view face never writes
anything.

- The **agent face** never touches SQL — it calls engine fact ops that return
  values into RAM. An action's authoritative outcome is an **md receipt**, never a
  DB row (R5). Losing the DB costs a ~1 s rebuild, never truth.
- The **view face** (this site) reads **only** a CoW snapshot, never the live
  file — the round-3 spike measured that an open writer takes the DB file
  *exclusively*: not even a read-only attach is permitted while the engine runs.
  So the boundary isn't a rule anyone remembers to follow — it is **structural**:
  the live file is physically unreadable by a second process, and the facts API
  exposes no SQL. Wrong routing is impossible, not merely discouraged.

*Two faces, one projection. The reader always knows their tense; the actor always
writes a receipt.*
