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
    let catalog = Catalog::capture(&docs, &controller, &backing).unwrap();
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
    let catalog = Catalog::capture(&files, &controller, &backing).unwrap();
    drop(files);
    // The three pages are one length, so one peak.
    let peak = super::super::load_peak(catalog.entries["c.md"].member.identity().len);
    let read_member = |path: &str| {
        drop(
            catalog.entries[path]
                .member
                .pin(Access::Read, peak)
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
    controller.set_limit(two_hot + peak - 1);
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

#[test]
fn cold_catalog_link_scan_covers_every_member_after_body_and_source_loss() {
    let dir = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let root = fs::WorkspaceRoot(work.path().to_path_buf());
    let backing = RawStore::new(dir.path()).unwrap();
    let controller = Controller::new(1);
    let mut index = CorpusIndex::new();
    let docs: Docs = [
        ("a.md", "# A\n\n[[b]] ![[c]] [[nowhere]]\n"),
        ("b.md", "# B\n\n[[a]] [[#B]]\n"),
        ("c.md", "# C\n\nno edges\n"),
    ]
    .into_iter()
    .map(|(path, raw)| {
        let doc = model::build(raw.to_owned(), syntax::parse(raw));
        index.insert(path, &doc);
        (path.to_owned(), Arc::new(doc))
    })
    .collect();
    let frame = |catalog: &dyn DocumentCatalog, path: Option<&str>| {
        let stamp = wire::Root("b3b:selected-generation".to_owned());
        let live = stamp.clone();
        let body = read::links(
            &root,
            &index,
            catalog,
            &BTreeMap::new(),
            path.map(|p| wire::Path(p.to_owned())).as_ref(),
            stamp,
            0,
            || Ok(live),
        )
        .unwrap();
        serde_json::to_value(body).unwrap()
    };
    let eager = model::catalog::EagerCatalog::new(&docs);
    let expected = [frame(&eager, None), frame(&eager, Some("b.md"))];
    let catalog = Catalog::capture(&docs, &controller, &backing).unwrap();
    drop(docs);
    drop(backing);
    assert_eq!(controller.report().bodies, 0);
    assert_eq!(
        [frame(&catalog, None), frame(&catalog, Some("b.md"))],
        expected,
        "a cold member keeps its edge-map entry"
    );
    let after = controller.report();
    assert_eq!(
        (after.bodies, after.pinned, after.reserved),
        (0, 0, 0),
        "the scan retains no body"
    );
}
