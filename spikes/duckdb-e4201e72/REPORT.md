# DuckDB spike — REPORT (panelist e4201e72, round-3 foundation panel)

Numbers, not opinions. Every figure below shows the command that produced it;
raw stdout is saved under `raw/`. Reproduce everything from clean with
`./scripts/run-all.sh` (requires `cargo` + `duckdb` CLI on PATH).

## Environment

| Fact | Value |
|---|---|
| duckdb CLI | `v1.5.2 (Variegata) 8a5851971f` |
| bundled libduckdb (Rust `duckdb` crate, `features=["bundled"]`) | **`v1.5.4`** (`SELECT version()`) |
| cross-version file compat | files written by bundled **1.5.4**, reopened by CLI **1.5.2** in the crash + attach tests — **worked** (1.x storage format is backward-readable) |
| toolchain | rustc/cargo 1.96.0, macOS (Darwin) |
| extractor | cheap Rust regex walk (`regex` + `walkdir`); fact extraction is NOT the subject — extract and DB time are **timed separately everywhere** |

## Corpus actually ingested (both trees, amended by ZT)

| Metric | Value | Command |
|---|---|---|
| files (`*.md`, excl. dot-dirs) | **22,342** | `duckdb-spike build wiki.duckdb` → `files` |
| bytes ingested | **106,154,404** (101.24 MiB) | `build` → `corpus_bytes` |
| roots (wiki + sessions) | `/Users/Shared/projects/home-wiki` + `/Users/Shared/projects/home-wiki-sessions` | — |

Host baseline was ~22,336 files / ~101 MB; the live sessions tree drifts by a
handful of files between runs (a rebuild mid-session saw 22,343). Reported values
are the actual counts from each run, never the baseline.

## Schema

Protocol minimum, verbatim: `files(path,file_rev,bytes)` ·
`nodes(path,kind,span_start,span_end,hpath,node_rev)` ·
`links(src_path,target,resolved)` · `frontmatter(path,key,value)` ·
`changes(seq,ts,path,kind,change,old_rev,new_rev)` append-only ·
`roots(root,built_at)`.

**One documented extension:** `batches(seq, root_before, root_after, ts)`.
Justification: host commitment 3 and the rootstamp demo require a **root
transition** (`root_before` → `root_after`) per write batch; `roots(root,
built_at)` alone records a single root and cannot express the transition.
Nothing else deviates from the protocol schema.

Row counts after full build: nodes **158,726** (headings), links **26,642**
(wikilinks; 22,885 resolved / 3,757 dangling), frontmatter **188,392**.

---

## Measurement table — the five axes

### AXIS 1 — SPEED  (`raw/build.txt`, `raw/churn.txt`, `raw/append.txt`, `raw/rootstamp.txt`)

| Measurement | Value | How |
|---|---|---|
| **Cold full build, total** | **0.958 s** | `duckdb-spike build wiki.duckdb` |
| — extract phase (walk+read+parse+link-resolve) | 0.718 s (of which file-read 0.358 s) | same, `extract_secs` |
| — DB insert phase (Appender fast path) | **0.240 s** | same, `insert_secs` |
| **Rebuild after discard** (extract + insert) | 0.714 s + 0.481 s = **1.195 s** | `rootstamp` step 5 |
| Single-file append, cold standalone | 11.03 ms | `duckdb-spike append wiki.duckdb` |
| Incremental append, warm steady-state **p50** | **3.21 ms** | `churn` `per_append_p50_us` |
| Incremental append, warm steady-state **p99** | **8.07 ms** (min 1.91 / max 91.04) | `churn` `per_append_p99_us` |
| **Sustained appends/sec over 10k** | **280.3 /s** | `churn wiki.duckdb 10000` |
| Append-log ONLY throughput (isolated, 1 txn) | 100k rows in **7.7 ms** (~13 M rows/s) | `EXPLAIN ANALYZE INSERT … range(100000)` (`raw/changes-only-append.txt`) |

**Reading:** full ingest of 101 MB / 22 k files in **under one second**; DB insert
is a quarter-second. The append throughput ceiling (280/s) is **not** the log —
appending 100 k pure `changes` rows takes 7.7 ms. The 3.2 ms/append cost is
**per-transaction commit (WAL fsync) + full re-projection of the edited file's
child rows** (delete + re-insert nodes/links/frontmatter). Batch N edits into one
transaction and this collapses toward the 13 M rows/s ceiling. → **DQ2 evidence:
the changes stream is nearly free; the cost is projection maintenance + fsync.**

### AXIS 2 — SIZE  (`raw/build.txt`, `raw/churn.txt`)

| Measurement | Value | How |
|---|---|---|
| `.duckdb` after full build | **21,508,096 B** (20.51 MiB) | `build` `db_bytes` |
| ratio vs 101 MB corpus | **0.2026** | `build` `size_ratio_vs_corpus` |
| `.duckdb` after +10k change rows (post-CHECKPOINT) | 23,867,392 B (+2.36 MB) | `churn` `db_bytes_after_ckpt` |
| WAL during 10k appends (auto-ckpt disabled) | 0 → **38,139,302 B** (36.4 MiB) | `churn` `wal_bytes_grown` |
| WAL after `CHECKPOINT` | **0 B** | `churn` `wal_bytes_after_ckpt` |

**Reading:** the projection is ~1/5 the source corpus on disk. WAL is where cost
hides: with auto-checkpoint suppressed (`SET checkpoint_threshold='1TB'`), 10 k
delete-and-re-project appends inflated the WAL to **36 MB** — because each append
re-writes that file's whole node/link/frontmatter set, not a minimal diff. One
`CHECKPOINT` folds it into the main file (+2.4 MB) and truncates WAL to 0. →
checkpoint cadence is a real operational knob, not a detail.

### AXIS 3 — WAL CAPABILITY  (`raw/crash.txt`, `raw/attach.txt`) — the topology-deciding axis

**Crash durability — `duckdb-spike crash wiki.duckdb`:**

```
last_committed_seq_before_kill 10775
--- reopen stdout ---
┌──────────────┬─────────┐
│ changes_rows │ max_seq │
├──────────────┼─────────┤
│        10775 │   10775 │
└──────────────┴─────────┘
reopen_exit_ok true
```

A child process appended in autocommit; `kill -9` (SIGKILL) mid-append. Reopen
(via the **CLI v1.5.2**, cross-version) replayed the WAL: `changes_rows` and
`max_seq` **both = 10775 = exactly the last committed seq**. The in-flight
uncommitted append was atomically discarded. **VERDICT: durable.** No corruption,
no partial row, no manual recovery.

**Mixed-mode attach — `duckdb-spike attach wiki.duckdb`** — a writer holds the
file RW (PID 39122) for 12 s; three read-only probes fire while it is live:

| Probe | Result | Exact output |
|---|---|---|
| (a) 2nd spike process, RO open | **FAIL** | `IO Error: Could not set lock on file "…wiki.duckdb.attach": Conflicting lock is held … (PID 39122) …` |
| (b) `duckdb -readonly …` | **FAIL** (exit≠0) | same lock error |
| (c) `duckdb … ATTACH '…' (READ_ONLY)` | **FAIL** (exit≠0) | same lock error |
| (d) CONTROL: RO open **after** writer released | **OK** | `RO-OPEN ok files=22342` |

**VERDICT: DuckDB takes the file exclusively. A second process cannot read —
not even read-only — while a writer is live.** The writer-vs-writer case is the
same (the crash test's parent could not open RW until the child was killed).
This is the decisive result: **direct cross-process attach is not a query path
while the engine is writing.** It forces the wire topology. → **DQ1 evidence,
and it confirms the host's stated expectation.**

### AXIS 4 — PERFORMANCE  (`raw/bench.txt`) — p50/p99, warm 200 runs, cold 100 runs

`duckdb-spike bench wiki.duckdb 200 100` (run **after** churn, so `q4_history`
has real rows). **warm** = shared connection, repeated (plan+buffers hot);
**cold** = fresh `Connection::open` per run (cold buffer pool). Times in **µs**.

| query | rows | warm p50 | warm p99 | cold p50 | cold p99 |
|---|---:|---:|---:|---:|---:|
| q1 backlinks (hot page) | 38 | **194–217** | 350–506 | 374–458 | 634–676 |
| q2 tags (corpus-wide agg) | 50 | **1,317–1,340** | 1,534–1,625 | 1,541–1,600 | 1,804–1,832 |
| q3 board (tasks by status) | 16 | **4,071–4,179** | 4,609–4,890 | 4,567–4,606 | 6,215–6,386 |
| q4 history (last 20 for path) | 20 | **679** | 1,062 | 866 | 1,080 |
| q5 orphans (no inbound link) | 16,827 | **14,706–14,822** | 15,624–20,800 | 15,074–15,267 | 15,649–16,580 |
| q6 viz (nested graph feed) | 1 | **2,867–2,871** | 4,842–6,485 | 3,080–3,199 | 3,877–3,904 |

**Reading:** the four point-lookup / aggregate queries are **sub-5 ms warm**;
backlinks and history are **sub-1 ms**. Cold is ~1.3–2× warm for small queries
and **converges** for the full-corpus scan (q5), which is scan-bound, not
cache-bound. The two expensive queries (q3 board 4 ms, q5 orphans 15 ms) both do
whole-table `IN`-subqueries over 22 k–188 k rows with no index — DuckDB's
vectorized scan eats it without tuning. → **DQ3 evidence: no query stressed the
schema to breaking; orphans is the ceiling of the "normal" set at 15 ms.**

### AXIS 5 — QUERYABILITY  (`queries/*.sql`)

| query | SQL LOC | extension/UDF needed | expressible? |
|---|---:|---|---|
| q1 backlinks | 4 | none | yes |
| q2 tags | 6 | none | yes |
| q3 board | 10 | none | yes |
| q4 history | 7 | none | yes |
| q5 orphans | 6 | none (`regexp_extract` built-in) | yes |
| q6 viz (ceiling) | 24 | none (`list()`, STRUCT literal built-in) | yes |

**Nothing needed an extension or a UDF. Nothing was inexpressible.** The ceiling
query (q6) — my own design — builds a **force-graph visualization feed** for the
1-hop ego network of `LLM_WIKI`: **18 nodes (with degree) + 38 edges** emitted as
nested `LIST<STRUCT>` in a **single row**, consumable verbatim as graph JSON
(proved with `duckdb -json`). The only thing beyond flat SQL that a richer viz
would want is **N-hop traversal**, which is a recursive CTE — supported, untested
here (out of scope). → **DQ3/constraint-6 evidence: the surface reaches nested
graph output natively.**

---

## Evidence → the panelist's design questions (DQ1–DQ5)

The panelist writes the commitments in `round-3/e4201e72.md`; this spike supplies
the numbers each rests on.

- **DQ1 (topology):** Axis 3 attach test — cross-process RO attach **fails** while
  a writer is live (3/3 probes, exact lock errors in `raw/attach.txt`). The wire
  `query` op is the only concurrent query face; direct attach is a
  humans-only-when-idle path.
- **DQ2 (changes vs geography):** Axis 1 — append-log is ~free (13 M rows/s);
  Axis 3 rootstamp — the `changes` table **survived** a full current-table
  rebuild (10,002 rows before and after). History can live in the DB cheaply; the
  R5 line (promise-bearing outcomes as md receipts) is a policy choice the numbers
  don't force either way.
- **DQ3 (schema):** protocol-minimum schema + one documented `batches` table
  answered all six queries; orphans (15 ms, full-corpus `NOT IN`) stressed it
  hardest and still held. Extrapolation to 100 k files: q5 is a linear scan, so
  ≈ 4.5× today ≈ ~65 ms warm — still interactive.
- **DQ4 (staleness):** Axis 3 rootstamp demonstrates root-mismatch → discard →
  rebuild end-to-end; `as_of_root` is a cheap column to stamp on every response.
- **DQ5 (verdict):** see below.

## Kill criteria (stated before measuring) → judged

| Kill criterion (fail ⇒ demote DuckDB to optional accelerator) | Result |
|---|---|
| Full build > 30 s | **PASS** — 0.96 s |
| Reference query p99 > 100 ms warm | **PASS** — worst is q5 at ~21 ms |
| Crash loses committed appends / corrupts file | **PASS** — exact recovery, no corruption |
| `.duckdb` > corpus size | **PASS** — 0.20× |

Every kill criterion passed. The one caveat the numbers raise is **not** a kill:
DuckDB's **single-writer, file-exclusive** lock (Axis 3) means the query surface
**must** be the in-engine wire op — a second process cannot even read concurrently.
That is a topology constraint, not a disqualifier.

## Reproduce

```
cd spikes/duckdb-e4201e72
./scripts/run-all.sh          # build → append → churn → bench → crash → attach → rootstamp
./scripts/queries.sh wiki.duckdb   # see the six reference-query RESULT SETS via the CLI
```

Raw stdout for every run is in `raw/` (`build.txt`, `bench.txt`, `churn.txt`,
`append.txt`, `crash.txt`, `attach.txt`, `rootstamp.txt`,
`changes-only-append.txt`).

## Not-run / honest gaps

- **N-hop recursive graph traversal** — not measured (out of the six queries;
  recursive CTE is available).
- **Concurrent multi-connection reads *within* one process** — not benchmarked;
  the cross-process case (the one that decides topology) is measured and is the
  binding constraint.
- **100 k-file corpus** — not built; the 100 k figure in DQ3 is a linear-scan
  extrapolation from the measured 22 k, labelled as such, not a measured number.
