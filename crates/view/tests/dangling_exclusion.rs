//! Dangling-base exclusion (ruling 2026-08-14, design (a)): **an excluded
//! target is not dangling.** `dangling` narrows from "broken vault refs" to
//! "broken vault refs with no exclusion explanation" — a row whose target is a
//! real, deliberately-unhashed file (`exclusion` stamped by the shared mint)
//! leaves the view; a genuine typo (no file ⇒ no reason ⇒ NULL) stays. Raw
//! rows remain reachable unchanged: `link WHERE dest_path IS NULL AND
//! dest_root IS NULL` is the escape hatch the view comment names.
//!
//! Both lanes are pinned — the `:memory:` view and the `sql.duckdb` cache's
//! `main.dangling` — because the DDL lives twice (schema.rs / store.rs) and a
//! one-lane edit would fork the contract.

use std::collections::BTreeMap;

use model::Document;

fn doc(raw: &str) -> Document {
    model::build(raw.to_string(), syntax::parse(raw))
}

/// One doc, two broken refs: `TAG-FILES.base` (a real out-of-domain file —
/// the injected probe stamps it `non-md`) and `nowhere` (a genuine typo — the
/// probe answers `None`, the row keeps NULL).
fn fixture() -> BTreeMap<String, Document> {
    let mut docs = BTreeMap::new();
    docs.insert(
        "notes.md".to_owned(),
        doc("# Notes\n\n[[TAG-FILES.base]]\n[[nowhere]]\n"),
    );
    docs
}

fn fold(docs: &BTreeMap<String, Document>) -> String {
    let files: Vec<(&str, &[u8])> = docs
        .iter()
        .map(|(path, d)| (path.as_str(), d.raw.as_bytes()))
        .collect();
    model::merkle_root(&files, 0).0
}

/// The caller-side mint stand-in: `.base` target excluded (`non-md`), the
/// typo unanswered. The view crate never probes disk itself.
fn probe(target: &str) -> Option<String> {
    (target == "TAG-FILES.base").then(|| "non-md".to_owned())
}

fn one_text(conn: &duckdb::Connection, sql: &str) -> Vec<String> {
    let mut stmt = conn.prepare(sql).expect("prepare");
    stmt.query_map([], |r| r.get::<_, Option<String>>(0))
        .expect("query")
        .map(|r| r.expect("row").unwrap_or_else(|| "NULL".to_owned()))
        .collect()
}

/// The `:memory:` lane. RED until the view carries `AND exclusion IS NULL`.
#[test]
fn an_excluded_target_is_not_dangling_memory_lane() {
    let docs = fixture();
    let corpus = model::RootedCorpus::ambient(&docs);
    let mounts = addr::MountSet::new([]);
    let conn = view::build_memory_rooted(&docs, &corpus, &mounts, &fold(&docs), Some(&probe))
        .expect("view");

    // The stamp landed where injected (control — the probe was consulted).
    assert_eq!(
        one_text(
            &conn,
            "SELECT coalesce(exclusion,'NULL') FROM link ORDER BY target_raw"
        ),
        vec!["non-md", "NULL"],
        "the .base row carries the minted word; the typo carries NO reason",
    );

    // The clause under test: an explained row is not dangling; the typo is.
    assert_eq!(
        one_text(&conn, "SELECT target_raw FROM dangling ORDER BY target_raw"),
        vec!["nowhere"],
        "dangling = broken refs with NO exclusion explanation",
    );

    // The escape hatch is unchanged: raw rows stay reachable, both of them.
    assert_eq!(
        one_text(
            &conn,
            "SELECT target_raw FROM link \
             WHERE dest_path IS NULL AND dest_root IS NULL ORDER BY target_raw"
        ),
        vec!["TAG-FILES.base", "nowhere"],
        "the raw link table keeps every unresolved row",
    );
}

/// The cache lane: `main.dangling` in `sql.duckdb` is the same contract.
#[test]
fn an_excluded_target_is_not_dangling_cache_lane() {
    let docs = fixture();
    let corpus = model::RootedCorpus::ambient(&docs);
    let tmp = tempfile::tempdir().expect("tempdir");
    let file = tmp.path().join(view::store::SQL_CACHE_FILENAME);
    let mut store = view::store::SqlStore::open(&file).expect("open");
    store
        .sync(&docs, &corpus, None, Some(&probe), &fold(&docs))
        .expect("sync");

    let (_, rows) = store
        .query("SELECT target_raw FROM dangling ORDER BY target_raw")
        .expect("query lane")
        .expect("caller sql");
    let targets: Vec<String> = rows
        .iter()
        .map(|r| r[0].as_str().expect("text").to_owned())
        .collect();
    assert_eq!(
        targets,
        vec!["nowhere"],
        "the cached main.dangling narrows identically to the :memory: view",
    );
}
