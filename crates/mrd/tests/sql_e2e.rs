//! Gates for `mrd sql` — the ephemeral `:memory:` operator face. The real
//! binary (`CARGO_BIN_EXE_mrd`) is driven over its process boundary with an
//! isolated cache root (`XDG_CACHE_HOME`) + `HOME`;
//! `MERIDIAN_DAEMON_BIN=/nonexistent` keeps any daemon out of the frame — the
//! build is in-process by design, never a degrade.
//!
//! Covered:
//! - the ephemeral build folds post-result and reports an honest frame;
//! - it writes NOTHING on disk;
//! - G14 versioned hash domain: the stamp is the workspace's own fold, prefix
//!   included.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

fn mrd_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mrd")
}

/// An isolated cache root + HOME under one tempdir.
struct Sandbox {
    tmp: tempfile::TempDir,
    cache_home: PathBuf,
    home: PathBuf,
}

fn sandbox() -> Sandbox {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cache_home = tmp.path().join("xdg-cache");
    let home = tmp.path().join("home");
    std::fs::create_dir_all(&home).expect("home");
    Sandbox {
        tmp,
        cache_home,
        home,
    }
}

impl Sandbox {
    /// Run `mrd <args>` from `cwd` with NO reachable daemon.
    fn run(&self, cwd: &Path, args: &[&str]) -> Output {
        Command::new(mrd_bin())
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
            .env_remove("MERIDIAN_WORKSPACE")
            .env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-daemon")
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("spawn mrd")
    }
}

/// A bare workspace (no git, no override) seeded with `files` — resolves as
/// the cwd default.
fn write_bare_ws(sb: &Sandbox, name: &str, files: &[(&str, &str)]) -> PathBuf {
    let ws = sb.tmp.path().join(name);
    std::fs::create_dir_all(&ws).expect("ws");
    for (rel, content) in files {
        std::fs::write(ws.join(rel), content).expect("write");
    }
    ws
}

fn json(out: &Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not JSON ({e}): {}\nstderr: {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    })
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

// ---------------------------------------------------------------------------
// the ephemeral build: FRESH, writes nothing on disk
// ---------------------------------------------------------------------------

#[test]
fn ephemeral_query_is_fresh_and_writes_nothing() {
    let sb = sandbox();
    let ws = write_bare_ws(
        &sb,
        "bare",
        &[
            ("a.md", "# A\n\nsee [[b]] and [[gone]]\n"),
            ("b.md", "# B\n"),
        ],
    );

    let out = sb.run(&ws, &["sql", "--json", "SELECT path FROM doc ORDER BY path"]);
    assert!(out.status.success(), "sql failed: {}", stderr(&out));
    let doc = json(&out);

    assert_eq!(
        doc["state"], "FRESH_AT_SAMPLE",
        "ephemeral folds post-result"
    );
    assert_eq!(doc["live_source"], "fold");
    assert_eq!(doc["stale"], Value::Bool(false));
    assert_eq!(doc["rows"], serde_json::json!([["a.md"], ["b.md"]]));

    // The ephemeral build writes NOTHING: no database file anywhere under the
    // cache root.
    let stray = walk_count_suffix(&sb.cache_home, ".duckdb");
    assert_eq!(stray, 0, "the ephemeral build must write no database file");
}

/// Count files whose name ends in `suffix` anywhere under `dir` (missing dir ⇒ 0).
fn walk_count_suffix(dir: &Path, suffix: &str) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut n = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            n += walk_count_suffix(&path, suffix);
        } else if path
            .file_name()
            .and_then(|f| f.to_str())
            .is_some_and(|f| f.ends_with(suffix))
        {
            n += 1;
        }
    }
    n
}

// ---------------------------------------------------------------------------
// a SQL error is a buffered UNVERIFIED frame under --json, loud in human mode
// ---------------------------------------------------------------------------

#[test]
fn sql_error_is_buffered_unverified_under_json() {
    let sb = sandbox();
    let ws = write_bare_ws(&sb, "err", &[("a.md", "# A\n")]);

    let out = sb.run(&ws, &["sql", "--json", "SELECT nope FROM missing"]);
    assert!(
        out.status.success(),
        "--json buffers the SQL error: {}",
        stderr(&out)
    );
    let doc = json(&out);
    assert_eq!(doc["state"], "UNVERIFIED", "no rows to certify: {doc}");
    assert_eq!(doc["live_source"], "none");
    assert_eq!(doc["stale"], Value::Null);
    assert!(doc["error"].is_string(), "the error rides the doc: {doc}");
}

// ---------------------------------------------------------------------------
// G14 — a versioned hash domain: the stamp is the workspace's own fold,
// prefix included
// ---------------------------------------------------------------------------

/// The ephemeral build stamps the workspace's own fold, prefix included:
/// `mdfs_config.yaml` declares `version: 2`, so every honest fold of this
/// corpus is a `b3b:` token (§12.3), and a refold under a hardcoded version
/// would report STALE over a correct, current answer.
#[test]
fn g14_versioned_domain_ephemeral_stamp_carries_the_domain_prefix() {
    let sb = sandbox();
    let ws = write_bare_ws(
        &sb,
        "g14",
        &[
            ("mdfs_config.yaml", "version: 2\n"),
            ("plan.md", "---\ntype: task\nstatus: todo\n---\n# Plan\n"),
        ],
    );

    let out = sb.run(&ws, &["sql", "--json", "SELECT path FROM doc"]);
    assert!(out.status.success(), "ephemeral sql: {}", stderr(&out));
    let ephemeral = json(&out);
    assert_eq!(
        ephemeral["state"], "FRESH_AT_SAMPLE",
        "a correct current ephemeral answer is FRESH, never STALE: {ephemeral}"
    );
    assert_eq!(ephemeral["stale"], Value::Bool(false));
    let ephemeral_as_of = ephemeral["as_of_fingerprint"].as_str().unwrap();
    assert!(
        ephemeral_as_of.starts_with("b3b:"),
        "the stamp carries the domain's version-2 prefix: {ephemeral_as_of}"
    );
    assert_eq!(
        ephemeral["rows"],
        serde_json::json!([["plan.md"]]),
        "the answer itself is correct"
    );
}
