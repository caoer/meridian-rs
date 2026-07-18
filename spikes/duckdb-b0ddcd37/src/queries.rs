//! The five rung-5 reference queries + one ceiling query, as literal SQL.
//! These strings ARE the queryability inventory reproduced in REPORT.md.

/// Q1 — backlinks of a hot page (param: normalized target key).
pub const Q1_BACKLINKS: &str = "SELECT src_path FROM links WHERE target = ? AND resolved";

/// Q2 — tag aggregation corpus-wide.
pub const Q2_TAGS: &str = "\
SELECT value AS tag, count(*) AS n
FROM frontmatter
WHERE key = 'tags'
GROUP BY value
ORDER BY n DESC";

/// Q3 — board view: tasks by frontmatter status.
pub const Q3_BOARD: &str = "\
SELECT s.value AS status, count(*) AS n
FROM frontmatter t
JOIN frontmatter s USING (path)
WHERE t.key = 'type' AND t.value = 'task'
  AND s.key = 'status'
GROUP BY s.value
ORDER BY n DESC";

/// Q4 — history window: last N changes for a path (param: path).
pub const Q4_HISTORY: &str = "\
SELECT seq, ts, change, old_rev, new_rev
FROM changes
WHERE path = ?
ORDER BY seq DESC
LIMIT 50";

/// Q5 — orphans: pages with no inbound resolved links.
pub const Q5_ORPHANS: &str = "\
SELECT f.path
FROM files f
WHERE NOT EXISTS (
    SELECT 1 FROM links l
    WHERE l.target = f.norm_key AND l.resolved
)";

/// Ceiling — visualization feed (constraint 6): degree-centrality node list,
/// viz-ready. Shows correlated subqueries / graph metrics in plain SQL.
pub const QC_VIZFEED: &str = "\
WITH deg AS (
    SELECT
        f.path,
        f.norm_key,
        (SELECT count(*) FROM links l WHERE l.src_path = f.path AND l.resolved) AS out_deg,
        (SELECT count(*) FROM links l WHERE l.target = f.norm_key AND l.resolved) AS in_deg
    FROM files f
)
SELECT path, in_deg, out_deg, in_deg + out_deg AS degree
FROM deg
WHERE in_deg + out_deg > 0
ORDER BY degree DESC
LIMIT 100";
