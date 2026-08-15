//! The `.base` projection's §10.1 red tests (`docs/base-projection.md`).
//!
//! They live here because every one of them needs BOTH halves of the design at
//! once: the `fs` walk that defines membership and folds the witness, and the
//! `view` projection that turns its bytes into rows. Neither crate can host
//! them alone — `view` reads no disk by charter, and `fs` holds no `DuckDB`.

use std::collections::BTreeMap;
use std::path::Path;

use fs::WorkspaceRoot;

// ---------------------------------------------------------------------------
// fixture plumbing
// ---------------------------------------------------------------------------

/// Write `contents` at `rel` under `root`, creating parents.
fn write(root: &Path, rel: &str, contents: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("mkdir");
    }
    std::fs::write(&path, contents).expect("write fixture");
}

/// The `.base` walk as the sql lanes take it.
fn walk(root: &WorkspaceRoot) -> fs::base::BaseSnapshot {
    fs::base::base_snapshot(root).expect("base walk")
}

/// The projection inputs `view` takes, owned so they outlive the borrow.
fn members(snapshot: &fs::base::BaseSnapshot) -> Vec<view::BaseMember> {
    snapshot
        .members
        .iter()
        .map(|m| view::BaseMember {
            path: m.path.clone(),
            bytes: m.bytes.clone(),
        })
        .collect()
}

/// Parse the md corpus and build the `:memory:` projection over both planes,
/// with the real exclusion probe — the shape `mrd sql`'s memory lane builds.
fn build(root: &WorkspaceRoot) -> duckdb::Connection {
    let (files, fingerprint) = fs::domain_snapshot(root).expect("domain snapshot");
    let (_index, docs, _unserved) = fs::build_corpus(files);
    let domain = fs::domain::Domain::load(root).expect("domain");
    let probe = fs::domain::LinkTargetProbe::new(root, &domain);
    let exclusion = |target: &str| {
        probe
            .resolution(target)
            .map(|(p, why)| (p, why.word().to_owned()))
    };
    let snapshot = walk(root);
    let owned = members(&snapshot);
    let base = view::BaseWalk {
        members: &owned,
        fold: &snapshot.fold,
    };
    let corpus = model::RootedCorpus::ambient(&docs);
    let mounts = addr::MountSet::new([]);
    view::build_memory_rooted(
        &docs,
        &corpus,
        &mounts,
        &fingerprint.0,
        Some(&exclusion),
        Some(&base),
    )
    .expect("build")
}

/// Every row of `sql` as a vector of nullable text cells.
fn rows(conn: &duckdb::Connection, sql: &str, cols: usize) -> Vec<Vec<String>> {
    let mut stmt = conn.prepare(sql).expect("prepare");
    stmt.query_map([], |r| {
        (0..cols)
            .map(|i| {
                Ok(r.get::<_, Option<String>>(i)?
                    .unwrap_or_else(|| "~N~".into()))
            })
            .collect::<Result<Vec<String>, duckdb::Error>>()
    })
    .expect("query")
    .map(|r| r.expect("row"))
    .collect()
}

/// One scalar text cell (`~N~` for NULL).
fn one(conn: &duckdb::Connection, sql: &str) -> String {
    conn.query_row(sql, [], |r| r.get::<_, Option<String>>(0))
        .expect("query")
        .unwrap_or_else(|| "~N~".into())
}

/// The §11 measured target, verbatim.
const TASKS_BASE: &str = r#"filters:
  and:
    - file.hasTag("type/task")
views:
  - type: table
    name: Board
    groupBy:
      property: status
      direction: ASC
    order:
      - file.name
      - status
      - file.folder
  - type: cards
    name: Kanban
    groupBy:
      property: status
      direction: ASC
    order:
      - file.name
      - file.folder
"#;

// ---------------------------------------------------------------------------
// §10.1 test 1 — THE GATE
// ---------------------------------------------------------------------------

/// The mandate's own question, as one SELECT: *which views/filters does
/// TASKS.base define* (§11), exact rows.
#[test]
fn tasks_base_answers_the_worked_gate_select() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let root = WorkspaceRoot(dir.path().to_path_buf());
    write(dir.path(), "bases/TASKS.base", TASKS_BASE);
    write(dir.path(), "note.md", "# Note\n");

    let conn = build(&root);
    let got = rows(
        &conn,
        "SELECT b.filters, v.ord::VARCHAR, v.name, v.type, v.config \
         FROM base b JOIN base_view v USING (path) \
         WHERE b.path = 'bases/TASKS.base' ORDER BY v.ord",
        5,
    );

    let filters = r#"{"and":["file.hasTag(\"type/task\")"]}"#;
    assert_eq!(
        got,
        vec![
            vec![
                filters.to_owned(),
                "0".to_owned(),
                "Board".to_owned(),
                "table".to_owned(),
                r#"{"groupBy":{"property":"status","direction":"ASC"},"order":["file.name","status","file.folder"]}"#.to_owned(),
            ],
            vec![
                filters.to_owned(),
                "1".to_owned(),
                "Kanban".to_owned(),
                "cards".to_owned(),
                r#"{"groupBy":{"property":"status","direction":"ASC"},"order":["file.name","file.folder"]}"#.to_owned(),
            ],
        ],
        "the §11 worked gate, row for row"
    );
}

// ---------------------------------------------------------------------------
// §10.1 test 2 — alien honesty
// ---------------------------------------------------------------------------

/// A shell script wearing `.base` is a NAMED row carrying the parser's message,
/// with every content column NULL — never an absence (§4.4).
#[test]
fn an_alien_base_is_a_named_error_row_with_no_content() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let root = WorkspaceRoot(dir.path().to_path_buf());
    write(
        dir.path(),
        "gpurun.base",
        "#!/bin/sh\nexec nvidia-smi \"$@\"\n",
    );

    let conn = build(&root);
    let got = rows(
        &conn,
        "SELECT path, (error IS NOT NULL)::VARCHAR, filters, properties, extra, \
                (file_rev IS NOT NULL)::VARCHAR FROM base",
        6,
    );
    assert_eq!(got.len(), 1, "the alien is PRESENT, not dropped");
    assert_eq!(got[0][0], "gpurun.base");
    assert_eq!(got[0][1], "true", "it says why");
    assert_eq!(
        (&got[0][2], &got[0][3], &got[0][4]),
        (&"~N~".to_owned(), &"~N~".to_owned(), &"~N~".to_owned()),
        "every content column NULL"
    );
    assert_eq!(
        got[0][5], "true",
        "its bytes WERE read, so it keeps a file_rev"
    );
    assert_eq!(
        one(&conn, "SELECT count(*)::VARCHAR FROM base_view"),
        "0",
        "an error row has zero children"
    );
}

// ---------------------------------------------------------------------------
// §10.1 test 3 — the floor
// ---------------------------------------------------------------------------

/// Membership is the hash domain's rules with the floor swapped (§3):
/// case-exact extension, the dot-segment floor, and the custom ignore list.
#[test]
fn the_base_floor_is_case_exact_dot_pruned_and_custom_ignored() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let root = WorkspaceRoot(dir.path().to_path_buf());
    write(
        dir.path(),
        "meridian/domain.md",
        "---\nignore:\n  - \"drafts/**\"\n---\n",
    );
    write(dir.path(), "keep.base", "views: []\n");
    write(dir.path(), "abc.BASE", "views: []\n");
    write(dir.path(), ".bases/X.base", "views: []\n");
    write(dir.path(), "drafts/D.base", "views: []\n");

    let found: Vec<String> = walk(&root).members.into_iter().map(|m| m.path).collect();
    assert_eq!(
        found,
        vec!["keep.base".to_owned()],
        "case-folded `abc.BASE`, the dot-segment path, and the custom-ignored \
         path are all NON-members — one rule surface with the hash domain"
    );
}

// ---------------------------------------------------------------------------
// §10.1 test 4 — the constitutional pin
// ---------------------------------------------------------------------------

/// `.base` bytes are in NO fingerprint (§2/§12.1): adding, editing, and
/// removing a member leaves the workspace fingerprint byte-identical while
/// `base_fold` moves every time.
#[test]
fn base_motion_moves_the_fold_and_never_the_fingerprint() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let root = WorkspaceRoot(dir.path().to_path_buf());
    write(dir.path(), "note.md", "# Note\n");

    let fingerprint = || fs::domain_snapshot(&root).expect("snapshot").1.0;
    let fold = || walk(&root).fold;

    let (f0, b0) = (fingerprint(), fold());
    write(dir.path(), "A.base", "views: []\n");
    let (f1, b1) = (fingerprint(), fold());
    write(dir.path(), "A.base", "views: []\nformulas:\n  x: \"1\"\n");
    let (f2, b2) = (fingerprint(), fold());
    std::fs::remove_file(dir.path().join("A.base")).expect("rm");
    let (f3, b3) = (fingerprint(), fold());

    assert_eq!(
        [&f1, &f2, &f3],
        [&f0, &f0, &f0],
        "the workspace fingerprint is byte-identical across every base motion"
    );
    assert_ne!(b0, b1, "adding a member moves the fold");
    assert_ne!(b1, b2, "editing a member moves the fold");
    assert_eq!(b3, b0, "removing it returns the fold to where it started");
    assert!(b0.starts_with("bf:"), "the witness carries its own prefix");
}

// ---------------------------------------------------------------------------
// §10.1 test 5 — pairing
// ---------------------------------------------------------------------------

/// `exclusion_path` is set iff `exclusion` is set, and a bare-name `.base`
/// target carries the TIE-BROKEN path (§5.1: shortest path, then
/// lexicographic).
#[test]
fn exclusion_path_pairs_with_exclusion_and_carries_the_tie_break() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let root = WorkspaceRoot(dir.path().to_path_buf());
    write(
        dir.path(),
        "src.md",
        "![[TAG-FILES.base]] and [[nowhere]]\n",
    );
    // Two same-basename members: the tie-break must pick the shorter path.
    write(dir.path(), "deep/nested/TAG-FILES.base", "views: []\n");
    write(dir.path(), "b/TAG-FILES.base", "views: []\n");

    let conn = build(&root);
    let got = rows(
        &conn,
        "SELECT target_raw, exclusion, exclusion_path FROM link ORDER BY target_raw",
        3,
    );
    assert_eq!(
        got,
        vec![
            vec![
                "TAG-FILES.base".to_owned(),
                "non-md".to_owned(),
                "b/TAG-FILES.base".to_owned(),
            ],
            vec!["nowhere".to_owned(), "~N~".to_owned(), "~N~".to_owned()],
        ],
        "the stamped row names the tie-broken file; a genuine typo earns neither"
    );

    // The join §5.1 unlocks — no basename re-derivation in SQL.
    assert_eq!(
        one(
            &conn,
            "SELECT l.src_path FROM link l JOIN base b ON l.exclusion_path = b.path \
             WHERE b.path = 'b/TAG-FILES.base'"
        ),
        "src.md",
        "*who embeds this base* is a join"
    );
}

// ---------------------------------------------------------------------------
// §10.1 test 6 — verbatim expressions
// ---------------------------------------------------------------------------

/// `this.note["tag"]` survives byte-exact from YAML into `base.filters` JSON
/// (§4.2/§4.3 — the engine serves the Bases language, it never parses it).
#[test]
fn expressions_survive_byte_exact_into_the_json_columns() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let root = WorkspaceRoot(dir.path().to_path_buf());
    write(
        dir.path(),
        "TAG-FILES.base",
        "filters:\n  and:\n    - this.note[\"tag\"]\nformulas:\n  age: date() - file.ctime\n",
    );

    let conn = build(&root);
    assert_eq!(
        one(&conn, "SELECT filters FROM base"),
        r#"{"and":["this.note[\"tag\"]"]}"#,
        "the expression is a JSON string carrying its exact bytes"
    );
    assert_eq!(
        one(&conn, "SELECT expr FROM base_formula"),
        "date() - file.ctime",
        "a scalar formula is its own text, never re-quoted"
    );
}

// ---------------------------------------------------------------------------
// §10.1 test 9 — duplicate keys
// ---------------------------------------------------------------------------

/// One duplicated mapping key makes the whole file an alien — the PINNED
/// PARSER's rule, not YAML's (§4.4), pinned by fixture because the live corpus
/// has not produced one.
#[test]
fn a_duplicate_mapping_key_is_an_alien_with_zero_children() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let root = WorkspaceRoot(dir.path().to_path_buf());
    write(
        dir.path(),
        "dup.base",
        "views:\n  - type: table\n    groupBy: a\n    groupBy: b\n",
    );

    let conn = build(&root);
    assert_eq!(
        one(&conn, "SELECT (error IS NOT NULL)::VARCHAR FROM base"),
        "true",
        "the parser refuses the document rather than picking a winner"
    );
    assert_eq!(
        one(&conn, "SELECT count(*)::VARCHAR FROM base_view"),
        "0",
        "an error row has zero children"
    );
}

// ---------------------------------------------------------------------------
// §10.1 test 8 — the on-disk spelling mint rule
// ---------------------------------------------------------------------------

/// A literal target whose case mismatches the on-disk name does NOT stamp: the
/// row stays honestly dangling, and every stamped row's `exclusion_path` equals
/// a `base.path` byte-for-byte (§5.1).
///
/// On a case-SENSITIVE volume the mismatched spelling reaches no bytes at all,
/// so it is dangling for the ordinary reason — the assertion holds either way,
/// which is what makes it a portable pin of the rule rather than of the
/// filesystem.
#[test]
fn a_case_mismatched_literal_target_never_stamps() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let root = WorkspaceRoot(dir.path().to_path_buf());
    write(dir.path(), "bases/TASKS.base", TASKS_BASE);
    write(
        dir.path(),
        "src.md",
        "![[bases/tasks.base]] and ![[bases/TASKS.base]]\n",
    );

    let conn = build(&root);
    assert_eq!(
        one(
            &conn,
            "SELECT coalesce(exclusion,'~N~') FROM link WHERE target_raw='bases/tasks.base'"
        ),
        "~N~",
        "a spelling that reaches bytes only through case-folding is NOT verified"
    );
    assert_eq!(
        one(
            &conn,
            "SELECT target_raw FROM dangling WHERE target_raw='bases/tasks.base'"
        ),
        "bases/tasks.base",
        "it stays honestly dangling"
    );
    assert_eq!(
        one(
            &conn,
            "SELECT exclusion_path FROM link WHERE target_raw='bases/TASKS.base'"
        ),
        "bases/TASKS.base",
        "the on-disk spelling stamps, and it IS the base row's key"
    );
    // Every stamped `.base` row joins `base` exactly — no row stamps a path
    // the projection does not carry.
    assert_eq!(
        one(
            &conn,
            "SELECT count(*)::VARCHAR FROM link l \
             WHERE l.exclusion_path LIKE '%.base' \
               AND NOT EXISTS (SELECT 1 FROM base b WHERE b.path = l.exclusion_path)"
        ),
        "0",
        "a stamped .base path is always a base.path, byte-for-byte"
    );
}

// ---------------------------------------------------------------------------
// §6.2 amendment — the unreadable member's fold contribution
// ---------------------------------------------------------------------------

/// An unreadable member is DISTINCT in the witness from both a readable one and
/// an absent one (§6.2 amendment 2026-08-15): it contributes
/// `varint(len) ‖ path ‖ 0x01 ‖ zeroes`, so "seen but unreadable" can never
/// fold to the same value as "gone".
///
/// The fold is asserted through `model::base_fold` directly rather than through
/// an unreadable file on disk: making a read fail portably needs permissions
/// this test cannot rely on (a root-running CI reads a 0o000 file fine), and
/// the encoding — not the errno — is what the amendment rules.
#[test]
fn an_unreadable_member_folds_distinctly_from_readable_and_absent() {
    let leaf = model::leaf_digest(b"views: []\n");
    let readable = model::base_fold(&[model::BaseMemberLeaf {
        path: "A.base",
        leaf: Some(leaf),
    }]);
    let unreadable = model::base_fold(&[model::BaseMemberLeaf {
        path: "A.base",
        leaf: None,
    }]);
    let absent = model::base_fold(&[]);

    assert_ne!(
        unreadable, readable,
        "an unreadable member is not its readable self"
    );
    assert_ne!(
        unreadable, absent,
        "an unreadable member is not an absence — the §12.1 rule, inside the witness"
    );
    assert_ne!(readable, absent, "control: a readable member is not absent");
    for fold in [&readable, &unreadable, &absent] {
        assert!(fold.starts_with("bf:"), "every fold carries the bf: prefix");
    }
}

// ---------------------------------------------------------------------------
// §10.1 tests 7 + 10 — the cache lane
// ---------------------------------------------------------------------------

/// Sync `root`'s live state into `store`, both planes.
fn sync(store: &mut view::store::SqlStore, root: &WorkspaceRoot) {
    let (files, fingerprint) = fs::domain_snapshot(root).expect("domain snapshot");
    let (_index, docs, _unserved) = fs::build_corpus(files);
    let domain = fs::domain::Domain::load(root).expect("domain");
    let probe = fs::domain::LinkTargetProbe::new(root, &domain);
    let exclusion = |target: &str| {
        probe
            .resolution(target)
            .map(|(p, why)| (p, why.word().to_owned()))
    };
    let snapshot = walk(root);
    let owned = members(&snapshot);
    let base = view::BaseWalk {
        members: &owned,
        fold: &snapshot.fold,
    };
    let corpus = model::RootedCorpus::ambient(&docs);
    store
        .sync(
            &docs,
            &corpus,
            None,
            Some(&exclusion),
            &fingerprint.0,
            Some(&base),
        )
        .expect("sync");
}

/// One text column out of the cache's query lane.
fn cache_col(store: &view::store::SqlStore, sql: &str) -> Vec<String> {
    let (_cols, rows) = store.query(sql).expect("query lane").expect("caller sql");
    rows.iter()
        .map(|r| match &r[0] {
            serde_json::Value::Null => "~N~".to_owned(),
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .collect()
}

/// Base-only motion APPENDS: the fingerprint pin is unchanged, `base_fold`
/// advances, and the latest views carry the new rows — the store's standing
/// invariant, extended to the second plane (§7 / §10.1 test 7).
#[test]
fn base_only_motion_appends_under_an_unchanged_fingerprint() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let root = WorkspaceRoot(dir.path().to_path_buf());
    write(dir.path(), "note.md", "# Note\n");
    write(
        dir.path(),
        "A.base",
        "views:\n  - type: table\n    name: One\n",
    );

    let file = dir
        .path()
        .join("cache")
        .join(view::store::SQL_CACHE_FILENAME);
    let mut store = view::store::SqlStore::open(&file).expect("open");
    sync(&mut store, &root);
    let first = store.pin().expect("pin").expect("pinned");

    // The md corpus does not move; only the base plane does.
    write(
        dir.path(),
        "A.base",
        "views:\n  - type: cards\n    name: Two\n",
    );
    sync(&mut store, &root);
    let second = store.pin().expect("pin").expect("pinned");

    assert_eq!(
        second.fingerprint, first.fingerprint,
        "the md fingerprint did not move"
    );
    assert!(
        second.generation > first.generation,
        "base-only motion still APPENDS — it used to (wrongly) cost nothing"
    );
    assert_ne!(
        second.base_fold, first.base_fold,
        "the second witness advanced"
    );
    assert_eq!(
        cache_col(&store, "SELECT name FROM base_view ORDER BY ord"),
        vec!["Two".to_owned()],
        "the latest views serve the new definition, and only it"
    );

    // A third sync at rest is a no-op on BOTH witnesses.
    sync(&mut store, &root);
    assert_eq!(
        store.pin().expect("pin").expect("pinned").generation,
        second.generation,
        "nothing moved, so nothing appended"
    );
}

/// Deleting a `.base` member un-stamps its embed rows at the next append —
/// without a rebuild (§7 / §10.1 test 10). Under a dangling-only affected-set
/// predicate those rows would stay stamped forever while the cache reported
/// itself fresh.
#[test]
fn removing_a_base_member_clears_its_stamps_without_a_rebuild() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let root = WorkspaceRoot(dir.path().to_path_buf());
    write(dir.path(), "TAG-FILES.base", "views: []\n");
    write(dir.path(), "src.md", "![[TAG-FILES.base]]\n");

    let file = dir
        .path()
        .join("cache")
        .join(view::store::SQL_CACHE_FILENAME);
    let mut store = view::store::SqlStore::open(&file).expect("open");
    sync(&mut store, &root);
    assert_eq!(
        cache_col(&store, "SELECT exclusion_path FROM link"),
        vec!["TAG-FILES.base".to_owned()],
        "the embed row is stamped while the member exists"
    );

    std::fs::remove_file(dir.path().join("TAG-FILES.base")).expect("rm");
    sync(&mut store, &root);

    assert_eq!(
        cache_col(&store, "SELECT coalesce(exclusion,'~N~') FROM link"),
        vec!["~N~".to_owned()],
        "the stamp is cleared — the row is no longer explained"
    );
    assert_eq!(
        cache_col(&store, "SELECT coalesce(exclusion_path,'~N~') FROM link"),
        vec!["~N~".to_owned()],
        "and its path with it (the DDL holds the pair)"
    );
    assert_eq!(
        cache_col(&store, "SELECT target_raw FROM dangling"),
        vec!["TAG-FILES.base".to_owned()],
        "the row is back in the dangling census, where it now belongs"
    );
    assert_eq!(
        cache_col(&store, "SELECT count(*)::VARCHAR FROM base"),
        vec!["0".to_owned()],
        "the member itself is tombstoned out of the latest view"
    );
}

/// The cache's base surface equals a fresh ephemeral build of the same state —
/// the store's standing acceptance, on the new relations.
#[test]
fn the_cached_base_surface_equals_a_fresh_build() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let root = WorkspaceRoot(dir.path().to_path_buf());
    write(dir.path(), "note.md", "# Note\n");
    write(dir.path(), "bases/TASKS.base", TASKS_BASE);
    write(dir.path(), "x.base", "#!/bin/sh\necho alien\n");

    let file = dir
        .path()
        .join("cache")
        .join(view::store::SQL_CACHE_FILENAME);
    let mut store = view::store::SqlStore::open(&file).expect("open");
    sync(&mut store, &root);

    let digest = "SELECT coalesce(md5(string_agg(\
         path || '|' || coalesce(file_rev,'~N~') || '|' || coalesce(error,'~N~') || '|' || \
         coalesce(filters,'~N~') || '|' || coalesce(extra,'~N~'), \
         chr(10) ORDER BY path)), 'EMPTY') FROM base";

    let fresh = build(&root);
    let want: String = fresh
        .query_row(digest, [], |r| r.get(0))
        .expect("fresh digest");
    assert_eq!(
        cache_col(&store, digest),
        vec![want],
        "the cache's latest base view is bit-identical to a fresh build"
    );
}

/// A build handed NO walk stamps `base_fold` NULL — "not asked", never
/// "measured empty" (§6.2/§6.3). The one state silence would erase.
#[test]
fn a_build_with_no_walk_stamps_the_fold_null() {
    let docs: BTreeMap<String, model::Document> = BTreeMap::new();
    let conn = view::build_memory(&docs, "b3:empty").expect("build");
    assert_eq!(
        one(
            &conn,
            "SELECT coalesce(base_fold,'~N~') FROM _meridian_view"
        ),
        "~N~",
        "not asked is NULL; a walk that found nothing would carry a bf: fold instead"
    );
}
