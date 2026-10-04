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

/// `link.candidates` through the cache lane: the set a written target matches
/// when there is a choice, and a later page that joins the set without
/// moving the pick reaches the unchanged linking page on the next append.
#[test]
fn link_candidates_list_the_choice_and_follow_a_page_added_later() {
    let sb = sandbox();
    let ws = write_bare_ws(&sb, "candidates", &[("p.md", "# P\n\n[[x]]\n[[y]]\n")]);
    for (rel, content) in [
        ("a/x.md", "# A x\n"),
        ("b/x.md", "# B x\n"),
        ("y.md", "# Y\n"),
    ] {
        let file = ws.join(rel);
        std::fs::create_dir_all(file.parent().expect("parent")).expect("dir");
        std::fs::write(file, content).expect("write");
    }
    let query = "SELECT target_raw, dest_path, candidates FROM link ORDER BY target_raw";

    let out = sb.run(&ws, &["sql", "--json", query]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        json(&out)["rows"],
        serde_json::json!([["x", "a/x.md", ["a/x.md", "b/x.md"]], ["y", "y.md", null]])
    );

    // A longer path joins the set; the pick stays a/x.md and p.md is unchanged.
    std::fs::create_dir_all(ws.join("c/deep")).expect("dir");
    std::fs::write(ws.join("c/deep/x.md"), "# Deep x\n").expect("write");
    let out = sb.run(&ws, &["sql", "--json", query]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        json(&out)["rows"],
        serde_json::json!([
            ["x", "a/x.md", ["a/x.md", "b/x.md", "c/deep/x.md"]],
            ["y", "y.md", null]
        ])
    );
}

/// `frontmatter_link` through `mrd sql`: a frontmatter wikilink is a row with
/// its key, resolved the way a body link is, and a page added later reaches
/// the unchanged linking page on the next append. `link` stays body-only.
#[test]
fn frontmatter_links_are_rows_of_their_own_relation() {
    let sb = sandbox();
    let ws = write_bare_ws(
        &sb,
        "fm-links",
        &[(
            "p.md",
            "---\nsource: \"[[x]]\"\nsee: [\"[[y]]\", \"[[#^here]]\"]\n---\n# P\n\n[[y]]\n",
        )],
    );
    for (rel, content) in [("a/x.md", "# A x\n"), ("y.md", "# Y\n")] {
        let file = ws.join(rel);
        std::fs::create_dir_all(file.parent().expect("parent")).expect("dir");
        std::fs::write(file, content).expect("write");
    }
    let query = "SELECT key, target_raw, dest_path, candidates FROM frontmatter_link ORDER BY seq";

    let out = sb.run(&ws, &["sql", "--json", query]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        json(&out)["rows"],
        serde_json::json!([["source", "x", "a/x.md", null], ["see", "y", "y.md", null]])
    );
    let body = sb.run(&ws, &["sql", "--json", "SELECT target_raw FROM link"]);
    assert_eq!(json(&body)["rows"], serde_json::json!([["y"]]));

    std::fs::create_dir_all(ws.join("b")).expect("dir");
    std::fs::write(ws.join("b/x.md"), "# B x\n").expect("write");
    let out = sb.run(&ws, &["sql", "--json", query]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        json(&out)["rows"],
        serde_json::json!([
            ["source", "x", "a/x.md", ["a/x.md", "b/x.md"]],
            ["see", "y", "y.md", null]
        ])
    );
}

/// `lock` through `mrd sql`: one row per pin, its object resolved the way the
/// walk resolves it and a heading path spelled as `section.hpath`, and a page
/// added later reaches the unchanged pinning page on the next append.
#[test]
fn pins_are_rows_of_the_lock_relation() {
    let sb = sandbox();
    let ws = write_bare_ws(&sb, "lock-rows", &[]);
    let pinning = "# P\n\n```meridian-lock\nversion: 2\npins:\n\
                   \x20 - object: \"[[x]]\"\n\
                   \x20   hash: \"h1\"\n\
                   \x20   path: [\"A x\", \"Q3\"]\n\
                   \x20   fingerprint: \"fp1.span2.b3.aa\"\n\
                   \x20 - object: \"[[x]]\"\n\
                   \x20   hash: \"h1\"\n\
                   \x20   properties: [\"tags\"]\n\
                   \x20   fingerprint: \"fp1.props1.b3.bb\"\n\
                   ```\n";
    for (rel, content) in [("c/p.md", pinning), ("a/x.md", "# A x\n\n## Q3\n")] {
        let file = ws.join(rel);
        std::fs::create_dir_all(file.parent().expect("parent")).expect("dir");
        std::fs::write(file, content).expect("write");
    }
    let query = "SELECT object, dest_path, arm, selector, hpath FROM lock ORDER BY seq";

    let out = sb.run(&ws, &["sql", "--json", query]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        json(&out)["rows"],
        serde_json::json!([
            [
                "x",
                "a/x.md",
                "path",
                r#"["A x","Q3"]"#,
                r#"[{"h":"A x"},{"h":"Q3"}]"#
            ],
            ["x", "a/x.md", "properties", r#"["tags"]"#, null]
        ])
    );

    // A sibling x appears; c/p.md is unchanged and both pins move to it.
    std::fs::write(ws.join("c/x.md"), "# C x\n").expect("write");
    let out = sb.run(&ws, &["sql", "--json", "SELECT dest_path FROM lock"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        json(&out)["rows"],
        serde_json::json!([["c/x.md"], ["c/x.md"]])
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
    assert_eq!(
        out.status.code(),
        Some(2),
        "a refused query exits 2 under --json too: {}",
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
// a SQL error exits 2 in both modes; under --json it also rides an UNVERIFIED
// document on stdout
// ---------------------------------------------------------------------------

#[test]
fn sql_error_exits_2_with_an_unverified_document_under_json() {
    let sb = sandbox();
    let ws = write_bare_ws(&sb, "err", &[("a.md", "# A\n")]);

    for query in ["SELEC 1", "SELECT nope FROM missing"] {
        let plain = sb.run(&ws, &["sql", query]);
        assert_eq!(
            plain.status.code(),
            Some(2),
            "{query}: a failed query exits 2: {}",
            stderr(&plain)
        );
    }

    let parser = sb.run(&ws, &["sql", "--json", "SELEC 1"]);
    assert_eq!(
        parser.status.code(),
        Some(2),
        "a parser error exits 2 under --json: {}",
        stderr(&parser)
    );
    let doc = json(&parser);
    assert_eq!(doc["state"], "UNVERIFIED", "no rows to certify: {doc}");
    assert!(doc["error"].is_string(), "the error rides the doc: {doc}");

    let out = sb.run(&ws, &["sql", "--json", "SELECT nope FROM missing"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "a binder error exits 2 under --json: {}",
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
    config.drain_cold_builds = Duration::from_secs(30);
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

/// A daemon refusal must stay on the resident route. The local fixture would
/// answer successfully if the CLI fell through to corpus loading/projection.
#[test]
fn sql_retry_refusal_never_projects_locally() {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;
    for disconnect_after_first in [false, true] {
        let sb = sandbox();
        let ws = write_bare_ws(&sb, "busy", &[("a.md", "# A\n")]);
        let socket = common::child_socket_path(&sb.home, &sb.cache_home);
        std::fs::create_dir_all(socket.parent().unwrap()).unwrap();
        let listener = UnixListener::bind(&socket).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(10)))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut queries = 0;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap() == 0 {
                    break;
                }
                let request: Value = serde_json::from_str(&line).unwrap();
                let response = if request["op"] == "hello" {
                    serde_json::json!({"ok": true, "body": {"caps": ["sql"]}})
                } else {
                    queries += 1;
                    serde_json::json!({"ok": false, "error": {
                        "code": if queries < 3 { "workspace_busy" } else { "lock_timeout" }, "recovery": "retry",
                        "message": "projection lease occupied; retry"
                    }})
                };
                writeln!(stream, "{response}").unwrap();
                if disconnect_after_first && queries == 1 {
                    break;
                }
            }
            queries
        });
        let out = sb.run(&ws, &["sql", "--json", "SELECT path FROM doc"]);
        let queries = server.join().unwrap();
        assert_eq!(
            out.status.code(),
            Some(2),
            "busy must not fall back: {}",
            stderr(&out)
        );
        let frame = json(&out);
        assert_eq!(frame["state"], "NO_VIEW", "{frame}");
        assert!(
            frame["error"]
                .as_str()
                .unwrap()
                .contains("projection lease occupied"),
            "{frame}"
        );
        assert_eq!(queries, 1, "refusal is not automatically replayed");
        assert_eq!(
            walk_count_suffix(&sb.cache_home, "sql.duckdb"),
            0,
            "no local projection"
        );
    }
}

#[test]
fn fresh_requery_stays_resident_after_stale_and_lost_or_failed_reply() {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;
    for disconnect in [true, false] {
        let sb = sandbox();
        let ws = write_bare_ws(&sb, "fresh-busy", &[("a.md", "# A\n")]);
        let socket = common::child_socket_path(&sb.home, &sb.cache_home);
        std::fs::create_dir_all(socket.parent().unwrap()).unwrap();
        let listener = UnixListener::bind(socket).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(10)))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut queries = 0;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap() == 0 {
                    break;
                }
                let request: Value = serde_json::from_str(&line).unwrap();
                let response = if request["op"] == "hello" {
                    serde_json::json!({"ok":true,"body":{"caps":["sql"]}})
                } else {
                    queries += 1;
                    match queries {
                        1 => {
                            serde_json::json!({"ok":true,"body":{"as_of_fingerprint":"b3c:one","live":"b3c:two","state":"STALE","columns":[],"rows":[]}})
                        }
                        2 if !disconnect => {
                            serde_json::json!({"ok":false,"error":{"code":"io_error","recovery":"env","cause":"fixture refusal"}})
                        }
                        _ => break,
                    }
                };
                writeln!(stream, "{response}").unwrap();
            }
            queries
        });
        let out = sb.run(&ws, &["sql", "--json", "--fresh", "SELECT path FROM doc"]);
        assert_eq!(server.join().unwrap(), 2);
        assert_eq!(
            out.status.code(),
            Some(2),
            "a resident stale result must forbid fallback across --fresh: {}",
            stderr(&out)
        );
        assert_eq!(json(&out)["state"], "NO_VIEW");
        assert_eq!(walk_count_suffix(&sb.cache_home, "sql.duckdb"), 0);
    }
}

#[test]
fn ordinary_query_edit_overlap_succeeds_through_the_cli() {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    use std::time::{Duration, Instant};
    let sb = sandbox();
    let ws = write_bare_ws(&sb, "overlap", &[("a.md", "# A\n")]);
    let mut config = registry::Config::for_cache_root(sb.cache_home.join("meridian"));
    config.socket_path = common::child_socket_path(&sb.home, &sb.cache_home);
    config.idle_exit = None;
    config.drain_cold_builds = Duration::from_secs(30);
    let server = registry::RunningServer::start(config).unwrap();
    let query = "SELECT path FROM doc ORDER BY path";
    assert!(sb.run(&ws, &["sql", "--json", query]).status.success());
    let socket = server.socket_path().to_path_buf();
    let holder_ws = ws.clone();
    let holder = std::thread::spawn(move || {
        let mut writer = UnixStream::connect(socket).unwrap();

        let mut reader = BufReader::new(writer.try_clone().unwrap());
        let mut call = |request: Value| {
            writeln!(writer, "{request}").unwrap();
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            serde_json::from_str::<Value>(&line).unwrap()
        };
        assert_eq!(
            call(serde_json::json!({"op":"hello","proto":1,"contract":"v3","workspace":holder_ws}))
                ["ok"],
            true
        );
        let started = Instant::now();
        let answer = call(serde_json::json!({"id":1,"op":"sql","query":
            "WITH RECURSIVE spin(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM spin WHERE i<40000) SELECT count(*)::BIGINT FROM spin"}));
        (started.elapsed(), answer)
    });
    std::thread::sleep(Duration::from_millis(750));
    assert!(!holder.is_finished(), "query must be executing at the edit");
    std::fs::write(ws.join("b.md"), "# B\n").unwrap();
    let started = Instant::now();
    let out = sb.run(&ws, &["sql", "--json", query]);
    let elapsed = started.elapsed();
    let (held, answer) = holder.join().unwrap();
    server.shutdown();
    eprintln!(
        "ordinary overlap: holder={held:?}, cli={elapsed:?}, exit={:?}",
        out.status.code()
    );
    assert!(held > Duration::from_millis(1500));
    assert_eq!(answer["ok"], true);
    assert_eq!(
        out.status.code(),
        Some(0),
        "finite ordinary overlap must succeed: {}",
        stderr(&out)
    );
    assert_eq!(json(&out)["rows"], serde_json::json!([["a.md"], ["b.md"]]));
}

/// Reproduce the shared-session boundary: two SQL requests and an unrelated
/// read are sent while another connection holds a SQL snapshot past 10 s.
/// A FIFO keeps the holder in real DuckDB I/O without CPU-dependent spin time.
#[test]
fn pipelined_sql_contenders_do_not_timeout_an_unrelated_read() {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::net::UnixStream;
    use std::time::{Duration, Instant};

    fn exchange(
        writer: &mut UnixStream,
        reader: &mut BufReader<UnixStream>,
        value: &Value,
    ) -> Value {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            writeln!(writer, "{value}").unwrap();
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let answer: Value = serde_json::from_str(&line).unwrap();
            if answer["error"]["code"] != "corpus_warming" {
                return answer;
            }
            assert!(Instant::now() < deadline, "fixture did not warm: {answer}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    let sb = sandbox();
    let ws = write_bare_ws(&sb, "pipelined", &[("a.md", "# A\n")]);
    let mut config = registry::Config::for_cache_root(sb.cache_home.join("meridian"));
    config.socket_path = common::child_socket_path(&sb.home, &sb.cache_home);
    config.idle_exit = None;
    config.drain_cold_builds = Duration::from_secs(30);
    let server = registry::RunningServer::start(config).unwrap();
    assert!(
        sb.run(&ws, &["sql", "--json", "SELECT path FROM doc"])
            .status
            .success()
    );
    let fifo = sb.tmp.path().join("hold.csv");
    assert!(
        Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    let (opened, ready) = std::sync::mpsc::channel();
    let (start_hold, hold_started) = std::sync::mpsc::channel();
    let feeder = std::thread::spawn({
        let fifo = fifo.clone();
        move || {
            let deadline = Instant::now() + Duration::from_secs(20);
            let mut file = loop {
                match std::fs::OpenOptions::new()
                    .write(true)
                    .custom_flags(libc::O_NONBLOCK)
                    .open(&fifo)
                {
                    Ok(file) => break file,
                    Err(error)
                        if error.raw_os_error() == Some(libc::ENXIO)
                            && Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("FIFO query never opened: {error}"),
                }
            };
            opened.send(()).unwrap();
            hold_started.recv_timeout(Duration::from_secs(20)).unwrap();
            std::thread::sleep(Duration::from_secs(12));
            file.write_all(b"1\n").unwrap();
        }
    });
    let mut holder_writer = UnixStream::connect(server.socket_path()).unwrap();
    holder_writer
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let mut holder_reader = BufReader::new(holder_writer.try_clone().unwrap());
    let hello = serde_json::json!({"op":"hello","proto":1,"contract":"v3","workspace":ws});
    assert_eq!(
        exchange(&mut holder_writer, &mut holder_reader, &hello)["ok"],
        true
    );
    let holder = std::thread::spawn(move || {
        let started = Instant::now();
        let path = fifo.display().to_string().replace('\'', "''");
        let query = format!(
            "SELECT count(*) FROM read_csv('{path}', auto_detect=false, header=false, columns={{'n':'INTEGER'}}, parallel=false)"
        );
        let answer = exchange(
            &mut holder_writer,
            &mut holder_reader,
            &serde_json::json!({"id":1,"op":"sql","query":query}),
        );
        (started.elapsed(), answer)
    });
    ready
        .recv_timeout(Duration::from_secs(20))
        .expect("holder is executing SQL");
    std::fs::write(ws.join("b.md"), "# B\n").unwrap();
    let mut writer = UnixStream::connect(server.socket_path()).unwrap();
    let mut reader = BufReader::new(writer.try_clone().unwrap());
    assert_eq!(exchange(&mut writer, &mut reader, &hello)["ok"], true);
    // This read warms the changed corpus without advancing the SQL cache.
    assert_eq!(
        exchange(
            &mut writer,
            &mut reader,
            &serde_json::json!({"op":"read","path":"b.md"})
        )["ok"],
        true
    );
    let started = Instant::now();
    start_hold.send(()).unwrap();
    for id in [11, 12] {
        writeln!(
            writer,
            "{}",
            serde_json::json!({"id":id,"op":"sql","query":"SELECT path FROM doc ORDER BY path"})
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(100));
    }
    writeln!(
        writer,
        "{}",
        serde_json::json!({"id":13,"op":"read","path":"a.md"})
    )
    .unwrap();
    let mut answers = Vec::new();
    let mut transport_error = None;
    for _ in 0..3 {
        reader
            .get_ref()
            .set_read_timeout(Some(
                Duration::from_secs(10)
                    .saturating_sub(started.elapsed())
                    .max(Duration::from_millis(1)),
            ))
            .unwrap();
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => {
                transport_error = Some("closed".to_owned());
                break;
            }
            Ok(_) => answers.push(serde_json::from_str::<Value>(&line).unwrap()),
            Err(error) => {
                transport_error = Some(error.to_string());
                break;
            }
        }
    }
    let elapsed = started.elapsed();
    drop(reader);
    drop(writer);
    feeder.join().unwrap();
    let (held, answer) = holder.join().unwrap();
    server.shutdown();
    eprintln!(
        "pipelined SQL/read: holder={held:?}, replies={elapsed:?}, fault={transport_error:?}, answers={answers:?}"
    );
    assert!(
        held >= Duration::from_secs(10),
        "holder must outlast the transport budget"
    );
    assert_eq!(
        answer["body"]["rows"],
        serde_json::json!([[1]]),
        "real CSV query: {answer}"
    );
    assert!(
        transport_error.is_none(),
        "SQL wait retired the pooled session: {transport_error:?}"
    );
    assert_eq!(answers.len(), 3);
    for (answer, id) in answers.iter().zip([11, 12, 13]) {
        assert_eq!(answer["id"], id);
        assert_eq!(answer["ok"], true, "ordinary overlap: {answer}");
    }
    assert!(elapsed < Duration::from_secs(10));
}
