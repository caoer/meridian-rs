//! Pin proof rides the request (§ A.3 proof law), proven over one live
//! daemon's socket: a sections read serves each section's `fp1.…` token, a
//! `splice.pin` carries it back, and the engine recomputes live and compares.
//! No server-side read state exists — which is why the token survives a
//! daemon restart, the exact class the killed read-receipt store lost.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use registry::{Config, RunningServer};
use serde_json::{Value, json};
use tempfile::TempDir;

mod common;

/// A daemon config rooted under `tmp`, with horizons large enough that neither
/// the reaper nor the pre-warm thread fires mid-test.
#[allow(clippy::duration_suboptimal_units)]
fn test_config(tmp: &TempDir) -> Config {
    let forever = Duration::from_secs(365 * 24 * 60 * 60);
    let dir = tmp.path().join("registry");
    let mut config = Config::for_cache_root(tmp.path().join("cache"));
    config.socket_path = dir.join("daemon.sock");
    config.state_path = dir.join("state.json");
    config.idle_threshold = forever;
    config.reap_interval = forever;
    config.prewarm_interval = forever;
    config.prewarm_quiet_max = forever;
    config.idle_exit = None;
    config.drain_cold_builds = Duration::from_secs(30);
    config
}

/// A workspace `tmp/ws` seeded with `files`, git-initialised so a pin's R4
/// row can carry its blob oid.
fn write_ws(tmp: &TempDir, files: &[(&str, &str)]) -> PathBuf {
    let ws = tmp.path().join("ws");
    for (rel, content) in files {
        let path = ws.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }
    for args in [
        vec!["init", "-q"],
        vec!["add", "."],
        vec![
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-qm",
            "s0",
        ],
    ] {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(&ws)
            .args(&args)
            .status()
            .expect("git runs in the test environment");
        assert!(status.success(), "git {args:?}");
    }
    ws
}

/// A persistent NDJSON connection: `hello` binds the workspace, then every op
/// rides the same connection.
struct Conn {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
}

impl Conn {
    fn open(socket: &Path) -> Self {
        let stream = UnixStream::connect(socket).unwrap();
        Conn {
            writer: stream.try_clone().unwrap(),
            reader: BufReader::new(stream),
        }
    }

    fn call(&mut self, request: &Value) -> Value {
        common::honour_retry(|| {
            let mut line = serde_json::to_string(request).unwrap();
            line.push('\n');
            self.writer.write_all(line.as_bytes()).unwrap();
            self.writer.flush().unwrap();
            let mut response = String::new();
            self.reader.read_line(&mut response).unwrap();
            serde_json::from_str(&response).unwrap()
        })
    }

    fn hello(&mut self, ws: &Path) -> Value {
        self.call(&json!({
            "op": "hello",
            "proto": 1,
            "contract": "v3",
            "workspace": ws.to_str().unwrap(),
        }))
    }

    /// A composed sections read; returns the whole response.
    fn read_sections(&mut self, path: &str, selectors: &[&str]) -> Value {
        let out = self.call(&json!({
            "op": "read",
            "path": path,
            "sections": selectors
                .iter()
                .map(|s| wire::ReadSel::parse(s))
                .collect::<Vec<_>>(),
        }));
        assert_eq!(out["ok"], json!(true), "read ok: {out}");
        out
    }
}

const PLAN: &str = "# Alpha\n\nalpha body\n\n# Beta\n\nbeta body\n";
const PINNER: &str = "# Plan\n\ndraws from the plan.\n";

/// The proof pair the read served for `sel`: (`fingerprint`, `sec_rev`).
fn served_proof(read: &Value, sel: &str) -> (String, String) {
    let row = read["body"]["sections"]
        .as_array()
        .expect("sections array")
        .iter()
        .find(|s| s["sel"] == json!(wire::ReadSel::parse(sel)))
        .unwrap_or_else(|| panic!("section {sel} served: {read}"))
        .clone();
    (
        row["fingerprint"]
            .as_str()
            .expect("a served section carries its proof token")
            .to_string(),
        row["sec_rev"].as_str().expect("sec_rev string").to_string(),
    )
}

/// The pin request frame, proof fields riding when given.
fn pin_frame(id: u64, actor: &str, fingerprint: Option<&str>) -> Value {
    let mut pin = json!({
        "target": "plan.md",
        "selector": {"hpath": [{"h": "Alpha"}]},
    });
    if let Some(fp) = fingerprint {
        pin["fingerprint"] = json!(fp);
    }
    json!({
        "id": id, "op": "splice", "path": "pinner.md",
        "actor": actor, "now": "2026-08-16T12:00:00Z",
        "pin": pin,
    })
}

/// Gate 1 — a sections read serves each section's own `fp1.…` proof token;
/// the toc mode serves none (a map does not prove a read).
#[test]
fn a_sections_read_serves_the_proof_token_and_a_toc_read_serves_none() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("plan.md", PLAN)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    conn.hello(&ws);

    let read = conn.read_sections("plan.md", &["Alpha", "Beta"]);
    let (alpha_fp, _) = served_proof(&read, "Alpha");
    let (beta_fp, _) = served_proof(&read, "Beta");
    assert!(
        alpha_fp.starts_with("fp1."),
        "the token is the content-identity CID form: {alpha_fp}"
    );
    assert_ne!(alpha_fp, beta_fp, "each section serves its own token");

    let toc = conn.call(&json!({"op": "read", "path": "plan.md"}));
    assert_eq!(toc["ok"], json!(true), "toc read ok: {toc}");
    let rows = toc["body"]["toc"].as_array().expect("toc rows");
    assert!(!rows.is_empty(), "the toc read served the map: {toc}");
    assert!(
        rows.iter().all(|r| r.get("fingerprint").is_none()),
        "no toc row serves a proof token: {toc}"
    );

    server.shutdown();
}

/// Gate 2 — the MCP shape end to end: a session actor's pin carries the token
/// its own read served, and commits. Without the token, the same pin refuses
/// `pin_proof_required` and writes nothing.
#[test]
fn an_actor_pin_carries_the_reads_token_and_commits_where_a_proofless_one_refuses() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("plan.md", PLAN), ("pinner.md", PINNER)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    conn.hello(&ws);

    let refused = conn.call(&pin_frame(6, "agent-7", None));
    assert_eq!(refused["ok"], json!(false), "{refused}");
    assert_eq!(
        refused["error"]["code"],
        json!("pin_proof_required"),
        "{refused}"
    );
    assert_eq!(
        fs::read_to_string(ws.join("pinner.md")).unwrap(),
        PINNER,
        "a refused pin writes nothing"
    );

    let read = conn.read_sections("plan.md", &["Alpha"]);
    let (proof, _) = served_proof(&read, "Alpha");
    let out = conn.call(&pin_frame(7, "agent-7", Some(&proof)));
    assert_eq!(out["ok"], json!(true), "the read-backed pin commits: {out}");
    assert_eq!(
        out["body"]["pin"]["fingerprint"],
        json!(proof),
        "the minted fact IS the carried token: {out}"
    );

    server.shutdown();
}

/// Gate 3 — the token survives a daemon restart: no server-side state backs
/// it, so the restart/reap evaporation class of the killed receipt store
/// cannot exist. The read happens on daemon A; the pin commits on daemon B.
#[test]
fn the_proof_survives_a_daemon_restart_because_no_server_state_backs_it() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("plan.md", PLAN), ("pinner.md", PINNER)]);

    let first = RunningServer::start(test_config(&tmp)).unwrap();
    let proof = {
        let mut conn = Conn::open(first.socket_path());
        conn.hello(&ws);
        let read = conn.read_sections("plan.md", &["Alpha"]);
        served_proof(&read, "Alpha").0
    };
    first.shutdown();

    let second = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(second.socket_path());
    conn.hello(&ws);
    let out = conn.call(&pin_frame(9, "agent-7", Some(&proof)));
    assert_eq!(
        out["ok"],
        json!(true),
        "the token from before the restart still proves the read: {out}"
    );
    second.shutdown();
}

/// Gate 4 — the retired `actor` field on `read` refuses at the strict wall:
/// a read is identity-free, and the wall says so out loud rather than
/// silently ignoring an identity the engine would do nothing with.
#[test]
fn an_actor_on_read_refuses_at_the_strict_wall() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("plan.md", PLAN)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    conn.hello(&ws);

    let out = conn.call(&json!({
        "op": "read", "path": "plan.md",
        "sections": [{"hpath": [{"h": "Alpha"}]}],
        "actor": "agent-7",
    }));
    assert_eq!(out["ok"], json!(false), "{out}");
    assert_eq!(out["error"]["code"], json!("bad_request"), "{out}");

    server.shutdown();
}
