-- one row per task file (path matching */tasks/*) with pivoted frontmatter
SELECT
  fm.path,
  max(CASE WHEN fm.key = 'status' THEN trim(split_part(fm.value, '#', 1)) END) AS status,
  max(CASE WHEN fm.key = 'owner'  THEN fm.value END) AS owner,
  max(CASE WHEN fm.key = 'stream' THEN fm.value END) AS stream
FROM frontmatter fm
WHERE fm.path LIKE '%/tasks/%'
  AND fm.key IN ('status', 'owner', 'stream')
GROUP BY fm.path
