-- Q1: backlinks of the hot page (most-linked resolved target = '/SCHEMA', 1160 inbound)
SELECT l.src_path, f.file_rev
FROM links l
JOIN files f ON f.path = l.src_path
WHERE l.resolved = 1 AND l.target = '/SCHEMA'
ORDER BY l.src_path;
