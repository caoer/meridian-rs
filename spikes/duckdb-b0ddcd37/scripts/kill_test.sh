#!/usr/bin/env bash
# Axis 3 (WAL crash durability): kill -9 a live writer mid-append, reopen, verify.
set -uo pipefail
cd "$(dirname "$0")/.."
BIN=./target/release/spike
DB=data/kill.duckdb
sz() { stat -f '%z' "$1" 2>/dev/null || echo 0; }
rm -f "$DB" "$DB.wal"

echo "== KILL -9 CRASH DURABILITY =="
$BIN build "$DB" >/dev/null
echo "seeded build: files=$($BIN verify "$DB" | awk '/^files/{print $2}')"

# start a live writer that commits appends in a loop; keep WAL uncheckpointed so
# there is definitely committed-but-unmerged state in the WAL at kill time.
SPIKE_NO_CHECKPOINT=1 $BIN writer-hold "$DB" >logs/kill_writer.out 2>logs/kill_writer.err &
PID=$!
sleep 2
LAST=$(tail -1 logs/kill_writer.err)
echo "writer live (pid=$PID); ${LAST:-<no progress line yet>}"
echo "wal_bytes_before_kill=$(sz "$DB.wal")"

kill -9 "$PID"
wait "$PID" 2>/dev/null
echo "-> sent SIGKILL to writer (pid=$PID); process dead, file lock released"
sleep 1

echo "== reopen after crash (WAL replay) =="
$BIN verify "$DB"
