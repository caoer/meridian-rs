//! Round-1 view schema — 8 physical tables + 4 SQL views (binding design §Q1).
//! Disposable projections of the parsed corpus, rebuilt on fingerprint change
//! (view-never-store). DDL is the contract: singleton `_meridian_view`,
//! task→section identity FK, derived `resolved`, C1 locator triple on every
//! fact row.

/// Full round-1 DDL: singleton stamp + 7 fact tables + 4 views. Binding design —
/// do not edit without leader approval.
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
    value      TEXT     NOT NULL,                 -- flat scalar, § A.6.1-decoded (published value plane); '' when empty, never NULL
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
    dest_path  TEXT     REFERENCES doc(path),     -- resolved AMBIENT vault path; NULL = dangling/external/cross-root
    -- U21 Q7(B): a CROSS-ROOT destination, as two columns and never a joined
    -- `root:path` string (Q5's prohibition). It is kept OUT of `dest_path`
    -- because that column carries an enforced foreign key into `doc`, and a
    -- cross-root path is not a key in THIS corpus — measured, not assumed:
    -- DuckDB answers "Violates foreign key constraint because key
    -- "path: notes.md" does not exist in the referenced table". So `dest_path`
    -- keeps meaning "a path in this corpus" ALWAYS, rather than only sometimes.
    dest_root      TEXT,                          -- mount name; NULL = ambient
    dest_root_path TEXT,                          -- the path INSIDE dest_root
    -- Session decision 0034: WHY a dangling edge dangles, when its target IS a
    -- real file the hash domain does not carry -- the §12.1 rule word
    -- ('non-md' | 'dot-segment' | 'custom-ignore'). NULL for a resolved edge
    -- AND for a genuine typo: a target with no file behind it earns no reason,
    -- which is the discriminator this column exists to restore. `resolved`
    -- keeps its exact meaning; this is read BESIDE it, never instead of it.
    exclusion  TEXT,
    resolved   BOOLEAN  GENERATED ALWAYS AS (dest_path IS NOT NULL OR dest_root IS NOT NULL) VIRTUAL,  -- DERIVED, never stored
    span_start UBIGINT  NOT NULL,                 -- C1: Wikilink/Link/Embed node span
    span_end   UBIGINT  NOT NULL,
    node_rev   TEXT     NOT NULL,
    PRIMARY KEY (src_path, seq),
    CHECK (kind IN ('wikilink','embed','link')),
    CHECK (kind <> 'link' OR dest_path IS NULL),  -- external links never carry a vault dest
    -- The third column widens the error space, so the illegal states are made
    -- UNREPRESENTABLE here rather than left to the projector's discipline.
    CHECK ((dest_root IS NULL) = (dest_root_path IS NULL)),  -- a root without its path names nothing
    CHECK (dest_path IS NULL OR dest_root IS NULL),          -- one destination, never two
    CHECK (kind <> 'link' OR dest_root IS NULL)              -- external links are not cross-root either
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
    -- AMBIENT edges only, and that is a STATED v1 non-goal rather than an
    -- oversight: this corpus is one vault, so a cross-vault link is visible
    -- from the source side alone. `dest_path IS NOT NULL` already excludes
    -- cross-root rows, since their destination is not a path in this corpus.
    SELECT dest_path AS path, src_path, kind, alias FROM link WHERE dest_path IS NOT NULL;
CREATE VIEW dangling AS                            -- broken VAULT refs with NO exclusion explanation
    -- **A RESOLVED CROSS-ROOT EDGE IS NOT DANGLING.** `dest_path` is NULL for
    -- it by construction (its target is not a path in this corpus), so the
    -- `dest_root IS NULL` clause is what stops every working cross-vault link
    -- being reported broken. Pinned as a RED TEST, not left as a comment:
    -- `view/tests/u21_cross_root_link_rows.rs`.
    -- **AN EXCLUDED TARGET IS NOT DANGLING EITHER** (ruling 2026-08-14): an
    -- edge whose target is a real, deliberately-unhashed file (`exclusion`
    -- stamped) is an authoring choice, not rot — mirror of the cross-root
    -- clause above. Pinned in `view/tests/dangling_exclusion.rs`. Raw rows
    -- stay reachable unchanged: `link WHERE dest_path IS NULL AND dest_root
    -- IS NULL` is the escape hatch, and it is also where the stated limit
    -- lives — a pathed spelling only a suffix walk could find (e.g.
    -- `attachments/….xlsx`) is never stamped, so it stays visible here.
    SELECT src_path, target_raw FROM link
     WHERE kind IN ('wikilink','embed') AND dest_path IS NULL AND dest_root IS NULL
       AND exclusion IS NULL;
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

/// Schema version in `_meridian_view.schema_version`. Mismatch ⇒ treat view as
/// ABSENT (delete-don't-migrate).
///
/// `2`: `dangling` gained `AND exclusion IS NULL` (ruling 2026-08-14), and the
/// shared mint's bare-name fallback changed `exclusion` content for identical
/// corpora — either alone obligates the bump.
pub const SCHEMA_VERSION: i32 = 2;

/// Run the full round-1 DDL against `conn` (8 tables + 4 views).
///
/// # Errors
/// Propagates any `DuckDB` error from the DDL batch.
pub fn create_schema(conn: &duckdb::Connection) -> duckdb::Result<()> {
    conn.execute_batch(SCHEMA_SQL)
}
