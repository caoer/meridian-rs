//! Controller-backed retained bytes for watcher baselines (merkle §6.8).
//! This capability does not choose a backing parent or activate a daemon
//! budget. Its host owns source/backing metadata and change-staging charges.

use super::Access;
use super::controller::{Controller, Member, RawPin};
use crate::raw_store::RawStore;
use model::bytes::{ByteIdentity, ByteLease, ByteRetention, RetainedBytes};
use std::{fmt, io, sync::Arc};

/// A host's retained baseline store. Each version loads under the length
/// rule ([`super::load_peak`]), as a serving slot of the same bytes does.
/// Baseline descriptors are cold until borrowed; a serving member with the
/// same identity uses the very same controller slot and raw allocation.
pub struct Retention {
    controller: Controller,
    store: Arc<RawStore>,
}
impl fmt::Debug for Retention {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Retention").finish_non_exhaustive()
    }
}
impl Retention {
    /// Use the host's shared store/controller.
    #[must_use]
    pub fn new(controller: Controller, store: Arc<RawStore>) -> Self {
        Self { controller, store }
    }
}
#[derive(Debug)]
struct Version {
    member: Member,
}
impl ByteRetention for Retention {
    fn retain(&self, bytes: Vec<u8>) -> io::Result<Arc<dyn RetainedBytes>> {
        let source = self.store.retain(&bytes)?;
        Ok(Arc::new(Version {
            member: self.controller.retain(Arc::new(source)),
        }))
    }
}
impl RetainedBytes for Version {
    fn identity(&self) -> ByteIdentity {
        self.member.identity()
    }
    fn pin(&self) -> io::Result<Box<dyn ByteLease>> {
        // Through the catalog's admission: its refusals are transient.
        let pin = super::catalog::pin(
            &self.member,
            Access::Scan,
            super::load_peak(self.member.identity().len),
        )?;
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
