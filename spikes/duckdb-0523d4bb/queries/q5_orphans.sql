-- Q5: orphans — files with zero inbound resolved links (anti-join on the same
-- basename key the extractor used for resolution), total count + sample 20
WITH inbound AS (
    SELECT DISTINCT lower(regexp_replace(regexp_extract(target, '([^/]+)$'), '\.md$', '')) AS key
    FROM links
    WHERE resolved = 1
),
orphans AS (
    SELECT f.path
    FROM files f
    ANTI JOIN inbound i
      ON i.key = lower(regexp_replace(regexp_extract(f.path, '([^/]+)$'), '\.md$', ''))
)
SELECT path, count(*) OVER () AS orphan_total
FROM orphans
ORDER BY path
LIMIT 20;
