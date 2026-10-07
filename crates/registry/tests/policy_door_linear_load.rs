//! A door that writes one section of a page under an armed law stages memory
//! in proportion to the page and its answer, not to rows times the heading
//! chain (merkle-spec §6.4, "The policy plane keeps it as well"). The page is
//! the amplifying shape: a heading of half the page governing many short
//! `##` sections, so every section's address repeats the long heading. The
//! workspace arms a CHECK law over the page whose predicate reads every
//! node's `hpath`, so each write builds the change facts and their Starlark
//! values. Five doors run through the in-process daemon, each measured above
//! the warm engine with a counting allocator: a one-section read, a write's
//! `check_write` pre-flight, a planned append, a script put, and the sub
//! frame each write pushes. The SQL door's
//! first query is measured and printed, not bounded: its per-row heading
//! text is the next slice's. This binary holds one test, so nothing else
//! allocates while it measures.

use std::alloc::{GlobalAlloc, Layout, System};
use std::fmt::Write as _;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use registry::{Config, TestServer};
use serde_json::{Value, json};

mod common;

struct Counting;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn grew(size: usize) {
    let live = LIVE.fetch_add(size, Ordering::Relaxed) + size;
    PEAK.fetch_max(live, Ordering::Relaxed);
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            grew(layout.size());
        }
        pointer
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            grew(layout.size());
        }
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let next = unsafe { System.realloc(pointer, layout, size) };
        if !next.is_null() {
            if size >= layout.size() {
                grew(size - layout.size());
            } else {
                LIVE.fetch_sub(layout.size() - size, Ordering::Relaxed);
            }
        }
        next
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// The peak live heap `work` adds above what was live when it started.
fn peak_of<T>(work: impl FnOnce() -> T) -> (T, usize) {
    let base = LIVE.load(Ordering::Relaxed);
    PEAK.store(base, Ordering::Relaxed);
    let out = work();
    (out, PEAK.load(Ordering::Relaxed) - base)
}

/// Per input byte, above the warm engine, that a door may stage.
const PER_BYTE: usize = 1024;
const FIXED: usize = 256 * 1024;
/// Off the doubling points, where growth buffers are not exactly full.
const SIZE: usize = 48 * 1024 + 3;
const PAGE: &str = "tasks/page.md";

/// The armed law: a CHECK over `tasks/**` whose predicate reads every node's
/// heading path, before and after the write. It passes, so writes land.
const LAW: &str = "---\ntags: [type/rule, rules/check]\nid: depth.check\n\
    paths:\n  - tasks/**\n---\n\n# depth.check\n\n\
    ```starlark\ndef check_change(change):\n    \
    for doc in [change.before, change.doc]:\n        \
    for n in doc.nodes:\n            \
    if len(n.hpath) > 6:\n                \
    refuse(message = \"too deep\", passing = \"scenarios/flat.md\")\n```\n";
const LAW_PATH: &str = "rules/depth.md";

#[allow(clippy::duration_suboptimal_units)]
fn test_config(root: &Path) -> Config {
    let forever = Duration::from_secs(365 * 24 * 60 * 60);
    let dir = root.join("registry");
    let mut config = Config::for_cache_root(root.join("cache"));
    config.socket_path = dir.join("daemon.sock");
    config.state_path = dir.join("state.json");
    config.idle_threshold = forever;
    config.reap_interval = forever;
    config.prewarm_interval = forever;
    config.prewarm_quiet_max = forever;
    config.idle_exit = None;
    config.drain_cold_builds = Duration::from_secs(30);
    config.sub_idle_write_timeout = forever;
    config
}

struct Conn {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
}

impl Conn {
    fn open(socket: &Path) -> Self {
        let stream = UnixStream::connect(socket).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .unwrap();
        Conn {
            writer: stream.try_clone().unwrap(),
            reader: BufReader::new(stream),
        }
    }

    /// One request; the response and its line length (the answer's bytes).
    fn call(&mut self, request: &Value) -> (Value, usize) {
        let mut len = 0;
        let value = common::honour_retry(|| {
            let mut line = serde_json::to_string(request).unwrap();
            line.push('\n');
            self.writer.write_all(line.as_bytes()).unwrap();
            self.writer.flush().unwrap();
            let (value, n) = self.line(&request["op"]);
            len = n;
            value
        });
        (value, len)
    }

    /// The next line the daemon sends, parsed, with its length; `after`
    /// names what it answers when none arrives.
    fn line(&mut self, after: &Value) -> (Value, usize) {
        let mut line = String::new();
        if let Err(e) = self.reader.read_line(&mut line) {
            panic!("no line after {after}: {e}");
        }
        let value = serde_json::from_str(&line).unwrap_or_else(|e| panic!("{line:?}: {e}"));
        (value, line.len())
    }

    /// Bind the connection: the v3 contract for doors, the default one for
    /// a subscriber (as `sub_push.rs` subscribes).
    fn hello(&mut self, ws: &Path, v3: bool) {
        let mut hello = json!({"op": "hello", "proto": 1, "workspace": ws.to_str().unwrap()});
        if v3 {
            hello["contract"] = json!("v3");
        }
        let (hi, _) = self.call(&hello);
        assert_eq!(hi["ok"], json!(true), "{hi}");
    }
}

/// The long heading, and the page: that heading over short `##` leaves.
fn page() -> (String, String) {
    let heading = "h".repeat(SIZE / 2);
    let mut page = format!("# {heading}\n\n");
    let mut leaf = 0;
    while page.len() + 10 <= SIZE {
        let _ = write!(page, "## s{leaf:04}\n\n");
        leaf += 1;
    }
    page.push_str(&" ".repeat(SIZE - page.len() - 1));
    page.push('\n');
    (heading, page)
}

/// The workspace: the page, the law's page, and the attested artifact that
/// arms the law at the workspace root, with the once-armed marker.
fn seeded(root: &Path, page: &str) -> PathBuf {
    let ws = root.join("project");
    let index = policy::RuleIndex::discover([policy::PageRef {
        layer: policy::ScopeLayer::Workspace,
        page: LAW_PATH,
        bytes: LAW,
    }]);
    let source = std::collections::BTreeMap::from([(LAW_PATH.to_string(), LAW.to_string())]);
    let artifact = policy::armed::arm(
        &index,
        &policy::armed::ArmRoot::workspace(),
        [policy::armed::ArmRequest {
            id: policy::RuleId::parse("depth.check").expect("a legal id"),
            mode: policy::armed::Mode::Block,
            attested_rev: policy::page_rev(LAW),
        }],
        &source,
        policy::CheckLimits::default(),
    )
    .expect("arms")
    .render();
    for (rel, content) in [
        (PAGE, page),
        (LAW_PATH, LAW),
        (::fs::domain::ARMED_RULES_PATH, artifact.as_str()),
        (::fs::domain::ATTESTED_MARKER_PATH, ""),
    ] {
        let path = ws.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
    ws
}

/// Each door's peak against its bound, and whether it refused: a door
/// fails when it stages more than 256 KiB + 1024 per page byte + its answer,
/// or when it refuses (a law that cannot evaluate a write refuses it).
struct Meter {
    page: usize,
    report: String,
    failed: Vec<String>,
}

impl Meter {
    fn door(&mut self, name: &str, peak: usize, answer: usize, refusal: Option<&Value>) {
        let bound = FIXED + PER_BYTE * self.page + answer;
        let refused = refusal.map_or_else(String::new, |r| {
            format!(
                ", REFUSED {}",
                r.to_string().chars().take(400).collect::<String>()
            )
        });
        let _ = writeln!(
            self.report,
            "{name}: peak {peak} B ({} per page byte), answer {answer} B, bound {bound}{refused}",
            peak / self.page
        );
        if peak > bound || refusal.is_some() {
            self.failed.push(name.to_string());
        }
    }

    fn measured(&mut self, name: &str, peak: usize, answer: usize) {
        let _ = writeln!(
            self.report,
            "{name} (measured, not bounded): peak {peak} B ({} per page byte), answer {answer} B",
            peak / self.page
        );
    }

    fn finish(self) {
        eprintln!("page {} B\n{}", self.page, self.report);
        assert!(
            self.failed.is_empty(),
            "over the bound or refused: {:?}\npage {} B\n{}",
            self.failed,
            self.page,
            self.report
        );
    }
}

/// One write through `conn` and, when it commits, the frame it pushes to
/// `sub`, measured together: (response, answer bytes, frame, peak).
fn write_door(
    conn: &mut Conn,
    sub: &mut Conn,
    request: &Value,
    committed: impl Fn(&Value) -> bool,
) -> (Value, usize, Option<Value>, usize) {
    let (((out, len), frame), peak) = peak_of(|| {
        let out = conn.call(request);
        let frame = committed(&out.0).then(|| sub.line(&json!("the write's sub frame")));
        (out, frame)
    });
    let (frame, frame_len) = frame.map_or((None, 0), |(f, n)| (Some(f), n));
    (out, len + frame_len, frame, peak)
}

#[test]
fn a_write_under_an_armed_law_stages_memory_in_proportion_to_the_page() {
    let (heading, page) = page();
    let daemon = TestServer::start(test_config).unwrap();
    let ws = seeded(daemon.path(), &page);
    let socket = daemon.socket_path();
    let mut conn = Conn::open(&socket);
    conn.hello(&ws, true);
    // Warm: the page is resident before any door is measured.
    let (toc, _) = conn.call(&json!({"op": "toc", "path": PAGE}));
    assert_eq!(toc["ok"], json!(true), "{toc}");
    let mut sub = Conn::open(&socket);
    sub.hello(&ws, false);
    let (ack, _) = sub.call(&json!({"op": "sub"}));
    assert_eq!(ack["ok"], json!(true), "{ack}");

    let address = |leaf: &str| json!([{"h": heading}, {"h": leaf}]);
    let mut meter = Meter {
        page: page.len(),
        report: String::new(),
        failed: Vec::new(),
    };

    let ((read, read_len), peak) = peak_of(|| {
        conn.call(&json!({
            "id": 1, "op": "read", "path": PAGE,
            "sections": [{"hpath": address("s0777")}],
        }))
    });
    assert_eq!(read["ok"], json!(true), "{read}");
    meter.door("read one section", peak, read_len, None);
    // The write's CAS token is the read's own `sec_rev`.
    let rev = read["body"]["sections"][0]["sec_rev"]
        .as_str()
        .unwrap_or_else(|| panic!("the read serves sec_rev: {read}"))
        .to_string();

    let target = ws.join(PAGE);
    let ((check, check_len), peak) = peak_of(|| {
        conn.call(&json!({
            "id": 5, "op": "check_write", "path": PAGE,
            "target": target.to_str().unwrap(), "actor": "agent:t",
            "now": "2026-10-07T00:00:00Z",
            "edits": [{"op": "append", "at": address("s0780"), "body": "z\n"}],
        }))
    });
    assert_eq!(check["ok"], json!(true), "{check}");
    let refused = (!check["body"]["refuse"].is_null()).then_some(&check["body"]["refuse"]);
    meter.door("check_write pre-flight", peak, check_len, refused);

    let ok = |out: &Value| out["ok"] == json!(true);
    let (write, answer, frame, peak) = write_door(
        &mut conn,
        &mut sub,
        &json!({
            "id": 2, "op": "splice", "path": PAGE,
            "plan_edits": [{"append": {"hpath": address("s0777"), "body": "x\n", "rev": rev}}],
        }),
        ok,
    );
    if let Some(frame) = frame {
        assert_eq!(frame["delta"]["files"][0]["path"], json!(PAGE), "{frame}");
    }
    let refused = (!ok(&write)).then_some(&write["error"]);
    meter.door("plan + write, with its sub frame", peak, answer, refused);

    let committed = |out: &Value| out["body"]["outcome"] == json!("committed");
    let source = format!("put(\"{PAGE}\", section=\"{heading}/s0778\", append=\"y\\n\")\n");
    let (script, answer, _, peak) = write_door(
        &mut conn,
        &mut sub,
        &json!({"id": 3, "op": "script", "source": source}),
        committed,
    );
    let refused = (!committed(&script)).then_some(&script["body"]);
    meter.door("script put, with its sub frame", peak, answer, refused);

    let ((sql, sql_len), peak) = peak_of(|| {
        conn.call(&json!({
            "id": 4, "op": "sql",
            "query": "SELECT heading FROM section WHERE heading = 's0779'",
        }))
    });
    assert_eq!(sql["ok"], json!(true), "{sql}");
    meter.measured("sql first query", peak, sql_len);

    meter.finish();
    daemon.shutdown();
}
