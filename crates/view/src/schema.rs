//! The round-1 view schema — 8 physical tables + 4 SQL views, verbatim from the
//! binding design (`tournament-duckdb/team-a/design.md` §Q1). Every table is a
//! disposable projection of the resident engine's parsed corpus, rebuilt on
//! fingerprint change (view-never-store). The DDL is the contract: DDL-enforced
//! identities (singleton `_meridian_view`, task→section identity FK), a derived
//! `resolved` generated column, the C1 locator triple on every fact row.

/// The full round-1 DDL. `_meridian_view` (singleton stamp) + 7 fact tables
/// = 8 physical tables, then 4 convenience SQL views. Copied EXACTLY from
/// design §Q1 — do not edit without leader approval (the design is binding).
pub const SCHEMA_SQL: &str = r#"
-- View-never-store: every table is a disposable projection of the resident engine's parsed corpus,
-- rebuilt on fingerprint change. Ordinals are UBIGINT (source counters are usize); a narrower type
-- only where a source bound proves it. Singleton stamp: PK(singleton)+CHECK(singleton) => AT MOST
-- ONE row by DDL; the '_' prefix reserves the name; published atomically with the data by one rename.
CREATE TABLE _meridian_view (
    singleton         BOOLEAN  NOT NULL DEFAULT true,
    schema_version    INTEGER  NOT NULL,          -- mismatch => treat as ABSENT
    as_of_fingerprint VARCHAR  NOT NULL,          -- workspace fingerprint built at ("b3:"+hex)
    workspace         VARCHAR  NOT NULL,
    built_epoch       VARCHAR,                     -- daemon epoch; NULL for a daemonless :memory: build (pairs with built_seq)
    built_seq         UBIGINT,                     -- wire changes_seq (u64); NULL for a daemonless build (pairs with built_epoch)
    built_unix        BIGINT   NOT NULL,          -- wall clock; ADVISORY, never a freshness input
    builder           VARCHAR  NOT NULL,          -- ADVISORY
    doc_count         UBIGINT  NOT NULL,
    PRIMARY KEY (singleton),
    CHECK (singleton),
    CHECK ((built_epoch IS NULL) = (built_seq IS NULL))  -- both NULL (daemonless :memory:) or both set (daemon-built)
);
CREATE TABLE doc (
    path       TEXT     PRIMARY KEY,              -- workspace-relative path (docs-map key)
    file_rev   TEXT     NOT NULL,                 -- blake3(whole file)[:16]
    line_count UINTEGER NOT NULL,                 -- u32
    bytes      UBIGINT  NOT NULL
);
CREATE TABLE frontmatter (
    path       TEXT     NOT NULL REFERENCES doc(path),
    ord        UBIGINT  NOT NULL,                 -- 0-based document-order index
    key        TEXT     NOT NULL,
    value      TEXT     NOT NULL,                 -- flat scalar; '' when empty, never NULL
    span_start UBIGINT  NOT NULL,                 -- C1: Frontmatter node span (all rows of a doc share it)
    span_end   UBIGINT  NOT NULL,
    node_rev   TEXT     NOT NULL,                 -- blake3(raw[span])[:16]
    PRIMARY KEY (path, key)                       -- first-occurrence wins (YamlMap)
);
CREATE TABLE section (
    path       TEXT     NOT NULL REFERENCES doc(path),
    node_seq   UBIGINT  NOT NULL,                 -- document-order ordinal; section identity
    hpath      TEXT[]   NOT NULL,                 -- heading chain; ADVISORY, NOT a join key
    heading    TEXT     NOT NULL,
    level      UTINYINT NOT NULL,                 -- u8
    node_rev   TEXT     NOT NULL,                 -- blake3(section span)[:16]
    span_start UBIGINT  NOT NULL,
    span_end   UBIGINT  NOT NULL,
    PRIMARY KEY (path, node_seq)                  -- identity; hpath text may repeat legally
);
CREATE TABLE link (
    src_path   TEXT     NOT NULL REFERENCES doc(path),
    seq        UBIGINT  NOT NULL,
    kind       TEXT     NOT NULL,                 -- 'wikilink' | 'embed' | 'link'
    target_raw TEXT     NOT NULL,                 -- linktext as written ('' for self [[#H]])
    heading    TEXT, block TEXT, alias TEXT,      -- NULL unless present in the linktext
    dest_path  TEXT     REFERENCES doc(path),     -- resolved vault path; NULL = dangling/external
    resolved   BOOLEAN  GENERATED ALWAYS AS (dest_path IS NOT NULL) VIRTUAL,  -- DERIVED, never stored
    span_start UBIGINT  NOT NULL,                 -- C1: Wikilink/Link/Embed node span
    span_end   UBIGINT  NOT NULL,
    node_rev   TEXT     NOT NULL,
    PRIMARY KEY (src_path, seq),
    CHECK (kind IN ('wikilink','embed','link')),
    CHECK (kind <> 'link' OR dest_path IS NULL)   -- external links never carry a vault dest
);
CREATE TABLE tag (                                -- inline #hashtag bodies (NodeKind::Tag) — real node spans
    path       TEXT     NOT NULL REFERENCES doc(path),
    seq        UBIGINT  NOT NULL,
    tag        TEXT     NOT NULL,                 -- Tag.name, no leading '#'
    span_start UBIGINT  NOT NULL,                 -- C1: real inline node span
    span_end   UBIGINT  NOT NULL,
    node_rev   TEXT     NOT NULL,
    PRIMARY KEY (path, seq)
);
CREATE TABLE frontmatter_tag (                    -- B2: keys 'tag'/'tags', scalar-parsed at build time
    path       TEXT     NOT NULL REFERENCES doc(path),
    seq        UBIGINT  NOT NULL,                 -- order within THIS key's value ('tag'/'tags' seq spaces are separate)
    tag        TEXT     NOT NULL,                 -- normalized: strip '#'/[]/quotes, trim; '' dropped
    key        TEXT     NOT NULL,                 -- 'tag' | 'tags' (provenance)
    span_start UBIGINT  NOT NULL,                 -- C1: Frontmatter NODE span (not per-item; see limitation)
    span_end   UBIGINT  NOT NULL,
    node_rev   TEXT     NOT NULL,
    PRIMARY KEY (path, key, seq),                 -- key in the PK: simultaneous 'tag' and 'tags' seq=0 rows never collide
    CHECK (key IN ('tag','tags'))
);
CREATE TABLE task (
    path        TEXT     NOT NULL REFERENCES doc(path),
    seq         UBIGINT  NOT NULL,
    checked     BOOLEAN  NOT NULL,
    depth       UINTEGER NOT NULL,                -- u32
    section_seq UBIGINT,                          -- governing section node_seq; NULL = document-level
    hpath       TEXT[],                           -- ADVISORY; NULL when document-level
    text        TEXT     NOT NULL,                -- task-line text (identity-bearing, bounded; not body)
    span_start  UBIGINT  NOT NULL,                -- C1: TaskItem node span
    span_end    UBIGINT  NOT NULL,
    node_rev    TEXT     NOT NULL,
    PRIMARY KEY (path, seq),
    FOREIGN KEY (path, section_seq) REFERENCES section(path, node_seq)  -- task->section by IDENTITY (<=1)
);

-- Convenience SQL views (no new storage) --
CREATE VIEW backlink AS                           -- inbound vault edges = resolved reverse read
    SELECT dest_path AS path, src_path, kind, alias FROM link WHERE dest_path IS NOT NULL;
CREATE VIEW dangling AS                            -- broken VAULT refs only (external excluded)
    SELECT src_path, target_raw FROM link WHERE kind IN ('wikilink','embed') AND dest_path IS NULL;
CREATE VIEW card AS                                -- session tree as a board: pivot frontmatter
    SELECT d.path,
        max(fm.value) FILTER (fm.key = 'type')    AS type,
        max(fm.value) FILTER (fm.key = 'status')  AS status,
        max(fm.value) FILTER (fm.key = 'owner')   AS owner,
        max(fm.value) FILTER (fm.key = 'session') AS session
    FROM doc d LEFT JOIN frontmatter fm USING (path) GROUP BY d.path;
CREATE VIEW tag_all AS                             -- B2: the union — inline + frontmatter, source-labeled + addressed
    SELECT path, tag, 'inline'      AS source, span_start, span_end, node_rev FROM tag
    UNION ALL
    SELECT path, tag, 'frontmatter' AS source, span_start, span_end, node_rev FROM frontmatter_tag;
"#;

/// The schema version stamped into `_meridian_view.schema_version`. A reader whose
/// current version differs treats the view as ABSENT (delete-don't-migrate).
pub const SCHEMA_VERSION: i32 = 1;

/// Run the full round-1 DDL against `conn`, creating the 8 tables + 4 views.
///
/// # Errors
/// Propagates any `DuckDB` error from executing the DDL batch (a parse or bind
/// failure means the schema is malformed — gate 1 asserts a clean parse).
pub fn create_schema(conn: &duckdb::Connection) -> duckdb::Result<()> {
    conn.execute_batch(SCHEMA_SQL)
}
