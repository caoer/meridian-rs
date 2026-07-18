#!/usr/bin/env python3
"""Phase 4 mixed-mode live writer. Run with: uv run --with duckdb python3 live_writer.py <seconds>
Holds spike.duckdb read-write; appends one changes row every ~100ms.
Logs committed seqs (>= 2,000,000) to logs/live_writer.log.
"""
import duckdb, os, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
dur = float(sys.argv[1]) if len(sys.argv) > 1 else 60
con = duckdb.connect(os.path.join(HERE, "spike.duckdb"))
log = open(os.path.join(HERE, "logs", "live_writer.log"), "w")
log.write("connected\n"); log.flush()
end = time.monotonic() + dur
i = 0
while time.monotonic() < end:
    seq = 2_000_000 + i
    con.execute("BEGIN")
    con.execute(
        "INSERT INTO changes VALUES (?, now(), 'live/test.md', 'file', 'modify', 'old', ?)",
        [seq, f"live{i:08d}"],
    )
    con.execute("COMMIT")
    log.write(f"{seq}\n"); log.flush()
    i += 1
    time.sleep(0.1)
con.close()
log.write("closed-clean\n"); log.flush()
