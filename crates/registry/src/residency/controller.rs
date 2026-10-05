//! Global admission and member LRU (merkle §6.8).

use super::{Access, BodyPin, LoadError, Phase, Resident};
use model::bytes::ByteSource;
use std::sync::Arc;

pub struct Controller;

pub struct Member(Resident);

impl Controller {
    pub fn new(_limit: usize) -> Self {
        Self
    }

    pub fn retain(&self, source: Arc<dyn ByteSource>) -> Member {
        Member(Resident::new(source))
    }

    pub fn set_limit(&self, _limit: usize) {}
}

impl Member {
    pub fn pin(&self, access: Access, _peak: usize) -> Result<BodyPin, Arc<LoadError>> {
        self.0.pin(access)
    }

    pub fn phase(&self) -> Phase {
        self.0.snapshot().phase
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raw_store::RawStore;

    #[test]
    fn global_budget_sheds_unpinned_members_and_retries_after_last_pin() {
        let parent = tempfile::tempdir().unwrap();
        let store = RawStore::new(parent.path()).unwrap();
        let budget = Controller::new(0);
        let a = budget.retain(Arc::new(store.retain(b"# A\n\nold\n").unwrap()));
        let b = budget.retain(Arc::new(store.retain(b"# B\n\nnew\n").unwrap()));
        drop(a.pin(Access::Read, 8192).unwrap());
        let pinned = b.pin(Access::Read, 8192).unwrap();
        budget.set_limit(1);
        assert_eq!(a.phase(), Phase::Cold, "global limit did not shed member A");
        assert_eq!(b.phase(), Phase::Hot, "operation pin must protect member B");
        drop(pinned);
        assert_eq!(
            b.phase(),
            Phase::Cold,
            "last pin did not enforce the budget"
        );
        let old = a.pin(Access::Read, 8192).unwrap();
        assert_eq!(old.document().unwrap().raw.as_str(), "# A\n\nold\n");
        drop(old);
        assert_eq!(a.phase(), Phase::Cold, "oversized transient became cached");
    }
}
