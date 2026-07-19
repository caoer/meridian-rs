-- SYNTHETIC-DEMO churn. The round-3 spike's `churn` harness appended 10,002
-- synthetic `update`/`edit` rows over 21 paths to exercise the append log — this
-- is NOT real history. Every page built on this table carries a synthetic-demo
-- banner until a live appender exists.
select seq, ts, path, kind, change, old_rev, new_rev from changes
