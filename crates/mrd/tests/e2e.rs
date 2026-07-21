//! End-to-end gates for the `mrd` CLI, driving the REAL binary
//! (`CARGO_BIN_EXE_mrd`) over its process boundary with an overridden cache root
//! (`XDG_CACHE_HOME`) and `HOME`. This is the phase integration evidence: the
//! landed `workspace` / `cache` / `registry` crates wired into the settled verb
//! surface, exercised as an operator would.

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
    assert!(ws.join(".meridian.toml").exists(), "marker created");

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

// ---------------------------------------------------------------------------
// Gate: tier-4, no daemon → ephemeral, NOTHING written under the cache root
// ---------------------------------------------------------------------------

#[test]
fn e2e_tier4_no_daemon_is_ephemeral_and_writes_nothing() {
    let sb = sandbox();
    let bare = sb.dir("bare");
    assert!(
        !sb.cache_root.exists(),
        "precondition: cache root not yet created"
    );

    let out = sb.run(&bare, &["resolve", "--json"]);
    assert!(out.status.success(), "resolve: {}", stderr(&out));
    let v = json(&out);
    assert_eq!(v["source"], "ephemeral");
    assert_eq!(v["ephemeral"], true);
    assert_eq!(v["state"], "cold");

    assert!(
        !sb.cache_root.exists(),
        "a tier-4 ephemeral resolution must write nothing under the cache root"
    );
}

// ---------------------------------------------------------------------------
// Gate: downgrade — a future-schema sentinel is a cold start, exit 0
// ---------------------------------------------------------------------------

#[test]
fn e2e_downgrade_future_schema_sentinel_is_cold_exit0() {
    let sb = sandbox();
    let ws = sb.dir("marked");
    std::fs::write(ws.join(".meridian.toml"), "version = 1\n").unwrap();
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
        v["source"], "marker",
        "the marker still resolves the workspace"
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
    // Refused BEFORE any write: no marker, no drawer.
    assert!(
        !sb.home.join(".meridian.toml").exists(),
        "no marker written"
    );
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
