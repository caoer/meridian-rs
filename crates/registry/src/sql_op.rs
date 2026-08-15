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

use std::path::Path;
use std::sync::PoisonError;

use wire::{ErrorBody, ErrorCode, ResponseBody, SqlCol};

use crate::Registry;

/// Serve one `sql` call. The caller has warmed the workspace already.
///
/// # Errors
/// `io_error` when the cache file cannot be opened/appended or the corpus
/// cannot be read for the currency pass; the caller's own SQL failing is a
/// SUCCESS body with `error` set and `state: UNVERIFIED` — their register,
/// not ours.
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
    let mut store = store.lock().unwrap_or_else(PoisonError::into_inner);
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

    let result = store
        .query(query)
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
            // Post-result currency: the leaf memo's stat sweep (O(corpus)
            // stats, O(changed) bytes) — the fold post-dates the rows.
            let live = registry
                .currency_root(ws)
                .map_err(|e| io_error(format!("cannot fold the corpus for live: {e}")))?
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
