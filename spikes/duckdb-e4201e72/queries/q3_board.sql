-- Q3 board view: tasks grouped by frontmatter status.
-- A "task" is any page whose frontmatter declares type=task OR carries the
-- type/task tag (both conventions appear in the sessions tree).
SELECT s.value AS status, count(*) AS n
FROM frontmatter s
WHERE s.key = 'status'
  AND s.path IN (
    SELECT path FROM frontmatter
    WHERE (key = 'type'  AND value = 'task')
       OR (key = 'tag'   AND value = 'type/task')
  )
GROUP BY s.value
ORDER BY n DESC, status;
