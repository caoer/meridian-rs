SELECT tag, count(*) AS n
FROM tags
GROUP BY tag
ORDER BY n DESC
LIMIT 50
