#!/usr/bin/env python3
"""Phase 3 incremental-append harness. Run with: uv run --with duckdb python3 incremental.py

10,000 simulated single-file changes. Each iteration = ONE transaction:
  INSERT one changes row
  UPDATE that file's files.file_rev
  DELETE + re-INSERT its nodes rows (reusing extracted node data)
Random files, fixed seed. Measures per-append latency p50/p99 and sustained
appends/sec; samples spike.duckdb.wal size every 500 iterations.
"""
import duckdb, os, random, statistics, time

HERE = os.path.dirname(os.path.abspath(__file__))
DB = os.path.join(HERE, "spike.duckdb")
WAL = DB + ".wal"
N = 10_000
random.seed(42)

con = duckdb.connect(DB)
paths = [r[0] for r in con.execute("SELECT path FROM files").fetchall()]
# preload node rows per path (only for paths we will touch)
touched = [random.choice(paths) for _ in range(N)]
node_cache = {}
for p in set(touched):
    node_cache[p] = con.execute(
        "SELECT path, span_start, span_end, hpath, node_rev, kind FROM nodes WHERE path = ?", [p]
    ).fetchall()

def wal_size():
    try:
        return os.path.getsize(WAL)
    except OSError:
        return 0

lat = []
wal_samples = []
t_start = time.monotonic()
for i, p in enumerate(touched):
    new_rev = f"{i:016x}"
    rows = node_cache[p]
    t0 = time.perf_counter()
    con.execute("BEGIN")
    con.execute(
        "INSERT INTO changes VALUES (?, now(), ?, 'file', 'modify', "
        "(SELECT file_rev FROM files WHERE path = ?), ?)",
        [i, p, p, new_rev],
    )
    con.execute("UPDATE files SET file_rev = ? WHERE path = ?", [new_rev, p])
    con.execute("DELETE FROM nodes WHERE path = ?", [p])
    if rows:
        con.executemany("INSERT INTO nodes VALUES (?, ?, ?, ?, ?, ?)", rows)
    con.execute("COMMIT")
    lat.append(time.perf_counter() - t0)
    if i % 500 == 0:
        wal_samples.append((i, wal_size()))
t_total = time.monotonic() - t_start
wal_samples.append((N, wal_size()))

lat_ms = sorted(x * 1000 for x in lat)
def pct(p):
    return lat_ms[min(len(lat_ms) - 1, int(p / 100 * len(lat_ms)))]

print(f"appends={N}")
print(f"total_wall_s={t_total:.3f}")
print(f"sustained_appends_per_sec={N / t_total:.1f}")
print(f"latency_ms p50={pct(50):.3f} p90={pct(90):.3f} p99={pct(99):.3f} "
      f"max={lat_ms[-1]:.3f} mean={statistics.mean(lat_ms):.3f}")
print("wal_samples (iter, bytes):")
for it, sz in wal_samples:
    print(f"  {it:6d} {sz}")
print(f"changes_rows={con.execute('SELECT count(*) FROM changes').fetchone()[0]}")
con.close()
print(f"post_close db_bytes={os.path.getsize(DB)} wal_bytes={wal_size()}")
