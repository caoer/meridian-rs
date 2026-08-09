//! E2E gates for the resident-engine handshake (`[[0002-resident-daemon]]` §4).
//! One `hello` round trip negotiates the contract rev, resolves the workspace,
//! pins its storage, warms the engine, binds the connection, and lists the
//! served caps. An unknown declared rev is a loud refusal; `hello` subsumes the
//! deleted `attach` op — no parallel binding path (§5).

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::symlink;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use registry::{Config, RunningServer};
use serde_json::{Value, json};
use tempfile::TempDir;

/// A daemon config rooted under `tmp`, with reap horizons large enough that the
/// background reaper never evicts a warm engine mid-test.
// `Duration::from_hours` is not const-stable at MSRV 1.96; the seconds form is
// the workspace precedent.
#[allow(clippy::duration_suboptimal_units)]
fn test_config(tmp: &TempDir) -> Config {
    test_config_with_build(tmp, None)
}

/// [`test_config`] with a build identity: `Some(sha)` makes the daemon publish
/// `hello.identity.build`, `None` makes it publish no identity at all.
#[allow(clippy::duration_suboptimal_units)]
fn test_config_with_build(tmp: &TempDir, build_sha: Option<&str>) -> Config {
    let forever = Duration::from_secs(365 * 24 * 60 * 60);
    let dir = tmp.path().join("registry");
    let mut config = Config::for_cache_root(tmp.path().join("cache"));
    config.build_sha = build_sha.map(str::to_owned);
    config.socket_path = dir.join("daemon.sock");
    config.state_path = dir.join("state.json");
    config.idle_threshold = forever;
    config.reap_interval = forever;
    config.prewarm_interval = forever;
    config.prewarm_quiet_max = forever;
    // Lifetime is the test's; idle-exit would flake mid-assertion.
    config.idle_exit = None;
    config
}

/// A workspace `tmp/<name>` seeded with `files` — a sibling of the cache root,
/// so the corpus walk never sees the drawer.
fn write_ws(tmp: &TempDir, name: &str, files: &[(&str, &str)]) -> PathBuf {
    let ws = tmp.path().join(name);
    for (rel, content) in files {
        let path = ws.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }
    ws
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

    /// A v3 hello binding `ws`.
    fn hello(&mut self, ws: &Path) -> Value {
        self.call(&json!({
            "op": "hello",
            "proto": 1,
            "contract": "v3",
            "workspace": ws.to_str().unwrap(),
        }))
    }
}

/// The U3 spine gate: one `hello` resolves the workspace, pins storage,
/// negotiates v3, and lists the served caps — one round trip.
#[test]
fn hello_resolves_pins_negotiates_and_lists_caps_in_one_round_trip() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(
        &tmp,
        "ws",
        &[("a.md", "# A\n\nsee [[b]]\n"), ("b.md", "# B\n")],
    );
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());

    let hi = conn.hello(&ws);
    assert_eq!(hi["ok"], json!(true), "hello ok: {hi}");
    let body = &hi["body"];

    // Rev negotiated: proto 1, a named server.
    assert_eq!(body["proto"], json!(1), "hello negotiates proto 1: {hi}");
    assert!(body["server"].is_string(), "hello names the server: {hi}");

    // §3.2 discovery honesty: served ops present, `hello` absent. v3 spells the
    // root capability `fingerprint`.
    let caps: Vec<String> = body["caps"]
        .as_array()
        .expect("caps array")
        .iter()
        .map(|c| c.as_str().unwrap().to_string())
        .collect();
    for served in [
        "toc",
        "cat",
        "extract",
        "resolve",
        "links",
        "fingerprint",
        "diff",
        "splice",
        // Discovery honesty at field grain: dotted `op.field` caps.
        "splice.plan_edits",
        "splice.pin",
        "sub",
    ] {
        assert!(
            caps.contains(&served.to_string()),
            "caps list `{served}`: {hi}"
        );
    }
    for absent in ["hello", "root"] {
        assert!(
            !caps.contains(&absent.to_string()),
            "caps must not list `{absent}` (§3.2 discovery honesty; v3 hides `root`): {hi}"
        );
    }

    // Storage pinned: the drawer sits under this daemon's cache root...
    let storage = body["storage"].as_str().expect("hello pins storage");
    let cache_root = tmp.path().join("cache");
    assert!(
        Path::new(storage).starts_with(&cache_root),
        "the pinned drawer is under the cache root {cache_root:?}: {storage}"
    );
    // ...and the pin registered the workspace — admin `list` now reports it.
    let listed = conn.call(&json!({"op": "list"}));
    assert_eq!(
        listed["entries"].as_array().map(Vec::len),
        Some(1),
        "hello's storage pin registered the workspace: {listed}"
    );

    // The bind is live: a read on the same connection needs no separate attach.
    let toc = conn.call(&json!({"op": "toc", "path": "a.md"}));
    assert_eq!(
        toc["ok"],
        json!(true),
        "bound read serves from the engine: {toc}"
    );
    assert_eq!(toc["body"]["path"], json!("a.md"));

    server.shutdown();
}

/// An unknown declared contract rev is refused loudly at the handshake — never
/// a silent downgrade.
#[test]
fn hello_with_an_unknown_rev_is_a_loud_refusal() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, "ws", &[("a.md", "# A\n")]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());

    let refused = conn.call(&json!({
        "op": "hello",
        "proto": 1,
        "contract": "v9",
        "workspace": ws.to_str().unwrap(),
    }));
    assert_eq!(
        refused["ok"],
        json!(false),
        "unknown rev refused: {refused}"
    );
    assert_eq!(
        refused["error"]["code"],
        json!("bad_request"),
        "an unknown declared rev is bad_request: {refused}"
    );
    assert_eq!(
        refused["error"]["recovery"],
        json!("fix"),
        "bad_request recovers by fix: {refused}"
    );
    assert!(
        refused["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("v9")),
        "the refusal names the bad rev: {refused}"
    );
}

/// The exact-or-refuse gate: `hello.workspace` is a declaration, so it binds
/// exactly the declared path even when an ancestor is already registered — a
/// declared root must never silently widen to an enclosing tree. The ancestor
/// walk lives on only in `Registry::resolve`, whose input is a cwd hint.
#[test]
fn hello_declaration_binds_exactly_never_a_registered_ancestor() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(
        &tmp,
        "ws",
        &[("a.md", "# A\n\nsee [[b]]\n"), ("b.md", "# B\n")],
    );
    let nested = ws.join("sub/deep");
    fs::create_dir_all(&nested).unwrap();
    fs::write(nested.join("c.md"), "# C\n").unwrap();
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());

    // Register the ancestor first.
    let root_pin = conn.hello(&ws);
    let root_storage = root_pin["body"]["storage"]
        .as_str()
        .expect("root hello pins storage")
        .to_string();
    let root_bound = root_pin["body"]["workspace"]
        .as_str()
        .expect("hello names the bound root")
        .to_string();

    // Now declare the nested path. It must bind itself, not the ancestor.
    let sub_pin = conn.hello(&nested);
    let sub_bound = sub_pin["body"]["workspace"]
        .as_str()
        .expect("hello names the bound root");
    assert_eq!(
        Path::new(sub_bound),
        fs::canonicalize(&nested).unwrap(),
        "a declared path binds ITSELF, never a registered ancestor: {sub_pin}"
    );
    assert_ne!(
        sub_bound, root_bound,
        "the declaration did not widen to the ancestor: {sub_pin}"
    );
    assert_ne!(
        sub_pin["body"]["storage"].as_str(),
        Some(root_storage.as_str()),
        "an exactly-bound declaration gets its OWN drawer: {sub_pin}"
    );

    // Both are registered now — the declaration registered itself.
    let listed = conn.call(&json!({"op": "list"}));
    assert_eq!(
        listed["entries"].as_array().map(Vec::len),
        Some(2),
        "the declared path registered itself alongside the ancestor: {listed}"
    );

    // And the bound corpus is the declared tree's, not the ancestor's: `c.md`
    // is addressable at its root, while the ancestor's `a.md` is not in scope.
    let toc = conn.call(&json!({"op": "toc", "path": "c.md"}));
    assert_eq!(
        toc["ok"],
        json!(true),
        "the declared tree's own file is addressable: {toc}"
    );

    server.shutdown();
}

/// A declared path that is a symlink binds the canonicalized real path, and the
/// hello response names it: "exact" means no ancestor widening, not identical
/// bytes — canonicalization still rewrites the caller's spelling.
#[test]
fn a_declared_symlink_binds_the_canonical_root_and_the_answer_names_it() {
    let tmp = TempDir::new().unwrap();
    let real = write_ws(&tmp, "real/proj", &[("a.md", "# A\n")]);
    // tmp/link -> tmp/real, so tmp/link/proj is a second spelling of `real`.
    symlink(tmp.path().join("real"), tmp.path().join("link")).unwrap();
    let declared = tmp.path().join("link/proj");

    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());

    // Fixture precondition: if the spellings coincided (symlink failed, or an
    // already-canonical temp root) every assertion below would hold vacuously.
    let canonical = fs::canonicalize(&real).unwrap();
    assert_ne!(
        declared.as_path(),
        canonical.as_path(),
        "MIS-CONSTRUCTED FIXTURE: the declared spelling must differ from the \
         canonical root or this test discriminates nothing. Built under {}",
        tmp.path().display()
    );

    let hi = conn.hello(&declared);
    assert_eq!(hi["ok"], json!(true), "symlinked declaration ok: {hi}");

    let bound = hi["body"]["workspace"]
        .as_str()
        .expect("hello names the bound root");

    // The answer names the canonical root...
    assert_eq!(
        Path::new(bound),
        canonical,
        "the answer names the canonicalized bound root: {hi}"
    );
    // ...which is not the string the caller declared.
    assert_ne!(
        Path::new(bound),
        declared.as_path(),
        "the bound root differs from the declared spelling — the field is \
         load-bearing, not decorative: {hi}"
    );

    // Declaring the real path is the same workspace: one identity, one entry.
    let again = conn.hello(&real);
    assert_eq!(
        again["body"]["workspace"].as_str(),
        Some(bound),
        "both spellings name one identity: {again}"
    );
    let listed = conn.call(&json!({"op": "list"}));
    assert_eq!(
        listed["entries"].as_array().map(Vec::len),
        Some(1),
        "two spellings of one tree register once: {listed}"
    );

    server.shutdown();
}

/// The ceiling-escape gate: a declared path that looks ordinary but resolves
/// into a denied root must refuse. Pins both symlink-safe layers at once
/// (`Registry::register` and `workspace::deny_reason` each canonicalize): the
/// ceiling is about the resolved tree, not the spelling.
#[test]
fn a_declaration_whose_canonical_target_is_denied_refuses() {
    let tmp = TempDir::new().unwrap();
    // `/tmp` is a denied root (DenyReason::TempDir), denied by its canonical
    // form, so this holds wherever /tmp resolves to.
    let escape = tmp.path().join("escape");
    symlink("/tmp", &escape).unwrap();
    let ordinary = write_ws(&tmp, "ordinary", &[("a.md", "# A\n")]);

    // Precondition: a sibling real directory in the same temp root binds fine,
    // so the refusal below cannot be "everything under the temp root is denied".
    assert_eq!(
        workspace::deny_reason(&ordinary),
        None,
        "MIS-CONSTRUCTED FIXTURE: an ordinary sibling must be allowed, or the \
         refusal proves nothing about symlink escape. Built under {}",
        tmp.path().display()
    );
    assert!(
        workspace::deny_reason(&escape).is_some(),
        "MIS-CONSTRUCTED FIXTURE: the escaping link must be denied. Built \
         under {}",
        tmp.path().display()
    );
    // Denied for its target, not its own spelling.
    assert_ne!(
        fs::canonicalize(&escape).unwrap(),
        escape.as_path(),
        "MIS-CONSTRUCTED FIXTURE: the link must actually resolve elsewhere. \
         Built under {}",
        tmp.path().display()
    );

    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());

    let refused = conn.hello(&escape);
    assert_eq!(
        refused["ok"],
        json!(false),
        "a declaration resolving into the deny ceiling refuses: {refused}"
    );
    assert_eq!(
        refused["error"]["code"],
        json!("bad_request"),
        "the refusal is bad_request: {refused}"
    );
    assert!(
        refused["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("deny ceiling")),
        "the refusal names its cause: {refused}"
    );

    // Nothing bound, nothing registered.
    let listed = conn.call(&json!({"op": "list"}));
    assert_eq!(
        listed["entries"].as_array().map(Vec::len),
        Some(0),
        "a refused declaration registers nothing: {listed}"
    );

    server.shutdown();
}

/// The same property through a second mechanism: on a case-insensitive
/// filesystem a case-variant declaration binds the on-disk casing, and the
/// answer names it. Skipped on a case-sensitive filesystem, where the variant
/// is genuinely a different path.
#[test]
fn a_declared_case_variant_binds_the_on_disk_casing_and_the_answer_names_it() {
    let tmp = TempDir::new().unwrap();
    let real = write_ws(&tmp, "MixedCase", &[("a.md", "# A\n")]);
    let variant = tmp.path().join("mixedcase");
    if !fs::canonicalize(&variant).is_ok_and(|c| c == fs::canonicalize(&real).unwrap()) {
        eprintln!("note: filesystem is case-sensitive; case-variant test skipped");
        return;
    }

    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());

    let hi = conn.hello(&variant);
    let bound = hi["body"]["workspace"]
        .as_str()
        .expect("hello names the bound root");
    assert_eq!(
        Path::new(bound).file_name().unwrap(),
        "MixedCase",
        "the answer names the ON-DISK casing, not the declared spelling: {hi}"
    );

    server.shutdown();
}

/// A workspace-less `hello` is a pure version handshake: it negotiates the rev
/// and lists caps, but binds and pins nothing — no `storage`, no `root`, and a
/// following read op is refused for want of a binding.
#[test]
fn workspace_less_hello_is_a_pure_version_handshake() {
    let tmp = TempDir::new().unwrap();
    write_ws(&tmp, "ws", &[("a.md", "# A\n")]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());

    let hi = conn.call(&json!({"op": "hello", "proto": 1, "contract": "v3"}));
    assert_eq!(hi["ok"], json!(true), "bare hello ok: {hi}");
    assert_eq!(hi["body"]["proto"], json!(1));
    assert!(
        !hi["body"]["caps"].as_array().unwrap().is_empty(),
        "a bare hello still lists caps: {hi}"
    );
    assert!(
        hi["body"]["storage"].is_null(),
        "a workspace-less hello pins nothing: {hi}"
    );
    // The `None` arm: absent, never an empty string — "nothing bound" and
    // "bound the empty path" must not look alike.
    assert!(
        hi["body"]["workspace"].is_null(),
        "a workspace-less hello names no bound root: {hi}"
    );
    assert!(
        !hi["body"].as_object().unwrap().contains_key("workspace"),
        "the field is omitted entirely, not serialized as null/empty: {hi}"
    );
    // v3 session: the binding cursor is spelled `fingerprint`, and it is absent
    // (nothing bound) — never `root`.
    assert!(
        hi["body"]["fingerprint"].is_null(),
        "a workspace-less hello binds nothing: {hi}"
    );
    assert!(
        hi["body"]["root"].is_null(),
        "a v3 hello never spells `root`: {hi}"
    );

    // Nothing bound → a read op is refused loudly.
    let toc = conn.call(&json!({"op": "toc", "path": "a.md"}));
    assert_eq!(toc["ok"], json!(false), "unbound read is refused: {toc}");

    // And it registered nothing.
    let listed = conn.call(&json!({"op": "list"}));
    assert_eq!(
        listed["entries"].as_array().map(Vec::len),
        Some(0),
        "a bare hello registers no workspace: {listed}"
    );

    server.shutdown();
}

// ---------------------------------------------------------------------------
// R1 — `hello.identity`: the build the daemon was made from
// (`docs/wire-contract.md`)
// ---------------------------------------------------------------------------

/// The configured sha reaches the wire as `identity.build`, and the field is
/// advertised as `hello.identity`. The value is carried verbatim: a client's
/// check is strict equality against the sha it deployed, so any normalization
/// here would silently defeat it.
#[test]
fn a_v3_hello_carries_the_configured_build_sha_and_advertises_the_field() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, "ws", &[("a.md", "# A\n")]);
    let server =
        RunningServer::start(test_config_with_build(&tmp, Some("6c4b1f0adeadbeef"))).unwrap();
    let mut conn = Conn::open(server.socket_path());

    let hi = conn.hello(&ws);
    assert_eq!(hi["ok"], json!(true), "hello ok: {hi}");
    assert_eq!(
        hi["body"]["identity"],
        json!({"build": "6c4b1f0adeadbeef"}),
        "the identity object carries exactly `build`, verbatim: {hi}"
    );

    let caps: Vec<&str> = hi["body"]["caps"]
        .as_array()
        .expect("caps array")
        .iter()
        .map(|c| c.as_str().unwrap())
        .collect();
    assert!(
        caps.contains(&"hello.identity"),
        "the v3 projection advertises the field amendment at dotted grain: {caps:?}"
    );
    assert!(
        !caps.contains(&"hello"),
        "the bare op cap stays absent — hello is the discovery door, not a \
         capability a client asks for: {caps:?}"
    );

    server.shutdown();
}

/// The build script's `unknown` fallback survives to the wire unchanged —
/// never mapped to null, never dropped: "this host cannot name its build" and
/// "this host publishes no build identity" must stay distinguishable.
#[test]
fn the_unknown_build_fallback_reaches_the_wire_verbatim() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, "ws", &[("a.md", "# A\n")]);
    let server = RunningServer::start(test_config_with_build(&tmp, Some("unknown"))).unwrap();
    let mut conn = Conn::open(server.socket_path());

    let hi = conn.hello(&ws);
    assert_eq!(
        hi["body"]["identity"]["build"],
        json!("unknown"),
        "`unknown` is a published identity, carried as the literal string: {hi}"
    );
    assert!(
        hi["body"]["identity"].is_object(),
        "the published-unknown case is still an object, never null: {hi}"
    );

    server.shutdown();
}

/// Optionality is real, not nominal: a daemon given no sha omits the field
/// entirely, and a v3 client's session is otherwise unaffected — it binds,
/// lists caps, and serves reads exactly as it does with an identity.
#[test]
fn a_daemon_with_no_configured_sha_omits_the_field_and_the_session_is_unaffected() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, "ws", &[("a.md", "# A\n")]);
    let server = RunningServer::start(test_config_with_build(&tmp, None)).unwrap();
    let mut conn = Conn::open(server.socket_path());

    let hi = conn.hello(&ws);
    assert_eq!(hi["ok"], json!(true), "hello ok without an identity: {hi}");
    assert!(
        !hi["body"].as_object().unwrap().contains_key("identity"),
        "the key is omitted entirely, never serialized as null: {hi}"
    );

    // The rest of the handshake is untouched by the absence.
    assert_eq!(hi["body"]["proto"], json!(1));
    assert!(
        hi["body"]["workspace"].is_string(),
        "the bind still names its root: {hi}"
    );
    let toc = conn.call(&json!({"op": "toc", "path": "a.md"}));
    assert_eq!(
        toc["ok"],
        json!(true),
        "a v3 client accepts an identity-less hello and reads on: {toc}"
    );

    server.shutdown();
}

/// The v2 freeze: a v2 session carries neither the field nor the cap, even from
/// a daemon that has a sha configured. The amendment is v3-only, so a v2
/// client's hello body is the one it always got.
#[test]
fn a_v2_session_carries_neither_the_identity_field_nor_its_cap() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, "ws", &[("a.md", "# A\n")]);
    let server =
        RunningServer::start(test_config_with_build(&tmp, Some("6c4b1f0adeadbeef"))).unwrap();
    let mut conn = Conn::open(server.socket_path());

    let hi = conn.call(&json!({
        "op": "hello",
        "proto": 1,
        "workspace": ws.to_str().unwrap(),
    }));
    assert_eq!(hi["ok"], json!(true), "v2 hello ok: {hi}");
    assert!(
        !hi["body"].as_object().unwrap().contains_key("identity"),
        "a v2 hello body never grows the key: {hi}"
    );
    let caps: Vec<&str> = hi["body"]["caps"]
        .as_array()
        .expect("caps array")
        .iter()
        .map(|c| c.as_str().unwrap())
        .collect();
    assert!(
        !caps.contains(&"hello.identity"),
        "a v2 session never advertises the v3 field amendment: {caps:?}"
    );

    server.shutdown();
}
