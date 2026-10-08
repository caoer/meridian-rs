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
use std::time::Instant;

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

/// Which retained version a member or pin is, for a caller that records what
/// it holds ([`Member::held_blocks`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemberId(Key);

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
    /// Largest retained-body excess over a declared peak, not a measured peak.
    pub peak_shortfall: usize,
    /// Loads whose retained body exceeded their declared peak, each learned
    /// on its version's row (merkle-spec §6.4).
    pub peak_overruns: usize,
    pub members: usize,
    pub reservations: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionError {
    Busy,
    /// This member is being retired, or the serialized slot an oversized
    /// load needs waits only for another body's retirement; retry the same
    /// retained version.
    Retiring,
    /// Something that ends without this caller's help is completing: a
    /// failed load; a load of this version declared before an overrun was
    /// learned, or its reservation after its calls ended. Retry the same
    /// retained version.
    Settling,
    /// This oversized load is the one waiting member, or is kept out by
    /// paused loads owed their turn: what keeps it out ends without this
    /// caller's help. Retry the same retained version until it has.
    Waiting,
    /// Another member's oversized load waits or runs, and this new load is
    /// paused for it. Retry the same retained version; past the catalog's
    /// patience, ask again with [`Member::pin_unpaused`].
    Paused,
    Exhausted,
}

impl fmt::Display for AdmissionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Busy => "residency allowance is occupied",
            Self::Retiring => "retained member is being retired; retry the same version",
            Self::Settling => "retained load is completing; retry the same version",
            Self::Waiting => {
                "oversized load waits for the live loads to drain; retry the same version"
            }
            Self::Paused => "another member's oversized load waits; retry the same version",
            Self::Exhausted => "residency reservation identity or local revision exhausted",
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
            state.insert_row(
                key,
                Row {
                    resident,
                    descriptor: descriptor_bytes(&*source),
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
                    learned: 0,
                },
            );
        }
        let member = Member {
            controller: self.clone(),
            key,
        };
        drop(state);
        // A rejected duplicate source may have a filesystem destructor.
        drop(source);
        self.trim();
        member
    }

    #[must_use]
    pub fn report(&self) -> Report {
        lock(&self.0.ledger).report()
    }

    pub fn set_limit(&self, limit: usize) {
        let mut state = lock(&self.0.ledger);
        state.limit = limit;
        state.clear_transient_if_unused();
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
        let floor = Floor {
            controller: self.clone(),
            id,
        };
        drop(state);
        self.trim();
        Ok(floor)
    }

    /// Reserve staging work; ordinary reservations refuse if trimming cannot
    /// make room. Oversized transient service is specific to member pinning.
    ///
    /// # Errors
    /// The allowance is occupied or the lease identity is exhausted.
    pub fn reserve(&self, bytes: usize) -> Result<Reservation, AdmissionError> {
        // Only a member admission can request reselection.
        self.admit(None, bytes, 0)?.ok_or(AdmissionError::Busy)
    }

    fn admit(
        &self,
        key: Option<Key>,
        bytes: usize,
        revision: u64,
    ) -> Result<Option<Reservation>, AdmissionError> {
        self.admit_past(key, bytes, revision, false)
    }

    /// [`Self::admit`]; `unpaused` asks past another member's waiting
    /// oversized load and its hold of the serialized slot. A member's load
    /// answered anything but `Paused` is no longer owed a turn.
    fn admit_past(
        &self,
        key: Option<Key>,
        bytes: usize,
        revision: u64,
        unpaused: bool,
    ) -> Result<Option<Reservation>, AdmissionError> {
        let answer = self.admit_turn(key, bytes, revision, unpaused);
        if let Some(key) = key
            && !matches!(answer, Err(AdmissionError::Paused))
        {
            lock(&self.0.ledger).unowe(key);
        }
        answer
    }

    fn admit_turn(
        &self,
        key: Option<Key>,
        bytes: usize,
        revision: u64,
        unpaused: bool,
    ) -> Result<Option<Reservation>, AdmissionError> {
        loop {
            let mut state = lock(&self.0.ledger);
            state.clear_transient_if_unused();
            if let Some(key) = key {
                let row = &state.rows[&key];
                if row.retiring {
                    return Err(AdmissionError::Retiring);
                }
                if let Some(id) = row.wave {
                    state.permits.get_mut(&id).unwrap().references += 1;
                    return Ok(Some(Reservation {
                        controller: self.clone(),
                        id,
                    }));
                }
                // The row may have become hot (or started a call) while admit
                // was unlocked for reclamation. Never open a new wave here.
                if row.body != 0 || row.calls != 0 {
                    return Ok(None);
                }
            }
            state.permits.reserve(1);
            if key.is_some() {
                let pending = state.pending + 1;
                state.raw.reserve(pending);
                state.raw_versions.reserve(pending);
            }
            let floor = state.floor_exact();
            let fits =
                state.limit == 0 || state.total_exact() + bytes as u128 <= state.limit as u128;
            let oversized = key.is_some()
                && state.limit != 0
                && bytes as u128 > (state.limit as u128).saturating_sub(floor);
            // While another member's oversized load waits, a new load that is
            // not oversized pauses, so the live loads drain (merkle-spec §6.4).
            let now = Instant::now();
            if let Some(key) = key
                && !unpaused
                && !oversized
                && state.limit != 0
                && state.waits_for(key, now)
            {
                state.owe(key, now);
                return Err(AdmissionError::Paused);
            }
            // One member waits at a time, and loads another member's wait or
            // slot paused take their turn before the next oversized load
            // (merkle-spec §6.4): no record forms against them, and they end
            // without this caller's help.
            if let Some(key) = key
                && oversized
                && let Some(kept_out) = state.keeps_out(key, now)
            {
                return Err(kept_out);
            }
            let exclusive_available = state.transient.is_none() || state.transient == key;
            let slot_waits_for_retirement = oversized
                && exclusive_available
                && state.permits.is_empty()
                && state.detached_floor == 0;
            let may_overflow = slot_waits_for_retirement && state.retiring == 0;
            if !fits
                && !may_overflow
                && let Some(refused) = refusal(
                    &mut state,
                    key,
                    bytes,
                    oversized,
                    slot_waits_for_retirement,
                    unpaused,
                )
            {
                return Err(refused);
            }
            if fits || (may_overflow && state.reclaimable == 0) {
                let id = open_wave(&mut state, key, bytes, revision, fits)?;
                return Ok(Some(Reservation {
                    controller: self.clone(),
                    id,
                }));
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
            state.clear_transient_if_unused();
            let over = state.limit != 0 && state.total_exact() > state.limit as u128;
            let transient_ready = state.limit != 0
                && state.transient.is_some_and(|key| {
                    state.rows.get(&key).is_some_and(|row| {
                        row.body != 0 && row.calls == 0 && row.pins == 0 && !row.retiring
                    })
                });
            if state.reclaimable == 0
                || (!over && !for_admission && !transient_ready && state.forced.is_empty())
            {
                return false;
            }
            let idle =
                |row: &Row| row.body != 0 && row.calls == 0 && row.pins == 0 && !row.retiring;
            // An overrun's body first: shedding it alone restores the limit
            // (merkle-spec §6.4). Then the least recently used.
            let mut selected = state.forced.iter().find_map(|current| {
                let row = &state.rows[current];
                idle(row)
                    .then(|| row.resident.eviction_ticket())
                    .flatten()
                    .map(|ticket| (*current, row.resident.clone(), ticket))
            });
            let mut key = if selected.is_some() {
                None
            } else {
                state.first
            };
            while let Some(current) = key {
                let row = &state.rows[&current];
                key = row.next;
                let forced =
                    row.force_shed || (state.limit != 0 && state.transient == Some(current));
                if idle(row)
                    && (over || for_admission || forced)
                    && let Some(ticket) = row.resident.eviction_ticket()
                {
                    selected = Some((current, row.resident.clone(), ticket));
                    break;
                }
            }
            if let Some((key, _, _)) = &selected {
                state.update_row(*key, |row| row.retiring = true);
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
            let row = state.remove_row(key);
            if row.force_shed {
                state.unforce(key);
            }
            state.detached_floor = state.detached_floor.saturating_add(row.descriptor);
            state.detached_body = state.detached_body.saturating_add(row.body);
            state.shrink();
            row
        };
        let cleanup = Detached {
            controller: self.clone(),
            descriptor: retired.descriptor,
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
/// Open the loading wave of an admitted load: its permit, its member's row
/// and, for a load that does not fit, the serialized slot. An admitted
/// waiting member's record ends here.
fn open_wave(
    state: &mut Ledger,
    key: Option<Key>,
    bytes: usize,
    revision: u64,
    fits: bool,
) -> Result<u64, AdmissionError> {
    let id = state.next_id()?;
    state.permits.insert(
        id,
        Permit {
            bytes,
            peak: bytes,
            member: key,
            settled: key.is_none(),
            references: 1,
            revision,
            transient: if fits { None } else { key },
        },
    );
    state.reserved_bytes += bytes as u128;
    if let Some(key) = key {
        state.pending += 1;
        state.rows.get_mut(&key).unwrap().wave = Some(id);
        if state.waiting.is_some_and(|(waiting, _)| waiting == key) {
            state.waiting = None;
        }
    }
    if !fits {
        state.transient = key;
    }
    Ok(id)
}

/// Why a load that neither fits nor may overflow is refused, or `None` when
/// reclaiming cached bodies can still make room for it (merkle-spec §6.4).
fn refusal(
    state: &mut Ledger,
    key: Option<Key>,
    bytes: usize,
    oversized: bool,
    slot_waits_for_retirement: bool,
    unpaused: bool,
) -> Option<AdmissionError> {
    if let Some(key) = key
        && oversized
    {
        // Refused oversized: it becomes the one waiting member, or renews its
        // record. Another member's record or hold of the slot kept it out
        // before (`Ledger::keeps_out`).
        state.wait_oversized(key, Instant::now());
        return Some(if slot_waits_for_retirement {
            AdmissionError::Retiring
        } else {
            AdmissionError::Waiting
        });
    }
    // Exclusivity restricts only another over-budget admission. A policy
    // refusal or an impossible byte fit cannot benefit from discarding
    // somebody else's cache first.
    if state
        .total_exact()
        .saturating_sub(state.reclaim_upper_bound())
        + bytes as u128
        > state.limit as u128
    {
        // Another member's hold of the serialized slot ends without this
        // caller's help: a pause, not a refusal.
        if let Some(key) = key
            && state.transient.is_some()
            && state.transient != Some(key)
            && !unpaused
        {
            state.owe(key, Instant::now());
            return Some(AdmissionError::Paused);
        }
        return Some(AdmissionError::Busy);
    }
    None
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
        let (raw, released, forced) = state.update_row(self.key, |row| {
            row.retiring = false;
            if cold {
                let previous = (row.raw, row.body, row.force_shed);
                row.body = 0;
                row.force_shed = false;
                previous
            } else {
                (None, 0, false)
            }
        });
        if cold {
            if forced {
                state.unforce(self.key);
            }
            state.body_bytes -= released as u128;
            state.release_raw(raw);
            state.clear_transient_if_unused();
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
    descriptor: usize,
    body: usize,
    raw: Option<usize>,
}
impl Drop for Detached {
    fn drop(&mut self) {
        let mut state = lock(&self.controller.0.ledger);
        state.detached_floor -= self.descriptor;
        state.detached_body -= self.body;
        state.body_bytes -= self.body as u128;
        if self.body != 0 {
            state.release_raw(self.raw);
        } else if let Some(id) = self.raw {
            state.prune_raw(id);
        }
        state.clear_transient_if_unused();
        state.shrink();
    }
}

#[cfg(test)]
mod tests;
