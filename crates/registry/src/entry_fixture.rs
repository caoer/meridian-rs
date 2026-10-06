//! A generation whose bodies can be made unleasable, for the entry-world
//! seams (merkle-spec §6.8). Membership and revisions stay answerable.

use model::catalog::{DocumentCatalog, DocumentLease, EagerCatalog};
use std::{
    collections::BTreeSet,
    io,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fault {
    None,
    /// The retained bytes cannot be served.
    Backing,
    /// Admission refused the lease for now.
    Transient,
}

#[derive(Debug)]
pub(crate) struct Entry<'a> {
    eager: EagerCatalog<'a>,
    fault: Mutex<Fault>,
    scans: AtomicUsize,
    reads: AtomicUsize,
}

impl<'a> Entry<'a> {
    pub(crate) fn new(docs: &'a model::Docs) -> Self {
        Self {
            eager: EagerCatalog::new(docs),
            fault: Mutex::new(Fault::None),
            scans: AtomicUsize::new(0),
            reads: AtomicUsize::new(0),
        }
    }

    /// Every lease from now on meets `fault`.
    pub(crate) fn fail(&self, fault: Fault) {
        *self.fault.lock().unwrap() = fault;
    }

    /// Scan pins asked for so far.
    pub(crate) fn scans(&self) -> usize {
        self.scans.load(Ordering::Relaxed)
    }

    /// Interactive leases asked for so far.
    pub(crate) fn reads(&self) -> usize {
        self.reads.load(Ordering::Relaxed)
    }

    fn lease(&self, path: &str) -> io::Result<Option<Box<dyn DocumentLease + '_>>> {
        let fault = *self.fault.lock().unwrap();
        match fault {
            Fault::None => self.eager.pin(path),
            Fault::Backing => Err(io::Error::other("required backing unavailable")),
            Fault::Transient => Err(model::catalog::transient("residency allowance is occupied")),
        }
    }
}

impl DocumentCatalog for Entry<'_> {
    fn revisions(&self) -> Box<dyn Iterator<Item = (&str, &model::NodeRev)> + '_> {
        self.eager.revisions()
    }
    fn revision(&self, path: &str) -> Option<&model::NodeRev> {
        self.eager.revision(path)
    }
    fn index(&self) -> &model::CorpusIndex {
        self.eager.index()
    }
    fn link_roots(&self) -> BTreeSet<addr::MountName> {
        self.eager.link_roots()
    }
    fn pin(&self, path: &str) -> io::Result<Option<Box<dyn DocumentLease + '_>>> {
        self.scans.fetch_add(1, Ordering::Relaxed);
        self.lease(path)
    }
    fn pin_read(&self, path: &str) -> io::Result<Option<Box<dyn DocumentLease + '_>>> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        self.lease(path)
    }
}

pub(crate) fn docs(files: &[(&str, &str)]) -> model::Docs {
    files
        .iter()
        .map(|(path, raw)| {
            (
                (*path).to_owned(),
                std::sync::Arc::new(model::build((*raw).to_owned(), syntax::parse(raw))),
            )
        })
        .collect()
}
