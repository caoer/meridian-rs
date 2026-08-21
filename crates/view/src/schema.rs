//! Round-1 view schema — 9 physical tables + 4 SQL views (binding design §Q1;
//! `body` added by docs/body-projection.md). Disposable projections of the
//! parsed corpus, rebuilt on fingerprint change (view-never-store). DDL is the
//! contract: singleton `_meridian_view`, task→section identity FK, derived
//! `resolved`, C1 locator triple on every fact row.

/// Full round-1 DDL: singleton stamp + 8 fact tables + 4 views. Binding design —
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
    -- The SECOND witness (`base-projection.md` §6.2): 'bf:'+blake3-hex over the
    -- `.base` member list. NULL = the base walk did not run ("not asked",
    -- never "empty"). It rides beside `as_of_fingerprint` rather than inside
    -- it because `.base` bytes are in NO fingerprint (§12.1 md-only floor), so
    -- one stamp row carries two witnesses, each naming exactly what it covers.
    base_fold         VARCHAR,
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
    node_rev   TEXT     NOT NULL,                 -- blake3(raw[span])[:16] -- BLOCK grain, shared by every key of the doc
    -- The PER-KEY CAS token (node-rev-merkle-spec §2.1): blake3 over the key
    -- line plus its indented continuation lines. This is the grain the write
    -- door compares `if_node_rev` against on an fm_key target, so it is the
    -- only one of the two rev columns a guarded single-key splice can use --
    -- `node_rev` above is the whole block and refuses cas_mismatch whenever any
    -- OTHER key moved. SERVED, never recomputed: the value comes off the same
    -- `model::resolve(Ref::FmKey)` owner the read face's `props[].prop_rev`
    -- and the write door both use, so the three cannot drift.
    prop_rev   TEXT     NOT NULL,
    PRIMARY KEY (path, key)                       -- first-occurrence wins (YamlMap)
);
CREATE TABLE section (
    path       TEXT     NOT NULL REFERENCES doc(path),
    node_seq   UBIGINT  NOT NULL,                 -- document-order ordinal; section identity
    -- The published machine address, as the read face's toc publishes it and
    -- read/put accept it verbatim: '[{"h":…},…]' compact JSON, per-segment "n"
    -- (1-based occurrence among same-parent same-text siblings) only where the
    -- raw text is ambiguous. ADVISORY, NOT a join key. The former TEXT[] chain
    -- rendered duplicate siblings identically and its joined spelling could
    -- not address (card sql-hpath-read-grammar, dogfood r8 § D5).
    hpath      TEXT     NOT NULL,
    -- This row's OWN occurrence index (`wire-contract.md` § A.11, ZT ruling
    -- 2026-08-15): 1-based position among the same-parent, same-raw-text
    -- sibling sections, and NULL exactly where the published address omits it
    -- -- so `n IS NOT NULL` is the ambiguity predicate and this is the last
    -- segment of `hpath` above, never a second spelling of it. SERVED from the
    -- same address owner `hpath` renders from, never recomputed here: a second
    -- owner of one fact drifts silently, both answering a plausible integer.
    -- It is NOT `node_seq` -- that ordinal counts every section of the file
    -- (this row's identity), while `n` counts one heading text under one
    -- parent. Deriving one from the other addresses a real but DIFFERENT
    -- section, whose node_rev then guards the write, so a wrong-target commit
    -- passes CAS instead of refusing.
    n          UINTEGER,
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
    -- WHICH file the probe resolved (either arm), workspace-relative
    -- (`base-projection.md` §5.1). The probe computed it and the projection
    -- used to discard it; keeping it makes *who embeds this base* a join
    -- (`link.exclusion_path = base.path`, exact, no basename re-derivation in
    -- SQL) and gives every other excluded class ('.svg', '.xlsx', dot-segment,
    -- custom-ignore) the same honesty for free. Exactness is earned by the
    -- §5.1 mint rule: a stamp carries the ON-DISK spelling, or it does not
    -- stamp.
    exclusion_path TEXT,
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
    CHECK (kind <> 'link' OR dest_root IS NULL),             -- external links are not cross-root either
    CHECK ((exclusion IS NULL) = (exclusion_path IS NULL))   -- the word and the file it names arrive together
);
-- The three `.base` relations (`base-projection.md` §4), view-lane only.
-- They ride the `base_fold` witness, NEVER `as_of_fingerprint`: their bytes
-- cannot move the workspace fingerprint (§12.1 md-only floor), so their
-- coverage claim is `base_fold`'s alone.
CREATE TABLE base (
    path       TEXT     PRIMARY KEY,   -- workspace-relative ON-DISK spelling (§3 membership)
    file_rev   TEXT,                   -- blake3(whole file)[:16] — leaf-shaped, in NO fingerprint (§6.1); NULL only on an unreadable member
    bytes      UBIGINT,                -- NULL only with file_rev NULL (unreadable member)
    error      TEXT,                   -- NULL = parsed as a YAML mapping; else the parser's or the read's own message
    filters    TEXT,                   -- file-level filters subtree, compact JSON (§4.2); NULL when absent
    properties TEXT,                   -- display-config subtree, compact JSON; NULL when absent
    extra      TEXT,                   -- compact JSON OBJECT: every top-level key §4.5 does not lift, subtree intact; NULL when none
    CHECK (error IS NULL OR (filters IS NULL AND properties IS NULL AND extra IS NULL)),
    CHECK ((file_rev IS NULL) = (bytes IS NULL)),
    CHECK (error IS NOT NULL OR file_rev IS NOT NULL)  -- an unreadable member always says why
);
CREATE TABLE base_view (                -- rides base_fold (see base)
    path    TEXT     NOT NULL REFERENCES base(path),
    ord     UBIGINT  NOT NULL,         -- 0-based document order within views:
    name    TEXT,                      -- lifted when the entry's name is a string (§4.5); else NULL
    type    TEXT,                      -- lifted when a string ('table', 'cards', …) — OPEN SET, no CHECK (§4.3)
    filters TEXT,                      -- view-level filters subtree, compact JSON; NULL when absent
    config  TEXT,                      -- remaining view keys as one compact JSON object in written order, or the whole entry when it is not a mapping (§4.5); NULL when none
    PRIMARY KEY (path, ord)
);
CREATE TABLE base_formula (             -- rides base_fold (see base)
    path TEXT     NOT NULL REFERENCES base(path),
    ord  UBIGINT  NOT NULL,            -- document order within formulas: (unconstrained beside the PK — the frontmatter precedent)
    name TEXT     NOT NULL,
    expr TEXT     NOT NULL,            -- the expression, verbatim scalar or compact JSON of a non-scalar — never interpreted (§4.3)
    PRIMARY KEY (path, name)           -- guaranteed by the PARSER, not by YAML: the pinned parser refuses duplicate mapping keys (§4.4)
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
    hpath       TEXT,                             -- governing section's machine address ('[{"h":…},…]', as section.hpath); ADVISORY; NULL when document-level
    text        TEXT     NOT NULL,                -- task-line text (identity-bearing, bounded; not body)
    span_start  UBIGINT  NOT NULL,                -- C1: TaskItem node span
    span_end    UBIGINT  NOT NULL,
    node_rev    TEXT     NOT NULL,
    PRIMARY KEY (path, seq),
    FOREIGN KEY (path, section_seq) REFERENCES section(path, node_seq)  -- task->section by IDENTITY (<=1)
);
CREATE TABLE body (                               -- exclusive-content chunks (docs/body-projection.md §2-§3)
    path        TEXT     NOT NULL REFERENCES doc(path),
    seq         UBIGINT  NOT NULL,                -- 0-based document order of chunks
    section_seq UBIGINT,                          -- owning section's node_seq; NULL = preamble (before first heading)
    hpath       TEXT,                             -- owning section's machine address (as section.hpath); ADVISORY; NULL on preamble
    text        TEXT     NOT NULL,                -- the chunk bytes, verbatim: content start to first child section (heading lines in NO chunk)
    span_start  UBIGINT  NOT NULL,                -- the chunk's own byte range (C1: slicing raw bytes yields text)
    span_end    UBIGINT  NOT NULL,
    -- The OWNING SECTION's CAS token, not a chunk hash: a search hit's next
    -- move is a guarded splice on that section, and a chunk-grain rev would
    -- teach an affordance that refuses (no chunk splice door exists). NULL on
    -- preamble rows.
    node_rev    TEXT,
    PRIMARY KEY (path, seq),
    FOREIGN KEY (path, section_seq) REFERENCES section(path, node_seq)
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
CREATE VIEW record AS                              -- one row per frontmatter-carrying record, corpus-wide: pivot frontmatter
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
///
/// `3`: `frontmatter` gained `prop_rev`, the per-key CAS token
/// (`node-rev-merkle-spec.md` §2.1).
///
/// `4`: the `card` view became `record` — one row per frontmatter-carrying
/// record, corpus-wide; the old noun promised a board (dogfood r6 U-S1) —
/// and `task.text` dropped its list-marker + checkbox prefix (`checked`
/// already carries the bit; dogfood r6 S11).
///
/// `5`: `section.hpath` / `task.hpath` became TEXT — the published
/// `[{"h":…},…]` machine address with per-segment `n` on ambiguity — replacing
/// the TEXT[] chain whose rendering could not address (card
/// sql-hpath-read-grammar, dogfood r8 § D5).
///
/// `6`: `section.n` added — the row's own occurrence index, served beside the
/// `hpath` that already carries it (`wire-contract.md` § A.11, ZT ruling
/// 2026-08-15 "Rule: add n"). Additive; no existing column re-grained.
///
/// `7`: the `.base` projection (`docs/base-projection.md`) — the three `base`
/// relations, `link.exclusion_path`, and `_meridian_view.base_fold`. The
/// §5.1 mint rule also narrows `exclusion` CONTENT on a case-insensitive
/// volume (a spelling that reached bytes only through case-folding stops
/// stamping), so a v6 file would serve pre-rule rows at the same fingerprint.
///
/// `8`: the `body` relation — exclusive-content chunks per section plus
/// preamble (`docs/body-projection.md`); a version-7 projection has no body
/// rows to serve.
///
/// `9`: `frontmatter_tag` reads `model::fm_tags` off the frontmatter BLOCK
/// instead of the flat map, so a YAML block sequence (`tags:` then indented
/// `- item` lines) finally projects its items — 98 pages on the fleet corpus
/// went from zero rows to their real tags. No DDL moved; ROW CONTENT did, for
/// identical corpora at identical fingerprints, which is the obligation `2`
/// and `7` were bumped under (card `tag-all-block-form-blindness`).
///
/// `10`: `frontmatter.value` reads `model::fm_value` off the BLOCK for every
/// key, so a block sequence under ANY key (`agents:`, `handoff-to:`, …)
/// renders as its flow-style text instead of `''` — 50 of 50 `agents` rows on
/// the fleet corpus. Same obligation as `9`: no DDL moved, ROW CONTENT did for
/// identical corpora at identical fingerprints (card `fm-block-list-sql-empty`).
pub const SCHEMA_VERSION: i32 = 10;

/// Run the full round-1 DDL against `conn` (8 tables + 4 views).
///
/// # Errors
/// Propagates any `DuckDB` error from the DDL batch.
pub fn create_schema(conn: &duckdb::Connection) -> duckdb::Result<()> {
    conn.execute_batch(SCHEMA_SQL)
}
