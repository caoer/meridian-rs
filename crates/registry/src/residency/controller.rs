//! Process-wide admission and member LRU; daemon ownership wiring is separate.
//!
//! One controller must own all roots. Callers supply measured non-body floor
//! charges and conservative load-peak reservations. Retained raw clones must
//! have a `RawPin` (or keep their parent `Pin` alive). These contracts must be
//! established before activating the controller in a serving host.
//!
//! Lock order: ledger, then the local slot/flight. No ledger lock spans source
//! callbacks, parsing, waiting or destruction of a body/backing descriptor.

use super::{Access, BodyPin, LoadError, Phase, Resident, lock};
use model::bytes::{ByteIdentity, ByteSource, SharedBytes};
use std::fmt;
use std::sync::{Arc, Mutex};

mod ledger;
use ledger::{Ledger, Permit, Row, descriptor_bytes};
mod leases;

// Content identity is enough: model documents have no member-path-dependent
// parse state. Roots/generations retain their own membership/name rows.
type Key = ([u8; 32], u64);

#[derive(Debug)]
struct Control {
    ledger: Mutex<Ledger>,
}

#[derive(Clone)]
pub struct Controller(Arc<Control>);

impl fmt::Debug for Controller {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // No ledger lock, recursion through source callbacks or corpus dump
        // when a lease/error appears in a diagnostic.
        f.debug_struct("Controller").finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub struct Member {
    controller: Controller,
    key: Key,
}

#[derive(Debug)]
pub struct Pin {
    body: Option<BodyPin>,
    member: Member,
}

/// A raw-only owner that may outlive its parsed body and member descriptor.
#[derive(Debug)]
pub struct RawPin {
    bytes: Option<SharedBytes>,
    controller: Controller,
    id: usize,
}

/// Explicit staging charge; clones share one ledger reservation.
#[derive(Debug)]
pub struct Reservation {
    controller: Controller,
    id: u64,
}

/// A measured, non-sheddable owner's charge. Destroy that owner before dropping
/// this lease; update it when its allocated capacity changes.
#[derive(Debug)]
pub struct Floor {
    controller: Controller,
    id: u64,
}

/// Allocated corpus bytes and declared peak reservations are separate. This
/// does not measure allocator overhead, thread stacks, native SQL or RSS.
#[derive(Debug, Clone, Copy)]
pub struct Report {
    pub limit: usize,
    pub floor: usize,
    pub raw: usize,
    pub bodies: usize,
    pub reserved: usize,
    pub allocated: usize,
    pub total: usize,
    /// Body/tree bytes protected by operation pins; shared raw is in `raw`.
    pub pinned: usize,
    /// Body/descriptor charges held through destruction; shared raw is in `raw`.
    pub retiring: usize,
    pub overage: usize,
    pub floor_overage: usize,
    pub transient: bool,
    /// Largest observed underestimate of a caller's declared load peak.
    pub peak_shortfall: usize,
    pub members: usize,
    pub reservations: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionError {
    Busy,
    Exhausted,
}

impl fmt::Display for AdmissionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Busy => "residency allowance is occupied by pins, loads or retirement",
            Self::Exhausted => "residency reservation identity exhausted",
        })
    }
}
impl std::error::Error for AdmissionError {}

#[derive(Debug)]
pub enum Error {
    Admission(AdmissionError),
    Load(Failure),
    PeakExceeded { reserved: usize, observed: usize },
}

/// Keep the failed wave's reservation until its last returned error is gone.
/// Field order drops the error (including its source payload) before its charge.
#[derive(Debug)]
pub struct Failure {
    source: Arc<LoadError>,
    _reservation: Option<Reservation>,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Admission(error) => error.fmt(f),
            Self::Load(failure) => failure.source.fmt(f),
            Self::PeakExceeded { reserved, observed } => write!(
                f,
                "residency load exceeded its declared peak: reserved {reserved}, observed {observed}"
            ),
        }
    }
}
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Admission(error) => Some(error),
            Self::Load(failure) => Some(failure.source.as_ref()),
            Self::PeakExceeded { .. } => None,
        }
    }
}
impl Failure {
    #[must_use]
    pub fn error(&self) -> &LoadError {
        &self.source
    }
}

impl Controller {
    #[must_use]
    pub fn new(limit: usize) -> Self {
        Self(Arc::new(Control {
            ledger: Mutex::new(Ledger::new(limit)),
        }))
    }

    /// Share a retained version across roots/generations. The source's own
    /// allocation and backing metadata belong to separately declared floors.
    #[must_use]
    pub fn retain(&self, source: Arc<dyn ByteSource>) -> Member {
        let identity = source.identity(); // user callback: outside ledger lock
        let key = (identity.digest, identity.len);
        let mut state = lock(&self.0.ledger);
        if let Some(row) = state.rows.get_mut(&key) {
            row.references += 1;
        } else {
            // Capture the already obtained identity; no second source callback
            // may run under the ledger lock.
            let resident = Resident(Arc::new(super::Inner {
                identity,
                source: source.clone(),
                state: Mutex::new(super::State {
                    body: super::StateBody::Cold,
                    revision: 0,
                    last_raw: None,
                }),
            }));
            state.rows.insert(
                key,
                Row {
                    resident,
                    references: 1,
                    calls: 0,
                    pins: 0,
                    body: 0,
                    raw: None,
                    previous: None,
                    next: None,
                    listed: false,
                    retiring: false,
                    force_shed: false,
                    wave: None,
                },
            );
        }
        drop(state);
        // A rejected duplicate source may have a filesystem destructor.
        drop(source);
        self.trim();
        Member {
            controller: self.clone(),
            key,
        }
    }

    #[must_use]
    pub fn report(&self) -> Report {
        lock(&self.0.ledger).report()
    }

    pub fn set_limit(&self, limit: usize) {
        let mut state = lock(&self.0.ledger);
        state.limit = limit;
        if limit == 0 {
            state.transient = None;
        }
        drop(state);
        self.trim();
    }

    /// Register a measured metadata owner; a floor may exceed the limit and is
    /// reported as such, never hidden by evicting its membership.
    ///
    /// # Errors
    /// The monotonically allocated lease identity is exhausted.
    pub fn charge_floor(&self, bytes: usize) -> Result<Floor, AdmissionError> {
        let mut state = lock(&self.0.ledger);
        let id = state.next_id()?;
        state.floors.insert(id, bytes);
        state.external_floor += bytes as u128;
        drop(state);
        self.trim();
        Ok(Floor {
            controller: self.clone(),
            id,
        })
    }

    /// Reserve staging work; ordinary reservations refuse if trimming cannot
    /// make room. Oversized transient service is specific to member pinning.
    ///
    /// # Errors
    /// The allowance is occupied or the lease identity is exhausted.
    pub fn reserve(&self, bytes: usize) -> Result<Reservation, AdmissionError> {
        self.admit(None, bytes, 0)
    }

    fn admit(
        &self,
        key: Option<Key>,
        bytes: usize,
        revision: u64,
    ) -> Result<Reservation, AdmissionError> {
        loop {
            let mut state = lock(&self.0.ledger);
            state.permits.reserve(1);
            let floor = state.floor_exact();
            let fits =
                state.limit == 0 || state.total_exact() + bytes as u128 <= state.limit as u128;
            let oversized = key.is_some()
                && state.limit != 0
                && bytes as u128 > (state.limit as u128).saturating_sub(floor);
            let reclaimable = state
                .rows
                .values()
                .any(|row| row.body != 0 && row.calls == 0 && row.pins == 0 && !row.retiring);
            let exclusive_available = state.transient.is_none();
            if (fits && exclusive_available)
                || (oversized
                    && exclusive_available
                    && !reclaimable
                    && state.permits.is_empty()
                    && state.detached_floor == 0
                    && !state.rows.values().any(|row| row.retiring))
            {
                let id = state.next_id()?;
                state.permits.insert(
                    id,
                    Permit {
                        bytes,
                        references: 1,
                        revision,
                        transient: if oversized { key } else { None },
                        activated: false,
                    },
                );
                state.reserved_bytes += bytes as u128;
                if oversized {
                    state.transient = key;
                }
                return Ok(Reservation {
                    controller: self.clone(),
                    id,
                });
            }
            drop(state);
            if !self.shed_one(true) {
                return Err(AdmissionError::Busy);
            }
        }
    }

    /// Trim at admission, pin release and as a periodic backstop. The census
    /// stays charged through each retirement, even when its local slot is cold.
    pub fn trim(&self) {
        while self.shed_one(false) {}
    }

    fn shed_one(&self, for_admission: bool) -> bool {
        self.shed_one_with(for_admission, Resident::try_shed)
    }

    fn shed_one_with(
        &self,
        for_admission: bool,
        shed: impl FnOnce(&Resident, &super::EvictionTicket) -> bool,
    ) -> bool {
        let selected = {
            let mut state = lock(&self.0.ledger);
            let over = state.limit != 0 && state.total_exact() > state.limit as u128;
            let mut key = state.first;
            let mut selected = None;
            while let Some(current) = key {
                let row = &state.rows[&current];
                key = row.next;
                let forced = row.force_shed || state.transient == Some(current);
                if row.body != 0
                    && row.calls == 0
                    && row.pins == 0
                    && !row.retiring
                    && (over || for_admission || forced)
                    && let Some(ticket) = row.resident.eviction_ticket()
                {
                    selected = Some((current, row.resident.clone(), ticket));
                    break;
                }
            }
            if let Some((key, _, _)) = &selected {
                state.rows.get_mut(key).unwrap().retiring = true;
                state.unlink(*key);
            }
            selected
        };
        let Some((key, resident, ticket)) = selected else {
            return false;
        };
        let mut retirement = Retirement {
            controller: self.clone(),
            key,
            resident: Some(resident),
        };
        let freed = shed(retirement.resident.as_ref().unwrap(), &ticket);
        retirement.settle();
        freed
    }

    fn collect_unused(&self, key: Key) {
        let retired = {
            let mut state = lock(&self.0.ledger);
            let eligible = state
                .rows
                .get(&key)
                .is_some_and(|row| row.references == 0 && row.calls == 0 && !row.retiring);
            if !eligible {
                return;
            }
            state.unlink(key);
            let row = state.rows.remove(&key).unwrap();
            state.detached_floor = state.detached_floor.saturating_add(descriptor_bytes());
            state.detached_body = state.detached_body.saturating_add(row.body);
            if state.transient == Some(key) {
                state.transient = None;
            }
            row
        };
        let cleanup = Detached {
            controller: self.clone(),
            body: retired.body,
            raw: retired.raw,
        };
        drop(retired); // source/backing destruction may perform I/O or unwind
        drop(cleanup);
    }
}

// The old body stays in the ledger until the entire shedding call returns
// or unwinds. In particular, observing local Phase::Cold is not enough.
struct Retirement {
    controller: Controller,
    key: Key,
    resident: Option<Resident>,
}
impl Retirement {
    fn settle(&mut self) {
        let resident = self.resident.take().unwrap();
        let cold = resident.snapshot().phase == Phase::Cold;
        if cold {
            lock(&resident.0.state).last_raw = None;
        }
        drop(resident);
        let mut state = lock(&self.controller.0.ledger);
        let row = state.rows.get_mut(&self.key).unwrap();
        row.retiring = false;
        if cold {
            let (raw, released) = (row.raw, row.body);
            row.body = 0;
            row.force_shed = false;
            state.body_bytes -= released as u128;
            state.release_raw(raw);
            if state.transient == Some(self.key) {
                state.transient = None;
            }
        } else {
            state.link(self.key, Access::Scan);
        }
        drop(state);
        self.controller.collect_unused(self.key);
    }
}
impl Drop for Retirement {
    fn drop(&mut self) {
        if self.resident.is_some() {
            self.settle();
        }
    }
}

// This guard runs after the descriptor's complete drop, including unwind.
struct Detached {
    controller: Controller,
    body: usize,
    raw: Option<usize>,
}
impl Drop for Detached {
    fn drop(&mut self) {
        let mut state = lock(&self.controller.0.ledger);
        state.detached_floor -= descriptor_bytes();
        state.detached_body -= self.body;
        state.body_bytes -= self.body as u128;
        if self.body != 0 {
            state.release_raw(self.raw);
        } else if let Some(id) = self.raw {
            state.prune_raw(id);
        }
    }
}

#[cfg(test)]
mod tests;
