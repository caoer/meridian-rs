use super::*;
use model::catalog::{DocumentCatalog, RootedCatalog};
use view::store::SqlStore;

fn docs(files: &[(&str, &str)]) -> Docs {
    files
        .iter()
        .map(|(path, raw)| {
            (
                (*path).to_owned(),
                Arc::new(model::build((*raw).to_owned(), syntax::parse(raw))),
            )
        })
        .collect()
}
fn sql(store: &SqlStore, statement: &str) -> Vec<Vec<serde_json::Value>> {
    store.query(statement).unwrap().unwrap().1
}

#[test]
fn cold_catalog_sql_keeps_historical_members_and_aliases_after_source_loss() {
    let dir = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let backing = RawStore::new(dir.path()).unwrap();
    let controller = Controller::new(1);
    let original = docs(&[
        ("a.md", "# A\n\n[[old-alias]] [[b:remote]]\n"),
        (
            "b.md",
            "---\naliases: [old-alias]\n---\n# B\n\nhistorical\n",
        ),
    ]);
    for (path, doc) in &original {
        std::fs::write(work.path().join(path), doc.raw.as_bytes()).unwrap();
    }
    let retained = Catalog::capture(&original, &controller, &backing, 131_072).unwrap();
    let mounted = Catalog::capture(
        &docs(&[("remote.md", "# Remote\n")]),
        &controller,
        &backing,
        131_072,
    )
    .unwrap();
    let root = addr::MountName::parse("b").unwrap();
    let mounts = addr::MountSet::new([root.clone()]);
    let expected = view::build_memory_rooted(
        &original,
        &model::RootedCorpus::ambient(&original)
            .with_root(root.clone(), &docs(&[("remote.md", "# Remote\n")])),
        &mounts,
        "b3b:historical",
        None,
        None,
    )
    .unwrap();
    let weak: Vec<_> = original.values().map(Arc::downgrade).collect();
    drop(original);
    assert!(
        weak.iter().all(|owner| owner.upgrade().is_none()),
        "catalog retained eager documents"
    );
    std::fs::write(work.path().join("a.md"), "# Replaced\n").unwrap();
    std::fs::remove_file(work.path().join("b.md")).unwrap();
    drop(work);
    drop(backing);
    assert_eq!(controller.report().bodies, 0);
    let roots = RootedCatalog::ambient(&retained).with_root(root, &mounted);
    let mut store = SqlStore::open(&dir.path().join("sql.duckdb")).unwrap();
    let counts = store
        .sync_catalog(
            &retained,
            &roots,
            Some(&mounts),
            None,
            "b3b:historical",
            None,
        )
        .unwrap()
        .unwrap();
    assert_eq!(counts.added, 2, "cold membership must remain complete");
    assert_eq!(
        sql(&store, "SELECT path FROM doc ORDER BY path"),
        vec![
            vec![serde_json::json!("a.md")],
            vec![serde_json::json!("b.md")]
        ]
    );
    for table in [
        "doc",
        "frontmatter",
        "section",
        "link",
        "body",
        "backlink",
        "dangling",
    ] {
        let statement = format!(
            "SELECT coalesce(string_agg(row::VARCHAR, chr(10) ORDER BY row::VARCHAR), '') FROM (SELECT * FROM {table}) row"
        );
        let actual: String = store
            .connection()
            .query_row(&statement, [], |row| row.get(0))
            .unwrap();
        let eager: String = expected
            .query_row(&statement, [], |row| row.get(0))
            .unwrap();
        assert_eq!(actual, eager, "cold/eager {table}");
    }
    assert_eq!(
        controller.report().bodies,
        0,
        "scan bodies must shed between members"
    );
    assert_eq!(controller.report().raw, 0);
    assert_eq!(controller.report().reserved, 0);
    assert_eq!(retained.revisions().count(), 2);
    assert!(retained.pin("absent.md").unwrap().is_none());
}

#[test]
fn cold_catalog_alias_and_mount_motion_reprojects_unchanged_sources() {
    let dir = tempfile::tempdir().unwrap();
    let backing = RawStore::new(dir.path()).unwrap();
    let controller = Controller::new(1);
    let before = Catalog::capture(
        &docs(&[
            ("a.md", "# A\n\n[[alias]] [[b:x.md]]\n"),
            ("old.md", "---\nalias: alias\n---\n# Old\n"),
        ]),
        &controller,
        &backing,
        131_072,
    )
    .unwrap();
    let after = Catalog::capture(
        &docs(&[
            ("a.md", "# A\n\n[[alias]] [[b:x.md]]\n"),
            ("new.md", "---\nalias: alias\n---\n# New\n"),
        ]),
        &controller,
        &backing,
        131_072,
    )
    .unwrap();
    let empty = Catalog::capture(&docs(&[]), &controller, &backing, 131_072).unwrap();
    let mounted =
        Catalog::capture(&docs(&[("x.md", "# X\n")]), &controller, &backing, 131_072).unwrap();
    let root = addr::MountName::parse("b").unwrap();
    let mounts = addr::MountSet::new([root.clone()]);
    let mut store = SqlStore::open(&dir.path().join("sql.duckdb")).unwrap();
    store
        .sync_catalog(
            &before,
            &RootedCatalog::ambient(&before).with_root(root.clone(), &empty),
            Some(&mounts),
            None,
            "b3b:before",
            None,
        )
        .unwrap();
    let roots = RootedCatalog::ambient(&after).with_root(root.clone(), &empty);
    let counts = store
        .sync_catalog(&after, &roots, Some(&mounts), None, "b3b:after", None)
        .unwrap()
        .unwrap();
    assert_eq!((counts.added, counts.changed, counts.removed), (1, 1, 1));
    assert_eq!(
        sql(
            &store,
            "SELECT dest_path FROM link WHERE target_raw = 'alias'"
        ),
        vec![vec![serde_json::json!("new.md")]]
    );
    let roots = RootedCatalog::ambient(&after).with_root(root, &mounted);
    let counts = store
        .sync_catalog(&after, &roots, Some(&mounts), None, "b3b:after", None)
        .unwrap()
        .unwrap();
    assert_eq!((counts.added, counts.changed, counts.removed), (0, 1, 0));
    assert_eq!(
        sql(
            &store,
            "SELECT dest_root, dest_root_path FROM link WHERE target_raw = 'b:x.md'"
        ),
        vec![vec![serde_json::json!("b"), serde_json::json!("x.md")]]
    );
    assert!(
        store
            .sync_catalog(&after, &roots, Some(&mounts), None, "b3b:after", None)
            .unwrap()
            .is_none()
    );
    assert_eq!(controller.report().bodies, 0);
    assert_eq!(controller.report().reserved, 0);
}

#[test]
fn catalog_scan_preserves_the_oldest_hot_member_for_eviction() {
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
    let read = |path: &str| {
        drop(
            catalog.entries[path]
                .member
                .pin(Access::Read, PEAK)
                .unwrap(),
        );
    };
    read("a.md");
    read("b.md");
    let two_hot = controller.report().total;
    drop(catalog.pin("a.md").unwrap().unwrap());
    // Admission is one byte short while both bodies are hot. Shedding one
    // member must suffice; the scan of a must not make b the oldest member.
    controller.set_limit(two_hot + PEAK - 1);
    read("c.md");
    assert_eq!(
        catalog.entries["a.md"].member.phase(),
        super::super::Phase::Cold,
        "a catalog scan must not promote the oldest member"
    );
    assert_eq!(
        catalog.entries["b.md"].member.phase(),
        super::super::Phase::Hot
    );
    assert_eq!(
        catalog.entries["c.md"].member.phase(),
        super::super::Phase::Hot
    );
    assert_eq!(controller.report().reserved, 0);
}

#[test]
fn cold_catalog_names_link_addressed_roots_without_reading_a_body() {
    let dir = tempfile::tempdir().unwrap();
    let backing = RawStore::new(dir.path()).unwrap();
    let controller = Controller::new(1);
    let original = docs(&[
        ("a.md", "# A\n\n[[b:remote]] and [[local]]\n"),
        ("n/c.md", "# C\n\n![[c:img.png]]\n\n- [[b:other#Heading]]\n"),
        ("plain.md", "# Plain\n\nno links\n"),
    ]);
    let expected = view::walk::link_addressed_roots(&original, None);
    assert_eq!(
        expected,
        ["b", "c"]
            .into_iter()
            .map(|name| addr::MountName::parse(name).unwrap())
            .collect::<BTreeSet<_>>(),
        "the fixture addresses two roots"
    );
    let catalog = Catalog::capture(&original, &controller, &backing, 131_072).unwrap();
    drop(original);
    assert_eq!(controller.report().bodies, 0);
    // Truncate every retained inode: from here a body pin can only fault.
    for entry in std::fs::read_dir(dir.path()).unwrap() {
        for file in std::fs::read_dir(entry.unwrap().path()).unwrap() {
            std::fs::write(file.unwrap().path(), []).unwrap();
        }
    }
    assert!(
        catalog.pin("a.md").is_err(),
        "the instrument: a body of this catalog cannot be read"
    );
    assert_eq!(
        catalog.link_roots(),
        expected,
        "link-addressed roots are descriptor facts, not read off bodies"
    );
    assert_eq!(controller.report().bodies, 0);
}

#[test]
fn cold_catalog_names_lock_holders_and_lock_roots_without_reading_a_body() {
    use view::walk::LockFacts;

    fn pinning(objects: &[&str]) -> String {
        let mut l = lock::Lock::new();
        for object in objects {
            l.upsert_pin(lock::PinEntry::new(
                object,
                "9ae3f1deadbeef",
                lock::Selector::Path(Vec::new()),
                &format!("fp1.span2.b3.{}", "0".repeat(64)),
            ));
        }
        format!("# Page\n\n{}\n", lock::render(&l))
    }

    let dir = tempfile::tempdir().unwrap();
    let backing = RawStore::new(dir.path()).unwrap();
    let controller = Controller::new(1);
    let original = docs(&[
        ("a.md", &pinning(&["b:remote", "local"])),
        ("n/c.md", &pinning(&["c:other"])),
        ("local.md", &pinning(&["plain"])),
        ("plain.md", "# Plain\n\nno lock, [[b:linked]] only\n"),
    ]);
    let expected = view::walk::lock_addressed_roots(&original);
    assert_eq!(
        expected,
        ["b", "c"]
            .into_iter()
            .map(|name| addr::MountName::parse(name).unwrap())
            .collect::<BTreeSet<_>>(),
        "the fixture's locks address two roots"
    );
    let catalog = Catalog::capture(&original, &controller, &backing, 131_072).unwrap();
    drop(original);
    assert_eq!(controller.report().bodies, 0);
    // Truncate every retained inode: from here a body pin can only fault.
    for entry in std::fs::read_dir(dir.path()).unwrap() {
        for file in std::fs::read_dir(entry.unwrap().path()).unwrap() {
            std::fs::write(file.unwrap().path(), []).unwrap();
        }
    }
    assert!(
        catalog.pin("a.md").is_err(),
        "the instrument: a body of this catalog cannot be read"
    );
    assert_eq!(
        catalog.lock_roots(),
        expected,
        "lock-addressed roots are descriptor facts, not read off bodies"
    );
    assert_eq!(
        catalog.lock_holders().collect::<Vec<_>>(),
        ["a.md", "local.md", "n/c.md"],
        "which members carry a lock row is a descriptor fact"
    );
    assert_eq!(controller.report().bodies, 0);
}
