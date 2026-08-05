//! `mrd sql <query>` — client-side `DuckDB` over the daemon-published, read-only,
//! fingerprint-stamped view file, under the full honest-tense freshness frame
//! (design `tournament-duckdb/team-a` §Q2/§Q3/§Q5, OD8/OD9, B1/B5, C3).
//!
//! # The order-of-operations law (§Q3, buffered)
//! 1. `view_path` → a candidate path (the reply's fingerprints are a PRE-OPEN
//!    hint only, discarded here);
//! 2. B1 reader-side pre-open check — a present `view.duckdb.wal` OR a
//!    non-read-only file ⇒ **never open** (would replay a dead generation or read
//!    a half-written file);
//! 3. open the file, read `as_of = SELECT as_of_fingerprint FROM _meridian_view`
//!    — the **authoritative** `as_of`, never the reply's hint;
//! 4. apply the `--execution-profile` sandbox (B5 order), execute the query to
//!    completion, materialise all rows (a pure function of `as_of`);
//! 5. sample `live` = a full-corpus disk fold ([`fs::domain_snapshot`]) — the
//!    LAST step, so it post-dates the result — **only** when the fold is opted
//!    into (`--verify`/`--fresh`); OD8's default degrade runs **no** fold;
//! 6. `FRESH_AT_SAMPLE` iff `as_of == live`, else `STALE` (or `RACED` under a
//!    bounded `--fresh` that could not converge); attach the three-valued frame.
//!
//! # Three-valued freshness (C3)
//! `live_source ∈ {fold, watch, none}`, `stale ∈ {true, false, null}`. Only a
//! real post-result fold sets `stale = true|false`; a skipped fold is
//! `live_source=none, stale=null` (`state=UNVERIFIED`) — **never** a watcher
//! value (OD8 guardrail).
//!
//! # OD8 default-degrade
//! Until a measured perfsuite PASS arms fold-by-default, the default query path
//! over a **published** view runs NO fold and returns
//! `live_source=none, stale=null` (`UNVERIFIED`). Fold is opt-in via
//! `--verify`/`--fresh`. Round-1's `claims.toml` rows stay `Measured`/`Untested`,
//! so the default is UNVERIFIED — correct, not a bug. An **ephemeral `:memory:`
//! build** (the daemon-absent degrade) is exempt: it MUST fold post-result to be
//! correct (§tier-4 — never an unconditionally-fresh ephemeral build).

use std::path::{Path, PathBuf};

use cache::CacheDrawer;
use duckdb::Connection;
use duckdb::types::ValueRef;
use registry::Client;
use serde_json::{Value, json};

use crate::resolve::{Source, resolve_runtime};
use crate::{Fail, current_dir};

/// The buffered top-level JSON document's schema version (OD9).
const JSON_SCHEMA_VERSION: u32 = 1;

/// The `local` (trusted) profile RAM cap — generous, only to avoid exhausting
/// host RAM; the user already has a shell (B5).
const LOCAL_MEMORY_LIMIT: &str = "4GB";
/// The `agent` (untrusted) profile RAM cap (B5).
const AGENT_MEMORY_LIMIT: &str = "512MB";
/// The `agent` profile CPU fan-out bound (B5).
const AGENT_THREADS: i64 = 1;
/// The `agent` profile deep-parse-bomb bound (B5; `DuckDB` default is 1000).
const AGENT_MAX_EXPRESSION_DEPTH: u32 = 1000;

// ---------------------------------------------------------------------------
// arguments
// ---------------------------------------------------------------------------

/// The `--execution-profile` selector (B5/OD9). A missing flag defaults to
/// `local`, so untrusted agent SQL can never silently fall into the trusted
/// profile — the agent caller MUST pass `--execution-profile=agent`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExecProfile {
    /// Trusted local CLI: `memory_limit` only, no sandbox.
    Local,
    /// Untrusted agent / proxy: the locked `DuckDB` sandbox (B5 order).
    Agent,
}

impl ExecProfile {
    fn label(self) -> &'static str {
        match self {
            ExecProfile::Local => "local",
            ExecProfile::Agent => "agent",
        }
    }

    fn parse(value: &str) -> Result<Self, Fail> {
        match value {
            "local" => Ok(ExecProfile::Local),
            "agent" => Ok(ExecProfile::Agent),
            other => Err(Fail::tool(format!(
                "unknown --execution-profile `{other}` (expected `local` or `agent`)"
            ))),
        }
    }
}

/// The parsed `mrd sql` invocation.
struct SqlArgs {
    query: String,
    fresh: bool,
    verify: bool,
    json: bool,
    profile: ExecProfile,
    cwd: Option<PathBuf>,
}

impl SqlArgs {
    /// Whether the post-result `live` fold runs on the published-view path.
    /// `--verify` opts in; `--fresh` implies it (a bounded rebuild is pointless
    /// without the honest post-check).
    fn folds(&self) -> bool {
        self.verify || self.fresh
    }

    fn parse(tail: &[String]) -> Result<Self, Fail> {
        let mut query: Option<String> = None;
        let mut fresh = false;
        let mut verify = false;
        let mut json = false;
        let mut profile = ExecProfile::Local;
        let mut cwd: Option<PathBuf> = None;

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
                "--verify" => verify = true,
                "--json" => json = true,
                "--execution-profile" => {
                    let v = take_value(flag, inline, tail, &mut i)?;
                    profile = ExecProfile::parse(&v)?;
                }
                "--cwd" => {
                    let v = take_value(flag, inline, tail, &mut i)?;
                    cwd = Some(PathBuf::from(v));
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

        let query =
            query.ok_or_else(|| Fail::tool("mrd sql needs a <query> argument".to_owned()))?;
        Ok(SqlArgs {
            query,
            fresh,
            verify,
            json,
            profile,
            cwd,
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

/// The provenance of the `live` value (§Q3 C3). Round-1 `mrd sql` only ever
/// produces `fold` (a real post-result fold ran) or `none` (no fold) — never
/// `watch` (a watcher value is a daemon-side pre-open hint, never a delivered
/// result's verdict).
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

/// One result column (OD9 JSON `columns` element).
struct ColMeta {
    name: String,
    ty: String,
}

/// The buffered result + its freshness frame — the OD9 document, rendered to
/// human or JSON.
struct Frame {
    as_of: Option<String>,
    live: Option<String>,
    profile: ExecProfile,
    live_source: LiveSource,
    stale: Option<bool>,
    state: QueryState,
    /// Present only when the daemon served the file (it has an epoch); omitted
    /// on any daemonless path (OD9).
    changes_seq: Option<u64>,
    columns: Vec<ColMeta>,
    rows: Vec<Vec<Value>>,
    /// A SQL execution error (buffered into the OD9 doc), if any.
    error: Option<String>,
    /// OD7: the daemon's last-refresh-failure telemetry, for the enriched STALE
    /// banner. Only ever set on the daemon-served path.
    last_error: Option<Value>,
}

impl Frame {
    /// The empty `NO_VIEW` frame (§Q3 — loud, never empty-as-if-fresh).
    fn no_view(profile: ExecProfile, message: String) -> Self {
        Frame {
            as_of: None,
            live: None,
            profile,
            live_source: LiveSource::None,
            stale: None,
            state: QueryState::NoView,
            changes_seq: None,
            columns: Vec::new(),
            rows: Vec::new(),
            error: Some(message),
            last_error: None,
        }
    }
}

// ---------------------------------------------------------------------------
// entry point
// ---------------------------------------------------------------------------

/// Run `mrd sql <query> [--fresh] [--verify] [--json]
/// [--execution-profile local|agent] [--cwd PATH]`.
///
/// # Errors
/// The cwd/workspace cannot be resolved, the view cannot be opened, or (in human
/// mode) the SQL fails. A `NO_VIEW`/`STALE`/`UNVERIFIED` frame is a success, not
/// an error — the frame is the honest report.
pub(crate) fn run(tail: &[String]) -> Result<(), Fail> {
    let args = SqlArgs::parse(tail)?;
    let (frame, path) = execute(&args)?;
    emit(&args, &frame, path)
}

/// **G13 — which path served this run, for the degrade voice only.**
///
/// The dogfood measured `mrd sql` degrading at 248× the warm cost (0.24s →
/// 59.63s) with stdout, stderr and exit byte-identical, so a person could pay
/// a minute for an answer and never learn why. This is the one bit the voice
/// needs, and it is deliberately NOT the freshness frame: `state` answers "is
/// this answer current?", which is a different question from "did the warm
/// engine serve it?" — a cold last-published file is `UNVERIFIED` exactly as a
/// warm one is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ServedBy {
    /// The resident daemon answered `view_path` and its file was queried.
    Daemon,
    /// A tiers-1-3 workspace whose daemon did not answer — the cold-file or
    /// in-process degrade ran instead. The costly, formerly mute path.
    Degrade,
    /// Tier-4 bare: `:memory:` is this tier's DESIGNED path, never a fallback,
    /// so it is silent. Voicing here would cry degrade on every correct run in
    /// an unregistered directory and teach the reader to ignore the line.
    Tier4,
}

/// The §Q3 order-of-operations, dispatched by resolution tier.
fn execute(args: &SqlArgs) -> Result<(Frame, ServedBy), Fail> {
    let cwd = match &args.cwd {
        Some(p) => p.clone(),
        None => current_dir()?,
    };
    let resolved = resolve_runtime(&cwd).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace for {}: {e}",
            cwd.display()
        ))
    })?;

    match resolved.source {
        // Tiers 1-3 / daemon-adopted: the daemon (sole builder) publishes; on any
        // daemon-path failure, degrade — open the last-good file cold, else build
        // ephemeral `:memory:` (§Behavior when daemon absent).
        Source::Direct(_) | Source::DaemonAdopted => {
            if let Some(reply) = try_daemon_view_path(&cwd, args.fresh) {
                let path = reply_path(&reply);
                if let Some(path) = path {
                    let frame = query_published(
                        Path::new(&path),
                        &resolved.workspace,
                        args,
                        reply_changes_seq(&reply),
                        reply_last_error(&reply),
                    )?;
                    return Ok((frame, ServedBy::Daemon));
                }
            }
            let frame = degrade_cold_or_ephemeral(&resolved.drawer, &resolved.workspace, args)?;
            Ok((frame, ServedBy::Degrade))
        }
        // Tier-4 bare: ephemeral `:memory:` ONLY — never the daemon, never a
        // drawer, never a claim on a prior registered workspace (§tier-4).
        Source::Ephemeral => Ok((ephemeral_query(&resolved.workspace, args)?, ServedBy::Tier4)),
    }
}

/// The tiers-1-3 daemon-absent degrade: open the last-published `view.duckdb`
/// cold if present, else build ephemeral `:memory:` (§Behavior when daemon
/// absent).
fn degrade_cold_or_ephemeral(
    drawer: &CacheDrawer,
    workspace: &Path,
    args: &SqlArgs,
) -> Result<Frame, Fail> {
    if let Some(dir) = drawer.dir() {
        let dest = dir.join("view.duckdb");
        if dest.is_file() {
            return query_published(&dest, workspace, args, None, None);
        }
    }
    ephemeral_query(workspace, args)
}

// ---------------------------------------------------------------------------
// the daemon `view_path` dial
// ---------------------------------------------------------------------------

/// Dial the resident daemon (auto-spawning it, the watchman model) and call
/// `view_path` for `cwd`. `None` on ANY failure — the caller degrades. `fresh`
/// asks the daemon for the bounded `--fresh` rebuild (§Q3).
pub(crate) fn try_daemon_view_path(cwd: &Path, fresh: bool) -> Option<Value> {
    let client = Client::from_default().ok()?;
    crate::engine::ensure_daemon(&client).ok()?;
    dial_view_path(client.socket_path(), cwd, fresh)
        .ok()
        .flatten()
}

/// One NDJSON round trip: send `{op:"view_path", cwd, fresh?}`, return the
/// success `body` object. `Ok(None)` on an op error (degrade); `Err` on
/// transport failure.
fn dial_view_path(socket: &Path, cwd: &Path, fresh: bool) -> std::io::Result<Option<Value>> {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;

    let stream = UnixStream::connect(socket)?;
    let mut writer = stream.try_clone()?;
    let mut reader = BufReader::new(stream);

    let mut req = json!({ "op": "view_path", "cwd": cwd.to_string_lossy() });
    if fresh {
        req["fresh"] = json!(true);
    }
    let mut line = serde_json::to_string(&req).map_err(std::io::Error::other)?;
    line.push('\n');
    writer.write_all(line.as_bytes())?;
    writer.flush()?;

    let mut response = String::new();
    if reader.read_line(&mut response)? == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "daemon closed the connection without a response",
        ));
    }
    let value: Value = serde_json::from_str(&response).map_err(std::io::Error::other)?;
    if value.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(value.get("body").cloned())
    } else {
        Ok(None)
    }
}

/// The daemon's `view_path` reply `path` (authoritative — the inode the consumer
/// opens).
fn reply_path(body: &Value) -> Option<String> {
    body.get("path").and_then(Value::as_str).map(str::to_owned)
}

/// The daemon's per-epoch `changes_seq` (OD9: present on the daemon path,
/// omitted daemonless).
fn reply_changes_seq(body: &Value) -> Option<u64> {
    body.get("changes_seq").and_then(Value::as_u64)
}

/// The daemon's OD7 `last_error` telemetry, if any (for the enriched STALE
/// banner).
fn reply_last_error(body: &Value) -> Option<Value> {
    match body.get("last_error") {
        Some(Value::Null) | None => None,
        Some(other) => Some(other.clone()),
    }
}

// ---------------------------------------------------------------------------
// the published-view path (daemon-served OR cold degrade)
// ---------------------------------------------------------------------------

/// Why a managed reader refused to open a published file (§B1 reader-side).
enum OpenRefusal {
    /// A `view.duckdb.wal` sidecar is present — opening would replay a dead
    /// generation (proven on v1.5.2, gate 14).
    WalPresent,
    /// The file is not read-only (`0444`) — a writer could be mutating it in
    /// place, so it may be half-written.
    NotReadOnly,
    /// The file is gone.
    Missing,
}

/// B1 reader-side pre-open check: refuse to open a `.wal`-shadowed OR non-
/// read-only published file. `stat view.duckdb.wal` AND verify the file carries
/// read-only mode (no writable bit). A **raw external** `duckdb` attach bypasses
/// this and is explicitly outside enforcement (design §Enforcement boundary).
fn check_openable(path: &Path) -> Result<(), OpenRefusal> {
    if !path.is_file() {
        return Err(OpenRefusal::Missing);
    }
    // A present WAL sidecar would be replayed by every open, including read-only.
    if wal_present(path) {
        return Err(OpenRefusal::WalPresent);
    }
    // Read-only mode (no writable bit) proves no in-place writer can be mutating
    // it (the publish `chmod`s the candidate `0444` before rename).
    if !is_read_only(path) {
        return Err(OpenRefusal::NotReadOnly);
    }
    Ok(())
}

/// Whether the `<path>.wal` sidecar `DuckDB` would replay is present beside
/// `path` (B1 reader-side).
pub(crate) fn wal_present(path: &Path) -> bool {
    wal_path(path).exists()
}

/// Whether `path` carries read-only mode — no writable bit for any principal
/// (the publish `chmod`s the candidate `0444`). A writable file may be
/// half-written by an in-place writer, so a managed reader refuses it (B1). A
/// missing file is treated as not-read-only (refuse).
pub(crate) fn is_read_only(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o222 == 0
    }
    #[cfg(not(unix))]
    {
        meta.permissions().readonly()
    }
}

/// The `<path>.wal` sidecar `DuckDB` would replay beside `path`.
fn wal_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".wal");
    PathBuf::from(s)
}

/// Query a published `view.duckdb` under the §Q3 buffered order. On a B1 refusal
/// the file is NOT opened — degrade to an ephemeral build (a managed reader never
/// replays a dead generation or reads a half-written file).
#[allow(clippy::similar_names)] // `stale` and `state` are both design vocabulary
fn query_published(
    path: &Path,
    workspace: &Path,
    args: &SqlArgs,
    changes_seq: Option<u64>,
    last_error: Option<Value>,
) -> Result<Frame, Fail> {
    if let Err(_refusal) = check_openable(path) {
        // The published file is unusable (WAL-shadowed / writable / gone). A
        // managed reader never opens it — fall back to a cold ephemeral build.
        return ephemeral_query(workspace, args);
    }

    // Open `:memory:` main, then ATTACH the view READ_ONLY (B5: attach FIRST,
    // while external access is on) and switch to it, so the user's unqualified
    // table names resolve.
    let conn = Connection::open_in_memory()
        .map_err(|e| Fail::tool(format!("cannot open an in-memory database: {e}")))?;
    conn.execute_batch(&attach_sql(path))
        .map_err(|e| Fail::tool(format!("cannot attach the view {}: {e}", path.display())))?;

    let as_of = read_as_of(&conn)?;
    apply_profile(&conn, args.profile)?;
    let query = run_user_query(&conn, &args.query);

    let (columns, rows, error) = match query {
        Ok((c, r)) => (c, r, None),
        Err(msg) => (Vec::new(), Vec::new(), Some(msg)),
    };

    // OD8 default-degrade: no fold unless opted in. A SQL error already yields no
    // rows to certify, so no fold runs (state UNVERIFIED, error carried).
    let fold = args.folds() && error.is_none();
    let (live_source, stale, state, live) = if fold {
        let live = fold_live(workspace)?;
        freshness(&as_of, &live, args.fresh)
    } else {
        (LiveSource::None, None, QueryState::Unverified, None)
    };

    Ok(Frame {
        as_of: Some(as_of),
        live,
        profile: args.profile,
        live_source,
        stale,
        state,
        changes_seq,
        columns,
        rows,
        error,
        last_error,
    })
}

/// Map a post-result (`as_of`, `live`) comparison to the three-valued frame.
/// `--fresh` maps a mismatch to `RACED` (a bounded rebuild that could not
/// converge); `--verify` maps it to `STALE`.
fn freshness(
    as_of: &str,
    live: &str,
    fresh: bool,
) -> (LiveSource, Option<bool>, QueryState, Option<String>) {
    let equal = as_of == live;
    let state = if equal {
        QueryState::FreshAtSample
    } else if fresh {
        QueryState::Raced
    } else {
        QueryState::Stale
    };
    (LiveSource::Fold, Some(!equal), state, Some(live.to_owned()))
}

/// `ATTACH '<path>' AS meridian (READ_ONLY); USE meridian;` with the path's
/// single quotes doubled (SQL-string escape).
fn attach_sql(path: &Path) -> String {
    let escaped = path.to_string_lossy().replace('\'', "''");
    format!("ATTACH '{escaped}' AS meridian (READ_ONLY); USE meridian;")
}

// ---------------------------------------------------------------------------
// the ephemeral `:memory:` build path (daemon-absent degrade)
// ---------------------------------------------------------------------------

/// Build an ephemeral `:memory:` view over the workspace corpus and query it
/// under the same post-result fold (§tiers-1-3 / §tier-4). Writes NOTHING to
/// disk (`view::build_memory` is `:memory:`-only). An ephemeral build MUST fold
/// post-result to be correct — it is never unconditionally fresh (§rejected: an
/// unconditionally-fresh ephemeral tier-4 build).
fn ephemeral_query(workspace: &Path, args: &SqlArgs) -> Result<Frame, Fail> {
    let built = match build_and_run_ephemeral(workspace, args) {
        Ok(b) => b,
        // No buildable corpus ⇒ NO_VIEW loud, never empty-as-if-fresh.
        Err(EphemeralError::NoCorpus(msg)) => return Ok(Frame::no_view(args.profile, msg)),
        Err(EphemeralError::Fail(f)) => return Err(f),
    };

    let EphemeralRun {
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
            profile: args.profile,
            live_source: LiveSource::None,
            stale: None,
            state: QueryState::Unverified,
            changes_seq: None,
            columns,
            rows,
            error: Some(error),
            last_error: None,
        });
    }

    // Post-result fold: `FRESH_AT_SAMPLE` iff `as_of == F_now`.
    let f_now = fold_live(workspace)?;
    if as_of == f_now {
        return Ok(ephemeral_frame(
            as_of,
            f_now,
            columns,
            rows,
            args.profile,
            QueryState::FreshAtSample,
        ));
    }

    // A mid-build change. `--fresh` gets ONE bounded retry → FRESH_AT_SAMPLE or
    // RACED; `--verify`/default report STALE at the F0 build.
    if args.fresh {
        let retry = build_and_run_ephemeral(workspace, args);
        if let Ok(EphemeralRun {
            as_of: as_of2,
            columns: columns2,
            rows: rows2,
            error: None,
        }) = retry
        {
            let f_now2 = fold_live(workspace)?;
            let state = if as_of2 == f_now2 {
                QueryState::FreshAtSample
            } else {
                QueryState::Raced
            };
            return Ok(ephemeral_frame(
                as_of2,
                f_now2,
                columns2,
                rows2,
                args.profile,
                state,
            ));
        }
    }

    Ok(ephemeral_frame(
        as_of,
        f_now,
        columns,
        rows,
        args.profile,
        QueryState::Stale,
    ))
}

/// Assemble an ephemeral-build frame (always `live_source=fold`; daemonless, so
/// `changes_seq` is omitted).
#[allow(clippy::similar_names)] // `stale` and `state` are both design vocabulary
fn ephemeral_frame(
    as_of: String,
    live: String,
    columns: Vec<ColMeta>,
    rows: Vec<Vec<Value>>,
    profile: ExecProfile,
    state: QueryState,
) -> Frame {
    let stale = Some(as_of != live);
    Frame {
        as_of: Some(as_of),
        live: Some(live),
        profile,
        live_source: LiveSource::Fold,
        stale,
        state,
        changes_seq: None,
        columns,
        rows,
        error: None,
        last_error: None,
    }
}

/// One ephemeral build + query: fold `F0`, build `:memory:`, read the stamp's
/// `as_of` (`== F0`), apply the profile, run the query.
struct EphemeralRun {
    as_of: String,
    columns: Vec<ColMeta>,
    rows: Vec<Vec<Value>>,
    error: Option<String>,
}

/// An ephemeral-build failure: a genuinely absent corpus (`NO_VIEW`) vs a real
/// tool failure.
enum EphemeralError {
    NoCorpus(String),
    Fail(Fail),
}

fn build_and_run_ephemeral(
    workspace: &Path,
    args: &SqlArgs,
) -> Result<EphemeralRun, EphemeralError> {
    let canonical = workspace::canonicalize(workspace).map_err(|e| {
        EphemeralError::NoCorpus(format!(
            "cannot resolve workspace {}: {e}",
            workspace.display()
        ))
    })?;
    let root = fs::WorkspaceRoot(canonical);
    // F0 — the authoritative fold over exactly the bytes this build projects,
    // under the workspace's own domain (filter + `version` prefix). It IS the
    // stamp: the post-result comparison below is F0 vs F_now, and `fold_live`
    // folds the same way, so the two sides can only differ when the corpus
    // actually moved. Refolding it inside the view crate is what G14 was — a
    // domain-blind `b3:` stamp against a `b3b:` live fold, STALE over identical
    // content.
    let (files, f0) = fs::domain_snapshot(&root)
        .map_err(|e| EphemeralError::NoCorpus(format!("cannot read the corpus: {e}")))?;
    let (_index, docs) = fs::build_corpus(files)
        .map_err(|e| EphemeralError::Fail(Fail::tool(format!("cannot build the corpus: {e}"))))?;
    // U21 — the ephemeral view is built with MOUNT AUTHORITY, from the same
    // loader the pin plane and the link plane use (S3-R59, one owner —
    // `walk_cmd::load_mounts_for`). Without it a cross-vault link projects as
    // dangling and every SQL consumer reads a working link as broken.
    //
    // Corpora narrow to the roots ambient wikilink/embed targets NAME
    // ([`crate::walk_cmd::link_addressed_roots`]); the MountSet stays whole.
    // This is NOT a lock-item scan and NOT a genuine full-table need: the view
    // projects ambient docs only, and mounted pages exist so `resolve_ref` can
    // land a rooted spelling. Measured on the multi-root table with zero rooted
    // spellings: ~27 s CPU before, same shape as the W5 residual.
    let mounts =
        crate::walk_cmd::load_mounts_for(&crate::walk_cmd::link_addressed_roots(&docs, None));
    let corpus = mounts.rooted(&docs);
    let conn = view::build_memory_rooted(&docs, &corpus, mounts.set(), &f0.0)
        .map_err(|e| EphemeralError::Fail(Fail::tool(format!("cannot build the view: {e}"))))?;

    let as_of = read_as_of(&conn).map_err(EphemeralError::Fail)?;
    apply_profile(&conn, args.profile).map_err(EphemeralError::Fail)?;
    let (columns, rows, error) = match run_user_query(&conn, &args.query) {
        Ok((c, r)) => (c, r, None),
        Err(msg) => (Vec::new(), Vec::new(), Some(msg)),
    };
    Ok(EphemeralRun {
        as_of,
        columns,
        rows,
        error,
    })
}

// ---------------------------------------------------------------------------
// shared: read the stamp, apply the sandbox, run the query, fold live
// ---------------------------------------------------------------------------

/// Read the authoritative `as_of` from the opened view's `_meridian_view` stamp
/// (§Q3 — never the `view_path` reply's hint).
fn read_as_of(conn: &Connection) -> Result<String, Fail> {
    conn.query_row("SELECT as_of_fingerprint FROM _meridian_view", [], |r| {
        r.get::<_, String>(0)
    })
    .map_err(|e| Fail::tool(format!("cannot read the view's _meridian_view stamp: {e}")))
}

/// Apply the `--execution-profile` resource limits (B5). Order is load-bearing
/// for `agent`: the view is already attached `READ_ONLY` (external access still on), then set
/// the caps, then disable external access, then LOCK the configuration so
/// untrusted SQL cannot re-raise any of it. There is **no** `statement_timeout`
/// pragma — a wall-clock cap is the parent's process kill (OD9).
fn apply_profile(conn: &Connection, profile: ExecProfile) -> Result<(), Fail> {
    let sql = match profile {
        ExecProfile::Local => format!("SET memory_limit='{LOCAL_MEMORY_LIMIT}';"),
        ExecProfile::Agent => format!(
            "SET memory_limit='{AGENT_MEMORY_LIMIT}';\n\
             SET threads={AGENT_THREADS};\n\
             SET max_expression_depth={AGENT_MAX_EXPRESSION_DEPTH};\n\
             SET enable_external_access=false;\n\
             SET lock_configuration=true;"
        ),
    };
    conn.execute_batch(&sql)
        .map_err(|e| Fail::tool(format!("cannot apply the {} sandbox: {e}", profile.label())))
}

/// Execute the user's query and materialise all rows + column metadata. Returns
/// the SQL error string (never a `Fail`) so the caller can buffer it into the
/// OD9 document.
fn run_user_query(
    conn: &Connection,
    query: &str,
) -> Result<(Vec<ColMeta>, Vec<Vec<Value>>), String> {
    let mut stmt = conn.prepare(query).map_err(|e| e.to_string())?;
    let mut rows = stmt.query([]).map_err(|e| e.to_string())?;

    // Column metadata is available once the query has executed (`query` binds +
    // executes); collect owned copies before stepping the rows.
    let columns: Vec<ColMeta> = {
        let stmt_ref = rows
            .as_ref()
            .ok_or_else(|| "no result statement".to_owned())?;
        let n = stmt_ref.column_count();
        let mut cols = Vec::with_capacity(n);
        for i in 0..n {
            let name = stmt_ref
                .column_name(i)
                .map_or_else(|_| format!("col{i}"), String::clone);
            let ty = arrow_type_name(&stmt_ref.column_type(i));
            cols.push(ColMeta { name, ty });
        }
        cols
    };

    let ncol = columns.len();
    let mut out = Vec::new();
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let mut r = Vec::with_capacity(ncol);
        for i in 0..ncol {
            let v = row.get_ref(i).map_or(Value::Null, value_ref_to_json);
            r.push(v);
        }
        out.push(r);
    }
    Ok((columns, out))
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

/// A friendly type name for an arrow result column (best effort — the JSON
/// `type` field, OD9).
fn arrow_type_name(dt: &duckdb::arrow::datatypes::DataType) -> String {
    use duckdb::arrow::datatypes::DataType as D;
    match dt {
        D::Boolean => "BOOLEAN".to_owned(),
        D::Int8 => "TINYINT".to_owned(),
        D::Int16 => "SMALLINT".to_owned(),
        D::Int32 => "INTEGER".to_owned(),
        D::Int64 => "BIGINT".to_owned(),
        D::UInt8 => "UTINYINT".to_owned(),
        D::UInt16 => "USMALLINT".to_owned(),
        D::UInt32 => "UINTEGER".to_owned(),
        D::UInt64 => "UBIGINT".to_owned(),
        D::Float32 => "FLOAT".to_owned(),
        D::Float64 => "DOUBLE".to_owned(),
        D::Utf8 | D::LargeUtf8 => "VARCHAR".to_owned(),
        D::Binary | D::LargeBinary => "BLOB".to_owned(),
        D::List(_) | D::LargeList(_) => "LIST".to_owned(),
        other => format!("{other:?}"),
    }
}

/// Render one result cell into JSON. Scalars are exact; complex cells
/// (list/struct/decimal/timestamp) fall back to a debug string in round-1
/// (nested/non-scalar projection is a documented non-goal).
fn value_ref_to_json(v: ValueRef<'_>) -> Value {
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Boolean(b) => Value::Bool(b),
        ValueRef::TinyInt(n) => Value::from(i64::from(n)),
        ValueRef::SmallInt(n) => Value::from(i64::from(n)),
        ValueRef::Int(n) => Value::from(i64::from(n)),
        ValueRef::BigInt(n) => Value::from(n),
        ValueRef::HugeInt(n) => {
            i64::try_from(n).map_or_else(|_| Value::String(n.to_string()), Value::from)
        }
        ValueRef::UTinyInt(n) => Value::from(u64::from(n)),
        ValueRef::USmallInt(n) => Value::from(u64::from(n)),
        ValueRef::UInt(n) => Value::from(u64::from(n)),
        ValueRef::UBigInt(n) => Value::from(n),
        ValueRef::Float(f) => json_f64(f64::from(f)),
        ValueRef::Double(f) => json_f64(f),
        ValueRef::Text(bytes) => Value::String(String::from_utf8_lossy(bytes).into_owned()),
        ValueRef::Blob(bytes) => Value::String(hex(bytes)),
        other => Value::String(format!("{other:?}")),
    }
}

/// A finite `f64` as a JSON number; NaN/±Inf render as `null` (JSON has no
/// representation).
fn json_f64(f: f64) -> Value {
    serde_json::Number::from_f64(f).map_or(Value::Null, Value::Number)
}

/// Lowercase hex of a byte slice (BLOB rendering).
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

// ---------------------------------------------------------------------------
// output
// ---------------------------------------------------------------------------

/// Emit the frame: one buffered JSON document under `--json` (OD9), else a human
/// table with the freshness banner. In human mode a SQL error is a loud tool
/// failure (exit 2); under `--json` it rides the buffered document (the parent
/// reads `error`).
fn emit(args: &SqlArgs, frame: &Frame, path: ServedBy) -> Result<(), Fail> {
    let result = emit_result(args, frame);
    // **G13 — the voice fires on EVERY exit path, after stdout is written.**
    // The dogfood's most expensive silent degrade was the ERROR path (0.70s →
    // 99s, byte-identical), so voicing only the success arm would leave the
    // worst case mute. It runs last so the answer (or the refusal) reaches the
    // reader first, and it touches stderr only — stdout and the exit code are
    // byte-identical to the warm run, which is the constraint that lets this
    // voice exist at all.
    if path == ServedBy::Degrade {
        crate::engine::voice_degrade(&crate::engine::EngineSource::Ephemeral);
    }
    result
}

/// The face itself: the OD9 document, the human table, or the buffered error.
fn emit_result(args: &SqlArgs, frame: &Frame) -> Result<(), Fail> {
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

/// The OD9 buffered top-level JSON document (not NDJSON — the fold completes
/// before serialization, so the frame rides the header, no trailer).
fn frame_json(frame: &Frame) -> String {
    let columns: Vec<Value> = frame
        .columns
        .iter()
        .map(|c| json!({ "name": c.name, "type": c.ty }))
        .collect();
    let mut doc = json!({
        "schema_version": JSON_SCHEMA_VERSION,
        "as_of_fingerprint": frame.as_of,
        "execution_profile": frame.profile.label(),
        "live_source": frame.live_source.wire(),
        "stale": frame.stale,
        "state": frame.state.wire(),
        "columns": columns,
        "rows": frame.rows,
        "row_count": frame.rows.len(),
        "error": frame.error,
    });
    // `changes_seq` omitted when daemonless (no epoch) — OD9.
    if let Some(seq) = frame.changes_seq {
        doc["changes_seq"] = json!(seq);
    }
    serde_json::to_string_pretty(&doc).unwrap_or_else(|_| doc.to_string())
}

/// Print the human freshness banner + a simple aligned table.
fn print_human(frame: &Frame) {
    println!("{}", banner(frame));
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

/// The one-line freshness banner (§Q3 honest tense). STALE carries the OD7
/// enrichment when the daemon's last rebuild failed.
fn banner(frame: &Frame) -> String {
    let as_of = frame.as_of.as_deref().unwrap_or("(none)");
    match frame.state {
        QueryState::FreshAtSample => {
            format!("-- FRESH_AT_SAMPLE (as_of == live at the post-result sample; as_of={as_of})")
        }
        QueryState::Stale => {
            let live = frame.live.as_deref().unwrap_or("(none)");
            if let Some(enriched) = stale_refresh_suffix(frame) {
                format!("-- STALE ({enriched}; serving last-good as_of={as_of})")
            } else {
                format!("-- STALE (as_of != live; as_of={as_of}, live={live})")
            }
        }
        QueryState::Raced => {
            let live = frame.live.as_deref().unwrap_or("(none)");
            format!(
                "-- RACED (--fresh could not reach as_of == live in its bound; as_of={as_of}, live={live})"
            )
        }
        QueryState::Unverified => format!(
            "-- UNVERIFIED (no liveness fold — OD8 default degrade; pass --verify/--fresh; as_of={as_of})"
        ),
        QueryState::NoView => "-- NO_VIEW (no view.duckdb and no buildable corpus)".to_owned(),
    }
}

/// The OD7 STALE-banner enrichment: `last refresh failed: <code>, <age> ago`
/// when the daemon reported a `last_error`.
fn stale_refresh_suffix(frame: &Frame) -> Option<String> {
    let err = frame.last_error.as_ref()?;
    let code = err.get("code").and_then(Value::as_str).unwrap_or("unknown");
    let age = err.get("unix").and_then(Value::as_u64).map_or_else(
        || "unknown age".to_owned(),
        |unix| format!("{}s ago", age_secs(unix)),
    );
    Some(format!("last refresh failed: {code}, {age}"))
}

/// Whole seconds between `unix` and now (saturating; `0` if in the future).
fn age_secs(unix: u64) -> u64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    now.saturating_sub(unix)
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

    #[test]
    fn parse_defaults_to_local_profile() {
        let args = SqlArgs::parse(&["SELECT 1".to_owned()]).expect("parse");
        assert_eq!(args.profile.label(), "local");
        assert!(!args.fresh && !args.verify && !args.json);
        assert_eq!(args.query, "SELECT 1");
    }

    #[test]
    fn parse_execution_profile_inline_and_spaced() {
        let inline = SqlArgs::parse(&[
            "--execution-profile=agent".to_owned(),
            "SELECT 1".to_owned(),
        ])
        .expect("inline");
        assert_eq!(inline.profile.label(), "agent");

        let spaced = SqlArgs::parse(&[
            "--execution-profile".to_owned(),
            "agent".to_owned(),
            "SELECT 1".to_owned(),
        ])
        .expect("spaced");
        assert_eq!(spaced.profile.label(), "agent");
    }

    #[test]
    fn parse_unknown_profile_is_loud() {
        let err = SqlArgs::parse(&["--execution-profile=root".to_owned(), "SELECT 1".to_owned()]);
        assert!(err.is_err(), "an unknown profile must fail loud");
    }

    #[test]
    fn parse_missing_query_is_error() {
        assert!(SqlArgs::parse(&["--json".to_owned()]).is_err());
    }

    #[test]
    fn folds_only_under_verify_or_fresh() {
        let base = SqlArgs::parse(&["SELECT 1".to_owned()]).unwrap();
        assert!(!base.folds(), "OD8 default runs no fold");
        let verify = SqlArgs::parse(&["--verify".to_owned(), "SELECT 1".to_owned()]).unwrap();
        assert!(verify.folds());
        let fresh = SqlArgs::parse(&["--fresh".to_owned(), "SELECT 1".to_owned()]).unwrap();
        assert!(fresh.folds());
    }

    #[test]
    #[allow(clippy::similar_names)] // `stale` and `state` are both design vocabulary
    fn freshness_maps_states() {
        let (src, stale, state, live) = freshness("b3:x", "b3:x", false);
        assert!(matches!(src, LiveSource::Fold));
        assert_eq!(stale, Some(false));
        assert!(matches!(state, QueryState::FreshAtSample));
        assert_eq!(live.as_deref(), Some("b3:x"));

        let (_, stale, state, _) = freshness("b3:x", "b3:y", false);
        assert_eq!(stale, Some(true));
        assert!(matches!(state, QueryState::Stale));

        let (_, _, state, _) = freshness("b3:x", "b3:y", true);
        assert!(
            matches!(state, QueryState::Raced),
            "--fresh mismatch is RACED"
        );
    }

    #[test]
    fn attach_sql_escapes_single_quotes() {
        let sql = attach_sql(Path::new("/tmp/it's/view.duckdb"));
        assert!(
            sql.contains("'/tmp/it''s/view.duckdb'"),
            "quotes doubled: {sql}"
        );
        assert!(sql.contains("READ_ONLY"));
    }

    /// Gate 14 (reader side): the B1 pre-open check refuses a `.wal`-shadowed OR
    /// non-read-only file, accepts a clean `0444` file, and reports a missing one.
    #[test]
    fn check_openable_enforces_no_wal_and_read_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("view.duckdb");
        std::fs::write(&path, b"placeholder").unwrap();

        // A writable file (a writer could be mutating it in place) is refused.
        assert!(matches!(
            check_openable(&path),
            Err(OpenRefusal::NotReadOnly)
        ));

        // `chmod 0444` (the publish mode) is accepted.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444)).unwrap();
        assert!(check_openable(&path).is_ok());

        // A present `.wal` sidecar ⇒ refuse open (would replay a dead generation).
        let wal = dir.path().join("view.duckdb.wal");
        std::fs::write(&wal, b"STALE-WAL-GEN").unwrap();
        assert!(matches!(
            check_openable(&path),
            Err(OpenRefusal::WalPresent)
        ));
        std::fs::remove_file(&wal).unwrap();

        // A gone file is Missing.
        assert!(matches!(
            check_openable(&dir.path().join("absent.duckdb")),
            Err(OpenRefusal::Missing)
        ));
    }

    /// Gate 15 (B5): the `agent` sandbox blocks file reads
    /// (`enable_external_access=false`), freezes settings
    /// (`lock_configuration=true`), and there is NO `statement_timeout` pragma on
    /// this `DuckDB` — a wall-clock cap is the parent's process kill (OD9).
    #[test]
    fn agent_sandbox_blocks_external_access_locks_config_no_statement_timeout() {
        let conn = Connection::open_in_memory().unwrap();
        apply_profile(&conn, ExecProfile::Agent).expect("apply agent sandbox");

        assert!(
            conn.execute_batch("SELECT * FROM read_csv('/etc/hosts')")
                .is_err(),
            "enable_external_access=false must block file reads"
        );
        assert!(
            conn.execute_batch("SET memory_limit='8GB'").is_err(),
            "lock_configuration=true must freeze settings"
        );
        let timeout_settings: i64 = conn
            .query_row(
                "SELECT count(*) FROM duckdb_settings() WHERE name ILIKE '%statement_timeout%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(timeout_settings, 0, "no statement_timeout setting exists");
    }

    /// The `local` sandbox sets a memory cap only — it does NOT lock the
    /// configuration (the trusted user already has a shell).
    #[test]
    fn local_sandbox_does_not_lock_configuration() {
        let conn = Connection::open_in_memory().unwrap();
        apply_profile(&conn, ExecProfile::Local).expect("apply local sandbox");
        assert!(
            conn.execute_batch("SET memory_limit='2GB'").is_ok(),
            "local does not lock configuration"
        );
    }
}
