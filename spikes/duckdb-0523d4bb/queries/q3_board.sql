-- Q3: board view — tasks (paths under */tasks/*) grouped by frontmatter status
SELECT value AS status,
       count(*) AS n,
       list(path ORDER BY path)[1:5] AS sample_paths
FROM frontmatter
WHERE key = 'status' AND path LIKE '%/tasks/%'
GROUP BY value
ORDER BY n DESC;
