-- files with zero inbound resolved links.
-- links.target stores wiki-style short names, so resolution is by filename stem
-- (last path segment minus .md) matched against the last segment of the target.
WITH file_stems AS (
  SELECT path, size AS bytes,
         regexp_replace(regexp_extract(path, '[^/]+$'), '\.md$', '') AS stem
  FROM files
),
target_stems AS (
  SELECT DISTINCT regexp_replace(regexp_extract(target, '[^/]*$'), '\.md$', '') AS stem
  FROM links
  WHERE resolved = 1
)
SELECT
  f.path,
  f.bytes,
  (f.path LIKE 'home-wiki/assets/%' OR f.path LIKE 'home-wiki/inbox/scraped/%') AS is_bulk_capture
FROM file_stems f
LEFT JOIN target_stems t USING (stem)
WHERE t.stem IS NULL
