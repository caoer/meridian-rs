#!/usr/bin/env bash
# Run each reference query against a built DB with the plain duckdb CLI, so a
# stranger can see the actual result sets (not just timings).
#   ./scripts/queries.sh wiki.duckdb
set -euo pipefail
cd "$(dirname "$0")/.."
DB="${1:-wiki.duckdb}"
for q in queries/q*.sql; do
  echo "==================== $q ===================="
  duckdb "$DB" ".read $q"
  echo
done
