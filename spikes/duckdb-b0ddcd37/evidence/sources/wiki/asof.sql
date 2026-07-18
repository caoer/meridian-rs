-- as_of_root banner feed: root stamp read from the snapshot's own contents
-- (roots/batches) + the snapshot file's real mtime (captured at scaffold time,
-- injected as a literal since the file's own mtime is a filesystem fact).
select
  (select root from roots limit 1)                                   as root_hex,
  (select root_after from batches order by batch_seq desc limit 1)   as head_root,
  strftime((select built_at from roots limit 1), '%Y-%m-%d %H:%M:%S UTC') as root_built_at,
  '2026-07-18 13:25:59 EDT'                                          as snapshot_mtime_local,
  (select n_rows from batches order by batch_seq desc limit 1)       as head_rows,
  (select max(seq) from changes)                                     as head_seq,
  'wiki-snap.duckdb'                                                 as snapshot_file
