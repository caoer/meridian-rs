-- Q4 history window: the last 20 changes for the hottest path (self-contained,
-- so it runs from the duckdb CLI without a bound parameter). In production the
-- path is a bound '?'; here we pick the most-changed path automatically.
SELECT seq, ts, kind, change, old_rev, new_rev
FROM changes
WHERE path = (
    SELECT path FROM changes GROUP BY path ORDER BY count(*) DESC LIMIT 1
)
ORDER BY seq DESC
LIMIT 20;
