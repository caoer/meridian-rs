//! C1 projection gates:
//! - `gate_stale_projection` — `doc_rev` staleness triggers rebuild

use std::collections::BTreeMap;

use model::Document;
use view::{open_board, stale_paths};

fn doc(raw: &str) -> Document {
    model::build(raw.to_string(), syntax::parse(raw))
}

/// Projection keyed on `doc_rev` — rev change is stale; rebuild refreshes (§2.1/§8).
#[test]
fn gate_stale_projection() {
    let subject_v1 = "# Subject\n\nThe reviewed claim body. ^claim\n";
    let subject_v2 = "# Subject\n\nThe body drifted. ^claim\n";

    let mut docs_v1 = BTreeMap::new();
    docs_v1.insert("subject.md".to_string(), doc(subject_v1));
    let conn = open_board(&docs_v1).expect("open board v1");

    assert!(
        stale_paths(&conn, &docs_v1)
            .expect("stale check")
            .is_empty(),
        "a projection is not stale against the docs it was built from"
    );

    let mut docs_v2 = BTreeMap::new();
    docs_v2.insert("subject.md".to_string(), doc(subject_v2));
    let stale = stale_paths(&conn, &docs_v2).expect("stale check");
    assert_eq!(
        stale,
        vec!["subject.md".to_string()],
        "a rev change is detected as stale (rev-compare invalidation)"
    );

    let conn2 = open_board(&docs_v2).expect("rebuild v2");
    assert!(
        stale_paths(&conn2, &docs_v2)
            .expect("stale check")
            .is_empty(),
        "the recomputed projection is fresh against v2"
    );
    let rev_v1 = docs_v1["subject.md"].root.node_rev.0.clone();
    let rev_v2 = docs_v2["subject.md"].root.node_rev.0.clone();
    assert_ne!(rev_v1, rev_v2, "the edit changed the doc_rev");
    let recorded: String = conn2
        .query_row(
            "SELECT DISTINCT doc_rev FROM node WHERE path='subject.md'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        recorded, rev_v2,
        "the recomputed node rows record the new doc_rev, not the stale one"
    );

    // An absent path is stale too (the projection cannot answer for it).
    let mut docs_new = BTreeMap::new();
    docs_new.insert("brand-new.md".to_string(), doc("# New\n"));
    assert_eq!(
        stale_paths(&conn2, &docs_new).expect("stale check"),
        vec!["brand-new.md".to_string()],
        "a path absent from the projection is stale"
    );
}
