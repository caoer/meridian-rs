//! `mrd view status` — the OD7 refresh-observability surface (synchronous half).
//!
//! Prints, per workspace: `as_of`, `state`, and the last-refresh-failure basics
//! (`code` + `age` + `fingerprint_attempted`) plus `refresh_in_progress`. This
//! telemetry lives in **daemon memory** and rides the `view_path` reply's OD7
//! advisory fields (`refresh_in_progress`, `last_error`) — it is **telemetry,
//! never correctness**: it explains WHY a view is stale, it never gates a
//! query's fresh/stale/null verdict.
//!
//! `consecutive_failures` / `next_retry` are the **async-executor** half of OD7
//! (round-2, V4); the wire reply carries no such fields (V3 adds NO new wire
//! surface), so round-1 reports them as `n/a (round-2 async executor)`.
//!
//! Daemon absent ⇒ no daemon-memory telemetry exists; the command reports the
//! last-built stamp from the cold `view.duckdb` if present, else NO_VIEW.

use std::path::{Path, PathBuf};

use duckdb::Connection;
use serde_json::{Value, json};

use crate::resolve::resolve_runtime;
use crate::{Fail, Format, current_dir};

/// Run `mrd view status [--json] [--cwd PATH]`.
///
/// # Errors
/// The cwd/workspace cannot be resolved.
pub(crate) fn run(tail: &[String]) -> Result<(), Fail> {
    let (format, cwd_arg) = parse(tail)?;
    let cwd = match cwd_arg {
        Some(p) => p,
        None => current_dir()?,
    };
    let resolved = resolve_runtime(&cwd).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace for {}: {e}",
            cwd.display()
        ))
    })?;

    let status = match crate::sql::try_daemon_view_path(&cwd, false) {
        Some(body) => Status::from_daemon(&resolved.workspace, &body),
        None => Status::from_cold(&resolved.workspace, resolved.drawer.dir()),
    };

    match format {
        Format::Json => println!("{}", status.json()),
        Format::Human => status.print_human(),
    }
    Ok(())
}

/// Parse `[--json] [--cwd PATH]`.
fn parse(tail: &[String]) -> Result<(Format, Option<PathBuf>), Fail> {
    let mut json = false;
    let mut cwd: Option<PathBuf> = None;
    let mut i = 0;
    while i < tail.len() {
        let arg = tail[i].as_str();
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) => (f, Some(v.to_owned())),
            None => (arg, None),
        };
        match flag {
            "--json" => json = true,
            "--cwd" => {
                let v = match inline {
                    Some(v) => v,
                    None => {
                        i += 1;
                        tail.get(i)
                            .cloned()
                            .ok_or_else(|| Fail::tool("--cwd needs a value".to_owned()))?
                    }
                };
                cwd = Some(PathBuf::from(v));
            }
            other => return Err(Fail::tool(format!("unknown flag: {other}"))),
        }
        i += 1;
    }
    let format = if json { Format::Json } else { Format::Human };
    Ok((format, cwd))
}

/// One workspace's view status, from either the daemon (warm telemetry) or a
/// cold stamp read (daemon absent).
struct Status {
    workspace: String,
    /// `daemon` (warm) or `cold` (daemon absent) or `absent` (NO_VIEW).
    source: &'static str,
    as_of: Option<String>,
    state: String,
    refresh_in_progress: bool,
    last_error: Option<Value>,
}

impl Status {
    /// Build from the daemon's `view_path` reply (v2 vocabulary — `as_of_root`).
    fn from_daemon(workspace: &Path, body: &Value) -> Self {
        let as_of = body
            .get("as_of_root")
            .or_else(|| body.get("as_of_fingerprint"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let state = body
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("UNKNOWN")
            .to_owned();
        let refresh_in_progress = body
            .get("refresh_in_progress")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let last_error = match body.get("last_error") {
            Some(Value::Null) | None => None,
            Some(other) => Some(other.clone()),
        };
        Status {
            workspace: workspace.display().to_string(),
            source: "daemon",
            as_of,
            state,
            refresh_in_progress,
            last_error,
        }
    }

    /// Build from a cold `view.duckdb` stamp (daemon absent): read the stamp's
    /// `as_of` READ_ONLY if the file is present, else NO_VIEW. Liveness is not
    /// folded here (that is `mrd sql`'s job), so `state` is `UNKNOWN`.
    fn from_cold(workspace: &Path, drawer_dir: Option<&Path>) -> Self {
        let dest = drawer_dir.map(|d| d.join("view.duckdb"));
        let as_of = dest.as_deref().filter(|p| p.is_file()).and_then(cold_as_of);
        let (source, state) = match &as_of {
            Some(_) => ("cold", "UNKNOWN"),
            None => ("absent", "NO_VIEW"),
        };
        Status {
            workspace: workspace.display().to_string(),
            source,
            as_of,
            state: state.to_owned(),
            refresh_in_progress: false,
            last_error: None,
        }
    }

    fn json(&self) -> String {
        let last_error = self.last_error.clone().map_or(Value::Null, |mut e| {
            // Enrich with a human `age_secs` beside the wire `unix` (OD7 basics).
            if let Some(unix) = e.get("unix").and_then(Value::as_u64) {
                e["age_secs"] = json!(age_secs(unix));
            }
            e
        });
        let doc = json!({
            "workspace": self.workspace,
            "source": self.source,
            "as_of": self.as_of,
            "state": self.state,
            "refresh_in_progress": self.refresh_in_progress,
            "last_error": last_error,
            // OD7 async half (round-2, V4) — no wire field carries these in V3.
            "consecutive_failures": Value::Null,
            "next_retry": Value::Null,
        });
        serde_json::to_string_pretty(&doc).unwrap_or_else(|_| doc.to_string())
    }

    fn print_human(&self) {
        println!("view status  workspace {}", self.workspace);
        println!("  source: {}", self.source);
        println!("  as_of:  {}", self.as_of.as_deref().unwrap_or("(none)"));
        println!("  state:  {}", self.state);
        println!("  refresh_in_progress: {}", self.refresh_in_progress);
        match &self.last_error {
            Some(err) => {
                let code = err.get("code").and_then(Value::as_str).unwrap_or("unknown");
                let fp = err
                    .get("fingerprint_attempted")
                    .and_then(Value::as_str)
                    .unwrap_or("(none)");
                let age = err
                    .get("unix")
                    .and_then(Value::as_u64)
                    .map_or_else(|| "unknown".to_owned(), |u| format!("{}s ago", age_secs(u)));
                println!("  last_error: {code} ({age}, fingerprint_attempted={fp})");
            }
            None => println!("  last_error: (none)"),
        }
        println!("  consecutive_failures: n/a (round-2 async executor)");
        println!("  next_retry:           n/a (round-2 async executor)");
    }
}

/// Read a cold `view.duckdb`'s `_meridian_view.as_of_fingerprint` READ_ONLY. A
/// present `.wal` sidecar OR a non-read-only file ⇒ do NOT open (B1 reader-side)
/// → `None` (reported as NO_VIEW / absent stamp).
fn cold_as_of(path: &Path) -> Option<String> {
    if crate::sql::wal_present(path) || !crate::sql::is_read_only(path) {
        return None;
    }
    let escaped = path.to_string_lossy().replace('\'', "''");
    let conn = Connection::open_in_memory().ok()?;
    conn.execute_batch(&format!(
        "ATTACH '{escaped}' AS meridian (READ_ONLY); USE meridian;"
    ))
    .ok()?;
    conn.query_row("SELECT as_of_fingerprint FROM _meridian_view", [], |r| {
        r.get::<_, String>(0)
    })
    .ok()
}

/// Whole seconds between `unix` and now (saturating).
fn age_secs(unix: u64) -> u64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    now.saturating_sub(unix)
}
