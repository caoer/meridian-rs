-- Q2: corpus-wide tag aggregation, top 50
SELECT tag, count(*) AS n
FROM tags
GROUP BY tag
ORDER BY n DESC, tag
LIMIT 50;
