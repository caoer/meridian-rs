//! Controller-backed immutable document generation for fallible consumers.
//! Capture bridges an existing eager generation. Streaming builders, measured
//! metadata/staging floors and daemon publication remain the host's work.

use super::{
    Access,
    controller::{Controller, Member, Pin},
};
use crate::raw_store::RawStore;
use model::{
    CorpusIndex, Docs, NodeRev,
    catalog::{DocumentCatalog, DocumentLease},
};
use std::{collections::BTreeMap, io, sync::Arc};

#[derive(Debug)]
struct Entry {
    revision: NodeRev,
    member: Member,
}

/// Complete generation membership with sheddable bodies and exact backing.
/// Holding the catalog keeps descriptors alive, never every parsed document.
/// Pins are scan accesses: they protect bodies without promoting LRU recency.
#[derive(Debug)]
pub struct Catalog {
    entries: BTreeMap<String, Entry>,
    index: CorpusIndex,
    load_peak: usize,
}
impl Catalog {
    /// Capture an immutable eager generation, with the host's conservative
    /// per-load peak (source scratch, parsing and flight storage included).
    /// The host accounts for capture staging, index and backing metadata.
    ///
    /// # Errors
    /// Required backing cannot be retained. No partial catalog is published;
    /// the caller's original generation remains untouched.
    pub fn capture(
        docs: &Docs,
        controller: &Controller,
        store: &RawStore,
        load_peak: usize,
    ) -> io::Result<Self> {
        let mut entries = BTreeMap::new();
        let mut index = CorpusIndex::new();
        for (path, doc) in docs {
            let source = store.retain(doc.raw.as_bytes())?;
            entries.insert(
                path.clone(),
                Entry {
                    revision: doc.root.node_rev.clone(),
                    member: controller.retain(Arc::new(source)),
                },
            );
            index.insert(path, doc);
        }
        Ok(Self {
            entries,
            index,
            load_peak,
        })
    }
}
impl DocumentCatalog for Catalog {
    fn revisions(&self) -> Box<dyn Iterator<Item = (&str, &NodeRev)> + '_> {
        Box::new(
            self.entries
                .iter()
                .map(|(path, entry)| (path.as_str(), &entry.revision)),
        )
    }
    fn revision(&self, path: &str) -> Option<&NodeRev> {
        self.entries.get(path).map(|entry| &entry.revision)
    }
    fn index(&self) -> &CorpusIndex {
        &self.index
    }
    fn pin(&self, path: &str) -> io::Result<Option<Box<dyn DocumentLease + '_>>> {
        self.entries
            .get(path)
            .map(|entry| {
                entry
                    .member
                    .pin(Access::Scan, self.load_peak)
                    .map(|pin| Box::new(pin) as Box<dyn DocumentLease>)
                    .map_err(io::Error::other)
            })
            .transpose()
    }
}
impl DocumentLease for Pin {
    fn document(&self) -> io::Result<&model::Document> {
        Pin::document(self).map_err(io::Error::other)
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod read_tests;
