#!/bin/zsh
# Phase 2 REBUILD flow: simulate root mismatch -> truncate current-state tables
# (NOT changes/roots history) -> re-COPY -> new root stamp. Measures wall time.
set -e
cd "$(dirname "$0")"

echo "=== rebuild: simulating root mismatch ==="
duckdb spike.duckdb -c "SELECT root, built_at FROM roots ORDER BY built_at DESC LIMIT 1;"
echo "stored root != current root 'blake2b:fake-root-stamp-REBUILT' -> full rebuild"

echo "=== changes rows before rebuild ==="
duckdb spike.duckdb -c "SELECT count(*) AS changes_before FROM changes;"

START=$(python3 -c 'import time; print(time.time())')
duckdb spike.duckdb <<'SQL'
.timer on
BEGIN;
DELETE FROM files;
DELETE FROM nodes;
DELETE FROM links;
DELETE FROM frontmatter;
DELETE FROM tags;
COPY files FROM 'files.csv' (HEADER);
COPY nodes FROM 'nodes.csv' (HEADER);
COPY links FROM 'links.csv' (HEADER);
COPY frontmatter FROM 'frontmatter.csv' (HEADER);
COPY tags FROM 'tags.csv' (HEADER);
INSERT INTO roots VALUES ('blake2b:fake-root-stamp-REBUILT', now());
COMMIT;
SQL
END=$(python3 -c 'import time; print(time.time())')
echo "rebuild_wall_s=$(python3 -c "print(f'{$END-$START:.3f}')")"

echo "=== verify after rebuild ==="
duckdb spike.duckdb -c "SELECT 'files' AS t, count(*) AS n FROM files UNION ALL SELECT 'changes', count(*) FROM changes UNION ALL SELECT 'roots', count(*) FROM roots;"
