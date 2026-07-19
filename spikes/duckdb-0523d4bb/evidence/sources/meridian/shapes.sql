-- shape-conformance flags for every task file: does it carry the required keys?
WITH task_files AS (
  SELECT path FROM files WHERE path LIKE '%/tasks/%'
),
fm AS (
  SELECT DISTINCT path, key
  FROM frontmatter
  WHERE key IN ('status', 'stream', 'owner')
)
SELECT
  t.path,
  max(CASE WHEN fm.key = 'status' THEN 1 ELSE 0 END) AS has_status,
  max(CASE WHEN fm.key = 'stream' THEN 1 ELSE 0 END) AS has_stream,
  max(CASE WHEN fm.key = 'owner'  THEN 1 ELSE 0 END) AS has_owner
FROM task_files t
LEFT JOIN fm ON fm.path = t.path
GROUP BY t.path
