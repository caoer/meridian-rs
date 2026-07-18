#!/usr/bin/env bash
# Reproduce every number in REPORT.md from a clean state. Runnable by a stranger:
#   cd spikes/duckdb-e4201e72 && ./scripts/run-all.sh
# Requires: cargo, duckdb CLI on PATH. Writes raw outputs under raw/.
set -euo pipefail
cd "$(dirname "$0")/.."
DB=wiki.duckdb
BIN=./target/release/duckdb-spike

mkdir -p raw
echo ">> building spike binary (release; bundled libduckdb, first build is slow)"
cargo build --release

# NB: bench runs AFTER churn so the changes table is populated and q4_history
# (history window) measures real rows instead of an empty table.
echo ">> [1/7] build (full corpus ingest)"            && $BIN build    "$DB"        | tee raw/build.txt
echo ">> [2/7] append (single-file delta)"            && $BIN append   "$DB"        | tee raw/append.txt
echo ">> [3/7] churn (10k appends, WAL growth)"       && $BIN churn    "$DB" 10000   | tee raw/churn.txt
echo ">> [4/7] bench (reference queries, warm+cold)"  && $BIN bench    "$DB" 200 100 | tee raw/bench.txt
echo ">> [5/7] crash (kill -9 mid-append, reopen)"    && $BIN crash    "$DB"        | tee raw/crash.txt
echo ">> [6/7] attach (writer live, RO attach probes)"&& $BIN attach   "$DB"        | tee raw/attach.txt
echo ">> [7/7] rootstamp (stale-open discard+rebuild)"&& $BIN rootstamp "$DB"       | tee raw/rootstamp.txt
echo ">> done. raw outputs in raw/"
