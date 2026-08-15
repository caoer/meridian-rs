//! E2E gates for `rules` — effective rules at a path over the wire
//! (`docs/wire-contract.md` § A.12): per rule id at PATH, the page that
//! governs, its scope, and the pages it shadows, plus the ARM artifact's
//! cell — the diagnostic for a write this wire refused on an armed rule.
//! Read-only, workspace-bound, v3-only, advertised as cap `rules`.
//!
//! The env-dependent lifecycle rides ONE test fn by design (edition 2024:
//! env mutation is unsafe, and `MERIDIAN_CONFIG` is process-global); the
//! caps and strict-wall gates are env-free — they refuse before the serve
//! path reads the environment.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use registry::{Config, RunningServer};
use serde_json::{Value, json};
use tempfile::TempDir;

/// A daemon config rooted under `tmp`, with reap horizons large enough that
/// the background reaper never evicts state mid-test.
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
        serde_json::from_str(&response).unwrap()
    }

    fn hello_v3(&mut self, workspace: &Path) -> Value {
        self.call(&json!({
            "op": "hello", "proto": 1, "contract": "v3",
            "workspace": workspace.to_str().unwrap(),
        }))
    }
}

fn write(ws: &Path, rel: &str, body: &str) {
    let p = ws.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

/// A minimal rule page: registration tag + id.
fn rule_page(id: &str) -> String {
    format!("---\ntags: [type/rule, rules/hook]\nid: {id}\n---\n\n# rule\n")
}

/// Arm `requests` (`(arm root, id, mode)`) through the landed resolver and
/// write the attested artifact — the `rules_cli.rs` fixture, one owner of
/// the arm law (`policy::armed::arm`), never a hand-built page.
fn arm(ws: &Path, requests: &[(&str, &str, &str)]) {
    let root = ::fs::WorkspaceRoot(ws.to_path_buf());
    let (files, _) = ::fs::domain_snapshot(&root).expect("snapshot");
    let text: Vec<(String, String)> = files
        .into_iter()
        .filter_map(|(page, bytes)| String::from_utf8(bytes).ok().map(|text| (page, text)))
        .collect();
    let index = policy::RuleIndex::discover(text.iter().map(|(page, bytes)| policy::PageRef {
        layer: policy::ScopeLayer::Workspace,
        page,
        bytes,
    }));

    let mut artifact: Option<policy::armed::ArmedArtifact> = None;
    for (arm_root, id, mode) in requests {
        let root = policy::armed::ArmRoot::parse(arm_root).expect("a legal root");
        let attested_rev = index
            .narrowed_to(root.as_str())
            .resolve()
            .get(id)
            .expect("the id resolves at this root")
            .winner()
            .rev()
            .to_owned();
        let act = policy::armed::arm(
            &index,
            &root,
            vec![policy::armed::ArmRequest {
                id: policy::RuleId::parse(id).expect("a legal id"),
                mode: policy::armed::Mode::parse(mode).expect("a legal mode"),
                attested_rev,
            }],
        )
        .expect("the arm act");
        match artifact.as_mut() {
            None => artifact = Some(act),
            Some(held) => held.merge(act).expect("merge"),
        }
    }
    let page = artifact.expect("at least one arm").render();
    write(ws, policy::armed::ARMED_RULES_PATH, &page);
}

fn rows(frame: &Value) -> &Vec<Value> {
    frame["body"]["rules"]
        .as_array()
        .unwrap_or_else(|| panic!("rules array: {frame}"))
}

fn row<'a>(frame: &'a Value, id: &str) -> &'a Value {
    rows(frame)
        .iter()
        .find(|r| r["id"] == json!(id))
        .unwrap_or_else(|| panic!("a row for {id}: {frame}"))
}

/// The § A.12 lifecycle, one env pin (`MERIDIAN_CONFIG`), one workspace:
/// effective rules at a path with the override chain (winner first, shadowed
/// visible), the armed cell from the attested artifact, the user rung in the
/// chain, refusal scoping, the non-UTF-8 census, the §12.1 `not_offered`
/// census (workspace + user + undecidable), an armed orphan with an
/// established cause, a never-landed path served (the refused-CREATE
/// diagnostic), and the un-narrowed workspace-root view.
#[test]
#[allow(clippy::too_many_lines)] // one sequential lifecycle script by design
fn rules_lifecycle_chain_armed_census_and_orphans() {
    let tmp = TempDir::new().unwrap();

    // The user rung: the anchor and its rules/ tree, plus a dot-declined page.
    let scope_dir = tmp.path().join("conf");
    write(
        &scope_dir,
        "MERIDIAN.md",
        "---\ntype: meridian-config\nversion: 1\n---\n",
    );
    write(&scope_dir, "rules/tone.md", &rule_page("user.tone"));
    write(
        &scope_dir,
        "rules/.drafts/hidden.md",
        &rule_page("user.hidden"),
    );
    let config_path = scope_dir.join("MERIDIAN.md");
    unsafe { std::env::set_var("MERIDIAN_CONFIG", &config_path) };

    // The workspace: an override pair, a refused rule page, a rule page the
    // hash domain excludes, excluded markdown whose frontmatter does not
    // parse, a non-UTF-8 domain member, and a to-be-orphaned armed rule.
    let ws = tmp.path().join("ws");
    write(&ws, "rules.md", &rule_page("task.notify"));
    write(&ws, "sessions/s1/rules.md", &rule_page("task.notify"));
    write(
        &ws,
        "anonymous.md",
        "---\ntags: [rules/hook]\n---\n\n# no id\n",
    );
    write(
        &ws,
        "meridian/domain.md",
        "---\nversion: 1\nignore:\n  - \"bulk/**\"\n---\n",
    );
    write(&ws, "bulk/dropped-rule.md", &rule_page("task.dropped"));
    write(
        &ws,
        "bulk/broken.md",
        "---\ntags: [type/rule\n---\n\n# broken\n",
    );
    write(&ws, "doomed.md", &rule_page("task.doomed"));
    fs::write(ws.join("binary.md"), [0xff, 0xfe, 0x00]).unwrap();

    // Arm the pair's id at the deep root and the doomed id at the workspace
    // root, both through the landed resolver...
    arm(
        &ws,
        &[
            ("sessions/s1", "task.notify", "armed"),
            ("", "task.doomed", "armed"),
        ],
    );
    // ...then delete the doomed page: the artifact still pins it, discovery
    // no longer resolves it — the orphan whose cause is established by
    // looking ("not on disk").
    fs::remove_file(ws.join("doomed.md")).unwrap();

    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    let hi = conn.hello_v3(&ws);
    assert_eq!(hi["ok"], json!(true), "v3 hello binds the workspace: {hi}");

    // Effective rules at the deep path: the chain shows winner first, the
    // shadowed root page visible, and the armed cell from the artifact.
    let s1 = conn.call(&json!({"id": 1, "op": "rules", "path": "sessions/s1"}));
    assert_eq!(s1["ok"], json!(true), "rules serves: {s1}");
    assert_eq!(s1["body"]["at"], json!("sessions/s1"), "{s1}");
    assert!(
        s1["body"]["fingerprint"].is_string(),
        "the answer names the corpus state it was computed at: {s1}"
    );
    let notify = row(&s1, "task.notify");
    assert_eq!(notify["state"], json!("resolved"), "{s1}");
    assert_eq!(notify["armed"]["mode"], json!("armed"), "{s1}");
    assert_eq!(notify["armed"]["rendered"], json!("armed"), "{s1}");
    assert!(
        notify["armed"].get("pinned_page").is_none(),
        "the arm pins the page discovery resolves — no divergence key: {s1}"
    );
    let chain = notify["chain"].as_array().unwrap();
    assert_eq!(chain[0]["role"], json!("winner"), "{s1}");
    assert_eq!(chain[0]["page"], json!("sessions/s1/rules.md"), "{s1}");
    assert_eq!(chain[0]["layer"], json!("workspace"), "{s1}");
    assert_eq!(chain[0]["depth"], json!(2), "{s1}");
    assert_eq!(chain[0]["kinds"], json!(["hook"]), "{s1}");
    assert!(
        chain[0]["rev"].is_string(),
        "a chain row cites its rev: {s1}"
    );
    assert_eq!(chain[1]["role"], json!("shadowed"), "{s1}");
    assert_eq!(chain[1]["page"], json!("rules.md"), "{s1}");
    assert_eq!(chain[1]["depth"], json!(0), "{s1}");

    // The user rung rides the same effective view: declared scope, its rule
    // in effect, its dot-declined page in the user census.
    assert_eq!(
        s1["body"]["user_scope"]["anchor"],
        json!(config_path.to_str().unwrap()),
        "{s1}"
    );
    let tone = row(&s1, "user.tone");
    assert_eq!(tone["chain"][0]["layer"], json!("user"), "{s1}");
    assert_eq!(tone["chain"][0]["page"], json!("rules/tone.md"), "{s1}");

    // Refusal scoping: the root-scoped anonymous page governs this path's
    // chain, so its refusal reddens here.
    let refused = s1["body"]["refused"].as_array().unwrap();
    assert!(
        refused
            .iter()
            .any(|r| r.as_str().unwrap().contains("anonymous.md")),
        "a refused rule page on this chain is named: {s1}"
    );

    // The armed set names the artifact; the orphaned row is counted there
    // and shown in armed_orphans with its established cause.
    assert_eq!(s1["body"]["armed_set"]["state"], json!("present"), "{s1}");
    assert_eq!(
        s1["body"]["armed_set"]["path"],
        json!("meridian/armed-rules.md"),
        "{s1}"
    );
    let orphans = s1["body"]["armed_orphans"].as_array().unwrap();
    assert_eq!(orphans.len(), 1, "one armed row lost its page: {s1}");
    assert_eq!(orphans[0]["id"], json!("task.doomed"), "{s1}");
    assert_eq!(orphans[0]["page"], json!("doomed.md"), "{s1}");
    assert_eq!(orphans[0]["mode"], json!("armed"), "{s1}");
    assert_eq!(orphans[0]["cause"], json!("not on disk"), "{s1}");

    // §12.1 enumerator clause: the rule page the hash domain excludes is
    // named; the unparseable one is undecidable, never claimed either way;
    // the user rung's dot-declined page rides the user census.
    let not_offered = &s1["body"]["not_offered"];
    assert_eq!(
        not_offered["workspace"],
        json!(["bulk/dropped-rule.md"]),
        "{s1}"
    );
    assert_eq!(
        not_offered["undecidable"],
        json!(["bulk/broken.md"]),
        "{s1}"
    );
    assert_eq!(
        not_offered["user"],
        json!(["rules/.drafts/hidden.md"]),
        "{s1}"
    );

    // The non-UTF-8 domain member is a named hole, not a silent skip.
    let unreadable = s1["body"]["unreadable"].as_array().unwrap();
    assert!(
        unreadable
            .iter()
            .any(|u| u.as_str().unwrap().contains("binary.md")),
        "{s1}"
    );

    // A never-landed path is served — the refused-CREATE diagnostic. The
    // deep pair still governs the containing folder's spelling.
    let ghost = conn.call(&json!({"id": 2, "op": "rules", "path": "sessions/s1/never-landed.md"}));
    assert_eq!(
        ghost["ok"],
        json!(true),
        "a never-landed path serves: {ghost}"
    );
    let ghost_notify = row(&ghost, "task.notify");
    assert_eq!(
        ghost_notify["chain"][0]["page"],
        json!("sessions/s1/rules.md"),
        "{ghost}"
    );

    // Absent path = the workspace root, un-narrowed: the deep page still
    // registers (offered un-narrowed), and the root page governs the root.
    let root = conn.call(&json!({"id": 3, "op": "rules"}));
    assert_eq!(root["ok"], json!(true), "{root}");
    assert_eq!(root["body"]["at"], json!(""), "{root}");
    let root_notify = row(&root, "task.notify");
    assert_eq!(
        root_notify["chain"][0]["page"],
        json!("rules.md"),
        "at the root the root page governs: {root}"
    );

    // An armed rule refusing a write is the op's motivating consumer: the
    // splice this artifact blocks and the diagnostic that explains it come
    // from the same law. (The block mode was armed on task.doomed whose page
    // is gone — the orphan above IS that diagnostic.)
}

/// v3 advertises `rules` at op grain; the frozen v2 caps stay byte-identical,
/// and a v2 session's `rules` answers `unknown_op` (§3.2 discovery honesty).
#[test]
fn v3_advertises_rules_and_v2_answers_unknown_op() {
    let tmp = TempDir::new().unwrap();
    let server = RunningServer::start(test_config(&tmp)).unwrap();

    let mut v3 = Conn::open(server.socket_path());
    let hi = v3.call(&json!({"op": "hello", "proto": 1, "contract": "v3"}));
    let caps: Vec<&str> = hi["body"]["caps"]
        .as_array()
        .expect("caps array")
        .iter()
        .map(|c| c.as_str().unwrap())
        .collect();
    assert!(caps.contains(&"rules"), "v3 caps advertise the op: {hi}");

    let ws = tmp.path().join("ws");
    fs::create_dir_all(&ws).unwrap();
    let mut v2 = Conn::open(server.socket_path());
    let hi2 = v2.call(&json!({
        "op": "hello", "proto": 1, "workspace": ws.to_str().unwrap(),
    }));
    let v2_caps: Vec<&str> = hi2["body"]["caps"]
        .as_array()
        .expect("caps array")
        .iter()
        .map(|c| c.as_str().unwrap())
        .collect();
    assert!(
        !v2_caps.contains(&"rules"),
        "frozen v2 caps never grow the op: {hi2}"
    );
    let refused = v2.call(&json!({"id": 2, "op": "rules"}));
    assert_eq!(refused["ok"], json!(false));
    assert_eq!(
        refused["error"]["code"],
        json!("unknown_op"),
        "a v2 session answers unknown_op: {refused}"
    );
}

/// The strict field wall (§3.2): `rules` takes `path` alone, and an unknown
/// field refuses by name — before the serve path reads anything.
#[test]
fn strict_decode_refuses_an_unknown_field_by_name() {
    let tmp = TempDir::new().unwrap();
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    conn.call(&json!({"op": "hello", "proto": 1, "contract": "v3"}));

    let refused = conn.call(&json!({"id": 3, "op": "rules", "layer": "user"}));
    assert_eq!(refused["ok"], json!(false), "{refused}");
    assert_eq!(refused["error"]["code"], json!("bad_request"), "{refused}");
    let message = refused["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("unknown request field `layer` on `rules`"),
        "the wall names the field and the op: {message}"
    );
}
