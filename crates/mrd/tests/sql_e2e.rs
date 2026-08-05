//! V3 gates for `mrd sql` / `mrd view status` — the client side of the `DuckDB`
//! view-organ (design `tournament-duckdb/team-a` §Q5). The real binary
//! (`CARGO_BIN_EXE_mrd`) is driven over its process boundary with an isolated
//! cache root (`XDG_CACHE_HOME`) + `HOME`; `MERIDIAN_DAEMON_BIN=/nonexistent`
//! forces the daemon-absent degrade where a test wants it.
//!
//! Covered (task §Quality Gates / §Encode as tests):
//! - §Q5 gate: query returns correct rows; staleness surfaced (`as_of != live`
//!   after a mutation → STALE, both fingerprints travel); daemon-absent degrades
//!   by tier; the `view_path` client path against a live daemon.
//! - OD8 default-degrade: the default query path returns `live_source=none,
//!   stale=null` (UNVERIFIED) unless `--verify`/`--fresh`.
//! - RO-attach INSERT refusal (gate 7).
//! - No-WAL replay defense (gate 14 reader side): a present `.wal` ⇒ never opened.
//! - Tier-4 ephemeral writes NOTHING on disk.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::Duration;

use serde_json::Value;

fn mrd_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mrd")
}

/// An isolated cache root + HOME under one tempdir.
struct Sandbox {
    tmp: tempfile::TempDir,
    cache_home: PathBuf,
    home: PathBuf,
    cache_root: PathBuf,
}

fn sandbox() -> Sandbox {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cache_home = tmp.path().join("xdg-cache");
    let home = tmp.path().join("home");
    std::fs::create_dir_all(&home).expect("home");
    let cache_root = cache_home.join("meridian");
    Sandbox {
        tmp,
        cache_home,
        home,
        cache_root,
    }
}

impl Sandbox {
    /// Base command with the isolated env. `daemon_bin`: `None` forces the
    /// daemon-absent degrade (auto-spawn points at a nonexistent binary); `Some`
    /// lets `mrd` auto-spawn/dial the real daemon.
    fn base(&self, daemon_bin: Option<&str>) -> Command {
        let mut cmd = Command::new(mrd_bin());
        cmd.env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
            .env_remove("MERIDIAN_WORKSPACE");
        match daemon_bin {
            Some(bin) => cmd.env("MERIDIAN_DAEMON_BIN", bin),
            None => cmd.env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-daemon"),
        };
        cmd
    }

    /// Run `mrd <args>` from `cwd` with NO reachable daemon (degrade path).
    fn run_no_daemon(&self, cwd: &Path, args: &[&str]) -> Output {
        self.base(None)
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("spawn mrd")
    }

    /// Run `mrd <args>` from `cwd` allowing daemon dial/auto-spawn.
    fn run_with_daemon(&self, cwd: &Path, args: &[&str]) -> Output {
        self.base(Some(mrd_bin()))
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("spawn mrd")
    }
}

/// A git-anchored workspace `ws` seeded with `files` — both `mrd` and the
/// daemon resolve it to the same canonical directory.
fn write_anchored_ws(sb: &Sandbox, name: &str, files: &[(&str, &str)]) -> (PathBuf, PathBuf) {
    let ws = sb.tmp.path().join(name);
    std::fs::create_dir_all(&ws).expect("ws");
    // Anchored by a `.git` entry — the ladder's structural rung.
    std::fs::create_dir_all(ws.join(".git")).expect("git anchor");
    for (rel, content) in files {
        let path = ws.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(path, content).expect("write");
    }
    let canonical = std::fs::canonicalize(&ws).expect("canonicalize");
    (ws, canonical)
}

/// An unanchored workspace (no git, no override) seeded with `files` — the
/// ladder answers nothing, so it resolves as the cwd default.
fn write_bare_ws(sb: &Sandbox, name: &str, files: &[(&str, &str)]) -> PathBuf {
    let ws = sb.tmp.path().join(name);
    std::fs::create_dir_all(&ws).expect("ws");
    for (rel, content) in files {
        std::fs::write(ws.join(rel), content).expect("write");
    }
    ws
}

/// Pre-publish a `view.duckdb` for `canonical_ws` into the sandbox drawer,
/// exactly where `mrd`'s cold degrade resolves it (the daemon is the sole
/// persistent writer; the test stands in for it). Returns the published path.
fn publish_view(sb: &Sandbox, canonical_ws: &Path) -> PathBuf {
    let root = fs::WorkspaceRoot(canonical_ws.to_path_buf());
    let (files, fingerprint) = fs::domain_snapshot(&root).expect("snapshot");
    let (_index, docs): (model::CorpusIndex, BTreeMap<String, model::Document>) =
        fs::build_corpus(files).expect("build corpus");

    let dir = cache::drawer_dir(&sb.cache_root, canonical_ws);
    std::fs::create_dir_all(&dir).expect("drawer dir");
    let drawer = cache::CacheDrawer::Disk {
        dir,
        workspace: canonical_ws.to_path_buf(),
    };
    let stamp = view::PublishStamp {
        workspace: canonical_ws.to_string_lossy().into_owned(),
        as_of_fingerprint: fingerprint.0,
        epoch: "test-epoch".to_owned(),
        seq: 0,
    };
    view::publish(&docs, &drawer, &stamp).expect("publish")
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
// tier-4 ephemeral: FRESH, writes nothing on disk
// ---------------------------------------------------------------------------

#[test]
fn tier4_ephemeral_query_is_fresh_and_writes_nothing() {
    let sb = sandbox();
    let ws = write_bare_ws(
        &sb,
        "bare",
        &[
            ("a.md", "# A\n\nsee [[b]] and [[gone]]\n"),
            ("b.md", "# B\n"),
        ],
    );

    let out = sb.run_no_daemon(
        &ws,
        &["sql", "--json", "SELECT path FROM doc ORDER BY path"],
    );
    assert!(out.status.success(), "sql failed: {}", stderr(&out));
    let doc = json(&out);

    assert_eq!(
        doc["state"], "FRESH_AT_SAMPLE",
        "ephemeral folds post-result"
    );
    assert_eq!(doc["live_source"], "fold");
    assert_eq!(doc["stale"], Value::Bool(false));
    assert!(
        doc.get("changes_seq").is_none(),
        "daemonless ⇒ changes_seq omitted: {doc}"
    );
    assert_eq!(doc["rows"], serde_json::json!([["a.md"], ["b.md"]]));

    // Tier-4 writes NOTHING: no drawer, no view.duckdb anywhere under the cache.
    let stray = walk_count(&sb.cache_home, "view.duckdb");
    assert_eq!(
        stray, 0,
        "tier-4 ephemeral must write no view.duckdb on disk"
    );
}

/// Count files named `name` anywhere under `dir` (missing dir ⇒ 0).
fn walk_count(dir: &Path, name: &str) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut n = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            n += walk_count(&path, name);
        } else if path.file_name().and_then(|f| f.to_str()) == Some(name) {
            n += 1;
        }
    }
    n
}

// ---------------------------------------------------------------------------
// OD8 default-degrade over a published view: UNVERIFIED unless --verify/--fresh
// ---------------------------------------------------------------------------

#[test]
fn od8_default_is_unverified_verify_folds() {
    let sb = sandbox();
    let (ws, canonical) = write_anchored_ws(
        &sb,
        "od8",
        &[("plan.md", "---\ntype: task\nstatus: todo\n---\n# Plan\n")],
    );
    publish_view(&sb, &canonical);

    // Default (no --verify/--fresh), daemon absent → cold-open the published file,
    // run NO fold: live_source=none, stale=null, state=UNVERIFIED (OD8).
    let out = sb.run_no_daemon(&ws, &["sql", "--json", "SELECT path FROM doc"]);
    assert!(out.status.success(), "default sql: {}", stderr(&out));
    let doc = json(&out);
    assert_eq!(
        doc["state"], "UNVERIFIED",
        "OD8 default runs no fold: {doc}"
    );
    assert_eq!(doc["live_source"], "none", "degrade is none, never watch");
    assert_eq!(doc["stale"], Value::Null, "no fold ⇒ stale null");
    assert_eq!(
        doc["rows"],
        serde_json::json!([["plan.md"]]),
        "rows correct"
    );

    // --verify opts into the post-result fold; quiescent ⇒ FRESH_AT_SAMPLE.
    let out = sb.run_no_daemon(&ws, &["sql", "--json", "--verify", "SELECT path FROM doc"]);
    assert!(out.status.success(), "verify sql: {}", stderr(&out));
    let doc = json(&out);
    assert_eq!(doc["state"], "FRESH_AT_SAMPLE", "--verify folds: {doc}");
    assert_eq!(doc["live_source"], "fold");
    assert_eq!(doc["stale"], Value::Bool(false));
}

// ---------------------------------------------------------------------------
// §Q5 gate: staleness surfaced — as_of != live after a mutation → STALE
// ---------------------------------------------------------------------------

#[test]
fn q5_stale_surfaces_after_mutation_both_fingerprints_travel() {
    let sb = sandbox();
    let (ws, canonical) = write_anchored_ws(&sb, "stale", &[("a.md", "# A\n")]);
    let published = publish_view(&sb, &canonical);
    let published_as_of = read_stamp_as_of(&published);

    // Mutate the corpus AFTER publish: disk moves off the stamped fingerprint.
    std::fs::write(ws.join("a.md"), "# A changed\n\nmore\n").unwrap();

    // --verify, daemon absent → cold-open the last-good file (as_of = the stamp),
    // fold live (mutated disk) → as_of != live ⇒ a visible STALE frame.
    let out = sb.run_no_daemon(
        &ws,
        &["sql", "--json", "--verify", "SELECT count(*) AS n FROM doc"],
    );
    assert!(out.status.success(), "stale sql: {}", stderr(&out));
    let doc = json(&out);
    assert_eq!(
        doc["state"], "STALE",
        "a mutated corpus is a visible STALE frame: {doc}"
    );
    assert_eq!(doc["live_source"], "fold");
    assert_eq!(doc["stale"], Value::Bool(true));
    // Both fingerprints travel: as_of is the published stamp; the banner/live moved.
    assert_eq!(
        doc["as_of_fingerprint"].as_str().unwrap(),
        published_as_of,
        "STALE still reads the last-good published as_of"
    );

    // The human banner surfaces STALE with both fingerprints.
    let human = sb.run_no_daemon(&ws, &["sql", "--verify", "SELECT count(*) FROM doc"]);
    let banner = String::from_utf8_lossy(&human.stdout);
    assert!(banner.contains("STALE"), "human banner is STALE: {banner}");
    assert!(
        banner.contains("as_of=") && banner.contains("live="),
        "both fingerprints: {banner}"
    );
}

// ---------------------------------------------------------------------------
// G14 — a versioned hash domain: the ephemeral stamp and the daemon stamp are
// the SAME fold, so identical state gets the same verdict on both paths
// ---------------------------------------------------------------------------

/// **The ephemeral build stamps the workspace's own fold, prefix included.**
///
/// `mdfs_config.yaml` declares `version: 2`, so every honest fold of this corpus
/// is a `b3b:` token (§12.3). The ephemeral build used to REFOLD the projected
/// docs under a hardcoded version 0 — same hex, `b3:` prefix — so the
/// post-result comparison against the live `b3b:` fold reported STALE over a
/// correct, current answer (G14, dogfood pass 3).
///
/// The two arms run over the SAME unmutated state, ephemeral first (nothing is
/// published yet, so the daemon-absent degrade lands on the ephemeral build).
#[test]
fn g14_versioned_domain_ephemeral_and_daemon_agree_on_identical_state() {
    let sb = sandbox();
    let (ws, _canonical) = write_anchored_ws(
        &sb,
        "g14",
        &[
            ("mdfs_config.yaml", "version: 2\n"),
            ("plan.md", "---\ntype: task\nstatus: todo\n---\n# Plan\n"),
        ],
    );

    // Arm A — forced ephemeral: no reachable daemon, no published view.
    let out = sb.run_no_daemon(&ws, &["sql", "--json", "--verify", "SELECT path FROM doc"]);
    assert!(out.status.success(), "ephemeral sql: {}", stderr(&out));
    let ephemeral = json(&out);
    assert_eq!(
        ephemeral["state"], "FRESH_AT_SAMPLE",
        "a correct current ephemeral answer is FRESH, never STALE: {ephemeral}"
    );
    assert_eq!(ephemeral["stale"], Value::Bool(false));
    let ephemeral_as_of = ephemeral["as_of_fingerprint"].as_str().unwrap().to_owned();
    assert!(
        ephemeral_as_of.starts_with("b3b:"),
        "the stamp carries the domain's version-2 prefix: {ephemeral_as_of}"
    );
    assert_eq!(
        ephemeral["rows"],
        serde_json::json!([["plan.md"]]),
        "the answer itself is correct"
    );

    // Arm B — a warm daemon over the same unmutated state: same verdict, and the
    // SAME fingerprint. Two paths that fold the same corpus cannot disagree.
    let _daemon = start_daemon(&sb);
    let out = sb.run_with_daemon(&ws, &["sql", "--json", "--verify", "SELECT path FROM doc"]);
    assert!(out.status.success(), "daemon sql: {}", stderr(&out));
    let warm = json(&out);
    assert_eq!(
        warm["state"], "FRESH_AT_SAMPLE",
        "the warm arm is fresh on the same state: {warm}"
    );
    assert_eq!(
        warm["as_of_fingerprint"].as_str().unwrap(),
        ephemeral_as_of,
        "identical state ⇒ identical stamp on both paths"
    );
}

/// **The fix does not trade a false STALE for a false FRESH.** In the same
/// version-2 domain, a corpus that really moved still reports STALE — and so
/// does a DOMAIN that moved while every markdown byte stayed put (`version: 2`
/// → `3`), which is the case a body-only comparison would wrongly call fresh.
#[test]
fn g14_versioned_domain_still_reports_a_genuine_stale() {
    let sb = sandbox();
    let (ws, canonical) = write_anchored_ws(
        &sb,
        "g14-stale",
        &[("mdfs_config.yaml", "version: 2\n"), ("a.md", "# A\n")],
    );
    let published = publish_view(&sb, &canonical);
    let published_as_of = read_stamp_as_of(&published);
    assert!(
        published_as_of.starts_with("b3b:"),
        "the published stamp is the version-2 fold: {published_as_of}"
    );

    // 1. The bytes moved.
    std::fs::write(ws.join("a.md"), "# A changed\n").unwrap();
    let out = sb.run_no_daemon(
        &ws,
        &["sql", "--json", "--verify", "SELECT count(*) AS n FROM doc"],
    );
    assert!(out.status.success(), "mutated sql: {}", stderr(&out));
    let doc = json(&out);
    assert_eq!(
        doc["state"], "STALE",
        "a mutated corpus is still STALE under a versioned domain: {doc}"
    );
    assert_eq!(doc["stale"], Value::Bool(true));

    // 2. The DOMAIN moved, the bytes did not: restore the byte, bump the domain
    //    version. The hex repeats; the prefix does not — `b3b:` vs `b3c:` — and
    //    a cursor from the old world is exactly what §12.3 refuses to call fresh.
    std::fs::write(ws.join("a.md"), "# A\n").unwrap();
    std::fs::write(ws.join("mdfs_config.yaml"), "version: 3\n").unwrap();
    let out = sb.run_no_daemon(
        &ws,
        &["sql", "--json", "--verify", "SELECT count(*) AS n FROM doc"],
    );
    assert!(out.status.success(), "rebumped sql: {}", stderr(&out));
    let doc = json(&out);
    assert_eq!(
        doc["state"], "STALE",
        "a domain-version bump is STALE even with identical bytes: {doc}"
    );
    assert_eq!(
        doc["as_of_fingerprint"].as_str().unwrap(),
        published_as_of,
        "the published `b3b:` stamp still travels"
    );
}

/// Read `_meridian_view.as_of_fingerprint` from a published file (`READ_ONLY`).
fn read_stamp_as_of(path: &Path) -> String {
    let conn = duckdb::Connection::open_in_memory().unwrap();
    let escaped = path.to_string_lossy().replace('\'', "''");
    conn.execute_batch(&format!("ATTACH '{escaped}' AS v (READ_ONLY); USE v;"))
        .unwrap();
    conn.query_row("SELECT as_of_fingerprint FROM _meridian_view", [], |r| {
        r.get::<_, String>(0)
    })
    .unwrap()
}

// ---------------------------------------------------------------------------
// gate 7: INSERT on a READ_ONLY attach is refused
// ---------------------------------------------------------------------------

#[test]
fn ro_attach_insert_is_refused() {
    let sb = sandbox();
    let (ws, canonical) = write_anchored_ws(&sb, "ro", &[("a.md", "# A\n")]);
    publish_view(&sb, &canonical);

    let out = sb.run_no_daemon(
        &ws,
        &[
            "sql",
            "INSERT INTO doc (path, file_rev, line_count, bytes) VALUES ('x','y',1,1)",
        ],
    );
    assert!(!out.status.success(), "INSERT on a RO view must fail");
    let err = stderr(&out).to_lowercase();
    assert!(
        err.contains("read-only") || err.contains("read only"),
        "the refusal names read-only mode: {err}"
    );
}

// ---------------------------------------------------------------------------
// gate 14 (reader side): a present .wal ⇒ never open the published file
// ---------------------------------------------------------------------------

#[test]
fn no_wal_replay_defense_refuses_shadowed_file() {
    let sb = sandbox();
    // Publish corpus A, then move disk to corpus B, then shadow the published
    // file with a `.wal`. A managed reader must NOT open (and replay) the
    // shadowed A-generation file — it degrades to an ephemeral build of live B.
    let (ws, canonical) = write_anchored_ws(&sb, "wal", &[("a.md", "# A\n")]);
    let published = publish_view(&sb, &canonical);

    std::fs::write(ws.join("b.md"), "# B\n").unwrap(); // corpus is now {a, b}
    let wal = wal_sidecar(&published);
    std::fs::write(&wal, b"STALE-WAL-GEN").unwrap();

    // --verify so a fold runs and the frame is decisive.
    let out = sb.run_no_daemon(
        &ws,
        &["sql", "--json", "--verify", "SELECT count(*) AS n FROM doc"],
    );
    assert!(
        out.status.success(),
        "sql over a wal-shadowed file: {}",
        stderr(&out)
    );
    let doc = json(&out);
    // The ephemeral build sees LIVE disk (2 docs), never the shadowed file's rows.
    assert_eq!(
        doc["rows"],
        serde_json::json!([[2]]),
        "reader built live corpus, never replayed the stale WAL: {doc}"
    );
    assert_eq!(
        doc["state"], "FRESH_AT_SAMPLE",
        "ephemeral build of live disk is fresh"
    );
}

fn wal_sidecar(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".wal");
    PathBuf::from(s)
}

// ---------------------------------------------------------------------------
// §Q5 gate: the view_path client path against a LIVE daemon
// ---------------------------------------------------------------------------

#[test]
fn q5_daemon_served_view_path_client_is_fresh_with_rows() {
    let sb = sandbox();
    let (ws, _canonical) = write_anchored_ws(
        &sb,
        "served",
        &[("plan.md", "---\ntype: task\nstatus: todo\n---\n# Plan\n")],
    );
    let _daemon = start_daemon(&sb);

    // A live daemon publishes via view_path; --verify folds post-result → FRESH,
    // and changes_seq rides the daemon path (OD9).
    let out = sb.run_with_daemon(
        &ws,
        &["sql", "--json", "--verify", "SELECT path, type FROM card"],
    );
    assert!(out.status.success(), "daemon-served sql: {}", stderr(&out));
    let doc = json(&out);
    assert_eq!(
        doc["state"], "FRESH_AT_SAMPLE",
        "daemon-served + --verify is fresh: {doc}"
    );
    assert_eq!(doc["live_source"], "fold");
    assert_eq!(doc["stale"], Value::Bool(false));
    assert!(
        doc.get("changes_seq").is_some(),
        "daemon path carries changes_seq: {doc}"
    );
    assert_eq!(
        doc["rows"],
        serde_json::json!([["plan.md", "task"]]),
        "rows correct"
    );

    // `mrd view status` renders the daemon's OD7 telemetry (as_of/state).
    let out = sb.run_with_daemon(&ws, &["view", "status", "--json"]);
    assert!(out.status.success(), "view status: {}", stderr(&out));
    let status = json(&out);
    assert_eq!(status["source"], "daemon");
    assert!(
        status["as_of"]
            .as_str()
            .is_some_and(|s| s.starts_with("b3:")),
        "view status reports a b3 as_of: {status}"
    );
}

/// Start an in-process daemon bound to the sandbox's default socket, so the
/// `mrd` subprocess dials it via `default_socket_path()`.
#[allow(clippy::duration_suboptimal_units)]
fn start_daemon(sb: &Sandbox) -> registry::RunningServer {
    let reg_dir = sb.cache_root.join("registry");
    std::fs::create_dir_all(&reg_dir).expect("registry dir");
    let never = Duration::from_secs(365 * 24 * 60 * 60);
    let config = registry::Config {
        socket_path: reg_dir.join("daemon.sock"),
        state_path: reg_dir.join("state.json"),
        cache_root: sb.cache_root.clone(),
        idle_threshold: never,
        reap_interval: never,
        prewarm_interval: never,
        prewarm_quiet_max: never,
        // No idle exit: this server's lifetime is the test's, and a daemon that
        // reaped itself mid-assertion would fail as a flake, not a finding.
        idle_exit: None,
        push_write_timeout: registry::DEFAULT_PUSH_WRITE_TIMEOUT,
        sub_idle_write_timeout: registry::DEFAULT_SUB_IDLE_WRITE_TIMEOUT,
        // No build identity configured: this fixture is not testing the hello
        // identity field, and an absent sha is the honest state for a server
        // started from a test harness rather than a deployed binary.
        build_sha: None,
    };
    registry::RunningServer::start(config).expect("daemon start")
}
