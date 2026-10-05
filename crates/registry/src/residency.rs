//! Per-version body lifecycle; budget admission and daemon wiring are separate.

use model::bytes::{ByteIdentity, ByteSource, SharedBytes};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct Resident(Arc<dyn ByteSource>);

#[derive(Debug, Clone, Copy)]
pub enum Access {
    Read,
    Scan,
}

#[derive(Debug)]
pub struct BodyPin;

#[derive(Debug)]
pub struct EvictionTicket;

impl Resident {
    pub fn new(source: Arc<dyn ByteSource>) -> Self {
        Self(source)
    }

    pub fn identity(&self) -> ByteIdentity {
        self.0.identity()
    }

    pub fn pin(&self, _access: Access) -> Result<BodyPin, &'static str> {
        Err("resident lifecycle not implemented")
    }

    pub fn eviction_ticket(&self) -> Option<EvictionTicket> {
        None
    }

    pub fn try_shed(&self, _ticket: &EvictionTicket) -> bool {
        false
    }
}

impl BodyPin {
    pub fn bytes(&self) -> &SharedBytes {
        unreachable!()
    }

    pub fn document(&self) -> Result<&model::Document, &'static str> {
        Err("resident lifecycle not implemented")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raw_store::RawStore;

    #[test]
    fn shed_rehydrates_the_selected_version_after_source_and_store_drop() {
        let parent = tempfile::tempdir().unwrap();
        let path = parent.path().join("member.md");
        let raw = b"# Version A\n\n[[retained]]\n";
        std::fs::write(&path, raw).unwrap();
        let store = RawStore::new(parent.path()).unwrap();
        let handle = store.retain(raw).unwrap();
        let resident = Resident::new(Arc::new(handle));
        let identity = resident.identity();
        let pin = resident.pin(Access::Read).expect("load retained version");
        let expected = pin.document().unwrap().root.clone();
        assert_eq!(pin.bytes().as_bytes(), raw);
        assert!(resident.eviction_ticket().is_none());
        drop(pin);
        let ticket = resident
            .eviction_ticket()
            .expect("unpinned body is evictable");
        assert!(resident.try_shed(&ticket));
        std::fs::write(&path, b"# Version B\n").unwrap();
        std::fs::remove_file(&path).unwrap();
        drop(store);
        let pin = resident
            .pin(Access::Read)
            .expect("rehydrate from retained backing");
        assert_eq!(resident.identity(), identity);
        assert_eq!(pin.bytes().as_bytes(), raw);
        assert_eq!(pin.document().unwrap().root, expected);
    }
}
