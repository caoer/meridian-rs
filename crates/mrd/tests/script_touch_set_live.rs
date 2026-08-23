//! **The commit's authority is the §4.6 TOUCH SET** — card
//! `script-door-commit-premise-world-grain-vs-touch-set`, done predicate 1-3,
//! measured through `mrd script` against a live daemon.
//!
//! The law, as `docs/run-plane.md` 925-946 / 1073-1076 states it and as the MCP
//! `script` face text states it to customers: the nodes an attempt actually
//! touched — what it read, what a pattern expanded to, what it armed — are
//! verified entry-vs-live at exactly those nodes. **A foreign write OUTSIDE that
//! set does not refuse at all**; one INSIDE it refuses `fingerprint_mismatch`
//! naming the moved premise's scope. A caller's own `--if-fingerprint` stays
//! legal as a WIDENING premise (D-04) — strictest wins, never sufficient alone.
//!
//! What was measured in production and made this card P1: a 64-file slice
//! refused `fingerprint_mismatch` while all 64 of its targets stood byte-still,
//! because the CLI lane put a WHOLE-CORPUS fingerprint on its own splice. Every
//! corpus sweep on a busy root was a race until this landed.
//!
//! ⭐ **HOW THE MID-ATTEMPT CLAIM IS KEPT FROM PASSING VACUOUSLY.** These tests
//! must land a foreign write *between the program's reads and its commit* — a
//! window inside the daemon that no socket signal marks. A write that lands too
//! EARLY is inside the entry world (nothing moved, the test proves nothing); one
//! that lands too LATE arrives after the commit (same). So the window is
//! SEARCHED for, not assumed, and the search's success criterion is a fact only
//! a correctly-placed write can produce:
//!
//! * too early ⇒ the entry fingerprint differs from the one sampled before the
//!   write, and a pinned caller refuses PRE-EVAL — which the trace announces by
//!   carrying `guard_expected`;
//! * too late ⇒ the pinned caller's widening premise sees an unmoved world and
//!   COMMITS;
//! * inside ⇒ the entry fingerprint is the pre-write one AND the refusal has no
//!   `guard_expected`, because it happened at the commit.
//!
//! Only the third is accepted. The delay that produced it is then reused for the
//! matched control, so the pair differs in exactly one thing: the flag.

use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use mrd::script::cmd::attempt;
use mrd::script::{Door, ScriptOutcome, ScriptTrace};
use registry::{Config, RunningServer};
use serde_json::{Value, json};
use tempfile::TempDir;

/// The agent every attempt runs as.
const ME: &str = "8ab41c02";

/// The card the program reads and arms — in the touch set at both grains.
const ARMED: &str = "tasks/0011-token-audit.md";
/// A card the program READS and never writes — in the touch set by the read
/// alone, which is the half a naive "premise the writes" implementation misses.
const READ_ONLY: &str = "tasks/0012-cache-sweep.md";
/// A path no attempt here touches. A birth at a disjoint path is the §4.6
/// headline case.
const DISJOINT: &str = "notes/foreign.md";

/// The program: read both cards, arm one property on the first. Its touch set is
/// exactly `{ARMED, READ_ONLY}`.
const PROGRAM: &str = r#"
card = read("tasks/0011-token-audit.md")
other = read("tasks/0012-cache-sweep.md")
if card["fm"]["owner"] == "":
    put("tasks/0011-token-audit.md", props={"owner": me(), "status": "doing"})
"#;

/// The delay ladder the window search walks, in milliseconds. It spans three
/// orders of magnitude because the window's position is a property of the
/// machine, not of the test: a warm daemon on a fast disk commits in
/// microseconds, a loaded CI box in tens of milliseconds.
const LADDER: [u64; 9] = [1, 2, 4, 8, 16, 32, 64, 128, 256];

// ── the corpus ───────────────────────────────────────────────────────────────

fn corpus() -> Vec<(&'static str, String)> {
    vec![
        (
            ARMED,
            "---\nowner:\nstatus: todo\n---\n\n# Goals\n\nship the script entry\n".to_owned(),
        ),
        (
            READ_ONLY,
            "---\nowner: 16613c6d\nstatus: todo\n---\n\n# Goals\n\nsweep\n".to_owned(),
        ),
        (
            "tasks/0014-lease-sweep.md",
            "---\nowner: 3f9a1c07\nstatus: doing\n---\n\n# Goals\n\nsweep\n".to_owned(),
        ),
    ]
}

// ── the live daemon ──────────────────────────────────────────────────────────

/// A real `RunningServer` on a real socket, bound to a fresh corpus.
///
/// Struct fields drop in declaration order: `server` (stop → drain) MUST precede
/// `_tmp`, else the workspace vanishes under the builder — the class-2 flake
/// (pipelines 1098/1101). Locals drop the other way.
struct Fixture {
    server: RunningServer,
    ws: PathBuf,
    _tmp: TempDir,
}

impl Fixture {
    fn start() -> Self {
        let tmp = TempDir::new().expect("tempdir");
        let ws = tmp.path().join("ws");
        for (rel, content) in corpus() {
            let path = ws.join(rel);
            std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
            std::fs::write(path, content).expect("seed");
        }
        let server = RunningServer::start(config(&tmp)).expect("the daemon starts");
        Self {
            server,
            ws,
            _tmp: tmp,
        }
    }

    fn door(&self) -> LiveDoor {
        LiveDoor::open(self.server.socket_path(), &self.ws)
    }

    /// The live workspace fingerprint, through a connection of its own so the
    /// sample cannot disturb the attempt's own dialogue.
    fn fingerprint(&self) -> String {
        self.door().fingerprint()
    }
}

/// A real daemon config: the reaper never evicts a warm engine mid-test, and the
/// idle-exit clock is the test's.
#[allow(clippy::duration_suboptimal_units)]
fn config(tmp: &TempDir) -> Config {
    let forever = Duration::from_secs(365 * 24 * 60 * 60);
    let mut config = Config::for_cache_root(tmp.path().join("cache"));
    config.idle_threshold = forever;
    config.reap_interval = forever;
    config.prewarm_interval = forever;
    config.prewarm_quiet_max = forever;
    config.idle_exit = None;
    config.drain_cold_builds = Duration::from_secs(30);
    // The fixture daemon publishes THIS build's identity: the 0025 socket law
    // refuses an identity-less local hello.
    config.build_sha = Some(env!("MRD_BUILD_SHA").to_owned());
    config
}

/// The production door plus a census: the same NDJSON dialogue with every
/// request kept.
struct LiveDoor {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
    requests: Vec<Value>,
    /// A foreign write to perform after the request is flushed and before the
    /// answer is read — the interleaving these tests exist to create.
    foreign: Option<Foreign>,
}

/// What to write mid-attempt, and how long to wait first.
struct Foreign {
    page: PathBuf,
    bytes: String,
    delay: Duration,
    landed: bool,
}

impl LiveDoor {
    fn open(socket: &Path, ws: &Path) -> Self {
        let stream = UnixStream::connect(socket).expect("dial the daemon");
        let mut door = Self {
            writer: stream.try_clone().expect("clone"),
            reader: BufReader::new(stream),
            requests: Vec::new(),
            foreign: None,
        };
        let hello = door
            .call(&json!({
                "op": "hello", "proto": 1, "contract": "v3",
                "workspace": ws.to_str().expect("utf-8 workspace"),
            }))
            .expect("the handshake");
        assert_eq!(
            serde_json::from_str::<Value>(&hello).expect("a frame")["ok"],
            json!(true),
            "the daemon binds the workspace: {hello}"
        );
        door
    }

    fn landing(mut self, page: PathBuf, bytes: &str, delay: Duration) -> Self {
        self.foreign = Some(Foreign {
            page,
            bytes: bytes.to_owned(),
            delay,
            landed: false,
        });
        self
    }

    fn fingerprint(&mut self) -> String {
        let line = self
            .call(&json!({"op": "fingerprint"}))
            .expect("fingerprint");
        serde_json::from_str::<Value>(&line).expect("a frame")["body"]["fingerprint"]
            .as_str()
            .expect("a fingerprint")
            .to_owned()
    }

    fn ops(&self) -> Vec<String> {
        self.requests
            .iter()
            .filter_map(|r| r["op"].as_str().map(str::to_owned))
            .collect()
    }

    /// The one `script` request this attempt put on the socket.
    fn script_request(&self) -> &Value {
        self.requests
            .iter()
            .find(|request| request["op"] == json!("script"))
            .expect("one lane: the attempt is a `script` op")
    }
}

impl Door for LiveDoor {
    fn call(&mut self, request: &Value) -> io::Result<String> {
        self.requests.push(request.clone());
        let started = Instant::now();
        loop {
            let mut line = serde_json::to_string(request)?;
            line.push('\n');
            self.writer.write_all(line.as_bytes())?;
            self.writer.flush()?;

            // ⭐ THE INTERLEAVING. The request is on the wire and the daemon is
            // pinning, evaluating and committing. Landing the bytes HERE is what
            // puts the foreign write inside the attempt; the delay is what the
            // window search tunes.
            if request["op"] == json!("script")
                && let Some(foreign) = self.foreign.as_mut()
                && !foreign.landed
            {
                foreign.landed = true;
                std::thread::sleep(foreign.delay);
                if let Some(parent) = foreign.page.parent() {
                    std::fs::create_dir_all(parent).expect("mkdir for the foreign write");
                }
                std::fs::write(&foreign.page, &foreign.bytes).expect("a foreign write lands");
            }

            let mut response = String::new();
            self.reader.read_line(&mut response)?;
            if let Ok(frame) = serde_json::from_str::<Value>(&response)
                && frame["ok"] != json!(true)
                && frame["error"]["code"] == json!("corpus_warming")
            {
                assert!(
                    started.elapsed() < Duration::from_secs(30),
                    "corpus_warming persisted past 30s; last: {response}"
                );
                std::thread::sleep(Duration::from_millis(20));
                continue;
            }
            return Ok(response);
        }
    }
}

// ── one measured attempt ─────────────────────────────────────────────────────

/// One whole attempt with a foreign write landed mid-flight.
struct Probe {
    /// The workspace fingerprint sampled BEFORE the foreign write.
    before: String,
    trace: ScriptTrace,
    /// Whether the foreign write actually happened.
    landed: bool,
    /// The one request the attempt sent.
    request: Value,
    ops: Vec<String>,
    /// The fixture, held so the workspace outlives the attempt: an outcome word
    /// is not a write, and the disk is what says one happened.
    fixture: Fixture,
}

impl Probe {
    /// Did the entry world predate the foreign write? `true` means the daemon
    /// pinned before the bytes landed — the LOWER bound of the window.
    fn entered_before_the_write(&self) -> bool {
        self.trace.entry_fingerprint == self.before
    }

    /// Did the refusal happen at the COMMIT rather than at the pre-eval guard?
    /// `guard_expected` is present EXACTLY on a pre-eval guard refusal, so its
    /// absence on a conflict is the UPPER bound of the window: the world the
    /// daemon entered at still matched the caller's pin, and what refused was
    /// the commit seeing the moved world.
    fn refused_at_the_commit(&self) -> bool {
        self.trace.outcome == ScriptOutcome::Conflict && self.trace.guard_expected.is_none()
    }

    /// The daemon's own `root_before` — a LIVE observation taken inside the
    /// commit (`wire_serve::write::splice` → `observed_root`). Differing from
    /// [`Probe::before`] proves the foreign write was VISIBLE to the commit.
    fn commit_saw(&self) -> Option<String> {
        let commit = self.trace.commit.as_ref()?;
        let body: Value = serde_json::from_str(commit.get()).ok()?;
        body.get("fingerprint_before")?.as_str().map(str::to_owned)
    }
}

/// Run one attempt: sample the fingerprint, then run the program with a foreign
/// write scheduled `delay` into the attempt.
fn run_probe(foreign_page: &str, pin: bool, delay: Duration) -> Probe {
    let fixture = Fixture::start();
    let before = fixture.fingerprint();

    let page = fixture.ws.join(foreign_page);
    let mut door = fixture.door().landing(
        page,
        "---\nowner:\n---\n\n# Foreign\n\na line another agent wrote\n",
        delay,
    );

    let mut argv = vec!["--actor".to_owned(), ME.to_owned()];
    for path in [ARMED, READ_ONLY] {
        argv.push("--files".to_owned());
        argv.push(path.to_owned());
    }
    if pin {
        argv.push("--if-fingerprint".to_owned());
        argv.push(before.clone());
    }
    let trace = attempt(&argv, PROGRAM, &mut door).expect("the attempt answers a trace");
    let landed = door.foreign.as_ref().is_some_and(|foreign| foreign.landed);
    let request = door.script_request().clone();
    let ops = door.ops();
    drop(door);
    Probe {
        before,
        trace,
        landed,
        request,
        ops,
        fixture,
    }
}

/// Walk the ladder until a delay lands the foreign write INSIDE the attempt,
/// proved by the pinned arm refusing at the commit rather than pre-eval.
///
/// Returns the delay and the probe that proved it. Panics with every
/// measurement when no delay works — a window that cannot be found is a
/// finding, never a skip.
fn find_the_window(foreign_page: &str) -> (Duration, Probe) {
    let mut seen: Vec<String> = Vec::new();
    for ms in LADDER {
        let delay = Duration::from_millis(ms);
        let probe = run_probe(foreign_page, true, delay);
        assert!(probe.landed, "{ms}ms: the foreign write never happened");
        if probe.entered_before_the_write() && probe.refused_at_the_commit() {
            return (delay, probe);
        }
        seen.push(format!(
            "  {ms:>4}ms → outcome {:?}, entry {} the write, guard_expected {}",
            probe.trace.outcome,
            if probe.entered_before_the_write() {
                "PREDATES"
            } else {
                "INCLUDES"
            },
            if probe.trace.guard_expected.is_some() {
                "present (pre-eval refusal — the write was too EARLY)"
            } else {
                "absent"
            },
        ));
    }
    panic!(
        "no delay on the ladder landed the foreign write between the entry pin and \
         the commit. Every row below is a real attempt against a real daemon; read \
         them before widening the ladder, because \"outcome committed with the entry \
         predating the write\" at EVERY delay would mean the pinned premise stopped \
         being checked at all:\n{}",
        seen.join("\n")
    );
}

// ── predicate 1 + 3: the matched pair ────────────────────────────────────────

/// ⭐ **DONE PREDICATE 1 and 3, as one controlled comparison.**
///
/// Two attempts, identical in every respect — same program, same `files[]`, same
/// corpus, same foreign write landed at the same point inside the attempt —
/// differing in ONE thing: whether the caller passed `--if-fingerprint`.
///
/// * **Without the pin it COMMITS** (predicate 1). The foreign write is a birth
///   at `notes/foreign.md`, which the attempt never read, never expanded to and
///   never armed. It is outside the touch set, so it holds no premise, so it
///   cannot refuse. Under the deleted world-grain law this run refused, and that
///   is the production failure this card exists to fix.
/// * **With the pin it REFUSES** (predicate 3, D-04 preserved). The caller's own
///   token is world-grain by design, and it rides through the touch set as a
///   WIDENING premise — strictest wins. A caller who asks to be told about any
///   corpus movement still is.
///
/// The pair is what makes either half mean anything. The refusal alone could be
/// a lane that refuses everything; the commit alone could be a lane that checks
/// nothing. Together they say the premise is exactly the caller's, and the
/// engine adds none of its own.
#[test]
fn an_untouched_set_commits_while_the_callers_own_pin_still_refuses() {
    let (delay, pinned) = find_the_window(DISJOINT);

    // The pinned arm, as the search proved it: refused, at the commit, with the
    // entry world predating the foreign write.
    assert_eq!(
        pinned.trace.outcome,
        ScriptOutcome::Conflict,
        "D-04: a caller-supplied world-grain premise still refuses on movement \
         OUTSIDE the touch set — strictest wins: {:?}",
        pinned.trace.fault
    );
    assert!(
        pinned.trace.guard_expected.is_none(),
        "and it refused at the COMMIT, not at the pre-eval courtesy check — the \
         pre-eval arm would prove nothing about the widening premise"
    );
    assert_eq!(
        pinned.request["if_fingerprint"],
        json!(pinned.before),
        "the caller's token is what rode the wire"
    );

    // The control: the same everything, minus the flag.
    let free = run_probe(DISJOINT, false, delay);
    assert!(free.landed, "the foreign write has to have happened");
    assert!(
        free.entered_before_the_write(),
        "the control must share the pinned arm's window: its entry world predates \
         the foreign write ({} vs {})",
        free.trace.entry_fingerprint,
        free.before
    );
    assert_eq!(
        free.trace.outcome,
        ScriptOutcome::Committed,
        "THE HEADLINE. A foreign write outside the touch set does not refuse. \
         This is the run that refused in production with all 64 of its targets \
         byte-still: {:?}",
        free.trace.fault
    );
    assert!(
        free.request.get("if_fingerprint").is_none(),
        "and it pinned nothing of its own — the lane mints no world premise: {}",
        free.request
    );

    // The commit really did see the moved world: `fingerprint_before` is the
    // daemon's own LIVE observation, taken inside the commit. Equal to the
    // pre-write sample would mean the write landed after the commit and this
    // control proved nothing.
    let saw = free.commit_saw().expect("a committed run carries its leg");
    assert_ne!(
        saw, free.before,
        "the commit's live observation must include the foreign write, or the \
         write landed too late and the commit was never tested against it"
    );

    // One lane, at the live seam.
    assert_eq!(
        free.ops.iter().filter(|op| *op == "script").count(),
        1,
        "one attempt, one trip: {:?}",
        free.ops
    );

    // And the armed edit really landed. `committed` is a word; the disk is the
    // fact, and a commit that wrote nothing would satisfy every assertion above.
    let on_disk =
        std::fs::read_to_string(free.fixture.ws.join(ARMED)).expect("the card reads back");
    assert!(
        on_disk.lines().any(|line| line == "status: doing"),
        "the claim landed under foreign churn: {on_disk}"
    );
    // The pinned arm, by contrast, landed NOTHING — it refused.
    let untouched =
        std::fs::read_to_string(pinned.fixture.ws.join(ARMED)).expect("the card reads back");
    assert!(
        !untouched.contains(ME),
        "a refused attempt writes nothing: {untouched}"
    );

    println!("the window this machine offered: {delay:?}");
}

// ── predicate 2: a moved touch set refuses, naming the scope ─────────────────

/// ⭐ **DONE PREDICATE 2 — and the word is the one the code emits, not one
/// assumed.**
///
/// The counter-proof to the headline: a foreign write INSIDE the touch set does
/// refuse. The target here is `READ_ONLY` — a page the program READ and never
/// wrote — because the touch set's read half is the half a "premise the writes"
/// implementation silently drops, and dropping it would let a script decide on
/// bytes that changed under it.
///
/// The refusal's spelling is `fingerprint_mismatch` carrying `scope`, the
/// workspace-relative path of the moved premise (§5.7 scoped-guard family).
/// **`scope` is the load-bearing half**: the world-grain refusal this card
/// deleted could not carry one — a root premise has no scope — and its absence
/// is exactly what the production report noticed ("the refusal text also did not
/// name the moved premise's SCOPE, which run-plane.md says a due refusal does").
/// The daemon-side twin is `crates/registry/src/script_op.rs`
/// § `a_foreign_edit_inside_the_touch_set_refuses_naming_the_scope`.
#[test]
fn a_moved_touch_set_refuses_fingerprint_mismatch_naming_the_moved_scope() {
    let mut seen: Vec<String> = Vec::new();
    for ms in LADDER {
        let probe = run_probe(READ_ONLY, false, Duration::from_millis(ms));
        assert!(probe.landed, "{ms}ms: the foreign write never happened");
        if !probe.entered_before_the_write() {
            seen.push(format!("  {ms:>4}ms → the write beat the entry pin"));
            continue;
        }
        if probe.trace.outcome != ScriptOutcome::Conflict {
            seen.push(format!(
                "  {ms:>4}ms → outcome {:?} (the write landed after the commit)",
                probe.trace.outcome
            ));
            continue;
        }

        assert!(
            probe.request.get("if_fingerprint").is_none(),
            "no caller pin rode — so what refused can only be the touch set: {}",
            probe.request
        );
        assert!(
            probe.trace.guard_expected.is_none(),
            "a commit-time refusal, not the pre-eval guard"
        );
        let commit: Value =
            serde_json::from_str(probe.trace.commit.as_ref().expect("the leg").get())
                .expect("the leg is the daemon's own bytes");
        assert_eq!(
            commit["code"],
            json!("fingerprint_mismatch"),
            "the word the code emits: {commit}"
        );
        assert_eq!(
            commit["scope"],
            json!(READ_ONLY),
            "and it NAMES the moved premise's scope — a refusal with no scope is \
             the whole-corpus guard this card deleted: {commit}"
        );
        return;
    }
    panic!(
        "no delay on the ladder landed the foreign write inside the attempt:\n{}",
        seen.join("\n")
    );
}

// ── the deterministic half: no timing, no window ─────────────────────────────

/// **The same law with the race taken out of it.** The foreign write lands
/// BEFORE the attempt starts, so there is nothing to time: the daemon enters at
/// the moved world.
///
/// * A caller pinning the PRE-write fingerprint refuses — the pre-eval courtesy
///   check, which the trace announces by carrying `guard_expected` — with zero
///   evaluation.
/// * The identical run without the flag commits, at the moved world, because the
///   lane invents no premise of its own.
///
/// This is the cheapest possible statement of what this card changed, and the
/// one that cannot go flaky: whatever the machine, the ONLY world-grain premise
/// left on this lane is the one the caller typed.
#[test]
fn the_only_world_grain_premise_left_is_the_one_the_caller_typed() {
    let fixture = Fixture::start();
    let before = fixture.fingerprint();

    std::fs::create_dir_all(fixture.ws.join("notes")).expect("mkdir");
    std::fs::write(
        fixture.ws.join(DISJOINT),
        "---\nowner:\n---\n\n# Foreign\n\nwritten before the attempt\n",
    )
    .expect("the foreign write lands");

    let files = [
        "--files".to_owned(),
        ARMED.to_owned(),
        "--files".to_owned(),
        READ_ONLY.to_owned(),
    ];

    // 1. The caller's pin, now stale by a write that touched nothing this
    //    attempt names. It refuses — D-04 is the caller's to ask for.
    let mut door = fixture.door();
    let mut pinned_argv = vec!["--actor".to_owned(), ME.to_owned()];
    pinned_argv.extend_from_slice(&files);
    pinned_argv.push("--if-fingerprint".to_owned());
    pinned_argv.push(before.clone());
    let pinned = attempt(&pinned_argv, PROGRAM, &mut door).expect("a trace comes back");

    assert_eq!(
        pinned.outcome,
        ScriptOutcome::Conflict,
        "a stale caller pin refuses: {:?}",
        pinned.fault
    );
    assert_eq!(
        pinned.guard_expected.as_deref(),
        Some(before.as_str()),
        "the pre-eval guard names the caller's own token as `expected`, and the \
         entry world as `actual` — both in band, so the face renders from the \
         trace and nothing else"
    );
    assert_ne!(
        pinned.entry_fingerprint, before,
        "the world really did move: {} vs {}",
        pinned.entry_fingerprint, before
    );
    assert_eq!(
        pinned.telemetry.reads_used, 0,
        "zero evaluation — the guard refuses before it reads"
    );

    // 2. The same run without the flag. The world moved just as much; nothing
    //    this attempt touched did.
    let mut door = fixture.door();
    let mut free_argv = vec!["--actor".to_owned(), ME.to_owned()];
    free_argv.extend_from_slice(&files);
    let free = attempt(&free_argv, PROGRAM, &mut door).expect("a trace comes back");

    assert_eq!(
        free.outcome,
        ScriptOutcome::Committed,
        "the lane pins no world of its own, so a moved corpus is not its business: \
         {:?}",
        free.fault
    );
    assert!(
        door.script_request().get("if_fingerprint").is_none(),
        "and the request carries no premise the caller did not type: {}",
        door.script_request()
    );

    // The armed edit landed on disk — a commit that wrote nothing would satisfy
    // the outcome word and prove nothing.
    let on_disk = std::fs::read_to_string(fixture.ws.join(ARMED)).expect("read the card");
    assert!(
        on_disk.lines().any(|line| line == "status: doing"),
        "the claim landed: {on_disk}"
    );
    assert!(
        on_disk.contains(ME),
        "and it landed as this actor's: {on_disk}"
    );
}
