-- SYNTHETIC-DEMO: per-minute change counts for the 20 hottest paths
WITH top_paths AS (
  SELECT path FROM changes GROUP BY path ORDER BY count(*) DESC LIMIT 20
)
SELECT
  strftime(date_trunc('minute', c.ts), '%H:%M') AS minute,
  regexp_extract(c.path, '[^/]+$') AS file,
  count(*) AS n
FROM changes c
JOIN top_paths t ON t.path = c.path
GROUP BY 1, 2
ORDER BY 1
