-- SYNTHETIC-DEMO: spike churn rows (seeds 42-45 + crash/live/pure test writers)
SELECT path, count(*) AS n_changes, min(ts) AS first_ts, max(ts) AS last_ts
FROM changes
GROUP BY path
ORDER BY n_changes DESC
LIMIT 20
