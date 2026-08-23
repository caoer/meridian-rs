//! The resident per-block-rev module cache (hook-support design § 2.2 step 3:
//! *"Declarations and frozen modules are cached by block rev (plus the
//! prelude's blake3); an unchanged block is served from cache, never
//! re-evaluated"*).
//!
//! This is why a WARM fire is one function call. A cold fire parses the
//! block, evaluates its top level and freezes the module; a warm one does
//! none of those.
//!
//! **It holds no truth — only work already done.** The key is the block's own
//! `node_rev` plus a digest of the prelude, so an edited block is a different
//! key and its stale module is simply never asked for again. Nothing
//! invalidates: there is nothing to get wrong, because a rev IS the identity
//! of the bytes. A restart re-evaluates and loses nothing.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use run::modes::{LoadedBlock, ModuleCache};

/// How many blocks one workspace keeps. A page carries a handful of blocks
/// and a fleet root a few pages of them, so this is generous for the real
/// shape and still a bound — an unbounded map keyed by a value that changes
/// on every edit is a slow leak, and a fleet daemon runs for weeks.
const CAPACITY: usize = 512;

/// One workspace's cached modules.
#[derive(Default)]
pub struct WorkspaceModules {
    blocks: Mutex<HashMap<String, Arc<LoadedBlock>>>,
}

// `LoadedBlock` holds a starlark `FrozenModule`, which has no `Debug`. The
// registry derives `Debug`, so the cache states its SIZE rather than its
// contents — which is the only thing about it a reader of a registry dump
// could act on anyway.
impl std::fmt::Debug for WorkspaceModules {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let held = self.blocks.lock().map_or(0, |b| b.len());
        write!(f, "WorkspaceModules({held} blocks)")
    }
}

impl ModuleCache for WorkspaceModules {
    fn get(&self, key: &str) -> Option<Arc<LoadedBlock>> {
        self.blocks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .map(Arc::clone)
    }

    fn put(&self, key: String, loaded: Arc<LoadedBlock>) {
        let mut blocks = self.blocks.lock().unwrap_or_else(PoisonError::into_inner);
        // At the bound, drop everything and start again rather than evict by
        // a recency the cache does not track. A cache that holds no truth can
        // be emptied at any moment for free; inventing an LRU here would add
        // a data structure to defend a cost nobody has measured.
        if blocks.len() >= CAPACITY {
            blocks.clear();
        }
        blocks.insert(key, loaded);
    }
}
