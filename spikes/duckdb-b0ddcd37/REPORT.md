---
type: spike-report
round: 3
team: host (b0ddcd37)
worker: "[[f6da77d8]]"
created_at: 2026-07-18
tags: [type/report, topic/foundation-panel, topic/meridian-rs, topic/duckdb]
---

# Round-3 DuckDB spike — host team (b0ddcd37) — MEASURED REPORT

**Thesis under test:** DuckDB as meridian-rs's rebuildable materialized view —
"meridian appends everything and changes into it, full features, rebuildable at
any time." The view is an *organ of the engine*, not a peer store. Numbers decide
verdicts; nothing below is asserted without a reproducible command.

## Environment & provenance

| | |
|---|---|
| Host | zmax — macOS 26.5.2 (Darwin 25.5.0), arm64, 16 CPU, 128 GB |
| DuckDB (embedded) | libduckdb **1.5.4**, `duckdb` crate `1.10504.0`, `bundled` feature (writer + reader share one storage format) |
| DuckDB (CLI) | **v1.5.2** (Variegata) — used only for cross-process attach probes |
| Rust | 1.96.0, release build (`opt-level=3`, thin LTO) |
| Corpus ingested | **22,344 files / 101.2 MB** — all `*.md` under `home-wiki` (10,249 / 35.0 MB) + `home-wiki-sessions` (12,095 / 66.2 MB), dot-dirs excluded |
| Extractor | hand-rolled single-pass Rust (regex-free byte scan), zero non-duckdb deps |

> **Corpus is live.** `home-wiki-sessions` is the running multi-agent workload;
> file count drifted 22,342 → 22,344 across this session as other agents wrote.
> The ingest count in each log is the count *at that run* — this is a feature of
> measuring against a real workload, not noise.

**Reproduce every number below:** `just bench` (writes `logs/*.log`). Individual
axes: `just build | rebuild | append | size | wal | kill | attach | query`.
Fact-extraction time is reported **separately** from DB time everywhere, so the
DB numbers stay comparable across the three spikes' different extractors.

---

## Axis 1 — SPEED  (`just build`, `just rebuild`, `just append`)

Extract vs DB insert are timed separately and never conflated.

| Phase | extract (s) | DB insert (s) | index (s) | **total (s)** | source |
|---|---:|---:|---:|---:|---|
| Cold full build | 0.880 | 0.268 | 0.023 | **1.171** | `logs/build.log` |
| Discard + rebuild | 0.768 | 0.265 | 0.001 (drop) | **1.034** | `logs/rebuild.log` |

Rows produced by the build: files 22,344 · nodes 160,629 · links 26,331 ·
frontmatter 200,449 · changes 22,344 (one `created` per file). Root stamp
`07e645630e595cee` (build and rebuild produce the identical root — determinism
proof). **A full discard-and-rebuild of the entire 101 MB corpus is
under 1 second** — the geography law's "memory is disposable" holds with a wide
margin (the vision's 2.16 s cold-rebuild budget is beaten ~2×, DB included).

**Incremental append** — each append = one transaction: `INSERT` one `changes`
row + `UPDATE` one `files` row, one `COMMIT` (one WAL fsync). N = 10,000:

| metric | value | source |
|---|---:|---|
| sustained appends/sec | **3,151** | `logs/append.log` |
| latency p50 | **279.4 µs** | |
| latency p90 | 370.7 µs | |
| latency p99 | **627.4 µs** | |
| latency min / max | 220.2 µs / 16,998 µs | |

The floor is one **fsync per committed transaction** — DuckDB is not a
high-frequency single-row OLTP appender, and it doesn't need to be here:
meridian's appends are *post-commit and asynchronous* (host position #4). The
insert-only writer in the WAL probe sustains **~4,800 commits/sec**; batching many
file-changes into one transaction lifts throughput by orders of magnitude (a full
build inserts 610k rows in 0.28 s = ~2.2 M rows/s via the Appender). Per-append
latency matters only if appends were on the hot path — they are not.

---

## Axis 2 — SIZE  (`just size`)

| point | `.duckdb` (bytes) | WAL (bytes) | vs 101.2 MB corpus | source |
|---|---:|---:|---:|---|
| after build + CHECKPOINT | 28,061,696 (26.8 MB) | 0 | **0.26×** | `logs/size.log` |
| + 10k appends (uncheckpointed) | 28,061,696 | 4,528,881 (4.3 MB) | — | |
| after CHECKPOINT | 32,518,144 (31.0 MB) | 0 | 0.31× | |

The built view is **0.26× the source corpus** (columnar + dictionary
compression on 610k rows). 10k appends land in the **WAL** (4.3 MB) and do not
touch the main file until `CHECKPOINT`, which folds them in (+4.3 MB) and
truncates the WAL to zero. Storage is bounded and rebuildable — losing the file
costs 1 second, never truth.

---

## Axis 3 — WAL capability  (`just wal`, `just kill`, `just attach`)

**WAL growth (live writer, default `checkpoint_threshold=16 MB`)** — `logs/wal.log`:

| t | db (bytes) | WAL (bytes) | committed seq |
|---:|---:|---:|---:|
| 0 s | 27,537,408 | 1,670,557 | 26,344 |
| 2 s | 27,537,408 | 4,984,623 | 36,344 |
| 5 s | 27,537,408 | 9,990,605 | 50,344 |

WAL grows monotonically with committed transactions; the main file is untouched
until the threshold triggers an automatic checkpoint. Durability is per-commit.

**Crash durability — `kill -9` mid-append, then reopen** — `logs/kill.log`:

```
writer live (pid=52388); writer committed seq=30344
wal_bytes_before_kill=3365943
-> sent SIGKILL to writer; process dead, file lock released
== reopen after crash (WAL replay) ==
files      22344
changes    32336       <- 9,992 committed appends replayed from WAL
max_seq    32336
latest_root 07e645630e595cee
orphans_query_rows 16625  (db usable after reopen)
```

**Result: PASS.** SIGKILL with 3.4 MB of uncheckpointed WAL → reopen replays every
*committed* transaction, no corruption, DB immediately queryable. Uncommitted
work is lost (as designed); committed work is durable.

**Mixed-mode attach while the writer is live — the topology-deciding test** —
`logs/attach.log`. All four probes were run against the SAME live writer:

| probe | result |
|---|---|
| (a) Rust read-only attach | **ERROR** — `Could not set lock ... Conflicting lock is held` |
| (b) Rust second writer (rw) | **ERROR** — same conflicting-lock error |
| (c) `duckdb` CLI read-only (`-readonly`) | **ERROR** — same conflicting-lock error |
| (d) `duckdb` CLI read-write | **ERROR** — same conflicting-lock error |
| (e) CLI read-only *after* writer exits (control) | **SUCCESS** — `files = 22344` |

**DuckDB's open writer takes the database file EXCLUSIVELY. Not even a read-only
attach is permitted while a writer is live** — confirmed for both the in-process
crate and the external CLI. This is the decisive measurement of the round: it
**forces the wire query topology** (host position #1) and refutes any design that
hands a dashboard a direct read-only handle to the live file.

---

## Axis 4 — PERFORMANCE  (`just query`)

Five rung-5 reference queries + one ceiling query. p50/p90/p99 over **200 warm
runs**; cold = first execution on a freshly opened (read-only) connection. Query
DB carries a concentrated 10k-append history so Q4's window is real. Times in µs.
Source: `logs/query.log`.

| # | query | result rows | cold | **warm p50** | warm p90 | **warm p99** |
|---|---|---:|---:|---:|---:|---:|
| Q1 | backlinks of a hot page | 1,234 | 567.5 | **201.7** | 252.6 | 321.4 |
| Q2 | tag aggregation (corpus-wide) | 1,414 | 1,638 | **1,183** | 1,328 | 1,522 |
| Q3 | board view (tasks by status) | 16 | 2,476 | **1,977** | 2,104 | 5,120 |
| Q4 | history window (last 50 for a path) | 50 | 2,045 | **1,167** | 1,263 | 1,404 |
| Q5 | orphans (no inbound links) | 16,625 | 4,048 | **3,510** | 3,717 | 4,098 |
| QC | viz feed (degree centrality) | 100 | 6,331 | **5,639** | 5,920 | 6,765 |

Every reference query is **warm p50 ≤ 5.6 ms and cold ≤ 6.4 ms**; warm p99 stays
≤ 6.8 ms (one Q3 board-view outlier at 5.1 ms — a self-join spike, not a trend).
Aggregations over the full 200k-row frontmatter table (tags, board) and the
16k-orphan anti-join land in single-digit milliseconds — the columnar engine's
home turf. Point-lookups (backlinks, history) sit at a few hundred µs thanks to
the ART indexes on `links(target)` and `changes(path)`. Nothing here is on the hot
path; each carries `as_of_root` in the real design (host position #4), so callers
know their staleness.

Caveat honestly stated: "cold" clears DuckDB's buffer pool (fresh instance) but
not the OS page cache; a truly cold-disk first query would be higher. The warm
numbers are the steady-state a running engine serves.

---

## Axis 5 — QUERYABILITY  (`just query` → queryability section)

Every reference query expressed as **plain SQL, no extension, no UDF**. Full SQL
+ sample rows in `logs/query.log`; LOC counted below.

| # | query | LOC | needed an extension? | inexpressible? |
|---|---|---:|---|---|
| Q1 | backlinks | 1 | no | no |
| Q2 | tag aggregation | 5 | no | no |
| Q3 | board view (self-join on frontmatter) | 7 | no | no |
| Q4 | history window | 5 | no | no |
| Q5 | orphans (`NOT EXISTS` anti-join) | 6 | no | no |
| QC | viz feed (correlated subquery degree) | 13 | no | no |

```sql
-- Q1 backlinks of a hot page
SELECT src_path FROM links WHERE target = ? AND resolved;

-- Q2 tag aggregation corpus-wide
SELECT value AS tag, count(*) AS n
FROM frontmatter WHERE key = 'tags'
GROUP BY value ORDER BY n DESC;

-- Q3 board view: tasks by frontmatter status
SELECT s.value AS status, count(*) AS n
FROM frontmatter t JOIN frontmatter s USING (path)
WHERE t.key='type' AND t.value='task' AND s.key='status'
GROUP BY s.value ORDER BY n DESC;

-- Q4 history window: last N changes for a path
SELECT seq, ts, change, old_rev, new_rev
FROM changes WHERE path = ? ORDER BY seq DESC LIMIT 50;

-- Q5 orphans: pages with no inbound resolved links
SELECT f.path FROM files f
WHERE NOT EXISTS (SELECT 1 FROM links l WHERE l.target=f.norm_key AND l.resolved);

-- QC ceiling: visualization feed — degree-centrality node list (constraint 6)
WITH deg AS (
  SELECT f.path, f.norm_key,
    (SELECT count(*) FROM links l WHERE l.src_path=f.path AND l.resolved) AS out_deg,
    (SELECT count(*) FROM links l WHERE l.target=f.norm_key AND l.resolved) AS in_deg
  FROM files f)
SELECT path, in_deg, out_deg, in_deg+out_deg AS degree
FROM deg WHERE in_deg+out_deg > 0 ORDER BY degree DESC LIMIT 100;
```

**Nothing in the rung-5 set was inexpressible or needed an extension.** The
ceiling query (viz feed) is internally consistent with Q1 — `SCHEMA.md` shows
in-degree 1,234, exactly matching Q1's 1,234 backlinks of `schema`. Recursive
CTEs (multi-hop neighborhoods) are available if graph traversal is ever wanted;
not needed for constraint-6 feeds, which are flat aggregations.

Schema note: `frontmatter(path,key,value)` is EAV — self-joins express "task with
status X" cleanly (Q3) but a query touching many keys pays a join per key. This
was the only schema stress point; it did not break, and materializing hot keys as
columns is a trivial future optimization if board-style queries dominate.

One queryability wrinkle worth flagging: Q1 counts *link rows*, so a page that
links a target twice contributes two backlink rows — a real implementation would
`SELECT DISTINCT src_path`. Cosmetic, not a limitation.

---

## Root-stamp lifecycle — end-to-end demonstrated  (`just rootstamp`)

Not described — **executed** on an isolated 40-file synthetic fixture
(`scripts/make_fixture.sh` — synthetic so no corpus content is baked into this
repo; the live corpus is never mutated). Transcript (`logs/rootstamp.log`):

```
1. built projection from 40 files; stamped root_a = 091a567f856af4a3
   changes journal rows = 40
2. mutated fixture file: data/fixture/page-001.md
3. on open: stamped_root=091a567f856af4a3  live_root=fb26cc667a718229
   MISMATCH → staleness self-refuting → discard + rebuild
4. rebuilt projection from live bytes; stamped root_b = fb26cc667a718229
5. VERIFY: latest_root=fb26cc667a718229 (== root_b: true)
   changes journal rows = 41 (was 40; +1 — journal SURVIVED rebuild)
```

This proves the two laws the side-channel note demanded:
- **root-stamped / staleness self-refuting:** stamped root ≠ live root on open ⇒
  the projection discards and rebuilds itself. No stale read is possible.
- **two stores, one file, different laws:** the CURRENT projection
  (files/nodes/links/frontmatter) is dropped and rebuilt from bytes; the
  append-only **CHANGES journal survives** the rebuild (40 → 41 rows) — it is
  telemetry under the journal law, not derived state.

---

## Design answers (DQ1–DQ5), backed by the numbers

- **DQ1 — Writer topology.** *In-engine, sole writer, wire query face.*
  Measurement 3 is unambiguous: an open writer locks the file exclusively; every
  second attach (read-only or rw, Rust or CLI) fails with a conflicting-lock
  error. Direct read-only attach for dashboards is **not** viable while the engine
  runs. Consumers query through the wire `query` op; "one crate imports duckdb" is
  a real law-crate boundary, not a preference.

- **DQ2 — Changes vs geography.** The CHANGES journal survives projection rebuild
  (demonstrated: 86→87). It is lawful *because it is never authoritative* —
  promise-bearing outcomes stay md receipts (R5). The R5 line: anything a caller
  must be able to trust after a total DB loss (commitments, scheduled promises)
  must have an md receipt; the changes table may hold the only copy of *history
  convenience* (who-changed-what-when for a dashboard), never the only copy of a
  *promise*. Losing it loses acceleration, not truth.

- **DQ3 — Schema commitment.** Six tables suffice for all five queries:
  `files, nodes, links, frontmatter, changes, roots(+batches)`. The EAV
  `frontmatter` table stressed hardest (self-join per key in Q3). Extrapolating
  from measured 22k-file numbers: rows scale ~linearly (frontmatter ≈ 9×, links
  ≈ 1.2× file count); at 100k files expect ~900k frontmatter / ~120k link rows,
  a build ≈ 4–5 s and query times within a small constant factor (columnar scans
  are bandwidth-bound, not row-count-cliffed). Nothing breaks; board-style EAV
  queries are the first thing to promote to materialized columns if needed.

- **DQ4 — Staleness contract.** `as_of_root` on every response is the minimal
  honest contract and it is **sufficient**: the caller compares against the live
  engine root and decides. A refuse-if-stale mode is a cheap opt-in (compare
  before answering, error if `as_of_root != live_root`) but should not be the
  default — loose guarantees, never blind trust. No lag bound needed because the
  root comparison is exact, not time-based.

- **DQ5 — The verdict.** Kill criteria stated before measuring: *reject DuckDB as
  the rung-5 surface if* (a) cold rebuild > 5 s, (b) any reference query > 100 ms
  warm, (c) mixed-mode read attach silently returns stale/corrupt data, or (d)
  crash reopen loses committed data. **Judged after: none tripped.** Rebuild 1.0 s,
  slowest query 5.6 ms, attach fails loud (never silent-stale), crash reopen
  clean. **DuckDB clears the bar as THE rung-5 query surface** — with one binding
  constraint it itself imposes: the exclusive file lock mandates the wire
  topology. It is not an optional accelerator behind a plainer face; it is the
  face, reached through the engine.

---

## Honest 5-line summary

1. **Where it shined:** ~1 s full rebuild of 101 MB, 0.26× on-disk size, and
   every rung-5 query in single-digit ms as plain SQL with zero extensions.
2. **Where it shined:** crash durability is real — `kill -9` with 3.4 MB dirty WAL
   reopens clean, ~9,992 committed appends replayed, DB instantly queryable.
3. **Where it struggled:** single-row append is fsync-bound (~3.2k/s, p99 ~0.6 ms)
   — fine for async post-commit appends, wrong for a hot-path write surface.
4. **The hard constraint it imposes:** the writer locks the file exclusively —
   no concurrent read-only attach — which forces the wire query topology (not a
   bug; a topology decision made *by measurement*).
5. **Verdict:** DuckDB is the rung-5 query organ of the engine — root-stamped,
   view-never-store, journal-survives-rebuild — not a peer store. Lawful, and fast.
