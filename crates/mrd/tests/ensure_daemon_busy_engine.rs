//! **A busy daemon is not an absent one: `ensure_daemon` never spawns beside
//! it** (card `mrd-ensure-daemon-false-dead-spawns-second-daemons`).
//!
//! `engine::ensure_daemon` used to spawn whenever its liveness ping did not
//! pong. A ping a live daemon holds unanswered past `wedge::PROBE_TIMEOUT` —
//! swapping, saturated — did not pong, so the client launched a second
//! `mrd daemon`, which lost the singleton flock and exited, and the client
//! then waited out the 5 s spawn deadline and degraded. Measured on
//! devstation-nyc-1, 2026-10-24: 369 such launches in under three hours
//! beside one swapping engine.
//!
//! The launches are counted directly: `MERIDIAN_DAEMON_BIN` points at a stub
//! that records each invocation and exits, so a spawn is a line in a file —
//! no real daemon is ever started, and the count is exact.

use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

mod common;

const DOC: &str = "# Alpha\n\nsee [[beta]]\n";

/// How long the busy fake holds each connection before closing it. Past the
/// probe timeout, so every ping it is sent goes unanswered — the trigger.
const HOLD: Duration = Duration::from_secs(3);

/// A daemon that is alive and too loaded to answer: it accepts every
/// connection, answers nothing, and closes each one [`HOLD`] after accepting
/// it. That is the nyc-1 engine as its clients saw it.
struct Busy {
    done: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Busy {
    fn on(listener: UnixListener) -> Self {
        let done = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&done);
        listener.set_nonblocking(true).expect("poll, not block");
        let thread = thread::spawn(move || {
            let mut held: Vec<(UnixStream, Instant)> = Vec::new();
            while !flag.load(Ordering::SeqCst) {
                if let Ok((stream, _)) = listener.accept() {
                    held.push((stream, Instant::now()));
                    continue;
                }
                held.retain(|(_, since)| since.elapsed() < HOLD);
                thread::sleep(Duration::from_millis(20));
            }
        });
        Self {
            done,
            thread: Some(thread),
        }
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        self.done.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

struct Sandbox {
    tmp: tempfile::TempDir,
    home: PathBuf,
    cache_home: PathBuf,
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        common::reap_daemon(&self.home, &self.cache_home);
    }
}

impl Sandbox {
    fn new() -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let home = tmp.path().join("home");
        let cache_home = tmp.path().join("xdg-cache");
        std::fs::create_dir_all(&home).expect("home");
        Self {
            tmp,
            home,
            cache_home,
        }
    }

    fn workspace(&self) -> PathBuf {
        let ws = self.tmp.path().join("project");
        std::fs::create_dir_all(ws.join(".git")).expect("git anchor");
        std::fs::write(ws.join("a.md"), DOC).expect("doc");
        std::fs::canonicalize(&ws).expect("canonical ws")
    }

    /// The socket this sandbox's `mrd` child derives, its parent created.
    fn socket(&self) -> PathBuf {
        let socket = common::child_socket_path(&self.home, &self.cache_home);
        if let Some(parent) = socket.parent() {
            std::fs::create_dir_all(parent).expect("socket parent");
        }
        socket
    }

    fn launches_file(&self) -> PathBuf {
        self.tmp.path().join("launches")
    }

    /// A daemon binary that records its launch and exits without serving.
    fn stub_daemon(&self) -> PathBuf {
        let stub = self.tmp.path().join("stub-daemon");
        std::fs::write(
            &stub,
            format!(
                "#!/bin/sh\necho \"$*\" >> '{}'\nexit 1\n",
                self.launches_file().display()
            ),
        )
        .expect("stub");
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        stub
    }

    fn launches(&self) -> usize {
        std::fs::read_to_string(self.launches_file()).map_or(0, |s| s.lines().count())
    }

    /// `mrd links --json` in `ws`, spawning through the stub.
    fn links(&self, ws: &Path) -> Output {
        let mut cmd = common::mrd_command(&self.home, &self.cache_home);
        cmd.args(["links", "--json"])
            .env_remove("MRD_TIMING")
            .env_remove("MERIDIAN_WORKSPACE")
            .env("MERIDIAN_DAEMON_BIN", self.stub_daemon())
            .current_dir(ws);
        cmd.output().expect("mrd links runs")
    }
}

fn stderr_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// **The slow-live case.** Every probe the busy daemon is sent goes
/// unanswered, and still no second daemon is launched: holding a connection is
/// something only a live daemon does. The run still completes.
#[test]
fn a_busy_daemon_that_misses_its_ping_is_not_spawned_beside() {
    let sb = Sandbox::new();
    let ws = sb.workspace();
    let _busy = Busy::on(UnixListener::bind(sb.socket()).expect("bind the busy fake"));

    let out = sb.links(&ws);
    let said = stderr_of(&out);

    assert_eq!(
        sb.launches(),
        0,
        "a daemon that holds the probe is alive — spawning beside it can only lose the \
         singleton flock: {said}"
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "the run still completes: {said}"
    );
    assert!(
        !said.contains("could not be launched") && !said.contains("never bound its socket"),
        "a busy daemon must not be reported as a spawn failure: {said}"
    );
}

/// **No daemon at all still spawns one** — no socket file, so the connect
/// fails at once and the launch happens.
#[test]
fn no_daemon_still_spawns_one() {
    let sb = Sandbox::new();
    let ws = sb.workspace();
    let _ = sb.socket();

    let out = sb.links(&ws);

    assert!(
        sb.launches() >= 1,
        "with nothing serving the socket the client must launch a daemon: {}",
        stderr_of(&out)
    );
}

/// **A dead daemon still spawns one** — its socket file is left behind with
/// nothing listening (an exit does not unlink it), so the connect is refused
/// and the launch happens.
#[test]
fn a_dead_daemons_stale_socket_still_spawns_one() {
    let sb = Sandbox::new();
    let ws = sb.workspace();
    let socket = sb.socket();
    drop(UnixListener::bind(&socket).expect("bind, then die"));
    assert!(
        socket.exists(),
        "the dead daemon's socket file stays behind"
    );

    let out = sb.links(&ws);

    assert!(
        sb.launches() >= 1,
        "a refused connect is an absent daemon, and the client must launch one: {}",
        stderr_of(&out)
    );
}
