-- SYNTHETIC-DEMO: changes per minute bucket
SELECT date_trunc('minute', ts) AS minute, change, count(*) AS n
FROM changes
GROUP BY 1, 2
ORDER BY 1
