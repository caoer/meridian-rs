#!/usr/bin/env bash
# Axis 4 (PERFORMANCE) + axis 5 (QUERYABILITY). Prepares a dedicated query db
# from a fresh build, then concentrates 10k appends onto 20 paths so Q4's
# "history window, last N" has a real hot path to read.
set -uo pipefail
cd "$(dirname "$0")/.."
BIN=./target/release/spike
DB=data/query.duckdb
RUNS="${1:-200}"
rm -f "$DB" "$DB.wal"
$BIN build "$DB" >/dev/null
SPIKE_HOTSET=20 $BIN append-bench "$DB" 10000 >/dev/null 2>&1
$BIN checkpoint "$DB" >/dev/null
$BIN query-bench "$DB" "$RUNS"
echo
$BIN queryability "$DB"
