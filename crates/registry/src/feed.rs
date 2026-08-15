//! The §6.4 event feed — the engine owns its senses (merkle-spec, adjudicated
//! engine-side, five lanes unanimous; merged plan §4.3).
//!
//! One kernel file watcher per workspace (notify: `FSEvents` on macOS, inotify
//! on Linux), owned by this daemon — the engine process. Events accumulate
//! into a per-workspace DIRTY SET held by the registry, and the set applies
//! into the workspace's resident memo ([`fs::DomainCache`]) whenever the
//! cache is next borrowed ([`crate::Registry::domain_cache`]).
//!
//! # The registration-lifetime law (kimi D1, adopted)
//! The feed's lifetime is the workspace REGISTRATION, not engine warmth: it
//! starts when the workspace first grows resident state, survives every
//! engine idle-reap, and ends at `unregister` (or daemon exit). While the
//! engine is cold the set keeps accumulating, so the next warm applies
//! exactly the members that moved — O(dirty), never O(corpus). That is what
//! lets the reap RETAIN the resident memo: the feed covers the cold gap the
//! old design paid a full corpus re-read to close.
//!
//! # Hints only — never an instrument (§6.4 guard-plane law)
//! A dirty path is a HINT: applying one can only ADD a conservative re-read
//! (the applied entry lands with a spoiled [`fs::StatKey`], so the next
//! observation re-verifies it against disk). Nothing here can suppress a
//! read, and no guard or currency answer ever waits on an event arriving —
//! guards stay live folds through the memo's own stat evidence. The daemon
//! journal stays a legal ADDITIONAL feed through the same door
//! ([`WorkspaceFeed::note_dirty`]); its three vacuous windows are irrelevant
//! by construction because nothing depends on it.
//!
//! # What the set holds
//! Workspace-relative paths that could ever carry a member digest: `*.md`
//! with no dot-prefixed segment — both §12.1 STRUCTURAL floors, deliberately
//! config-independent so a domain-config change while cold can never have
//! filtered away the one event that mattered. Everything else (`.git`
//! churn, build artifacts, the `.meridian/cookie` sentinel) can never move
//! the root and never enters the set. Order is not kept: a spoil-set is
//! order-insensitive by construction (the ORDERED stream consumers — the
//! currency cookie — are the stamps card's, upstream of this filter).
//!
//! # Doubt collapses loudly
//! A kernel overflow (rescan flag), a watcher error, or the set outgrowing
//! [`DIRTY_CAP`] collapses the set to ALL-DIRTY: the next apply resets the
//! resident memo, and the following pass re-reads the corpus — exactly the
//! pre-feed baseline, correct and loud. The named-and-throttled rescan
//! ladder above this floor is the sibling rescan card's.

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use notify::event::{AccessKind, AccessMode};
use notify::{EventKind, RecursiveMode, Watcher as _};

/// The dirty set's size floor for the all-dirty collapse. Sized well above
/// any plausible cold-gap edit burst (the delta plane bounds one frame at
/// 128 rows) and well below corpus scale, so a runaway producer degrades to
/// the correct full re-read instead of holding an unbounded set.
const DIRTY_CAP: usize = 4096;

/// One workspace's event feed: the kernel watcher and the dirty set it
/// accumulates. Dropping it releases the kernel watch — which is why the
/// registry drops it only at `unregister`, never at reap.
#[derive(Debug)]
pub(crate) struct WorkspaceFeed {
    /// Keeps the kernel stream alive; the handler thread owns `state`'s
    /// other [`Arc`]. Held, never spoken to after construction.
    _watcher: notify::RecommendedWatcher,
    state: Arc<Mutex<FeedState>>,
}

/// The registry-held dirty set plus its instrument counters.
#[derive(Debug, Default)]
struct FeedState {
    /// Member-candidate rel paths seen dirty since the last take.
    dirty: BTreeSet<PathBuf>,
    /// Doubt collapse: overflow, watcher error, or cap breach. Sticky until
    /// taken; applying it resets the resident memo.
    all_dirty: bool,
    /// Kernel events accepted into the set (post-filter), over the feed's life.
    events: u64,
    /// Doubt collapses over the feed's life (rescan flags, errors, cap).
    overflows: u64,
    /// Dirty members applied into the resident memo, over the feed's life.
    applied: u64,
}

/// Published feed counters (the card's "counter published" receipt, probe
/// surface class): what arrived, what collapsed, what applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeedStats {
    /// Kernel events accepted into the dirty set (post-filter).
    pub events: u64,
    /// Doubt collapses (kernel rescan, watcher error, cap breach).
    pub overflows: u64,
    /// Dirty members applied into the resident memo.
    pub applied: u64,
    /// Paths currently pending application.
    pub pending: usize,
    /// Whether the pending state is the all-dirty collapse.
    pub all_dirty: bool,
}

/// What a take found pending.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Pending {
    /// Nothing pending — the hot no-op.
    Clean,
    /// These members were seen dirty since the last take.
    Paths(Vec<PathBuf>),
    /// Doubt collapse: everything is suspect.
    All,
}

/// One apply's outcome.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Applied {
    /// This many dirty members were re-derived against disk.
    Members(u64),
    /// The resident memo was reset (all-dirty, or an apply-time I/O failure):
    /// the next pass rebuilds from a full read — the pre-feed baseline.
    Reset,
}

impl WorkspaceFeed {
    /// Start the kernel watcher for `workspace` (a canonical root). The feed
    /// must exist BEFORE the first observation lands in the workspace's
    /// resident memo — an observation without gap coverage behind it would
    /// let a later re-warm trust the memo blind.
    ///
    /// # Errors
    /// The kernel watch could not be created or attached. The caller records
    /// the failure loudly; the workspace then keeps the pre-feed semantics
    /// (its resident memo drops on every reap).
    pub(crate) fn start(workspace: &Path) -> notify::Result<WorkspaceFeed> {
        let state = Arc::new(Mutex::new(FeedState::default()));
        let sink = Arc::clone(&state);
        let root = workspace.to_path_buf();
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                let mut s = sink.lock().unwrap_or_else(PoisonError::into_inner);
                match event {
                    Ok(event) => {
                        if event.need_rescan() {
                            s.collapse();
                            return;
                        }
                        if !relevant(&event.kind) {
                            return;
                        }
                        for path in &event.paths {
                            let Ok(rel) = path.strip_prefix(&root) else {
                                continue;
                            };
                            s.insert(rel);
                        }
                    }
                    // A watcher error is event loss until proven otherwise.
                    Err(_) => s.collapse(),
                }
            })?;
        watcher.watch(workspace, RecursiveMode::Recursive)?;
        Ok(WorkspaceFeed {
            _watcher: watcher,
            state,
        })
    }

    /// The ADDITIONAL-feed door (§6.4): dirty-path hints from any secondary
    /// source — the daemon journal where it already watches, or a test.
    /// Hints ride the same structural filter and the same conservative apply
    /// as kernel events; nothing anywhere depends on one arriving.
    pub(crate) fn note_dirty<'a>(&self, paths: impl IntoIterator<Item = &'a Path>) {
        let mut s = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        for path in paths {
            s.insert(path);
        }
    }

    /// Take whatever is pending, leaving the set clean.
    pub(crate) fn take(&self) -> Pending {
        let mut s = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if s.all_dirty {
            s.all_dirty = false;
            s.dirty.clear();
            return Pending::All;
        }
        if s.dirty.is_empty() {
            return Pending::Clean;
        }
        Pending::Paths(std::mem::take(&mut s.dirty).into_iter().collect())
    }

    /// Record members applied into the resident memo (the published counter).
    pub(crate) fn note_applied(&self, members: u64) {
        let mut s = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        s.applied += members;
    }

    /// The published counters.
    pub(crate) fn stats(&self) -> FeedStats {
        let s = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        FeedStats {
            events: s.events,
            overflows: s.overflows,
            applied: s.applied,
            pending: s.dirty.len(),
            all_dirty: s.all_dirty,
        }
    }
}

impl FeedState {
    /// Admit one rel path through the structural filter; collapse at the cap.
    fn insert(&mut self, rel: &Path) {
        if !member_candidate(rel) {
            return;
        }
        self.events += 1;
        if self.all_dirty {
            return;
        }
        self.dirty.insert(rel.to_path_buf());
        if self.dirty.len() > DIRTY_CAP {
            self.collapse();
        }
    }

    /// Doubt: drop the enumeration, mark everything suspect.
    fn collapse(&mut self) {
        self.all_dirty = true;
        self.dirty.clear();
        self.overflows += 1;
    }
}

/// Could `rel` ever carry a member digest? The two §12.1 STRUCTURAL floors —
/// md-only and no dot-prefixed segment — plus plain-relative shape (all
/// `Normal` components, so a hint can never name a path outside the root).
/// Deliberately config-free: the custom ignore list can change while the
/// engine is cold, so filtering by it here could drop the one event that
/// mattered; a custom-ignored member's spoil is merely a no-op re-read.
fn member_candidate(rel: &Path) -> bool {
    let mut any_segment = false;
    for component in rel.components() {
        let Component::Normal(seg) = component else {
            return false;
        };
        let Some(seg) = seg.to_str() else {
            return false;
        };
        if fs::domain::dot_segment(seg) {
            return false;
        }
        any_segment = true;
    }
    any_segment
        && rel
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("md"))
}

/// Which event kinds can carry a content change. `Access` is read-side noise
/// — the engine's own observation reads would feed themselves back through
/// the watcher as fresh dirt — EXCEPT close-after-write, which on inotify is
/// the one signal some write patterns leave.
fn relevant(kind: &EventKind) -> bool {
    match kind {
        EventKind::Access(AccessKind::Close(AccessMode::Write)) => true,
        EventKind::Access(_) => false,
        _ => true,
    }
}

/// Apply one take into the workspace's resident memo. Conservative by
/// construction: a present member is re-read NOW and lands through the
/// own-write overlay with a spoiled identity, so the next observation
/// re-verifies it; a vanished member leaves the fold through the overlay's
/// removal half; an unchanged digest applies nothing (self-echo dedup — a
/// cost saving only, overlay idempotence is the correctness). The tree is
/// never torn: no present member is ever transiently removed, so a
/// concurrent `overlay_root` fold stays truthful at every instant.
///
/// All-dirty — and any apply-time I/O failure other than absence — resets
/// the memo instead: the next pass re-reads the corpus, the pre-feed
/// baseline. A memo with no observation baseline yet applies nothing (the
/// cold first pass reads everything anyway).
pub(crate) fn apply(
    root: &fs::WorkspaceRoot,
    cache: &mut fs::DomainCache,
    pending: Pending,
) -> Applied {
    let paths = match pending {
        Pending::Clean => return Applied::Members(0),
        Pending::All => {
            *cache = fs::DomainCache::new();
            return Applied::Reset;
        }
        Pending::Paths(paths) => paths,
    };
    let mut applied = 0u64;
    for rel in paths {
        match std::fs::read(root.0.join(&rel)) {
            Ok(bytes) => {
                let digest = model::leaf_digest(&bytes);
                if cache.fold_at(&rel) == Ok(fs::resident::ScopeFold::Value(digest)) {
                    continue; // unchanged — the engine's own write echoing back
                }
                match cache.overlay_leaf(&rel, digest) {
                    Ok(_) => applied += 1,
                    // No baseline: the memo is cold and the first observation
                    // reads everything — the rest of the take is moot.
                    Err(_) => return Applied::Members(applied),
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                match cache.overlay_remove(&rel) {
                    Ok(_) => applied += 1,
                    Err(_) => return Applied::Members(applied),
                }
            }
            // Unreadable ≠ absent: no digest can be derived and absence would
            // be a lie — reset to the loud, correct baseline.
            Err(_) => {
                *cache = fs::DomainCache::new();
                return Applied::Reset;
            }
        }
    }
    Applied::Members(applied)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The structural filter: md-only, no dot segment, plain-relative shape.
    #[test]
    fn the_dirty_set_admits_only_member_candidates() {
        assert!(member_candidate(Path::new("notes/plan.md")));
        assert!(member_candidate(Path::new("UPPER.MD")));
        assert!(!member_candidate(Path::new("notes/data.json")));
        assert!(!member_candidate(Path::new(".git/index.md")));
        assert!(!member_candidate(Path::new("a/.hidden/x.md")));
        assert!(!member_candidate(Path::new(".meridian/cookie")));
        assert!(!member_candidate(Path::new("/abs/olute.md")));
        assert!(!member_candidate(Path::new("../escape.md")));
        assert!(!member_candidate(Path::new("")));
    }

    /// Read-side kinds never feed the set — the engine's own observation
    /// reads must not dirty what they read — except close-after-write.
    #[test]
    fn access_noise_is_filtered_except_close_write() {
        use notify::event::{AccessKind, AccessMode};
        assert!(!relevant(&EventKind::Access(AccessKind::Read)));
        assert!(!relevant(&EventKind::Access(AccessKind::Open(
            AccessMode::Any
        ))));
        assert!(!relevant(&EventKind::Access(AccessKind::Close(
            AccessMode::Read
        ))));
        assert!(relevant(&EventKind::Access(AccessKind::Close(
            AccessMode::Write
        ))));
        assert!(relevant(&EventKind::Modify(notify::event::ModifyKind::Any)));
        assert!(relevant(&EventKind::Create(notify::event::CreateKind::Any)));
        assert!(relevant(&EventKind::Remove(notify::event::RemoveKind::Any)));
        assert!(relevant(&EventKind::Any));
    }

    /// Cap breach collapses to all-dirty; the take drains it and the next
    /// take is clean.
    #[test]
    fn the_cap_collapses_to_all_dirty_and_take_drains() {
        let mut s = FeedState::default();
        for i in 0..=DIRTY_CAP {
            s.insert(Path::new(&format!("m{i}.md")));
        }
        assert!(s.all_dirty, "past the cap everything is suspect");
        assert_eq!(s.overflows, 1);
        assert!(s.dirty.is_empty(), "the enumeration is dropped, not kept");
        // Late arrivals during a collapse change nothing.
        s.insert(Path::new("late.md"));
        assert!(s.dirty.is_empty());
    }

    /// A take empties the set; a second take is clean; all-dirty wins over
    /// any path enumeration.
    #[test]
    fn takes_drain_and_all_dirty_wins() {
        let mut s = FeedState::default();
        s.insert(Path::new("a.md"));
        s.insert(Path::new("b.md"));
        s.insert(Path::new("a.md")); // set semantics
        assert_eq!(s.dirty.len(), 2);
        s.collapse();
        assert!(s.all_dirty && s.dirty.is_empty());
    }

    /// Apply, path arm: a changed member lands through the overlay (spoiled
    /// identity — the next observation re-verifies), an unchanged member
    /// applies nothing, a vanished member leaves the fold. All against a
    /// baselined memo.
    #[test]
    fn apply_re_derives_changed_members_and_skips_echoes() {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::WorkspaceRoot(dir.path().to_path_buf());
        std::fs::write(dir.path().join("a.md"), "# A\n").unwrap();
        std::fs::write(dir.path().join("b.md"), "# B\n").unwrap();
        std::fs::write(dir.path().join("c.md"), "# C\n").unwrap();
        let mut cache = fs::DomainCache::new();
        let baseline = cache.root(&root).unwrap();

        std::fs::write(dir.path().join("a.md"), "# A moved\n").unwrap();
        std::fs::remove_file(dir.path().join("c.md")).unwrap();
        let pending = Pending::Paths(vec![
            PathBuf::from("a.md"),
            PathBuf::from("b.md"), // untouched — the echo arm
            PathBuf::from("c.md"), // vanished
        ]);
        assert_eq!(
            apply(&root, &mut cache, pending),
            Applied::Members(2),
            "a and c apply; b is an unchanged echo"
        );
        let after = cache.root(&root).unwrap();
        assert_ne!(baseline, after);
        // The applied state equals a fresh derivation of the same disk.
        let fresh = fs::DomainCache::new().root(&root).unwrap();
        assert_eq!(after, fresh);
    }

    /// Apply, doubt arms: all-dirty resets the memo (next pass re-reads the
    /// corpus — the pre-feed baseline), and a cold memo applies nothing.
    #[test]
    fn apply_resets_on_all_dirty_and_skips_a_cold_memo() {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::WorkspaceRoot(dir.path().to_path_buf());
        std::fs::write(dir.path().join("a.md"), "# A\n").unwrap();
        let mut cache = fs::DomainCache::new();
        cache.root(&root).unwrap();
        assert_eq!(cache.leaves_read(), 1);

        assert_eq!(apply(&root, &mut cache, Pending::All), Applied::Reset);
        cache.root(&root).unwrap();
        assert_eq!(
            cache.leaves_read(),
            1,
            "the reset memo re-reads the corpus (fresh counter, one member)"
        );

        let mut cold = fs::DomainCache::new();
        assert_eq!(
            apply(
                &root,
                &mut cold,
                Pending::Paths(vec![PathBuf::from("a.md")])
            ),
            Applied::Members(0),
            "no baseline — the cold first pass covers everything"
        );
    }
}
