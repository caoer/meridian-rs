SELECT target, count(*) AS inbound
FROM links
WHERE resolved = 1
GROUP BY target
ORDER BY inbound DESC
LIMIT 100
