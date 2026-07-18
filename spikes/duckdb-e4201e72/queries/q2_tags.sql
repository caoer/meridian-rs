-- Q2 tag aggregation corpus-wide: frontmatter tags ranked by frequency.
SELECT value AS tag, count(*) AS n
FROM frontmatter
WHERE key = 'tag'
GROUP BY value
ORDER BY n DESC, tag
LIMIT 50;
