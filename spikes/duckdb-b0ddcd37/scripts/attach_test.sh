#!/usr/bin/env bash
# Axis 3 (WAL): can a second process attach while the writer is live?
# Tests (a) Rust read-only attach, (b) Rust second writer, (c) duckdb CLI r/o
# and (d) duckdb CLI r/w — each while the Rust writer holds the file. This
# decides the query topology (wire vs direct attach).
set -uo pipefail
cd "$(dirname "$0")/.."
BIN=./target/release/spike
DB=data/attach.duckdb
rm -f "$DB" "$DB.wal"

echo "== MIXED-MODE ATTACH (writer live) =="
$BIN build "$DB" >/dev/null
$BIN writer-hold "$DB" >logs/attach_writer.out 2>logs/attach_writer.err &
PID=$!
sleep 2
echo "writer live (pid=$PID); $(tail -1 logs/attach_writer.err)"
echo

echo "(a) Rust read-only attach while writer live:"
$BIN reader-attach "$DB"
echo
echo "(b) Rust second writer (rw) while writer live:"
$BIN writer-contend "$DB"
echo
echo "(c) duckdb CLI $(duckdb --version | awk '{print $1}') read-only attach while writer live:"
duckdb -readonly "$DB" "SELECT count(*) AS files FROM files;" 2>&1 | sed 's/^/    /'
echo
echo "(d) duckdb CLI read-write attach while writer live:"
duckdb "$DB" "SELECT count(*) FROM files;" 2>&1 | sed 's/^/    /'

kill -9 "$PID" 2>/dev/null
wait "$PID" 2>/dev/null
echo
echo "(writer killed)"
echo "(e) duckdb CLI read-only attach AFTER writer gone (control):"
duckdb -readonly "$DB" "SELECT count(*) AS files FROM files;" 2>&1 | sed 's/^/    /'
