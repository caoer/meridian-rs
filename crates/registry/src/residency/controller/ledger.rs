use super::*;
use std::collections::hash_map::RandomState;

type Table<K, V> = hashbrown::HashMap<K, V, RandomState>;

#[derive(Debug)]
pub(super) struct Row {
    pub resident: Resident,
    pub references: usize,
    pub calls: usize,
    pub pins: usize,
    pub body: usize,
    pub raw: Option<usize>,
    pub previous: Option<Key>,
    pub next: Option<Key>,
    pub listed: bool,
    pub retiring: bool,
    pub force_shed: bool,
    pub wave: Option<u64>,
}

#[derive(Debug)]
pub(super) struct Raw {
    pub weak: model::bytes::WeakBytes,
    pub bytes: usize,
    pub key: Key,
    // Rows retain an owner through the entire retirement call; raw leases
    // retain one through payload destruction. Weak sampling alone is unsafe
    // while the last strong owner's destructor is still in progress.
    pub owners: usize,
}

#[derive(Debug)]
pub(super) struct Permit {
    pub bytes: usize,
    pub references: usize,
    pub revision: u64,
    pub transient: Option<Key>,
    pub activated: bool,
}

#[derive(Debug)]
pub(super) struct Ledger {
    pub limit: usize,
    pub rows: Table<Key, Row>,
    pub raw: Table<usize, Raw>,
    pub floors: Table<u64, usize>,
    pub permits: Table<u64, Permit>,
    pub serial: u64,
    pub first: Option<Key>,
    pub last: Option<Key>,
    pub transient: Option<Key>,
    pub detached_floor: usize,
    pub detached_body: usize,
    pub peak_shortfall: usize,
    pub external_floor: u128,
    pub raw_bytes: u128,
    pub body_bytes: u128,
    pub reserved_bytes: u128,
}

impl Ledger {
    pub fn new(limit: usize) -> Self {
        Self {
            limit,
            rows: Table::with_hasher(RandomState::new()),
            raw: Table::with_hasher(RandomState::new()),
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
        }
    }

    pub fn unlink(&mut self, key: Key) {
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

    pub fn link(&mut self, key: Key, access: Access) {
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

    pub fn add_raw(&mut self, key: Key, bytes: &SharedBytes) -> usize {
        let id = bytes.allocation_id();
        let raw = self.raw.entry(id).or_insert_with(|| {
            self.raw_bytes += bytes.heap_bytes() as u128;
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

    pub fn release_raw(&mut self, id: Option<usize>) {
        if let Some(id) = id {
            self.raw.get_mut(&id).unwrap().owners -= 1;
            self.prune_raw(id);
        }
    }

    pub fn prune_raw(&mut self, id: usize) {
        if self
            .raw
            .get(&id)
            .is_some_and(|raw| raw.owners == 0 && raw.weak.upgrade().is_none())
        {
            self.raw_bytes -= self.raw.remove(&id).unwrap().bytes as u128;
        }
    }

    pub fn floor_exact(&self) -> u128 {
        model::allocation::arc_bytes::<Control>() as u128
            + self.rows.allocation_size() as u128
            + self.raw.allocation_size() as u128
            + self.floors.allocation_size() as u128
            + self.permits.allocation_size() as u128
            + self.rows.len() as u128 * descriptor_bytes() as u128
            + self.detached_floor as u128
            + self.external_floor
    }

    pub fn total_exact(&self) -> u128 {
        self.floor_exact() + self.raw_bytes + self.body_bytes + self.reserved_bytes
    }

    pub fn prune(&mut self) {
        self.raw.retain(|_, raw| {
            if raw.owners != 0 || raw.weak.upgrade().is_some() {
                return true;
            }
            self.raw_bytes -= raw.bytes as u128;
            false
        });
        if self.rows.len().saturating_mul(4) < self.rows.capacity() {
            self.rows.shrink_to_fit();
        }
        if self.raw.len().saturating_mul(4) < self.raw.capacity() {
            self.raw.shrink_to_fit();
        }
        if self.floors.len().saturating_mul(4) < self.floors.capacity() {
            self.floors.shrink_to_fit();
        }
        if self.permits.len().saturating_mul(4) < self.permits.capacity() {
            self.permits.shrink_to_fit();
        }
    }

    pub fn report(&mut self) -> Report {
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

    pub fn next_id(&mut self) -> Result<u64, AdmissionError> {
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
