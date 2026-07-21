//! The in-memory registry: a mutex-guarded map keyed by canonical workspace
//! path, plus the register / resolve / unregister / list / reap operations.
//!
//! The map lock is the serialization point. Every mutation takes the write
//! lock; the whole register critical section (deny check → sentinel write →
//! insert → persist) runs under one held guard, so two concurrent registrars
//! for the same path serialize: the first creates the entry and the drawer
//! sentinel, the second sees the key present and adopts it. One registry
//! entry, one drawer sentinel — first-writer-wins **by serialization**
//! (decision 0001 round 5, point 1).

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{PoisonError, RwLock};

use crate::engine::{WarmOutcome, WorkspaceEngine};
use crate::now_secs;
use crate::protocol::{DenyKind, WorkspaceEntry};
use crate::state::StateStore;

/// The outcome of a [`Registry::register`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegisterOutcome {
    /// This caller created the entry (first writer) and wrote the sentinel.
    Registered(WorkspaceEntry),
    /// The path was already registered; this caller adopted the existing
    /// entry (no second sentinel written).
    Adopted(WorkspaceEntry),
    /// The deny ceiling refused the path.
    Denied(DenyKind),
    /// The path could not be canonicalized, or the sentinel write failed.
    Error(String),
}

/// The outcome of a [`Registry::resolve`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveOutcome {
    /// The cwd is inside a registered workspace; the adopted entry is returned.
    Adopted(WorkspaceEntry),
    /// No registered ancestor — the caller degrades to ephemeral.
    Miss,
}

/// The daemon's workspace registry: the guarded map plus the state store and
/// the drawer cache root.
///
/// `engines` is the resident query-engine state (decision 0002 spine root, U1):
/// a warm `WorkspaceEngine` per workspace, parallel to `inner` and keyed by the
/// same canonical `PathBuf`. It is a disposable projection of disk — never
/// persisted, never loaded at start (a cold daemon holds no engines) — so it is
/// NOT part of the state file. Idle-reap drops a warm engine alongside the
/// registration it belongs to.
#[derive(Debug)]
pub struct Registry {
    inner: RwLock<HashMap<PathBuf, WorkspaceEntry>>,
    engines: RwLock<HashMap<PathBuf, WorkspaceEngine>>,
    state: StateStore,
    cache_root: PathBuf,
}

impl Registry {
    /// Build a registry seeded with `entries` (loaded from the state file),
    /// persisting to `state` and writing drawer sentinels under `cache_root`.
    pub(crate) fn new(
        state: StateStore,
        cache_root: PathBuf,
        entries: Vec<WorkspaceEntry>,
    ) -> Self {
        let inner = entries
            .into_iter()
            .map(|entry| (entry.workspace.clone(), entry))
            .collect();
        Registry {
            inner: RwLock::new(inner),
            // Cold start holds no warm engines: residency is a disposable
            // projection, rebuilt from disk on the first `warm_or_build`.
            engines: RwLock::new(HashMap::new()),
            state,
            cache_root,
        }
    }

    /// Register `path` as a warm workspace.
    ///
    /// Canonicalizes, enforces the deny ceiling, then — under the write lock —
    /// adopts an existing entry or writes the drawer sentinel, inserts, and
    /// persists the state file. See the module docs for the serialization
    /// guarantee.
    pub fn register(&self, path: &Path) -> RegisterOutcome {
        let canonical = match workspace::canonicalize(path) {
            Ok(canonical) => canonical,
            Err(e) => {
                return RegisterOutcome::Error(format!(
                    "cannot canonicalize {} ({e})",
                    path.display()
                ));
            }
        };
        // Deny ceiling enforced IN THE DAEMON, not merely client-side.
        if let Some(reason) = workspace::deny_reason(&canonical) {
            return RegisterOutcome::Denied(reason.into());
        }

        let mut map = self.inner.write().unwrap_or_else(PoisonError::into_inner);
        if let Some(existing) = map.get_mut(&canonical) {
            existing.last_use = now_secs();
            return RegisterOutcome::Adopted(existing.clone());
        }

        // First writer for this path. Write the drawer sentinel BEFORE the map
        // insert, still under the lock, so a sentinel failure leaves no
        // dangling registry entry — one entry iff one sentinel.
        let drawer = cache::drawer_dir(&self.cache_root, &canonical);
        if let Err(e) = cache::register(&drawer, &canonical) {
            return RegisterOutcome::Error(format!(
                "drawer sentinel write failed for {} ({e})",
                canonical.display()
            ));
        }

        let now = now_secs();
        let entry = WorkspaceEntry {
            workspace: canonical.clone(),
            registered_at: now,
            last_use: now,
        };
        map.insert(canonical, entry.clone());
        self.persist(&map);
        RegisterOutcome::Registered(entry)
    }

    /// Resolve `cwd` against the registry: canonicalize, then walk it and its
    /// ancestors for the nearest registered workspace. A hit is adopted (its
    /// `last_use` bumped in memory — an LRU touch, not persisted); no hit is a
    /// [`ResolveOutcome::Miss`]. Never registers.
    pub fn resolve(&self, cwd: &Path) -> ResolveOutcome {
        let Ok(canonical) = workspace::canonicalize(cwd) else {
            return ResolveOutcome::Miss;
        };
        let mut map = self.inner.write().unwrap_or_else(PoisonError::into_inner);
        for ancestor in canonical.ancestors() {
            if let Some(entry) = map.get_mut(ancestor) {
                entry.last_use = now_secs();
                return ResolveOutcome::Adopted(entry.clone());
            }
        }
        ResolveOutcome::Miss
    }

    /// Warm the resident query engine for `workspace`, rebuilding it ONLY when
    /// the corpus content hash has changed (decision 0002 spine root, U1).
    ///
    /// Canonicalizes `workspace` (the `engines` key is the same canonical path
    /// `inner` uses), reads + folds the corpus content hash fresh from disk
    /// (`fs::domain_snapshot` — the cheap half, no parse), and:
    /// - reuses the warm engine and parses NOTHING when the hash is unchanged
    ///   ([`WarmOutcome::Reused`]);
    /// - rebuilds the index + document map exactly once when the hash changed or
    ///   the workspace was cold ([`WarmOutcome::Built`]).
    ///
    /// The reuse key is the corpus CONTENT hash the commit guards already
    /// compute (risk R5), not the unimplemented workspace-identity Merkle. The
    /// parse-heavy `fs::build_corpus` runs on the rebuild branch alone, so a
    /// `Reused` result provably ran zero parses.
    ///
    /// The fingerprint read holds no lock, and the rebuild parses OUTSIDE the
    /// `engines` write lock — the lock is taken only for the final insert — so
    /// warming one workspace never blocks another. A rare concurrent rebuild of
    /// the same workspace is last-write-wins and still correct (generous
    /// residency, decision 0002 §2).
    ///
    /// # Errors
    /// `workspace` cannot be canonicalized (does not exist), the corpus cannot
    /// be read, or a corpus file is non-UTF-8 (refused,
    /// [`io::ErrorKind::InvalidData`]).
    pub fn warm_or_build(&self, workspace: &Path) -> io::Result<WarmOutcome> {
        let canonical = workspace::canonicalize(workspace)
            .map_err(|e| io::Error::new(io::ErrorKind::NotFound, e))?;
        let root = fs::WorkspaceRoot(canonical.clone());

        // Cheap half (no parse): read + fold the corpus content hash from disk.
        let (files, fingerprint) = fs::domain_snapshot(&root)?;

        // Warm and unchanged → reuse, ZERO parses.
        {
            let engines = self.engines.read().unwrap_or_else(PoisonError::into_inner);
            if engines
                .get(&canonical)
                .is_some_and(|engine| engine.at_fingerprint == fingerprint)
            {
                return Ok(WarmOutcome::Reused);
            }
        }

        // Cold or content changed → rebuild exactly once (the only parse site).
        let (index, docs) = fs::build_corpus(files)?;
        let parsed = docs.len();
        let engine = WorkspaceEngine {
            index,
            docs,
            at_fingerprint: fingerprint,
        };
        self.engines
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(canonical, engine);
        Ok(WarmOutcome::Built { docs: parsed })
    }

    /// Borrow the warm engine for `canonical` (an already-canonical workspace
    /// path) under the read lock, running `f` on it — `None` when no engine is
    /// resident. The daemon's read path calls [`warm_or_build`](Self::warm_or_build)
    /// first (ensuring the engine reflects current disk and is warm), then serves
    /// the borrowed state through this accessor — so the read never parses when
    /// the corpus is unchanged (U2, served from U1's resident state).
    ///
    /// The closure runs UNDER the read lock; keep it to a borrow-and-project (no
    /// blocking, no re-entrancy into the engines lock), so concurrent reads of
    /// other workspaces never wait on it.
    pub fn with_engine<R>(
        &self,
        canonical: &Path,
        f: impl FnOnce(Option<&WorkspaceEngine>) -> R,
    ) -> R {
        let engines = self.engines.read().unwrap_or_else(PoisonError::into_inner);
        f(engines.get(canonical))
    }

    /// Unregister `path`, dropping it from memory and the state file. The
    /// drawer is left for `cache::gc`. Returns `true` when an entry was
    /// removed.
    ///
    /// Matches on the canonical path when the directory still resolves, else
    /// on the path as given — so a vanished workspace can still be unregistered
    /// by the canonical path a `list` reported.
    pub fn unregister(&self, path: &Path) -> bool {
        let key = workspace::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let mut map = self.inner.write().unwrap_or_else(PoisonError::into_inner);
        let removed = map.remove(&key).is_some();
        if removed {
            self.persist(&map);
        }
        removed
    }

    /// Every registered workspace, unordered.
    #[must_use]
    pub fn entries(&self) -> Vec<WorkspaceEntry> {
        self.inner
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .cloned()
            .collect()
    }

    /// Drop entries whose `last_use` is at or before `now - threshold_secs`,
    /// persisting the survivors. Returns the reaped workspace paths.
    ///
    /// `now` and `threshold_secs` are parameters (an injectable clock), so the
    /// reap horizon is unit-testable without waiting days: a far-future `now`
    /// ages every entry past the horizon, and `threshold_secs == 0` reaps all
    /// present entries. The reaper only deregisters; it never touches the
    /// drawer (that is `cache::gc`'s separate horizon).
    ///
    /// A reaped workspace's warm engine is dropped too (decision 0002 risk R4):
    /// warm-engine eviction hangs off this ONE idle-reap horizon — no separate
    /// memory budget or eviction policy. The `engines` lock is taken AFTER the
    /// `inner` lock is released, so the two maps are never held at once.
    pub fn reap(&self, now: u64, threshold_secs: u64) -> Vec<PathBuf> {
        let cutoff = now.saturating_sub(threshold_secs);
        let reaped: Vec<PathBuf> = {
            let mut map = self.inner.write().unwrap_or_else(PoisonError::into_inner);
            let reaped: Vec<PathBuf> = map
                .iter()
                .filter(|(_, entry)| entry.last_use <= cutoff)
                .map(|(key, _)| key.clone())
                .collect();
            for key in &reaped {
                map.remove(key);
            }
            if !reaped.is_empty() {
                self.persist(&map);
            }
            reaped
        };
        if !reaped.is_empty() {
            let mut engines = self.engines.write().unwrap_or_else(PoisonError::into_inner);
            for key in &reaped {
                engines.remove(key);
            }
        }
        reaped
    }

    /// Persist the current map to the state file, logging (never failing) on a
    /// write error — a lost persist costs a warm registration across restart,
    /// which is recoverable; it must not crash the daemon.
    fn persist(&self, map: &HashMap<PathBuf, WorkspaceEntry>) {
        let entries: Vec<WorkspaceEntry> = map.values().cloned().collect();
        if let Err(e) = self.state.save(&entries) {
            eprintln!("registry: state save failed ({e}); warm set may not survive restart");
        }
    }

    /// Persist the current map to the state file (used at graceful shutdown to
    /// capture in-memory `last_use` bumps from `resolve`).
    pub(crate) fn flush(&self) {
        let map = self.inner.read().unwrap_or_else(PoisonError::into_inner);
        self.persist(&map);
    }
}

#[cfg(test)]
mod engine_tests {
    //! Resident-engine gates (decision 0002 spine root, U1): warm reuse parses
    //! nothing, a corpus change forces exactly one rebuild, the warm engine
    //! answers a real query, and idle-reap drops the warm engine.

    use super::*;
    use crate::state::StateStore;
    use std::fs;

    /// A registry rooted under `home` (state file + cache drawer root). No
    /// socket — `warm_or_build` is an in-process method (the socket is U2/U3).
    fn registry_in(home: &Path) -> Registry {
        let cache_root = home.join("cache");
        fs::create_dir_all(&cache_root).unwrap();
        Registry::new(
            StateStore::new(home.join("state.json")),
            cache_root,
            Vec::new(),
        )
    }

    /// A workspace `home/ws` seeded with `files` (a sibling of the cache root,
    /// so the corpus walk never sees the drawer).
    fn write_ws(home: &Path, files: &[(&str, &str)]) -> PathBuf {
        let ws = home.join("ws");
        fs::create_dir_all(&ws).unwrap();
        for (rel, content) in files {
            let path = ws.join(rel);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(path, content).unwrap();
        }
        ws
    }

    #[test]
    fn second_warm_at_same_fingerprint_parses_nothing() {
        let home = tempfile::tempdir().unwrap();
        let reg = registry_in(home.path());
        let ws = write_ws(
            home.path(),
            &[("a.md", "# A\n\nsee [[b]]\n"), ("b.md", "# B\n")],
        );

        // Cold → build: two files, two parses (one syntax::parse per doc).
        assert_eq!(
            reg.warm_or_build(&ws).unwrap(),
            WarmOutcome::Built { docs: 2 },
            "first warm builds the corpus"
        );
        // Unchanged corpus content hash → reuse. `Reused` is the parse-count
        // proof: `fs::build_corpus` (the only parse site) is reached on the
        // rebuild branch alone, so a `Reused` result ran ZERO parses.
        assert_eq!(
            reg.warm_or_build(&ws).unwrap(),
            WarmOutcome::Reused,
            "second warm at the same fingerprint parses nothing"
        );
        assert_eq!(
            reg.warm_or_build(&ws).unwrap(),
            WarmOutcome::Reused,
            "and stays warm — no rebuild storm"
        );
    }

    #[test]
    fn corpus_mutation_triggers_exactly_one_rebuild() {
        let home = tempfile::tempdir().unwrap();
        let reg = registry_in(home.path());
        let ws = write_ws(home.path(), &[("a.md", "# A\n"), ("b.md", "# B\n")]);

        assert_eq!(
            reg.warm_or_build(&ws).unwrap(),
            WarmOutcome::Built { docs: 2 }
        );
        assert_eq!(reg.warm_or_build(&ws).unwrap(), WarmOutcome::Reused);

        // Mutate one file → the corpus content hash changes.
        fs::write(ws.join("a.md"), "# A changed\n\nnew body\n").unwrap();

        // Exactly ONE rebuild on the next warm...
        assert_eq!(
            reg.warm_or_build(&ws).unwrap(),
            WarmOutcome::Built { docs: 2 },
            "a corpus change rebuilds once"
        );
        // ...then warm reuse resumes at the new fingerprint.
        assert_eq!(
            reg.warm_or_build(&ws).unwrap(),
            WarmOutcome::Reused,
            "the rebuild is once, not a storm"
        );
    }

    #[test]
    fn warm_engine_serves_a_real_query() {
        let home = tempfile::tempdir().unwrap();
        let reg = registry_in(home.path());
        let ws = write_ws(
            home.path(),
            &[("a.md", "# A\n\nsee [[b]]\n"), ("b.md", "# B\n")],
        );

        reg.warm_or_build(&ws).unwrap();

        // The resident index + docs answer a real `query::links` — the warm
        // state is genuinely usable, not just present (U2's read surface).
        let canonical = workspace::canonicalize(&ws).unwrap();
        let engines = reg.engines.read().unwrap();
        let engine = engines.get(&canonical).expect("warm engine resident");
        let links = query::links(&engine.index, &engine.docs, Some("a.md"));
        let a = links.get("a.md").expect("a.md edge entry");
        assert_eq!(
            a.resolved.get("b.md"),
            Some(&1),
            "the resident index resolves [[b]] → b.md"
        );
    }

    #[test]
    fn reap_drops_the_warm_engine() {
        let home = tempfile::tempdir().unwrap();
        let reg = registry_in(home.path());
        let ws = write_ws(home.path(), &[("a.md", "# A\n")]);
        let canonical = workspace::canonicalize(&ws).unwrap();

        // A registration gives the reaper an `inner` entry to age out; warm the
        // engine alongside it.
        reg.register(&canonical);
        reg.warm_or_build(&canonical).unwrap();
        assert!(
            reg.engines.read().unwrap().contains_key(&canonical),
            "engine warm before reap"
        );

        // Reap the whole warm set (far-future now, zero horizon) — the entry AND
        // its warm engine are dropped on the ONE idle-reap horizon (risk R4).
        let reaped = reg.reap(u64::MAX, 0);
        assert!(reaped.contains(&canonical), "the entry was reaped");
        assert!(
            !reg.engines.read().unwrap().contains_key(&canonical),
            "reap drops the warm engine with the registration"
        );
    }
}
