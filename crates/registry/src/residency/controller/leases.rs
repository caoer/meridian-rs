use super::{
    Access, AdmissionError, BodyPin, ByteIdentity, Controller, Error, Failure, Floor, Key,
    LoadError, Member, Phase, Pin, RawPin, Reservation, Resident, SharedBytes, lock,
};

use std::sync::Arc;

pub(super) struct Call {
    controller: Controller,
    key: Key,
    resident: Resident,
    prepared: Option<Result<super::super::PreparedPin, Arc<LoadError>>>,
    reservation: Option<Reservation>,
    finished: bool,
    // Dropped last, after local actions, resident clones and reservations.
    _member: Member,
}

impl Member {
    #[must_use]
    pub fn identity(&self) -> ByteIdentity {
        ByteIdentity {
            digest: self.key.0,
            len: self.key.1,
        }
    }

    #[must_use]
    pub fn phase(&self) -> Phase {
        let state = lock(&self.controller.0.ledger);
        local_status(&state.rows[&self.key].resident).0
    }

    /// Pin this exact version. `peak` bounds source bytes, parser staging and
    /// flight/error storage for one load, not merely the retained final tree.
    /// Shared-flight callers share one reservation. No admission wait is used.
    ///
    /// # Errors
    /// Occupied allowance, a retryable retirement/failed-flight completion,
    /// backing/identity failure, exhausted identifiers, or an observed body
    /// larger than the declared peak. Underestimates are
    /// reported and shed; they are not evidence of a bounded load peak.
    ///
    /// # Panics
    /// Source panics propagate; the call guard retains the reservation through
    /// local unwind completion and restores the controller's call bookkeeping.
    pub fn pin(&self, access: Access, peak: usize) -> Result<Pin, Error> {
        let mut call = self.begin(access, peak).map_err(Error::Admission)?;
        match call.resolve() {
            Ok(body) => {
                let snapshot = call.resident.snapshot();
                let observed = snapshot
                    .raw_bytes
                    .saturating_add(snapshot.tree_bytes)
                    .saturating_add(snapshot.body_bytes);
                let reserved = call.reservation.as_ref().map(Reservation::peak);
                if let Some(reserved) = reserved
                    && observed > reserved
                {
                    {
                        let mut state = lock(&self.controller.0.ledger);
                        state.peak_shortfall = state.peak_shortfall.max(observed - reserved);
                        if !state.rows[&self.key].force_shed {
                            state.rows.get_mut(&self.key).unwrap().force_shed = true;
                            state.forced += 1;
                        }
                    }
                    // Record the real body before releasing the local pin.
                    call.finish(false);
                    drop(body);
                    self.controller.trim();
                    return Err(Error::PeakExceeded { reserved, observed });
                }
                let member = self.clone();
                call.finish(true);
                Ok(Pin {
                    body: Some(body),
                    member,
                })
            }
            Err(source) => {
                let failure = Failure {
                    source,
                    _reservation: call.reservation.clone(),
                };
                call.finish(false);
                Err(Error::Load(failure))
            }
        }
    }

    pub(super) fn begin(&self, access: Access, peak: usize) -> Result<Call, AdmissionError> {
        let mut spare: Option<Reservation> = None;
        loop {
            let mut state = lock(&self.controller.0.ledger);
            let row = &state.rows[&self.key];
            if row.retiring {
                return Err(AdmissionError::Retiring);
            }
            let (phase, revision) = local_status(&row.resident);
            if revision == u64::MAX && (access == Access::Read || phase == Phase::Cold) {
                return Err(AdmissionError::Exhausted);
            }
            if let Some(reservation) = &spare
                && phase == Phase::Cold
                && row.wave != Some(reservation.id)
                && revision != state.permits[&reservation.id].revision
            {
                drop(state);
                drop(spare.take());
                continue;
            }
            let needs_wave = row.body == 0 && row.calls == 0;
            if needs_wave && spare.is_none() {
                drop(state);
                spare = self.controller.admit(Some(self.key), peak, revision)?;
                continue;
            }
            // A completed failed local flight may be cold before its owner has
            // returned through this wrapper. Do not start a second flight on
            // that first wave's still-live reservation.
            if row.calls != 0
                && let Some(id) = row.wave
                && phase == Phase::Cold
                && revision != state.permits[&id].revision
            {
                return Err(AdmissionError::Settling);
            }
            let resident = row.resident.clone();
            let existing_wave = row.wave;
            let raw = state
                .raw_versions
                .get(&self.key)
                .and_then(|(id, _)| state.raw.get(id))
                .filter(|raw| raw.key == self.key)
                .and_then(|raw| raw.weak.upgrade());
            if needs_wave {
                // The global raw ledger, rather than a dormant slot weak,
                // keeps any escaped raw owner available for reuse.
                lock(&resident.0.state).last_raw = raw.as_ref().map(SharedBytes::downgrade);
            }
            let reservation = if needs_wave {
                let reservation = spare.take().unwrap();
                state.rows.get_mut(&self.key).unwrap().wave = Some(reservation.id);
                Some(reservation)
            } else if let Some(id) = existing_wave {
                state.permits.get_mut(&id).unwrap().references += 1;
                Some(Reservation {
                    controller: self.controller.clone(),
                    id,
                })
            } else {
                None
            };
            state.update_row(self.key, |row| {
                row.calls += 1;
                row.references += 1;
            });
            state.link(self.key, access);
            let prepared = Some(resident.prepare(access));
            drop(state);
            let call = Call {
                controller: self.controller.clone(),
                key: self.key,
                resident,
                prepared,
                reservation,
                finished: false,
                _member: Member {
                    controller: self.controller.clone(),
                    key: self.key,
                },
            };
            drop(spare); // any race-lost reservation releases outside the lock
            return Ok(call);
        }
    }
}

impl Call {
    pub(super) fn resolve(&mut self) -> Result<BodyPin, Arc<LoadError>> {
        self.prepared
            .take()
            .expect("one prepared action per call")
            .and_then(super::super::PreparedPin::resolve)
    }

    fn finish(&mut self, pin: bool) {
        if self.finished {
            return;
        }
        // A guard must never repeat a failed finish while the stack unwinds.
        self.finished = true;
        let (snapshot, raw) = {
            let snapshot = self.resident.snapshot();
            let mut local = lock(&self.resident.0.state);
            let raw = match &local.body {
                super::super::StateBody::Hot(body) => Some(body.bytes.clone()),
                _ => None,
            };
            if snapshot.phase == Phase::Cold {
                local.last_raw = None;
            }
            (snapshot, raw)
        };
        let mut state = lock(&self.controller.0.ledger);
        if let Some(raw) = raw
            && state.rows[&self.key].body == 0
        {
            let id = state.add_raw(self.key, &raw);
            state.update_row(self.key, |row| {
                row.raw = Some(id);
                row.body = snapshot.body_bytes.saturating_add(snapshot.tree_bytes);
            });
            state.body_bytes += snapshot.body_bytes as u128 + snapshot.tree_bytes as u128;
        }
        if let Some(reservation) = &self.reservation
            && snapshot.phase != Phase::Loading
        {
            let body = (snapshot.phase == Phase::Hot).then_some(
                snapshot
                    .raw_bytes
                    .saturating_add(snapshot.body_bytes)
                    .saturating_add(snapshot.tree_bytes),
            );
            // The call holds this permit alive; row.wave is only a join hint.
            state.settle_wave(reservation.id, body);
        }
        let inactive_cold = state.update_row(self.key, |row| {
            row.calls -= 1;
            if pin {
                row.pins += 1;
            }
            if row.calls == 0 {
                row.wave = None;
            }
            row.body == 0 && row.calls == 0
        });
        if inactive_cold {
            state.unlink(self.key);
            state.clear_transient_if_unused();
        }
    }
}

impl Drop for Call {
    fn drop(&mut self) {
        // Cancel the selected local action before releasing its reservation.
        drop(self.prepared.take());
        if !self.finished {
            self.finish(false);
        }
        // Reservation::drop will trim after removing the wave's last charge.
        if self.reservation.is_none() {
            self.controller.trim();
        }
    }
}

impl Clone for Member {
    fn clone(&self) -> Self {
        lock(&self.controller.0.ledger)
            .rows
            .get_mut(&self.key)
            .unwrap()
            .references += 1;
        Self {
            controller: self.controller.clone(),
            key: self.key,
        }
    }
}
impl Drop for Member {
    fn drop(&mut self) {
        lock(&self.controller.0.ledger)
            .rows
            .get_mut(&self.key)
            .unwrap()
            .references -= 1;
        self.controller.collect_unused(self.key);
        self.controller.trim();
    }
}

impl Pin {
    fn body(&self) -> &BodyPin {
        self.body.as_ref().expect("live pin owns its body")
    }

    /// Borrow under this pin. Retain `raw()` alongside any escaping raw clone;
    /// a bare clone of Document.raw is not a managed accounting lease.
    ///
    /// # Errors
    /// The retained member has verified invalid UTF-8 bytes.
    pub fn document(&self) -> Result<&model::Document, fs::hydration::HydrationError> {
        self.body().document()
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        self.body().bytes().as_bytes()
    }

    /// Retain raw bytes independently of the tree or member descriptor.
    #[must_use]
    pub fn raw(&self) -> RawPin {
        let bytes = self.body().bytes().clone();
        let id = bytes.allocation_id();
        lock(&self.member.controller.0.ledger).retain_raw(id);
        RawPin {
            bytes: Some(bytes),
            controller: self.member.controller.clone(),
            id,
        }
    }
}
impl Clone for Pin {
    fn clone(&self) -> Self {
        let member = self.member.clone();
        let body = self.body.clone();
        lock(&member.controller.0.ledger).update_row(member.key, |row| row.pins += 1);
        Self { body, member }
    }
}
impl Drop for Pin {
    fn drop(&mut self) {
        // BodyPin may be the final operation owner; release it before telling
        // the ledger that eviction is possible, and outside its mutex.
        drop(self.body.take());
        lock(&self.member.controller.0.ledger).update_row(self.member.key, |row| row.pins -= 1);
        self.member.controller.trim();
    }
}

impl RawPin {
    fn body(&self) -> &SharedBytes {
        self.bytes.as_ref().expect("live raw pin owns bytes")
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        self.body().as_bytes()
    }
    #[must_use]
    pub fn allocation_id(&self) -> usize {
        self.id
    }
}
impl Clone for RawPin {
    fn clone(&self) -> Self {
        let bytes = self.bytes.clone();
        lock(&self.controller.0.ledger).retain_raw(self.id);
        Self {
            bytes,
            controller: self.controller.clone(),
            id: self.id,
        }
    }
}
impl Drop for RawPin {
    fn drop(&mut self) {
        drop(self.bytes.take());
        lock(&self.controller.0.ledger).release_raw(Some(self.id));
        self.controller.trim();
    }
}

impl Reservation {
    fn peak(&self) -> usize {
        lock(&self.controller.0.ledger).permits[&self.id].peak
    }

    #[must_use]
    pub fn bytes(&self) -> usize {
        lock(&self.controller.0.ledger).permits[&self.id].bytes
    }
}
impl Clone for Reservation {
    fn clone(&self) -> Self {
        lock(&self.controller.0.ledger)
            .permits
            .get_mut(&self.id)
            .unwrap()
            .references += 1;
        Self {
            controller: self.controller.clone(),
            id: self.id,
        }
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        let mut state = lock(&self.controller.0.ledger);
        let permit = state.permits.get_mut(&self.id).unwrap();
        permit.references -= 1;
        if permit.references == 0 {
            let permit = state.permits.remove(&self.id).unwrap();
            state.reserved_bytes -= permit.bytes as u128;
            if !permit.settled {
                state.pending -= 1;
            }
            // No call can own this permit after its last reference ends.
            // Clear its join hint regardless of unrelated hot hits; never clear a retry.
            if let Some(key) = permit.member
                && let Some(row) = state.rows.get_mut(&key)
                && row.wave == Some(self.id)
            {
                row.wave = None;
            }
            state.clear_transient_if_unused();
            state.shrink();
        }
        drop(state);
        self.controller.trim();
    }
}

impl Floor {
    pub fn set(&self, bytes: usize) {
        let mut state = lock(&self.controller.0.ledger);
        state.set_floor(self.id, bytes);
        drop(state);
        self.controller.trim();
    }
}
impl Drop for Floor {
    fn drop(&mut self) {
        let mut state = lock(&self.controller.0.ledger);
        let bytes = state.floors.remove(&self.id).unwrap();
        state.external_floor -= bytes as u128;
        state.shrink();
        drop(state);
        self.controller.trim();
    }
}

fn local_status(resident: &Resident) -> (Phase, u64) {
    let state = lock(&resident.0.state);
    let phase = match state.body {
        super::super::StateBody::Cold => Phase::Cold,
        super::super::StateBody::Loading(_) => Phase::Loading,
        super::super::StateBody::Hot(_) => Phase::Hot,
    };
    (phase, state.revision)
}
