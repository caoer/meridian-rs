-- The datalake convergence receipt as data: each base table Evidence extracts to
-- Parquet, with its live row count. Row counts are REAL (from the projection);
-- the changes count is synthetic-demo. Evidence extracts each of these to
-- static/data/meridian/<name>/<name>.parquet — one pipeline, two consumers
-- (dashboards + datalake).
select 'files' as table_name, count(*) as rows, 'real' as provenance from files
union all select 'links', count(*), 'real' from links
union all select 'frontmatter', count(*), 'real' from frontmatter
union all select 'roots', count(*), 'real' from roots
union all select 'batches', count(*), 'synthetic-demo' from batches
union all select 'changes', count(*), 'synthetic-demo' from changes
order by rows desc
