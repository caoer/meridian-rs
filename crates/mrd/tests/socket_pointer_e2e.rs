//! The published socket pointer, end to end (`registry/src/server.rs`
//! § Socket placement): two `mrd` clients whose environments derive
//! DIFFERENT sockets for one cache root are both served by the one resident
//! daemon — the second finds the holder through the pointer instead of
//! spawning a flock loser and degrading.
//!
//! The measured shape (workstation-nyc-2): a systemd-started process carried
//! `XDG_RUNTIME_DIR=/run/user/1000`, a non-interactive ssh shell had it
//! unset, and the two derived different sockets for one cache root. The
//! fixture reproduces the disagreement on every platform: on Linux the two
//! runs carry different `XDG_RUNTIME_DIR`s (the lane that variable selects),
//! elsewhere different `HOME`s (the lane the law falls to).

use std::path::{Path, PathBuf};
use std::process::Output;
use std::time::{Duration, Instant};

mod common;

const DOC: &str = "# Alpha\n\none two three\n\n## Beta\n\nfour five\n";

/// One client's environment: the two variables the socket law reads.
struct Env {
    home: PathBuf,
    runtime_dir: PathBuf,
}

struct Sandbox {
    tmp: tempfile::TempDir,
    cache_home: PathBuf,
    cache_root: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cache_home = tmp.path().join("xdg-cache");
        let cache_root = cache_home.join("meridian");
        Sandbox {
            tmp,
            cache_home,
            cache_root,
        }
    }

    fn env(&self, tag: &str) -> Env {
        let home = self.tmp.path().join(format!("home-{tag}"));
        let runtime_dir = self.tmp.path().join(format!("run-{tag}"));
        std::fs::create_dir_all(&home).expect("home");
        std::fs::create_dir_all(&runtime_dir).expect("runtime dir");
        Env { home, runtime_dir }
    }

    /// An anchored workspace holding the fixture doc.
    fn workspace(&self) -> PathBuf {
        let ws = self.tmp.path().join("project");
        std::fs::create_dir_all(ws.join(".git")).expect("git anchor");
        std::fs::write(ws.join("doc.md"), DOC).expect("doc");
        std::fs::canonicalize(&ws).expect("canonical ws")
    }

    /// Run `mrd` as `env` sees the world (auto-spawn allowed).
    fn run(&self, env: &Env, cwd: &Path, args: &[&str]) -> Output {
        let mut cmd = common::mrd_command(&env.home, &self.cache_home);
        cmd.env_remove("MERIDIAN_WORKSPACE");
        cmd.env("XDG_RUNTIME_DIR", &env.runtime_dir);
        cmd.args(args).current_dir(cwd).output().expect("spawn mrd")
    }

    /// The socket the law derives for a child running as `env` — mirrored
    /// lane by lane against the child's OWN variables, not this process's
    /// (`common::child_socket_path` reads the test process's runtime dir).
    fn derived_socket(&self, env: &Env) -> PathBuf {
        if cfg!(target_os = "linux") {
            env.runtime_dir
                .join("mrd")
                .join(format!("{}.sock", cache::sock_key(&self.cache_root)))
        } else {
            registry::socket_path_under_home(&env.home, &self.cache_root)
        }
    }

    /// The pid a daemon claimed beside the socket it published, right now.
    fn published_pid(&self) -> Option<i32> {
        let socket = registry::published_socket_path(&self.cache_root)?;
        let text = std::fs::read_to_string(socket.with_extension("pid")).ok()?;
        text.trim().parse::<i32>().ok()
    }

    /// Wait for a daemon to publish its socket and claim the pidfile beside
    /// it; the pid, or `None` at the timeout.
    fn wait_published_pid(&self, timeout: Duration) -> Option<i32> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(pid) = self.published_pid() {
                return Some(pid);
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}

/// The fixture owns its daemon (common § Fixture rule): reap through the
/// pointer, on the panic path too.
impl Drop for Sandbox {
    fn drop(&mut self) {
        if let Some(pid) = self.published_pid() {
            signal(pid, libc::SIGTERM);
            if !wait_dead(pid, Duration::from_secs(5)) {
                signal(pid, libc::SIGKILL);
                let _ = wait_dead(pid, Duration::from_secs(2));
            }
        }
    }
}

/// Send `signal` to `pid` (a detached daemon we do not own as a child).
fn signal(pid: i32, signal: libc::c_int) {
    // SAFETY: a plain `kill(2)` on a pid the daemon wrote to its own pidfile.
    unsafe {
        libc::kill(pid, signal);
    }
}

/// Poll until `pid` is gone.
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

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// The whole claim: the first client spawns the resident and is served warm;
/// the second — deriving a different socket for the same cache root — is
/// served warm by THAT resident, spawns nothing, and the pointer still names
/// the first socket afterwards.
#[test]
fn a_client_with_a_different_derivation_is_served_by_the_published_holder() {
    let sb = Sandbox::new();
    let ws = sb.workspace();
    let a = sb.env("a");
    let b = sb.env("b");
    assert_ne!(
        sb.derived_socket(&a),
        sb.derived_socket(&b),
        "the two environments must derive different sockets, or the gate measures nothing"
    );

    // A: the cold first use auto-spawns the resident at A's derived socket.
    let first = sb.run(&a, &ws, &["read", "doc.md", "--json"]);
    let pid = sb.wait_published_pid(Duration::from_secs(5));
    assert_eq!(
        first.status.code(),
        Some(0),
        "first read exits 0: {}",
        stderr(&first)
    );
    assert!(
        stdout(&first).contains("\"source\": \"daemon\""),
        "the first client is served warm by the daemon it spawned: {}",
        stdout(&first)
    );
    assert!(
        pid.is_some(),
        "the daemon published its socket and claimed the pidfile beside it"
    );
    assert_eq!(
        registry::published_socket_path(&sb.cache_root).as_deref(),
        Some(sb.derived_socket(&a).as_path()),
        "the published socket is what A's environment derives — the lane under test is in play"
    );

    // B: derives another socket for the same cache root. Nothing listens
    // there; the pointer names the holder; the holder serves.
    let started = Instant::now();
    let second = sb.run(&b, &ws, &["read", "doc.md", "--json"]);
    let elapsed = started.elapsed();

    assert_eq!(
        second.status.code(),
        Some(0),
        "second read exits 0: {}",
        stderr(&second)
    );
    assert!(
        stdout(&second).contains("\"source\": \"daemon\""),
        "the second client is served warm through the pointer, not degraded after a lost \
         flock (took {elapsed:?}): stdout={} stderr={}",
        stdout(&second),
        stderr(&second)
    );
    assert_eq!(
        registry::published_socket_path(&sb.cache_root).as_deref(),
        Some(sb.derived_socket(&a).as_path()),
        "the pointer still names the first resident — no second daemon took the flock"
    );
    assert!(
        !sb.derived_socket(&b).with_extension("pid").exists(),
        "no daemon claimed a pidfile beside B's derived socket"
    );
}
