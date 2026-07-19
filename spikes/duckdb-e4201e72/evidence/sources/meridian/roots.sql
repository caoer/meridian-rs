-- Build roots: one row per full projection build (content-addressed root hash +
-- build time). The latest by built_at is the snapshot's as_of_root.
select root, built_at from roots
