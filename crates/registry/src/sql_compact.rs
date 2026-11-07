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
//! generations published meanwhile. It holds no store mutex: `sql` calls keep
//! answering from the old file throughout. The swap
//! ([`view::store::SqlStore::swap_in`]) takes the store's mutex, waits for
//! the reads open on the old instance to end, and renames the new file over
//! the old one. Whatever fails before the rename — the build, the catch-up,
//! the daemon itself — leaves the old file serving, and the look pauses for
//! [`FAILED_PAUSE`]; the next rebuild clears what a failed one left in
//! [`COMPACT_DIR`].

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use view::store::SqlStore;

use crate::Registry;

/// The drawer subdirectory a rebuild writes its file in.
pub(crate) const COMPACT_DIR: &str = "sql-compact";

/// How long the reaper's look pauses after a failed rebuild, so a rebuild
/// that cannot land is not paid for again every reap interval.
const FAILED_PAUSE: Duration = Duration::from_secs(3600);

/// The daemon's compaction state, held by the registry.
#[derive(Debug, Default)]
pub(crate) struct State {
    /// Set while a look or a rebuild runs: one at a time per daemon.
    running: AtomicBool,
    /// Unix seconds before which the look does not run: a failed rebuild's
    /// [`FAILED_PAUSE`]; `0` never set.
    resume_at: AtomicU64,
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
const SWAP_GIVE_UP: Duration = Duration::from_secs(600);

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
    if now_secs() < state.resume_at.load(Ordering::Acquire)
        || state.running.swap(true, Ordering::AcqRel)
    {
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

/// One look: rebuild the open cache most due at `multiple`, if any; after a
/// failed rebuild, pause the looks for [`FAILED_PAUSE`].
fn look(registry: &Registry, multiple: u64) {
    let Some((ws, hist, live)) = most_due(registry, multiple) else {
        return;
    };
    timing::say!(
        "sql compaction: {} holds {hist} frontmatter rows for {live} live; rebuilding",
        ws.display()
    );
    match compact(registry, &ws) {
        Ok(done) => timing::say!(
            "sql compaction: {} rebuilt in {:.1}s, {} bytes",
            ws.display(),
            done.took.as_secs_f64(),
            done.bytes
        ),
        Err(why) => {
            timing::say!(
                "sql compaction: {} keeps its cache: {why}; next look in {}s",
                ws.display(),
                FAILED_PAUSE.as_secs()
            );
            registry
                .sql_compaction()
                .resume_at
                .store(now_secs() + FAILED_PAUSE.as_secs(), Ordering::Release);
        }
    }
}

/// The open cache whose history is the largest multiple of its live rows,
/// among those due at `multiple`, with its counts.
fn most_due(registry: &Registry, multiple: u64) -> Option<(PathBuf, u64, u64)> {
    registry
        .open_sql_stores()
        .into_iter()
        .filter_map(|(ws, store)| match frontmatter_rows(&store) {
            Ok((hist, live)) => due(hist, live, multiple).then_some((ws, hist, live)),
            Err(why) => {
                timing::say!("sql compaction: cannot count {}: {why}", ws.display());
                None
            }
        })
        .max_by(|(_, h1, l1), (_, h2, l2)| {
            (u128::from(*h1) * u128::from(*l2)).cmp(&(u128::from(*h2) * u128::from(*l1)))
        })
}

/// A finished rebuild.
#[derive(Debug)]
pub(crate) struct Compacted {
    /// From the start of the build to the swap.
    pub(crate) took: Duration,
    /// The swapped-in file's size.
    pub(crate) bytes: u64,
}

/// Rebuild `ws`'s open sql cache and swap it in. Synchronous: [`tick`] runs
/// it on its own thread; tests call it directly.
///
/// # Errors
/// Why the old file still serves: the cache is not open or the workspace not
/// warm, the build or a catch-up append failed, or the swap could not take
/// place. Nothing of the attempt is left in the drawer.
pub(crate) fn compact(registry: &Registry, ws: &Path) -> Result<Compacted, String> {
    let live = registry
        .open_sql_stores()
        .into_iter()
        .find_map(|(open, store)| (open == ws).then_some(store))
        .ok_or("the sql cache is not open")?;
    let dir = registry.drawer(ws).join(COMPACT_DIR);
    // What a failed rebuild, or a daemon that died in one, left behind.
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot make {}: {e}", dir.display()))?;
    let file = dir.join(view::store::SQL_CACHE_FILENAME);
    let started = Instant::now();
    let swapped = build(registry, ws, &file).and_then(|()| swap(registry, ws, &live, &file));
    let _ = std::fs::remove_dir_all(&dir);
    swapped?;
    let bytes = std::fs::metadata(live.lock().unwrap_or_else(PoisonError::into_inner).file())
        .map_or(0, |meta| meta.len());
    Ok(Compacted {
        took: started.elapsed(),
        bytes,
    })
}

/// Cold-build `file` at the newest generation, then catch up, then close it.
fn build(registry: &Registry, ws: &Path, file: &Path) -> Result<(), String> {
    let mut built = SqlStore::open_limited(file, registry.sql_cache_limit())
        .map_err(|e| format!("cannot open the rebuild file: {e}"))?;
    let mut at = None;
    for _ in 0..=CATCH_UP {
        let engine = registry
            .engine_snapshot(ws)
            .ok_or("the workspace is not warm")?;
        if at.as_deref() == Some(engine.at_fingerprint.0.as_str()) {
            break;
        }
        crate::sql_op::with_appender(registry, ws, &engine, |append| {
            crate::sql_op::append_escalating(registry, &mut built, append)
        })
        .map_err(|e| format!("the rebuild's append failed: {}", describe(&e)))?;
        at = Some(engine.at_fingerprint.0.clone());
        #[cfg(test)]
        if FAIL_AFTER_BUILD.with(std::cell::Cell::take) {
            return Err("test fault after the cold build".to_owned());
        }
    }
    drop(built);
    if std::fs::exists(format!("{}.wal", file.display())).unwrap_or(true) {
        return Err("the rebuild file did not close cleanly".to_owned());
    }
    Ok(())
}

/// Swap `file` in for `live`, which must still be `ws`'s open handle: the
/// reads open on the old instance drain under its mutex, then the rename and
/// reopen run under the handle map's lock as well, so the handle cannot be
/// replaced, nor the file opened a second time, in between.
fn swap(
    registry: &Registry,
    ws: &Path,
    live: &Arc<Mutex<SqlStore>>,
    file: &Path,
) -> Result<(), String> {
    let give_up = Instant::now() + SWAP_GIVE_UP;
    loop {
        let mut store = live.lock().unwrap_or_else(PoisonError::into_inner);
        if store.wait_for_reads(SWAP_WAIT) {
            return match registry.while_open_sql_store(ws, live, || store.swap_in(file)) {
                None => Err("the sql cache was closed during the rebuild".to_owned()),
                Some(swapped) => swapped.map_err(|e| format!("the swap failed: {e}")),
            };
        }
        drop(store);
        if Instant::now() >= give_up {
            return Err(format!(
                "reads kept the old cache open past {}s",
                SWAP_GIVE_UP.as_secs()
            ));
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
thread_local! {
    /// Test-only, one shot: the next rebuild on this thread fails right
    /// after its cold build, before the swap.
    pub(crate) static FAIL_AFTER_BUILD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

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
            "---\ntype: task\nstatus: s{i}\nowner: w1\nsession: s\n---\n# C\nbody [[a]] {}\n",
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

    /// What callers read, view by view.
    const SURFACE: [&str; 5] = [
        "SELECT path, file_rev FROM doc ORDER BY ALL",
        "SELECT path, key, value, node_rev FROM frontmatter ORDER BY ALL",
        "SELECT src_path, seq, target_raw, dest_path FROM link ORDER BY ALL",
        "SELECT path, hpath, heading FROM section ORDER BY ALL",
        "SELECT path, type, status, owner FROM record ORDER BY ALL",
    ];

    fn surface(reg: &Registry, ws: &Path) -> Vec<Vec<Vec<serde_json::Value>>> {
        SURFACE.iter().map(|query| rows(reg, ws, query)).collect()
    }

    /// A warm workspace whose card was edited `edits` times, each edit
    /// served, so its cache holds `edits + 1` card versions.
    fn churned(reg: &Registry, home: &Path, edits: usize) -> PathBuf {
        let ws = home.join("ws");
        fs::create_dir_all(ws.join("cards")).unwrap();
        fs::write(ws.join("a.md"), "---\ntitle: A\n---\n# A\nsee [[b]]\n").unwrap();
        fs::write(ws.join("b.md"), "# B\nsee [[a]] and [[cards/c]]\n").unwrap();
        fs::write(ws.join("cards/c.md"), card(0)).unwrap();
        let ws = workspace::canonicalize(&ws).unwrap();
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

    fn store_of(reg: &Registry, ws: &Path) -> Arc<Mutex<SqlStore>> {
        reg.open_sql_stores()
            .into_iter()
            .find_map(|(open, store)| (open == ws).then_some(store))
            .expect("the cache is open")
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
    /// its live rows alone, answers every query with the same rows, and
    /// takes new edits on.
    #[test]
    fn a_cache_at_the_multiple_is_rebuilt_and_answers_the_same_rows() {
        let home = tempfile::tempdir().unwrap();
        let reg = registry_in(home.path());
        let ws = churned(&reg, home.path(), 6);
        let store = store_of(&reg, &ws);
        let (hist, live) = frontmatter_rows(&store).unwrap();
        let multiple = hist / live;
        assert!(multiple >= 2, "{hist} rows for {live}");
        assert!(most_due(&reg, multiple + 1).is_none(), "below it: left");
        assert_eq!(
            most_due(&reg, multiple).map(|(due, ..)| due),
            Some(ws.clone()),
            "at it: picked"
        );
        let before = surface(&reg, &ws);

        compact(&reg, &ws).expect("rebuilt");

        assert_eq!(frontmatter_rows(&store).unwrap(), (live, live));
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
        let store = store_of(&reg, &ws);
        let history = frontmatter_rows(&store).unwrap();
        let before = surface(&reg, &ws);

        FAIL_AFTER_BUILD.with(|fail| fail.set(true));
        let failed = compact(&reg, &ws).expect_err("the fault fails it");
        assert!(failed.contains("test fault"), "{failed}");
        assert_eq!(
            frontmatter_rows(&store).unwrap(),
            history,
            "the old file, whole"
        );
        assert_eq!(surface(&reg, &ws), before);
        let dir = reg.drawer(&ws).join(COMPACT_DIR);
        assert!(!dir.exists());

        fs::create_dir_all(dir.join("sql-spill")).unwrap();
        fs::write(dir.join(view::store::SQL_CACHE_FILENAME), b"torn").unwrap();
        compact(&reg, &ws).expect("the next rebuild lands");
        assert_eq!(frontmatter_rows(&store).unwrap(), (history.1, history.1));
        assert_eq!(surface(&reg, &ws), before);
    }

    /// Queries running all through a rebuild and its swap each answer whole
    /// and the same, none refused.
    #[test]
    fn queries_through_a_rebuild_and_its_swap_answer_whole() {
        let home = tempfile::tempdir().unwrap();
        let reg = registry_in(home.path());
        let ws = churned(&reg, home.path(), 6);
        let before = surface(&reg, &ws);
        let done = AtomicBool::new(false);
        let served = std::thread::scope(|scope| {
            let queries = scope.spawn(|| {
                let mut served = 0usize;
                while !done.load(Ordering::Acquire) {
                    assert_eq!(surface(&reg, &ws), before, "answer {served}");
                    served += 1;
                }
                served
            });
            compact(&reg, &ws).expect("rebuilt");
            // A few answers from the swapped-in file too.
            std::thread::sleep(Duration::from_millis(200));
            done.store(true, Ordering::Release);
            queries.join().expect("no query failed")
        });
        assert!(served > 1, "the queries ran through it: {served}");
        let (hist, live) = frontmatter_rows(&store_of(&reg, &ws)).unwrap();
        assert_eq!(hist, live);
    }
}
