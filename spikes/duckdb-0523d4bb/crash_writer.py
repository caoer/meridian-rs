#!/usr/bin/env python3
"""Phase 4 crash-test writer. Run with: uv run --with duckdb python3 crash_writer.py
Appends single-row transactions to changes (seq >= 1,000,000) as fast as possible.
After each COMMIT, logs the seq to crash_writer_committed.log (flushed) so the
last-committed row is known when the process is kill -9'd.
"""
import duckdb, os, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
con = duckdb.connect(os.path.join(HERE, "spike.duckdb"))
log = open(os.path.join(HERE, "logs", "crash_writer_committed.log"), "w")
i = 0
while True:
    seq = 1_000_000 + i
    con.execute("BEGIN")
    con.execute(
        "INSERT INTO changes VALUES (?, now(), 'crash/test.md', 'file', 'modify', 'old', ?)",
        [seq, f"crash{i:08d}"],
    )
    con.execute("COMMIT")
    log.write(f"{seq}\n")
    log.flush()
    os.fsync(log.fileno())
    i += 1
