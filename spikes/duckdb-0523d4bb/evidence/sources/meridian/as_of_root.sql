SELECT
  root,
  built_at,
  'as_of_root: ' || root || ' · built ' || strftime(built_at, '%Y-%m-%d %H:%M:%S') AS banner
FROM roots
ORDER BY built_at DESC
LIMIT 1
