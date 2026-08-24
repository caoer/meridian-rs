//! Gates for `mrd sql` — the operator face over the drawer's append-only
//! `sql.duckdb` cache (direct-file lane), degrading to `:memory:` when no
//! cache root resolves. The real binary (`CARGO_BIN_EXE_mrd`) is driven over
//! its process boundary with an isolated cache root (`XDG_CACHE_HOME`) +
//! `HOME`; `MERIDIAN_DAEMON_BIN=/nonexistent` keeps any daemon out of the
//! frame.
//!
//! Covered:
//! - the cache lane folds post-result and reports an honest frame, creating
//!   `sql.duckdb` in the drawer on first use and appending delta-grain on
//!   corpus movement (pin ledger observable through the face itself);
//! - STALE and RACED surface when the corpus moves inside the §Q3 window
//!   (driven deterministically via the `MRD_SQL_TEST_MUTATE` hook);
//! - the ruled DML contract (OQ1): a latest VIEW refuses with the teaching;
//!   hist DML is accepted and never durable (always-rollback lane);
//! - `--rebuild` recreates the file at gen 1 (ruling OQ3), and refuses (exit
//!   2, holder + remedy named) rather than degrading to `:memory:` when the
//!   drawer is HELD — a repair verb that exits 0 rebuilt nothing;
//! - no cache root ⇒ the `:memory:` lane still answers (writes nothing, base
//!   tables accept DML that dies with the process);
//! - `--verify` refuses at the process boundary (dropped published-view flag);
//! - G14 versioned hash domain: the stamp is the workspace's own fold, prefix
//!   included.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

mod common;

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
        self.run_env(cwd, args, &[])
    }

    /// [`run`](Self::run) with NO resolvable cache root: both `XDG_CACHE_HOME`
    /// and `HOME` empty, so the drawer degrades ephemeral and the `:memory:`
    /// lane answers.
    #[allow(clippy::unused_self)] // rides the Sandbox for call-site symmetry with run/run_env
    fn run_no_cache_root(&self, cwd: &Path, args: &[&str]) -> Output {
        let mut cmd = Command::new(mrd_bin());
        cmd.env("XDG_CACHE_HOME", "")
            .env("HOME", "")
            .env_remove("MERIDIAN_WORKSPACE")
            .env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-daemon")
            .args(args)
            .current_dir(cwd);
        cmd.output().expect("spawn mrd")
    }

    /// [`run`](Self::run) with extra environment variables.
    fn run_env(&self, cwd: &Path, args: &[&str], envs: &[(&str, &str)]) -> Output {
        let mut cmd = Command::new(mrd_bin());
        cmd.env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
            .env_remove("MERIDIAN_WORKSPACE")
            .env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-daemon")
            .args(args)
            .current_dir(cwd);
        for (key, value) in envs {
            cmd.env(key, value);
        }
        cmd.output().expect("spawn mrd")
    }
}

/// A git-anchored workspace (no override) seeded with `files`. Anchored on
/// purpose: an unanchored tree now refuses (outside a declared workspace,
/// exit 2), and these arms test the sql lanes, not the resolution tier. The
/// `.git` entry is a directory, invisible to the markdown corpus.
fn write_bare_ws(sb: &Sandbox, name: &str, files: &[(&str, &str)]) -> PathBuf {
    let ws = sb.tmp.path().join(name);
    std::fs::create_dir_all(&ws).expect("ws");
    std::fs::create_dir_all(ws.join(".git")).expect(".git");
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
// the cache lane: FRESH, creates the drawer file, appends delta-grain
// ---------------------------------------------------------------------------

#[test]
fn sql_answers_fresh_and_creates_the_drawer_cache() {
    let sb = sandbox();
    let ws = write_bare_ws(
        &sb,
        "bare",
        &[
            ("a.md", "# A\n\nsee [[b]] and [[gone]]\n"),
            ("b.md", "# B\n"),
        ],
    );

    let out = sb.run(
        &ws,
        &["sql", "--json", "SELECT path FROM doc ORDER BY path"],
    );
    assert!(out.status.success(), "sql failed: {}", stderr(&out));
    let doc = json(&out);

    assert_eq!(
        doc["state"], "FRESH_AT_SAMPLE",
        "the cache lane folds post-result"
    );
    assert_eq!(doc["live_source"], "fold");
    assert_eq!(doc["stale"], Value::Bool(false));
    assert_eq!(doc["rows"], serde_json::json!([["a.md"], ["b.md"]]));

    // The drawer now holds the one cache file (ruling OQ4: `sql.duckdb`).
    let files = walk_count_suffix(&sb.cache_home, "sql.duckdb");
    assert_eq!(files, 1, "the direct-file lane creates the drawer cache");
}

#[test]
fn cache_appends_are_delta_grain_across_invocations() {
    let sb = sandbox();
    let ws = write_bare_ws(&sb, "delta", &[("a.md", "# A\n"), ("b.md", "# B\n")]);

    let first = sb.run(&ws, &["sql", "--json", "SELECT count(*) FROM hist.pin"]);
    assert!(first.status.success(), "{}", stderr(&first));
    assert_eq!(
        json(&first)["rows"],
        serde_json::json!([[1]]),
        "cold build = gen 1"
    );

    // Unmoved corpus: no new append.
    let warm = sb.run(&ws, &["sql", "--json", "SELECT count(*) FROM hist.pin"]);
    assert_eq!(
        json(&warm)["rows"],
        serde_json::json!([[1]]),
        "an unmoved corpus appends nothing"
    );

    // Move ONE file: exactly one gen-2 doc row rides the append.
    std::fs::write(ws.join("a.md"), "# A moved\n").expect("edit");
    let after = sb.run(
        &ws,
        &[
            "sql",
            "--json",
            "SELECT (SELECT count(*) FROM hist.pin), (SELECT count(*) FROM hist.doc WHERE gen = 2)",
        ],
    );
    assert!(after.status.success(), "{}", stderr(&after));
    assert_eq!(
        json(&after)["rows"],
        serde_json::json!([[2, 1]]),
        "one moved file = one appended doc version (O(k), never O(corpus))"
    );
}

#[test]
fn rebuild_recreates_the_file_at_gen_one() {
    let sb = sandbox();
    let ws = write_bare_ws(&sb, "rebuild", &[("a.md", "# A\n")]);

    sb.run(&ws, &["sql", "--json", "SELECT 1"]);
    std::fs::write(ws.join("a.md"), "# A moved\n").expect("edit");
    let grown = sb.run(&ws, &["sql", "--json", "SELECT max(gen) FROM hist.pin"]);
    assert_eq!(json(&grown)["rows"], serde_json::json!([[2]]));

    // The explicit rebuild verb (OQ3): the file restarts at gen 1.
    let rebuilt = sb.run(
        &ws,
        &[
            "sql",
            "--rebuild",
            "--json",
            "SELECT max(gen) FROM hist.pin",
        ],
    );
    assert!(rebuilt.status.success(), "{}", stderr(&rebuilt));
    assert_eq!(
        json(&rebuilt)["rows"],
        serde_json::json!([[1]]),
        "rebuild-and-swap restarts history"
    );
}

#[test]
fn no_cache_root_degrades_to_the_memory_lane() {
    let sb = sandbox();
    let ws = write_bare_ws(&sb, "nocache", &[("a.md", "# A\n")]);

    let out = sb.run_no_cache_root(&ws, &["sql", "--json", "SELECT path FROM doc"]);
    assert!(
        out.status.success(),
        "memory lane answers: {}",
        stderr(&out)
    );
    let doc = json(&out);
    assert_eq!(doc["state"], "FRESH_AT_SAMPLE");
    assert_eq!(doc["rows"], serde_json::json!([["a.md"]]));

    // And it wrote nothing anywhere under the sandbox.
    let stray = walk_count_suffix(sb.tmp.path(), ".duckdb");
    assert_eq!(stray, 0, "the :memory: lane writes no database file");
}

/// The repair verb never degrades (card
/// `mrd-sql-rebuild-silent-noop-under-daemon-lock`). With the drawer HELD —
/// the resident daemon owns it under lifecycle B; an in-test `SqlStore`
/// stands in for that holder, taking the same `DuckDB` inter-process lock —
/// `--rebuild` must refuse (exit 2) naming the holder and the verb that WOULD
/// rebuild, NOT answer from `:memory:` at exit 0 with the drawer untouched:
/// that success rebuilt nothing, and the caller then measures the OLD drawer
/// believing it repaired.
#[test]
fn rebuild_refuses_while_the_drawer_is_held() {
    let sb = sandbox();
    let ws = write_bare_ws(&sb, "held", &[("a.md", "# A\n")]);

    // Cold-build so there IS a drawer file to hold.
    let cold = sb.run(&ws, &["sql", "--json", "SELECT 1"]);
    assert!(cold.status.success(), "{}", stderr(&cold));
    let drawer = walk_find_suffix(&sb.cache_home, "sql.duckdb").expect("the drawer cache file");

    // Hold it for the whole arm. `DuckDB` locks with POSIX `fcntl` records, and
    // those are dropped when the process closes ANY descriptor on the file —
    // so this arm must never OPEN the drawer again, not even to read it. The
    // identity snapshot is `stat` only (measured: an `fs::read` here released
    // the hold, the child rebuilt happily, and the arm failed on exit 0).
    let _holder = view::store::SqlStore::open(&drawer).expect("hold the drawer");
    let before = drawer_identity(&drawer);

    // Control: prove the hold is REAL before asserting on what it causes. A
    // plain query degrades, voiced — no line, and every assertion below would
    // be vacuous rather than failing honestly.
    let control = sb.run(&ws, &["sql", "SELECT 1"]);
    assert!(
        stderr(&control).contains("cache file unavailable"),
        "the in-test holder must actually hold the DuckDB lock, else this arm proves nothing\nstderr: {}",
        stderr(&control),
    );

    for args in [
        &["sql", "--rebuild", "SELECT 1"][..],
        &["sql", "--rebuild"][..],
    ] {
        let out = sb.run(&ws, args);
        assert_eq!(
            out.status.code(),
            Some(2),
            "`mrd {}` under a held drawer must refuse, not exit 0 having rebuilt nothing\nstdout: {}\nstderr: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stdout),
            stderr(&out),
        );
        let err = stderr(&out);
        assert!(
            err.contains("nothing was rebuilt"),
            "the refusal says what did NOT happen: {err}"
        );
        assert!(
            err.contains("Conflicting lock"),
            "the refusal names the holder (DuckDB's message carries binary + PID): {err}"
        );
        assert!(
            err.contains("mrd unregister"),
            "the refusal names the verb that WOULD rebuild: {err}"
        );
    }

    assert_eq!(
        drawer_identity(&drawer),
        before,
        "a refused rebuild leaves the drawer where it was — rebuild-and-swap moves the inode"
    );
}

/// `(inode, size, mtime)` of `path`, read by `stat` alone: identifying the
/// drawer must not cost a descriptor (see the holder comment above), and the
/// inode is exactly what rebuild-and-swap moves.
fn drawer_identity(path: &Path) -> (u64, u64, std::time::SystemTime) {
    use std::os::unix::fs::MetadataExt as _;
    let meta = std::fs::metadata(path).expect("stat the drawer");
    (meta.ino(), meta.size(), meta.modified().expect("mtime"))
}

/// The first file whose name ends in `suffix` anywhere under `dir`.
fn walk_find_suffix(dir: &Path, suffix: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(hit) = walk_find_suffix(&path, suffix) {
                return Some(hit);
            }
        } else if path
            .file_name()
            .and_then(|f| f.to_str())
            .is_some_and(|f| f.ends_with(suffix))
        {
            return Some(path);
        }
    }
    None
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
// STALE / RACED — the corpus moves inside the §Q3 window
// ---------------------------------------------------------------------------

/// The §Q3 window (build → post-result fold) is intra-process, so the mutation
/// is injected deterministically: `MRD_SQL_TEST_MUTATE` appends one line to the
/// named file before every post-result fold. Without `--fresh` the first
/// mismatch is final: STALE at the F0 build.
#[test]
fn stale_surfaces_when_the_corpus_moves_inside_the_window() {
    let sb = sandbox();
    let ws = write_bare_ws(&sb, "stale", &[("a.md", "# A\n")]);
    let mutated = ws.join("mut.md");

    let out = sb.run_env(
        &ws,
        &["sql", "--json", "SELECT path FROM doc ORDER BY path"],
        &[("MRD_SQL_TEST_MUTATE", mutated.to_str().unwrap())],
    );
    assert!(
        out.status.success(),
        "a STALE frame is a success: {}",
        stderr(&out)
    );
    let doc = json(&out);

    assert_eq!(doc["state"], "STALE", "as_of != live at the sample: {doc}");
    assert_eq!(doc["stale"], Value::Bool(true));
    assert_eq!(doc["live_source"], "fold", "a real post-result fold ran");
    assert_eq!(
        doc["rows"],
        serde_json::json!([["a.md"]]),
        "the rows are the F0 build's — the mutation post-dates them"
    );
}

/// `--fresh` gets one bounded retry; when the corpus moves again inside the
/// retry's window too, the bound is spent and the frame reports RACED — never
/// a silent STALE-as-FRESH, never an unbounded loop.
#[test]
fn raced_surfaces_when_a_bounded_fresh_cannot_converge() {
    let sb = sandbox();
    let ws = write_bare_ws(&sb, "raced", &[("a.md", "# A\n")]);
    let mutated = ws.join("mut.md");

    let out = sb.run_env(
        &ws,
        &[
            "sql",
            "--json",
            "--fresh",
            "SELECT path FROM doc ORDER BY path",
        ],
        &[("MRD_SQL_TEST_MUTATE", mutated.to_str().unwrap())],
    );
    assert!(
        out.status.success(),
        "a RACED frame is a success: {}",
        stderr(&out)
    );
    let doc = json(&out);

    assert_eq!(doc["state"], "RACED", "the retry could not converge: {doc}");
    assert_eq!(doc["stale"], Value::Bool(true));
    assert_eq!(doc["live_source"], "fold");
    assert_eq!(
        doc["rows"],
        serde_json::json!([["a.md"], ["mut.md"]]),
        "the retry REBUILT: its rows carry the first mutation"
    );
}

// ---------------------------------------------------------------------------
// the ruled DML contract (OQ1): views refuse with teaching, hist is
// ephemeral, the :memory: lane keeps the old base-table acceptance
// ---------------------------------------------------------------------------

/// On the cache lane the projection names are VIEWS over append-only history:
/// DML against them refuses through `DuckDB`'s own error, extended with the
/// hist-lane remedy (ruling OQ1 — the refusal teaches).
#[test]
fn dml_against_a_latest_view_refuses_with_the_teaching() {
    let sb = sandbox();
    let ws = write_bare_ws(&sb, "dmlview", &[("a.md", "# A\n")]);

    let out = sb.run(
        &ws,
        &[
            "sql",
            "--json",
            "INSERT INTO doc VALUES ('ghost.md', '0000000000000000', 1, 1)",
        ],
    );
    assert!(
        out.status.success(),
        "--json buffers the refusal: {}",
        stderr(&out)
    );
    let doc = json(&out);
    assert_eq!(doc["state"], "UNVERIFIED", "no rows to certify: {doc}");
    let error = doc["error"].as_str().expect("the refusal rides the doc");
    assert!(
        error.contains("hist"),
        "the refusal teaches the hist lane (OQ1): {error}"
    );
}

/// DML against the hist tables is accepted and never durable: the
/// always-rollback query lane keeps the writes-nothing-durable contract on a
/// persistent file.
#[test]
fn dml_against_hist_is_accepted_and_never_durable() {
    let sb = sandbox();
    let ws = write_bare_ws(&sb, "dmlhist", &[("a.md", "# A\n")]);

    let out = sb.run(
        &ws,
        &[
            "sql",
            "--json",
            "INSERT INTO hist.doc (path, gen, tombstone) VALUES ('ghost.md', 999, false)",
        ],
    );
    assert!(
        out.status.success(),
        "hist DML is accepted: {}",
        stderr(&out)
    );
    let doc = json(&out);
    assert_eq!(doc["error"], Value::Null, "no refusal: {doc}");

    // The write died at ROLLBACK: the next invocation sees no ghost.
    let after = sb.run(
        &ws,
        &[
            "sql",
            "--json",
            "SELECT count(*) FROM hist.doc WHERE path = 'ghost.md'",
        ],
    );
    assert_eq!(
        json(&after)["rows"],
        serde_json::json!([[0]]),
        "the rollback lane keeps hist DML ephemeral"
    );
}

/// The `:memory:` lane keeps the pinned pre-cache contract: the projection
/// tables are base tables, DML is accepted and dies with the process.
#[test]
fn memory_lane_dml_is_accepted_and_dies_with_the_process() {
    let sb = sandbox();
    let ws = write_bare_ws(&sb, "dmlmem", &[("a.md", "# A\n")]);

    let out = sb.run_no_cache_root(
        &ws,
        &[
            "sql",
            "--json",
            "INSERT INTO doc VALUES ('ghost.md', '0000000000000000', 1, 1)",
        ],
    );
    assert!(out.status.success(), "DML is accepted: {}", stderr(&out));
    let doc = json(&out);
    assert_eq!(doc["error"], Value::Null, "no refusal: {doc}");
    assert_eq!(
        doc["state"], "FRESH_AT_SAMPLE",
        "the post-result fold still matches — the INSERT reached no disk: {doc}"
    );
}

// ---------------------------------------------------------------------------
// `--verify` refuses at the process boundary
// ---------------------------------------------------------------------------

/// `--verify` belonged to the dropped published-view path; the ephemeral build
/// always folds, so accept-and-ignore would lie. The unit test pins the parse;
/// this pins the process boundary: non-zero exit, unknown-flag message.
#[test]
fn verify_flag_is_refused_at_the_process_boundary() {
    let sb = sandbox();
    let ws = write_bare_ws(&sb, "verify", &[("a.md", "# A\n")]);

    let out = sb.run(&ws, &["sql", "--verify", "SELECT 1"]);
    assert!(
        !out.status.success(),
        "--verify must refuse, not silently no-op"
    );
    let err = stderr(&out);
    assert!(
        err.contains("unknown flag: --verify"),
        "the refusal names the flag: {err}"
    );
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
        ephemeral_as_of.starts_with("b3c:"),
        "the stamp carries the domain's version-2 prefix: {ephemeral_as_of}"
    );
    assert_eq!(
        ephemeral["rows"],
        serde_json::json!([["plan.md"]]),
        "the answer itself is correct"
    );
}

// ---------------------------------------------------------------------------
// ladder rung 1: a resident daemon holding the root serves EVERY caller's
// queries over the wire (§ A.11 — one ladder, NO-SANDBOX ruling 2026-08-14)
// ---------------------------------------------------------------------------

/// A daemon lives at the sandbox's DEFAULT socket (the one the CLI derives
/// from `XDG_CACHE_HOME`), holding the workspace's cache file. A plain
/// `mrd sql` routes through it — observable because the wire lane appends
/// through the DAEMON's open handle while the file stays held (a direct
/// open would refuse, and a `:memory:` degrade would answer without
/// touching hist at all).
#[test]
fn sql_routes_through_a_resident_daemon() {
    use std::time::Duration;
    let sb = sandbox();
    let ws = write_bare_ws(&sb, "daemonized", &[("a.md", "# A\n")]);

    // The daemon at the default socket for this XDG_CACHE_HOME.
    let cache_root = sb.cache_home.join("meridian");
    let registry_dir = cache_root.join("registry");
    std::fs::create_dir_all(&registry_dir).expect("registry dir");
    #[allow(clippy::duration_suboptimal_units)]
    let forever = Duration::from_secs(365 * 24 * 60 * 60);
    let mut config = registry::Config::for_cache_root(cache_root);
    config.socket_path = common::child_socket_path(&sb.home, &sb.cache_home);
    config.state_path = registry_dir.join("state.json");
    config.idle_threshold = forever;
    config.reap_interval = forever;
    config.prewarm_interval = forever;
    config.prewarm_quiet_max = forever;
    config.idle_exit = None;
    let server = registry::RunningServer::start(config).expect("daemon");

    // Warm the daemon's ownership of the file: one wire sql through it.
    let out = sb.run(&ws, &["sql", "--json", "SELECT path FROM doc"]);
    assert!(out.status.success(), "daemon route: {}", stderr(&out));
    let doc = json(&out);
    assert_eq!(doc["state"], "FRESH_AT_SAMPLE", "{doc}");
    assert_eq!(doc["rows"], serde_json::json!([["a.md"]]));

    // The daemon holds the file now: a second call still answers, and
    // hist is reachable — proof the answer came through the held file, not a
    // :memory: degrade (which has no hist schema).
    let pins = sb.run(&ws, &["sql", "--json", "SELECT count(*) FROM hist.pin"]);
    assert!(pins.status.success(), "{}", stderr(&pins));
    assert_eq!(
        json(&pins)["rows"],
        serde_json::json!([[1]]),
        "one cold build, warm since"
    );

    server.shutdown();
}
