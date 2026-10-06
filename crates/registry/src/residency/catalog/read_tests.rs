use super::*;
use wire_serve::read;

fn projections(doc: &model::Document, path: &wire::Path) -> Vec<serde_json::Value> {
    let root = wire::Root("b3b:selected-generation".to_owned());
    vec![
        read::toc(doc, path, &root),
        read::cat(doc, None).unwrap(),
        read::extract(doc, path, None, true),
        read::composed_read(
            doc,
            path,
            &root,
            &read::ReadParams::default(),
            &read::NO_DECORATIONS,
        )
        .unwrap(),
    ]
    .into_iter()
    .map(|body| serde_json::to_value(body).unwrap())
    .collect()
}

#[test]
fn named_cold_catalog_serves_old_wire_facts_after_body_and_source_loss() {
    let dir = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let root = fs::WorkspaceRoot(work.path().to_path_buf());
    let backing = RawStore::new(dir.path()).unwrap();
    let controller = Controller::new(1);
    let raw = "---\ntitle: Retained\n---\n# 原来\n\nhistorical bytes\n";
    let document = Arc::new(model::build(raw.to_owned(), syntax::parse(raw)));
    let weak = Arc::downgrade(&document);
    let path = wire::Path("a.md".to_owned());
    let expected = projections(&document, &path);
    let docs = Docs::from([("a.md".to_owned(), document)]);
    let catalog = Catalog::capture(&docs, &controller, &backing, 131_072).unwrap();
    drop(docs);
    drop(backing);
    assert!(weak.upgrade().is_none());
    std::fs::write(work.path().join("a.md"), "# New source\n").unwrap();
    for turn in 0..2 {
        if turn == 1 {
            std::fs::remove_file(work.path().join("a.md")).unwrap();
        }
        assert_eq!(controller.report().bodies, 0);
        let got = read::with_document(&root, &catalog, &BTreeMap::new(), &path, |doc| {
            assert!(
                controller.report().pinned > 0,
                "serving must protect the selected body"
            );
            Ok(projections(doc, &path))
        })
        .unwrap();
        assert_eq!(
            got, expected,
            "cold serving must preserve historical wire facts"
        );
        assert_eq!(controller.report().bodies, 0);
        assert_eq!(controller.report().raw, 0);
        assert_eq!(controller.report().reserved, 0);
    }
}

#[test]
fn named_cold_catalog_read_promotes_interactive_recency() {
    const PEAK: usize = 131_072;
    let dir = tempfile::tempdir().unwrap();
    let backing = RawStore::new(dir.path()).unwrap();
    let controller = Controller::new(0);
    let files: Docs = ["a.md", "b.md", "c.md"]
        .into_iter()
        .map(|path| {
            let raw = format!("# {path}\n\n{}\n", "x".repeat(2048));
            let parsed = syntax::parse(&raw);
            (path.to_owned(), Arc::new(model::build(raw, parsed)))
        })
        .collect();
    let catalog = Catalog::capture(&files, &controller, &backing, PEAK).unwrap();
    drop(files);
    let read_member = |path: &str| {
        drop(
            catalog.entries[path]
                .member
                .pin(Access::Read, PEAK)
                .unwrap(),
        );
    };
    read_member("a.md");
    read_member("b.md");
    let two_hot = controller.report().total;
    read::with_document(
        &fs::WorkspaceRoot(dir.path().to_path_buf()),
        &catalog,
        &BTreeMap::new(),
        &wire::Path("a.md".to_owned()),
        |doc| read::cat(doc, None),
    )
    .unwrap();
    controller.set_limit(two_hot + PEAK - 1);
    read_member("c.md");
    assert_eq!(
        catalog.entries["a.md"].member.phase(),
        super::super::Phase::Hot
    );
    assert_eq!(
        catalog.entries["b.md"].member.phase(),
        super::super::Phase::Cold,
        "an interactive read must promote the selected member"
    );
    assert_eq!(
        catalog.entries["c.md"].member.phase(),
        super::super::Phase::Hot
    );
}
