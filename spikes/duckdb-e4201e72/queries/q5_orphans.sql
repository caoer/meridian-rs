-- Q5 orphans: pages with no inbound resolved link. A page is orphaned when its
-- stem (basename without .md, lowercased) is never the target of a resolved link.
SELECT f.path
FROM files f
WHERE lower(regexp_extract(f.path, '([^/]+)\.md$', 1)) NOT IN (
    SELECT DISTINCT target FROM links WHERE resolved
)
ORDER BY f.path;
