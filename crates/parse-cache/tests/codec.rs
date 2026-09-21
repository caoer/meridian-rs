use model::{Document, NodeKind};

fn document(raw: &str) -> Document {
    model::build(raw.to_owned(), syntax::parse(raw))
}

#[test]
fn parsed_document_round_trips_byte_exactly() {
    let raw = "---\ntags:\n  - one\ndescription: |\n  multiline 值\n---\n# Hello / 世界\n\ntext [[Other#Sub|alias]] ![[image]]\n\n## Nested\n\n- [x] done\n\n> [!NOTE]+ Title\n> details\n\n| A | B |\n|---|---|\n| 1 | 2 |\n\n~~~rust\nlet x = 1;\n~~~\n\n`inline` %% comment %%\n\nparagraph ^anchor\n";
    let doc = document(raw);
    let bytes = parse_cache::encode(&doc).unwrap();
    let restored = parse_cache::decode(&bytes, &model::leaf_digest(raw.as_bytes())).unwrap();
    assert_eq!(restored.raw, doc.raw);
    assert_eq!(restored.root, doc.root);
}

#[test]
fn empty_document_and_unicode_boundaries_round_trip() {
    for raw in ["", "#", "世界\r\n", "# A\r\n\r\n## B\r\n"] {
        let doc = document(raw);
        let bytes = parse_cache::encode(&doc).unwrap();
        assert_eq!(
            parse_cache::decode(&bytes, &model::leaf_digest(raw.as_bytes()))
                .unwrap()
                .root,
            doc.root
        );
    }
}

#[test]
fn wrong_content_truncation_and_trailing_bytes_are_misses() {
    let doc = document("# A\n\nbody\n");
    let bytes = parse_cache::encode(&doc).unwrap();
    let digest = model::leaf_digest(doc.raw.as_bytes());
    assert!(parse_cache::decode(&bytes, &model::leaf_digest(b"other")).is_none());
    for end in 0..bytes.len() {
        assert!(
            parse_cache::decode(&bytes[..end], &digest).is_none(),
            "truncation {end}"
        );
    }
    let mut extra = bytes.clone();
    extra.push(0);
    assert!(parse_cache::decode(&extra, &digest).is_none());
    let mut huge = bytes;
    huge[..8].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(parse_cache::decode(&huge, &digest).is_none());
}

#[test]
fn invalid_spans_revs_and_nested_documents_are_misses() {
    let original = document("# A\n\n世界\n");
    let digest = model::leaf_digest(original.raw.as_bytes());
    let mut bad = original.clone();
    bad.root.children[0].span.end = usize::MAX;
    assert!(parse_cache::decode(&parse_cache::encode(&bad).unwrap(), &digest).is_none());
    bad = original.clone();
    bad.root.children[0].node_rev.0 = "0000000000000000".to_owned();
    assert!(parse_cache::decode(&parse_cache::encode(&bad).unwrap(), &digest).is_none());
    bad = original;
    bad.root.children[0].kind = NodeKind::Document {
        path: String::new(),
        line_count: 1,
    };
    assert!(parse_cache::decode(&parse_cache::encode(&bad).unwrap(), &digest).is_none());
}

#[test]
fn depth_limit_is_a_cache_miss_not_a_recursive_decoder_crash() {
    let mut doc = document("");
    let mut node = doc.root.clone();
    node.kind = NodeKind::Paragraph;
    for _ in 0..100 {
        let mut parent = node.clone();
        parent.children = vec![node];
        node = parent;
    }
    doc.root.children.push(node);
    assert!(parse_cache::encode(&doc).is_none());
}
