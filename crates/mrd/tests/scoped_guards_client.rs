//! The mrd CLIENT half of the `scoped-guards` family (fp-grain PR-3 +
//! PR-4 flip). Cap-aware: `--scope` and the §4.7 mint ride only when
//! hello advertised the family.
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
//! - a `--scope` spelling the §1 path law refuses (`../escape.md`, an
//!   absolute path, the empty string) refuses at the face with the family
//!   teaching, exit 2 before any dial — dogfood 88877785 measured the bare
//!   engine echo (`bad_path: ../escape.md`, and NOTHING after the colon for
//!   the empty spelling): no rule, no law, no recovery. The `--json` face
//!   keeps the `{workspace, error}` frame.
//! - `mrd read --json` under a cap-serving hello captures the read file's
//!   scoped token through the §4.7 mint arm — one extra `fingerprint {scope}`
//!   exchange on the same connection — relayed verbatim as the house frame's
//!   `mint` key. The human face never mints; a cap-less hello never mints; the
//!   `read` body's own `fingerprint` stays the ambient world token (§5.1).
//!
//! Harness: the cap-PRESENT client-shape half still runs against a canned
//! fake daemon (identity-matched so the 0025 skew law passes). The
//! dogfood-verbatim acceptance half runs against the real in-process
//! `RunningServer`, which now advertises `scoped-guards`.

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

    /// The real in-process daemon, publishing this build's identity.
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
// put --scope: the §1 path-law wall (dogfood 88877785)
// ---------------------------------------------------------------------------

/// A `--scope` spelling the §1 path law refuses is the CLI's own refusal:
/// exit 2 with the family teaching and a recovery, and NO frame reaches the
/// daemon. The dogfood measured `mrd: bad_path: ../escape.md` — no rule, no
/// law, no recovery.
#[test]
fn put_scope_dotdot_refuses_taught_and_sends_nothing() {
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
            "../escape.md",
        ],
        Some(EDIT),
    );
    let err = stderr_of(&out);
    assert_eq!(
        out.status.code(),
        Some(2),
        "a path-law-violating scope is a bad invocation\nstdout: {}\nstderr: {err}",
        stdout_of(&out)
    );
    assert!(
        err.contains("--scope ../escape.md is not a workspace-relative path"),
        "the teaching names the flag and speaks the family voice: {err}"
    );
    assert!(
        err.contains("§1 path law"),
        "the teaching names the law: {err}"
    );
    assert!(
        err.contains("Nothing was sent and nothing was written"),
        "the nothing-happened clause: {err}"
    );
    assert!(
        err.contains("mint") && err.contains("Without --scope"),
        "both recoveries: the §4.7 mint echo and the world-grain retry: {err}"
    );

    let log = log.lock().expect("log");
    assert!(
        ops_named(&log, "splice").is_empty(),
        "no splice reached the daemon: {log:?}"
    );
}

/// The empty `--scope` — the measured mistake is an unquoted shell variable
/// that expanded to nothing — refuses taught before any dial (no daemon runs
/// here). The dogfood measured `mrd: bad_path: ` with nothing after the
/// colon: the wire echo of an empty spelling names nothing.
#[test]
fn put_scope_empty_refuses_taught_before_any_dial() {
    let sb = sandbox();
    let ws = sb.workspace();
    let out = sb.run(
        &ws,
        &[
            "put",
            "doc.md",
            "--if-fingerprint",
            "b3:leaf-token",
            "--scope",
            "",
        ],
        Some(EDIT),
    );
    let err = stderr_of(&out);
    assert_eq!(
        out.status.code(),
        Some(2),
        "an empty scope is a bad invocation\nstdout: {}\nstderr: {err}",
        stdout_of(&out)
    );
    assert!(
        err.contains("--scope is empty"),
        "the teaching names the flag and the emptiness — the echo alone cannot: {err}"
    );
    assert!(
        err.contains("§1 path law"),
        "the teaching names the law: {err}"
    );
    assert!(
        err.contains("shell variable"),
        "the teaching names the measured cause: {err}"
    );
    assert!(
        err.contains("Nothing was sent and nothing was written"),
        "the nothing-happened clause: {err}"
    );
    assert!(
        err.contains("mint") && err.contains("Without --scope"),
        "both recoveries: the §4.7 mint echo and the world-grain retry: {err}"
    );
}

/// Under `--json` the wall keeps the machine face: the `{workspace, error}`
/// envelope on stdout, `bad_path` code and echoed path unchanged, the
/// teaching riding the §8 `message` field.
#[test]
fn put_scope_bad_path_json_face_keeps_the_error_frame() {
    let sb = sandbox();
    let ws = sb.workspace();
    let out = sb.run(
        &ws,
        &[
            "put",
            "doc.md",
            "--if-fingerprint",
            "b3:leaf-token",
            "--scope",
            "../escape.md",
            "--json",
        ],
        Some(EDIT),
    );
    assert_eq!(
        out.status.code(),
        Some(2),
        "the wall's triad is unchanged by --json\nstderr: {}",
        stderr_of(&out)
    );
    let frame: Value = serde_json::from_str(&stdout_of(&out)).expect("--json frame");
    assert!(
        frame.get("workspace").is_some(),
        "the envelope names the workspace: {frame}"
    );
    assert_eq!(
        frame["error"]["code"],
        json!("bad_path"),
        "the code is unchanged: {frame}"
    );
    assert_eq!(
        frame["error"]["path"],
        json!("../escape.md"),
        "the offending spelling is echoed unchanged: {frame}"
    );
    assert!(
        frame["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("§1 path law"),
        "the teaching rides the §8 message field: {frame}"
    );
}

/// An absolute spelling that lies INSIDE the workspace earns the fitted
/// respell — the one shipped computation, same as every family door.
#[test]
fn put_scope_absolute_inside_earns_the_respell() {
    let sb = sandbox();
    let ws = sb.workspace();
    let abs = ws.join("doc.md");
    let out = sb.run(
        &ws,
        &[
            "put",
            "doc.md",
            "--if-fingerprint",
            "b3:leaf-token",
            "--scope",
            abs.to_str().expect("utf8 abs path"),
        ],
        Some(EDIT),
    );
    let err = stderr_of(&out);
    assert_eq!(
        out.status.code(),
        Some(2),
        "an absolute scope refuses at the wall\nstderr: {err}"
    );
    assert!(
        err.contains("respell it as `doc.md`"),
        "a spelling inside the workspace earns the fitted respell: {err}"
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
// Dogfood against the REAL engine (PR-4 flip)
// ---------------------------------------------------------------------------

/// Ruling item 4 through adopted mrd: capture the file token, birth a
/// disjoint sibling, put the untouched file pinned on that token — commits.
#[test]
fn put_scope_survives_a_disjoint_sibling_birth() {
    let sb = sandbox();
    let ws = sb.workspace();
    std::fs::create_dir_all(ws.join("a")).expect("mkdir");
    std::fs::write(ws.join("a/target.md"), DOC).expect("seed");
    let _daemon = sb.real_daemon();

    let read = sb.run(&ws, &["read", "a/target.md", "--json"], None);
    assert_eq!(
        read.status.code(),
        Some(0),
        "the capture read serves\nstderr: {}",
        stderr_of(&read)
    );
    let frame: Value = serde_json::from_str(&stdout_of(&read)).expect("--json frame");
    let token = frame["mint"]["fingerprint"]
        .as_str()
        .expect("the serving daemon mints the file token")
        .to_owned();
    assert_eq!(
        frame["mint"]["scope"],
        json!("a/target.md"),
        "the mint echoes the read path: {frame}"
    );
    assert_ne!(
        frame["read"]["fingerprint"].as_str(),
        Some(token.as_str()),
        "the read body's fingerprint stays the world token (§5.1): {frame}"
    );

    std::fs::create_dir_all(ws.join("b")).expect("mkdir");
    std::fs::write(ws.join("b/mover.md"), "# Mover\n\ndisjoint.\n").expect("birth");

    let put = sb.run(
        &ws,
        &[
            "put",
            "a/target.md",
            "--if-fingerprint",
            &token,
            "--scope",
            "a/target.md",
        ],
        Some(EDIT),
    );
    assert_eq!(
        put.status.code(),
        Some(0),
        "a disjoint sibling must not refuse a file-scoped put\nstdout: {}\nstderr: {}",
        stdout_of(&put),
        stderr_of(&put)
    );
    let bytes = std::fs::read_to_string(ws.join("a/target.md")).expect("after");
    assert!(
        bytes.contains("one two three four"),
        "the edit reached disk: {bytes}"
    );
}

/// The counter-proof through adopted mrd: a write that moves the planned
/// scope still refuses, naming the scope.
#[test]
fn put_scope_refuses_when_the_scope_itself_moved() {
    let sb = sandbox();
    let ws = sb.workspace();
    std::fs::create_dir_all(ws.join("a")).expect("mkdir");
    std::fs::write(ws.join("a/target.md"), DOC).expect("seed");
    let _daemon = sb.real_daemon();

    let read = sb.run(&ws, &["read", "a/target.md", "--json"], None);
    assert_eq!(read.status.code(), Some(0), "{}", stderr_of(&read));
    let frame: Value = serde_json::from_str(&stdout_of(&read)).expect("--json frame");
    let token = frame["mint"]["fingerprint"]
        .as_str()
        .expect("minted")
        .to_owned();

    std::fs::write(
        ws.join("a/target.md"),
        format!("{DOC}\nout-of-band tail.\n"),
    )
    .expect("move the scope");

    let put = sb.run(
        &ws,
        &[
            "put",
            "a/target.md",
            "--if-fingerprint",
            &token,
            "--scope",
            "a/target.md",
        ],
        Some(EDIT),
    );
    assert_eq!(
        put.status.code(),
        Some(1),
        "the moved scope refuses at the engine\nstdout: {}\nstderr: {}",
        stdout_of(&put),
        stderr_of(&put)
    );
    let err = format!("{}{}", stdout_of(&put), stderr_of(&put));
    assert!(
        err.contains("fingerprint_mismatch") || err.contains("the premise at a/target.md moved"),
        "the refusal names the scoped mismatch: {err}"
    );
}
