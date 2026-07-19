-- SYNTHETIC-DEMO: churn rows scoped to compound-relevant paths (memos/decisions/results)
SELECT seq, ts, path, kind, change
FROM changes
WHERE regexp_matches(path, '/(memos|decisions|results)/')
