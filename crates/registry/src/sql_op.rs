//! The `sql` op (`docs/wire-contract.md` § A.11): one SQL statement over the
//! workspace's fingerprint-pinned projection cache, served by the resident
//! engine — the ruled lifecycle B (2026-08-14), which knowingly supersedes
//! §10.4's no-view-organ close for sql. The wire carries results, never a
//! file path: the daemon is the cache file's single owner (`DuckDB`'s own
//! lock excludes every other process), and this module is the one append
//! actor per workspace.
//!
//! Serve shape per call: warm engine snapshot → pre-query pin check + delta
//! append ([`view::store::SqlStore::sync`] — O(changed files), the watcher
//! grain rides the same path later) → always-rollback query (one execution
//! path, the NO-SANDBOX ruling 2026-08-14) → post-result currency pass
//! through the workspace's leaf memo, so the answer's freshness state
//! post-dates its rows (§Q3 honest tense).
//!
//! **Where the workspace's store mutex opens and closes.** It covers the
//! append and the opening of this call's read — [`view::store::SqlStore::sync`]
//! then [`view::store::SqlStore::begin_read`] — and it is released before the
//! caller's statement runs. The read owns a connection of its own on the same
//! `DuckDB` instance, with its snapshot pinned by the `BEGIN` taken under the
//! lock, so the answer is as fresh as the `as_of` it reports while the
//! expensive part costs concurrent callers nothing but `DuckDB`'s MVCC.
//! Queries on one workspace therefore overlap, across connections and across
//! seats. What still serializes is the append, which is the one thing that
//! genuinely needs a single actor.

use std::path::Path;
use std::path::Path as StdPath;
use std::sync::{Mutex, MutexGuard, TryLockError};
use std::thread;
use std::time::{Duration, Instant};

use wire::{ErrorBody, ErrorCode, ResponseBody, SqlCol};

use crate::Registry;

/// How long a `sql` call waits for the workspace's `SqlStore` before refusing
/// `lock_timeout`.
///
/// **What it bounds is the WAIT, never the work.** The holder is not
/// interrupted and its query is not cancelled: a waiter that gives up here
/// leaves a correct in-flight query alone and loses nothing but its own turn.
/// That is what makes the refusal safe to hand out — there is no half-done
/// work to reconcile and no result to discard.
///
/// The value's two edges, and the window between them:
/// - **Floor — the legitimate hold.** One holder occupies the store for
///   `sync()` + [`view::store::SqlStore::begin_read`], because a fingerprint
///   advance makes the next call fold the delta. The floor on record was
///   measured against the OLDER, larger hold that also spanned the caller's
///   query: over a 47,185-file corpus under continuous fleet writes, sampled
///   720 s (n=795), median 378 ms and **max 5,581 ms** (the tail had not
///   converged — the same measurement over 90 s, n=103, peaked at 4,779 ms, so
///   an eightfold longer sample moved it up 17%; the value below is chosen
///   against the larger figure). The hold this bound now guards is a strict
///   subset of what that sampled — an append plus a connection clone — so
///   those figures are an UPPER bound on it and the headroom below is at
///   least what it says. **The floor at the current grain is unmeasured**, and
///   a successor moving this constant should measure it rather than read 378
///   ms and 5,581 ms as if they described today's hold. Both are also
///   WARM-INCREMENTAL figures; the cold-projection hold below is a separate,
///   larger, unbounded case they do not cover.
/// - **Ceiling — the host's own deadline.** `ccc-statusd` gives a request 10 s
///   (`internal/registryclient/client.go`) and retires the CONNECTION when it
///   expires, failing every op pipelined behind it. The refusal has to land
///   with room to spare, or the bound buys nothing: the burst it exists to
///   prevent happens anyway, one layer up. The op's own pre-lock work (domain
///   load, mount corpus, base walk) runs BEFORE this wait, so a refusal costs
///   the caller that plus the bound — measured at 7,628 ms total against a 7 s
///   bound, i.e. ~0.6 s of pre-lock work on a debug build.
///
/// So the usable window is roughly 5.6 s to 9 s, and 8 s sits in it at 1.4×
/// that floor with ~2 s under the deadline. The window being this tight was
/// the finding that named the repair, and the repair is the one this module
/// now implements: the lock is not held across the caller's query, so the work
/// a waiter can queue behind is an append rather than an arbitrary statement.
/// The bound stays because the append is still exclusive and a cold projection
/// build still happens inside it — a brace is still wanted where the hold can
/// still be long.
///
/// **How far these numbers carry.** Both edges are debug-build macOS figures
/// taken under live fleet load, so 1.4× is a debug-build RATIO, not a
/// production margin; a release build moves the floor and the pre-lock work
/// down together, which is the favourable direction but not a measured one. A
/// successor who wants to move this constant should re-measure both edges on
/// the build that ships rather than treat 5,581 ms and 378 ms as given.
///
/// **What the floor does NOT cover: a cold projection cache.** Two caches sit
/// on this path and only one of them is gated. The ENGINE drawer has its own
/// cold gate ahead of this call ([`crate::Registry::cold_gate`], keyed on
/// engine residency alone) which bounds its rebuild at `COLD_BUILD_WAIT`,
/// backgrounds it, and refuses `corpus_warming` — so engine coldness never
/// converts into time held here. The SQL PROJECTION cache is a different
/// artifact with no gate of its own: [`crate::Registry::sql_store`] opens the
/// drawer's `sql.duckdb` on first use, and `sql` is its ONLY opener. A
/// workspace that ordinary `read`/`put` traffic has made engine-warm therefore
/// passes the cold gate, takes this lock, and — when the projection file is
/// absent or is recreated inside `sync()` on a fingerprint version-prefix
/// change — meets an empty manifest
/// and projects the WHOLE corpus, inline, under this mutex, unbounded and not
/// backgrounded. Measured on 10,000 synthetic files (debug, macOS): a
/// warm-engine/cold-projection run costs the same as a fully cold one, ~2 s
/// per 10 k files above warm. Concurrent `sql` on that workspace refuses
/// `lock_timeout` for the duration. Raising this bound is not the answer —
/// giving the projection build its own gate, the way the engine drawer already
/// has one, is.
///
/// **What the floor does NOT cover: a pileup, and it is UNMEASURED.** The
/// floor is the tail of ONE legitimate hold, which is what the bound is
/// justified against. What a waiter actually absorbs is the SUM of the holds
/// ahead of it plus the turns it loses to `try_lock`'s races, and headroom
/// over the floor is 2.4 s — less than one median-plus-tail pileup would
/// consume. No measurement in this record probes that regime: the concurrency
/// that was measured is short-op load, which cannot queue past a deadline
/// whatever the lock discipline. So this is a gap in the evidence, not a
/// clean result, and it is the first place to look if legitimate work ever
/// starts refusing here.
///
/// Taking the caller's query out of the hold shrinks what can pile up — an
/// arbitrary statement no longer occupies the store, an append does — but it
/// does not close that gap, and it must not be read as closing it. A pileup of
/// appends is still a pileup, `try_lock` is still not FIFO, and neither the
/// summed hold nor the lost races has been measured at this grain.
///
/// Engine-internal on purpose, the same as the §3.2 cold gate's
/// `COLD_BUILD_WAIT` beside it: the contract publishes the bound's ORDER,
/// never its value. That privacy is why the near-miss band below asserts
/// against [`crate::server::SLOW_OP_LOG`] here rather than exporting this
/// value to `server`.
const SQL_STORE_WAIT: Duration = Duration::from_secs(8);

/// The near-miss band is a compile-time fact, not a comment.
///
/// The slow-op log exists to name the tail BEFORE it starts refusing, which is
/// only true while it fires strictly under this bound. The two constants live
/// in different modules with nothing but prose between them, so lowering the
/// bound or raising the log would silently invert the band and no test would
/// fail. This is that test, and it costs one line.
const _: () = assert!(
    crate::server::SLOW_OP_LOG.as_millis() < SQL_STORE_WAIT.as_millis(),
    "SLOW_OP_LOG must fire strictly below SQL_STORE_WAIT: as written, a `sql` \
     refused at the bound would be the FIRST thing the slow-op log ever names, \
     and the near-miss warning it exists to give would arrive too late to be one"
);

/// Re-check cadence while waiting for the store. Coarse against a multi-second
/// bound and far finer than the contention it measures, so it costs nothing to
/// be wrong about.
const SQL_STORE_POLL: Duration = Duration::from_millis(10);

/// Acquire the workspace's `SqlStore` under [`SQL_STORE_WAIT`], or refuse
/// `lock_timeout` (retry class — "same request may succeed", v2 §8).
///
/// **Why a bound belongs here and not at the op-dispatch seam.** The daemon
/// serves one connection's ops serially, so the first reading of a slow `sql`
/// is head-of-line blocking on that connection — but the contention was not
/// the connection's. Every `sql` on a workspace passes through this one mutex,
/// and while the hold spanned the caller's query a fresh connection's `sql` on
/// the same workspace waited that query out (measured: 350.6 s against a
/// 351.6 s holder, while the same probe on a DIFFERENT workspace served in
/// 70 ms through the same daemon — the control that refutes a process-global
/// lock and CPU saturation alike). A bound placed at the dispatch seam would
/// therefore refuse ops on every connection touching that workspace — turning
/// one seat's expensive query into a fleet-wide refusal storm, which is a
/// worse correlated burst than the one it set out to cure. Bounding the wait
/// for THIS lock keeps the refusal with the seat that is actually blocked.
///
/// That 350.6 s figure is what the hold used to cost a sibling, and it is kept
/// here because it is what the bound was justified against. [`serve`] now
/// releases this guard before the caller's statement runs, so the wait a
/// sibling takes is an append's, not a query's.
///
/// `try_lock` polling is not FIFO: waiters race for each release, so a waiter
/// can lose turns it would have won under a queue. A bound caps the damage —
/// every waiter refuses at the same horizon — but it does not distribute it,
/// and the horizon is justified against a SINGLE legitimate hold while a
/// waiter absorbs the sum of the holds ahead of it plus its lost races.
/// **Whether that sum stays under the bound is not known.** The only
/// concurrency measured here was eight seats running sub-second ops, which
/// cannot queue past a deadline whatever the lock discipline and so says
/// nothing about the long-hold regime where the unfairness would bite. Stated
/// as a gap rather than a reassurance on purpose: this is exactly the shape of
/// evidence the bound exists to stop trusting.
///
/// `wait` is a parameter rather than a read of the constant so the contention
/// path can be gated in milliseconds instead of the production bound's
/// seconds; the one production caller passes [`SQL_STORE_WAIT`].
fn lock_store_bounded<'s, T>(
    store: &'s Mutex<T>,
    ws: &StdPath,
    wait: Duration,
) -> Result<MutexGuard<'s, T>, Box<ErrorBody>> {
    let deadline = Instant::now() + wait;
    loop {
        match store.try_lock() {
            Ok(guard) => return Ok(guard),
            // A poisoned store is the pre-existing contract on this path: a
            // panicked holder never leaves the cache half-appended, because
            // every query runs always-rollback.
            Err(TryLockError::Poisoned(p)) => return Ok(p.into_inner()),
            Err(TryLockError::WouldBlock) => {
                if Instant::now() >= deadline {
                    let mut e = ErrorBody::new(ErrorCode::LockTimeout);
                    e.message = Some(format!(
                        "another sql call has held this workspace's projection cache for \
                         longer than the {:.1}s service bound, so this call gave up its turn \
                         rather than hold the connection past the host's request deadline. \
                         The other call is still running and was not disturbed; its own \
                         answer is unaffected. Transient — the same request may succeed. \
                         Workspace: {}",
                        wait.as_secs_f32(),
                        ws.display()
                    ));
                    return Err(Box::new(e));
                }
                thread::sleep(SQL_STORE_POLL);
            }
        }
    }
}

/// Serve one `sql` call. The caller has warmed the workspace already.
///
/// # Errors
/// `io_error` when the cache file cannot be opened/appended or the corpus
/// cannot be read for the currency pass; `lock_timeout` (retry) when another
/// `sql` call held this workspace's projection cache past [`SQL_STORE_WAIT`];
/// the caller's own SQL failing is a SUCCESS body with `error` set and
/// `state: UNVERIFIED` — their register, not ours.
pub(crate) fn serve(
    registry: &Registry,
    ws: &Path,
    query: &str,
) -> Result<ResponseBody, Box<ErrorBody>> {
    let Some(engine) = registry.engine_snapshot(ws) else {
        // The dispatch warmed first; an absent engine here is a routing break.
        return Err(Box::new(ErrorBody::new(ErrorCode::Internal)));
    };
    let root = fs::WorkspaceRoot(ws.to_path_buf());

    // An unreadable domain config FAILS the door (decision 0034 — degrading
    // to the default domain would claim every path is hashed).
    let domain = fs::domain::Domain::load(&root)
        .map_err(|e| io_error(format!("cannot read the hash domain: {e}")))?;

    // The mount table, with a corpus for exactly the roots this corpus's own
    // wikilink/embed targets name — the shared assembly, so this door and the
    // CLI lane hand the projection the same resolver inputs.
    let mounts = wire_serve::mount_corpus::load_mounts_for(&view::walk::link_addressed_roots(
        &engine.docs,
        None,
    ));
    let corpus = mounts.rooted(&engine.docs, &domain, &root);
    let probe = fs::domain::LinkTargetProbe::new(&root, &domain);
    let exclusion = |target: &str| {
        probe
            .resolution(target)
            .map(|(path, why)| (path, why.word().to_owned()))
    };

    // The `.base` plane's own walk, under the SAME domain (`base-projection.md`
    // §3). A walk that fails hands the build no members: the append then leaves
    // every base row where it is and the stamp says NOT ASKED — an absent walk
    // must never read as an empty one, or the next append would tombstone every
    // member the workspace still has.
    let walk = fs::base::base_snapshot_under(&root, &domain).ok();
    let members: Vec<view::BaseMember> = walk
        .iter()
        .flat_map(|w| w.members.iter())
        .map(|m| view::BaseMember {
            path: m.path.clone(),
            bytes: m.bytes.clone(),
        })
        .collect();
    let base = walk.as_ref().map(|w| view::BaseWalk {
        members: &members,
        fold: &w.fold,
    });

    let store = registry
        .sql_store(ws)
        .map_err(|e| io_error(format!("cannot open the sql cache: {e}")))?;
    // THE HOLD, AND ITS END. The mutex covers the append and the opening of
    // this call's own read — nothing else. `begin_read` clones a connection off
    // the store's DuckDB instance and issues its `BEGIN` here, under the lock,
    // so the snapshot the caller will answer from is the one its own `sync`
    // just produced; the read then owns everything it needs and the store goes
    // back to the next caller. Before this split the guard lived until the
    // query returned, so every `sql` on this workspace — on ANY connection —
    // queued behind the slowest one (measured: 350.6 s against a 351.6 s
    // holder, while the same probe on a DIFFERENT workspace served in 70 ms
    // through the same daemon).
    let read = {
        let mut store = lock_store_bounded(&store, ws, SQL_STORE_WAIT)?;
        store
            .sync(
                &engine.docs,
                &corpus,
                Some(mounts.set()),
                Some(&exclusion),
                &engine.at_fingerprint.0,
                base.as_ref(),
            )
            .map_err(|e| io_error(format!("cannot append to the sql cache: {e}")))?;
        store
            .begin_read()
            .map_err(|e| io_error(format!("cannot open a read on the sql cache: {e}")))?
    };

    let result = read
        .run(query)
        .map_err(|e| io_error(format!("cannot query the sql cache: {e}")))?;
    let as_of = engine.at_fingerprint.0.clone();

    match result {
        // The caller's own SQL failed: no rows to certify, engine words
        // verbatim (state UNVERIFIED, §Q3 C3).
        Err(message) => Ok(ResponseBody::Sql {
            as_of_fingerprint: as_of,
            live: None,
            state: "UNVERIFIED".to_owned(),
            columns: Vec::new(),
            rows: Vec::new(),
            row_count: 0,
            error: Some(message),
        }),
        Ok((columns, rows)) => {
            // Post-result currency at the §6.7 vouched grade (cookie proof →
            // overlay fold, O(dirty); the leaf memo's stat sweep is the
            // floor on a named miss) — the fold post-dates the rows.
            let live = registry
                .currency_refresh(ws, crate::registry::DOOR_COOKIE_TIMEOUT)
                .map_err(|e| io_error(format!("cannot fold the corpus for live: {e}")))?
                .0
                .0;
            let state = if live == as_of {
                "FRESH_AT_SAMPLE"
            } else {
                "STALE"
            };
            let row_count = u64::try_from(rows.len()).unwrap_or(u64::MAX);
            Ok(ResponseBody::Sql {
                as_of_fingerprint: as_of,
                live: Some(live),
                state: state.to_owned(),
                columns: columns
                    .into_iter()
                    .map(|c| SqlCol {
                        name: c.name,
                        ty: c.ty,
                    })
                    .collect(),
                rows,
                row_count,
                error: None,
            })
        }
    }
}

fn io_error(cause: String) -> Box<ErrorBody> {
    let mut e = ErrorBody::new(ErrorCode::IoError);
    e.cause = Some(cause);
    Box::new(e)
}

#[cfg(test)]
mod tests {
    use super::{SQL_STORE_WAIT, lock_store_bounded};
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use wire::{ErrorCode, Recovery};

    const WS: &str = "/tmp/ws-under-test";

    /// The contended path refuses at the bound instead of waiting the holder
    /// out — the whole point of the leg. A held lock is exactly the state a
    /// slow `sql` puts the store in.
    #[test]
    fn a_held_store_refuses_lock_timeout_at_the_bound() {
        let store = Arc::new(Mutex::new(()));
        let held = Arc::clone(&store);
        let guard = held.lock().unwrap();

        let wait = Duration::from_millis(120);
        let started = Instant::now();
        let outcome = lock_store_bounded(&store, Path::new(WS), wait);
        let elapsed = started.elapsed();

        let err = outcome.expect_err("a held store must refuse, never block");
        assert_eq!(err.code, ErrorCode::LockTimeout);
        // Retry, not fix: the caller's request is well-formed and may succeed
        // unchanged once the holder finishes.
        assert_eq!(err.code.recovery(), Recovery::Retry);
        // It waited the bound rather than refusing instantly — a bound that
        // refuses on first contention would refuse ordinary overlap too.
        assert!(
            elapsed >= wait,
            "refused after {elapsed:?}, before the {wait:?} bound"
        );
        // The refusal names the workspace, so a fleet log attributes the stall.
        assert!(
            err.message.as_deref().unwrap_or_default().contains(WS),
            "refusal must name the workspace: {:?}",
            err.message
        );
        drop(guard);
    }

    /// The control that keeps the test above honest: the SAME call on an
    /// UNCONTENDED store must succeed promptly. Without it, a helper that
    /// refused unconditionally would pass the contention test.
    #[test]
    fn an_uncontended_store_is_acquired_and_not_refused() {
        let store = Mutex::new(());
        let started = Instant::now();
        let outcome = lock_store_bounded(&store, Path::new(WS), SQL_STORE_WAIT);
        let elapsed = started.elapsed();

        assert!(
            outcome.is_ok(),
            "an uncontended store must be acquired, not refused"
        );
        // It must not have sat out the production bound to get there.
        assert!(
            elapsed < Duration::from_secs(1),
            "uncontended acquisition took {elapsed:?}"
        );
    }

    /// A holder that releases inside the bound is waited for, not refused —
    /// the ordinary-overlap case that must NOT become a refusal. This is the
    /// case that fails if the bound is ever set below legitimate hold time.
    #[test]
    fn a_holder_that_releases_inside_the_bound_is_served() {
        let store = Arc::new(Mutex::new(()));
        let held = Arc::clone(&store);
        let handle = std::thread::spawn(move || {
            let guard = held.lock().unwrap();
            std::thread::sleep(Duration::from_millis(80));
            drop(guard);
        });
        // Let the thread take the lock first, so this really is contended.
        std::thread::sleep(Duration::from_millis(20));

        let outcome = lock_store_bounded(&store, Path::new(WS), Duration::from_secs(5));
        assert!(
            outcome.is_ok(),
            "a holder releasing inside the bound must be waited for, not refused"
        );
        drop(outcome);
        handle.join().unwrap();
    }
}
