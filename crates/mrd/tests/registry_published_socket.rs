//! The published socket path, end to end (`docs/wire-contract.md` § The
//! published socket path; `registry::server` § The published socket path).
//!
//! Measured defect (2026-09-02): the daemon's singleton lock is keyed on the
//! cache root while its socket base is env-derived (`$XDG_RUNTIME_DIR/mrd/`
//! when that variable is set, else `$HOME/.cache/mrd-run/`). A client without
//! `XDG_RUNTIME_DIR` spawned the daemon under the HOME base; every client WITH
//! it then dialled the absent runtime-base socket, auto-spawned a successor,
//! and got "another meridian registry daemon is already running" from a child
//! nobody could hear — a 15 s wait and a refusal, while the lock holder served
//! nobody. One lock, two socket dirs.
//!
//! The law under test: the daemon publishes the socket it bound in the lock's
//! own directory; a client whose derived socket is absent dials the published
//! one; the auto-spawn ladder pings the published socket BEFORE spawning, so
//! it never launches a daemon the lock holder would refuse; and when the
//! published socket was tried and did not answer, the degrade names it.
//!
//! Harness: an in-process `RunningServer` bound where the sandboxed child will
//! NOT derive its socket (`<tmp>/elsewhere/daemon.sock`), publishing this
//! build's identity so the socket law serves. The child runs with
//! `MERIDIAN_DAEMON_BIN=/nonexistent`, so any spawn attempt fails loud and
//! degrades to the ephemeral engine — a WARM answer proves the ladder never
//! spawned. The child's `XDG_RUNTIME_DIR` is a sandbox directory, so the
//! derived socket this test leaves stale is its own, never the user's.

use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::time::Duration;

use registry::{Config, RunningServer};

mod common;

/// The identity this test's own compilation carries — the same crate stamp the
/// `mrd` binary under test bakes (`engine_skew_refuse.rs`).
const OWN_BUILD: &str = env!("MRD_BUILD_SHA");

const DOC: &str = "# Alpha\n\none two three\n\n## Beta\n\nfour five\n";

/// A `put` edit on the fixture doc — the write door, which has no degrade leg
/// and so proves the ladder reached a daemon.
const BETA_EDIT: &str = r#"[{"target":{"hpath":[{"h":"Alpha"},{"h":"Beta"}]},"edit":{"match":{"old":"four five","new":"four five six"}}}]"#;

struct Sandbox {
    tmp: tempfile::TempDir,
    cache_home: PathBuf,
    home: PathBuf,
    runtime: PathBuf,
}

fn sandbox() -> Sandbox {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cache_home = tmp.path().join("xdg-cache");
    let home = tmp.path().join("home");
    let runtime = tmp.path().join("rt");
    std::fs::create_dir_all(&home).expect("home");
    std::fs::create_dir_all(&runtime).expect("runtime dir");
    Sandbox {
        tmp,
        cache_home,
        home,
        runtime,
    }
}

impl Sandbox {
    /// The cache root the child derives from this sandbox's env — where the
    /// lock and the publication live.
    fn cache_root(&self) -> PathBuf {
        self.cache_home.join("meridian")
    }

    /// The socket the CHILD derives from its sandboxed environment: the Linux
    /// `XDG_RUNTIME_DIR` lane under the sandbox's runtime dir, else the HOME
    /// lane — `registry::socket_path_for_cache_root` lane by lane, computed
    /// against the child's env rather than this process's.
    fn derived_socket(&self) -> PathBuf {
        #[cfg(target_os = "linux")]
        {
            self.runtime
                .join("mrd")
                .join(format!("{}.sock", cache::sock_key(&self.cache_root())))
        }
        #[cfg(not(target_os = "linux"))]
        {
            registry::socket_path_under_home(&self.home, &self.cache_root())
        }
    }

    /// Where the lock holder binds: a base NO environment derives.
    fn published_socket(&self) -> PathBuf {
        self.tmp.path().join("elsewhere").join("daemon.sock")
    }

    /// The lock holder: an in-process daemon on the sandbox's cache root,
    /// bound at [`Self::published_socket`], publishing this build's identity.
    #[allow(clippy::duration_suboptimal_units)]
    fn daemon(&self) -> RunningServer {
        let forever = Duration::from_secs(365 * 24 * 60 * 60);
        let mut config = Config::for_cache_root(self.cache_root());
        config.socket_path = self.published_socket();
        config.idle_threshold = forever;
        config.reap_interval = forever;
        config.prewarm_interval = forever;
        config.prewarm_quiet_max = forever;
        config.idle_exit = None;
        config.build_sha = Some(OWN_BUILD.to_owned());
        config.drain_cold_builds = Duration::from_secs(30);
        RunningServer::start(config).expect("in-process daemon binds the foreign-base socket")
    }

    /// An anchored workspace holding the fixture doc.
    fn workspace(&self) -> PathBuf {
        let ws = self.tmp.path().join("project");
        std::fs::create_dir_all(ws.join(".git")).expect("git anchor");
        std::fs::write(ws.join("doc.md"), DOC).expect("doc");
        std::fs::canonicalize(&ws).expect("canonical ws")
    }

    /// Run `mrd` in the sandbox, spawn-impossible: a spawn attempt fails loud
    /// and degrades, so a warm answer proves no spawn was needed.
    fn run(&self, cwd: &Path, args: &[&str], stdin: Option<&str>) -> Output {
        let mut cmd = common::mrd_command(&self.home, &self.cache_home);
        cmd.env("XDG_RUNTIME_DIR", &self.runtime)
            .env_remove("MERIDIAN_WORKSPACE")
            .env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-daemon")
            .args(args)
            .current_dir(cwd);
        match stdin {
            Some(bytes) => {
                let mut child = cmd
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .expect("spawn mrd");
                common::feed_stdin(&mut child, bytes.as_bytes());
                child.wait_with_output().expect("wait")
            }
            None => cmd.output().expect("spawn mrd"),
        }
    }

    /// Publish `socket` by hand, exactly where the daemon writes it — for the
    /// case where the daemon that published is gone.
    fn publish(&self, socket: &Path) {
        let dir = self.cache_root().join("registry");
        std::fs::create_dir_all(&dir).expect("registry dir");
        std::fs::write(
            dir.join("daemon.sock-path"),
            format!("{}\n", socket.display()),
        )
        .expect("publish");
    }
}

/// Leave a socket FILE at `path` with nothing listening behind it — a
/// `SIGKILL`ed daemon's residue, which a dial answers `ECONNREFUSED`.
fn leave_stale_socket(path: &Path) {
    std::fs::create_dir_all(path.parent().expect("socket dir")).expect("socket dir");
    drop(UnixListener::bind(path).expect("bind"));
    assert!(path.exists(), "the file outlives the listener");
}

fn stderr_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn stdout_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A warm answer, asserted the way the skew gates assert one: served, and no
/// degrade voice — which, spawn-impossible, is the proof no spawn happened.
fn assert_served_warm(out: &Output, lane: &str) {
    let err = stderr_of(out);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{lane}: the lock holder serves\nstdout: {}\nstderr: {err}",
        stdout_of(out)
    );
    assert!(
        stdout_of(out).contains("Alpha"),
        "{lane}: the answer is the corpus content: {}",
        stdout_of(out)
    );
    assert!(
        !err.contains("ephemeral"),
        "{lane}: warm, never the degrade — a degrade here means the ladder spawned (and failed) \
         instead of dialling the published socket: {err}"
    );
}

/// The incident itself: the child's derived socket is ABSENT, the lock holder
/// is bound under another base, and the read is served warm — the client
/// followed the publication and never spawned.
#[test]
fn an_absent_derived_socket_is_served_by_the_published_daemon_without_a_spawn() {
    let sb = sandbox();
    let ws = sb.workspace();
    let _daemon = sb.daemon();
    assert!(
        !sb.derived_socket().exists(),
        "the fixture's premise: the child derives a socket nobody bound"
    );

    assert_served_warm(&sb.run(&ws, &["read", "doc.md"], None), "read");
}

/// The ladder's own gate: the derived socket EXISTS and does not answer (a
/// `SIGKILL`ed predecessor's file), so `Client::from_default` keeps it and
/// the first dial fails — the shape that used to go straight to a spawn.
/// `ensure_daemon` pings the published socket BEFORE spawning and retargets
/// there; every lane that reaches the ladder is served warm.
#[test]
fn a_stale_derived_socket_sends_the_ladder_to_the_published_daemon_instead_of_spawning() {
    let sb = sandbox();
    let ws = sb.workspace();
    let _daemon = sb.daemon();
    leave_stale_socket(&sb.derived_socket());

    // `read`: connect-first, then the ladder (`read_cmd::connect_or_spawn`).
    assert_served_warm(&sb.run(&ws, &["read", "doc.md"], None), "read");
    // `links`: the ladder first (`engine::try_daemon_links`).
    let out = sb.run(&ws, &["links"], None);
    assert_eq!(out.status.code(), Some(0), "links: {}", stderr_of(&out));
    assert!(
        stdout_of(&out).contains("source: daemon"),
        "links names the warm source: {}",
        stdout_of(&out)
    );
    // `put`: the write door has no degrade leg — a landed edit IS the daemon.
    let out = sb.run(&ws, &["put", "doc.md", "--force"], Some(BETA_EDIT));
    assert_eq!(
        out.status.code(),
        Some(0),
        "put reaches the lock holder through the publication\nstdout: {}\nstderr: {}",
        stdout_of(&out),
        stderr_of(&out)
    );
    assert!(
        std::fs::read_to_string(ws.join("doc.md"))
            .expect("read back")
            .contains("four five six"),
        "the write landed through the published socket"
    );
}

/// A publication that names a socket nobody answers on is tried and then
/// named: the degrade keeps its teaching and gains one sentence pointing at
/// the published path, so an operator reading "no daemon" can see which
/// daemon the registry believes holds the lock.
#[test]
fn a_published_socket_that_does_not_answer_is_named_beside_the_teaching() {
    let sb = sandbox();
    let ws = sb.workspace();
    let dead = sb.published_socket();
    leave_stale_socket(&dead);
    sb.publish(&dead);

    // The read lane degrades, and its voice carries the sentence.
    let out = sb.run(&ws, &["read", "doc.md"], None);
    let err = stderr_of(&out);
    assert_eq!(out.status.code(), Some(0), "read degrades: {err}");
    assert!(
        err.contains("source: ephemeral"),
        "spawn-impossible with a dead publication is the degrade: {err}"
    );
    assert!(
        err.contains("published") && err.contains(dead.to_str().unwrap()),
        "the degrade names the published socket it tried: {err}"
    );

    // The write lane refuses, keeps its teaching, and carries the same sentence.
    let out = sb.run(&ws, &["put", "doc.md", "--force"], Some(BETA_EDIT));
    let err = stderr_of(&out);
    assert_eq!(
        out.status.code(),
        Some(2),
        "put refuses without a daemon: {err}"
    );
    assert!(
        err.contains("daemon must come up"),
        "the existing teaching is kept: {err}"
    );
    assert!(
        err.contains("published") && err.contains(dead.to_str().unwrap()),
        "and extended with the published socket that was tried: {err}"
    );
    assert!(
        std::fs::read_to_string(ws.join("doc.md"))
            .expect("read back")
            .contains("four five\n"),
        "nothing was written"
    );
}
