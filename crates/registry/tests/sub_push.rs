//! U20b gates — serve-side `Op::Sub` in the resident daemon: the push channel,
//! the per-workspace ring, the S2/S6 boundaries, and the chain's contiguity.
//!
//! Every gate drives the REAL daemon over its real socket. The push path only
//! exists as a property of the connection (a subscribed connection stops being a
//! request channel), so an in-process re-call of the arm would test a shape the
//! production wiring does not have.
//!
//! **Each pin here was watched to FAIL before it was trusted** (the advisor's
//! condition on this unit). The mutation that reddens each one is named in its
//! own doc comment — a pin whose failure mode is not written down is a pin
//! nobody can re-prove.
//!
//! # Where the positional selects get their soundness — it is NOT in this file
//! The gates below index pushed frames positionally (`frame["delta"]["files"][0]`,
//! the nth frame of an epoch). That is sound only because a subscribed
//! connection cannot desync: **the guarantee lives entirely at the PRODUCER —
//! `crates/registry/src/server.rs`, where `serve_conn` returns out of the
//! request read loop into `push_loop` on an accepted `Sub`, so one connection
//! is either a request channel or a push channel and never both.** A reader of
//! this file alone cannot see that; the pins here would look like they trust
//! frame order for no stated reason. **Carry the crate path, never the
//! basename** — `crates/sidecar/tests/sub_push.rs` is a different file and has
//! twice been read as this one. DECISION 25 (ZT, 2026-08-04), riding the
//! DECISION 22 cleanup: comment only, no code change.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use registry::{Config, RunningServer};
use serde_json::{Value, json};
use tempfile::TempDir;

/// How long a gate waits for a pushed frame before calling it absent. Generous
/// against the 250ms detect cadence + 50ms push tick, because a false "no frame
/// arrived" would be an infuriating flake in exactly the tests that prove
/// frames arrive.
const PUSH_WAIT: Duration = Duration::from_secs(10);

/// A daemon whose reaper and pre-warm are parked far in the future — the
/// default for every gate that is not ABOUT the reaper.
#[allow(clippy::duration_suboptimal_units)]
fn test_config(tmp: &TempDir) -> Config {
    let dir = tmp.path().join("registry");
    Config {
        socket_path: dir.join("daemon.sock"),
        state_path: dir.join("state.json"),
        cache_root: tmp.path().join("cache"),
        idle_threshold: Duration::from_secs(365 * 24 * 60 * 60),
        reap_interval: Duration::from_secs(365 * 24 * 60 * 60),
        prewarm_interval: Duration::from_secs(365 * 24 * 60 * 60),
        prewarm_quiet_max: Duration::from_secs(365 * 24 * 60 * 60),
        // No idle exit: this server's lifetime is the test's, and a daemon that
        // reaped itself mid-assertion would fail as a flake, not a finding.
        idle_exit: None,
    }
}

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

/// One NDJSON connection. After `sub` it is a push channel and `call` must never
/// be used on it again — which is the property [`framing`] pins.
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
        let mut line = serde_json::to_string(request).unwrap();
        line.push('\n');
        self.writer.write_all(line.as_bytes()).unwrap();
        self.writer.flush().unwrap();
        let mut response = String::new();
        self.reader.read_line(&mut response).unwrap();
        serde_json::from_str(&response).unwrap_or_else(|e| panic!("frame {response:?}: {e}"))
    }

    fn hello(&mut self, ws: &Path) -> Value {
        self.call(&json!({
            "op": "hello", "proto": 1, "workspace": ws.to_str().unwrap(),
        }))
    }

    /// Subscribe from `from_seq`. Returns the ack frame; the connection is a
    /// push channel afterwards if the ack is ok.
    fn sub(&mut self, from_seq: u64) -> Value {
        self.call(&json!({"op": "sub", "from_seq": from_seq}))
    }

    /// The next Notification frame, or `None` if none arrived within
    /// [`PUSH_WAIT`].
    fn next_frame(&mut self) -> Option<Value> {
        let mut line = String::new();
        match self.reader.read_line(&mut line) {
            // A frame. Anything else — clean EOF (`Ok(0)`) or the read timeout
            // expiring — means no frame arrived, which is a legitimate answer
            // several gates assert on.
            Ok(n) if n > 0 => Some(serde_json::from_str(&line).expect("notification is JSON")),
            _ => None,
        }
    }

    /// Collect frames until `want` have arrived or the wait expires.
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

/// Edit `rel` externally — the door the daemon does not own. This is the
/// external-process door the whole detector exists for.
fn external_edit(ws: &Path, rel: &str, content: &str) {
    fs::write(ws.join(rel), content).unwrap();
}

// ---------------------------------------------------------------------------
// The end-to-end proof (row F3: prove the path by RUNNING it, not by a unit test)
// ---------------------------------------------------------------------------

/// **E2E** — subscribe, write into the workspace, receive the notification.
/// The whole point of the unit in one test.
#[test]
fn subscribe_then_write_delivers_a_notification() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp.path().join("ws"), &[("plan.md", PLAN)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();

    let mut sub = Conn::open(server.socket_path());
    assert_eq!(sub.hello(&ws)["ok"], json!(true));
    let ack = sub.sub(0);
    assert_eq!(ack["ok"], json!(true), "sub is served: {ack}");
    assert!(ack["body"]["root"].is_string(), "ack carries a root: {ack}");
    assert_eq!(ack["body"]["seq"], json!(0), "fresh epoch tip: {ack}");

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

// ---------------------------------------------------------------------------
// PIN 1 — framing
// ---------------------------------------------------------------------------

/// **PIN: framing.** A live subscription must not disturb any other connection.
/// An ordinary request connection reads its own response and only its own,
/// while frames are being pushed to a subscriber on the same workspace.
///
/// *Mutation that reddens it:* push frames onto the request connection (write
/// notifications from `handle_line` instead of converting the connection in
/// `serve_conn`). The `toc` call below then reads a `{"delta":…}` frame where
/// its response should be, and the `ok` assertion fails.
#[test]
fn framing_a_subscription_never_desyncs_another_connection() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp.path().join("ws"), &[("plan.md", PLAN)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();

    let mut sub = Conn::open(server.socket_path());
    sub.hello(&ws);
    assert_eq!(sub.sub(0)["ok"], json!(true));

    // ORDER MATTERS, and this line is why. The ring must already HOLD frames
    // before the request connection speaks: an interleaving defect can only
    // inject a frame that exists. Measured — with the plain request issued
    // immediately after the edit, the mutation that pushes onto the request
    // connection did NOT redden this pin, because detection had not produced a
    // frame yet and there was nothing to interleave.
    external_edit(&ws, "plan.md", "# Goals\n\nrevision 0\n");
    assert!(
        !sub.frames(1).is_empty(),
        "control: the ring holds a frame, so a leak into the request plane is possible"
    );

    // An ordinary client on the same workspace, doing ordinary work while the
    // subscription is live and frames keep flowing.
    let mut plain = Conn::open(server.socket_path());
    assert_eq!(plain.hello(&ws)["ok"], json!(true));
    for n in 1..4 {
        external_edit(&ws, "plan.md", &format!("# Goals\n\nrevision {n}\n"));
        // Give detection time to land a frame in the ring between requests, so
        // every one of these calls is issued with frames pending.
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

// ---------------------------------------------------------------------------
// PIN 2 — S6 per-workspace keying, with its vacuity control
// ---------------------------------------------------------------------------

/// **PIN: S6 keying.** Two spellings of ONE workspace share one ring; two
/// DIFFERENT workspaces never see each other's frames.
///
/// The second half is the vacuity control: without it, a ring that delivered
/// nothing to anybody would pass the first half perfectly.
///
/// *Mutation that reddens it:* key `Registry::rings` on something other than the
/// canonical path — e.g. the raw `hello` spelling. The two-spellings subscriber
/// then gets its own empty ring and receives nothing.
#[test]
fn s6_one_ring_per_workspace_not_per_spelling() {
    let tmp = TempDir::new().unwrap();
    let ws_a = write_ws(&tmp.path().join("a"), &[("plan.md", PLAN)]);
    let ws_b = write_ws(&tmp.path().join("b"), &[("plan.md", PLAN)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();

    // Same workspace, spelled two ways: the canonical path, and a path that
    // walks out and back in. Canonicalization at the `hello` bind must collapse
    // them onto one ring.
    let spelled = ws_a.join("..").join("a");
    let mut canonical = Conn::open(server.socket_path());
    let mut roundabout = Conn::open(server.socket_path());
    let mut other = Conn::open(server.socket_path());
    assert_eq!(canonical.hello(&ws_a)["ok"], json!(true));
    assert_eq!(roundabout.hello(&spelled)["ok"], json!(true));
    assert_eq!(other.hello(&ws_b)["ok"], json!(true));
    assert_eq!(canonical.sub(0)["ok"], json!(true));
    assert_eq!(roundabout.sub(0)["ok"], json!(true));
    assert_eq!(other.sub(0)["ok"], json!(true));

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

    // VACUITY CONTROL: workspace B is untouched, so its subscriber must receive
    // NOTHING. Two rings that both delivered nothing would satisfy the equality
    // above and prove no isolation at all.
    assert!(
        other.next_frame().is_none(),
        "a workspace that did not change leaks no frame from one that did"
    );
    server.shutdown();
}

// ---------------------------------------------------------------------------
// PIN 3 — S2, the gate `sub` stands behind
// ---------------------------------------------------------------------------

/// **PIN: S2 gate.** `sub` cannot observe what a read on the same connection
/// could not: no bound workspace ⇒ refused, exactly as every other wire op is.
///
/// *Mutation that reddens it:* route `Op::Sub` before the bound-workspace guard
/// in `dispatch_read` (where `view_path` legitimately sits). The unbound `sub`
/// then succeeds and a client subscribes to a workspace it never bound.
#[test]
fn s2_sub_stands_behind_the_workspace_bind() {
    let tmp = TempDir::new().unwrap();
    write_ws(&tmp.path().join("ws"), &[("plan.md", PLAN)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();

    let mut bare = Conn::open(server.socket_path());
    let refused = bare.sub(0);
    assert_eq!(refused["ok"], json!(false), "unbound sub is refused");
    assert_eq!(refused["error"]["code"], json!("bad_request"));

    // The SAME refusal a composed read gets — the two ops are behind one gate,
    // which is the whole claim.
    let read_refused = bare.call(&json!({"op": "toc", "path": "plan.md"}));
    assert_eq!(read_refused["ok"], json!(false));
    assert_eq!(
        read_refused["error"]["code"], refused["error"]["code"],
        "sub and read refuse an unbound connection identically"
    );
    server.shutdown();
}

/// **PIN: S2 gate, anchor half.** A `from_seq` outside the retained history is
/// refused `root_unknown` rather than served a silent hole.
///
/// *Mutation that reddens it:* drop the `can_anchor` check in the `sub` arm. The
/// impossible-future subscription is then accepted and simply never delivers the
/// frames it skipped.
#[test]
fn an_unanchorable_from_seq_refuses_root_unknown() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp.path().join("ws"), &[("plan.md", PLAN)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();

    let mut conn = Conn::open(server.socket_path());
    conn.hello(&ws);
    let refused = conn.sub(9999);
    assert_eq!(refused["ok"], json!(false), "ahead of the tip: {refused}");
    assert_eq!(refused["error"]["code"], json!("root_unknown"));
    server.shutdown();
}

// ---------------------------------------------------------------------------
// PIN 4 — a pushed frame mints nothing
// ---------------------------------------------------------------------------

/// **PIN: mint isolation.** The push path must mint NOTHING into the
/// read-is-the-mint ledger.
///
/// Deltas carry identities, revs and spans — never content bytes — so a
/// subscriber has not read anything, and a receipt minted from a frame would let
/// a read-only channel open a write door: the holder could satisfy the pin
/// gate's `read_mint_required` for a section it never saw the text of.
///
/// The claim is asserted against the LEDGER directly, because that is where the
/// damage would appear. Note the stronger fact underneath: `sub` carries no
/// actor at all (advisor ruling Q1), so there is not even a name for the push
/// path to mint under. The control below proves the ledger is reachable and
/// really does record a genuine read — without it, an assertion that a ledger is
/// empty could pass because nothing ever writes to it under any circumstances.
///
/// *Mutation that reddens it:* mint a receipt per pushed frame from `push_loop`.
/// The emptiness assertion fails, naming the actor the push invented.
#[test]
fn a_pushed_frame_mints_no_read_receipt() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp.path().join("ws"), &[("plan.md", PLAN)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();

    let mut sub = Conn::open(server.socket_path());
    sub.hello(&ws);
    assert_eq!(sub.sub(0)["ok"], json!(true));
    external_edit(
        &ws,
        "plan.md",
        "# Goals\n\nship by September\n\n# Notes\n\nnothing yet\n",
    );
    let frame = sub.next_frame().expect("a notification arrives");
    // The frame really does carry the section's new rev — so what the ledger
    // lacks is a RECEIPT, not the information a receipt would have covered.
    assert!(
        frame["delta"]["files"][0]["nodes"]
            .as_array()
            .expect("node entries")
            .iter()
            .any(|n| n["node_rev_after"].is_string()),
        "control: the frame carries node revs: {frame}"
    );

    let canonical = workspace::canonicalize(&ws).unwrap();
    let mints = server.registry().read_mints(&canonical);
    for actor in ["agent:subscriber", "", "sub"] {
        assert!(
            mints
                .lookup(actor, "plan.md", &wire::ReadSel::parse("Goals"))
                .is_none(),
            "the push path minted a receipt under {actor:?} — a read-only \
             channel must not open a write door"
        );
    }

    // CONTROL: a genuine composed read by an actor DOES mint, on the same
    // ledger, for the same section. Without this the emptiness above would be
    // satisfied by a ledger nothing can ever write to.
    let mut reader = Conn::open(server.socket_path());
    reader.call(&json!({
        "op": "hello", "proto": 1, "contract": "v3",
        "workspace": ws.to_str().unwrap(),
    }));
    let read = reader.call(&json!({
        "op": "read", "path": "plan.md",
        "sections": [{"hpath": [{"h": "Goals"}]}], "actor": "agent:reader",
    }));
    assert_eq!(
        read["ok"],
        json!(true),
        "the control read is served: {read}"
    );
    assert!(
        mints
            .lookup("agent:reader", "plan.md", &wire::ReadSel::parse("Goals"))
            .is_some(),
        "control: a real read mints where the push did not"
    );
    server.shutdown();
}

// ---------------------------------------------------------------------------
// PIN 5 — chain contiguity
// ---------------------------------------------------------------------------

/// **PIN: seq contiguity.** Consecutive frames form one chain: every `seq`
/// advances by exactly one, and every `root_before` is the previous
/// `root_after`. A client that can walk the chain never needs a resync.
///
/// *Mutation that reddens it:* let two producers advance one ring (e.g. drop the
/// coalescing gate so two subscribers reconcile concurrently, or allocate `seq`
/// outside the flock). A duplicated `seq` or a broken root join fails here.
///
/// This pin is written to survive the `SeqSink` change: when the write path
/// starts allocating its own `seq` under the flock, this is the gate that proves
/// the two producers still make ONE chain.
#[test]
fn the_delta_chain_is_contiguous() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp.path().join("ws"), &[("plan.md", PLAN)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();

    let mut sub = Conn::open(server.socket_path());
    sub.hello(&ws);
    assert_eq!(sub.sub(0)["ok"], json!(true));

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

// ---------------------------------------------------------------------------
// PIN 6 — the reaper cannot evict a live subscription
// ---------------------------------------------------------------------------

/// **PIN: reap.** A push-only subscriber sends no requests, so it makes no
/// `last_use` touch and idles straight past the reap horizon. Its workspace must
/// not be reaped out from under it.
///
/// **What actually breaks, measured — not what the design first assumed.** The
/// original reasoning was that a reaped subscriber stops receiving. It does not:
/// `push_loop` holds an `Arc<WorkspaceRing>`, so a reaped ring stays alive and
/// keeps detecting for whoever already holds it. The real damage is a FORK. The
/// map entry is gone, so the next `sub` on the same workspace builds a SECOND
/// ring with its own epoch and its own counter — and `seq` is defined as a
/// monotone per-workspace batch counter (§4.7). Two rings for one workspace make
/// that definition false: two clients watching one workspace get different `seq`
/// for the same change, and the orphaned ring folds the corpus forever with
/// nothing able to reap it, because it is no longer in the map to be found.
///
/// So the pin is on ring IDENTITY, which is the fact that breaks, and it is
/// asserted deterministically rather than by waiting on a timer.
///
/// *Mutation that reddens it:* drop the `subscribed_workspaces` exemption from
/// `Registry::reap`. The post-reap `ring()` then mints a second ring and
/// `Arc::ptr_eq` fails.
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
    assert_eq!(sub.sub(0)["ok"], json!(true));

    let canonical = workspace::canonicalize(&ws).unwrap();
    let before = server.registry().ring(&canonical);

    // Everything is idle by this horizon; only the subscription protects it.
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

    // And the stream is still live end-to-end, which is what the user notices.
    external_edit(&ws, "plan.md", "# Goals\n\nafter the reaper ran\n");
    let frame = sub
        .next_frame()
        .expect("a live subscription still receives after the reaper has run");
    assert_eq!(frame["delta"]["files"][0]["path"], json!("plan.md"));
    server.shutdown();
}

/// **Control for the reap pin.** The exemption must be for SUBSCRIPTIONS, not a
/// blanket "never reap": an unsubscribed workspace on the same horizon really is
/// reaped. Without this, the pin above would pass just as well if reaping were
/// disabled outright.
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
    drop(conn);

    let reaped = server.registry().reap(u64::MAX, 0);
    assert!(
        reaped.iter().any(|p| p.ends_with("ws")),
        "an idle workspace with no subscribers IS reaped: {reaped:?}"
    );
    server.shutdown();
}
