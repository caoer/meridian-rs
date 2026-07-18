# DuckDB Measurement Spike — meridian-rs (0523d4bb)

Date: 2026-07-18 · Host: macOS (Darwin 25.5.0, arm64)
DuckDB CLI v1.5.2 (Variegata) · python duckdb module 1.5.4 via `uv run --with duckdb` (used for the append harness and query bench; CLI used for build, rebuild, crash-recovery reopen, and the attach matrix — the two versions interoperated on the same file, including CLI 1.5.2 replaying a WAL written by 1.5.4).

Every number below was measured in this session. Log files live in `logs/` in this directory.

## Corpus actually ingested (logs/extract.log, logs/count_delta.log)

| metric | value |
|---|---|
| files ingested | **22,343** |
| bytes ingested | **106,154,454** (101.24 MiB) |
| read failures | 0 (logs/extract.stderr.log is empty) |

Roots: `/Users/Shared/projects/home-wiki` (prefix `home-wiki/`) + `/Users/Shared/projects/home-wiki-sessions` (prefix `home-wiki-sessions/`), excluding dot-directory components. Count is one higher than a `find -not -path '*/.*'` count (22,342) because one corpus file is literally named `.md` (`home-wiki-sessions/year=2026/month=07/16-15-mdfs-campaign/agents/.md`) — a dot-*file*, which the amendment's dot-*directory* exclusion does not remove (logs/count_delta.log).

## Phase 1 — extract vs insert split

Extract (`extract.py`, python3.14 stdlib, single-threaded — logs/extract.log):

| stage | wall time |
|---|---|
| walk + parse (hashing, headings, links, frontmatter, tags) | **1.920 s** |
| CSV write | 0.529 s |
| total | 2.448 s |

Rows extracted: files 22,343 · nodes 143,916 · links 28,556 · frontmatter 177,771 · tags 28,664.

DB insert (`build.sql` via CLI `.timer on` — logs/build.log):

| step | real |
|---|---|
| COPY files | 0.065 s |
| COPY nodes | 0.121 s |
| COPY links | 0.020 s |
| COPY frontmatter | 0.086 s |
| COPY tags | 0.034 s |
| **sum of COPYs** | **0.326 s** |
| whole build process (DDL + COPY + verify, `time`d) | **0.394 s wall** |

Extract dominates insert ~6:1. Full cold pipeline (extract + build) ≈ 2.85 s for 101 MiB.

## Phase 2 — rebuild flow (rebuild.sh; logs/rebuild.log, logs/rebuild2.log)

Simulated root mismatch → single transaction: DELETE from files/nodes/links/frontmatter/tags (changes/roots untouched) → re-COPY all 5 CSVs → new root stamp.

| run | wall | changes rows before → after |
|---|---|---|
| rebuild #1 (empty changes) | **0.537 s** | 0 → 0 |
| rebuild #2 (after 10k appends) | **0.538 s** | **10,000 → 10,000** — survived |

`roots` accumulated 3 stamps across build + 2 rebuilds (logs/final_state.log).

## Size table (logs/build.log, checkpoint.log, rebuild.log, post_appends_sizes.log, final_state.log)

| point | .duckdb bytes | .wal bytes | ratio vs 106,154,454-byte corpus |
|---|---|---|---|
| post-build | 25,702,400 | none | **0.242** |
| post-build + explicit CHECKPOINT | 25,702,400 (unchanged) | none | 0.242 |
| post-rebuild #1 (+ CHECKPOINT) | 48,771,072 | none | 0.459 |
| post-10k-appends (clean close) | 48,771,072 | 0 (deleted on close) | 0.459 |
| post-appends + explicit CHECKPOINT | 48,771,072 (unchanged) | none | 0.459 |
| final (after rebuild #2 + crash/live tests) | 51,654,656 | none | **0.487** |

Findings: (1) the CLI checkpoints on clean exit, so explicit CHECKPOINT afterwards is a no-op; (2) **the file never shrinks** — DELETE+re-COPY nearly doubled it (25.7→48.8 MB); freed blocks are reused (rebuild #2 added only ~2.9 MB) but never returned to the filesystem.

## Phase 3 — incremental appends (incremental.py via uv duckdb 1.5.4; logs/incremental.log)

10,000 iterations, each ONE transaction = INSERT changes row + UPDATE files.file_rev + DELETE+re-INSERT that file's nodes rows (random files, seed 42).

| metric | value |
|---|---|
| total wall | 38.887 s |
| **sustained rate** | **257.2 appends/s** |
| latency p50 | **3.118 ms** |
| latency p90 | 6.996 ms |
| **latency p99** | **15.478 ms** |
| latency max / mean | 95.218 ms / 3.886 ms |

WAL during the run (sampled every 500 iters, full series in logs/incremental.log): grows linearly ~2 KB/append — 1.0 MB @ 500 → 16.1 MB @ 8,000 — then DuckDB auto-checkpointed (default 16 MB WAL threshold): 341 KB @ 8,500, regrew to 3.4 MB @ 10,000. Clean close checkpointed and deleted the WAL (post-close: db 48,771,072 / wal 0). The auto-checkpoint is the likely source of the 95 ms max latency. Note: `nodes` has no index on `path`, so each DELETE scans ~144k rows — 257/s includes that cost.

## Phase 4a — crash test (crash_writer.py; logs/crash_test2.log, logs/crash_writer_committed.log)

Background writer appending single-row transactions flat-out, each commit fsync-logged; `kill -9` mid-run.

- WAL at kill: **7,028,672 bytes** (writer PID 5777, confirmed the lock holder)
- Committed per fsync'd log: **28,806** rows (seqs 1,000,000–1,028,805)
- Reopen with CLI 1.5.2: **no error output, exit 0** — `crash_rows=28806, max_seq=1028805` — **exact match, zero committed transactions lost**
- WAL silently replayed and deleted on reopen (`.wal` gone after; db unchanged at 51,654,656)

Take-1 artifact (logs/crash_test.log): first kill hit the `uv run` wrapper, not the python child — the DB holder survived and the CLI got the lock error below. Operational lesson: kill the actual holder PID.

## Phase 4b — mixed-mode attach matrix (live_writer.py + CLI; logs/attach_matrix.log)

Writer live = second process appending one changes row every ~100 ms, holding the DB read-write.

| # | second process attempt | outcome (verbatim) |
|---|---|---|
| a | `duckdb -readonly spike.duckdb` + SELECT, writer LIVE | **FAIL, exit 1** — `IO Error: Could not set lock on file ".../spike.duckdb": Conflicting lock is held in .../python3.12 (PID 32472) by user caoer115. See also https://duckdb.org/docs/stable/connect/concurrency` |
| b | plain `duckdb spike.duckdb` (RW) + SELECT, writer LIVE | **FAIL, exit 1** — identical message |
| c | `ATTACH '.../spike.duckdb' AS x (READ_ONLY)` from a different primary db, writer LIVE | **FAIL, exit 1** — identical message |
| a2 | `duckdb -readonly` + SELECT, writer STOPPED (clean close) | **SUCCESS, exit 0** — 39,233 total rows; 427 live-writer rows = 427 fsync-logged commits, nothing lost |

**Topology decision input: DuckDB's process lock is all-or-nothing. While any process holds the file read-write, no other process can read it by any route — not `-readonly`, not RW, not READ_ONLY ATTACH.** Queries must go through the writer process (its own connection(s)), or the writer must close/pause. Multi-process read-only concurrency works only when *every* attacher is read-only.

## Phase 5 — reference queries (queries/*.sql, bench.py; logs/bench.log, samples in logs/query_outputs.log)

100 cold + 100 warm runs each. Cold = fresh read-only connection per run (first statement on that connection); warm = same connection after one discarded warmup. OS page cache warm in both (no purge attempted, per protocol).

| query | SQL lines | rows out | cold p50 | cold p99 | warm p50 | warm p99 |
|---|---|---|---|---|---|---|
| q1 backlinks of hot page (`/SCHEMA`, 1,160 inbound) | 5 | 1,160 | 2.029 ms | 3.295 ms | 1.558 ms | 1.939 ms |
| q2 tag aggregation, top 50 | 5 | 50 | 0.690 ms | 1.051 ms | 0.527 ms | 2.008 ms |
| q3 board: */tasks/* by status | 7 | 20 | 1.734 ms | 2.751 ms | 1.396 ms | 1.620 ms |
| q4 history: last 50 changes of hot path (28,806-row path) | 5 | 50 | 2.428 ms | 2.872 ms | 2.106 ms | 2.331 ms |
| q5 orphans: anti-join, count + sample 20 | 15 | 20 | 8.206 ms | 8.850 ms | 8.179 ms | 8.812 ms |
| q6 **ceiling**: 5-table dashboard feed | 27 | 20 | 10.587 ms | 11.435 ms | 10.211 ms | 11.301 ms |

Everything expressible in plain SQL — no extensions needed (regex via built-in `regexp_extract`/`regexp_replace`, lists via `list()`, windowed total via `count(*) OVER ()`, `ANTI JOIN` native). Nothing was inexpressible. Board-view status values found: done 584, dispatched 47, pending 33, review 27, in-progress 27, todo 22, … (logs/query_prep.log).

Ceiling query (q6, 27 lines): top-20 most-changed paths from `changes` joined with `files` (current rev/size), inbound resolved-link counts from `links`, `frontmatter` status, and a correlated `tags` list — a complete dashboard feed in one statement, p99 11.4 ms cold. Output sample in logs/query_outputs.log.

## Limitations (honest)

- **blake2b stand-in for blake3** (stdlib has no blake3); truncated to 16 hex chars.
- **Extractor is naive**: ATX headings only (no setext); fence tracking is a bare ```/~~~ toggle (indented/nested fences in lists can mis-toggle); frontmatter is a top-level `key: value` regex (no nested YAML, no block lists — tags captured only from inline `[a, b]` form); wikilink resolution is basename-only, case-insensitive, so distinct files sharing a basename all "resolve".
- **Cold definition**: fresh DuckDB process/connection state only; the OS page cache stays warm (no purge per protocol). True cold-disk numbers would be higher.
- **Version mix**: CLI 1.5.2 vs python module 1.5.4 on one file. Interop was clean in every direction exercised here (including WAL replay), but it is a mixed-version result.
- **File growth**: DuckDB never truncates the database file; the DELETE-heavy rebuild flow permanently roughly doubles the on-disk footprint (0.24 → 0.46 of corpus). Free blocks are reused, so it plateaus, but "rebuild in place" costs disk. A rebuild-to-fresh-file-and-rename would keep the 0.24 ratio.
- `nodes.path` is unindexed; append latency includes a full `nodes` scan per DELETE. An ART index would likely improve the 257/s figure (not measured).
- Crash/live-writer rows (`crash/test.md`, `live/test.md`) remain in `changes` and are visible in q6's top-2 rows — deliberate, they demonstrate the log surviving everything including `kill -9`.

## Log index

`logs/extract.log` `logs/extract.stderr.log` `logs/count_delta.log` — corpus/extract · `logs/build.log` `logs/checkpoint.log` — build/size · `logs/rebuild.log` `logs/rebuild2.log` — rebuild · `logs/incremental.log` `logs/post_appends_sizes.log` — appends/WAL · `logs/crash_test.log` (take 1, wrapper-kill artifact) `logs/crash_test2.log` `logs/crash_writer_committed.log` — crash · `logs/attach_matrix.log` `logs/live_writer.log` — attach matrix · `logs/query_prep.log` `logs/query_outputs.log` `logs/bench.log` — queries · `logs/final_state.log` — final DB state.

## Addendum — panelist verification pass (0523d4bb, own runs)

Independent re-measurements by the panelist after the worker's report; commands run directly, outputs inline in session transcript.

1. **Attach lock re-verified**: live python writer (RW) → `duckdb -readonly` fails with the identical `IO Error: Could not set lock … Conflicting lock is held …` — confirms the all-or-nothing lock.
2. **ART index REFUTES the report's speculation** (§ Limitations "an index would likely improve 257/s"): on a copy with `CREATE INDEX idx_nodes_path ON nodes(path)`, the same append shape measured **65.1/s, p50 11.3ms, p99 70.5ms** — 4× WORSE than unindexed (index maintenance dominates DELETE+INSERT churn). Scans win at this scale.
3. **Batching does not lift the plateau**: 100 changes/txn → **256 changes/s** (per-batch p50 388ms ≈ 3.9ms/change) — the cost is per-file node churn, not transaction overhead.
4. **Pure changes-append (no current-table churn)**: **3,734/s, p50 0.244ms, p99 0.468ms** (autocommit, single rows). The real-time surface is cheap; the per-row current-table maintenance is the trap.

Design consequence (panelist): real-time = append-only `changes` only; current tables refreshed by wholesale rebuild (0.54s measured) — "rebuild-is-the-update."
