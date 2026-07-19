-- inbound resolved-link count per file, resolved by filename stem
WITH file_stems AS (
  SELECT path,
         regexp_replace(regexp_extract(path, '[^/]+$'), '\.md$', '') AS stem
  FROM files
),
inbound AS (
  SELECT regexp_replace(regexp_extract(target, '[^/]*$'), '\.md$', '') AS stem,
         count(*) AS n
  FROM links
  WHERE resolved = 1
  GROUP BY 1
)
SELECT f.path, coalesce(i.n, 0) AS inbound
FROM file_stems f
LEFT JOIN inbound i USING (stem)
