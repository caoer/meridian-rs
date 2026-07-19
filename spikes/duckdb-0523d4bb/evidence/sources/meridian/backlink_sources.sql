SELECT target, src_path, count(*) AS n
FROM links
WHERE resolved = 1
GROUP BY target, src_path
