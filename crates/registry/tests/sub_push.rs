//! Serve-side `Op::Sub` gates: push channel, per-workspace ring, S2/S6
//! boundaries, chain contiguity. Every gate drives the real daemon over its
//! socket (the push path is a connection property; in-process re-call would
//! test a shape production wiring does not have).
//!
//! Positional frame selects are sound because a subscribed connection cannot
//! desync: `serve_conn` returns into `push_loop` on an accepted `Sub`, so one
//! connection is request-or-push, never both (`crates/registry/src/server.rs`).

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use registry::{Config, RunningServer};
use serde_json::{Value, json};
use tempfile::TempDir;

mod common;

/// Wait for a pushed frame before calling it absent. Covers 250ms detect
/// cadence + 50ms push tick with margin against flakes.
const PUSH_WAIT: Duration = Duration::from_secs(10);

/// Longer than any test's life: the horizon a gate parks when it is not the
/// thing under test. Also the fixture park (`Config::activity_park`).
#[allow(clippy::duration_suboptimal_units)]
const FOREVER: Duration = Duration::from_secs(365 * 24 * 60 * 60);

/// Reaper and pre-warm parked far ahead — default for gates not about the reaper.
fn test_config(tmp: &TempDir) -> Config {
    let dir = tmp.path().join("registry");
    let mut config = Config::for_cache_root(tmp.path().join("cache"));
    config.socket_path = dir.join("daemon.sock");
    config.state_path = dir.join("state.json");
    config.idle_threshold = FOREVER;
    config.reap_interval = FOREVER;
    config.prewarm_interval = FOREVER;
    config.prewarm_quiet_max = FOREVER;
    // Lifetime is the test's; idle-exit would flake mid-assertion.
    config.idle_exit = None;
    config.drain_cold_builds = Duration::from_secs(30);
    config.push_write_timeout = PUSH_WRITE_TIMEOUT;
    // Parked far ahead for every gate that is not about the idle-write
    // horizon; the two that are set it themselves.
    config.sub_idle_write_timeout = FOREVER;
    config
}

/// Short enough that a wedged-subscriber gate finishes in seconds; the
/// production horizon is `DEFAULT_PUSH_WRITE_TIMEOUT` (10 s).
const PUSH_WRITE_TIMEOUT: Duration = Duration::from_millis(500);

/// Short enough that the R2b gates finish in seconds. The production horizon is
/// `DEFAULT_SUB_IDLE_WRITE_TIMEOUT` (45 min), which is coupled to D3's
/// 30-minute client-side drain TTL — see that constant before re-tuning either.
const SUB_IDLE_WRITE_TIMEOUT: Duration = Duration::from_secs(2);

fn write_ws(root: &Path, files: &[(&str, &str)]) -> PathBuf {
    for (rel, content) in files {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }
    root.to_path_buf()
}

const PLAN: &str = "# Goals\n\nship by August\n\n# Notes\n\nnothing yet\n";

/// One NDJSON connection. After `sub` it is a push channel — `call` must not
/// be used again ([`framing`] pins that).
struct Conn {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
}

impl Conn {
    fn open(socket: &Path) -> Self {
        let stream = UnixStream::connect(socket).unwrap();
        stream.set_read_timeout(Some(PUSH_WAIT)).unwrap();
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
            serde_json::from_str(&response).unwrap_or_else(|e| panic!("frame {response:?}: {e}"))
        })
    }

    fn hello(&mut self, ws: &Path) -> Value {
        self.call(&json!({
            "op": "hello", "proto": 1, "workspace": ws.to_str().unwrap(),
        }))
    }

    /// Live subscribe — no cursor, anchors at the acked tip (B-01).
    /// Returns the ack; connection is push-only if ok.
    fn sub(&mut self) -> Value {
        self.call(&json!({"op": "sub"}))
    }

    /// Resumption subscribe with a full `{tree_instance, from_seq}` cursor
    /// (B-01). Returns the ack; connection is push-only if ok.
    fn sub_at(&mut self, tree_instance: &str, from_seq: u64) -> Value {
        self.call(&json!({
            "op": "sub", "tree_instance": tree_instance, "from_seq": from_seq,
        }))
    }

    /// Next Notification, or `None` within [`PUSH_WAIT`].
    fn next_frame(&mut self) -> Option<Value> {
        let mut line = String::new();
        match self.reader.read_line(&mut line) {
            // Frame arrived. EOF or timeout ⇒ no frame (legitimate for some gates).
            Ok(n) if n > 0 => Some(serde_json::from_str(&line).expect("notification is JSON")),
            _ => None,
        }
    }

    /// Collect frames until `want` arrive or the wait expires.
    fn frames(&mut self, want: usize) -> Vec<Value> {
        let deadline = Instant::now() + PUSH_WAIT;
        let mut out = Vec::new();
        while out.len() < want && Instant::now() < deadline {
            match self.next_frame() {
                Some(frame) => out.push(frame),
                None => break,
            }
        }
        out
    }
}

/// Edit outside the engine — the external door the detector exists for.
fn external_edit(ws: &Path, rel: &str, content: &str) {
    fs::write(ws.join(rel), content).unwrap();
}

/// E2E: subscribe, external write, receive notification.
#[test]
fn subscribe_then_write_delivers_a_notification() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp.path().join("ws"), &[("plan.md", PLAN)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();

    let mut sub = Conn::open(server.socket_path());
    assert_eq!(sub.hello(&ws)["ok"], json!(true));
    let ack = sub.sub();
    assert_eq!(ack["ok"], json!(true), "sub is served: {ack}");
    assert!(ack["body"]["root"].is_string(), "ack carries a root: {ack}");
    assert_eq!(ack["body"]["seq"], json!(0), "fresh epoch tip: {ack}");
    assert!(
        ack["body"]["tree_instance"].is_string(),
        "the ack teaches the cursor identity — B-01: {ack}"
    );

    external_edit(
        &ws,
        "plan.md",
        "# Goals\n\nship by September\n\n# Notes\n\nnothing yet\n",
    );

    let frame = sub.next_frame().expect("a notification arrives");
    assert!(
        frame.get("id").is_none(),
        "a Notification carries no `id` — §3.1 classification: {frame}"
    );
    assert_eq!(frame["delta"]["seq"], json!(1), "first frame of the epoch");
    assert_eq!(
        frame["delta"]["files"][0]["path"],
        json!("plan.md"),
        "the changed file is named: {frame}"
    );
    assert!(
        frame["delta"].get("actor").is_none(),
        "an external edit has no actor to name — §7.1: {frame}"
    );
    server.shutdown();
}

/// §52 per-file degradation on the watch plane (node-rev-merkle-spec §3 line
/// 52): a poison ADD mid-subscription degrades THAT FILE only. The delta
/// still arrives — the poison member's entry carries no revs and no node
/// entries — and the subscription stays live: the next sibling edit is served
/// in full, chained onto the poison frame.
///
/// The base defect (defect-ledger RES-A): `classify_to_wire`'s `doc_of(...)?`
/// escalated one poison member to a refused reconcile; the baseline never
/// rebased, so the detector error-looped every cycle and the subscriber
/// received NOTHING until the file was removed — the corpus-plane incident's
/// watch-leg twin (`corpus_refusal.rs`).
///
/// *Mutation:* restore `doc_of(...)?` in `classify_to_wire` — no frame ever
/// arrives and the first expect fails.
#[test]
fn a_poison_add_mid_subscription_degrades_that_file_only() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp.path().join("ws"), &[("plan.md", PLAN)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();

    let mut sub = Conn::open(server.socket_path());
    assert_eq!(sub.hello(&ws)["ok"], json!(true));
    assert_eq!(sub.sub()["ok"], json!(true));

    // The poison lands behind the engine's back, mid-subscription (the
    // incident's shape: a non-UTF-8 fixture copied into a live corpus).
    fs::write(ws.join("poison.md"), b"# P\n\n\xff\xfe raw bytes\n").unwrap();

    let frame = sub
        .next_frame()
        .expect("the delta still arrives — one poison member refuses nothing");
    let files = frame["delta"]["files"]
        .as_array()
        .expect("the delta carries file entries");
    assert_eq!(files.len(), 1, "one change, one entry: {frame}");
    let entry = &files[0];
    assert_eq!(entry["path"], json!("poison.md"), "{frame}");
    assert_eq!(entry["change"], json!("created"), "{frame}");
    assert_eq!(
        entry["nodes"],
        json!([]),
        "an unserved member serves no spans/nodes — §52: {frame}"
    );
    assert!(
        entry.get("file_rev_after").is_none(),
        "an unserved member mints no rev: {frame}"
    );

    // Siblings still served: the next healthy edit arrives in full, chained
    // onto the poison frame — the baseline advanced through it.
    external_edit(
        &ws,
        "plan.md",
        "# Goals\n\nship by September\n\n# Notes\n\nnothing yet\n",
    );
    let next = sub.next_frame().expect("the subscription is still live");
    assert_eq!(
        next["delta"]["root_before"], frame["delta"]["root_after"],
        "the chain advanced THROUGH the poison frame: {next}"
    );
    let entry = &next["delta"]["files"][0];
    assert_eq!(entry["path"], json!("plan.md"), "{next}");
    assert!(
        entry["file_rev_after"].is_string()
            && entry["nodes"].as_array().is_some_and(|n| !n.is_empty()),
        "the healthy sibling keeps full node grain: {next}"
    );
    server.shutdown();
}

/// Framing: a live subscription must not disturb another connection.
///
/// *Mutation:* push notifications from `handle_line` instead of converting in
/// `serve_conn` — the `toc` call then reads a delta where its response should be.
#[test]
fn framing_a_subscription_never_desyncs_another_connection() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp.path().join("ws"), &[("plan.md", PLAN)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();

    let mut sub = Conn::open(server.socket_path());
    sub.hello(&ws);
    assert_eq!(sub.sub()["ok"], json!(true));

    // Ring must hold frames before the request connection speaks, or an
    // interleaving defect has nothing to inject.
    external_edit(&ws, "plan.md", "# Goals\n\nrevision 0\n");
    assert!(
        !sub.frames(1).is_empty(),
        "control: the ring holds a frame, so a leak into the request plane is possible"
    );

    // Ordinary client on the same workspace while subscription is live.
    let mut plain = Conn::open(server.socket_path());
    assert_eq!(plain.hello(&ws)["ok"], json!(true));
    for n in 1..4 {
        external_edit(&ws, "plan.md", &format!("# Goals\n\nrevision {n}\n"));
        // Let detection land a frame between requests.
        std::thread::sleep(Duration::from_millis(350));
        let toc = plain.call(&json!({"op": "toc", "path": "plan.md"}));
        assert_eq!(toc["ok"], json!(true), "request connection is clean: {toc}");
        assert!(
            toc["body"]["nodes"].is_array(),
            "and it is the TOC response, not a stray delta frame: {toc}"
        );
    }
    server.shutdown();
}

/// S6 keying: two spellings of one workspace share one ring; distinct workspaces
/// never share frames. Second half is the vacuity control.
///
/// *Mutation:* key `Registry::rings` on non-canonical path — two-spellings
/// subscriber gets an empty ring.
#[test]
fn s6_one_ring_per_workspace_not_per_spelling() {
    let tmp = TempDir::new().unwrap();
    let ws_a = write_ws(&tmp.path().join("a"), &[("plan.md", PLAN)]);
    let ws_b = write_ws(&tmp.path().join("b"), &[("plan.md", PLAN)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();

    // Same workspace two ways: canonical path and walk-out-and-back. Hello bind
    // must canonicalize onto one ring.
    let spelled = ws_a.join("..").join("a");
    let mut canonical = Conn::open(server.socket_path());
    let mut roundabout = Conn::open(server.socket_path());
    let mut other = Conn::open(server.socket_path());
    assert_eq!(canonical.hello(&ws_a)["ok"], json!(true));
    assert_eq!(roundabout.hello(&spelled)["ok"], json!(true));
    assert_eq!(other.hello(&ws_b)["ok"], json!(true));
    assert_eq!(canonical.sub()["ok"], json!(true));
    assert_eq!(roundabout.sub()["ok"], json!(true));
    assert_eq!(other.sub()["ok"], json!(true));

    external_edit(&ws_a, "plan.md", "# Goals\n\nonly workspace A moved\n");

    let via_canonical = canonical
        .next_frame()
        .expect("canonical spelling is served");
    let via_roundabout = roundabout
        .next_frame()
        .expect("the roundabout spelling reaches the SAME ring");
    assert_eq!(
        via_canonical["delta"]["root_after"], via_roundabout["delta"]["root_after"],
        "one workspace, one ring, one frame — not two independent epochs"
    );

    // Vacuity: untouched workspace B must receive nothing.
    assert!(
        other.next_frame().is_none(),
        "a workspace that did not change leaks no frame from one that did"
    );
    server.shutdown();
}

/// S2: `sub` cannot observe what a read on the same connection could not.
///
/// *Mutation:* route `Op::Sub` before the bound-workspace guard — unbound `sub`
/// then succeeds.
#[test]
fn s2_sub_stands_behind_the_workspace_bind() {
    let tmp = TempDir::new().unwrap();
    write_ws(&tmp.path().join("ws"), &[("plan.md", PLAN)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();

    let mut bare = Conn::open(server.socket_path());
    let refused = bare.sub();
    assert_eq!(refused["ok"], json!(false), "unbound sub is refused");
    assert_eq!(refused["error"]["code"], json!("bad_request"));

    // Same refusal a composed read gets — one gate for both ops.
    let read_refused = bare.call(&json!({"op": "toc", "path": "plan.md"}));
    assert_eq!(read_refused["ok"], json!(false));
    assert_eq!(
        read_refused["error"]["code"], refused["error"]["code"],
        "sub and read refuse an unbound connection identically"
    );
    server.shutdown();
}

/// S2 anchor: a live-instance `from_seq` outside retained history refuses
/// `root_unknown`.
///
/// *Mutation:* drop `can_anchor` in the `sub` arm — impossible-future sub accepted.
#[test]
fn an_unanchorable_from_seq_refuses_root_unknown() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp.path().join("ws"), &[("plan.md", PLAN)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();

    // Learn the live instance on a throwaway live sub (an accepted sub
    // converts its connection, so the probe gets its own).
    let mut probe = Conn::open(server.socket_path());
    probe.hello(&ws);
    let instance = probe.sub()["body"]["tree_instance"]
        .as_str()
        .expect("the ack teaches tree_instance")
        .to_string();

    let mut conn = Conn::open(server.socket_path());
    conn.hello(&ws);
    let refused = conn.sub_at(&instance, 9999);
    assert_eq!(refused["ok"], json!(false), "ahead of the tip: {refused}");
    assert_eq!(refused["error"]["code"], json!("root_unknown"));
    server.shutdown();
}

/// THE B-01 daemon-door gate: a dead-instance cursor refuses on instance,
/// sequence never consulted — the cursor's seq is the acked tip, the one
/// number that anchors on the live instance (the control proves it).
///
/// *Mutation:* compare seq before instance in `can_anchor` — the dead cursor
/// anchors and the gate fails.
#[test]
fn a_dead_instance_cursor_refuses_root_unknown() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp.path().join("ws"), &[("plan.md", PLAN)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();

    let mut probe = Conn::open(server.socket_path());
    probe.hello(&ws);
    let ack = probe.sub();
    let live = ack["body"]["tree_instance"]
        .as_str()
        .expect("the ack teaches tree_instance")
        .to_string();
    let tip = ack["body"]["seq"].as_u64().expect("the ack teaches seq");

    let mut dead = Conn::open(server.socket_path());
    dead.hello(&ws);
    let refused = dead.sub_at("dead.0.0", tip);
    assert_eq!(refused["ok"], json!(false), "dead instance: {refused}");
    assert_eq!(refused["error"]["code"], json!("root_unknown"));
    let message = refused["error"]["message"]
        .as_str()
        .expect("the refusal teaches");
    assert!(
        message.contains("dead.0.0") && message.contains(&live),
        "the teaching names both instances: {message}"
    );

    // Control: the same seq under the live instance anchors — the refusal
    // above was the instance evaluation, not the window.
    let mut ok = Conn::open(server.socket_path());
    ok.hello(&ws);
    assert_eq!(ok.sub_at(&live, tip)["ok"], json!(true));
    server.shutdown();
}

/// B-01 upgrade-required: `from_seq` without `tree_instance` — anchoring by
/// number alone — refuses `bad_request` with the upgrade teaching; instance
/// without seq is half a cursor and refuses too. A refused sub leaves an
/// ordinary request channel.
///
/// *Mutation:* default a missing `tree_instance` to the live instance — the
/// bare-number client anchors and the gate fails.
#[test]
fn a_bare_from_seq_is_upgrade_required() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp.path().join("ws"), &[("plan.md", PLAN)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();

    let mut conn = Conn::open(server.socket_path());
    conn.hello(&ws);
    let refused = conn.call(&json!({"op": "sub", "from_seq": 0}));
    assert_eq!(refused["ok"], json!(false), "number alone: {refused}");
    assert_eq!(refused["error"]["code"], json!("bad_request"));
    let message = refused["error"]["message"]
        .as_str()
        .expect("the refusal teaches");
    assert!(
        message.contains("tree_instance") && message.contains("Upgrade"),
        "the teaching names the missing identity and the remedy: {message}"
    );

    let half = conn.call(&json!({"op": "sub", "tree_instance": "x.y.z"}));
    assert_eq!(half["ok"], json!(false), "half a cursor: {half}");
    assert_eq!(half["error"]["code"], json!("bad_request"));

    // The channel is still a request channel: a live sub is served.
    assert_eq!(conn.sub()["ok"], json!(true));
    server.shutdown();
}

/// B-01 resumption: a full `{tree_instance, from_seq}` cursor from a live
/// ack resumes with replay — the retained frame past the cursor arrives
/// byte-identical on the new connection.
#[test]
fn a_valid_cursor_resumes_with_replay() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp.path().join("ws"), &[("plan.md", PLAN)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();

    let mut first = Conn::open(server.socket_path());
    first.hello(&ws);
    let ack = first.sub();
    let cursor_instance = ack["body"]["tree_instance"]
        .as_str()
        .expect("the ack teaches tree_instance")
        .to_string();
    let cursor_seq = ack["body"]["seq"].as_u64().expect("the ack teaches seq");
    external_edit(
        &ws,
        "plan.md",
        "# Goals\n\nship by September\n\n# Notes\n\nnothing yet\n",
    );
    let seen = first.next_frame().expect("the live sub sees the frame");
    drop(first); // the client falls away holding its cursor

    let mut resumed = Conn::open(server.socket_path());
    resumed.hello(&ws);
    assert_eq!(
        resumed.sub_at(&cursor_instance, cursor_seq)["ok"],
        json!(true),
        "the held cursor anchors within its instance"
    );
    let replayed = resumed.next_frame().expect("the retained frame replays");
    assert_eq!(replayed, seen, "replay ≡ live — §7.3");
    server.shutdown();
}

/// Deltas carry identities/revs/spans, never content — and no server-side
/// read state exists anywhere (§ A.3 proof law: pin proof rides the request),
/// so a pushed frame structurally cannot open a write door. What this pins:
/// the frame serves node revs (world facts), not read authority.
#[test]
fn a_pushed_frame_carries_facts_never_read_authority() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp.path().join("ws"), &[("plan.md", PLAN)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();

    let mut sub = Conn::open(server.socket_path());
    sub.hello(&ws);
    assert_eq!(sub.sub()["ok"], json!(true));
    external_edit(
        &ws,
        "plan.md",
        "# Goals\n\nship by September\n\n# Notes\n\nnothing yet\n",
    );
    let frame = sub.next_frame().expect("a notification arrives");
    // The frame carries the section's new rev — a fact about the world, not a
    // proof of anyone's read: the § A.3 proof a pin needs is the section's
    // `fp1.…` token, which only a sections-mode READ serves.
    assert!(
        frame["delta"]["files"][0]["nodes"]
            .as_array()
            .expect("node entries")
            .iter()
            .any(|n| n["node_rev_after"].is_string()),
        "control: the frame carries node revs: {frame}"
    );
    assert!(
        frame["delta"]["files"][0]["nodes"]
            .as_array()
            .expect("node entries")
            .iter()
            .all(|n| n.get("fingerprint").is_none()),
        "a frame never serves the pin-proof token — reads do: {frame}"
    );
    server.shutdown();
}

/// Seq contiguity: consecutive frames form one chain (`seq` +1, `root_before`
/// joins prior `root_after`). Survives the `SeqSink` dual-producer change.
///
/// *Mutation:* two concurrent producers without flock serialization — duplicate
/// `seq` or broken root join.
#[test]
fn the_delta_chain_is_contiguous() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp.path().join("ws"), &[("plan.md", PLAN)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();

    let mut sub = Conn::open(server.socket_path());
    sub.hello(&ws);
    assert_eq!(sub.sub()["ok"], json!(true));

    let mut frames = Vec::new();
    for n in 1..=3 {
        external_edit(&ws, "plan.md", &format!("# Goals\n\nrevision {n}\n"));
        frames.push(
            sub.next_frame()
                .unwrap_or_else(|| panic!("frame {n} arrives")),
        );
    }

    for (i, frame) in frames.iter().enumerate() {
        let want_seq = u64::try_from(i).unwrap() + 1;
        assert_eq!(
            frame["delta"]["seq"],
            json!(want_seq),
            "seq advances by exactly one: {frame}"
        );
        if i > 0 {
            assert_eq!(
                frame["delta"]["root_before"],
                frames[i - 1]["delta"]["root_after"],
                "each frame chains onto the previous one"
            );
        }
    }
    server.shutdown();
}

/// r8 D4 at the drain grain (card pin-mint-sub-attribution): a promoting
/// pin's anchor mint reaches a subscriber as a row on the TARGET file, on the
/// actor-carrying frame, and the chain join holds — `fingerprint_before`
/// meets the previous frame's `fingerprint_after` instead of silently
/// skipping the promotion's own root movement. Five physical mints with zero
/// rows across drains 616→683 is the defect this gate must never re-admit.
#[test]
fn a_promoting_pin_pushes_the_targets_row_on_the_actors_frame() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(
        &tmp.path().join("ws"),
        &[
            ("plan.md", PLAN),
            (
                "guide.md",
                "# Guide\n\n## Steps\n\nreview before you close.\n",
            ),
        ],
    );
    // R4: a pin row folds the target's git blob oid — the workspace is a repo.
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "sub@example.invalid"],
        vec!["config", "user.name", "sub"],
    ] {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(&ws)
            .args(&args)
            .status()
            .expect("git runs in the test environment");
        assert!(status.success(), "git {args:?}");
    }
    let server = RunningServer::start(test_config(&tmp)).unwrap();

    // v3 session: `read` and `splice.pin` are v3-only at decode.
    let mut ops = Conn::open(server.socket_path());
    let hello = ops.call(&json!({
        "op": "hello", "proto": 1, "contract": "v3",
        "workspace": ws.to_str().unwrap(),
    }));
    assert_eq!(hello["ok"], json!(true), "{hello}");
    // § A.3 proof law: the covering read serves the token the pin carries.
    let read = ops.call(&json!({
        "op": "read", "path": "guide.md",
        "sections": [{"hpath": [{"h": "Guide"}, {"h": "Steps"}]}],
    }));
    assert_eq!(read["ok"], json!(true), "{read}");
    let proof = read["body"]["sections"][0]["fingerprint"]
        .as_str()
        .expect("a served section carries its proof token")
        .to_string();

    let mut sub = Conn::open(server.socket_path());
    let sub_hello = sub.call(&json!({
        "op": "hello", "proto": 1, "contract": "v3",
        "workspace": ws.to_str().unwrap(),
    }));
    assert_eq!(sub_hello["ok"], json!(true), "{sub_hello}");
    assert_eq!(sub.sub()["ok"], json!(true));

    // Frame 1: an external edit — the detector's actor-absent baseline.
    external_edit(
        &ws,
        "plan.md",
        "# Goals\n\nship by September\n\n# Notes\n\nnothing yet\n",
    );
    let external = sub.next_frame().expect("the external edit is pushed");

    // Frame 2: the promoting pin, carrying the read's own token.
    let splice = ops.call(&json!({
        "id": 7, "op": "splice", "path": "plan.md",
        "actor": "agent:pinner", "now": "2026-08-15T12:00:00Z",
        "pin": {
            "target": "guide.md",
            "selector": {"hpath": [{"h": "Guide"}, {"h": "Steps"}]},
            "fingerprint": proof,
        },
    }));
    assert_eq!(splice["ok"], json!(true), "the pin commits: {splice}");
    assert_eq!(
        splice["body"]["pin"]["promoted"],
        json!(true),
        "control: this pin really minted an anchor: {splice}"
    );
    let frame = sub.next_frame().expect("the pin's commit is pushed");

    let files = frame["delta"]["files"].as_array().expect("files: {frame}");
    let paths: Vec<&str> = files.iter().filter_map(|f| f["path"].as_str()).collect();
    assert_eq!(
        paths,
        vec!["plan.md", "guide.md"],
        "the pinning page and the minted target both ride: {frame}"
    );
    assert_eq!(
        frame["delta"]["actor"],
        json!("agent:pinner"),
        "the mint is actor-attributed: {frame}"
    );
    assert!(
        files[1]["nodes"].as_array().is_some_and(|n| !n.is_empty()),
        "the target row carries node grain: {frame}"
    );
    assert_eq!(
        frame["delta"]["fingerprint_before"], external["delta"]["fingerprint_after"],
        "the chain joins across the promoting pin — no silently-skipped root \
         movement: {frame}"
    );
    assert_eq!(
        frame["delta"]["seq"].as_u64(),
        external["delta"]["seq"].as_u64().map(|s| s + 1),
        "consecutive seq: {frame}"
    );
    server.shutdown();
}

/// Reap: push-only subscriber makes no `last_use` touch; workspace must not be
/// reaped. Real damage is a fork: map entry gone ⇒ next `sub` mints a second
/// ring and per-workspace `seq` (§4.7) splits. Pin is on ring identity.
///
/// *Mutation:* drop `subscribed_workspaces` exemption from `Registry::reap`.
#[allow(clippy::duration_suboptimal_units)]
#[test]
fn a_live_subscription_survives_the_reaper() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp.path().join("ws"), &[("plan.md", PLAN)]);
    let mut config = test_config(&tmp);
    config.idle_threshold = Duration::from_secs(0);
    let server = RunningServer::start(config).unwrap();

    let mut sub = Conn::open(server.socket_path());
    sub.hello(&ws);
    assert_eq!(sub.sub()["ok"], json!(true));

    let canonical = workspace::canonicalize(&ws).unwrap();
    let before = server.registry().ring(&canonical);

    // Everything idle at this horizon; only the subscription protects it.
    let reaped = server.registry().reap(u64::MAX, 0);
    assert!(
        !reaped.contains(&canonical),
        "a subscribed workspace is not reaped: {reaped:?}"
    );
    let after = server.registry().ring(&canonical);
    assert!(
        std::sync::Arc::ptr_eq(&before, &after),
        "one workspace keeps ONE ring across a reap — a second ring would fork \
         the per-workspace seq counter §4.7 defines"
    );

    // Stream still live end-to-end.
    external_edit(&ws, "plan.md", "# Goals\n\nafter the reaper ran\n");
    let frame = sub
        .next_frame()
        .expect("a live subscription still receives after the reaper has run");
    assert_eq!(frame["delta"]["files"][0]["path"], json!("plan.md"));
    server.shutdown();
}

/// S1: a subscriber that stops draining must not park a server thread forever.
/// The daemon is thread-per-connection, so a wedged consumer costs an OS thread
/// and (since an armed sub holds the quiet clock open) the daemon's mortality.
/// Past the write deadline the connection is dropped and `SubGuard` freed.
///
/// *Mutation:* drop `set_write_timeout` from `push_loop` — the subscription
/// stays live forever and the assertion times out.
#[test]
fn a_wedged_subscriber_is_dropped_and_frees_its_subguard() {
    let tmp = TempDir::new().unwrap();
    // A blocked write is only reachable once the socket buffers are full, so
    // frames must be fat: one entry per file, corpus-wide rewrites.
    let bodies: Vec<(String, String)> = (0..300)
        .map(|i| {
            (
                format!("doc{i:03}.md"),
                format!("# Doc {i}\n\nbody\n\n## Sub A\n\na\n\n## Sub B\n\nb\n"),
            )
        })
        .collect();
    let files: Vec<(&str, &str)> = bodies
        .iter()
        .map(|(name, body)| (name.as_str(), body.as_str()))
        .collect();
    let ws = write_ws(&tmp.path().join("ws"), &files);
    let server = RunningServer::start(test_config(&tmp)).unwrap();

    // Wedged by construction: this connection is never read from again.
    let mut sub = Conn::open(server.socket_path());
    assert_eq!(sub.hello(&ws)["ok"], json!(true));
    assert_eq!(sub.sub()["ok"], json!(true));

    let canonical = workspace::canonicalize(&ws).unwrap();
    let ring = server.registry().ring(&canonical);
    assert!(
        ring.has_subscribers(),
        "control: the subscription is armed, so there is something to free"
    );

    // Keep producing until the kernel stops absorbing and the write blocks.
    for round in 0..8 {
        for (name, _) in &bodies {
            external_edit(&ws, name, &format!("# {name}\n\nrevision {round}\n"));
        }
        std::thread::sleep(Duration::from_millis(300));
    }

    let deadline = Instant::now() + PUSH_WAIT;
    while ring.has_subscribers() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        !ring.has_subscribers(),
        "a subscriber that stopped draining must be dropped past the {PUSH_WRITE_TIMEOUT:?} \
         write deadline, freeing its SubGuard"
    );
    // The client end is still open — the server dropped it, not the test.
    drop(sub);
    server.shutdown();
}

/// S3: an armed sub connection counts toward the quiet clock — a subscribed
/// daemon must not idle-exit out from under its subscriber.
///
/// *Mutation:* drop the `has_subscribers` arm from the reaper's idle-exit check
/// — the daemon asks to exit while the subscription is live.
#[allow(clippy::duration_suboptimal_units)]
#[test]
fn an_armed_sub_defers_idle_exit_and_releasing_it_restores_mortality() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp.path().join("ws"), &[("plan.md", PLAN)]);
    let mut config = test_config(&tmp);
    config.idle_exit = Some(Duration::from_secs(2));
    config.reap_interval = Duration::from_millis(100);
    // The idle-exit clock is unix whole seconds from birth. A 2s horizon can
    // latch in 1–2s of wall time, during start→sub, before the subscriber is
    // visible — and the reaper then returns, so a later subscribe cannot
    // retract (CI 677 on 46caf36b3). Park past the first reap tick
    // (REAP_TICK = 200ms) so the hold measures the armed sub, not handshake
    // scheduling.
    //
    // The park is a FLOOR (card `registry-sweep-rebuild-flake-same-sha-split`).
    // It used to STORE into the activity clock, which the `hello`/`sub` below
    // then overwrote through `note_request` — the park was destroyed by the
    // handshake it was covering, leaving the ordinary 2s horizon to run
    // against the gap before `has_subscribers` turns true. Green on an idle
    // box, red on a loaded one, at one identical tree.
    //
    // And it is BORN parked, not parked on the handle `start()` returns: the
    // clock starts inside `Registry::new` and the reaper is spawned before
    // `start()` returns, so the after-the-fact form left `start()`'s own body
    // exposed — ~1.0 s of wall time is enough to latch a 2 s horizon (card
    // `registry-sweep-poll-flake-instance-1` § F1 full-close).
    config.activity_park = Some(FOREVER);
    let server = RunningServer::start(config).unwrap();

    let mut sub = Conn::open(server.socket_path());
    assert_eq!(sub.hello(&ws)["ok"], json!(true));
    assert_eq!(sub.sub()["ok"], json!(true));

    let canonical = workspace::canonicalize(&ws).unwrap();
    let ring = server.registry().ring(&canonical);
    let armed_deadline = Instant::now() + PUSH_WAIT;
    while !ring.has_subscribers() && Instant::now() < armed_deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        ring.has_subscribers(),
        "control: the subscription is armed before the hold clock starts"
    );
    assert!(
        !server.idle_exit_requested(),
        "parking the birth clock must keep idle-exit from latching during handshake"
    );
    // Horizon starts now. No further request traffic: only the armed sub holds.
    // Release first — the park is a floor, so `note_liveness` alone could not
    // bring the clock back down out of the future.
    server.registry().release_activity_park();
    server.registry().note_liveness();

    let hold = Instant::now() + Duration::from_secs(4);
    while Instant::now() < hold {
        assert!(
            !server.idle_exit_requested(),
            "a daemon with a live subscriber must not exit under it — the subscriber \
             sends no requests, so the request clock alone would kill it"
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    // The counterweight: the subscriber goes away as a dead owner process does
    // (the kernel closes the socket), and the push plane observes that EOF even
    // on a quiet workspace where no frame is ever written.
    drop(sub);
    let deadline = Instant::now() + PUSH_WAIT;
    while ring.has_subscribers() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        !ring.has_subscribers(),
        "a closed push peer is observed on a quiet workspace — no write is ever \
         attempted there, so EOF is the only signal"
    );

    let deadline = Instant::now() + PUSH_WAIT;
    while !server.idle_exit_requested() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        server.idle_exit_requested(),
        "and once the last subscription is gone the daemon ages out again — the \
         liveness rule defers idle-exit, it must not disable it (g11: 281 \
         immortal daemons, 9.6 GB)"
    );
    server.shutdown();
}

/// R2b, mode (c): an armed sub on a permanently quiet workspace is dropped after
/// the idle-write horizon, and the freed `SubGuard` restores the daemon's
/// mortality. This is the one residency mode neither earlier deadline can reach:
/// no frame is ever written (so the write deadline never fires) and the peer
/// never closes (so the EOF probe never fires).
///
/// *Mutation:* drop the `last_write.elapsed()` arm from `push_loop` — the
/// subscription stays armed forever and the daemon never asks to exit.
#[allow(clippy::duration_suboptimal_units)]
#[test]
fn an_armed_sub_with_zero_frames_written_is_dropped_after_the_idle_horizon() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp.path().join("ws"), &[("plan.md", PLAN)]);
    let mut config = test_config(&tmp);
    let idle_exit = Duration::from_secs(1);
    config.sub_idle_write_timeout = SUB_IDLE_WRITE_TIMEOUT;
    config.idle_exit = Some(idle_exit);
    config.reap_interval = Duration::from_millis(100);
    let server = RunningServer::start(config).unwrap();
    // The idle-exit clock is unix whole seconds from DAEMON BIRTH, so a 1s
    // horizon could latch on the reaper's first tick, before this test's
    // subscriber exists. That is not a red: the terminal assertion below WANTS
    // the exit, so a premature latch makes it pass earlier and for the wrong
    // reason — "freeing the SubGuard restores mortality" goes untested whenever
    // mortality had already latched before the SubGuard existed (card
    // `sub-push-913-latent-false-green`). The park is a FLOOR under the
    // activity clock, so the handshake's own `note_request` bumps cannot
    // destroy it (card `registry-sweep-rebuild-flake-same-sha-split`).
    server.registry().park_activity_clock(365 * 24 * 60 * 60);

    // Age the daemon PAST the horizon before the subscriber exists. Under the
    // park this is inert; without it the flag latches here every time — which
    // is what makes the guard assertion below a measurement rather than a
    // load-dependent one that a quiet box always passes.
    std::thread::sleep(idle_exit + Duration::from_millis(500));

    // Armed and never written to: the workspace is quiet for the whole test, and
    // this connection is never closed by the client side.
    let mut sub = Conn::open(server.socket_path());
    assert_eq!(sub.hello(&ws)["ok"], json!(true));
    assert_eq!(sub.sub()["ok"], json!(true));

    let canonical = workspace::canonicalize(&ws).unwrap();
    let ring = server.registry().ring(&canonical);
    assert!(
        ring.has_subscribers(),
        "control: the subscription is armed, so there is something to drop"
    );
    assert!(
        !server.idle_exit_requested(),
        "control: mortality has NOT latched before the subscriber exists — \
         otherwise the terminal assertion passes on the birth clock and never \
         tests the SubGuard at all"
    );

    // Horizon starts HERE, with the sub armed. Release first — the park is a
    // floor, so `note_liveness` alone could not bring the clock back down out
    // of the future. From this instant the reaper defers on `has_subscribers`,
    // so the flag can only be raised after the sub is dropped.
    server.registry().release_activity_park();
    server.registry().note_liveness();

    let deadline = Instant::now() + PUSH_WAIT;
    while ring.has_subscribers() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        !ring.has_subscribers(),
        "an armed sub with zero frames written must be dropped past the \
         {SUB_IDLE_WRITE_TIMEOUT:?} idle-write horizon — otherwise a wedged \
         subscriber on a quiet workspace pins the daemon forever"
    );

    // And the point of the drop: the daemon can age out again.
    let deadline = Instant::now() + PUSH_WAIT;
    while !server.idle_exit_requested() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        server.idle_exit_requested(),
        "freeing the SubGuard restores mortality — dropping the sub is not the \
         cost, it is the point (g11 lifecycle)"
    );

    // The client end was open the whole time: the server dropped it, not the test.
    drop(sub);
    server.shutdown();
}

/// R2b counterweight: any frame written resets the horizon, so a subscriber on a
/// workspace that keeps producing survives well past it. Without the reset the
/// backstop would be an absolute lifetime — a tax on every healthy subscriber
/// rather than a bound on mode (c)'s exact signature.
///
/// *Mutation:* drop the `last_write = Instant::now()` reset from the frame loop
/// — this subscriber dies mid-stream despite being drained continuously.
#[test]
fn a_frame_written_inside_the_horizon_resets_it() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp.path().join("ws"), &[("plan.md", PLAN)]);
    let mut config = test_config(&tmp);
    config.sub_idle_write_timeout = SUB_IDLE_WRITE_TIMEOUT;
    let server = RunningServer::start(config).unwrap();

    let mut sub = Conn::open(server.socket_path());
    assert_eq!(sub.hello(&ws)["ok"], json!(true));
    assert_eq!(sub.sub()["ok"], json!(true));

    let canonical = workspace::canonicalize(&ws).unwrap();
    let ring = server.registry().ring(&canonical);

    // Edits spaced well inside the horizon, spanning several horizons in total.
    // Each delivered frame must push the deadline out.
    let started = Instant::now();
    let mut delivered = 0;
    for round in 0..10 {
        external_edit(&ws, "plan.md", &format!("# Goals\n\nrevision {round}\n"));
        delivered += sub.frames(1).len();
        std::thread::sleep(Duration::from_millis(500));
    }
    let spanned = started.elapsed();
    assert!(
        spanned > SUB_IDLE_WRITE_TIMEOUT * 2,
        "control: the run must span more than one horizon or it proves nothing \
         ({spanned:?} vs {SUB_IDLE_WRITE_TIMEOUT:?})"
    );
    assert!(
        delivered >= 4,
        "control: frames really were written, so the reset had something to \
         reset ({delivered} delivered)"
    );
    assert!(
        ring.has_subscribers(),
        "a subscriber that is being written to must survive the idle-write \
         horizon — the timer bounds silence, not lifetime"
    );
    drop(sub);
    server.shutdown();
}

/// Control for the reap pin: exemption is for subscriptions, not "never reap".
#[allow(clippy::duration_suboptimal_units)]
#[test]
fn an_unsubscribed_workspace_is_still_reaped() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp.path().join("ws"), &[("plan.md", PLAN)]);
    let mut config = test_config(&tmp);
    config.idle_threshold = Duration::from_secs(0);
    config.reap_interval = Duration::from_secs(365 * 24 * 60 * 60);
    let server = RunningServer::start(config).unwrap();

    let mut conn = Conn::open(server.socket_path());
    assert_eq!(conn.hello(&ws)["ok"], json!(true));
    // Hello binds at config cost (§3.2); the read warms — giving the reaper
    // a resident engine to reap.
    assert_eq!(
        conn.call(&json!({"op": "toc", "path": "plan.md"}))["ok"],
        json!(true)
    );
    drop(conn);

    let reaped = server.registry().reap(u64::MAX, 0);
    assert!(
        reaped.iter().any(|p| p.ends_with("ws")),
        "an idle workspace with no subscribers IS reaped: {reaped:?}"
    );
    server.shutdown();
}
