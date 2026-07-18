#!/usr/bin/env bash
# Axis 2 (SIZE) + WAL growth. Uses a dedicated db so wiki.duckdb stays pristine.
# SPIKE_NO_CHECKPOINT=1 holds the WAL so we can stat sizes BEFORE the explicit
# CHECKPOINT (separates "after +10k appends" from "after CHECKPOINT").
set -uo pipefail
cd "$(dirname "$0")/.."
BIN=./target/release/spike
DB=data/size.duckdb
sz() { stat -f '%z' "$1" 2>/dev/null || echo 0; }
rm -f "$DB" "$DB.wal"

echo "== SIZE / WAL =="
$BIN build "$DB" >/dev/null
echo "corpus_bytes            106154404  (101.2 MB, measured ingest)"
echo "S0_after_build_ckpt_db  $(sz "$DB")"
echo "S0_after_build_ckpt_wal $(sz "$DB.wal")"

# append 10k, NO shutdown checkpoint → WAL persists after close
SPIKE_NO_CHECKPOINT=1 $BIN append-bench "$DB" 10000 | grep -E 'appends_per_sec|latency_us_p50|latency_us_p99|wall_s'
echo "S1_after_10k_db         $(sz "$DB")"
echo "S1_after_10k_wal        $(sz "$DB.wal")   <- WAL holds the 10k appends (uncheckpointed)"

# explicit checkpoint folds WAL into main file
$BIN checkpoint "$DB" >/dev/null
echo "S2_after_checkpoint_db  $(sz "$DB")"
echo "S2_after_checkpoint_wal $(sz "$DB.wal")"
