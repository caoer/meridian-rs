-- Q6 (ceiling): dashboard feed — top-20 most-changed paths from the changes log,
-- joined with current file state, inbound resolved-link counts, frontmatter
-- status, and tag list. 5 tables: changes, files, links, frontmatter, tags.
WITH changed AS (
    SELECT path, count(*) AS change_count, max(ts) AS last_change
    FROM changes
    GROUP BY path
),
inbound AS (
    SELECT lower(regexp_replace(regexp_extract(target, '([^/]+)$'), '\.md$', '')) AS key,
           count(*) AS inbound_links
    FROM links
    WHERE resolved = 1
    GROUP BY 1
)
SELECT c.path,
       c.change_count,
       c.last_change,
       f.file_rev,
       f.size,
       coalesce(i.inbound_links, 0) AS inbound_links,
       fm.value AS status,
       (SELECT list(t.tag) FROM tags t WHERE t.path = c.path) AS tags
FROM changed c
LEFT JOIN files f ON f.path = c.path
LEFT JOIN inbound i
       ON i.key = lower(regexp_replace(regexp_extract(c.path, '([^/]+)$'), '\.md$', ''))
LEFT JOIN frontmatter fm ON fm.path = c.path AND fm.key = 'status'
ORDER BY c.change_count DESC, c.path
LIMIT 20;
