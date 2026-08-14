//! The six golden scenarios (`inbox/run-golden.html` v9) run through `mrd
//! script` against a LIVE daemon, held to ONE law (CAS relaxation, ruling
//! 2026-08-13):
//!
//! > **Every write row reaches the wire door carrying a CAS token** — threaded
//! > from the script's own reads when they cover the target, minted by the
//! > lane's own commit-time `toc` trip when they do not. The author performs
//! > no read ritual; the commit's entry fingerprint is the enforcement point.
//!
//! Why a live daemon and not the `Door` fake (`script_cmd.rs`). The guard that
//! demands the token is scoped by ORIGIN (`wire-serve::guard`): `Origin::Wire` —
//! the daemon socket — demands a per-row fingerprint, and `Origin::InProcess` is
//! exempt. A fake door can assert what the client SAID; only a real socket can
//! assert that the engine ACCEPTED it. Both halves are needed, so this suite
//! records the requests on their way through a real `RunningServer`.
//!
//! The table below is the whole point: every golden scenario, its write rows, and
//! where each row's token came from.

use std::fmt::Write as _;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use mrd::script::cmd::attempt;
use mrd::script::{Door, ScriptOutcome, ScriptTrace, TraceEntry};
use registry::{Config, RunningServer};
use serde_json::{Value, json};
use tempfile::TempDir;

// ── the corpus ────────────────────────────────────────────────────────────────

/// The agent every scenario runs as — the golden page's `me()`.
const ME: &str = "8ab41c02";
/// The dead agent scenario 2 releases.
const DEAD: &str = "3f9a1c07";

/// A task card: frontmatter the scenarios branch on, plus one body section.
fn card(owner: &str, status: &str, title: &str) -> String {
    format!("---\nowner: {owner}\nstatus: {status}\n---\n\n# {title}\n\nbody\n")
}

/// The five `tasks/*.md` the host selector resolves, in sorted order — the same
/// `files[]` the golden page passes.
const FILES: [&str; 5] = [
    "tasks/0004-index-rebuild.md",
    "tasks/0009-peer-gossip.md",
    "tasks/0011-token-audit.md",
    "tasks/0012-cache-sweep.md",
    "tasks/0014-lease-sweep.md",
];

/// The card scenarios 1, 4, 5 and 6 claim: unowned, `status: todo`, one `Goals`
/// section for the `append` grain.
const CARD: &str = "tasks/0011-token-audit.md";
/// Scenario 3A's round file.
const ROUND: &str = "status/round-7.md";
/// Scenario 3B's board.
const BOARD: &str = "BROADCAST.md";

/// The workspace every scenario runs against. Sections sit at the top level —
/// `put(section="Close")` arms the one segment `Close`, so a section nested under
/// an `# H1` would be a different address and a different question than the one
/// this suite asks.
fn corpus() -> Vec<(&'static str, String)> {
    vec![
        (FILES[0], card(DEAD, "doing", "Index rebuild")),
        (FILES[1], card(DEAD, "doing", "Peer gossip")),
        (
            CARD,
            "---\nowner:\nstatus: todo\n---\n\n# Goals\n\nship the script entry\n".to_owned(),
        ),
        (FILES[3], card("16613c6d", "todo", "Cache sweep")),
        (FILES[4], card(DEAD, "doing", "Lease sweep")),
        (
            ROUND,
            "---\nround: 7\n---\n\n# Close\n\n- earlier line\n".to_owned(),
        ),
        (
            BOARD,
            "---\nround_7:\n---\n\n# Log\n\n- earlier line\n".to_owned(),
        ),
    ]
}

// ── the golden scripts, verbatim in shape ─────────────────────────────────────

const S1_CLAIM: &str = r#"
card = read("tasks/0011-token-audit.md")
if card["fm"]["owner"] == "":
    put("tasks/0011-token-audit.md", props={"owner": me(), "status": "doing"})
"#;

const S2A_NAIVE: &str = r#"
dead = "3f9a1c07"
for path in files:
    card = read(path)
    if card["fm"]["owner"] == dead:
        put(path, props={"owner": "", "status": "todo"})
"#;

const S2B_FANOUT: &str = r#"
card = read(files[0])
if card["fm"]["owner"] == "3f9a1c07":
    put(files[0], props={"owner": "", "status": "todo"})
"#;

const S3A_ROUND_CLOSE: &str = r#"
open_cards = [p for p in files
              if read(p)["fm"]["owner"] == me()
              and read(p)["fm"]["status"] != "done"]
close = read("status/round-7.md", section="Close")
put("status/round-7.md", section="Close",
    append="- 8ab41c02: " + str(len(open_cards)) + " open at close\n")
"#;

/// 3A as golden v8 wrote it — the same append with its line-4 `read` removed.
/// Not a golden scenario: it is the unread-target shape the CAS relaxation is
/// pinned against — the lane mints the token and the engine accepts the batch.
const S3A_WITHOUT_ITS_READ: &str = r#"
open_cards = [p for p in files
              if read(p)["fm"]["owner"] == me()
              and read(p)["fm"]["status"] != "done"]
put("status/round-7.md", section="Close",
    append="- 8ab41c02: " + str(len(open_cards)) + " open at close\n")
"#;

const S3B_BROADCAST: &str = r#"
open_cards = [p for p in files
              if read(p)["fm"]["owner"] == me()
              and read(p)["fm"]["status"] != "done"]
board = read("BROADCAST.md")
if len(open_cards) == 0 and board["fm"]["round_7"] == "":
    put("BROADCAST.md", section="Log",
        props={"round_7": "closed by 8ab41c02"},
        append="- round 7 closed: all cards done (8ab41c02)\n")
"#;

const S4_FAULT: &str = r#"
card = read("tasks/0011-token-audit.md")
put("tasks/0011-token-audit.md",
    props={"owner": me(), "status": "doing"})
report = read(card["fm"]["report_path"])
"#;

const S5_ECHO: &str = r#"
card = read("tasks/0011-token-audit.md")
owners = [read(t)["fm"]["owner"] for t in files]
if read("tasks/0012-cache-sweep.md")["fm"]["status"] == "todo":
    put("tasks/0011-token-audit.md", props={"status": "doing"})
"#;

/// Scenario 5's zero-armed branch — the same three read positions, a condition
/// that is false against this corpus (`0012` is `todo`, never `done`).
const S5Z_ZERO_ARMED: &str = r#"
card = read("tasks/0011-token-audit.md")
owners = [read(t)["fm"]["owner"] for t in files]
if read("tasks/0012-cache-sweep.md")["fm"]["status"] == "done":
    put("tasks/0011-token-audit.md", props={"status": "doing"})
"#;

// ── the scenario table ────────────────────────────────────────────────────────

/// What the golden page says this scenario ends as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ends {
    /// The one splice landed — `committed`.
    Committed,
    /// A rehearsal: the splice ran with `dry: true` and applied nothing.
    Rehearsed,
    /// Nothing was armed, so no splice was issued at all.
    ZeroArmed,
    /// An arm-time law refused before the commit — no splice.
    ArmRefused,
    /// Evaluation faulted before the commit — no splice.
    Faulted,
}

/// One row of the golden table.
struct Scenario {
    /// The golden page's own label.
    id: &'static str,
    source: &'static str,
    /// `files[]` as the host resolves the selector.
    files: bool,
    dry: bool,
    /// The caller's own `--if-fingerprint`: `true` pins the LIVE entry value,
    /// which is what scenario 3B's call carries.
    pin_entry: bool,
    ends: Ends,
    /// The write targets the scenario arms — each one must have been read.
    writes: &'static [&'static str],
}

/// Every golden scenario, 3A included: golden v9 gives it the one
/// `read("status/round-7.md", section="Close")` its append's node rev comes from,
/// so its token is its own read's. The unread-target direction — the lane mints
/// the token itself — is pinned separately, against the ENGINE, by
/// [`an_append_to_a_target_the_script_never_read_mints_and_commits`].
const TABLE: &[Scenario] = &[
    Scenario {
        id: "1 · claim-if-unowned",
        source: S1_CLAIM,
        files: false,
        dry: false,
        pin_entry: false,
        ends: Ends::Committed,
        writes: &[CARD],
    },
    Scenario {
        // Golden v9 depicted this arming REFUSED (`multi_file_write_set`).
        // That arm-time law retired with the §4.4 set form (ruling
        // 2026-08-14): the same script now arms every dead-owned card and
        // commits them as ONE sealed set — one splice, one fingerprint
        // advance. 2b's host fan-out remains legal, no longer required.
        id: "2a · the naive arming — one set commit",
        source: S2A_NAIVE,
        files: true,
        dry: false,
        pin_entry: false,
        ends: Ends::Committed,
        writes: &[FILES[0], FILES[1], FILES[4]],
    },
    Scenario {
        id: "2b · the host fan-out",
        source: S2B_FANOUT,
        files: true,
        dry: false,
        pin_entry: false,
        ends: Ends::Committed,
        writes: &[FILES[0]],
    },
    Scenario {
        id: "3A · round close — count + append status",
        source: S3A_ROUND_CLOSE,
        files: true,
        dry: false,
        pin_entry: false,
        ends: Ends::Committed,
        writes: &[ROUND],
    },
    Scenario {
        id: "3B · claim-shaped conditional broadcast",
        source: S3B_BROADCAST,
        files: true,
        dry: false,
        pin_entry: true,
        ends: Ends::Committed,
        writes: &[BOARD],
    },
    Scenario {
        id: "4 · runtime fault",
        source: S4_FAULT,
        files: false,
        dry: false,
        pin_entry: false,
        ends: Ends::Faulted,
        writes: &[],
    },
    Scenario {
        id: "5 · echo demo",
        source: S5_ECHO,
        files: true,
        dry: false,
        pin_entry: false,
        ends: Ends::Committed,
        writes: &[CARD],
    },
    Scenario {
        id: "5z · the zero-armed branch",
        source: S5Z_ZERO_ARMED,
        files: true,
        dry: false,
        pin_entry: false,
        ends: Ends::ZeroArmed,
        writes: &[],
    },
    Scenario {
        id: "6 · dry preview",
        source: S1_CLAIM,
        files: false,
        dry: true,
        pin_entry: false,
        ends: Ends::Rehearsed,
        writes: &[CARD],
    },
];

// ── the law ───────────────────────────────────────────────────────────────────

/// **Rider 1.** Every golden scenario, over a live daemon, with every write row
/// showing where its CAS token came from. Every golden reads what it writes, so
/// each token must be one the script's own reads published — the conforming
/// half of the law; the unread-target half (the lane mints the token itself) is
/// pinned by [`an_append_to_a_target_the_script_never_read_mints_and_commits`].
#[test]
fn every_golden_write_row_finds_its_token_in_the_scripts_own_reads() {
    let mut report = String::new();
    let mut broken: Vec<String> = Vec::new();

    for scenario in TABLE {
        let fixture = Fixture::start();
        let (trace, door) = fixture.run(scenario);
        writeln!(report, "\n{}", scenario.id).expect("a String never fails");

        // 1. The outcome the golden page depicts.
        let ends = ends_of(&trace, &door);
        if ends != scenario.ends {
            broken.push(format!(
                "{}: ends {ends:?}, the golden page depicts {:?}{}",
                scenario.id,
                scenario.ends,
                trace
                    .fault
                    .as_ref()
                    .map_or_else(String::new, |f| format!(" — {}", f.reason))
            ));
        }

        // 2. The write set is exactly the targets the scenario names.
        let armed: Vec<&str> = trace
            .armed_entries()
            .map(|entry| entry.path.as_str())
            .collect();
        for path in scenario.writes {
            assert!(
                armed.contains(path),
                "{}: {path} is armed nowhere. armed: {armed:?}",
                scenario.id
            );
        }

        // 3. THE LAW. Every plan row that reached the socket carries a token,
        //    and the token is one this run's own reads published.
        let read_revs = revs_read(&trace);
        for row in plan_rows(&door) {
            let (grain, rev) = grain_and_rev(&row);
            let Some(rev) = rev else {
                broken.push(format!(
                    "{}: a {grain} row went on the wire with NO token — the script never read \
                     its target. row: {row}",
                    scenario.id
                ));
                continue;
            };
            if !read_revs.contains(&rev) {
                broken.push(format!(
                    "{}: a {grain} row carries {rev}, which no read in this run published. \
                     read: {read_revs:?}",
                    scenario.id
                ));
                continue;
            }
            writeln!(report, "  {grain:<13} rev {rev}  ← the script's own read")
                .expect("a String never fails");
        }
        if scenario.writes.is_empty() {
            report.push_str("  (no write rows — nothing reaches the wire)\n");
        }
    }

    assert!(
        broken.is_empty(),
        "the write-follows-read law is broken:\n  {}\n\nscenario table:{report}",
        broken.join("\n  ")
    );
    println!("{report}");
}

/// **Rider 1, the second scenario the Advisor named.** 3B's broadcast writes
/// `BROADCAST.md` at two grains — a `set_property` and an `append` — and it DOES
/// read the board first (line 4, `board = read("BROADCAST.md")`). One toc read
/// serves both tokens: the file rev for the property row, the `Log` row's node
/// rev out of the same section map. It commits over a live daemon.
#[test]
fn scenario_3b_reads_the_board_before_it_claims_the_round() {
    let fixture = Fixture::start();
    let scenario = TABLE
        .iter()
        .find(|s| s.id.starts_with("3B"))
        .expect("3B is in the table");
    let (trace, door) = fixture.run(scenario);

    assert!(
        read_paths(&trace).contains(&BOARD),
        "3B claims {BOARD} but never read it. reads: {:?}",
        read_paths(&trace)
    );
    let rows = plan_rows(&door);
    let property = rows
        .iter()
        .find(|row| row.get("set_property").is_some())
        .expect("3B arms round_7");
    let append = rows
        .iter()
        .find(|row| row.get("append").is_some())
        .expect("3B arms the log line");
    assert!(
        !property["set_property"]["rev"].is_null(),
        "the file-grain token: {property}"
    );
    assert_eq!(
        append["append"]["hpath"],
        json!([{"h": "Log"}]),
        "the append addresses the Log section"
    );
    assert!(
        !append["append"]["rev"].is_null(),
        "the node-grain token, from the same toc read: {append}"
    );
    assert_eq!(trace.outcome, ScriptOutcome::Committed);
}

/// **The law's other direction — an unread target commits, on the lane's own
/// minted token.**
///
/// History, both rulings. Golden v8's 3A appended to `status/round-7.md`
/// having read only `files[]`; the append row went out `rev: null` and the
/// live daemon refused the batch. The Advisor (`d1f489b5`, 2026-08-07) ruled
/// the engine right and gave v9's 3A its read. ZT's CAS-relaxation ruling
/// (2026-08-13, dissolves F-S2) supersedes that direction: appends go
/// rev-free for the AUTHOR (put parity — append cannot clobber), the lane
/// mints the token itself at commit time, and consistency enforcement lives
/// at the commit's entry fingerprint — never as a read-the-section-first
/// ritual. The v8 shape runs on here to pin the relaxation against the real
/// engine: the batch lands, and the row that reached the socket carried the
/// minted token.
#[test]
fn an_append_to_a_target_the_script_never_read_mints_and_commits() {
    const UNREAD_APPEND: Scenario = Scenario {
        id: "3A without its read — the relaxation pin",
        source: S3A_WITHOUT_ITS_READ,
        files: true,
        dry: false,
        pin_entry: false,
        ends: Ends::Committed,
        writes: &[ROUND],
    };
    let fixture = Fixture::start();
    let (trace, door) = fixture.run(&UNREAD_APPEND);

    assert!(
        !read_paths(&trace).contains(&ROUND),
        "the script itself must not read {ROUND} — that is the whole premise"
    );
    let append = plan_rows(&door)
        .into_iter()
        .find(|row| row.get("append").is_some())
        .expect("the scenario arms the append");
    assert!(
        !append["append"]["rev"].is_null(),
        "the lane minted the node-grain token the wire door demands: {append}"
    );
    assert_eq!(
        trace.outcome,
        ScriptOutcome::Committed,
        "rev-free for the author is not tokenless on the wire — the batch \
         lands: {:?}",
        trace.fault
    );
}

/// **The season-1 canary, closed.** Dogfood season-1 finding 1: the golden
/// page's motivating scenario — a claim keyed on frontmatter — could not fire
/// against a card whose frontmatter follows fleet convention
/// (`owner: "3f9a1c07"` — QUOTED), because `card["fm"][k]` served the quote bytes
/// and the comparison silently evaluated false. Zero armed, and the face
/// rendered a legitimate-looking "no effects".
///
/// This is scenario 2b's shape against a fleet-canonical quoted card, over a
/// live daemon and real files: the § A.6 read law must make it arm, and the
/// § A.6.3 write law must land the release as canonical bytes on disk.
#[test]
fn a_condition_keyed_on_a_quoted_scalar_arms_and_commits() {
    const QUOTED_CARD: &str = "tasks/0001-quoted-owner.md";
    const CANARY: &str = r#"
card = read("tasks/0001-quoted-owner.md")
if card["fm"]["owner"] == "3f9a1c07":
    put("tasks/0001-quoted-owner.md", props={"owner": "", "status": "todo"})
"#;
    let fixture = Fixture::start_with(vec![(
        QUOTED_CARD,
        format!("---\nowner: \"{DEAD}\"\nstatus: doing\n---\n\n# Goals\n\nbody\n"),
    )]);
    let scenario = Scenario {
        id: "season-1 canary — quoted owner",
        source: CANARY,
        files: false,
        dry: false,
        pin_entry: false,
        ends: Ends::Committed,
        writes: &[QUOTED_CARD],
    };
    let (trace, door) = fixture.run(&scenario);

    assert_eq!(
        ends_of(&trace, &door),
        Ends::Committed,
        "a quoted fleet-canonical owner must compare equal and arm — the \
         season-1 failure was a silent zero-armed run. trace: {:?}",
        trace.outcome
    );
    let on_disk = std::fs::read_to_string(fixture.ws.join(QUOTED_CARD)).expect("read back");
    assert!(
        on_disk.lines().any(|l| l == "status: todo"),
        "the claim landed as the plain line: {on_disk}"
    );
    assert!(
        on_disk.lines().any(|l| l == r#"owner: """#),
        "the release cleared the owner to the § A.6.3 explicit-empty line, not \
         a forged null: {on_disk}"
    );
    assert!(
        !on_disk.contains(DEAD),
        "no trace of the dead owner remains: {on_disk}"
    );
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// What this run actually amounted to, at the grain the golden page depicts —
/// the outcome word plus whether a splice was issued at all, which is what
/// separates a zero-armed run from a rehearsal.
#[allow(clippy::match_same_arms)] // the Effects arm is spelled for exhaustiveness, not merged away
fn ends_of(trace: &ScriptTrace, door: &LiveDoor) -> Ends {
    let spliced = door.ops().iter().any(|op| op == "splice");
    match (trace.outcome, spliced) {
        (ScriptOutcome::Committed, _) => Ends::Committed,
        (ScriptOutcome::NoEffect, true) => Ends::Rehearsed,
        (ScriptOutcome::NoEffect, false) => Ends::ZeroArmed,
        (ScriptOutcome::Refused, _) => Ends::ArmRefused,
        (ScriptOutcome::Fault | ScriptOutcome::Conflict, _) => Ends::Faulted,
        // Effects is the wire lane's word; this pure CLI lane never produces
        // it — spelled for exhaustiveness only.
        (ScriptOutcome::Effects, _) => Ends::ZeroArmed,
    }
}

/// Every path this run read, in call order — echo and quiet alike.
fn read_paths(trace: &ScriptTrace) -> Vec<&str> {
    trace
        .trace
        .iter()
        .filter_map(|entry| match entry {
            TraceEntry::Read(read) | TraceEntry::Echo(read) => Some(read.path.as_str()),
            TraceEntry::Expanded(_)
            | TraceEntry::Armed(_)
            | TraceEntry::Wrote(_)
            | TraceEntry::Ran(_) => None,
        })
        .collect()
}

/// Every `plan_edits[]` row this run put on the socket.
fn plan_rows(door: &LiveDoor) -> Vec<Value> {
    door.requests()
        .iter()
        .filter(|request| request["op"] == json!("splice"))
        .filter_map(|request| request["plan_edits"].as_array())
        .flatten()
        .cloned()
        .collect()
}

/// The grain a plan row is guarded at, and the token it carried.
fn grain_and_rev(row: &Value) -> (&'static str, Option<String>) {
    let take = |slot: &Value| slot["rev"].as_str().map(str::to_owned);
    if let Some(slot) = row.get("set_property") {
        return ("set_property", take(slot));
    }
    if let Some(slot) = row.get("append") {
        return ("append", take(slot));
    }
    ("(other)", None)
}

/// Every rev this run's own reads published: the file rev of each toc read, every
/// node rev in its section map, and the node rev of each section read.
fn revs_read(trace: &ScriptTrace) -> Vec<String> {
    use effects::ReadFace;
    let mut revs = Vec::new();
    for entry in &trace.trace {
        let read = match entry {
            TraceEntry::Read(read) | TraceEntry::Echo(read) => read,
            TraceEntry::Expanded(_)
            | TraceEntry::Armed(_)
            | TraceEntry::Wrote(_)
            | TraceEntry::Ran(_) => continue,
        };
        match &read.face {
            ReadFace::Toc(facts) => {
                revs.push(facts.rev.clone());
                revs.extend(facts.toc.iter().map(|row| row.rev.clone()));
            }
            ReadFace::Section(facts) => revs.push(facts.rev.clone()),
        }
    }
    revs
}

// ── the live daemon ───────────────────────────────────────────────────────────

/// A real `RunningServer` on a real socket, bound to a fresh corpus.
struct Fixture {
    _tmp: TempDir,
    ws: PathBuf,
    server: RunningServer,
}

impl Fixture {
    fn start() -> Self {
        Self::start_with(corpus())
    }

    /// The same daemon over a caller-chosen corpus — one scenario that needs a
    /// card the golden table does not carry seeds its own.
    fn start_with(files: Vec<(&'static str, String)>) -> Self {
        let tmp = TempDir::new().expect("tempdir");
        let ws = tmp.path().join("ws");
        for (rel, content) in files {
            let path = ws.join(rel);
            std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
            std::fs::write(path, content).expect("seed");
        }
        let server = RunningServer::start(config(&tmp)).expect("the daemon starts");
        Self {
            _tmp: tmp,
            ws,
            server,
        }
    }

    /// Run one scenario end to end against this daemon.
    fn run(&self, scenario: &Scenario) -> (ScriptTrace, LiveDoor) {
        let mut door = LiveDoor::open(self.server.socket_path(), &self.ws);
        let mut argv = vec!["--actor".to_owned(), ME.to_owned()];
        if scenario.files {
            for path in FILES {
                argv.push("--files".to_owned());
                argv.push(path.to_owned());
            }
        }
        if scenario.dry {
            argv.push("--dry".to_owned());
        }
        if scenario.pin_entry {
            argv.push("--if-fingerprint".to_owned());
            argv.push(door.fingerprint());
        }
        let trace = attempt(&argv, scenario.source, &mut door)
            .unwrap_or_else(|e| panic!("{}: the attempt runs: {e}", scenario.id));
        (trace, door)
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
    // The fixture daemon publishes THIS build's identity: the 0025 socket law
    // refuses an identity-less local hello, and these tests measure the wire
    // shape, not the law.
    config.build_sha = Some(env!("MRD_BUILD_SHA").to_owned());
    config
}

/// The production door, plus the census. `SocketDoor` is the shipped one; this is
/// the same NDJSON dialogue with every request kept, which is what lets a live
/// run assert the SHAPE that went on the wire and not only what changed on disk.
struct LiveDoor {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
    requests: Vec<Value>,
}

impl LiveDoor {
    fn open(socket: &Path, ws: &Path) -> Self {
        let stream = UnixStream::connect(socket).expect("dial the daemon");
        let mut door = Self {
            writer: stream.try_clone().expect("clone"),
            reader: BufReader::new(stream),
            requests: Vec::new(),
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

    /// The live entry fingerprint, for a scenario that pins its own guard.
    fn fingerprint(&mut self) -> String {
        let line = self
            .call(&json!({"op": "fingerprint"}))
            .expect("fingerprint");
        serde_json::from_str::<Value>(&line).expect("a frame")["body"]["fingerprint"]
            .as_str()
            .expect("a fingerprint")
            .to_owned()
    }

    fn requests(&self) -> &[Value] {
        &self.requests
    }

    fn ops(&self) -> Vec<String> {
        self.requests
            .iter()
            .filter_map(|r| r["op"].as_str().map(str::to_owned))
            .collect()
    }
}

impl Door for LiveDoor {
    fn call(&mut self, request: &Value) -> io::Result<String> {
        self.requests.push(request.clone());
        let mut line = serde_json::to_string(request)?;
        line.push('\n');
        self.writer.write_all(line.as_bytes())?;
        self.writer.flush()?;
        let mut response = String::new();
        self.reader.read_line(&mut response)?;
        Ok(response)
    }
}

// ── the composed read's bracket, against the LIVE wire ────────────────────────

/// A door that lets a real writer in between the `toc` and the closing `read` of
/// one composed read — the exact interleaving the fake `Door` sequences of
/// `script_cmd.rs` cannot represent, and therefore the one a fixture alone
/// could never show REACHABLE.
///
/// It writes the file itself, once, immediately after the `toc` response comes
/// back. Nothing here simulates a moved world: the bytes change on disk and the
/// daemon answers whatever it then computes.
struct MidReadWriter {
    inner: LiveDoor,
    page: PathBuf,
    wrote: bool,
}

impl Door for MidReadWriter {
    fn call(&mut self, request: &Value) -> io::Result<String> {
        let is_toc = request["op"] == json!("toc");
        let answer = self.inner.call(request)?;
        if is_toc && !self.wrote {
            self.wrote = true;
            let mut body = std::fs::read_to_string(&self.page).expect("read the page");
            body.push_str("\na foreign line\n");
            std::fs::write(&self.page, body).expect("a foreign write lands");
        }
        Ok(answer)
    }
}

/// **The production wire really does answer two revisions across one composed
/// read — measured, not reasoned.** `read(path)` is `toc` + one `cat` per
/// frontmatter key + the closing `read`, and `wire-serve` computes `file_rev`
/// when it answers each one. So a writer landing between them makes the two
/// observations disagree, and the composition would otherwise hand the script a
/// map from one revision and a count from another: a state that never existed.
///
/// This is the reachability half of the gate that
/// `wire_host::tests::a_composition_spanning_two_revisions_refuses_instead_of_being_assembled`
/// states. That one proves the refusal; this one proves the frame it refuses is
/// one the live engine produces. The production call site is `WireHost::toc`,
/// reached by every whole-file `read(path)` in every script.
///
/// The control is scenario 1 in the table above: the identical script, the same
/// daemon, no interleaved writer — it commits.
#[test]
fn a_live_writer_between_the_toc_and_the_closing_read_is_caught_by_the_bracket() {
    let fixture = Fixture::start();
    let claim = "\ncard = read(\"tasks/0011-token-audit.md\")\nif card[\"fm\"][\"owner\"] == \"\":\n    put(\"tasks/0011-token-audit.md\", props={\"owner\": me(), \"status\": \"doing\"})\n";

    let mut door = MidReadWriter {
        inner: LiveDoor::open(fixture.server.socket_path(), &fixture.ws),
        page: fixture.ws.join(CARD),
        wrote: false,
    };
    let argv = ["--actor".to_owned(), ME.to_owned()];
    let trace = attempt(&argv, claim, &mut door).expect("the attempt answers a trace");

    assert!(
        door.wrote,
        "the foreign write has to have happened, or this proves nothing"
    );
    assert_eq!(
        trace.outcome,
        ScriptOutcome::Fault,
        "a composition spanning two revisions refuses: {:?}",
        trace.outcome
    );
    let reason = &trace
        .fault
        .as_ref()
        .expect("a fault carries its reason")
        .reason;
    assert!(
        reason.contains("moved while this read was being composed"),
        "the live refusal is the bracket's own: {reason}"
    );
    assert!(
        !door.inner.ops().iter().any(|op| op == "splice"),
        "nothing was issued: {:?}",
        door.inner.ops()
    );
    // And the world is untouched by the SCRIPT — only the foreign line is there.
    let on_disk = std::fs::read_to_string(fixture.ws.join(CARD)).expect("read the card");
    assert!(
        on_disk.contains("owner:\n") && on_disk.contains("a foreign line"),
        "the script armed nothing and committed nothing:\n{on_disk}"
    );
}
