//! The pointer read that the spawn ladder skips (`registry/src/server.rs`
//! § Socket placement; `socket_pointer_e2e.rs` for the settled case).
//!
//! `reachable_socket_path` fixes a client's socket ONCE, when the client is
//! built. That covers a holder already published by then. It does not cover
//! the cold start where nothing is published YET: two clients whose
//! environments derive different sockets for one cache root both find no
//! socket and no pointer, and both spawn. One child takes the flock, binds
//! its own derived path and publishes it; the other dies on the flock. The
//! loser's client then polls its OWN derived path for the whole
//! `SPAWN_READY_TIMEOUT` and degrades — with the holder live, published, and
//! one pointer read away for nearly all of those five seconds.
//!
//! Harness: the holder is an in-process daemon bound where no environment
//! derives it. Its pointer is removed so the client's construction cannot see
//! it, which is the cold start's shape. `MERIDIAN_DAEMON_BIN` is a child that
//! binds nothing and only writes the winner's pointer — the flock loser,
//! exactly. A warm answer means the ladder re-read the pointer; a degrade
//! means it spent the timeout on its own derived path.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::time::{Duration, Instant};

use registry::{Config, RunningServer};

mod common;

const DOC: &str = "# Alpha\n\none two three\n\n## Beta\n\nfour five\n";

struct Sandbox {
    tmp: tempfile::TempDir,
    cache_home: PathBuf,
    home: PathBuf,
    runtime: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
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

    fn cache_root(&self) -> PathBuf {
        self.cache_home.join("meridian")
    }

    /// Where the client's child would bind, from its own sandboxed variables.
    fn derived_socket(&self) -> PathBuf {
        if cfg!(target_os = "linux") {
            self.runtime
                .join("mrd")
                .join(format!("{}.sock", cache::sock_key(&self.cache_root())))
        } else {
            registry::socket_path_under_home(&self.home, &self.cache_root())
        }
    }

    /// Where the holder binds: a base no environment derives.
    fn holder_socket(&self) -> PathBuf {
        self.tmp.path().join("elsewhere").join("daemon.sock")
    }

    /// The flock holder, publishing this build's identity so the socket law
    /// serves rather than refusing for skew.
    #[allow(clippy::duration_suboptimal_units)]
    fn holder(&self) -> RunningServer {
        let forever = Duration::from_secs(365 * 24 * 60 * 60);
        let mut config = Config::for_cache_root(self.cache_root());
        config.socket_path = self.holder_socket();
        config.idle_threshold = forever;
        config.reap_interval = forever;
        config.prewarm_interval = forever;
        config.prewarm_quiet_max = forever;
        config.idle_exit = None;
        config.build_sha = Some(env!("MRD_BUILD_SHA").to_owned());
        config.drain_cold_builds = Duration::from_secs(30);
        RunningServer::start(config).expect("holder binds the foreign-base socket")
    }

    fn workspace(&self) -> PathBuf {
        let ws = self.tmp.path().join("project");
        std::fs::create_dir_all(ws.join(".git")).expect("git anchor");
        std::fs::write(ws.join("doc.md"), DOC).expect("doc");
        std::fs::canonicalize(&ws).expect("canonical ws")
    }

    /// A stand-in for the spawned daemon that models the flock LOSER: it
    /// binds nothing, and the only trace it leaves is the winner's pointer,
    /// written after this client already looked and found none.
    fn losing_daemon_bin(&self, winner: &Path) -> PathBuf {
        let bin = self.tmp.path().join("losing-daemon.sh");
        let dir = self.cache_root().join("registry");
        std::fs::write(
            &bin,
            format!(
                "#!/bin/sh\n\
                 printf '%s\\n' '{winner}' > '{dir}/socket.tmp'\n\
                 mv '{dir}/socket.tmp' '{dir}/socket'\n\
                 exit 1\n",
                winner = winner.display(),
                dir = dir.display(),
            ),
        )
        .expect("write fake daemon");
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        bin
    }

    fn run(&self, cwd: &Path, args: &[&str], daemon_bin: &Path) -> Output {
        let mut cmd = common::mrd_command(&self.home, &self.cache_home);
        cmd.env("XDG_RUNTIME_DIR", &self.runtime)
            .env_remove("MERIDIAN_WORKSPACE")
            .env("MERIDIAN_DAEMON_BIN", daemon_bin)
            .args(args)
            .current_dir(cwd);
        cmd.output().expect("spawn mrd")
    }
}

/// The cold-start race: nothing is published when the client is built, so
/// `reachable_socket_path` hands back the derived path and the ladder spawns.
/// The winner publishes during the wait. The ladder must notice.
#[test]
fn a_pointer_published_during_the_spawn_wait_is_read_before_the_timeout() {
    let sb = Sandbox::new();
    let ws = sb.workspace();
    let holder = sb.holder();

    // The holder published at start; take it away, so the client's
    // construction sees the cold start's shape and the ladder is reached.
    let pointer = registry::socket_pointer_path(&sb.cache_root());
    std::fs::remove_file(&pointer).expect("the holder published at start");
    assert!(
        !sb.derived_socket().exists(),
        "the fixture's premise: this client derives a socket nobody bound"
    );

    let bin = sb.losing_daemon_bin(&sb.holder_socket());
    let started = Instant::now();
    let out = sb.run(&ws, &["read", "doc.md"], &bin);
    let elapsed = started.elapsed();
    let loser_published = pointer.exists();
    holder.shutdown();

    assert!(
        loser_published,
        "the fixture's own premise: the child wrote the winner's pointer"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        out.status.code(),
        Some(0),
        "the holder serves\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        !stderr.contains("ephemeral"),
        "warm, never the degrade — a degrade here means the ladder polled only its own derived \
         path while the holder was live and published: {stderr}"
    );
    assert!(
        elapsed < Duration::from_secs(4),
        "the ladder re-read the pointer rather than spending the spawn timeout: {elapsed:?}"
    );
}
