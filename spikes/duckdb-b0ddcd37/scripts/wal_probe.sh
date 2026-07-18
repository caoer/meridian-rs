#!/usr/bin/env bash
# WAL growth WHILE a writer is live (default settings: shows real auto-checkpoint
# behavior at the 16MB threshold as commits accumulate).
set -uo pipefail
cd "$(dirname "$0")/.."
BIN=./target/release/spike
DB=data/wal.duckdb
sz() { stat -f '%z' "$1" 2>/dev/null || echo 0; }
rm -f "$DB" "$DB.wal"
$BIN build "$DB" >/dev/null

echo "== WAL growth (live writer, DEFAULT checkpoint_threshold=16MB) =="
$BIN writer-hold "$DB" >logs/wal_writer.out 2>logs/wal_writer.err &
PID=$!
printf "%-8s %-14s %-14s %s\n" "t" "db_bytes" "wal_bytes" "last_commit"
for t in 0 1 2 3 4 5; do
  sleep 1
  printf "%-8s %-14s %-14s %s\n" "${t}s" "$(sz "$DB")" "$(sz "$DB.wal")" "$(tail -1 logs/wal_writer.err 2>/dev/null)"
done
kill -9 "$PID" 2>/dev/null
wait "$PID" 2>/dev/null
echo "(writer killed)"
