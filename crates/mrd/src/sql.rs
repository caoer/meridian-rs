//! `mrd sql <query>` — operator SQL over an ephemeral in-process `:memory:`
//! projection of the corpus, under the honest-tense freshness frame. Writes
//! nothing to disk; carries no wire vocabulary (the published-view organ and
//! its `view_path` wire op were DROPPED by ruling — `wire-contract.md` §10.4,
//! 2026-08-06).
//!
//! # DML contract
//! The contract is **writes nothing durable**, not read-only SQL: DML against
//! the ephemeral projection is accepted and dies with the process. A
//! statement-kind guard would add a SQL classifier to prevent self-harm with
//! no durability behind it — under `agent` the sandbox already blocks every
//! durable exit (`enable_external_access=false`, `lock_configuration=true`).
//! `dml_against_the_ephemeral_view_is_accepted_and_writes_nothing` pins this.
//!
//! # Order of operations (§Q3, buffered)
//! 1. fold `F0` + build the `:memory:` view from the corpus;
//! 2. apply the `--execution-profile` sandbox (B5 order), execute the query to
//!    completion, materialise all rows;
//! 3. sample `live` = a full-corpus disk fold ([`fs::domain_snapshot`]) last,
//!    so it post-dates the result — an ephemeral build MUST fold post-result
//!    to be correct;
//! 4. `FRESH_AT_SAMPLE` iff `as_of == live`, else `STALE` (or `RACED` under a
//!    bounded `--fresh` that could not converge).
//!
//! # Three-valued freshness (C3)
//! `live_source ∈ {fold, none}`, `stale ∈ {true, false, null}`. Only a real
//! post-result fold sets `stale = true|false`; a SQL error yields no rows to
//! certify, so it reports `live_source=none, stale=null` (`state=UNVERIFIED`).

use std::path::{Path, PathBuf};

use duckdb::Connection;
use duckdb::types::ValueRef;
use serde_json::{Value, json};

use crate::resolve::resolve_runtime;
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
    json: bool,
    profile: ExecProfile,
    cwd: Option<PathBuf>,
}

impl SqlArgs {
    fn parse(tail: &[String]) -> Result<Self, Fail> {
        let mut query: Option<String> = None;
        let mut fresh = false;
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
                // `--verify` was the opt-in fold on the (dropped) published-view
                // path; the ephemeral build always folds, so accept-and-ignore
                // would lie. Refused as unknown below.
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
    columns: Vec<ColMeta>,
    rows: Vec<Vec<Value>>,
    /// A SQL execution error (buffered into the OD9 doc), if any.
    error: Option<String>,
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
            columns: Vec::new(),
            rows: Vec::new(),
            error: Some(message),
        }
    }
}

// ---------------------------------------------------------------------------
// entry point
// ---------------------------------------------------------------------------

/// Run `mrd sql <query> [--fresh] [--json]
/// [--execution-profile local|agent] [--cwd PATH]`.
///
/// # Errors
/// The cwd/workspace cannot be resolved, the view cannot be built, or (in human
/// mode) the SQL fails. A `NO_VIEW`/`STALE`/`UNVERIFIED` frame is a success, not
/// an error — the frame is the honest report.
pub(crate) fn run(tail: &[String]) -> Result<(), Fail> {
    let args = SqlArgs::parse(tail)?;
    let frame = execute(&args)?;
    emit(&args, &frame)
}

/// The §Q3 order-of-operations: resolve the workspace, build the ephemeral
/// `:memory:` view, query it under the post-result fold.
fn execute(args: &SqlArgs) -> Result<Frame, Fail> {
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
    ephemeral_query(&resolved.workspace, args)
}

// ---------------------------------------------------------------------------
// the ephemeral `:memory:` build path
// ---------------------------------------------------------------------------

/// Build an ephemeral `:memory:` view over the workspace corpus and query it
/// under the post-result fold. Writes nothing to disk. An ephemeral build
/// MUST fold post-result to be correct — it is never unconditionally fresh.
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
            columns,
            rows,
            error: Some(error),
        });
    }

    test_fold_race_hook();
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

    // A mid-build change: `--fresh` gets one bounded retry; the default
    // reports STALE at the F0 build.
    if args.fresh {
        let retry = build_and_run_ephemeral(workspace, args);
        if let Ok(EphemeralRun {
            as_of: as_of2,
            columns: columns2,
            rows: rows2,
            error: None,
        }) = retry
        {
            test_fold_race_hook();
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

/// Assemble an ephemeral-build frame (always `live_source=fold`).
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
        columns,
        rows,
        error: None,
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
    // F0 is the stamp: `fold_live` folds the same way, so F0 vs F_now can only
    // differ when the corpus moved.
    let (files, f0) = fs::domain_snapshot(&root)
        .map_err(|e| EphemeralError::NoCorpus(format!("cannot read the corpus: {e}")))?;
    let (_index, docs, unserved) = fs::build_corpus(files);
    crate::voice_unserved(&unserved);
    crate::voice_excluded(&root, &docs, &unserved);
    // The ephemeral view is built with mount authority, from the same loader the
    // pin and link planes use; without it a cross-vault link projects as dangling.
    // Corpora narrow to the roots ambient wikilink/embed targets name; the
    // MountSet stays whole.
    let mounts =
        crate::walk_cmd::load_mounts_for(&crate::walk_cmd::link_addressed_roots(&docs, None));
    // The same filter this snapshot was taken under, so any face reading the
    // corpus tells an excluded path from a missing one (§12.1 verdict plane).
    let domain = fs::domain::Domain::load(&root)
        .map_err(|e| EphemeralError::NoCorpus(format!("cannot read the hash domain: {e}")))?;
    let corpus = mounts.rooted(&docs, &domain, &root);
    // `link.exclusion` — the same mint the link doors ask through, injected as
    // data so `view` stays disk-free (session decision 0034). It reads the same
    // `domain` the corpus was rooted under, so the column and the rows cannot
    // disagree about which filter was in force.
    let exclusion = |target: &str| {
        fs::domain::link_target_exclusion(&root, &domain, target).map(|why| why.word().to_owned())
    };
    let conn = view::build_memory_rooted(&docs, &corpus, mounts.set(), &f0.0, Some(&exclusion))
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

/// Read the authoritative `as_of` from the built view's `_meridian_view` stamp
/// (§Q3).
fn read_as_of(conn: &Connection) -> Result<String, Fail> {
    conn.query_row("SELECT as_of_fingerprint FROM _meridian_view", [], |r| {
        r.get::<_, String>(0)
    })
    .map_err(|e| Fail::tool(format!("cannot read the view's _meridian_view stamp: {e}")))
}

/// Apply the `--execution-profile` resource limits (B5). Order is load-bearing
/// for `agent`: set the caps, disable external access, then lock the
/// configuration so untrusted SQL cannot re-raise any of it. There is no
/// `statement_timeout` pragma — a wall-clock cap is the parent's process kill.
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
/// (list/struct/decimal/timestamp) fall back to a debug string.
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
        // A list cell's ValueRef carries the WHOLE column array + a row index;
        // the debug fallback below would dump every row's values into every
        // cell (F1). The owned conversion slices out this row's elements.
        ValueRef::List(..) | ValueRef::Array(..) => {
            duck_value_to_json(&duckdb::types::Value::from(v))
        }
        other => Value::String(format!("{other:?}")),
    }
}

/// An owned duckdb value into JSON — the recursion arm for list/array cells,
/// whose elements arrive owned after the row slice. Scalars mirror
/// `value_ref_to_json`; unhandled shapes fall back to a debug string.
fn duck_value_to_json(v: &duckdb::types::Value) -> Value {
    use duckdb::types::Value as D;
    match v {
        D::Null => Value::Null,
        D::Boolean(b) => Value::Bool(*b),
        D::TinyInt(n) => Value::from(i64::from(*n)),
        D::SmallInt(n) => Value::from(i64::from(*n)),
        D::Int(n) => Value::from(i64::from(*n)),
        D::BigInt(n) => Value::from(*n),
        D::HugeInt(n) => {
            i64::try_from(*n).map_or_else(|_| Value::String(n.to_string()), Value::from)
        }
        D::UTinyInt(n) => Value::from(u64::from(*n)),
        D::USmallInt(n) => Value::from(u64::from(*n)),
        D::UInt(n) => Value::from(u64::from(*n)),
        D::UBigInt(n) => Value::from(*n),
        D::Float(f) => json_f64(f64::from(*f)),
        D::Double(f) => json_f64(*f),
        D::Text(s) => Value::String(s.clone()),
        D::Blob(bytes) => Value::String(hex(bytes)),
        D::List(items) | D::Array(items) => {
            Value::Array(items.iter().map(duck_value_to_json).collect())
        }
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
        "execution_profile": frame.profile.label(),
        "live_source": frame.live_source.wire(),
        "stale": frame.stale,
        "state": frame.state.wire(),
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
        assert!(!args.fresh && !args.json);
        assert_eq!(args.query, "SELECT 1");
    }

    #[test]
    fn parse_verify_is_refused_as_unknown() {
        // `--verify` belonged to the dropped published-view path; the ephemeral
        // build always folds, so the flag must fail loud, not silently no-op.
        assert!(SqlArgs::parse(&["--verify".to_owned(), "SELECT 1".to_owned()]).is_err());
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
    fn list_cells_render_per_row_values_not_column_debug_dumps() {
        // F1: a list cell (e.g. `section.hpath` VARCHAR[]) used to fall through
        // `value_ref_to_json`'s debug arm, which prints the ENTIRE column's
        // Arrow dump plus a row index for every cell. Each cell must carry its
        // own row's values only, as a real JSON array — the `--json` face gets
        // the array, the human face renders it as a compact JSON field.
        let conn = Connection::open_in_memory().unwrap();
        let (cols, rows) = run_user_query(
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
            profile: ExecProfile::Local,
            live_source: LiveSource::Fold,
            stale: Some(false),
            state: QueryState::FreshAtSample,
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
