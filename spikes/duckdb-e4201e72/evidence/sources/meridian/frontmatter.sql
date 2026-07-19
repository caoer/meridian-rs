-- REAL corpus facts. One row per (path, key, value) — YAML frontmatter melted
-- long. Multi-value keys (tag, alias) repeat the path.
select path, key, value from frontmatter
