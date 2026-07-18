-- Q4: history window — last 50 changes rows for one hot path
-- ('crash/test.md' has 28,806 changes rows, the hottest path in the changes table)
SELECT seq, ts, kind, change, old_rev, new_rev
FROM changes
WHERE path = 'crash/test.md'
ORDER BY seq DESC
LIMIT 50;
