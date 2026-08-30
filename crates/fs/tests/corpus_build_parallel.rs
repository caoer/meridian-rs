//! `fs::build_corpus` output law: the parse is chunked across worker threads
//! (the cold-start amplifier fix — a whole-corpus parse was the serial half of
//! every daemon restart), and the LAW is that parallelism must be
//! unobservable in the product. Same docs, same unserved map, same index
//! answers as the one-member-at-a-time loop, whatever the chunking did.
//!
//! Guarded here, above the floor, because the fast path is the one nobody
//! reads: a chunk boundary that dropped or reordered a member would still
//! compile, still parse most of the corpus, and only answer wrong on the
//! member it lost.

use std::collections::BTreeMap;

/// A corpus wide enough to cross `build_corpus`'s parallel floor several
/// chunks over, with the two degradation shapes threaded through it:
/// non-UTF-8 members (per-file degrade, never poison) and duplicate basenames
/// in different directories (multi-candidate index order).
fn wide_mixed_corpus() -> fs::DomainFiles {
    let mut files: fs::DomainFiles = Vec::new();
    for i in 0..300 {
        let rel = format!("dir{}/note-{i:03}.md", i % 7);
        let body = format!("# Note {i}\n\nmarker-{i:03} body line.\n\n## Sub {i}\n\ntail.\n");
        files.push((rel, body.into_bytes()));
    }
    // Two members that are not UTF-8, spread into different chunks.
    files.insert(41, ("bad/one.bin.md".to_owned(), vec![0xff, 0xfe, 0x01]));
    files.insert(250, ("bad/two.bin.md".to_owned(), vec![0xc3, 0x28]));
    // Duplicate basename in two directories — index candidate order is part
    // of the product (`corpus_index_of` runs over the final map's path order).
    files.push(("alpha/shared.md".to_owned(), b"# Shared A\n".to_vec()));
    files.push(("beta/shared.md".to_owned(), b"# Shared B\n".to_vec()));
    files
}

#[test]
fn parallel_build_matches_the_serial_law_member_for_member() {
    let files = wide_mixed_corpus();
    let expected_members = files.len();

    let (index, docs, unserved) = fs::build_corpus(files.clone());

    // Every member landed in exactly one of docs/unserved — nothing dropped,
    // nothing doubled, whatever chunk it rode.
    assert_eq!(docs.len() + unserved.len(), expected_members);
    assert_eq!(unserved.len(), 2, "exactly the two non-UTF-8 members degrade");
    for (rel, condition) in &unserved {
        assert!(rel.starts_with("bad/"), "degraded member is the planted one: {rel}");
        assert!(
            condition.contains("is not UTF-8"),
            "the condition names the refusal, not a generic error: {condition}"
        );
    }

    // Per-member product is the serial one: the document's raw is the very
    // bytes handed in (parse borrows, then moves — no copy, no truncation).
    for (rel, bytes) in &files {
        if rel.starts_with("bad/") {
            assert!(unserved.contains_key(rel), "{rel} degrades");
            continue;
        }
        let doc = docs.get(rel).unwrap_or_else(|| panic!("{rel} parsed"));
        assert_eq!(doc.raw.as_bytes(), &bytes[..], "{rel} raw round-trips");
    }

    // The index answers over the final map: a unique basename resolves, and
    // the duplicate-basename order is the docs map's own path order — the
    // one-index-constructor law that keeps build paths from disagreeing.
    assert_eq!(
        index.resolve_linkpath("note-123", "dir0/other.md").as_deref(),
        Some("dir4/note-123.md")
    );
    assert_eq!(
        index.resolve_linkpath("alpha/shared", "elsewhere.md").as_deref(),
        Some("alpha/shared.md"),
        "qualified linkpath picks its own directory"
    );

    // Determinism: a second build over the same bytes is byte-identical —
    // chunk scheduling must never show through.
    let (_, docs2, unserved2) = fs::build_corpus(files);
    assert_eq!(unserved, unserved2);
    assert_eq!(
        docs.keys().collect::<Vec<_>>(),
        docs2.keys().collect::<Vec<_>>()
    );
    for (rel, doc) in &docs {
        let again = &docs2[rel];
        assert_eq!(doc.raw, again.raw, "{rel} raw stable");
        assert_eq!(
            format!("{:?}", doc.root),
            format!("{:?}", again.root),
            "{rel} parse tree stable"
        );
    }
}

/// Below the parallel floor the same law holds on the serial path — the floor
/// is a latency choice, never a behavior fork.
#[test]
fn a_small_corpus_builds_identically_on_the_serial_path() {
    let files: fs::DomainFiles = vec![
        ("a.md".to_owned(), b"# A\n\nbody\n".to_vec()),
        ("b.md".to_owned(), vec![0xff, 0x00]),
        ("c/d.md".to_owned(), b"# D\n".to_vec()),
    ];
    let (index, docs, unserved) = fs::build_corpus(files);
    assert_eq!(docs.len(), 2);
    assert_eq!(
        unserved,
        BTreeMap::from([("b.md".to_owned(), unserved["b.md"].clone())]),
        "only the non-UTF-8 member degrades"
    );
    assert_eq!(index.resolve_linkpath("d", "a.md").as_deref(), Some("c/d.md"));
}
