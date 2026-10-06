//! Daemon-level gates for a registry whose engine generations are built
//! through a residency host (merkle-spec §6.8).
//!
//! Two daemons serve ONE workspace: one owns its documents, the other leases
//! them under a controller whose limit is below the corpus's body cost. Every
//! serving door must answer the same bytes on both, the leased one must end
//! within its limit with a live sub cursor riding it, and with its backing
//! gone every door must fault naming a member instead of reading the
//! workspace file.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use ::cache as registry_cache;
use registry::raw_store::RawStore;
use registry::residency::Host;
use registry::residency::controller::Controller;
use registry::{Config, RunningServer, WarmOutcome};
use serde_json::{Value, json};
use tempfile::TempDir;

mod common;

/// Members beside the four named pages, each of one size.
const FILLERS: usize = 96;

/// A hook block and a task, so the run plane has a page to load.
const HOOKS: &str = "\
---
caps: md.edit
task.stamp-task: \"[[#^task-1]]\"
task.stamp-task.caps: md.edit
task.stamp-task.args: value
---

# Hooks

```starlark
def run(event):
    set_field(field = \"seen\", value = event[\"name\"])
    return None

declare(on = \"Stop\")
```
^stamp

```starlark
def run(ctx):
    set_field(field = \"status\", value = ctx.args[0])
```
^task-1
";

/// A daemon config under `root/name`, with reap horizons large enough that no
/// background sweep evicts state mid-test.
#[allow(clippy::duration_suboptimal_units)]
fn config(root: &Path, name: &str, residency: Option<Arc<Host>>) -> Config {
    let forever = Duration::from_secs(365 * 24 * 60 * 60);
    let dir = root.join(name);
    let mut config = Config::for_cache_root(dir.join("cache"));
    config.socket_path = dir.join("daemon.sock");
    config.state_path = dir.join("state.json");
    config.idle_threshold = forever;
    config.reap_interval = forever;
    config.prewarm_interval = forever;
    config.prewarm_quiet_max = forever;
    config.idle_exit = None;
    config.drain_cold_builds = Duration::from_secs(30);
    config.residency = residency;
    config
}

/// A persistent connection speaking raw NDJSON: one frame in, one frame out.
struct Conn {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
}

impl Conn {
    fn open(socket: &Path) -> Self {
        let stream = UnixStream::connect(socket).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_mins(1)))
            .unwrap();
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

    fn hello_v3(&mut self, workspace: &Path) {
        let hello = self.call(&json!({
            "op": "hello", "proto": 1, "contract": "v3",
            "workspace": workspace.to_str().unwrap(),
        }));
        assert_eq!(hello["ok"], json!(true), "{hello}");
    }

    /// The next pushed frame.
    fn next_frame(&mut self) -> Value {
        let mut line = String::new();
        self.reader.read_line(&mut line).unwrap();
        serde_json::from_str(&line).expect("a notification arrives")
    }
}

/// One lock block carrying a pin per `(object, selector path, token)` row:
/// an empty path pins the whole body, `["^id"]` one anchored block.
fn chain_block(pins: &[(&str, &[&str], &str)]) -> String {
    let mut l = lock::Lock::new();
    for (object, path, token) in pins {
        l.upsert_pin(lock::PinEntry::new(
            object,
            "9ae3f1deadbeef",
            lock::Selector::Path(path.iter().map(|segment| (*segment).to_owned()).collect()),
            token,
        ));
    }
    lock::render(&l)
}

/// The live whole-document fingerprint token for `raw`.
fn live_token(raw: &str) -> String {
    let d = model::build(raw.to_string(), syntax::parse(raw));
    model::fingerprint::fingerprint(&d, &d.root)
        .expect("the fixture page has content")
        .into_string()
}

fn filler(n: usize) -> String {
    let line = format!("plain words of member {n}, one line of them.\n");
    format!("# Member {n}\n\nback to [[hub]]\n\n{}", line.repeat(64))
}

/// The workspace: a run page, a page with frontmatter and sections, a hub
/// every filler links back to, a page pinning a block of the hub that its
/// link claims (so its decorations lease the hub beside the page) and one
/// filler whole, and the fillers.
fn files() -> Vec<(String, String)> {
    let hub = "# Hub\n\n[[m000]] ![[m001]] [[nowhere]] ^h\n".to_owned();
    let pinner = format!(
        "# Pinner\n\nreads [[hub#^h]]\n\n{}\n",
        chain_block(&[
            ("hub", &["^h"], &live_token(&hub)),
            ("m002", &[], &live_token(&filler(2))),
        ])
    );
    let mut files = vec![
        ("HOOKS.md".to_owned(), HOOKS.to_owned()),
        (
            "doc.md".to_owned(),
            "---\nstatus: open\n---\n# Alpha\n\none two three\n\n## Beta\n\nfour five\n".to_owned(),
        ),
        ("hub.md".to_owned(), hub),
        ("pinner.md".to_owned(), pinner),
    ];
    files.extend((0..FILLERS).map(|n| (format!("m{n:03}.md"), filler(n))));
    files
}

fn seeded(root: &Path, files: &[(String, String)]) -> PathBuf {
    let ws = root.join("ws");
    fs::create_dir_all(&ws).unwrap();
    for (rel, raw) in files {
        fs::write(ws.join(rel), raw).unwrap();
    }
    ws
}

/// A host whose limit admits four member loads at once and holds a small
/// fraction of the fixture's bodies. Returns it with that limit.
fn host(root: &Path, files: &[(String, String)]) -> (Arc<Host>, usize) {
    let costs: Vec<usize> = files
        .iter()
        .map(|(_, raw)| {
            let doc = model::build(raw.clone(), syntax::parse(raw));
            let bytes = model::allocation::document(&doc);
            bytes.raw + bytes.tree
        })
        .collect();
    let largest = costs.iter().copied().max().unwrap();
    let load_peak = 8 * largest;
    let limit = 4 * load_peak;
    let all: usize = costs.iter().sum();
    assert!(
        all > 2 * limit,
        "the fixture's bodies ({all} bytes) cannot all be resident under the limit ({limit})"
    );
    let host = Arc::new(Host::new(
        Controller::new(limit),
        RawStore::new(&root.join("backing")).unwrap(),
        load_peak,
    ));
    (host, limit)
}

/// Shed every body, then restore the allowance.
fn all_cold(host: &Host, limit: usize) {
    host.controller.set_limit(1);
    assert_eq!(host.controller.report().bodies, 0, "every body is cold");
    host.controller.set_limit(limit);
}

/// Every serving door, by name.
fn doors() -> Vec<(&'static str, Value)> {
    vec![
        ("read", json!({"id": 1, "op": "read", "path": "doc.md"})),
        (
            "read with decorations",
            json!({"id": 2, "op": "read", "path": "pinner.md"}),
        ),
        (
            "links of a page",
            json!({"id": 3, "op": "links", "path": "hub.md"}),
        ),
        ("links map", json!({"id": 4, "op": "links"})),
        (
            "walk up",
            json!({"id": 5, "op": "walk", "path": "pinner.md"}),
        ),
        (
            "walk down",
            json!({"id": 6, "op": "walk", "path": "hub.md", "down": true}),
        ),
        (
            "sql",
            json!({"id": 7, "op": "sql",
                   "query": "SELECT path, file_rev, bytes FROM doc ORDER BY path"}),
        ),
        (
            "run",
            json!({"id": 8, "op": "run", "invocation": "load-8",
                   "targets": [{"page": "HOOKS.md", "mode": "load"}]}),
        ),
        (
            "script",
            json!({"id": 9, "op": "script",
                   "source": "a = read(\"doc.md\")\nb = read(\"m000.md\")\n"}),
        ),
    ]
}

/// Drop measured durations: the only fields two daemons may differ in.
fn scrub(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.retain(|key, _| !(key.ends_with("_ms") || key.ends_with("_us")));
            map.values_mut().for_each(scrub);
        }
        Value::Array(items) => items.iter_mut().for_each(scrub),
        _ => {}
    }
}

/// `request` on both daemons, durations dropped: (owning, leased).
fn both(owning: &mut Conn, leased: &mut Conn, request: &Value) -> (Value, Value) {
    let mut want = owning.call(request);
    let mut got = leased.call(request);
    scrub(&mut want);
    scrub(&mut got);
    (want, got)
}

#[test]
fn a_subscribed_workspace_sheds_to_the_host_limit_and_every_door_serves_the_eager_answer() {
    let tmp = TempDir::new().unwrap();
    let files = files();
    let ws = seeded(tmp.path(), &files);
    let (host, limit) = host(tmp.path(), &files);
    let owning = RunningServer::start(config(tmp.path(), "e", None)).unwrap();
    let leased = RunningServer::start(config(tmp.path(), "r", Some(Arc::clone(&host)))).unwrap();

    // A live sub cursor rides the leased daemon from before its first build.
    let mut sub = Conn::open(leased.socket_path());
    let hello = sub.call(&json!({
        "op": "hello", "proto": 1, "workspace": ws.to_str().unwrap(),
    }));
    assert_eq!(hello["ok"], json!(true), "{hello}");
    let ack = sub.call(&json!({"op": "sub"}));
    assert_eq!(ack["ok"], json!(true), "sub is served: {ack}");
    assert_eq!(ack["body"]["seq"], json!(0), "fresh epoch tip: {ack}");

    let mut plain = Conn::open(owning.socket_path());
    plain.hello_v3(&ws);
    let mut shed = Conn::open(leased.socket_path());
    shed.hello_v3(&ws);

    // Twice: over the generation as built, then with every body shed again.
    for round in 0..2 {
        for (door, request) in doors() {
            let (want, got) = both(&mut plain, &mut shed, &request);
            assert_eq!(
                want["ok"],
                json!(true),
                "{door}: the owning daemon serves: {want}"
            );
            assert_eq!(
                got, want,
                "{door}, round {round}: a leased generation answers the owning one's bytes"
            );
            if door == "sql" {
                assert_eq!(
                    want["body"]["rows"].as_array().map(Vec::len),
                    Some(files.len()),
                    "the statement covers every member: {want}"
                );
            }
        }
        let report = host.controller.report();
        assert_eq!(
            report.members,
            files.len(),
            "every member is a controller member: the generation was built through the host"
        );
        assert_eq!(
            (report.pinned, report.reserved),
            (0, 0),
            "no door kept a lease: {report:?}"
        );
        assert!(
            report.overage == 0 && report.bodies + report.raw <= limit,
            "the subscribed workspace is within the host's limit ({limit}): {report:?}"
        );
        all_cold(&host, limit);
    }

    // The cursor still tells an external edit, and the rebuilt generation
    // answers the owning one's bytes for the moved member.
    fs::write(
        ws.join("m003.md"),
        "# Member 3\n\nrewritten, back to [[hub]]\n",
    )
    .unwrap();
    let frame = sub.next_frame();
    assert_eq!(frame["delta"]["seq"], json!(1), "the next frame: {frame}");
    assert_eq!(
        frame["delta"]["files"][0]["path"],
        json!("m003.md"),
        "{frame}"
    );
    let canonical = fs::canonicalize(&ws).unwrap();
    assert_eq!(
        leased.registry().warm_or_build(&canonical).unwrap(),
        WarmOutcome::Built { docs: 1 },
        "a one-member edit re-publishes that member over the resident generation's slots"
    );
    let moved = json!({"id": 10, "op": "read", "path": "m003.md"});
    let (want, got) = both(&mut plain, &mut shed, &moved);
    assert_eq!(want["ok"], json!(true), "{want}");
    assert_eq!(got, want, "the moved member, after the rebuild");
    let report = host.controller.report();
    assert_eq!(report.overage, 0, "{report:?}");

    // A second workspace, warm and unsubscribed: the reaper charges a
    // host-built generation nothing, so no budget evicts it.
    let second = seeded(&tmp.path().join("second"), &files[..8]);
    let mut other = Conn::open(leased.socket_path());
    other.hello_v3(&second);
    let warmed = other.call(&json!({"id": 11, "op": "read", "path": "doc.md"}));
    assert_eq!(warmed["ok"], json!(true), "{warmed}");
    assert_eq!(
        leased.registry().reap_to_budget(1),
        Vec::<PathBuf>::new(),
        "the reaper charged a host-built generation"
    );

    no_parsed_snapshot_is_saved_or_read_by_a_host_build(
        tmp.path(),
        &files,
        &canonical,
        owning,
        leased,
    );
}

/// rv-352 P3-1, the host arm's snapshot claims. A host-built generation
/// saves no parsed snapshot at shutdown. A host daemon over the owning
/// daemon's cache, which holds an eager parsed snapshot of the workspace,
/// builds every member through its host and never reads that snapshot (a
/// read stamps the drawer's use).
fn no_parsed_snapshot_is_saved_or_read_by_a_host_build(
    tmp: &Path,
    files: &[(String, String)],
    canonical: &Path,
    owning: RunningServer,
    leased: RunningServer,
) {
    leased.shutdown();
    owning.shutdown();
    assert!(
        snapshots(&tmp.join("e")) > 0,
        "the owning daemon saves its parsed snapshot at shutdown"
    );
    assert_eq!(
        snapshots(&tmp.join("r")),
        0,
        "a host-built generation saves no parsed snapshot"
    );

    let drawer = registry_cache::parsed_drawer_dir(&tmp.join("e").join("cache"), canonical);
    let last_use = || match registry_cache::probe(&drawer) {
        registry_cache::Probe::Hit(sentinel) => sentinel.last_use,
        registry_cache::Probe::Miss => panic!("no parsed snapshot drawer at {}", drawer.display()),
    };
    let before = last_use();
    while now_secs() <= before {
        std::thread::sleep(Duration::from_millis(50));
    }
    let again_dir = tmp.join("again");
    fs::create_dir_all(&again_dir).unwrap();
    let (again_host, _) = host(&again_dir, files);
    let again = RunningServer::start(config(tmp, "e", Some(again_host))).unwrap();
    assert_eq!(
        again.registry().warm_or_build(canonical).unwrap(),
        WarmOutcome::Built { docs: files.len() },
        "a host cold build publishes every member"
    );
    assert_eq!(
        last_use(),
        before,
        "a host cold build read the eager parsed snapshot"
    );
    again.shutdown();
}

/// Parsed snapshots (`corpus.v1`) anywhere under `dir`.
fn snapshots(dir: &Path) -> usize {
    fs::read_dir(dir).map_or(0, |entries| {
        entries
            .map(|entry| entry.unwrap().path())
            .map(|path| {
                if path.is_dir() {
                    snapshots(&path)
                } else {
                    usize::from(path.file_name() == Some(std::ffi::OsStr::new("corpus.v1")))
                }
            })
            .sum()
    })
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[test]
fn with_its_backing_gone_every_door_faults_and_none_reads_the_workspace_file() {
    let tmp = TempDir::new().unwrap();
    let files = files();
    let ws = seeded(tmp.path(), &files);
    let (host, limit) = host(tmp.path(), &files);
    let leased = RunningServer::start(config(tmp.path(), "r", Some(Arc::clone(&host)))).unwrap();
    let mut conn = Conn::open(leased.socket_path());
    conn.hello_v3(&ws);

    // Build the generation; the workspace files stay exactly as they are.
    let built = conn.call(&json!({"id": 0, "op": "read", "path": "doc.md"}));
    assert_eq!(built["ok"], json!(true), "{built}");
    assert_eq!(host.controller.report().members, files.len());

    // Every body cold and every retained inode truncated: from here a body
    // can only fault, while each member's workspace file is still readable.
    all_cold(&host, limit);
    for entry in fs::read_dir(tmp.path().join("backing")).unwrap() {
        for file in fs::read_dir(entry.unwrap().path()).unwrap() {
            fs::write(file.unwrap().path(), []).unwrap();
        }
    }

    // What each door's fault must say: the member a named read leases, any
    // member for a scan, and for `sql` the append's own `io_error`.
    let retained = "cannot read retained member ";
    let named = [
        ("read", format!("{retained}doc.md")),
        ("read with decorations", format!("{retained}pinner.md")),
        ("links of a page", retained.to_owned()),
        ("links map", retained.to_owned()),
        ("walk up", format!("{retained}pinner.md")),
        ("walk down", retained.to_owned()),
        ("sql", "\"code\":\"io_error\"".to_owned()),
        ("run", format!("{retained}HOOKS.md")),
        ("script", format!("{retained}doc.md")),
    ];
    for ((door, request), (name, fault)) in doors().into_iter().zip(named) {
        assert_eq!(door, name);
        let frame = conn.call(&request).to_string();
        assert!(
            frame.contains(&fault),
            "{door}: a member whose backing is gone is a fault naming it, never a read of \
             the workspace file: {frame}"
        );
    }

    leased.shutdown();
}
