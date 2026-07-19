-- fleet view: one row per top-level unit (home-wiki, or home-wiki-sessions year=/month= partition)
WITH pref AS (
  SELECT path, size AS bytes,
    CASE
      WHEN path LIKE 'home-wiki-sessions/%'
        THEN coalesce(nullif(regexp_extract(path, '^(home-wiki-sessions/year=\d+/month=\d+)', 1), ''),
                      'home-wiki-sessions/(other)')
      WHEN path LIKE 'home-wiki/%' THEN 'home-wiki'
      ELSE split_part(path, '/', 1)
    END AS fleet_unit
  FROM files
),
status_norm AS (
  SELECT path, max(trim(split_part(value, '#', 1))) AS status
  FROM frontmatter
  WHERE key = 'status'
  GROUP BY path
)
SELECT
  p.fleet_unit,
  count(*) AS files,
  sum(p.bytes) AS bytes,
  count(*) FILTER (WHERE p.path LIKE '%/tasks/%') AS task_files,
  count(*) FILTER (WHERE p.path LIKE '%/tasks/%' AND s.status = 'done') AS tasks_done,
  count(*) FILTER (WHERE p.path LIKE '%/tasks/%' AND s.status IN ('in-progress', 'dispatched', 'assigned', 'active')) AS tasks_active,
  count(*) FILTER (WHERE p.path LIKE '%/tasks/%' AND s.status IN ('pending', 'todo')) AS tasks_queued,
  count(*) FILTER (WHERE p.path LIKE '%/tasks/%' AND s.status = 'review') AS tasks_review
FROM pref p
LEFT JOIN status_norm s ON s.path = p.path
GROUP BY p.fleet_unit
ORDER BY files DESC
