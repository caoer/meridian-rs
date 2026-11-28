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

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use run::modes::{LoadedBlock, ModuleCache, Page};

/// How many blocks one workspace keeps. A page carries a handful of blocks
/// and a fleet root a few pages of them, so this is generous for the real
/// shape and still a bound — an unbounded map keyed by a value that changes
/// on every edit is a slow leak, and a fleet daemon runs for weeks.
const CAPACITY: usize = 512;

/// How many validated prelude keys one workspace keeps.
const PRELUDE_CAPACITY: usize = 64;

/// How many parsed mode-row page copies (`(page_path, file_rev) -> Arc<Page>`)
/// one workspace keeps. Keyed by `file_rev`, so an edited page is a new key
/// and a warm fire on an unchanged page avoids re-walking and cloning every
/// fenced block of a large hooks page (e.g. 236 KB `HOOKS.md`).
const PAGE_CAPACITY: usize = 64;

/// One workspace's cached modules, validated preludes, and extracted mode pages.
#[derive(Default)]
pub struct WorkspaceModules {
    blocks: Mutex<HashMap<String, Arc<LoadedBlock>>>,
    preludes: Mutex<HashSet<String>>,
    pages: Mutex<HashMap<(String, String), Arc<Page>>>,
}

impl WorkspaceModules {
    /// Look up a cached [`Page`] copy for `(page, file_rev)`.
    #[must_use]
    pub fn get_page(&self, page: &str, file_rev: &str) -> Option<Arc<Page>> {
        self.pages
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&(page.to_owned(), file_rev.to_owned()))
            .map(Arc::clone)
    }

    /// Cache a [`Page`] copy for `(page, file_rev)`.
    pub fn put_page(&self, page: String, file_rev: String, copied: Arc<Page>) {
        let mut pages = self.pages.lock().unwrap_or_else(PoisonError::into_inner);
        if pages.len() >= PAGE_CAPACITY {
            pages.clear();
        }
        pages.insert((page, file_rev), copied);
    }
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

    fn prelude_ok(&self, key: &str) -> bool {
        self.preludes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(key)
    }

    fn mark_prelude_ok(&self, key: String) {
        let mut preludes = self.preludes.lock().unwrap_or_else(PoisonError::into_inner);
        if preludes.len() >= PRELUDE_CAPACITY {
            preludes.clear();
        }
        preludes.insert(key);
    }
}
