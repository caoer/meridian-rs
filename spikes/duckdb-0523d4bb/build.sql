-- Phase 2 build: schema + COPY from CSVs. Run via:
--   duckdb spike.duckdb -init /dev/null < build.sql   (with .timer on)
.timer on
CREATE TABLE files (
    path TEXT PRIMARY KEY,
    file_rev TEXT NOT NULL,
    size BIGINT NOT NULL
);
CREATE TABLE nodes (
    path TEXT NOT NULL,
    span_start BIGINT NOT NULL,
    span_end BIGINT NOT NULL,
    hpath TEXT,
    node_rev TEXT NOT NULL,
    kind TEXT NOT NULL
);
CREATE TABLE links (
    src_path TEXT NOT NULL,
    target TEXT NOT NULL,
    resolved TINYINT NOT NULL
);
CREATE TABLE frontmatter (
    path TEXT NOT NULL,
    key TEXT NOT NULL,
    value TEXT
);
CREATE TABLE tags (
    path TEXT NOT NULL,
    tag TEXT NOT NULL
);
CREATE TABLE changes (
    seq BIGINT,
    ts TIMESTAMP,
    path TEXT,
    kind TEXT,
    change TEXT,
    old_rev TEXT,
    new_rev TEXT
);
CREATE TABLE roots (
    root TEXT,
    built_at TIMESTAMP
);
COPY files FROM 'files.csv' (HEADER);
COPY nodes FROM 'nodes.csv' (HEADER);
COPY links FROM 'links.csv' (HEADER);
COPY frontmatter FROM 'frontmatter.csv' (HEADER);
COPY tags FROM 'tags.csv' (HEADER);
INSERT INTO roots VALUES ('blake2b:fake-root-stamp-0523d4bb', now());
SELECT 'files' AS t, count(*) AS n FROM files
UNION ALL SELECT 'nodes', count(*) FROM nodes
UNION ALL SELECT 'links', count(*) FROM links
UNION ALL SELECT 'frontmatter', count(*) FROM frontmatter
UNION ALL SELECT 'tags', count(*) FROM tags
UNION ALL SELECT 'changes', count(*) FROM changes
UNION ALL SELECT 'roots', count(*) FROM roots;
