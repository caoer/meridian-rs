//! One retained version's body lifecycle (merkle §6.8).
//!
//! No workspace lookup, global budget policy or daemon activation lives here.
//! The integrating controller must reserve load peaks before calling `pin`.
//! Slot locks cover transitions only; source I/O, parsing and flight waits
//! happen after releasing them. Lock order is slot, then flight outcome.

use fs::hydration::{self, HydrationError};
use model::bytes::{ByteIdentity, ByteSource, SharedBytes, WeakBytes};
use std::fmt;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Weak};

pub mod controller;
pub mod retention;

/// An immutable version descriptor shared across generations and operations.
#[derive(Debug, Clone)]
pub struct Resident(Arc<Inner>);

#[derive(Debug)]
struct Inner {
    identity: ByteIdentity,
    source: Arc<dyn ByteSource>,
    state: Mutex<State>,
}

#[derive(Debug)]
struct State {
    body: StateBody,
    revision: u64,
    last_raw: Option<WeakBytes>,
}

#[derive(Debug)]
enum StateBody {
    Cold,
    Loading(Arc<Flight>),
    Hot(Arc<Body>),
}

#[derive(Debug)]
struct Body {
    bytes: SharedBytes,
    document: Option<model::Document>,
    tree_bytes: usize,
}

/// Reads advance this slot's eviction revision; scans only hold a body pin.
/// Global LRU ordering belongs to the caller, not this per-slot counter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Read,
    Scan,
}

/// A lease over the body and descriptor. Cloning a pin does not copy either.
#[derive(Debug, Clone)]
pub struct BodyPin {
    body: Arc<Body>,
    _owner: Arc<Inner>,
}

/// A candidate tied to a slot and revision, never an unconditional eviction.
#[derive(Debug)]
pub struct EvictionTicket {
    owner: Weak<Inner>,
    revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Cold,
    Loading,
    Hot,
}

/// A bounded, per-slot observation, not a global or transient budget census.
/// Raw counts include escaped raw owners while this descriptor lives. Tree
/// counts exclude the body owner, which is reported separately. Source,
/// descriptor, flight/error, pin/container and staging allocations are outside
/// these fields; a controller must account for them independently.
#[derive(Debug, Clone, Copy)]
pub struct Snapshot {
    pub phase: Phase,
    pub revision: u64,
    pub waiters: usize,
    pub raw_bytes: usize,
    pub tree_bytes: usize,
    pub body_bytes: usize,
}

/// A whole loading wave shares one error. Required backing faults never mean
/// absent membership; invalid UTF-8 is instead a successfully loaded raw body.
#[derive(Debug)]
pub enum LoadError {
    Hydration(HydrationError),
    Abandoned,
    RevisionExhausted,
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Hydration(error) => error.fmt(f),
            Self::Abandoned => f.write_str("retained-version loader unwound before completion"),
            Self::RevisionExhausted => f.write_str("retained-version eviction revision exhausted"),
        }
    }
}

impl std::error::Error for LoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Hydration(error) => Some(error),
            Self::Abandoned | Self::RevisionExhausted => None,
        }
    }
}

type LoadResult = Result<Arc<Body>, Arc<LoadError>>;

#[derive(Debug, Default)]
struct Flight {
    state: Mutex<FlightState>,
    ready: Condvar,
}

#[derive(Debug, Default)]
struct FlightState {
    outcome: Option<LoadResult>,
    waiters: usize,
}

// Only owned, callback-free state transitions run under these locks. Recover
// poison so unwinding cleanup still completes its wave and wakes every waiter.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Flight {
    fn wait(&self) -> LoadResult {
        let mut state = lock(&self.state);
        while state.outcome.is_none() {
            state = self
                .ready
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        state.waiters -= 1;
        state.outcome.as_ref().expect("completed flight").clone()
    }
}

// An owner is installed before invoking any fallible/unwinding source. Its
// destructor settles the same wave even when the leader's stack unwinds.
struct LoadOwner {
    owner: Arc<Inner>,
    flight: Arc<Flight>,
    complete: bool,
}

impl LoadOwner {
    fn finish(&mut self, outcome: LoadResult) {
        let mut state = lock(&self.owner.state);
        let mut flight = lock(&self.flight.state);
        match &outcome {
            Ok(body) => {
                state.last_raw = Some(body.bytes.downgrade());
                state.body = StateBody::Hot(body.clone());
            }
            Err(_) => state.body = StateBody::Cold,
        }
        flight.outcome = Some(outcome);
        self.complete = true;
        self.flight.ready.notify_all();
    }
}

impl Drop for LoadOwner {
    fn drop(&mut self) {
        if !self.complete {
            self.finish(Err(Arc::new(LoadError::Abandoned)));
        }
    }
}

enum Action {
    Ready(Arc<Body>),
    Wait(Waiting),
    Load(LoadOwner, Option<SharedBytes>),
}

// Preparing chooses a flight without I/O or waiting. A controller can bind
// that choice to admission while holding its ledger, then resolve outside it.
struct PreparedPin {
    resident: Resident,
    action: Action,
}

struct Waiting {
    flight: Arc<Flight>,
    active: bool,
}
impl Waiting {
    fn wait(mut self) -> LoadResult {
        let result = self.flight.wait();
        self.active = false;
        result
    }
}
impl Drop for Waiting {
    fn drop(&mut self) {
        if self.active {
            lock(&self.flight.state).waiters -= 1;
        }
    }
}

impl PreparedPin {
    fn resolve(self) -> Result<BodyPin, Arc<LoadError>> {
        let body = match self.action {
            Action::Ready(body) => body,
            Action::Wait(waiting) => waiting.wait()?,
            Action::Load(mut owner, raw) => {
                let result = self.resident.load(raw).map(Arc::new).map_err(Arc::new);
                owner.finish(result.clone());
                result?
            }
        };
        Ok(BodyPin {
            body,
            _owner: self.resident.0.clone(),
        })
    }
}

impl Resident {
    /// Capture a fixed identity. The source must retain this version's backing
    /// for its lifetime, independently of any current workspace path.
    #[must_use]
    pub fn new(source: Arc<dyn ByteSource>) -> Self {
        Self(Arc::new(Inner {
            identity: source.identity(),
            source,
            state: Mutex::new(State {
                body: StateBody::Cold,
                revision: 0,
                last_raw: None,
            }),
        }))
    }

    #[must_use]
    pub fn identity(&self) -> ByteIdentity {
        self.0.identity
    }

    /// Load or join a single flight, then retain the selected body and version.
    /// This blocks the calling thread, without holding the slot lock.
    ///
    /// # Errors
    /// Required backing/identity faults, an unwinding load owner, or revision
    /// exhaustion. A later call may retry a failed wave using the same version.
    ///
    /// # Panics
    /// A source panic propagates to its loading caller; the owner's unwind
    /// guard still completes all waiting callers with `LoadError::Abandoned`.
    pub fn pin(&self, access: Access) -> Result<BodyPin, Arc<LoadError>> {
        self.prepare(access)?.resolve()
    }

    fn prepare(&self, access: Access) -> Result<PreparedPin, Arc<LoadError>> {
        let action = {
            let mut state = lock(&self.0.state);
            if access == Access::Read || matches!(state.body, StateBody::Cold) {
                state.revision = state
                    .revision
                    .checked_add(1)
                    .ok_or_else(|| Arc::new(LoadError::RevisionExhausted))?;
            }
            match &state.body {
                StateBody::Hot(body) => Action::Ready(body.clone()),
                StateBody::Loading(flight) => {
                    lock(&flight.state).waiters += 1;
                    Action::Wait(Waiting {
                        flight: flight.clone(),
                        active: true,
                    })
                }
                StateBody::Cold => {
                    let raw = state.last_raw.as_ref().and_then(WeakBytes::upgrade);
                    if raw.is_none() {
                        state.last_raw = None;
                    }
                    let flight = Arc::new(Flight::default());
                    let owner = LoadOwner {
                        owner: self.0.clone(),
                        flight: flight.clone(),
                        complete: false,
                    };
                    state.body = StateBody::Loading(flight);
                    Action::Load(owner, raw)
                }
            }
        };
        Ok(PreparedPin {
            resident: self.clone(),
            action,
        })
    }

    fn load(&self, raw: Option<SharedBytes>) -> Result<Body, LoadError> {
        let bytes = raw
            .map_or_else(|| self.0.source.read(), Ok)
            .map_err(|error| LoadError::Hydration(HydrationError::Source(error)))?;
        // Hydration verifies against the descriptor's captured identity, even
        // if a source violates its promise to keep identity() immutable.
        let selected = Selected {
            identity: self.0.identity,
            bytes,
        };
        let document = match hydration::document(&selected) {
            Ok(document) => Some(document),
            Err(HydrationError::InvalidUtf8) => None,
            Err(error) => return Err(LoadError::Hydration(error)),
        };
        let tree_bytes = document
            .as_ref()
            .map_or(0, |doc| model::allocation::document(doc).tree);
        Ok(Body {
            bytes: selected.bytes,
            document,
            tree_bytes,
        })
    }

    /// Select an unpinned hot body without preventing later reads.
    #[must_use]
    pub fn eviction_ticket(&self) -> Option<EvictionTicket> {
        let state = lock(&self.0.state);
        match &state.body {
            StateBody::Hot(body) if Arc::strong_count(body) == 1 => Some(EvictionTicket {
                owner: Arc::downgrade(&self.0),
                revision: state.revision,
            }),
            _ => None,
        }
    }

    /// Recheck the exact candidate and drop its body outside the slot lock.
    /// Raw clones may remain alive; `snapshot` continues to charge them.
    #[must_use]
    pub fn try_shed(&self, ticket: &EvictionTicket) -> bool {
        self.shed_with(ticket, || {})
    }

    // The hook permits a deterministic controller test of the cold-before-drop
    // window; production instantiates only the empty closure above.
    fn shed_with(&self, ticket: &EvictionTicket, before_drop: impl FnOnce()) -> bool {
        if !std::ptr::eq(ticket.owner.as_ptr(), Arc::as_ptr(&self.0)) {
            return false;
        }
        let retired = {
            let mut state = lock(&self.0.state);
            if state.revision != ticket.revision {
                return false;
            }
            match &state.body {
                StateBody::Hot(body) if Arc::strong_count(body) == 1 => (),
                _ => return false,
            }
            std::mem::replace(&mut state.body, StateBody::Cold)
        };
        before_drop();
        drop(retired);
        true
    }

    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        let mut state = lock(&self.0.state);
        let raw = state.last_raw.as_ref().and_then(WeakBytes::upgrade);
        if raw.is_none() {
            state.last_raw = None;
        }
        let (phase, waiters, tree_bytes, body_bytes) = match &state.body {
            StateBody::Cold => (Phase::Cold, 0, 0, 0),
            StateBody::Loading(flight) => (Phase::Loading, lock(&flight.state).waiters, 0, 0),
            StateBody::Hot(body) => (
                Phase::Hot,
                0,
                body.tree_bytes,
                model::allocation::arc_bytes::<Body>(),
            ),
        };
        Snapshot {
            phase,
            revision: state.revision,
            waiters,
            raw_bytes: raw.as_ref().map_or(0, SharedBytes::heap_bytes),
            tree_bytes,
            body_bytes,
        }
    }
}

impl BodyPin {
    #[must_use]
    pub fn bytes(&self) -> &SharedBytes {
        &self.body.bytes
    }

    /// Borrow the parsed body under this pin.
    ///
    /// # Errors
    /// Verified invalid UTF-8 has exact bytes but no parsed document.
    pub fn document(&self) -> Result<&model::Document, HydrationError> {
        self.body
            .document
            .as_ref()
            .ok_or(HydrationError::InvalidUtf8)
    }
}

#[derive(Debug)]
struct Selected {
    identity: ByteIdentity,
    bytes: SharedBytes,
}

impl ByteSource for Selected {
    fn identity(&self) -> ByteIdentity {
        self.identity
    }

    fn read(&self) -> std::io::Result<SharedBytes> {
        Ok(self.bytes.clone())
    }
}

#[cfg(test)]
mod tests;
