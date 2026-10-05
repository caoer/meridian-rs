use super::{Access, AdmissionError, Control, Key, Report, Resident, SharedBytes};
use std::collections::hash_map::RandomState;

type Table<K, V> = hashbrown::HashMap<K, V, RandomState>;

#[derive(Debug)]
pub(super) struct Row {
    pub(super) resident: Resident,
    pub(super) references: usize,
    pub(super) calls: usize,
    pub(super) pins: usize,
    pub(super) body: usize,
    pub(super) raw: Option<usize>,
    pub(super) previous: Option<Key>,
    pub(super) next: Option<Key>,
    pub(super) listed: bool,
    pub(super) retiring: bool,
    pub(super) force_shed: bool,
    pub(super) wave: Option<u64>,
}

#[derive(Debug)]
pub(super) struct Raw {
    pub(super) weak: model::bytes::WeakBytes,
    pub(super) bytes: usize,
    pub(super) key: Key,
    // Rows retain an owner through the entire retirement call; raw leases
    // retain one through payload destruction. Weak sampling alone is unsafe
    // while the last strong owner's destructor is still in progress.
    pub(super) owners: usize,
}

#[derive(Debug)]
pub(super) struct Permit {
    pub(super) bytes: usize,
    pub(super) peak: usize,
    pub(super) member: Option<Key>,
    pub(super) settled: bool,
    pub(super) references: usize,
    pub(super) revision: u64,
    pub(super) transient: Option<Key>,
}

#[derive(Debug)]
pub(super) struct Ledger {
    pub(super) limit: usize,
    pub(super) rows: Table<Key, Row>,
    pub(super) raw: Table<usize, Raw>,
    // Canonical raw allocation and record count per retained byte identity.
    pub(super) raw_versions: Table<Key, (usize, usize)>,
    pub(super) floors: Table<u64, usize>,
    pub(super) permits: Table<u64, Permit>,
    pub(super) serial: u64,
    pub(super) first: Option<Key>,
    pub(super) last: Option<Key>,
    pub(super) transient: Option<Key>,
    pub(super) detached_floor: usize,
    pub(super) detached_body: usize,
    pub(super) peak_shortfall: usize,
    pub(super) external_floor: u128,
    pub(super) raw_bytes: u128,
    pub(super) body_bytes: u128,
    pub(super) reserved_bytes: u128,
    pub(super) pending: usize,
    pub(super) forced: usize,
}

impl Ledger {
    pub(super) fn new(limit: usize) -> Self {
        Self {
            limit,
            rows: Table::with_hasher(RandomState::new()),
            raw: Table::with_hasher(RandomState::new()),
            raw_versions: Table::with_hasher(RandomState::new()),
            floors: Table::with_hasher(RandomState::new()),
            permits: Table::with_hasher(RandomState::new()),
            serial: 0,
            first: None,
            last: None,
            transient: None,
            detached_floor: 0,
            detached_body: 0,
            peak_shortfall: 0,
            external_floor: 0,
            raw_bytes: 0,
            body_bytes: 0,
            reserved_bytes: 0,
            pending: 0,
            forced: 0,
        }
    }

    pub(super) fn unlink(&mut self, key: Key) {
        let row = &self.rows[&key];
        if !row.listed {
            return;
        }
        let (previous, next) = (row.previous, row.next);
        if let Some(previous) = previous {
            self.rows.get_mut(&previous).unwrap().next = next;
        } else {
            self.first = next;
        }
        if let Some(next) = next {
            self.rows.get_mut(&next).unwrap().previous = previous;
        } else {
            self.last = previous;
        }
        let row = self.rows.get_mut(&key).unwrap();
        row.listed = false;
        row.previous = None;
        row.next = None;
    }

    pub(super) fn link(&mut self, key: Key, access: Access) {
        if access == Access::Scan && self.rows[&key].listed {
            return;
        }
        self.unlink(key);
        let (previous, next) = if access == Access::Read {
            (self.last, None)
        } else {
            (None, self.first)
        };
        let row = self.rows.get_mut(&key).unwrap();
        row.listed = true;
        row.previous = previous;
        row.next = next;
        if let Some(previous) = previous {
            self.rows.get_mut(&previous).unwrap().next = Some(key);
        } else {
            self.first = Some(key);
        }
        if let Some(next) = next {
            self.rows.get_mut(&next).unwrap().previous = Some(key);
        } else {
            self.last = Some(key);
        }
    }

    pub(super) fn add_raw(&mut self, key: Key, bytes: &SharedBytes) -> usize {
        let id = bytes.allocation_id();
        let raw = self.raw.entry(id).or_insert_with(|| {
            self.raw_bytes += bytes.heap_bytes() as u128;
            let version = self.raw_versions.entry(key).or_insert((id, 0));
            version.0 = id;
            version.1 += 1;
            Raw {
                weak: bytes.downgrade(),
                bytes: bytes.heap_bytes(),
                key,
                owners: 0,
            }
        });
        raw.owners += 1;
        id
    }

    pub(super) fn release_raw(&mut self, id: Option<usize>) {
        if let Some(id) = id {
            self.raw.get_mut(&id).unwrap().owners -= 1;
            self.prune_raw(id);
        }
    }

    pub(super) fn prune_raw(&mut self, id: usize) {
        if self
            .raw
            .get(&id)
            .is_some_and(|raw| raw.owners == 0 && !raw.weak.is_live())
        {
            let raw = self.raw.remove(&id).unwrap();
            self.raw_bytes -= raw.bytes as u128;
            let version = self.raw_versions.get_mut(&raw.key).unwrap();
            version.1 -= 1;
            if version.1 == 0 {
                self.raw_versions.remove(&raw.key);
            } else if version.0 == id {
                // Only overlapping retirement can leave a second allocation
                // for one identity. Prefer a live lease over dying storage.
                version.0 = self
                    .raw
                    .iter()
                    .filter(|(_, other)| other.key == raw.key)
                    .max_by_key(|(_, other)| other.weak.is_live())
                    .map(|(id, _)| *id)
                    .unwrap();
            }
        }
        self.clear_transient_if_unused();
        self.shrink();
    }

    pub(super) fn clear_transient_if_unused(&mut self) {
        let Some(key) = self.transient else {
            return;
        };
        let row = self
            .rows
            .get(&key)
            .is_some_and(|row| row.body != 0 || row.calls != 0 || row.retiring);
        let raw = self
            .raw_versions
            .get(&key)
            .and_then(|(id, _)| self.raw.get(id))
            .is_some_and(|raw| raw.owners != 0 || raw.weak.is_live());
        let reservation = self
            .permits
            .values()
            .any(|permit| permit.transient == Some(key));
        if !row && !raw && !reservation {
            self.transient = None;
        }
    }

    pub(super) fn floor_exact(&self) -> u128 {
        model::allocation::arc_bytes::<Control>() as u128
            + self.rows.allocation_size() as u128
            + self.raw.allocation_size() as u128
            + self.raw_versions.allocation_size() as u128
            + self.floors.allocation_size() as u128
            + self.permits.allocation_size() as u128
            + self.rows.len() as u128 * descriptor_bytes() as u128
            + self.detached_floor as u128
            + self.external_floor
    }

    pub(super) fn total_exact(&self) -> u128 {
        self.floor_exact() + self.raw_bytes + self.body_bytes + self.reserved_bytes
    }

    pub(super) fn prune(&mut self) {
        let old_len = self.raw.len();
        self.raw.retain(|_, raw| {
            if raw.owners != 0 || raw.weak.is_live() {
                return true;
            }
            self.raw_bytes -= raw.bytes as u128;
            false
        });
        if self.raw.len() != old_len {
            // A census may find an externally held source's final owner gone.
            // Rebuild without a scratch vector or a second raw payload owner.
            self.raw_versions.clear();
            for (&id, raw) in &self.raw {
                let version = self.raw_versions.entry(raw.key).or_insert((id, 0));
                version.1 += 1;
                if raw.weak.is_live() {
                    version.0 = id;
                }
            }
        }
        self.clear_transient_if_unused();
        self.shrink();
    }

    pub(super) fn shrink(&mut self) {
        let version_min = self.raw_versions.len().saturating_add(self.pending);
        if version_min.saturating_mul(4) < self.raw_versions.capacity() {
            self.raw_versions.shrink_to(version_min);
        }
        if self.rows.len().saturating_mul(4) < self.rows.capacity() {
            self.rows.shrink_to_fit();
        }
        let raw_min = self.raw.len().saturating_add(self.pending);
        if raw_min.saturating_mul(4) < self.raw.capacity() {
            self.raw.shrink_to(raw_min);
        }
        if self.floors.len().saturating_mul(4) < self.floors.capacity() {
            self.floors.shrink_to_fit();
        }
        if self.permits.len().saturating_mul(4) < self.permits.capacity() {
            self.permits.shrink_to_fit();
        }
    }

    pub(super) fn report(&mut self) -> Report {
        self.prune();
        let floor_exact = self.floor_exact();
        let total_exact = self.total_exact();
        let floor = bounded(floor_exact);
        let raw = bounded(self.raw_bytes);
        let bodies = bounded(self.body_bytes);
        let reserved = bounded(self.reserved_bytes);
        let allocated = bounded(floor_exact + self.raw_bytes + self.body_bytes);
        let total = bounded(total_exact);
        let pinned = sum(self
            .rows
            .values()
            .filter(|row| row.pins != 0)
            .map(|row| row.body));
        let retiring = sum(self
            .rows
            .values()
            .filter(|row| row.retiring)
            .map(|row| row.body))
        .saturating_add(self.detached_floor)
        .saturating_add(self.detached_body);
        Report {
            limit: self.limit,
            floor,
            raw,
            bodies,
            reserved,
            allocated,
            total,
            pinned,
            retiring,
            overage: if self.limit == 0 {
                0
            } else {
                bounded(total_exact.saturating_sub(self.limit as u128))
            },
            floor_overage: if self.limit == 0 {
                0
            } else {
                bounded(floor_exact.saturating_sub(self.limit as u128))
            },
            transient: self.transient.is_some(),
            peak_shortfall: self.peak_shortfall,
            members: self.rows.len(),
            reservations: self.permits.len(),
        }
    }

    pub(super) fn settle_wave(&mut self, id: u64, body: Option<usize>) {
        let permit = self.permits.get_mut(&id).unwrap();
        if permit.settled {
            return;
        }
        permit.settled = true;
        self.pending -= 1;
        if let Some(body) = body {
            // Replace the reservation's body portion with measured allocations;
            // retain the rest through the last flight caller. Errors keep the
            // entire reservation until their payload owners release it.
            let converted = permit.bytes.min(body);
            permit.bytes -= converted;
            self.reserved_bytes -= converted as u128;
        }
    }

    pub(super) fn retain_raw(&mut self, id: usize) {
        self.raw.get_mut(&id).unwrap().owners += 1;
    }

    pub(super) fn set_floor(&mut self, id: u64, bytes: usize) {
        let old = self.floors.insert(id, bytes).unwrap();
        self.external_floor = self.external_floor - old as u128 + bytes as u128;
    }

    pub(super) fn next_id(&mut self) -> Result<u64, AdmissionError> {
        self.serial = self
            .serial
            .checked_add(1)
            .ok_or(AdmissionError::Exhausted)?;
        Ok(self.serial)
    }
}

pub(super) fn descriptor_bytes() -> usize {
    model::allocation::arc_bytes::<super::super::Inner>()
}

fn sum(values: impl Iterator<Item = usize>) -> usize {
    values.fold(0, usize::saturating_add)
}

fn bounded(bytes: u128) -> usize {
    usize::try_from(bytes).unwrap_or(usize::MAX)
}
