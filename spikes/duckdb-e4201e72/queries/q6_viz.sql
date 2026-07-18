-- Q6 (own design) — the ceiling: a force-graph visualization feed for the
-- 1-hop ego network of LLM_WIKI, emitted as nested LIST<STRUCT> in ONE row
-- (nodes[] with degree + edges[] {source,target}). This is constraint 6
-- (visualization feeds): a dashboard can consume the row verbatim as graph JSON.
WITH ego_edges AS (
    SELECT src_path AS src, target AS dst
    FROM links
    WHERE resolved
),
ego_nodes AS (
    SELECT DISTINCT src AS node FROM ego_edges WHERE dst = 'llm_wiki'
    UNION
    SELECT 'llm_wiki'
),
sub AS (
    SELECT e.src, e.dst
    FROM ego_edges e
    WHERE e.src IN (SELECT node FROM ego_nodes)
      AND e.dst IN (SELECT node FROM ego_nodes)
),
degree AS (
    SELECT node, count(*) AS degree
    FROM (SELECT src AS node FROM sub UNION ALL SELECT dst FROM sub)
    GROUP BY node
)
SELECT
    (SELECT list({'node': node, 'degree': degree} ORDER BY degree DESC) FROM degree) AS nodes,
    (SELECT list({'source': src, 'target': dst}) FROM sub) AS edges;
