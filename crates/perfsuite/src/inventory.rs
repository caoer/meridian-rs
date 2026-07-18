//! Construct inventories — what the generator planted, per file and per
//! corpus. A correct parser must find **at least** these counts (the generator
//! only plants in recognizable contexts — see `gen`'s invariant), so
//! inventories extend correctness assertions to arbitrary generated scale
//! without ever becoming ground truth (the frozen GT pack stays that).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Counts keyed by construct id ([`crate::profile::KNOWN_CONSTRUCTS`], plus
/// `fence_unterminated` for the pathology variant — a subset of `fence`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inventory(pub BTreeMap<String, u64>);

impl Inventory {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn bump(&mut self, id: &str) {
        self.bump_n(id, 1);
    }

    pub fn bump_n(&mut self, id: &str, n: u64) {
        if n > 0 {
            *self.0.entry(id.to_owned()).or_default() += n;
        }
    }

    #[must_use]
    pub fn count(&self, id: &str) -> u64 {
        self.0.get(id).copied().unwrap_or(0)
    }

    pub fn merge(&mut self, other: &Self) {
        for (k, v) in &other.0 {
            self.bump_n(k, *v);
        }
    }
}
