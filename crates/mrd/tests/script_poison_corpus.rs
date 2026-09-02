//! The script entry through the REAL door with a poison member present —
//! the poison-member incident, held to node-rev-merkle-spec §3
//! line 52: non-UTF-8 files "still get leaf hashes … and participate in the
//! root; they simply serve no spans/nodes (wire `invalid_utf8` law)".
//!
//! The incident: one non-UTF-8 file in the workspace root made the daemon
//! refuse the ENTIRE workspace at `hello`, so every `mrd script` died "cannot
//! dial the daemon" fleet-wide until the file was removed. These gates drive
//! `SocketDoor::connect` — the production dial, hello over a real socket —
//! and the script face against a live daemon whose corpus carries a poison
//! member, and hold the ruled per-file grain.
//!
//! The companion P2 gates hold the OTHER half of the incident's cost: when
//! the daemon does refuse the handshake, it names the file and the cause in
//! its error frame, and `SocketDoor::connect` must surface that frame instead
//! of collapsing it into a static string the operator cannot act on.

use std::fs;
use std::io::{BufRead, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use mrd::script::ScriptOutcome;
use mrd::script::cmd::attempt;
use mrd::script::wire_host::{GREET_CAP, SocketDoor};
use registry::{Config, RunningServer};
use tempfile::TempDir;

/// A real daemon config: the reaper never evicts a warm engine mid-test, and
/// the idle-exit clock is the test's (the `script_golden_live` precedent).
#[allow(clippy::duration_suboptimal_units)]
fn config(tmp: &TempDir) -> Config {
    let forever = Duration::from_secs(365 * 24 * 60 * 60);
    let mut config = Config::for_cache_root(tmp.path().join("cache"));
    config.idle_threshold = forever;
    config.reap_interval = forever;
    config.prewarm_interval = forever;
    config.prewarm_quiet_max = forever;
    config.idle_exit = None;
    // The fixture daemon publishes THIS build's identity: the 0025 socket law
    // refuses an identity-less local hello, and these tests measure poison
    // handling, not the law.
    config.build_sha = Some(env!("MRD_BUILD_SHA").to_owned());
    config.drain_cold_builds = Duration::from_secs(30);
    config
}

/// A workspace `tmp/ws` seeded with byte-level `files` (poison members are
/// exactly the bytes the incident planted — no `&str` door would carry them).
fn write_ws(tmp: &TempDir, files: &[(&str, &[u8])]) -> PathBuf {
    let ws = tmp.path().join("ws");
    for (rel, bytes) in files {
        let path = ws.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, bytes).unwrap();
    }
    ws
}

const HEALTHY: &[u8] = b"---\nowner:\nstatus: todo\n---\n\n# Goals\n\nship the fix\n";
const POISON: &[u8] = b"# Poison\n\n\xff\xfe raw bytes\n";

/// **The e2e gate.** With a poison member present, the production dial binds
/// (daemon hello over the socket) and the script face serves a healthy card —
/// the fleet-killing "cannot dial the daemon" shape is gone.
#[test]
fn the_script_entry_serves_with_a_poison_member_present() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(
        &tmp,
        &[("tasks/0011-card.md", HEALTHY), ("notes/poison.md", POISON)],
    );
    let server = RunningServer::start(config(&tmp)).unwrap();

    let mut door = SocketDoor::connect(server.socket_path(), &ws, GREET_CAP).unwrap_or_else(|e| {
        panic!("hello over the real socket binds a poisoned-but-healthy workspace: {e}")
    });

    let argv = vec!["--actor".to_owned(), "e50dfd13".to_owned()];
    let trace = attempt(&argv, r#"card = read("tasks/0011-card.md")"#, &mut door)
        .expect("a read-only attempt runs");
    assert_eq!(
        trace.outcome,
        ScriptOutcome::NoEffect,
        "the healthy card serves through the script face: {:?}",
        trace.fault
    );

    server.shutdown();
}

/// The poison member itself serves no spans/nodes: a script read of it faults
/// with the per-file `invalid_utf8` refusal, naming the file — never a
/// workspace-wide condition.
#[test]
fn a_script_read_of_the_poison_member_faults_naming_the_file() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(
        &tmp,
        &[("tasks/0011-card.md", HEALTHY), ("notes/poison.md", POISON)],
    );
    let server = RunningServer::start(config(&tmp)).unwrap();

    let mut door = SocketDoor::connect(server.socket_path(), &ws, GREET_CAP)
        .expect("the workspace binds — degradation is per-file");
    let argv = vec!["--actor".to_owned(), "e50dfd13".to_owned()];
    let trace = attempt(&argv, r#"bad = read("notes/poison.md")"#, &mut door)
        .expect("the attempt runs — the refusal is an outcome, not a transport failure");
    assert_eq!(trace.outcome, ScriptOutcome::Fault);
    let fault = trace.fault.expect("a fault outcome carries its fault");
    assert!(
        fault.reason.contains("invalid_utf8") && fault.reason.contains("notes/poison.md"),
        "the fault carries the per-file refusal, naming code and member: {}",
        fault.reason
    );

    server.shutdown();
}

/// **P2, live half.** A handshake the daemon still refuses (ambiguous domain:
/// two config files — a corpus-scoped condition with no per-file grain) must
/// reach the operator with the daemon's own cause, not the static "refused
/// the v3 handshake" that hid the poison member for a whole dogfood session.
#[test]
fn a_live_handshake_refusal_surfaces_the_daemons_cause() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(
        &tmp,
        &[
            ("healthy.md", b"# Healthy\n".as_slice()),
            (
                "meridian/domain.md",
                b"---\nignore:\n  - \"a/**\"\n---\n".as_slice(),
            ),
            ("mdfs_config.yaml", b"ignore:\n  - \"b/**\"\n".as_slice()),
        ],
    );
    let server = RunningServer::start(config(&tmp)).unwrap();

    let Err(err) = SocketDoor::connect(server.socket_path(), &ws, GREET_CAP) else {
        panic!("an ambiguous domain refuses the handshake");
    };
    let rendered = err.to_string();
    assert!(
        rendered.contains("mdfs_config.yaml") && rendered.contains("meridian/domain.md"),
        "the dial error carries the daemon's cause — the operator must see WHAT \
         refused, not only THAT it refused: {rendered}"
    );

    server.shutdown();
}

/// **P2, deterministic half.** The daemon's error frame reaches the connect
/// error verbatim — code, path, and message — pinned against a fake listener
/// so the assertion is byte-level and needs no daemon behavior at all.
#[test]
fn the_handshake_refusal_carries_the_error_body_verbatim() {
    let dir = TempDir::new().unwrap();
    let sock = dir.path().join("daemon.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let served = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let mut w = stream;
        w.write_all(
            b"{\"ok\":false,\"error\":{\"code\":\"invalid_utf8\",\"recovery\":\"env\",\
              \"path\":\"notes/poison.md\",\
              \"message\":\"the corpus cannot be served: notes/poison.md is not UTF-8\"}}\n",
        )
        .unwrap();
        w.flush().unwrap();
    });

    let Err(err) = SocketDoor::connect(&sock, dir.path(), GREET_CAP) else {
        panic!("an ok:false handshake is a refusal");
    };
    served.join().unwrap();
    let rendered = err.to_string();
    assert!(
        rendered.contains("invalid_utf8")
            && rendered.contains("notes/poison.md")
            && rendered.contains("is not UTF-8"),
        "the refusal reaches the operator with the daemon's code, path, and \
         message intact: {rendered}"
    );
}
