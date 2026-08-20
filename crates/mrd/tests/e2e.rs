//! End-to-end gates for the `mrd` CLI, driving the REAL binary (`CARGO_BIN_EXE_mrd`) over its
//! process boundary with an overridden cache root (`XDG_CACHE_HOME`) and `HOME`. This is the
//! phase integration evidence: the landed `workspace` / `cache` / `registry` crates wired into
//! the settled verb surface, exercised as an operator would.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

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
    fn base(&self, program: &str) -> Command {
        let mut cmd = Command::new(program);
        cmd.env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
            .env_remove("MERIDIAN_WORKSPACE");
        cmd
    }

    /// Run `mrd <args>` from `cwd`, capturing the output.
    fn run(&self, cwd: &Path, args: &[&str]) -> Output {
        self.base(mrd_bin())
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("spawn mrd")
    }

    fn dir(&self, rel: &str) -> PathBuf {
        let p = self.tmp.path().join(rel);
        std::fs::create_dir_all(&p).expect("mkdir");
        p
    }
}

fn json(out: &Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not JSON ({e}): {}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

// ---------------------------------------------------------------------------
// Gate: bare tree → init → drawer+sentinel → cache ls → unregister
// ---------------------------------------------------------------------------

#[test]
fn e2e_init_ls_unregister_lifecycle() {
    let sb = sandbox();
    let ws = sb.dir("project");
    let canonical = std::fs::canonicalize(&ws).unwrap();

    let out = sb.run(&ws, &["init"]);
    assert!(out.status.success(), "init failed: {}", stderr(&out));

    // init's product is the root's own self-declaration, read back through the owner of what
    // a valid declaration is (`config`) rather than asserting the file merely exists.
    let declaration = ws.join("MERIDIAN.md");
    assert!(declaration.exists(), "the root declaration is written");
    let decl = config::mount::read_root_declaration(&canonical)
        .unwrap_or_else(|_| panic!("`config` reads init's declaration as a root declaration"));
    assert_eq!(decl.name, "project", "named after the directory");

    // The drawer holds a valid sentinel carrying the canonical workspace path.
    let drawer = cache::drawer_dir(&sb.cache_root, &canonical);
    match cache::probe(&drawer) {
        cache::Probe::Hit(s) => {
            assert_eq!(s.workspace, canonical.to_string_lossy(), "sentinel path");
            assert!(s.superseded_by.is_none(), "a fresh drawer is not retired");
        }
        cache::Probe::Miss => panic!("init must leave a valid drawer sentinel"),
    }

    // `cache ls --json` reports exactly that drawer.
    let out = sb.run(&ws, &["cache", "ls", "--json"]);
    assert!(out.status.success(), "cache ls: {}", stderr(&out));
    let rows = json(&out);
    let rows = rows.as_array().expect("ls emits a JSON array");
    assert_eq!(rows.len(), 1, "one drawer listed");
    assert_eq!(rows[0]["workspace"], canonical.to_string_lossy().as_ref());

    // `unregister` retires (removes) the drawer.
    let out = sb.run(&ws, &["unregister"]);
    assert!(out.status.success(), "unregister: {}", stderr(&out));
    assert!(!drawer.exists(), "unregister removes the drawer directory");
}

// ---------------------------------------------------------------------------
// Gate M2: a tier-4 descendant drawer is marked superseded_by an ancestor init
// ---------------------------------------------------------------------------

#[test]
fn e2e_m2_init_supersedes_descendant_drawer() {
    let sb = sandbox();
    let ancestor = sb.dir("mono");
    let descendant = sb.dir("mono/packages/leaf");
    let canon_anc = std::fs::canonicalize(&ancestor).unwrap();
    let canon_desc = std::fs::canonicalize(&descendant).unwrap();

    // A tier-4 leftover: register the descendant's drawer directly.
    let desc_drawer = cache::drawer_dir(&sb.cache_root, &canon_desc);
    cache::register(&desc_drawer, &canon_desc).unwrap();

    // init at the ancestor reconciles the shadowed descendant.
    let out = sb.run(&ancestor, &["init"]);
    assert!(out.status.success(), "init: {}", stderr(&out));

    match cache::probe(&desc_drawer) {
        cache::Probe::Hit(s) => assert_eq!(
            s.superseded_by.as_deref(),
            Some(canon_anc.to_string_lossy().as_ref()),
            "descendant retired, stamped with the ancestor"
        ),
        cache::Probe::Miss => panic!("the retired descendant must still be a valid sentinel"),
    }
}

/// The M2 guard: a descendant that is its own root is not retired.
///
/// The ladder returns the nearest `.git`, so a descendant repository resolves to itself —
/// stamping its drawer `superseded_by` would make a later `cache clean` reap a live
/// workspace's cache.
#[test]
fn e2e_m2_spares_a_descendant_that_is_its_own_git_root() {
    let sb = sandbox();
    let ancestor = sb.dir("mono");
    let own_root = sb.dir("mono/vendor/lib");
    std::fs::create_dir_all(own_root.join(".git")).expect("git anchor");
    let shadowed = sb.dir("mono/packages/leaf");
    let canon_own = std::fs::canonicalize(&own_root).unwrap();
    let canon_shadowed = std::fs::canonicalize(&shadowed).unwrap();

    let own_drawer = cache::drawer_dir(&sb.cache_root, &canon_own);
    cache::register(&own_drawer, &canon_own).unwrap();
    let shadowed_drawer = cache::drawer_dir(&sb.cache_root, &canon_shadowed);
    cache::register(&shadowed_drawer, &canon_shadowed).unwrap();

    let out = sb.run(&ancestor, &["init"]);
    assert!(out.status.success(), "init: {}", stderr(&out));

    match cache::probe(&own_drawer) {
        cache::Probe::Hit(s) => assert!(
            s.superseded_by.is_none(),
            "a descendant that anchors ITSELF keeps its drawer: {:?}",
            s.superseded_by
        ),
        cache::Probe::Miss => panic!("the spared drawer must still be a valid sentinel"),
    }
    // The precondition that makes the assertion above discriminating: the
    // unanchored sibling in the same run IS retired, so a reconcile that did
    // nothing at all could not pass this test.
    match cache::probe(&shadowed_drawer) {
        cache::Probe::Hit(s) => assert!(
            s.superseded_by.is_some(),
            "an unanchored descendant is still retired — M2 survives"
        ),
        cache::Probe::Miss => panic!("the retired drawer must still be a valid sentinel"),
    }
}

// ---------------------------------------------------------------------------
// Gate: cwd-default, no daemon → OUTSIDE a workspace: exit 2, NOTHING written
// under the cache root (2026-08-20: the ephemeral adopt walked a 75-repo
// parent for ~21 s; strict resolution refuses instead)
// ---------------------------------------------------------------------------

#[test]
fn e2e_tier4_no_daemon_refuses_and_writes_nothing() {
    let sb = sandbox();
    let bare = sb.dir("bare");
    assert!(
        !sb.cache_root.exists(),
        "precondition: cache root not yet created"
    );

    let out = sb.run(&bare, &["resolve", "--json"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "an unanchored tree with no daemon is outside a workspace — exit 2: {}",
        stderr(&out)
    );
    assert!(
        stderr(&out).contains("outside a declared meridian workspace"),
        "the refusal names the cause: {}",
        stderr(&out)
    );

    assert!(
        !sb.cache_root.exists(),
        "a tier-4 refusal must write nothing under the cache root"
    );
}

// ---------------------------------------------------------------------------
// Gate: downgrade — a future-schema sentinel is a cold start, exit 0
// ---------------------------------------------------------------------------

#[test]
fn e2e_downgrade_future_schema_sentinel_is_cold_exit0() {
    let sb = sandbox();
    let ws = sb.dir("marked");
    // Anchored by a `.git` entry.
    std::fs::create_dir_all(ws.join(".git")).unwrap();
    let canonical = std::fs::canonicalize(&ws).unwrap();

    // Plant a future-schema sentinel a newer binary would have written.
    let drawer = cache::drawer_dir(&sb.cache_root, &canonical);
    std::fs::create_dir_all(&drawer).unwrap();
    std::fs::write(
        drawer.join("registered.json"),
        br#"{"schema":999,"workspace":"whatever","created_at":0,"last_use":0}"#,
    )
    .unwrap();

    let out = sb.run(&ws, &["resolve", "--json"]);
    assert!(
        out.status.success(),
        "a future-schema drawer must cold-start with exit 0: {}",
        stderr(&out)
    );
    let v = json(&out);
    assert_eq!(
        v["source"], "git-root",
        "the `.git` anchor still resolves the workspace"
    );
    assert_eq!(
        v["state"], "cold",
        "future-schema sentinel probes as a miss"
    );
}

// ---------------------------------------------------------------------------
// Gate: deny ceiling — init in $HOME refused, exit 2, typed reason
// ---------------------------------------------------------------------------

#[test]
fn e2e_init_in_home_is_denied_exit2() {
    let sb = sandbox();
    let out = sb.run(&sb.home, &["init"]);
    assert_eq!(out.status.code(), Some(2), "deny ceiling → exit 2");
    assert!(
        stderr(&out).contains("home directory"),
        "typed deny reason on stderr: {}",
        stderr(&out)
    );
    // Refused before any write: no declaration, no drawer. `MERIDIAN.md` is the reserved
    // filename the machine config uses at $HOME, so the deny ceiling is what keeps init from
    // clobbering it.
    assert!(
        !sb.home.join("MERIDIAN.md").exists(),
        "no declaration written into $HOME"
    );
}

// ---------------------------------------------------------------------------
// Gate: init declares the root, and SAYS which tier/root the ladder answers
// ---------------------------------------------------------------------------

/// A tree declared below a git root still resolves to the git root — the declaration plane
/// is `config`'s, not the ladder's — and init must say so.
#[test]
fn e2e_init_below_a_git_root_names_the_tier_and_root_it_resolves_to() {
    let sb = sandbox();
    let repo = sb.dir("repo");
    std::fs::create_dir_all(repo.join(".git")).expect("git anchor");
    let inner = sb.dir("repo/packages/leaf");
    let canon_repo = std::fs::canonicalize(&repo).unwrap();
    let canon_inner = std::fs::canonicalize(&inner).unwrap();

    let out = sb.run(&inner, &["init", "--json"]);
    assert!(out.status.success(), "init: {}", stderr(&out));
    let v = json(&out);

    assert_eq!(
        v["workspace"],
        canon_inner.to_string_lossy().as_ref(),
        "init declared the directory it was pointed at"
    );
    assert_eq!(v["declaration_state"], "created");
    assert_eq!(
        v["resolved_tier"], "git-root",
        "the ladder answers git-root, not the declaration"
    );
    assert_eq!(
        v["resolved_root"],
        canon_repo.to_string_lossy().as_ref(),
        "and it names the REPO, not the declared directory"
    );
    assert_eq!(
        v["declared_root_is_resolved"], false,
        "init states the two are not the same directory"
    );

    // The human surface teaches the fix rather than leaving the change to be discovered.
    let out = sb.run(&inner, &["init"]);
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        text.contains("resolves: git-root"),
        "the human report names the tier and root: {text}"
    );
    assert!(
        text.contains("MERIDIAN_WORKSPACE="),
        "and names the way to change the answer: {text}"
    );
}

/// Re-init over an existing valid declaration leaves it byte-for-byte and says
/// `already declared` — a re-init never rewrites content it did not author.
#[test]
fn e2e_reinit_leaves_a_valid_declaration_untouched() {
    let sb = sandbox();
    let ws = sb.dir("project");
    let declaration = ws.join("MERIDIAN.md");
    let authored = "---\ntype: meridian-root\nversion: 1\nname: hand-written\n---\n\n# Mine\n";
    std::fs::write(&declaration, authored).expect("write");

    let out = sb.run(&ws, &["init", "--json"]);
    assert!(out.status.success(), "init: {}", stderr(&out));
    let v = json(&out);
    assert_eq!(v["declaration_state"], "already declared");
    assert_eq!(
        v["declared_name"], "hand-written",
        "the name comes from the file, never from the directory"
    );
    assert_eq!(
        std::fs::read_to_string(&declaration).unwrap(),
        authored,
        "byte-for-byte untouched"
    );
}

/// A `MERIDIAN.md` that is present but does NOT read as a root declaration
/// refuses (exit 2) and is left byte-untouched — the same law `mrd skill hook`'s
/// document states for a hook this engine did not write. The obvious accident is the
/// machine config copied into a tree.
#[test]
fn e2e_init_refuses_a_meridian_md_that_is_not_a_root_declaration() {
    let sb = sandbox();
    let ws = sb.dir("project");
    let declaration = ws.join("MERIDIAN.md");
    let foreign = "---\ntype: meridian-config\nversion: 1\n---\n\n# Machine config\n";
    std::fs::write(&declaration, foreign).expect("write");

    let out = sb.run(&ws, &["init"]);
    assert_eq!(out.status.code(), Some(2), "refuses: {}", stderr(&out));
    let said = stderr(&out);
    assert!(
        said.contains("does not read as a root declaration"),
        "the refusal names what is wrong: {said}"
    );
    assert!(
        said.contains("meridian-root"),
        "and quotes the owner's own reason: {said}"
    );
    assert_eq!(
        std::fs::read_to_string(&declaration).unwrap(),
        foreign,
        "byte-untouched"
    );
    // Refused before the drawer, too: init is all-or-nothing on this path.
    let canonical = std::fs::canonicalize(&ws).unwrap();
    let drawer = cache::drawer_dir(&sb.cache_root, &canonical);
    assert!(!drawer.exists(), "no drawer registered on a refusal");
}

/// A directory name that is not a canonical root name refuses, leaves NOTHING
/// on disk, and teaches `--name`; `--name` then succeeds.
#[test]
fn e2e_init_refuses_an_unnameable_directory_and_leaves_no_declaration() {
    let sb = sandbox();
    let ws = sb.dir("Project Root");
    let declaration = ws.join("MERIDIAN.md");

    let out = sb.run(&ws, &["init"]);
    assert_eq!(out.status.code(), Some(2), "refuses: {}", stderr(&out));
    let said = stderr(&out);
    assert!(
        said.contains("--name"),
        "the refusal teaches the recovery: {said}"
    );
    assert!(
        !declaration.exists(),
        "the write is rolled back — no broken declaration left behind"
    );

    let out = sb.run(&ws, &["init", "--name", "project-root", "--json"]);
    assert!(out.status.success(), "init --name: {}", stderr(&out));
    assert_eq!(json(&out)["declared_name"], "project-root");
}

// ---------------------------------------------------------------------------
// Gate: the real `mrd daemon` serves resolve-adopt, then shuts down cleanly
// ---------------------------------------------------------------------------

#[test]
fn e2e_daemon_serves_resolve_adopt_and_shuts_down() {
    let sb = sandbox();
    let proj = sb.dir("tree");
    let sub = sb.dir("tree/crates/inner");
    let canon_proj = std::fs::canonicalize(&proj).unwrap();

    let mut daemon = sb
        .base(mrd_bin())
        .arg("daemon")
        .spawn()
        .expect("spawn mrd daemon");

    let socket = sb.cache_root.join("registry").join("daemon.sock");
    let client = registry::Client::new(socket.clone());
    assert!(
        wait_for_ping(&client, Duration::from_secs(5)),
        "daemon must answer a ping"
    );

    // The daemon-mediated registration of a bare tree (the future flow that
    // warms tier-4). This writes the registry entry + the drawer sentinel.
    let registered = client.register(&proj).expect("register round-trips");
    assert!(
        matches!(registered, registry::Response::Registered { .. }),
        "register succeeds: {registered:?}"
    );

    // `mrd resolve` from a subdir adopts the registered ancestor.
    let out = sb.run(&sub, &["resolve", "--json"]);

    // Shut the daemon down BEFORE asserting, so a failure never leaks it.
    graceful_kill(&mut daemon);
    let status = daemon.wait().expect("daemon exits");

    assert!(out.status.success(), "resolve: {}", stderr(&out));
    let v = json(&out);
    assert_eq!(
        v["source"], "daemon-adopted",
        "subdir adopts the registered tree"
    );
    assert_eq!(v["workspace"], canon_proj.to_string_lossy().as_ref());
    assert!(status.success(), "daemon exits 0 on SIGTERM");
    assert!(!socket.exists(), "graceful shutdown removes the socket");
}

fn wait_for_ping(client: &registry::Client, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if client.ping().unwrap_or(false) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    false
}

fn graceful_kill(child: &mut std::process::Child) {
    // SAFETY: a plain `kill(2)` on the child pid with SIGTERM — the daemon's
    // handler flips its shutdown flag and the foreground loop tears it down.
    let pid = child.id().cast_signed();
    unsafe {
        libc::kill(pid, libc::SIGTERM);
    }
}

// P1 (decision 0002 §3): the engine-client path — `mrd links` auto-spawns the resident daemon
// on first use and degrades to an in-process ephemeral engine.

impl Sandbox {
    /// An anchored workspace `tmp/<name>` (a `.git` entry, so the ladder answers git-root and
    /// never needs a daemon) seeded with `files`. Returns its canonical path.
    fn anchored_ws(&self, name: &str, files: &[(&str, &str)]) -> PathBuf {
        let ws = self.dir(name);
        std::fs::create_dir_all(ws.join(".git")).expect("git anchor");
        for (rel, content) in files {
            std::fs::write(ws.join(rel), content).expect("seed file");
        }
        std::fs::canonicalize(&ws).expect("canonical ws")
    }

    /// The resident daemon's pidfile path (written by the singleton winner).
    fn daemon_pidfile(&self) -> PathBuf {
        self.cache_root.join("registry").join("daemon.pid")
    }

    /// Read the resident daemon's pid, polling until the pidfile appears (the
    /// daemon writes it just after binding — a small window past first ping).
    fn wait_daemon_pid(&self, timeout: Duration) -> Option<i32> {
        self.wait_daemon_pid_since(None, timeout)
    }

    /// Like [`Self::wait_daemon_pid`], but when `exclude` is set, poll until the pidfile names a
    /// different pid — so a respawn is not confused with the killed daemon's stale pidfile
    /// (SIGKILL leaves it behind until the fresh daemon overwrites it).
    fn wait_daemon_pid_since(&self, exclude: Option<i32>, timeout: Duration) -> Option<i32> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Ok(text) = std::fs::read_to_string(self.daemon_pidfile())
                && let Ok(pid) = text.trim().parse::<i32>()
                && Some(pid) != exclude
            {
                return Some(pid);
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        None
    }
}

/// Send `signal` to `pid` (a detached daemon we do not own as a child).
fn signal(pid: i32, signal: libc::c_int) {
    // SAFETY: a plain `kill(2)` to a pid we read from the daemon's own pidfile.
    unsafe {
        libc::kill(pid, signal);
    }
}

/// Poll until `pid` is gone (`kill(pid, 0)` → `ESRCH`), so a killed daemon has
/// released its socket + singleton flock before the next client dials.
fn wait_dead(pid: i32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        // SAFETY: signal 0 probes existence without delivering a signal.
        if unsafe { libc::kill(pid, 0) } == -1 {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    false
}

// ---------------------------------------------------------------------------
// Gate: cold client auto-spawns the daemon and answers warm — and the degrade
// answer is byte-identical (one projection, two state sources, no drift).
// ---------------------------------------------------------------------------

#[test]
fn e2e_links_cold_auto_spawns_and_answers_warm() {
    let sb = sandbox();
    let ws = sb.anchored_ws("proj", &[("a.md", "# A\n\nsee [[b]]\n"), ("b.md", "# B\n")]);

    // Cold: no daemon running. `mrd links` must auto-spawn one and answer.
    let warm = sb.run(&ws, &["links", "--json"]);

    // Reap the auto-spawned resident daemon BEFORE asserting, so a failed assertion never leaks
    // it. It is detached (reparented to init), so we signal it by the pid it wrote to its own
    // pidfile.
    let pid = sb.wait_daemon_pid(Duration::from_secs(5));
    if let Some(pid) = pid {
        signal(pid, libc::SIGTERM);
        wait_dead(pid, Duration::from_secs(5));
    }
    // With the daemon gone, the same query degrades to the in-process engine.
    let cold = sb
        .base(mrd_bin())
        .env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-daemon")
        .args(["links", "--json"])
        .current_dir(&ws)
        .output()
        .expect("spawn mrd");

    assert!(pid.is_some(), "the auto-spawned daemon wrote a pidfile");
    assert!(warm.status.success(), "warm links: {}", stderr(&warm));
    let warm = json(&warm);
    assert_eq!(
        warm["source"], "daemon",
        "cold first use auto-spawns the daemon"
    );
    assert_eq!(
        warm["links"]["files"]["a.md"]["resolved"]["b.md"],
        serde_json::json!(1),
        "the warm engine resolves [[b]] → b.md: {warm}"
    );

    assert!(cold.status.success(), "degrade links: {}", stderr(&cold));
    let cold = json(&cold);
    assert_eq!(
        cold["source"], "ephemeral",
        "no daemon → in-process degrade"
    );

    // The whole point: the warm and degrade paths answer the SAME body.
    assert_eq!(
        warm["links"], cold["links"],
        "warm and degrade answers must not drift"
    );

    // And both speak the v3 vocabulary the CLI negotiated (`contract:v3`): the degrade body
    // carries the `fingerprint` staleness triple, never `root`. A positive+negative check, so a
    // SYMMETRIC regression of both paths back to `root` (which the equality above would not catch)
    // fails here.
    assert!(
        cold["links"]["as_of_fingerprint"].is_string()
            && cold["links"]["live_fingerprint"].is_string(),
        "the degrade answer speaks the fingerprint vocabulary: {cold}"
    );
    assert!(
        cold["links"].get("as_of_root").is_none() && cold["links"].get("live_root").is_none(),
        "the degrade answer never leaks the `root` vocabulary: {cold}"
    );
}

// ---------------------------------------------------------------------------
// Gate: spawn impossible → the in-process ephemeral answer is correct; the
// degrade NEVER fails a run.
// ---------------------------------------------------------------------------

#[test]
fn e2e_links_spawn_impossible_degrades_and_answers_correctly() {
    let sb = sandbox();
    let ws = sb.anchored_ws(
        "proj",
        &[
            ("a.md", "# A\n\nsee [[b]] and [[ghost]]\n"),
            ("b.md", "# B\n"),
        ],
    );

    // No daemon is running, and spawning one is impossible (the override names a binary that
    // does not exist), so `ensure_daemon` fails and the client degrades — the run still
    // succeeds.
    let out = sb
        .base(mrd_bin())
        .env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-daemon")
        .args(["links", "--json"])
        .current_dir(&ws)
        .output()
        .expect("spawn mrd");

    assert!(
        out.status.success(),
        "degrade must never fail the run: {}",
        stderr(&out)
    );
    let v = json(&out);
    assert_eq!(v["source"], "ephemeral", "spawn-impossible → in-process");
    assert_eq!(
        v["links"]["files"]["a.md"]["resolved"]["b.md"],
        serde_json::json!(1),
        "the in-process engine resolves [[b]] → b.md: {v}"
    );
    assert_eq!(
        v["links"]["files"]["a.md"]["unresolved"]["ghost"],
        serde_json::json!(1),
        "and reports the dangling [[ghost]]: {v}"
    );

    // No daemon was spawned, so nothing to reap.
    assert!(
        !sb.daemon_pidfile().exists(),
        "a degraded run spawns no daemon"
    );
}

// ---------------------------------------------------------------------------
// Gate: SIGKILL the daemon mid-session → the next client respawns a fresh
// daemon and answers correctly via fingerprint recovery.
// ---------------------------------------------------------------------------

#[test]
fn e2e_links_respawns_after_daemon_sigkill() {
    let sb = sandbox();
    let ws = sb.anchored_ws("proj", &[("a.md", "# A\n\nsee [[b]]\n"), ("b.md", "# B\n")]);

    // First use auto-spawns daemon d1 and answers warm.
    let first = sb.run(&ws, &["links", "--json"]);
    let pid1 = sb
        .wait_daemon_pid(Duration::from_secs(5))
        .expect("first use spawns a daemon");

    // SIGKILL d1 mid-session (no graceful shutdown): the socket goes stale and
    // the singleton flock releases on process death.
    signal(pid1, libc::SIGKILL);
    assert!(
        wait_dead(pid1, Duration::from_secs(5)),
        "the killed daemon exits"
    );

    // The next client sees the stale socket, respawns a FRESH daemon (d2), and answers correctly —
    // d2 rebuilds the engine from disk (fingerprint recovery), never from d1's lost in-memory
    // state.
    let second = sb.run(&ws, &["links", "--json"]);
    let pid2 = sb
        .wait_daemon_pid_since(Some(pid1), Duration::from_secs(5))
        .expect("the next client respawns a daemon");

    // Reap d2 before asserting.
    signal(pid2, libc::SIGTERM);
    wait_dead(pid2, Duration::from_secs(5));

    assert!(first.status.success(), "first links: {}", stderr(&first));
    assert_eq!(json(&first)["source"], "daemon");

    assert_ne!(pid2, pid1, "the killed daemon was respawned, not reused");
    assert!(
        second.status.success(),
        "respawned links: {}",
        stderr(&second)
    );
    let second = json(&second);
    assert_eq!(
        second["source"], "daemon",
        "the next client respawns and serves warm"
    );
    assert_eq!(
        second["links"]["files"]["a.md"]["resolved"]["b.md"],
        serde_json::json!(1),
        "the respawned daemon answers correctly via fingerprint recovery: {second}"
    );
}
