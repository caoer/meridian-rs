//! **`mrd daemon` raises its soft open-file limit to the hard one.**
//!
//! Whatever starts the daemon may hand it a low soft `RLIMIT_NOFILE`: a
//! spawner that restores the limit it was itself started under (1024 on a
//! Linux service), or a shell's default (256 under macOS launchd). The engine
//! then runs out of descriptors (`EMFILE`) under a fleet's connections.
//!
//! The gate runs the real `mrd daemon` binary under a soft limit of 256,
//! drives one hello over its socket, and reads the limit the running process
//! has: the line it says at start, and on Linux `/proc/<pid>/limits`.

use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt as _;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

mod common;

/// How long the daemon may take to bind.
const BUDGET: Duration = Duration::from_secs(30);

/// The soft limit the daemon is started under.
const LOW: libc::rlim_t = 256;

struct Sandbox {
    tmp: tempfile::TempDir,
    home: PathBuf,
    cache_home: PathBuf,
    ws: PathBuf,
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
        let ws = tmp.path().join("project");
        std::fs::create_dir_all(&home).expect("home");
        std::fs::create_dir_all(ws.join(".git")).expect("git anchor");
        std::fs::write(ws.join("a.md"), "# Alpha\n").expect("doc");
        let ws = std::fs::canonicalize(&ws).expect("canonical ws");
        Self {
            tmp,
            home,
            cache_home,
            ws,
        }
    }
}

/// The soft limit the daemon said it raised `LOW` to.
fn said_raised_to(voice: &str) -> Option<u64> {
    let prefix = format!("open-file limit: soft {LOW} -> ");
    let rest = voice.lines().find_map(|line| line.strip_prefix(&prefix))?;
    rest.split_whitespace().next()?.parse().ok()
}

#[test]
fn a_daemon_started_under_a_low_soft_limit_runs_at_its_hard_one() {
    let sb = Sandbox::new();
    let voice_path = sb.tmp.path().join("daemon.err");
    let voice = std::fs::File::create(&voice_path).expect("voice file");
    let mut command = common::mrd_command(&sb.home, &sb.cache_home);
    command
        .arg("daemon")
        .env_remove("MRD_TIMING")
        .env_remove("MERIDIAN_WORKSPACE")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(voice));
    // SAFETY: the hook runs between fork and exec and calls only
    // `getrlimit`/`setrlimit`, both async-signal-safe.
    unsafe {
        command.pre_exec(|| {
            let mut limit = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            if libc::getrlimit(libc::RLIMIT_NOFILE, &raw mut limit) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            limit.rlim_cur = LOW.min(limit.rlim_max);
            if libc::setrlimit(libc::RLIMIT_NOFILE, &raw const limit) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn().expect("mrd daemon spawns");

    let socket = common::child_socket_path(&sb.home, &sb.cache_home);
    let started = Instant::now();
    let stream = loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            panic!("the daemon died under a soft limit of {LOW} ({status})");
        }
        if let Ok(stream) = UnixStream::connect(&socket) {
            break stream;
        }
        assert!(started.elapsed() < BUDGET, "the daemon never bound");
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut line = json!({"op": "hello", "proto": 1, "contract": "v3",
                          "workspace": sb.ws.to_str().expect("utf-8 ws")})
    .to_string();
    line.push('\n');
    (&stream).write_all(line.as_bytes()).expect("hello sent");
    let mut response = String::new();
    BufReader::new(&stream)
        .read_line(&mut response)
        .expect("hello answered");
    let ack: Value = serde_json::from_str(&response).expect("a JSON frame");
    assert_eq!(ack["ok"], json!(true), "hello: {ack}");

    let said = std::fs::read_to_string(&voice_path).expect("voice");
    let raised = said_raised_to(&said)
        .unwrap_or_else(|| panic!("the daemon did not say it raised its limit: {said}"));
    assert!(raised > LOW, "raised to {raised}");

    // The running process's own limits: soft equals hard, and is what it said.
    #[cfg(target_os = "linux")]
    {
        let limits = std::fs::read_to_string(format!("/proc/{}/limits", child.id()))
            .expect("the daemon's limits");
        let row = limits
            .lines()
            .find_map(|row| row.strip_prefix("Max open files"))
            .expect("an open-files row");
        let mut columns = row.split_whitespace();
        let (soft, hard) = (columns.next().unwrap(), columns.next().unwrap());
        assert_eq!(soft, hard, "soft is the hard limit: {row}");
        assert_eq!(soft, raised.to_string(), "and the limit it said: {row}");
    }

    drop(stream);
    let pid = libc::pid_t::try_from(child.id()).expect("pid fits");
    // SAFETY: kill(2) on our own child, still unreaped.
    unsafe {
        libc::kill(pid, libc::SIGTERM);
    }
    let status = child.wait().expect("wait");
    assert_eq!(
        status.code(),
        Some(0),
        "a SIGTERM ends it cleanly: {status}"
    );
}
