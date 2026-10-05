use super::{SqlStore, ViewError, tests};
use model::catalog::{DocumentCatalog, DocumentLease, EagerCatalog, RootedCatalog};
use std::{cell::Cell, io, rc::Rc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fault {
    None,
    Read,
    Missing,
    WrongVersion,
}

#[derive(Debug)]
struct Source<'a> {
    eager: EagerCatalog<'a>,
    fault: Cell<Fault>,
    target_calls: Cell<usize>,
    pins: Cell<usize>,
    live: Rc<Cell<usize>>,
    max_live: Cell<usize>,
    forbid_index: Cell<bool>,
}
impl<'a> Source<'a> {
    fn new(docs: &'a model::Docs) -> Self {
        Self {
            eager: EagerCatalog::new(docs),
            fault: Cell::new(Fault::None),
            target_calls: Cell::new(0),
            pins: Cell::new(0),
            live: Rc::default(),
            max_live: Cell::new(0),
            forbid_index: Cell::new(false),
        }
    }
}
#[derive(Debug)]
struct Lease<'a> {
    inner: Box<dyn DocumentLease + 'a>,
    live: Rc<Cell<usize>>,
}
impl DocumentLease for Lease<'_> {
    fn document(&self) -> io::Result<&model::Document> {
        self.inner.document()
    }
}
impl Drop for Lease<'_> {
    fn drop(&mut self) {
        self.live.set(self.live.get() - 1);
    }
}
impl DocumentCatalog for Source<'_> {
    fn revisions(&self) -> Box<dyn Iterator<Item = (&str, &model::NodeRev)> + '_> {
        self.eager.revisions()
    }
    fn revision(&self, path: &str) -> Option<&model::NodeRev> {
        self.eager.revision(path)
    }
    fn index(&self) -> &model::CorpusIndex {
        assert!(
            !self.forbid_index.get(),
            "unchanged sync rebuilt the name index"
        );
        self.eager.index()
    }
    fn pin(&self, path: &str) -> io::Result<Option<Box<dyn DocumentLease + '_>>> {
        self.pins.set(self.pins.get() + 1);
        let mut source_path = path;
        if path == "z.md" {
            let calls = self.target_calls.get() + 1;
            self.target_calls.set(calls);
            // Alias planning sees the true body. Fail only at projection,
            // after BEGIN and after earlier documents may have flushed.
            if calls == 2 {
                match self.fault.get() {
                    Fault::Read => return Err(io::Error::other("required backing unavailable")),
                    Fault::Missing => return Ok(None),
                    Fault::WrongVersion => source_path = "a.md",
                    Fault::None => {}
                }
            }
        }
        Ok(self.eager.pin(source_path)?.map(|inner| {
            self.live.set(self.live.get() + 1);
            self.max_live.set(self.max_live.get().max(self.live.get()));
            Box::new(Lease {
                inner,
                live: self.live.clone(),
            }) as Box<dyn DocumentLease>
        }))
    }
}

#[test]
fn catalog_stream_matches_all_eager_surfaces_and_noop_loads_nothing() {
    let docs = tests::fixture_v1();
    let source = Source::new(&docs);
    let roots = RootedCatalog::ambient(&source);
    let dir = tempfile::tempdir().unwrap();
    let mut store = SqlStore::open(&tests::tmp_store(&dir)).unwrap();
    store
        .sync_catalog(&source, &roots, None, None, "b3b:catalog", None)
        .unwrap();
    tests::assert_surface_matches_fresh(&store, &docs);
    assert_eq!(source.live.get(), 0);
    assert_eq!(
        source.max_live.get(),
        1,
        "SQL may pin only one body at a time"
    );
    let pins = source.pins.get();
    source.forbid_index.set(true);
    assert!(
        store
            .sync_catalog(&source, &roots, None, None, "b3b:catalog", None)
            .unwrap()
            .is_none()
    );
    assert_eq!(source.pins.get(), pins, "metadata no-op must not hydrate");
}

#[test]
fn catalog_late_source_fault_rolls_back_flushed_rows_and_pin() {
    let old = tests::corpus(&[("old.md", "# Old\n\nold body\n")]);
    let dir = tempfile::tempdir().unwrap();
    let mut store = SqlStore::open(&tests::tmp_store(&dir)).unwrap();
    tests::sync_ambient(&mut store, &old, "b3b:old");
    let raw = format!(
        "# Large\n\n[[flush-boundary]]\n\n{}\n",
        "x".repeat(33 * 1024 * 1024)
    );
    let docs = tests::corpus(&[("a.md", &raw), ("z.md", "# Last\n\nlast body\n")]);
    let source = Source::new(&docs);
    source.fault.set(Fault::Read);
    let roots = RootedCatalog::ambient(&source);
    let flushes = Cell::new(0);
    let exclusion = |target: &str| {
        assert_eq!(target, "flush-boundary");
        assert_eq!(
            source.live.get(),
            0,
            "SQL flush must start after the body lease ends"
        );
        flushes.set(flushes.get() + 1);
        None
    };
    super::MAX_STAGED_BODY_BYTES.with(|peak| peak.set(0));
    let error = store
        .sync_catalog(&source, &roots, None, Some(&exclusion), "b3b:new", None)
        .unwrap_err();
    assert!(matches!(error, ViewError::Io(_)), "{error}");
    assert!(error.to_string().contains("required backing unavailable"));
    assert_eq!(
        source.target_calls.get(),
        2,
        "failure must follow alias planning"
    );
    assert!(
        super::MAX_STAGED_BODY_BYTES.with(Cell::get) >= 33 * 1024 * 1024,
        "failure must follow a full staged batch"
    );
    assert_eq!(
        flushes.get(),
        1,
        "the probe must observe the flushed big member"
    );
    assert_eq!(source.live.get(), 0);
    assert_eq!(source.max_live.get(), 1);
    assert_eq!(store.pin().unwrap().unwrap().fingerprint, "b3b:old");
    for table in ["doc", "body", "pin", "body_text"] {
        let count: i64 = store
            .connection()
            .query_row(&format!("SELECT count(*) FROM hist.{table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 1, "failed append left history in {table}");
    }
    tests::assert_surface_matches_fresh(&store, &old);
    source.fault.set(Fault::None);
    let counts = store
        .sync_catalog(&source, &roots, None, Some(&exclusion), "b3b:new", None)
        .unwrap()
        .unwrap();
    assert_eq!((counts.generation, counts.added, counts.removed), (2, 2, 1));
    tests::assert_surface_matches_fresh(&store, &docs);
}

#[test]
fn catalog_missing_or_wrong_version_is_a_fault_not_a_skipped_member() {
    for fault in [Fault::Missing, Fault::WrongVersion] {
        let dir = tempfile::tempdir().unwrap();
        let mut store = SqlStore::open(&tests::tmp_store(&dir)).unwrap();
        let old = tests::corpus(&[("old.md", "# Old\n")]);
        tests::sync_ambient(&mut store, &old, "b3b:old");
        let docs = tests::corpus(&[("a.md", "# A\n"), ("z.md", "# Z\n")]);
        let source = Source::new(&docs);
        source.fault.set(fault);
        let roots = RootedCatalog::ambient(&source);
        let error = store
            .sync_catalog(&source, &roots, None, None, "b3b:new", None)
            .unwrap_err();
        assert!(matches!(error, ViewError::Io(_)), "{fault:?}: {error}");
        assert!(error.to_string().contains("z.md"));
        assert_eq!(source.live.get(), 0);
        assert_eq!(store.pin().unwrap().unwrap().fingerprint, "b3b:old");
        tests::assert_surface_matches_fresh(&store, &old);
    }
}

#[test]
fn catalog_rooted_resolution_reads_metadata_without_pinning_targets() {
    let docs = tests::corpus(&[("a.md", "# A\n\n[[b:remote]]\n")]);
    let targets = tests::corpus(&[("remote.md", "# Remote\n")]);
    let source = Source::new(&docs);
    let mounted = Source::new(&targets);
    let root = addr::MountName::parse("b").unwrap();
    let mounts = addr::MountSet::new([root.clone()]);
    let roots = RootedCatalog::ambient(&source).with_root(root, &mounted);
    let dir = tempfile::tempdir().unwrap();
    let mut store = SqlStore::open(&tests::tmp_store(&dir)).unwrap();
    store
        .sync_catalog(&source, &roots, Some(&mounts), None, "b3b:rooted", None)
        .unwrap();
    let dest: (String, String) = store
        .connection()
        .query_row("SELECT dest_root, dest_root_path FROM link", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap();
    assert_eq!(dest, ("b".to_owned(), "remote.md".to_owned()));
    assert_eq!(
        mounted.pins.get(),
        0,
        "name resolution hydrated a target body"
    );
    assert!(
        store
            .sync_catalog(&source, &roots, Some(&mounts), None, "b3b:rooted", None)
            .unwrap()
            .is_none()
    );
    assert_eq!(source.pins.get(), 1);
    assert_eq!(mounted.pins.get(), 0);
}
