use super::*;
use model::catalog::{DocumentLease, EagerCatalog};
use std::{cell::Cell, io, rc::Rc, sync::Arc};
use wire::{ErrorCode, ResponseBody, Root};

#[derive(Clone, Copy, Debug)]
enum Fault {
    None,
    Missing,
    WrongVersion,
    Read,
    InvalidBacking,
    Document,
}
#[derive(Debug)]
struct Source<'a> {
    eager: EagerCatalog<'a>,
    fault: Cell<Fault>,
    scans: Cell<usize>,
    reads: Cell<usize>,
    live: Rc<Cell<usize>>,
}
impl<'a> Source<'a> {
    fn new(docs: &'a model::Docs) -> Self {
        Self {
            eager: EagerCatalog::new(docs),
            fault: Cell::new(Fault::None),
            scans: Cell::new(0),
            reads: Cell::new(0),
            live: Rc::default(),
        }
    }
    fn lease(&self, path: &str) -> io::Result<Option<Box<dyn DocumentLease + '_>>> {
        let path = match self.fault.get() {
            Fault::Missing => return Ok(None),
            Fault::WrongVersion => "other.md",
            Fault::Read => {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "backing unavailable",
                ));
            }
            Fault::InvalidBacking => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "backing identity mismatch",
                ));
            }
            Fault::None | Fault::Document => path,
        };
        Ok(self.eager.pin(path)?.map(|inner| {
            self.live.set(self.live.get() + 1);
            Box::new(Lease {
                inner,
                live: self.live.clone(),
                document_fault: matches!(self.fault.get(), Fault::Document),
            }) as Box<dyn DocumentLease>
        }))
    }
}
#[derive(Debug)]
struct Lease<'a> {
    inner: Box<dyn DocumentLease + 'a>,
    live: Rc<Cell<usize>>,
    document_fault: bool,
}
impl DocumentLease for Lease<'_> {
    fn document(&self) -> io::Result<&model::Document> {
        if self.document_fault {
            return Err(io::Error::other("retained document unavailable"));
        }
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
        panic!("a named read must not enumerate the catalog");
    }
    fn revision(&self, path: &str) -> Option<&model::NodeRev> {
        self.eager.revision(path)
    }
    fn link_roots(&self) -> std::collections::BTreeSet<addr::MountName> {
        self.eager.link_roots()
    }
    fn index(&self) -> &model::CorpusIndex {
        panic!("a named body read needs no name index");
    }
    fn pin(&self, path: &str) -> io::Result<Option<Box<dyn DocumentLease + '_>>> {
        self.scans.set(self.scans.get() + 1);
        self.lease(path)
    }
    fn pin_read(&self, path: &str) -> io::Result<Option<Box<dyn DocumentLease + '_>>> {
        self.reads.set(self.reads.get() + 1);
        self.lease(path)
    }
}
fn docs() -> model::Docs {
    [
        ("a.md", "---\ntitle: Old\n---\n# 原来\n\nhistorical body\n"),
        ("other.md", "# Different\n"),
    ]
    .into_iter()
    .map(|(p, raw)| {
        (
            p.to_owned(),
            Arc::new(model::build(raw.to_owned(), syntax::parse(raw))),
        )
    })
    .collect()
}
fn projections(doc: &model::Document, path: &Path) -> Vec<serde_json::Value> {
    let root = Root("b3b:selected-generation".to_owned());
    vec![
        super::super::toc(doc, path, &root),
        super::super::cat(doc, None).unwrap(),
        super::super::extract(doc, path, None, true),
        super::super::composed_read(
            doc,
            path,
            &root,
            &super::super::ReadParams::default(),
            &super::super::NO_DECORATIONS,
        )
        .unwrap(),
    ]
    .into_iter()
    .map(|body| serde_json::to_value(body).unwrap())
    .collect()
}

#[test]
fn named_catalog_preserves_the_selected_body_across_source_motion() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    let docs = docs();
    let source = Source::new(&docs);
    let path = Path("a.md".to_owned());
    let expected = projections(&docs["a.md"], &path);
    std::fs::write(dir.path().join("a.md"), "# Newer source\n").unwrap();
    for turn in 0..2 {
        if turn == 1 {
            std::fs::remove_file(dir.path().join("a.md")).unwrap();
        }
        let got = with_document(&root, &source, &BTreeMap::new(), &path, |doc| {
            assert_eq!(
                source.live.get(),
                1,
                "the body must be pinned during projection"
            );
            Ok(projections(doc, &path))
        })
        .unwrap();
        assert_eq!(
            got, expected,
            "named reads must keep their selected generation"
        );
        assert_eq!(source.live.get(), 0);
    }
    assert_eq!(source.reads.get(), 2);
    assert_eq!(
        source.scans.get(),
        0,
        "interactive reads must request read recency"
    );
}

#[test]
fn named_catalog_faults_never_fall_through_to_a_live_file() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    std::fs::write(dir.path().join("a.md"), "# Current source is readable\n").unwrap();
    let docs = docs();
    let source = Source::new(&docs);
    let path = Path("a.md".to_owned());
    for fault in [
        Fault::Missing,
        Fault::WrongVersion,
        Fault::Read,
        Fault::InvalidBacking,
        Fault::Document,
    ] {
        source.fault.set(fault);
        let error = with_document(&root, &source, &BTreeMap::new(), &path, |doc| {
            super::super::cat(doc, None)
        })
        .unwrap_err();
        assert_eq!(
            error.code,
            ErrorCode::IoError,
            "{fault:?} is a retained-version fault"
        );
        assert_eq!(error.path.as_ref(), Some(&path));
        assert!(error.cause.as_deref().is_some_and(|c| c.contains("a.md")));
        assert_eq!(source.live.get(), 0);
    }
}

#[test]
fn named_catalog_nonmember_and_unserved_doors_keep_their_meaning() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    let docs = docs();
    let source = Source::new(&docs);
    std::fs::write(dir.path().join("outside.md"), "# Explicit path\n").unwrap();
    std::fs::write(dir.path().join("bad.md"), "# A newer valid source\n").unwrap();
    let unserved = BTreeMap::from([("bad.md".to_owned(), "is not UTF-8".to_owned())]);
    let error = with_document(
        &root,
        &source,
        &unserved,
        &Path("bad.md".to_owned()),
        |doc| super::super::cat(doc, None),
    )
    .unwrap_err();
    assert_eq!(
        error.code,
        ErrorCode::InvalidUtf8,
        "the selected unserved condition must win over live bytes"
    );
    let result = with_document(
        &root,
        &source,
        &unserved,
        &Path("outside.md".to_owned()),
        |doc| super::super::cat(doc, None),
    )
    .unwrap();
    assert!(matches!(result, ResponseBody::Cat { content, .. } if content == "# Explicit path\n"));
    let error = with_document(
        &root,
        &source,
        &unserved,
        &Path("absent.md".to_owned()),
        |doc| super::super::cat(doc, None),
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::FileNotFound);
    assert_eq!(source.scans.get() + source.reads.get(), 0);
}

#[test]
fn named_catalog_callback_refusal_releases_its_body() {
    let dir = tempfile::tempdir().unwrap();
    let docs = docs();
    let source = Source::new(&docs);
    let error = with_document::<()>(
        &fs::WorkspaceRoot(dir.path().to_path_buf()),
        &source,
        &BTreeMap::new(),
        &Path("a.md".to_owned()),
        |_| {
            assert_eq!(source.live.get(), 1);
            Err(crate::bad_request("projection refused"))
        },
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::BadRequest);
    assert_eq!(source.live.get(), 0);
}
