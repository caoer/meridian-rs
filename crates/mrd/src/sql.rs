//! `mrd sql <query>` — operator SQL over the corpus projection, under the
//! honest-tense freshness frame.
//!
//! # Lanes (the ladder, in order — ONE ladder for every caller since the
//! NO-SANDBOX ruling, 2026-08-14)
//! 1. **daemon route** — lifecycle-B wire residency; the daemon holds the
//!    file, so a held file is served warm instead of degraded around
//!    (`--rebuild` skips it: repair needs the file direct);
//! 2. **direct file**: the fingerprint-pinned append-only `sql.duckdb` cache
//!    in the workspace cache drawer ([`view::store`]) — open read-write when
//!    unheld, append the corpus delta, query through the always-rollback
//!    lane;
//! 3. **`:memory:`**: the ephemeral build, when no cache root resolves or the
//!    file is held/unusable.
//!
//! The old `wire-contract.md` §10.4 file-organ drop is knowingly superseded
//! for sql by the 2026-08-14 lifecycle-B ruling (`view::store` module docs).
//!
//! # DML contract
//! The contract is **writes nothing durable**, not read-only SQL. On the
//! cache lane every query runs `BEGIN → statement → ROLLBACK`: DML against
//! the `hist.*` tables executes and dies at call end; DML against a latest
//! view refuses through `DuckDB`'s own error, extended with the remedy
//! (ruling OQ1 — a deliberate parity change vs the ephemeral lane, where the
//! projection tables are base tables and DML dies with the process instead).
//!
//! # Order of operations (§Q3, buffered)
//! 1. fold `F0` + bring the projection to `F0` (cache append, or `:memory:`
//!    build);
//! 2. execute the query to completion, materialise all rows;
//! 3. sample `live` = a full-corpus disk fold ([`fs::domain_snapshot`]) last,
//!    so it post-dates the result;
//! 4. `FRESH_AT_SAMPLE` iff `as_of == live`, else `STALE` (or `RACED` under a
//!    bounded `--fresh` that could not converge).
//!
//! # Three-valued freshness (C3)
//! `live_source ∈ {fold, none}`, `stale ∈ {true, false, null}`. Only a real
//! post-result fold sets `stale = true|false`; a SQL error yields no rows to
//! certify, so it reports `live_source=none, stale=null` (`state=UNVERIFIED`).

use std::collections::BTreeMap;
use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use view::store::{ColMeta, SqlStore};

use crate::resolve::resolve_runtime;
use crate::{Fail, current_dir};

/// The buffered top-level JSON document's schema version (OD9).
/// v2 (2026-08-14, NO-SANDBOX ruling): the `execution_profile` key is gone —
/// it existed only for the deleted profile split.
const JSON_SCHEMA_VERSION: u32 = 2;

// ---------------------------------------------------------------------------
// arguments
// ---------------------------------------------------------------------------

/// The parsed `mrd sql` invocation.
struct SqlArgs {
    query: Option<String>,
    fresh: bool,
    json: bool,
    rebuild: bool,
    cwd: Option<PathBuf>,
    /// `--root NAME`: the projection workspace by canonical root name, from
    /// the machine mount table (2026-08-18 rooted-refs-everywhere addendum:
    /// the runtime cwd should not be a factor). Mutually exclusive with
    /// `--cwd` — both select the workspace.
    root: Option<String>,
}

impl SqlArgs {
    fn parse(tail: &[String]) -> Result<Self, Fail> {
        let mut query: Option<String> = None;
        let mut fresh = false;
        let mut json = false;
        let mut rebuild = false;
        let mut cwd: Option<PathBuf> = None;
        let mut root: Option<String> = None;

        let mut i = 0;
        while i < tail.len() {
            let arg = tail[i].as_str();
            // `--flag=value` split.
            let (flag, inline) = match arg.split_once('=') {
                Some((f, v)) => (f, Some(v.to_owned())),
                None => (arg, None),
            };
            match flag {
                "--fresh" => fresh = true,
                // `--verify` was the opt-in fold on the (dropped) published-view
                // path; the ephemeral build always folds, so accept-and-ignore
                // would lie. Refused as unknown below.
                "--json" => json = true,
                // The explicit rebuild verb (ruling OQ3) — delete the cache
                // file and cold-build at the live corpus; doubles as repair.
                "--rebuild" => rebuild = true,
                "--cwd" => {
                    let v = take_value(flag, inline, tail, &mut i)?;
                    cwd = Some(PathBuf::from(v));
                }
                "--root" => {
                    let v = take_value(flag, inline, tail, &mut i)?;
                    root = Some(v);
                }
                other if other.starts_with('-') => {
                    return Err(Fail::tool(format!("unknown flag: {other}")));
                }
                _ if query.is_none() => query = Some(arg.to_owned()),
                _ => {
                    return Err(Fail::tool(format!(
                        "unexpected argument: {arg} (the query is a single argument — quote it)"
                    )));
                }
            }
            i += 1;
        }

        if query.is_none() && !rebuild {
            return Err(Fail::tool("mrd sql needs a <query> argument".to_owned()));
        }
        if root.is_some() && cwd.is_some() {
            return Err(Fail::tool(
                "--root and --cwd both select the projection workspace — pass one. \
                 --root NAME reads the machine mount table; --cwd PATH walks the \
                 resolution ladder from that directory."
                    .to_owned(),
            ));
        }
        Ok(SqlArgs {
            query,
            fresh,
            json,
            rebuild,
            cwd,
            root,
        })
    }
}

/// Resolve a flag's value: the `=`-inline form, else the next argument.
fn take_value(
    flag: &str,
    inline: Option<String>,
    tail: &[String],
    i: &mut usize,
) -> Result<String, Fail> {
    if let Some(v) = inline {
        return Ok(v);
    }
    *i += 1;
    tail.get(*i)
        .cloned()
        .ok_or_else(|| Fail::tool(format!("{flag} needs a value")))
}

// ---------------------------------------------------------------------------
// the freshness frame
// ---------------------------------------------------------------------------

/// The three-valued freshness state a delivered `mrd sql` result carries (§Q3).
#[derive(Clone, Copy, PartialEq, Eq)]
enum QueryState {
    FreshAtSample,
    Stale,
    Raced,
    Unverified,
    NoView,
}

impl QueryState {
    fn wire(self) -> &'static str {
        match self {
            QueryState::FreshAtSample => "FRESH_AT_SAMPLE",
            QueryState::Stale => "STALE",
            QueryState::Raced => "RACED",
            QueryState::Unverified => "UNVERIFIED",
            QueryState::NoView => "NO_VIEW",
        }
    }
}

/// The provenance of the `live` value (§Q3 C3): `fold` (a real post-result fold
/// ran) or `none` (a SQL error left no rows to certify).
#[derive(Clone, Copy, PartialEq, Eq)]
enum LiveSource {
    Fold,
    None,
}

impl LiveSource {
    fn wire(self) -> &'static str {
        match self {
            LiveSource::Fold => "fold",
            LiveSource::None => "none",
        }
    }
}

/// The `.base` plane's own tense (`base-projection.md` §6.3).
///
/// The frame carries TWO witnesses because their remedies differ: a caller who
/// just wrote markdown must not be told their Bases changed. So "the corpus
/// moved" and "the base plane moved" are different sentences, and the
/// unmeasured states are said out loud rather than rendered as silence —
/// §12.1's absence rule forecloses exactly that.
#[derive(Clone, PartialEq, Eq)]
enum BaseTense {
    /// The live re-walk matched the stamp.
    Matched,
    /// The live re-walk differs — the base plane moved.
    Moved,
    /// A walk failed, so nothing can be said about this plane.
    CannotSay,
    /// The build was handed no walk: "not measured", never "measured empty".
    NotWalked,
}

impl BaseTense {
    fn wire(&self) -> &'static str {
        match self {
            BaseTense::Matched => "matched",
            BaseTense::Moved => "moved",
            BaseTense::CannotSay => "cannot-say",
            BaseTense::NotWalked => "not-walked",
        }
    }
}

/// The buffered result + its freshness frame — the OD9 document, rendered to
/// human or JSON.
struct Frame {
    /// The `.base` plane's tense beside the md plane's (§6.3).
    base: BaseTense,
    as_of: Option<String>,
    live: Option<String>,
    live_source: LiveSource,
    stale: Option<bool>,
    state: QueryState,
    columns: Vec<ColMeta>,
    rows: Vec<Vec<Value>>,
    /// A SQL execution error (buffered into the OD9 doc), if any.
    error: Option<String>,
}

impl Frame {
    /// The empty `NO_VIEW` frame (§Q3 — loud, never empty-as-if-fresh).
    fn no_view(message: String) -> Self {
        Frame {
            as_of: None,
            live: None,
            live_source: LiveSource::None,
            stale: None,
            state: QueryState::NoView,
            // No corpus loaded means no walk was run either.
            base: BaseTense::NotWalked,
            columns: Vec::new(),
            rows: Vec::new(),
            error: Some(message),
        }
    }
}

// ---------------------------------------------------------------------------
// entry point
// ---------------------------------------------------------------------------

/// Run `mrd sql <query> [--fresh] [--json] [--rebuild] [--cwd PATH]`.
///
/// # Errors
/// The cwd/workspace cannot be resolved, the view cannot be built, or (in human
/// mode) the SQL fails. A `NO_VIEW`/`STALE`/`UNVERIFIED` frame is a success, not
/// an error — the frame is the honest report.
pub(crate) fn run(tail: &[String]) -> Result<(), Fail> {
    let args = SqlArgs::parse(tail)?;
    // `--root NAME` selects the projection workspace by canonical root name
    // (2026-08-18 rooted-refs-everywhere addendum): the mount table is the
    // whole authority, the cwd plays no part, and an unbound name refuses
    // enumerating what does bind — the rooted seam's own refusal family.
    let workspace = if let Some(name_raw) = &args.root {
        let name = addr::MountName::parse(name_raw)
            .map_err(|e| Fail::tool(format!("--root {name_raw}: {e}")))?;
        let rooted = crate::rooted::resolve_name(&name, name_raw, "No query ran.")
            .map_err(|e| Fail::tool(crate::engine::render_wire_error(&e)))?;
        rooted.workspace
    } else {
        let cwd = match &args.cwd {
            Some(p) => p.clone(),
            None => current_dir()?,
        };
        resolve_runtime(&cwd)
            .map_err(|e| {
                Fail::tool(format!(
                    "cannot resolve workspace for {}: {e}",
                    cwd.display()
                ))
            })?
            .workspace
    };

    // Ladder rung 1: the resident daemon answers first for EVERY caller —
    // lifecycle B makes it the cache file's single owner, so a held file is
    // served warm instead of degraded around. One execution path (the
    // NO-SANDBOX ruling, 2026-08-14); only `--rebuild` goes direct, because
    // repair needs the file itself.
    if !args.rebuild
        && let Some(frame) = daemon_route(&workspace, &args)
    {
        return emit(&args, &frame);
    }

    let loaded = match load_corpus(&workspace) {
        Ok(loaded) => loaded,
        Err(msg) => return emit(&args, &Frame::no_view(msg)),
    };
    let mut lane = open_lane(&loaded.root.0, args.rebuild);

    let Some(_) = &args.query else {
        // `--rebuild` alone: cold-build the cache and report; no query to run.
        return rebuild_only(&mut lane, &loaded);
    };

    let frame = query_frame(&args, &loaded, &mut lane)?;
    emit(&args, &frame)
}

/// One `--rebuild`-without-query invocation: sync the fresh file cold, speak
/// the receipt. On the `:memory:` lane there is no file to rebuild — refuse
/// loud instead of pretending.
fn rebuild_only(lane: &mut Lane, loaded: &Loaded) -> Result<(), Fail> {
    match lane {
        Lane::Cache(store) => {
            let counts = sync_store(store, loaded)?;
            let (generation, docs) = counts.map_or((0, 0), |c| (c.generation, c.added));
            println!(
                "rebuilt {} at {} (gen {generation}, {docs} docs)",
                store.file().display(),
                loaded.f0
            );
            Ok(())
        }
        Lane::Memory => Err(Fail::tool(
            "no cache file to rebuild (no cache root resolves here — the :memory: lane has nothing on disk)"
                .to_owned(),
        )),
    }
}

// ---------------------------------------------------------------------------
// ladder rung 1: the resident daemon (§ A.11)
// ---------------------------------------------------------------------------

/// One NDJSON exchange on an open daemon stream.
fn daemon_call(
    writer: &mut UnixStream,
    reader: &mut BufReader<UnixStream>,
    request: &Value,
) -> Option<Value> {
    let mut line = request.to_string();
    line.push('\n');
    writer.write_all(line.as_bytes()).ok()?;
    writer.flush().ok()?;
    let mut response = String::new();
    reader.read_line(&mut response).ok()?;
    serde_json::from_str(&response).ok()
}

/// Ask the resident daemon (§ A.11). `None` = no daemon answered — no
/// socket, a refused handshake, or an engine predating the `sql` cap — and
/// the caller falls down the ladder. A `--fresh` STALE answer re-asks once
/// (the daemon re-warms + appends per call), mirroring the local bound.
fn daemon_route(workspace: &Path, args: &SqlArgs) -> Option<Frame> {
    let query = args.query.as_deref()?;
    let client = registry::Client::from_default().ok()?;
    let stream = UnixStream::connect(client.socket_path()).ok()?;
    let mut writer = stream.try_clone().ok()?;
    let mut reader = BufReader::new(stream);

    let hello = daemon_call(
        &mut writer,
        &mut reader,
        &serde_json::json!({
            "op": "hello", "proto": 1, "contract": "v3",
            "workspace": workspace.to_str()?,
        }),
    )?;
    if hello.get("ok") != Some(&Value::Bool(true)) {
        return None;
    }
    let caps = hello.get("body")?.get("caps")?.as_array()?;
    if !caps.iter().any(|c| c.as_str() == Some("sql")) {
        // The resident engine predates § A.11 — fall down the ladder (the
        // file is then also unheld by a sql owner, so direct-open can win).
        return None;
    }

    let ask = serde_json::json!({"id": 1, "op": "sql", "query": query});
    let mut frame = daemon_sql_frame(&daemon_call(&mut writer, &mut reader, &ask)?)?;
    if args.fresh && frame.state == QueryState::Stale {
        frame = daemon_sql_frame(&daemon_call(&mut writer, &mut reader, &ask)?)?;
        if frame.state == QueryState::Stale {
            frame.state = QueryState::Raced;
        }
    }
    Some(frame)
}

/// Project one § A.11 response onto the OD9 frame. A daemon-side fault
/// (`ok:false`) answers `None` — the ladder degrades rather than guessing.
fn daemon_sql_frame(response: &Value) -> Option<Frame> {
    if response.get("ok") != Some(&Value::Bool(true)) {
        return None;
    }
    let body = response.get("body")?;
    let as_of = body.get("as_of_fingerprint")?.as_str()?.to_owned();
    let live = body.get("live").and_then(Value::as_str).map(str::to_owned);
    let error = body.get("error").and_then(Value::as_str).map(str::to_owned);
    let state = match body.get("state")?.as_str()? {
        "FRESH_AT_SAMPLE" => QueryState::FreshAtSample,
        "STALE" => QueryState::Stale,
        _ => QueryState::Unverified,
    };
    let columns = body
        .get("columns")?
        .as_array()?
        .iter()
        .filter_map(|c| {
            Some(ColMeta {
                name: c.get("name")?.as_str()?.to_owned(),
                ty: c.get("type")?.as_str()?.to_owned(),
            })
        })
        .collect();
    let rows = body
        .get("rows")?
        .as_array()?
        .iter()
        .filter_map(|r| r.as_array().cloned())
        .collect();
    Some(Frame {
        as_of: Some(as_of),
        live,
        live_source: if state == QueryState::Unverified {
            LiveSource::None
        } else {
            LiveSource::Fold
        },
        stale: match state {
            QueryState::FreshAtSample => Some(false),
            QueryState::Stale | QueryState::Raced => Some(true),
            _ => None,
        },
        state,
        // The daemon serves its own frame; until § A.11 carries the base
        // tense, this route cannot say what it did not receive.
        base: BaseTense::CannotSay,
        columns,
        rows,
        error,
    })
}

// ---------------------------------------------------------------------------
// corpus load — shared by both lanes
// ---------------------------------------------------------------------------

/// The parsed corpus + everything a projection needs, loaded once per attempt.
struct Loaded {
    root: fs::WorkspaceRoot,
    f0: String,
    docs: model::Docs,
    mounts: crate::walk_cmd::Mounts,
    domain: fs::domain::Domain,
    /// The `.base` walk this attempt was taken under (`base-projection.md`
    /// §3/§6.2), or the error that refused it — the base plane then says
    /// **cannot say**, never "empty" (§6.3).
    base: Result<fs::base::BaseSnapshot, String>,
}

/// The projection inputs a `.base` walk contributes, in the shape `view`
/// takes. Held beside [`Loaded`] because the members must outlive the borrow.
fn base_walk(loaded: &Loaded) -> Option<(Vec<view::BaseMember>, String)> {
    let snapshot = loaded.base.as_ref().ok()?;
    let members = snapshot
        .members
        .iter()
        .map(|m| view::BaseMember {
            path: m.path.clone(),
            bytes: m.bytes.clone(),
        })
        .collect();
    Some((members, snapshot.fold.clone()))
}

/// Load the corpus for one attempt; an `Err` is the `NO_VIEW` message (a
/// genuinely absent/unreadable corpus — loud, never empty-as-if-fresh).
fn load_corpus(workspace: &Path) -> Result<Loaded, String> {
    let canonical = workspace::canonicalize(workspace)
        .map_err(|e| format!("cannot resolve workspace {}: {e}", workspace.display()))?;
    let root = fs::WorkspaceRoot(canonical);
    // F0 is the stamp: `fold_live` folds the same way, so F0 vs F_now can only
    // differ when the corpus moved.
    let (files, f0) =
        fs::domain_snapshot(&root).map_err(|e| format!("cannot read the corpus: {e}"))?;
    let (_index, docs, unserved) = fs::build_corpus(files);
    crate::voice_unserved(&unserved);
    crate::voice_excluded(&root, &docs);
    // The projection is built with mount authority, from the same loader the
    // pin and link planes use; without it a cross-vault link projects as
    // dangling. Corpora narrow to the roots ambient wikilink/embed targets
    // name; the MountSet stays whole.
    let mounts =
        crate::walk_cmd::load_mounts_for(&crate::walk_cmd::link_addressed_roots(&docs, None));
    // The same filter this snapshot was taken under, so any face reading the
    // corpus tells an excluded path from a missing one (§12.1 verdict plane).
    let domain =
        fs::domain::Domain::load(&root).map_err(|e| format!("cannot read the hash domain: {e}"))?;
    // The base plane's own walk, under the SAME domain (§3: the hash domain's
    // rules with the floor swapped). A failed walk is not a failed load — the
    // md plane still answers, and the frame says the base plane cannot speak.
    let base = fs::base::base_snapshot_under(&root, &domain)
        .map_err(|e| format!("cannot walk the base plane: {e}"));
    Ok(Loaded {
        root,
        f0: f0.0,
        docs,
        mounts,
        domain,
        base,
    })
}

// ---------------------------------------------------------------------------
// the two lanes
// ---------------------------------------------------------------------------

/// Which projection answers this invocation (module docs § Lanes).
enum Lane {
    /// The drawer's `sql.duckdb`, open read-write — this process appends.
    Cache(SqlStore),
    /// The ephemeral `:memory:` build.
    Memory,
}

/// Resolve the lane for `canonical`: the drawer file when a cache root
/// resolves and the file opens (held/unusable degrades, voiced), else
/// `:memory:`.
fn open_lane(canonical: &Path, rebuild: bool) -> Lane {
    let drawer = cache::CacheDrawer::open(canonical);
    let Some(dir) = drawer.dir() else {
        return Lane::Memory;
    };
    // The sentinel is gc bookkeeping; its failure must not cost the answer.
    let _ = drawer.register();
    let file = dir.join(view::store::SQL_CACHE_FILENAME);
    let opened = if rebuild {
        SqlStore::rebuild(&file)
    } else {
        SqlStore::open(&file)
    };
    match opened {
        Ok(store) => Lane::Cache(store),
        Err(e) => {
            eprintln!("mrd sql: cache file unavailable ({e}); answering from :memory:");
            Lane::Memory
        }
    }
}

/// Bring `store` to the loaded corpus state (one append transaction, or a
/// no-op at the pinned fingerprint).
fn sync_store(
    store: &mut SqlStore,
    loaded: &Loaded,
) -> Result<Option<view::store::AppendCounts>, Fail> {
    let corpus = loaded
        .mounts
        .rooted(&loaded.docs, &loaded.domain, &loaded.root);
    let probe = fs::domain::LinkTargetProbe::new(&loaded.root, &loaded.domain);
    let exclusion = |target: &str| {
        probe
            .resolution(target)
            .map(|(p, why)| (p, why.word().to_owned()))
    };
    let walk = base_walk(loaded);
    let base = walk
        .as_ref()
        .map(|(members, fold)| view::BaseWalk { members, fold });
    store
        .sync(
            &loaded.docs,
            &corpus,
            Some(loaded.mounts.set()),
            Some(&exclusion),
            &loaded.f0,
            base.as_ref(),
        )
        .map_err(|e| Fail::tool(format!("cannot append to the sql cache: {e}")))
}

/// One projection + query at the loaded corpus state, on whichever lane.
struct Attempt {
    as_of: String,
    columns: Vec<ColMeta>,
    rows: Vec<Vec<Value>>,
    error: Option<String>,
}

fn attempt(args: &SqlArgs, loaded: &Loaded, lane: &mut Lane) -> Result<Attempt, Fail> {
    let query = args.query.as_deref().unwrap_or_default();
    match lane {
        Lane::Cache(store) => {
            sync_store(store, loaded)?;
            let result = store
                .query(query)
                .map_err(|e| Fail::tool(format!("cannot query the sql cache: {e}")))?;
            let (columns, rows, error) = match result {
                Ok((c, r)) => (c, r, None),
                Err(msg) => (Vec::new(), Vec::new(), Some(msg)),
            };
            Ok(Attempt {
                as_of: loaded.f0.clone(),
                columns,
                rows,
                error,
            })
        }
        Lane::Memory => {
            let corpus = loaded
                .mounts
                .rooted(&loaded.docs, &loaded.domain, &loaded.root);
            let probe = fs::domain::LinkTargetProbe::new(&loaded.root, &loaded.domain);
            let exclusion = |target: &str| {
                probe
                    .resolution(target)
                    .map(|(p, why)| (p, why.word().to_owned()))
            };
            let walk = base_walk(loaded);
            let base = walk
                .as_ref()
                .map(|(members, fold)| view::BaseWalk { members, fold });
            let conn = view::build_memory_rooted(
                &loaded.docs,
                &corpus,
                loaded.mounts.set(),
                &loaded.f0,
                Some(&exclusion),
                base.as_ref(),
            )
            .map_err(|e| Fail::tool(format!("cannot build the view: {e}")))?;
            let as_of = read_as_of(&conn)?;
            // No drawer on this lane: the env temp root is the absolute
            // spill home (card sql-spill-config-lockout — the default was
            // `.tmp` RELATIVE to the shell cwd).
            view::store::apply_spill_containment(
                &conn,
                &std::env::temp_dir().join("mrd-sql-spill"),
            )
            .map_err(|e| Fail::tool(format!("cannot apply the spill containment: {e}")))?;
            let (columns, rows, error) = match view::store::run_query(&conn, query) {
                Ok((c, r)) => (c, r, None),
                Err(msg) => (Vec::new(), Vec::new(), Some(msg)),
            };
            Ok(Attempt {
                as_of,
                columns,
                rows,
                error,
            })
        }
    }
}

/// The §Q3 order-of-operations over the chosen lane, with the `--fresh`
/// bounded retry.
fn query_frame(args: &SqlArgs, loaded: &Loaded, lane: &mut Lane) -> Result<Frame, Fail> {
    let built = attempt(args, loaded, lane)?;
    let Attempt {
        as_of,
        columns,
        rows,
        error,
    } = built;

    // A SQL error yields no rows to certify — report UNVERIFIED with the error.
    if let Some(error) = error {
        return Ok(Frame {
            as_of: Some(as_of),
            live: None,
            live_source: LiveSource::None,
            stale: None,
            state: QueryState::Unverified,
            // The query failed, so no plane was sampled at all.
            base: BaseTense::CannotSay,
            columns,
            rows,
            error: Some(error),
        });
    }

    test_fold_race_hook();
    let f_now = fold_live(&loaded.root.0)?;
    // Both planes are sampled AFTER the rows, so each post-dates the result.
    let base = base_tense(loaded);
    if as_of == f_now {
        return Ok(frame(
            as_of,
            f_now,
            columns,
            rows,
            QueryState::FreshAtSample,
            base,
        ));
    }

    // A mid-build change: `--fresh` gets one bounded retry; the default
    // reports STALE at the F0 build.
    if args.fresh {
        let reloaded = match load_corpus(&loaded.root.0) {
            Ok(l) => l,
            Err(msg) => return Ok(Frame::no_view(msg)),
        };
        let retry = attempt(args, &reloaded, lane)?;
        if retry.error.is_none() {
            test_fold_race_hook();
            let f_now2 = fold_live(&reloaded.root.0)?;
            let state = if retry.as_of == f_now2 {
                QueryState::FreshAtSample
            } else {
                QueryState::Raced
            };
            let base = base_tense(&reloaded);
            return Ok(frame(
                retry.as_of,
                f_now2,
                retry.columns,
                retry.rows,
                state,
                base,
            ));
        }
    }

    Ok(frame(as_of, f_now, columns, rows, QueryState::Stale, base))
}

/// Assemble a folded frame (always `live_source=fold`).
#[allow(clippy::similar_names)] // `stale` and `state` are both design vocabulary
fn frame(
    as_of: String,
    live: String,
    columns: Vec<ColMeta>,
    rows: Vec<Vec<Value>>,
    state: QueryState,
    base: BaseTense,
) -> Frame {
    let stale = Some(as_of != live);
    Frame {
        as_of: Some(as_of),
        live: Some(live),
        live_source: LiveSource::Fold,
        stale,
        state,
        base,
        columns,
        rows,
        error: None,
    }
}

/// The base plane's tense at the post-result sample (`base-projection.md`
/// §6.3): re-walk the members and compare against the fold the build was
/// stamped with.
///
/// Its cells are the spec's, and none of them is silence: a build handed no
/// walk says **not walked** (never "measured empty"), a walk that fails says
/// **cannot say**, and only a completed re-walk answers matched or moved.
fn base_tense(loaded: &Loaded) -> BaseTense {
    let Ok(stamped) = &loaded.base else {
        return BaseTense::NotWalked;
    };
    match fs::base::base_snapshot_under(&loaded.root, &loaded.domain) {
        Ok(live) if live.fold == stamped.fold => BaseTense::Matched,
        Ok(_) => BaseTense::Moved,
        Err(_) => BaseTense::CannotSay,
    }
}

// ---------------------------------------------------------------------------
// shared: read the stamp, fold live, the race hook
// ---------------------------------------------------------------------------

/// Read the authoritative `as_of` from the built view's `_meridian_view` stamp
/// (§Q3).
fn read_as_of(conn: &duckdb::Connection) -> Result<String, Fail> {
    conn.query_row("SELECT as_of_fingerprint FROM _meridian_view", [], |r| {
        r.get::<_, String>(0)
    })
    .map_err(|e| Fail::tool(format!("cannot read the view's _meridian_view stamp: {e}")))
}

/// Determinism hook for the STALE/RACED e2e gates: the §Q3 window (build →
/// post-result fold) is intra-process, so a test driving the real binary
/// cannot land a corpus mutation inside it from outside. When
/// `MRD_SQL_TEST_MUTATE` names a file, append one line to it before each
/// post-result fold — each fire moves the corpus, so the fold can never match
/// the build's `as_of`. Unset (production), this is a no-op env read.
fn test_fold_race_hook() {
    use std::io::Write as _;
    let Ok(path) = std::env::var("MRD_SQL_TEST_MUTATE") else {
        return;
    };
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(file, "moved");
    }
}

/// Sample `live` = a full-corpus disk fold (§Q3 step 5, `fs::domain_snapshot`).
fn fold_live(workspace: &Path) -> Result<String, Fail> {
    let canonical = workspace::canonicalize(workspace).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace {}: {e}",
            workspace.display()
        ))
    })?;
    let root = fs::WorkspaceRoot(canonical);
    let (_files, fingerprint) = fs::domain_snapshot(&root)
        .map_err(|e| Fail::tool(format!("cannot fold the corpus for live: {e}")))?;
    Ok(fingerprint.0)
}

// ---------------------------------------------------------------------------
// output
// ---------------------------------------------------------------------------

/// Emit the frame: one buffered JSON document under `--json` (OD9), else a human
/// table with the freshness banner. In human mode a SQL error is a loud tool
/// failure (exit 2); under `--json` it rides the buffered document (the parent
/// reads `error`).
fn emit(args: &SqlArgs, frame: &Frame) -> Result<(), Fail> {
    if args.json {
        println!("{}", frame_json(frame));
        return Ok(());
    }
    if let Some(error) = &frame.error {
        if frame.state == QueryState::NoView {
            eprintln!("mrd sql: {error}");
            return Err(Fail::tool(format!("NO_VIEW: {error}")));
        }
        return Err(Fail::tool(format!("query failed: {error}")));
    }
    print_human(frame);
    Ok(())
}

/// The OD9 buffered top-level JSON document.
fn frame_json(frame: &Frame) -> String {
    let columns: Vec<Value> = frame
        .columns
        .iter()
        .map(|c| json!({ "name": c.name, "type": c.ty }))
        .collect();
    let doc = json!({
        "schema_version": JSON_SCHEMA_VERSION,
        "as_of_fingerprint": frame.as_of,
        "live_source": frame.live_source.wire(),
        "stale": frame.stale,
        "state": frame.state.wire(),
        // The SECOND witness's verdict, beside the md plane's (§6.3). It is a
        // separate key because the two planes name different remedies.
        "base_plane": frame.base.wire(),
        "columns": columns,
        "rows": frame.rows,
        "row_count": frame.rows.len(),
        "error": frame.error,
    });
    serde_json::to_string_pretty(&doc).unwrap_or_else(|_| doc.to_string())
}

/// Print the human freshness banner + a simple aligned table.
fn print_human(frame: &Frame) {
    println!("{}", banner(frame));
    println!("{}", base_banner(frame));
    if frame.columns.is_empty() {
        return;
    }
    let header = frame
        .columns
        .iter()
        .map(|c| c.name.as_str())
        .collect::<Vec<_>>()
        .join("\t");
    println!("{header}");
    for row in &frame.rows {
        let line = row.iter().map(cell_text).collect::<Vec<_>>().join("\t");
        println!("{line}");
    }
    println!("({} row{})", frame.rows.len(), plural(frame.rows.len()));
}

/// The one-line freshness banner (§Q3 honest tense).
fn banner(frame: &Frame) -> String {
    let as_of = frame.as_of.as_deref().unwrap_or("(none)");
    match frame.state {
        QueryState::FreshAtSample => {
            format!("-- FRESH_AT_SAMPLE (as_of == live at the post-result sample; as_of={as_of})")
        }
        QueryState::Stale => {
            let live = frame.live.as_deref().unwrap_or("(none)");
            format!("-- STALE (as_of != live; as_of={as_of}, live={live})")
        }
        QueryState::Raced => {
            let live = frame.live.as_deref().unwrap_or("(none)");
            format!(
                "-- RACED (--fresh could not reach as_of == live in its bound; as_of={as_of}, live={live})"
            )
        }
        QueryState::Unverified => format!(
            "-- UNVERIFIED (the query failed, so no liveness fold certified rows; as_of={as_of})"
        ),
        QueryState::NoView => "-- NO_VIEW (no buildable corpus)".to_owned(),
    }
}

/// The base plane's own line (`base-projection.md` §6.3). It is a SEPARATE
/// sentence from the md banner on purpose: the two planes have different
/// remedies, so a caller who just wrote markdown is never told their Bases
/// moved — and the unmeasured cells say so rather than staying silent.
fn base_banner(frame: &Frame) -> &'static str {
    match frame.base {
        BaseTense::Matched => "-- base plane: matched (the .base members are as built)",
        BaseTense::Moved => {
            "-- base plane: MOVED (a .base file changed; the md corpus is judged separately above)"
        }
        BaseTense::CannotSay => "-- base plane: cannot say (the base walk did not complete)",
        BaseTense::NotWalked => "-- base plane: not walked (not measured — never 'measured empty')",
    }
}

/// Render one JSON cell as human table text (`null` → empty, strings unquoted).
fn cell_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

#[cfg(test)]
mod tests {
    use super::*;
    use duckdb::Connection;

    #[test]
    fn parse_defaults() {
        let args = SqlArgs::parse(&["SELECT 1".to_owned()]).expect("parse");
        assert!(!args.fresh && !args.json && !args.rebuild);
        assert_eq!(args.query.as_deref(), Some("SELECT 1"));
    }

    #[test]
    fn parse_verify_is_refused_as_unknown() {
        // `--verify` belonged to the dropped published-view path; the ephemeral
        // build always folds, so the flag must fail loud, not silently no-op.
        assert!(SqlArgs::parse(&["--verify".to_owned(), "SELECT 1".to_owned()]).is_err());
    }

    #[test]
    fn parse_cwd_inline_and_spaced() {
        let inline = SqlArgs::parse(&["--cwd=/somewhere".to_owned(), "SELECT 1".to_owned()])
            .expect("inline");
        assert_eq!(inline.cwd.as_deref(), Some(Path::new("/somewhere")));

        let spaced = SqlArgs::parse(&[
            "--cwd".to_owned(),
            "/somewhere".to_owned(),
            "SELECT 1".to_owned(),
        ])
        .expect("spaced");
        assert_eq!(spaced.cwd.as_deref(), Some(Path::new("/somewhere")));
    }

    #[test]
    fn parse_missing_query_is_error_unless_rebuilding() {
        assert!(SqlArgs::parse(&["--json".to_owned()]).is_err());
        // `--rebuild` alone is a complete invocation (ruling OQ3).
        let args = SqlArgs::parse(&["--rebuild".to_owned()]).expect("rebuild alone");
        assert!(args.rebuild && args.query.is_none());
    }

    #[test]
    fn list_cells_render_per_row_values_not_column_debug_dumps() {
        // F1: a list cell (e.g. `section.hpath` VARCHAR[]) used to fall through
        // the JSON conversion's debug arm, which prints the ENTIRE column's
        // Arrow dump plus a row index for every cell. Each cell must carry its
        // own row's values only, as a real JSON array — the `--json` face gets
        // the array, the human face renders it as a compact JSON field.
        let conn = Connection::open_in_memory().unwrap();
        let (cols, rows) = view::store::run_query(
            &conn,
            "SELECT * FROM (VALUES (['a','b']), (['c'])) t(hpath)",
        )
        .expect("query");
        assert_eq!(cols[0].ty, "LIST");
        assert_eq!(rows[0][0], json!(["a", "b"]));
        assert_eq!(rows[1][0], json!(["c"]));
        assert_eq!(cell_text(&rows[0][0]), r#"["a","b"]"#);
    }

    #[test]
    fn excluded_note_promise_matches_the_machine_answer() {
        // F3: the stderr note used to promise the complete list on "the
        // `excluded` key of the machine answer (`--json`)" — but sql's
        // frame_json emits NO such key, and never should (4807 paths in every
        // answer is a token bomb; the anti-silence law wants count + sample +
        // a TRUE pointer, §12.1). The two halves are asserted together so a
        // change to either drags the other into the same commit: the frame
        // stays excluded-free, and the note points at the one carrier that
        // really serves the list — the bare `mrd links --json` enumeration.
        let frame = Frame {
            as_of: Some("f0".to_owned()),
            live: Some("f0".to_owned()),
            live_source: LiveSource::Fold,
            stale: Some(false),
            state: QueryState::FreshAtSample,
            base: BaseTense::Matched,
            columns: vec![],
            rows: vec![],
            error: None,
        };
        let doc: Value = serde_json::from_str(&frame_json(&frame)).expect("frame json");
        assert!(
            doc.get("excluded").is_none(),
            "sql's machine answer must not carry the excluded list"
        );

        let note = crate::excluded_note(4807, "a, b, c and 4804 more");
        assert!(
            note.contains("`mrd links --json`"),
            "the note must point at the carrier that serves the complete list: {note}"
        );
        assert!(
            !note.contains("the machine answer (`--json`"),
            "the self-referential promise is the F3 lie — sql's own --json has no `excluded` key: {note}"
        );
    }

    /// The ephemeral lane's containment is plain config (NO-SANDBOX ruling):
    /// spill is bounded, nothing is locked, nothing is disabled.
    #[test]
    fn spill_containment_bounds_spill_and_locks_nothing() {
        let conn = Connection::open_in_memory().unwrap();
        view::store::apply_spill_containment(&conn, &std::env::temp_dir().join("mrd-sql-spill"))
            .expect("apply spill containment");

        let budget: String = conn
            .query_row(
                "SELECT value FROM duckdb_settings() WHERE name = 'max_temp_directory_size'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            !budget.contains('%'),
            "the spill budget must be a bounded size, never a %-of-disk default: {budget}"
        );
        assert!(
            conn.execute_batch("SET memory_limit='2GB'").is_ok(),
            "the containment locks nothing — a caller SET succeeds"
        );
    }
}
