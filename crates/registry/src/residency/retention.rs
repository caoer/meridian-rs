//! Controller-backed retained bytes for watcher baselines (merkle §6.8).
//! This capability does not choose a backing parent or activate a daemon
//! budget. Its host owns source/backing metadata and change-staging charges.

use super::Access;
use super::controller::{Controller, Member, RawPin};
use crate::raw_store::RawStore;
use model::bytes::{ByteIdentity, ByteLease, ByteRetention, RetainedBytes};
use std::{fmt, io, sync::Arc};

/// A host's retained baseline store with an explicit load-peak declaration.
/// Baseline descriptors are cold until borrowed; a serving member with the
/// same identity uses the very same controller slot and raw allocation.
pub struct Retention {
    controller: Controller,
    store: Arc<RawStore>,
    load_peak: usize,
}
impl fmt::Debug for Retention {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Retention")
            .field("load_peak", &self.load_peak)
            .finish_non_exhaustive()
    }
}
impl Retention {
    /// Use the host's shared store/controller. `load_peak` must bound every
    /// admitted load's source scratch, parse staging and flight storage, not
    /// merely its retained body. Underestimation is a read fault. Hosts needing
    /// per-version bounds can implement `ByteRetention` with their own policy.
    #[must_use]
    pub fn new(controller: Controller, store: Arc<RawStore>, load_peak: usize) -> Self {
        Self {
            controller,
            store,
            load_peak,
        }
    }
}
#[derive(Debug)]
struct Version {
    member: Member,
    load_peak: usize,
}
impl ByteRetention for Retention {
    fn retain(&self, bytes: Vec<u8>) -> io::Result<Arc<dyn RetainedBytes>> {
        let source = self.store.retain(&bytes)?;
        Ok(Arc::new(Version {
            member: self.controller.retain(Arc::new(source)),
            load_peak: self.load_peak,
        }))
    }
}
impl RetainedBytes for Version {
    fn identity(&self) -> ByteIdentity {
        self.member.identity()
    }
    fn pin(&self) -> io::Result<Box<dyn ByteLease>> {
        let pin = self
            .member
            .pin(Access::Scan, self.load_peak)
            .map_err(io::Error::other)?;
        Ok(Box::new(pin.raw()))
    }
}
impl ByteLease for RawPin {
    fn bytes(&self) -> &[u8] {
        RawPin::bytes(self)
    }
}

#[cfg(test)]
mod tests;
