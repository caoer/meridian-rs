-- The snapshot's as_of_root: latest build root, rendered as a banner on EVERY
-- page (the honest-tense law made visible). Derived from the snapshot's own
-- contents, so it is correct for whatever snap.duckdb is present at build time.
select
    root as as_of_root,
    built_at,
    strftime(built_at, '%Y-%m-%d %H:%M:%S') as built_at_str
from roots
order by built_at desc
limit 1
