//! History compaction of the resident sql caches (`wire-contract.md` § A.11,
//! history compaction; `view::store` § History compaction). Off unless the
//! host sets [`crate::server::SQL_COMPACTION_ENV`].
//!
//! A cache's `hist` keeps every version of every page it ever appended, and
//! every latest view first picks each page's newest version out of them. Once
//! a reap interval the reaper starts one look ([`tick`]): over the open caches,
//! the one whose `hist.frontmatter` holds at least the configured multiple of
//! the rows `main.frontmatter` serves, the most such first, is rebuilt. One
//! at a time per daemon, on a thread of its own.
//!
//! The rebuild is a cold build of a fresh file in the drawer's
//! [`COMPACT_DIR`] — its own directory, so its instance spills into its own
//! `sql-spill` — through the same append a served `sql` call makes
//! ([`crate::sql_op::with_appender`]), then appends to catch up with the
//! generations published meanwhile. It holds neither the store's mutex nor a
//! handle on the store: `sql` calls keep answering from the old file
//! throughout, and a close, reap or eviction treats the cache like any other.
//! The swap ([`view::store::SqlStore::swap_in`]) takes the handle open at that
//! moment and its mutex, waits for the reads open on that instance to end, and
//! renames the new file over the old one. Whatever fails before the rename —
//! the build, the catch-up, the daemon itself — leaves the old file serving;
//! a failure leaves that cache out of the looks for [`FAILED_PAUSE`], and a
//! look clears what a dead daemon's rebuild left ([`clear_leftovers`]).

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use view::store::SqlStore;

use crate::Registry;
use crate::engine::WorkspaceEngine;

/// The drawer subdirectory a rebuild writes its file in.
pub(crate) const COMPACT_DIR: &str = "sql-compact";

/// How long a failed rebuild leaves its cache out of the looks, so a rebuild
/// that cannot land is not paid for again every reap interval.
const FAILED_PAUSE: Duration = Duration::from_hours(1);

/// The daemon's compaction state, held by the registry.
#[derive(Debug, Default)]
pub(crate) struct State {
    /// Set while a look or a rebuild runs: one at a time per daemon.
    running: AtomicBool,
    /// The caches a failed rebuild left out of the looks, each with the Unix
    /// second it comes back.
    paused: Mutex<HashMap<PathBuf, u64>>,
}

impl State {
    fn pause(&self, ws: &Path) {
        self.paused
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(ws.to_path_buf(), now_secs() + FAILED_PAUSE.as_secs());
    }

    fn is_paused(&self, ws: &Path) -> bool {
        let now = now_secs();
        let mut paused = self.paused.lock().unwrap_or_else(PoisonError::into_inner);
        paused.retain(|_, until| *until > now);
        paused.contains_key(ws)
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// How long one swap attempt holds the store's mutex waiting for the reads
/// open on the old instance to end: well under the 8 s a `sql` call waits
/// for that mutex before refusing `lock_timeout`.
const SWAP_WAIT: Duration = Duration::from_secs(2);

/// The pause between swap attempts, the mutex released.
const SWAP_RETRY: Duration = Duration::from_secs(1);

/// How long a built file waits for its swap before it is dropped.
const SWAP_GIVE_UP: Duration = Duration::from_mins(10);

/// Appends after the cold build that bring the file to the newest
/// generation before the swap; whatever moves after the last one, the next
/// `sql` call's append carries.
const CATCH_UP: usize = 3;

/// One cache's history and what its latest views serve, in frontmatter rows.
const COUNT_SQL: &str =
    "SELECT (SELECT count(*) FROM hist.frontmatter), (SELECT count(*) FROM main.frontmatter)";

/// Whether a cache whose `hist.frontmatter` holds `hist` rows, of which its
/// latest view serves `live`, is due a rebuild at `multiple`.
pub(crate) fn due(hist: u64, live: u64, multiple: u64) -> bool {
    live > 0 && hist >= live.saturating_mul(multiple)
}

/// `(hist, live)` frontmatter rows of `store`, read on a read of its own so
/// no append waits on the count.
///
/// # Errors
/// The read could not open, or the count failed.
pub(crate) fn frontmatter_rows(store: &Arc<Mutex<SqlStore>>) -> Result<(u64, u64), String> {
    let read = store
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .begin_held_read(store)
        .map_err(|e| e.to_string())?;
    let (_, rows) = read.run(COUNT_SQL).map_err(|e| e.to_string())??;
    let count = |i: usize| rows.first().and_then(|row| row.get(i)?.as_u64());
    match (count(0), count(1)) {
        (Some(hist), Some(live)) => Ok((hist, live)),
        _ => Err(format!("unexpected count rows: {rows:?}")),
    }
}

/// The reaper's look, once a reap interval: unless a rebuild is running,
/// pick the open cache most due at `multiple` and rebuild it, on a thread of
/// its own so the reaper stays prompt.
pub(crate) fn tick(registry: &Arc<Registry>, multiple: u64) {
    let state = registry.sql_compaction();
    if state.running.swap(true, Ordering::AcqRel) {
        return;
    }
    let worker = Arc::clone(registry);
    let spawned = thread::Builder::new()
        .name("mrd-sql-compact".to_owned())
        .spawn(move || {
            look(&worker, multiple);
            worker
                .sql_compaction()
                .running
                .store(false, Ordering::Release);
        });
    if let Err(error) = spawned {
        timing::say!("sql compaction: no thread ({error})");
        state.running.store(false, Ordering::Release);
    }
}

/// One look: rebuild the open cache most due at `multiple`, if any; a failed
/// rebuild leaves its cache out of the looks for [`FAILED_PAUSE`].
fn look(registry: &Registry, multiple: u64) {
    clear_leftovers(registry);
    let Some((ws, hist, live)) = most_due(registry, multiple) else {
        return;
    };
    timing::say!(
        "sql compaction: {} holds {hist} frontmatter rows for {live} live; rebuilding",
        ws.display()
    );
    match compact(registry, &ws) {
        Ok(done) => timing::say!(
            "sql compaction: {} rebuilt in {:.1}s, the swap {:.3}s of it, {} bytes",
            ws.display(),
            done.took.as_secs_f64(),
            done.swap.as_secs_f64(),
            done.bytes
        ),
        Err(Stopped::Gone(why)) => {
            timing::say!("sql compaction: {} keeps its cache: {why}", ws.display());
        }
        Err(Stopped::Failed(why)) => {
            timing::say!(
                "sql compaction: {} keeps its cache: {why}; not picked again for {}s",
                ws.display(),
                FAILED_PAUSE.as_secs()
            );
            registry.sql_compaction().pause(&ws);
        }
    }
}

/// Clear what a daemon that died in a rebuild left in an open cache's drawer,
/// and leave that cache out of the looks for [`FAILED_PAUSE`]: a rebuild that
/// took the daemon with it is not started again as soon as the daemon is back.
/// A look runs alone ([`State::running`]), so no rebuild owns those files.
fn clear_leftovers(registry: &Registry) {
    for ws in registry.open_sql_workspaces() {
        let dir = registry.drawer(&ws).join(COMPACT_DIR);
        if dir.exists() {
            timing::say!(
                "sql compaction: {} holds an unfinished rebuild; cleared, not picked again for {}s",
                ws.display(),
                FAILED_PAUSE.as_secs()
            );
            let _ = std::fs::remove_dir_all(&dir);
            registry.sql_compaction().pause(&ws);
        }
    }
}

/// The open cache whose history is the largest multiple of its live rows,
/// among those due at `multiple` and not paused, with its counts. Each handle
/// is held for its own count alone, so the rest stay free for a close.
fn most_due(registry: &Registry, multiple: u64) -> Option<(PathBuf, u64, u64)> {
    let state = registry.sql_compaction();
    registry
        .open_sql_workspaces()
        .into_iter()
        .filter(|ws| !state.is_paused(ws))
        .filter_map(|ws| {
            let store = registry.open_sql_store(&ws)?;
            match frontmatter_rows(&store) {
                Ok((hist, live)) => due(hist, live, multiple).then_some((ws, hist, live)),
                Err(why) => {
                    timing::say!("sql compaction: cannot count {}: {why}", ws.display());
                    None
                }
            }
        })
        .max_by(|(_, h1, l1), (_, h2, l2)| {
            (u128::from(*h1) * u128::from(*l2)).cmp(&(u128::from(*h2) * u128::from(*l1)))
        })
}

/// A finished rebuild.
#[derive(Debug)]
pub(crate) struct Compacted {
    /// From the start of the build to the swap's end.
    pub(crate) took: Duration,
    /// The swap alone, the handle map's lock held.
    pub(crate) swap: Duration,
    /// The swapped-in file's size.
    pub(crate) bytes: u64,
}

/// Why a rebuild did not land. Either way the old file still serves.
#[derive(Debug)]
pub(crate) enum Stopped {
    /// The workspace left the daemon, unregistered or no longer warm: nothing
    /// failed, and a look may pick the cache again once it is back.
    Gone(&'static str),
    /// The rebuild failed or could not land: [`FAILED_PAUSE`].
    Failed(String),
}

impl fmt::Display for Stopped {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gone(why) => f.write_str(why),
            Self::Failed(why) => f.write_str(why),
        }
    }
}

/// Rebuild `ws`'s open sql cache and swap it in. Synchronous: [`tick`] runs
/// it on its own thread; tests call it directly.
///
/// # Errors
/// Why the old file still serves ([`Stopped`]). Nothing of the attempt is left
/// in the drawer.
pub(crate) fn compact(registry: &Registry, ws: &Path) -> Result<Compacted, Stopped> {
    if registry.open_sql_store(ws).is_none() {
        return Err(Stopped::Gone("the sql cache is not open"));
    }
    let drawer = registry.drawer(ws);
    let dir = drawer.join(COMPACT_DIR);
    // What a failed rebuild, or a daemon that died in one, left behind.
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)
        .map_err(|e| Stopped::Failed(format!("cannot make {}: {e}", dir.display())))?;
    let file = dir.join(view::store::SQL_CACHE_FILENAME);
    let started = Instant::now();
    let swapped = build(registry, ws, &file).and_then(|()| swap(registry, ws, &file));
    let _ = std::fs::remove_dir_all(&dir);
    let swap_took = swapped?;
    let bytes = std::fs::metadata(drawer.join(view::store::SQL_CACHE_FILENAME))
        .map_or(0, |meta| meta.len());
    Ok(Compacted {
        took: started.elapsed(),
        swap: swap_took,
        bytes,
    })
}

/// `ws`'s engine, while the daemon still serves it. Checked before each step,
/// so a workspace that left is neither appended for nor reopened by the swap.
fn still_served(registry: &Registry, ws: &Path) -> Result<Arc<WorkspaceEngine>, Stopped> {
    if !registry.is_registered(ws) {
        return Err(Stopped::Gone("the workspace was unregistered"));
    }
    registry
        .engine_snapshot(ws)
        .ok_or(Stopped::Gone("the workspace is no longer warm"))
}

/// Cold-build `file` at the newest generation, then catch up, then close it.
fn build(registry: &Registry, ws: &Path, file: &Path) -> Result<(), Stopped> {
    let mut built = SqlStore::open_limited(file, registry.sql_cache_limit())
        .map_err(|e| Stopped::Failed(format!("cannot open the rebuild file: {e}")))?;
    let mut at = None;
    for _ in 0..=CATCH_UP {
        let engine = still_served(registry, ws)?;
        if at.as_deref() == Some(engine.at_fingerprint.0.as_str()) {
            break;
        }
        crate::sql_op::with_appender(registry, ws, &engine, |append| {
            crate::sql_op::append_escalating(registry, &mut built, append)
        })
        .map_err(|e| Stopped::Failed(format!("the rebuild's append failed: {}", describe(&e))))?;
        at = Some(engine.at_fingerprint.0.clone());
        #[cfg(test)]
        if let Some(hook) = AFTER_COLD_BUILD.take() {
            hook().map_err(Stopped::Failed)?;
        }
    }
    drop(built);
    if std::fs::exists(format!("{}.wal", file.display())).unwrap_or(true) {
        return Err(Stopped::Failed(
            "the rebuild file did not close cleanly".to_owned(),
        ));
    }
    Ok(())
}

/// Swap `file` in for the handle open now, opening the cache if a
/// least-recently-used close took it during the build. Each attempt takes the
/// handle afresh and lets it go, so nothing keeps an instance open past its
/// reads. The reads open on that instance drain under its mutex, then the
/// rename and reopen run under the handle map's lock as well, so the handle
/// cannot be replaced, nor the file opened a second time, in between. Returns
/// how long the swap itself took.
fn swap(registry: &Registry, ws: &Path, file: &Path) -> Result<Duration, Stopped> {
    let give_up = Instant::now() + SWAP_GIVE_UP;
    loop {
        still_served(registry, ws)?;
        let live = registry
            .sql_store(ws)
            .map_err(|e| Stopped::Failed(format!("cannot open the sql cache: {e}")))?;
        let mut store = live.lock().unwrap_or_else(PoisonError::into_inner);
        if store.wait_for_reads(SWAP_WAIT) {
            let started = Instant::now();
            return match registry.while_open_sql_store(ws, &live, || store.swap_in(file)) {
                None => Err(Stopped::Gone("the sql cache closed as the swap began")),
                Some(Err(e)) => Err(Stopped::Failed(format!("the swap failed: {e}"))),
                Some(Ok(())) => Ok(started.elapsed()),
            };
        }
        drop(store);
        drop(live);
        if Instant::now() >= give_up {
            return Err(Stopped::Failed(format!(
                "reads kept the old cache open past {}s",
                SWAP_GIVE_UP.as_secs()
            )));
        }
        thread::sleep(SWAP_RETRY);
    }
}

fn describe(error: &wire::ErrorBody) -> String {
    let mut text = format!("{:?}", error.code);
    for part in [&error.message, &error.cause].into_iter().flatten() {
        text.push_str(": ");
        text.push_str(part);
    }
    text
}

#[cfg(test)]
/// What a test runs inside a rebuild; an `Err` fails the rebuild there.
pub(crate) type RebuildHook = Box<dyn FnOnce() -> Result<(), String>>;

#[cfg(test)]
thread_local! {
    /// Test-only, one shot: runs on the rebuild's thread right after its cold
    /// build, before the catch-up and the swap.
    pub(crate) static AFTER_COLD_BUILD: std::cell::Cell<Option<RebuildHook>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;

    use super::*;
    use crate::state::StateStore;

    fn registry_in(home: &Path) -> Arc<Registry> {
        let cache_root = home.join("cache");
        fs::create_dir_all(&cache_root).unwrap();
        Arc::new(Registry::new(
            StateStore::new(home.join("state.json")),
            cache_root,
            Vec::new(),
        ))
    }

    /// The card at its `i`-th edit; each edit a different length, so no
    /// stat signature mistakes one for the last.
    fn card(i: usize) -> String {
        format!(
            "---\ntype: task\nstatus: s{i}\nowner: w1\nsession: s\n---\n# C\n- [ ] do #t\nbody [[a]] {}\n",
            "v".repeat(i + 1)
        )
    }

    /// The rows `query` answers on `ws` through the served door.
    fn rows(reg: &Registry, ws: &Path, query: &str) -> Vec<Vec<serde_json::Value>> {
        match crate::sql_op::serve(reg, ws, query) {
            Ok(wire::ResponseBody::Sql {
                rows, error: None, ..
            }) => rows,
            other => panic!("{query}: {other:?}"),
        }
    }

    /// What callers read: every latest view, whole.
    const SURFACE: [&str; 18] = [
        "doc",
        "frontmatter",
        "section",
        "link",
        "frontmatter_link",
        "lock",
        "tag",
        "frontmatter_tag",
        "frontmatter_fault",
        "task",
        "body",
        "base",
        "base_view",
        "base_formula",
        "backlink",
        "dangling",
        "record",
        "tag_all",
    ];

    fn surface(reg: &Registry, ws: &Path) -> Vec<Vec<Vec<serde_json::Value>>> {
        SURFACE
            .iter()
            .map(|view| rows(reg, ws, &format!("SELECT * FROM {view} ORDER BY ALL")))
            .collect()
    }

    /// A warm workspace `name` whose card was edited `edits` times, each edit
    /// served, so its cache holds `edits + 1` card versions.
    fn churned_named(reg: &Registry, home: &Path, name: &str, edits: usize) -> PathBuf {
        let ws = home.join(name);
        fs::create_dir_all(ws.join("cards")).unwrap();
        fs::create_dir_all(ws.join("bases")).unwrap();
        fs::write(ws.join("a.md"), "---\ntitle: A\n---\n# A\nsee [[b]]\n").unwrap();
        fs::write(ws.join("b.md"), "# B\nsee [[a]] and [[cards/c]] [[gone]]\n").unwrap();
        fs::write(
            ws.join("bases/x.base"),
            "views:\n  - type: table\n    name: T\nformulas:\n  f: \"1\"\n",
        )
        .unwrap();
        fs::write(ws.join("cards/c.md"), card(0)).unwrap();
        let ws = workspace::canonicalize(&ws).unwrap();
        reg.register(&ws);
        reg.warm_or_build(&ws).unwrap();
        rows(reg, &ws, "SELECT 1");
        for i in 1..=edits {
            fs::write(ws.join("cards/c.md"), card(i)).unwrap();
            reg.warm_or_build(&ws).unwrap();
            assert_eq!(
                rows(reg, &ws, "SELECT status FROM record WHERE type = 'task'"),
                vec![vec![serde_json::json!(format!("s{i}"))]],
                "edit {i} reached the cache"
            );
        }
        ws
    }

    fn churned(reg: &Registry, home: &Path, edits: usize) -> PathBuf {
        churned_named(reg, home, "ws", edits)
    }

    fn store_of(reg: &Registry, ws: &Path) -> Arc<Mutex<SqlStore>> {
        reg.open_sql_store(ws).expect("the cache is open")
    }

    fn counts(reg: &Registry, ws: &Path) -> (u64, u64) {
        frontmatter_rows(&store_of(reg, ws)).unwrap()
    }

    fn hook(run: impl FnOnce() -> Result<(), String> + 'static) {
        AFTER_COLD_BUILD.set(Some(Box::new(run)));
    }

    #[test]
    fn a_cache_is_due_from_exactly_the_multiple_of_its_live_rows() {
        assert!(!due(199, 100, 2));
        assert!(due(200, 100, 2));
        assert!(due(1_157, 108, 10));
        assert!(
            !due(1_000, 0, 2),
            "no live rows: nothing to measure against"
        );
    }

    /// A cache below the multiple is left; one at it is picked, rebuilt to
    /// its live rows alone, answers every view with the same rows, and takes
    /// new edits on.
    #[test]
    fn a_cache_at_the_multiple_is_rebuilt_and_answers_the_same_rows() {
        let home = tempfile::tempdir().unwrap();
        let reg = registry_in(home.path());
        let ws = churned(&reg, home.path(), 6);
        let (hist, live) = counts(&reg, &ws);
        let multiple = hist / live;
        assert!(multiple >= 2, "{hist} rows for {live}");
        assert!(most_due(&reg, multiple + 1).is_none(), "below it: left");
        assert_eq!(
            most_due(&reg, multiple).map(|(due, ..)| due),
            Some(ws.clone()),
            "at it: picked"
        );
        let before = surface(&reg, &ws);
        assert!(
            !before[SURFACE.iter().position(|v| *v == "base").unwrap()].is_empty(),
            "the fixture has base rows"
        );

        compact(&reg, &ws).expect("rebuilt");

        assert_eq!(counts(&reg, &ws), (live, live));
        assert_eq!(surface(&reg, &ws), before, "the same rows, view by view");
        assert!(!reg.drawer(&ws).join(COMPACT_DIR).exists());
        assert!(most_due(&reg, 2).is_none());
        fs::write(ws.join("cards/c.md"), card(7)).unwrap();
        reg.warm_or_build(&ws).unwrap();
        assert_eq!(
            rows(&reg, &ws, "SELECT status FROM record WHERE type = 'task'"),
            vec![vec![serde_json::json!("s7")]]
        );
    }

    /// A rebuild that fails before its swap leaves the old cache serving,
    /// whole, and nothing in the drawer; what a daemon that died mid-rebuild
    /// left there, the next rebuild clears, and lands.
    #[test]
    fn a_rebuild_that_fails_before_its_swap_leaves_the_old_cache_serving() {
        let home = tempfile::tempdir().unwrap();
        let reg = registry_in(home.path());
        let ws = churned(&reg, home.path(), 6);
        let history = counts(&reg, &ws);
        let before = surface(&reg, &ws);

        hook(|| Err("test fault".to_owned()));
        let failed = compact(&reg, &ws).expect_err("the fault fails it");
        assert!(matches!(failed, Stopped::Failed(_)), "{failed}");
        assert_eq!(counts(&reg, &ws), history, "the old file, whole");
        assert_eq!(surface(&reg, &ws), before);
        let dir = reg.drawer(&ws).join(COMPACT_DIR);
        assert!(!dir.exists());

        fs::create_dir_all(dir.join("sql-spill")).unwrap();
        fs::write(dir.join(view::store::SQL_CACHE_FILENAME), b"torn").unwrap();
        compact(&reg, &ws).expect("the next rebuild lands");
        assert_eq!(counts(&reg, &ws), (history.1, history.1));
        assert_eq!(surface(&reg, &ws), before);
    }

    /// Mid-rebuild, with the new file written, a query answers from the old
    /// file whole; a read opened there keeps the old file across the swap's
    /// wait, and the swap lands once it ends. Queries running all through
    /// answer the same and none is refused.
    #[test]
    fn queries_through_a_rebuild_and_its_swap_answer_whole() {
        let home = tempfile::tempdir().unwrap();
        let reg = registry_in(home.path());
        let ws = churned(&reg, home.path(), 6);
        let before = surface(&reg, &ws);
        let (hist, live) = counts(&reg, &ws);
        let (read_done, read_answer) = std::sync::mpsc::channel();
        let mid_rebuild = {
            let (reg, ws, before) = (Arc::clone(&reg), ws.clone(), before.clone());
            move || {
                let built = reg
                    .drawer(&ws)
                    .join(COMPACT_DIR)
                    .join(view::store::SQL_CACHE_FILENAME);
                assert!(built.exists(), "the new file stands");
                assert_eq!(surface(&reg, &ws), before, "mid-rebuild");
                assert_eq!(counts(&reg, &ws), (hist, live), "the old file, whole");
                let store = store_of(&reg, &ws);
                let read = store
                    .lock()
                    .unwrap()
                    .begin_held_read(&store)
                    .map_err(|e| e.to_string())?;
                thread::spawn(move || {
                    thread::sleep(Duration::from_millis(500));
                    let answer = read.run("SELECT count(*) FROM hist.frontmatter");
                    let _ = read_done.send(answer.unwrap().unwrap().1);
                });
                Ok(())
            }
        };
        hook(mid_rebuild);

        let done = AtomicBool::new(false);
        let served = AtomicUsize::new(0);
        // Ends the queries if a panic below leaves `done` unset.
        let deadline = Instant::now() + Duration::from_mins(5);
        thread::scope(|scope| {
            let queries = scope.spawn(|| {
                while !done.load(Ordering::Acquire) && Instant::now() < deadline {
                    assert_eq!(surface(&reg, &ws), before, "through the rebuild");
                    served.fetch_add(1, Ordering::Relaxed);
                }
            });
            let rebuilt = compact(&reg, &ws);
            // At least one answer from the swapped-in file.
            let at_swap = served.load(Ordering::Relaxed);
            while served.load(Ordering::Relaxed) == at_swap && !queries.is_finished() {
                thread::yield_now();
            }
            done.store(true, Ordering::Release);
            queries.join().expect("no query failed or was refused");
            rebuilt.expect("rebuilt");
        });
        assert_eq!(
            read_answer.recv().unwrap(),
            vec![vec![serde_json::json!(hist)]],
            "the read open across the swap's wait answered from the old file"
        );
        assert_eq!(counts(&reg, &ws), (live, live));
    }

    /// An edit published during the build reaches the new file through the
    /// catch-up, before the swap: read here with no call's append between.
    #[test]
    fn an_edit_during_the_build_is_caught_up_before_the_swap() {
        let home = tempfile::tempdir().unwrap();
        let reg = registry_in(home.path());
        let ws = churned(&reg, home.path(), 6);
        let (_, live) = counts(&reg, &ws);
        let (hooked_reg, hooked_ws) = (Arc::clone(&reg), ws.clone());
        hook(move || {
            fs::write(hooked_ws.join("cards/c.md"), card(7)).unwrap();
            hooked_reg.warm_or_build(&hooked_ws).unwrap();
            Ok(())
        });

        compact(&reg, &ws).expect("rebuilt");

        let (hist, after) = counts(&reg, &ws);
        assert_eq!(after, live);
        assert_eq!(hist, live + 4, "the card's edit appended on the new file");
        assert_eq!(
            rows(&reg, &ws, "SELECT status FROM record WHERE type = 'task'"),
            vec![vec![serde_json::json!("s7")]]
        );
    }

    /// The rebuild holds no handle, so a reap during it closes the old
    /// instance; the swap lands on what is open then — a handle a query
    /// reopened, or none, when it opens the cache itself.
    #[test]
    fn a_rebuild_holds_no_handle_and_lands_on_what_is_open_at_its_swap() {
        for query_reopens in [true, false] {
            let home = tempfile::tempdir().unwrap();
            let reg = registry_in(home.path());
            let ws = churned(&reg, home.path(), 6);
            let before = surface(&reg, &ws);
            let (_, live) = counts(&reg, &ws);
            let (hooked_reg, hooked_ws) = (Arc::clone(&reg), ws.clone());
            hook(move || {
                let (reg, ws) = (hooked_reg, hooked_ws);
                assert_eq!(
                    Arc::strong_count(&store_of(&reg, &ws)),
                    2,
                    "the handle map's and this one: the rebuild holds none"
                );
                reg.reap(u64::MAX, 0);
                assert!(reg.open_sql_store(&ws).is_none(), "the reap closed it");
                reg.warm_or_build(&ws).unwrap();
                if query_reopens {
                    rows(&reg, &ws, "SELECT 1");
                }
                Ok(())
            });

            compact(&reg, &ws).expect("landed");

            assert_eq!(counts(&reg, &ws), (live, live), "reopen: {query_reopens}");
            assert_eq!(surface(&reg, &ws), before, "reopen: {query_reopens}");
        }
    }

    /// A workspace that leaves the daemon during its rebuild keeps its old
    /// file and costs no pause: nothing failed.
    #[test]
    fn a_workspace_that_leaves_during_its_rebuild_keeps_its_cache_with_no_pause() {
        let home = tempfile::tempdir().unwrap();
        let reg = registry_in(home.path());
        let ws = churned(&reg, home.path(), 6);
        let before = surface(&reg, &ws);
        let history = counts(&reg, &ws);
        let hooked_reg = Arc::clone(&reg);
        hook(move || {
            hooked_reg.reap(u64::MAX, 0);
            Ok(())
        });

        look(&reg, 2);

        assert!(!reg.sql_compaction().is_paused(&ws));
        assert!(!reg.drawer(&ws).join(COMPACT_DIR).exists());
        reg.warm_or_build(&ws).unwrap();
        assert_eq!(surface(&reg, &ws), before);
        assert_eq!(counts(&reg, &ws), history, "the old file, whole");
    }

    /// An unregister during a rebuild ends it: nothing reopens the cache of
    /// a workspace the daemon no longer serves, though its engine stays.
    #[test]
    fn a_workspace_unregistered_during_its_rebuild_gets_no_handle_back() {
        let home = tempfile::tempdir().unwrap();
        let reg = registry_in(home.path());
        let ws = churned(&reg, home.path(), 6);
        let (hooked_reg, hooked_ws) = (Arc::clone(&reg), ws.clone());
        hook(move || {
            assert!(hooked_reg.unregister(&hooked_ws));
            Ok(())
        });

        look(&reg, 2);

        assert!(reg.engine_snapshot(&ws).is_some(), "its engine stays");
        assert!(reg.open_sql_store(&ws).is_none(), "no handle came back");
        assert!(!reg.sql_compaction().is_paused(&ws));
        assert!(!reg.drawer(&ws).join(COMPACT_DIR).exists());
    }

    /// A failed rebuild leaves its own cache out of the looks and no other;
    /// the pause over, it is picked again.
    #[test]
    fn a_failed_rebuild_pauses_its_own_cache_alone() {
        let home = tempfile::tempdir().unwrap();
        let reg = registry_in(home.path());
        let bad = churned_named(&reg, home.path(), "bad", 8);
        let good = churned_named(&reg, home.path(), "good", 4);
        let bad_history = counts(&reg, &bad);
        let (_, good_live) = counts(&reg, &good);

        hook(|| Err("test fault".to_owned()));
        look(&reg, 2);
        assert_eq!(counts(&reg, &bad), bad_history, "most due, and it failed");
        assert!(reg.sql_compaction().is_paused(&bad));

        look(&reg, 2);
        assert_eq!(counts(&reg, &good), (good_live, good_live));
        assert_eq!(counts(&reg, &bad), bad_history, "still out of the looks");
        assert!(most_due(&reg, 2).is_none());

        reg.sql_compaction().paused.lock().unwrap().clear();
        look(&reg, 2);
        assert_eq!(counts(&reg, &bad), (bad_history.1, bad_history.1));
    }

    /// What a dead daemon's rebuild left is cleared by a look, whichever cache
    /// it picks, and that cache sits out the pause.
    #[test]
    fn a_look_clears_what_a_dead_rebuild_left_and_pauses_that_cache() {
        let home = tempfile::tempdir().unwrap();
        let reg = registry_in(home.path());
        let ws = churned(&reg, home.path(), 6);
        let history = counts(&reg, &ws);
        let dir = reg.drawer(&ws).join(COMPACT_DIR);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(view::store::SQL_CACHE_FILENAME), b"torn").unwrap();

        look(&reg, 2);

        assert!(!dir.exists());
        assert!(reg.sql_compaction().is_paused(&ws));
        assert_eq!(counts(&reg, &ws), history, "no rebuild in that look");
    }

    /// The reaper's tick rebuilds a due cache on its own thread.
    #[test]
    fn the_tick_rebuilds_a_due_cache_on_its_own_thread() {
        let home = tempfile::tempdir().unwrap();
        let reg = registry_in(home.path());
        let ws = churned(&reg, home.path(), 6);
        let before = surface(&reg, &ws);
        let (_, live) = counts(&reg, &ws);

        tick(&reg, 2);

        let deadline = Instant::now() + Duration::from_mins(2);
        while reg.sql_compaction().running.load(Ordering::Acquire) {
            assert!(Instant::now() < deadline, "the rebuild never ended");
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(counts(&reg, &ws), (live, live));
        assert_eq!(surface(&reg, &ws), before);
    }
}
