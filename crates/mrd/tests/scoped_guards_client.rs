//! The mrd CLIENT half of the `scoped-guards` family (fp-grain PR-3) —
//! cap-aware and DORMANT until the family flips engine-side.
//!
//! Law under test (wire-contract §3.2/§5.4/§4.7; ruling
//! `decisions/2026-08-16-fp-grain-ruling.md` items 3 and 6):
//!
//! - `mrd put --scope` sends the §5.4 singular `scope` beside `if_fingerprint`
//!   ONLY when the connect-time hello advertised `scoped-guards`; against a
//!   daemon without the cap it refuses with a teaching at exit 2, before any
//!   engine write — never a strict-wall `bad_request` from an un-negotiated
//!   field.
//! - `--scope` without `--if-fingerprint` is half a premise: refused at parse.
//! - `mrd read --json` under a cap-serving hello captures the read file's
//!   scoped token through the §4.7 mint arm — one extra `fingerprint {scope}`
//!   exchange on the same connection — relayed verbatim as the house frame's
//!   `mint` key. The human face never mints; a cap-less hello never mints; the
//!   `read` body's own `fingerprint` stays the ambient world token (§5.1).
//!
//! Harness: the cap-PRESENT half runs against a canned-frame fake daemon (a
//! `UnixListener` at the sandbox's derived socket path answering the control
//! protocol and the wire ops, publishing this build's own identity so the 0025
//! skew law passes) — the REAL engine does not decode the family until the
//! flip PR, so these tests pin the client's wire behavior, not the engine's.
//! The DORMANCY half runs against the real in-process `RunningServer`.
//!
//! ⚠ Deliberate PR-4 tripwire: when the engine starts advertising
//! `scoped-guards`, [`put_scope_against_a_capless_daemon_is_taught_exit_2`]
//! and [`read_json_against_a_capless_daemon_stays_bare`] fail by design —
//! the flip PR replaces them with the dogfood-verbatim e2e.

use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use registry::{Config, RunningServer};
use serde_json::{Value, json};

fn mrd_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mrd")
}

/// The identity this test's own compilation carries — the same crate stamp the
/// `mrd` binary under test bakes (one `build.rs` run feeds every target).
const OWN_BUILD: &str = env!("MRD_BUILD_SHA");

const DOC: &str = "# Alpha\n\none two three\n";
const EDIT: &str = r#"[{"target":{"hpath":[{"h":"Alpha"}]},"edit":{"match":{"old":"one two three","new":"one two three four"}}}]"#;

struct Sandbox {
    tmp: tempfile::TempDir,
    cache_home: PathBuf,
    home: PathBuf,
}

fn sandbox() -> Sandbox {
    let tmp = tempfile::tempdir().expect("tempdir");
    // One-letter segments on purpose: the derived socket path must fit
    // `sun_path` (104 bytes on macOS) under the canonical non-symlinked
    // TMPDIR (`/private/var/folders/…`, 57 bytes) — `xdg-cache/` pushes
    // `…/meridian/registry/daemon.sock` to 107 and the bind refuses SUN_LEN.
    let cache_home = tmp.path().join("c");
    let home = tmp.path().join("h");
    std::fs::create_dir_all(&home).expect("home");
    Sandbox {
        tmp,
        cache_home,
        home,
    }
}

impl Sandbox {
    fn cache_root(&self) -> PathBuf {
        self.cache_home.join("meridian")
    }

    /// The derived socket path every client under this sandbox dials.
    fn socket(&self) -> PathBuf {
        self.cache_root().join("registry").join("daemon.sock")
    }

    /// An anchored workspace holding the fixture doc.
    fn workspace(&self) -> PathBuf {
        let ws = self.tmp.path().join("project");
        std::fs::create_dir_all(ws.join(".git")).expect("git anchor");
        std::fs::write(ws.join("doc.md"), DOC).expect("doc");
        std::fs::canonicalize(&ws).expect("canonical ws")
    }

    /// The real in-process daemon, publishing this build's identity — the
    /// dormancy half: the real engine does not advertise `scoped-guards`.
    #[allow(clippy::duration_suboptimal_units)]
    fn real_daemon(&self) -> RunningServer {
        let forever = Duration::from_secs(365 * 24 * 60 * 60);
        let mut config = Config::for_cache_root(self.cache_root());
        config.idle_threshold = forever;
        config.reap_interval = forever;
        config.prewarm_interval = forever;
        config.prewarm_quiet_max = forever;
        config.idle_exit = None;
        config.build_sha = Some(OWN_BUILD.to_owned());
        RunningServer::start(config).expect("in-process daemon binds the derived socket")
    }

    /// A canned-frame fake daemon at the derived socket path: answers the
    /// control protocol (`ping` → `pong`, `resolve_ws` → `miss`) and the wire
    /// ops with fixed frames, publishes `caps` on hello beside this build's
    /// own identity, and records every received wire frame into `log`.
    fn fake_daemon(&self, caps: &'static [&'static str], log: &Arc<Mutex<Vec<Value>>>) {
        let socket = self.socket();
        std::fs::create_dir_all(socket.parent().expect("registry dir")).expect("mkdir");
        let listener = UnixListener::bind(&socket).expect("bind fake daemon socket");
        let log = Arc::clone(log);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                let log = Arc::clone(&log);
                std::thread::spawn(move || serve_connection(&stream, caps, &log));
            }
        });
    }

    fn run(&self, cwd: &Path, args: &[&str], stdin: Option<&str>) -> Output {
        let mut cmd = Command::new(mrd_bin());
        cmd.args(args)
            .current_dir(cwd)
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
            .env_remove("MERIDIAN_WORKSPACE")
            // No auto-spawn may hide a test bug: a dead socket must fail loud,
            // not quietly start a real daemon over the fake.
            .env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-daemon");
        if let Some(bytes) = stdin {
            let mut child = cmd
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("spawn mrd");
            child
                .stdin
                .as_mut()
                .expect("stdin")
                .write_all(bytes.as_bytes())
                .expect("write stdin");
            child.wait_with_output().expect("wait")
        } else {
            cmd.output().expect("spawn mrd")
        }
    }
}

/// One fake-daemon connection: NDJSON in, canned NDJSON out. Wire frames (the
/// ones with an `op` the wire owns) are recorded for the test's assertions;
/// control frames are answered and not recorded.
fn serve_connection(stream: &UnixStream, caps: &[&str], log: &Arc<Mutex<Vec<Value>>>) {
    let mut writer = stream.try_clone().expect("clone fake stream");
    let reader = BufReader::new(stream.try_clone().expect("clone fake stream"));
    for line in reader.lines() {
        let Ok(line) = line else { return };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(frame) = serde_json::from_str::<Value>(&line) else {
            return;
        };
        let op = frame
            .get("op")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let answer = match op.as_str() {
            "ping" => json!({"status":"pong"}),
            "resolve_ws" => json!({"status":"miss"}),
            "hello" => json!({"ok":true,"body":{
                "proto":1,"server":"fake-daemon/0","contract":"v3",
                "caps": caps,
                "identity": {"build": OWN_BUILD},
                "workspace": frame.get("workspace").cloned().unwrap_or(Value::Null)}}),
            "read" => {
                log.lock().expect("log").push(frame.clone());
                json!({"ok":true,"body":{
                    "path":"doc.md","fingerprint":"b3:world-token","toc":[],
                    "rendered_text":"# Alpha\n"}})
            }
            "fingerprint" => {
                log.lock().expect("log").push(frame.clone());
                json!({"ok":true,"body":{
                    "fingerprint":"b3:leaf-token",
                    "scope": frame.get("scope").cloned().unwrap_or(Value::Null),
                    "seq":1}})
            }
            "splice" => {
                log.lock().expect("log").push(frame.clone());
                json!({"ok":true,"body":{
                    "path": frame.get("path").cloned().unwrap_or(Value::Null),
                    "fingerprint_before":"b3:leaf-token",
                    "fingerprint_after":"b3:after-token",
                    "file_rev_before":"b3:r1","file_rev_after":"b3:r2",
                    "armed":{"path":"doc.md","edits":[{"target":{"hpath":[{"h":"Alpha"}]}}]},
                    "verdicts":[]}})
            }
            _ => {
                log.lock().expect("log").push(frame.clone());
                json!({"ok":false,"error":{
                    "code":"bad_request","recovery":"fix_request",
                    "message":"fake daemon: unexpected op"}})
            }
        };
        let mut out = serde_json::to_string(&answer).expect("encode");
        out.push('\n');
        if writer.write_all(out.as_bytes()).is_err() {
            return;
        }
    }
}

fn stdout_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}
fn stderr_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}
fn ops_named<'a>(log: &'a [Value], op: &str) -> Vec<&'a Value> {
    log.iter()
        .filter(|f| f.get("op").and_then(Value::as_str) == Some(op))
        .collect()
}

/// The caps a scoped-guards-serving daemon's hello carries (fake half).
const CAPS_WITH_FAMILY: &[&str] = &[
    "toc",
    "splice",
    "splice.if_fingerprint",
    "fingerprint",
    "scoped-guards",
];
/// A v3 hello WITHOUT the family — dormancy against the fake, cap-keyed
/// (proves the client keys on the cap string, not on "is this daemon fake").
const CAPS_WITHOUT_FAMILY: &[&str] = &["toc", "splice", "splice.if_fingerprint", "fingerprint"];

// ---------------------------------------------------------------------------
// help face (no daemon)
// ---------------------------------------------------------------------------

/// `--help` is the face the status.md paragraph claims to teach from.
#[test]
fn put_help_names_scope_and_the_cap() {
    let out = Command::new(mrd_bin())
        .args(["put", "--help"])
        .output()
        .expect("mrd put --help");
    assert_eq!(out.status.code(), Some(0), "put --help is a success");
    let text = stdout_of(&out);
    assert!(
        text.contains("--scope PATH"),
        "the put synopsis names --scope: {text}"
    );
    assert!(
        text.contains("scoped-guards"),
        "the put page names the cap that arms --scope: {text}"
    );
}

#[test]
fn read_help_names_the_mint_capture() {
    let out = Command::new(mrd_bin())
        .args(["read", "--help"])
        .output()
        .expect("mrd read --help");
    assert_eq!(out.status.code(), Some(0), "read --help is a success");
    let text = stdout_of(&out);
    assert!(
        text.contains("mint"),
        "the read page names the mint key: {text}"
    );
    assert!(
        text.contains("scoped-guards"),
        "the read page names the cap that arms the mint: {text}"
    );
}

// ---------------------------------------------------------------------------
// put --scope
// ---------------------------------------------------------------------------

/// Under a cap-serving hello the scoped premise rides the wire verbatim:
/// `scope` beside `if_fingerprint`, both exactly as given.
#[test]
fn put_scope_rides_the_wire_when_the_hello_serves_scoped_guards() {
    let sb = sandbox();
    let ws = sb.workspace();
    let log = Arc::new(Mutex::new(Vec::new()));
    sb.fake_daemon(CAPS_WITH_FAMILY, &log);

    let out = sb.run(
        &ws,
        &[
            "put",
            "doc.md",
            "--if-fingerprint",
            "b3:leaf-token",
            "--scope",
            "doc.md",
            "--json",
        ],
        Some(EDIT),
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "a scoped put against a cap-serving daemon commits\nstdout: {}\nstderr: {}",
        stdout_of(&out),
        stderr_of(&out)
    );

    let log = log.lock().expect("log");
    let splices = ops_named(&log, "splice");
    assert_eq!(
        splices.len(),
        1,
        "exactly one splice rode the wire: {log:?}"
    );
    assert_eq!(
        splices[0]["scope"],
        json!("doc.md"),
        "the §5.4 singular scope rides verbatim: {:?}",
        splices[0]
    );
    assert_eq!(
        splices[0]["if_fingerprint"],
        json!("b3:leaf-token"),
        "the premise token rides beside its scope: {:?}",
        splices[0]
    );

    let frame: Value = serde_json::from_str(&stdout_of(&out)).expect("--json frame");
    assert!(
        frame.get("put").is_some(),
        "the commit frame is {{workspace, put}}: {frame}"
    );
}

/// Without the cap the scoped put refuses BEFORE any engine write, with the
/// teaching — fake half, cap-keyed.
#[test]
fn put_scope_without_the_cap_refuses_taught_and_sends_no_splice() {
    let sb = sandbox();
    let ws = sb.workspace();
    let log = Arc::new(Mutex::new(Vec::new()));
    sb.fake_daemon(CAPS_WITHOUT_FAMILY, &log);

    let out = sb.run(
        &ws,
        &[
            "put",
            "doc.md",
            "--if-fingerprint",
            "b3:leaf-token",
            "--scope",
            "doc.md",
        ],
        Some(EDIT),
    );
    let err = stderr_of(&out);
    assert_eq!(
        out.status.code(),
        Some(2),
        "cap-less scoped put is the CLI's own refusal (exit 2)\nstdout: {}\nstderr: {err}",
        stdout_of(&out)
    );
    assert!(
        err.contains("scoped-guards"),
        "the teaching names the missing cap: {err}"
    );
    assert!(
        err.contains("nothing was sent") || err.contains("Nothing was sent"),
        "the nothing-happened clause: {err}"
    );
    assert!(
        err.contains("without --scope"),
        "the world-grain retry suggestion: {err}"
    );

    let log = log.lock().expect("log");
    assert!(
        ops_named(&log, "splice").is_empty(),
        "no splice reached the daemon: {log:?}"
    );
}

/// The §5.4 pair law at parse: a scope with no fingerprint is half a premise —
/// refused before stdin, before the workspace, before any dial.
#[test]
fn put_scope_without_if_fingerprint_is_half_a_premise() {
    let sb = sandbox();
    let ws = sb.workspace();
    // No daemon at all: the parse wall answers first.
    let out = sb.run(&ws, &["put", "doc.md", "--scope", "doc.md"], Some(EDIT));
    let err = stderr_of(&out);
    assert_eq!(
        out.status.code(),
        Some(2),
        "half a premise is a bad invocation\nstdout: {}\nstderr: {err}",
        stdout_of(&out)
    );
    assert!(
        err.contains("half a premise"),
        "the pair-law teaching: {err}"
    );
    assert!(
        err.contains("--if-fingerprint"),
        "the teaching names the missing half: {err}"
    );
}

// ---------------------------------------------------------------------------
// read --json mint capture
// ---------------------------------------------------------------------------

/// Under a cap-serving hello a `--json` read captures the scoped token: one
/// `fingerprint {scope}` exchange on the same connection, relayed verbatim as
/// the frame's `mint` key — while the read body's own `fingerprint` stays the
/// world token (§5.1 unchanged, ruling item 7).
#[test]
fn read_json_mints_the_scoped_token_when_the_hello_serves_scoped_guards() {
    let sb = sandbox();
    let ws = sb.workspace();
    let log = Arc::new(Mutex::new(Vec::new()));
    sb.fake_daemon(CAPS_WITH_FAMILY, &log);

    let out = sb.run(&ws, &["read", "doc.md", "--json"], None);
    assert_eq!(
        out.status.code(),
        Some(0),
        "the warm read serves\nstdout: {}\nstderr: {}",
        stdout_of(&out),
        stderr_of(&out)
    );

    let frame: Value = serde_json::from_str(&stdout_of(&out)).expect("--json frame");
    assert_eq!(
        frame["mint"]["fingerprint"],
        json!("b3:leaf-token"),
        "the mint body rides the frame verbatim: {frame}"
    );
    assert_eq!(
        frame["mint"]["scope"],
        json!("doc.md"),
        "the mint echoes the scope it bound (§4.7 desync guard): {frame}"
    );
    assert_eq!(
        frame["read"]["fingerprint"],
        json!("b3:world-token"),
        "the read body's fingerprint stays the ambient world token (§5.1): {frame}"
    );

    let log = log.lock().expect("log");
    let mints = ops_named(&log, "fingerprint");
    assert_eq!(mints.len(), 1, "exactly one mint exchange: {log:?}");
    assert_eq!(
        mints[0]["scope"],
        json!("doc.md"),
        "the mint names the read path as its scope: {:?}",
        mints[0]
    );
}

/// The human face never mints — no capture consumer, no extra round trip.
#[test]
fn read_human_never_mints_even_under_the_cap() {
    let sb = sandbox();
    let ws = sb.workspace();
    let log = Arc::new(Mutex::new(Vec::new()));
    sb.fake_daemon(CAPS_WITH_FAMILY, &log);

    let out = sb.run(&ws, &["read", "doc.md"], None);
    assert_eq!(
        out.status.code(),
        Some(0),
        "the human read serves\nstderr: {}",
        stderr_of(&out)
    );
    let log = log.lock().expect("log");
    assert!(
        ops_named(&log, "fingerprint").is_empty(),
        "no mint exchange on the human face: {log:?}"
    );
}

// ---------------------------------------------------------------------------
// Dormancy against the REAL engine (pre-flip builds)
// ---------------------------------------------------------------------------

/// ⚠ PR-4 tripwire. Today's real daemon does not advertise `scoped-guards`,
/// so the scoped put refuses taught and writes nothing. When the family flips
/// engine-side this test fails by design — the flip PR replaces it with the
/// dogfood-verbatim e2e (ruling item 4).
#[test]
fn put_scope_against_a_capless_daemon_is_taught_exit_2() {
    let sb = sandbox();
    let ws = sb.workspace();
    let _daemon = sb.real_daemon();
    let before = std::fs::read_to_string(ws.join("doc.md")).expect("before");

    let out = sb.run(
        &ws,
        &[
            "put",
            "doc.md",
            "--if-fingerprint",
            "b3:anything",
            "--scope",
            "doc.md",
        ],
        Some(EDIT),
    );
    let err = stderr_of(&out);
    assert_eq!(
        out.status.code(),
        Some(2),
        "the real pre-flip daemon draws the taught refusal\nstdout: {}\nstderr: {err}",
        stdout_of(&out)
    );
    assert!(
        err.contains("scoped-guards"),
        "the teaching names the cap: {err}"
    );
    assert_eq!(
        std::fs::read_to_string(ws.join("doc.md")).expect("after"),
        before,
        "nothing was written"
    );
}

/// ⚠ PR-4 tripwire, read half: no cap ⇒ no mint call and no `mint` key — the
/// `--json` frame stays byte-compatible with pre-family builds.
#[test]
fn read_json_against_a_capless_daemon_stays_bare() {
    let sb = sandbox();
    let ws = sb.workspace();
    let _daemon = sb.real_daemon();

    let out = sb.run(&ws, &["read", "doc.md", "--json"], None);
    assert_eq!(
        out.status.code(),
        Some(0),
        "the warm read serves\nstderr: {}",
        stderr_of(&out)
    );
    let frame: Value = serde_json::from_str(&stdout_of(&out)).expect("--json frame");
    assert!(
        frame.get("mint").is_none(),
        "no cap, no mint key — dormant: {frame}"
    );
    // The token's PRESENCE is this test's fact, never its spelling: the
    // interior encoding is moving under the width ruling
    // (`node-rev-merkle-spec.md`), and a served value this file pinned would
    // fail on a law it has no stake in.
    assert!(
        frame["read"]["fingerprint"]
            .as_str()
            .is_some_and(|fp| !fp.is_empty()),
        "the read still serves the world token: {frame}"
    );
}
