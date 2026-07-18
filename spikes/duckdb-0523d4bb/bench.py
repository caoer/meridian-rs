#!/usr/bin/env python3
"""Phase 5 query benchmark. Run with: uv run --with duckdb python3 bench.py
For each .sql file in queries/ (sorted):
  cold: 100 runs, each in a FRESH read-only connection (first statement executed
        on that connection). DB file stays in OS page cache — 'cold' here means
        cold DuckDB process/connection state (no buffer pool, no compiled plans),
        NOT cold OS cache.
  warm: one read-only connection, 1 discarded warmup run, then 100 timed runs.
Reports p50/p99 per query in ms, plus row count and SQL line count.
"""
import duckdb, glob, os, statistics, time

HERE = os.path.dirname(os.path.abspath(__file__))
DB = os.path.join(HERE, "spike.duckdb")
RUNS = 100

def pct(sorted_ms, p):
    return sorted_ms[min(len(sorted_ms) - 1, int(p / 100 * len(sorted_ms)))]

for path in sorted(glob.glob(os.path.join(HERE, "queries", "*.sql"))):
    name = os.path.basename(path)
    sql = open(path).read()
    sql_lines = len([l for l in sql.splitlines() if l.strip() and not l.strip().startswith("--")])

    cold = []
    nrows = None
    for _ in range(RUNS):
        con = duckdb.connect(DB, read_only=True)
        t0 = time.perf_counter()
        rows = con.execute(sql).fetchall()
        cold.append((time.perf_counter() - t0) * 1000)
        nrows = len(rows)
        con.close()

    con = duckdb.connect(DB, read_only=True)
    con.execute(sql).fetchall()  # warmup, discarded
    warm = []
    for _ in range(RUNS):
        t0 = time.perf_counter()
        con.execute(sql).fetchall()
        warm.append((time.perf_counter() - t0) * 1000)
    con.close()

    cold.sort(); warm.sort()
    print(f"{name} sql_lines={sql_lines} rows={nrows} "
          f"cold_p50={pct(cold,50):.3f} cold_p99={pct(cold,99):.3f} "
          f"warm_p50={pct(warm,50):.3f} warm_p99={pct(warm,99):.3f} (ms, n={RUNS})")
