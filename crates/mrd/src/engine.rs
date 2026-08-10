//! The engine-client path: answer an engine read op by dialing the resident daemon —
//! auto-spawning it on first use — and degrading to an in-process ephemeral engine when the
//! daemon is unavailable. A run never fails for want of a daemon.

use std::fmt::Write as _;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

use registry::Client;
use serde_json::{Value, json};
use wire::{ErrorBody, Path as WirePath, Root};

use crate::{Fail, Format, current_dir, daemon};

/// Which path produced an engine answer — mirrors `resolve`'s `source` label so
/// the caller can report warm-vs-degrade in the same house grammar.
pub(crate) enum EngineSource {
    /// Served from the resident daemon's warm engine.
    Daemon,
    /// Served in-process from an ephemeral engine (the daemon was unavailable).
    Ephemeral,
}

impl EngineSource {
    /// A stable lowercase label for JSON / human output.
    pub(crate) fn label(&self) -> &'static str {
        match self {
            EngineSource::Daemon => "daemon",
            EngineSource::Ephemeral => "ephemeral",
        }
    }
}

/// The `sockaddr_un.sun_path` capacity, NUL terminator included: 108 on Linux,
/// 104 on the BSDs (macOS). A socket path that does not fit cannot be bound OR
/// dialled, so the daemon is unreachable however healthy it is — and the
/// degrade catches that the same way it catches every other daemon failure.
#[cfg(target_os = "linux")]
const SUN_PATH_CAPACITY: usize = 108;
#[cfg(not(target_os = "linux"))]
const SUN_PATH_CAPACITY: usize = 104;

/// Voice the degrade on a face a person reads — nothing on the warm path.
///
/// Goes to stderr: stdout must stay byte-identical between the warm and degraded paths, since
/// that output is piped and diffed.
pub(crate) fn voice_degrade(source: &EngineSource) {
    if !matches!(source, EngineSource::Ephemeral) {
        return;
    }
    eprintln!(
        "mrd: source: ephemeral — no daemon answered, so this answer came from the daemonless \
         path (an in-process build, or a cold last-published file), not the warm engine."
    );
    eprintln!(
        "mrd:   The content is what a warm daemon serves; the TIMING is not — do not measure \
         this run."
    );
    if let Some(reason) = degrade_reason() {
        eprintln!("mrd:   {reason}");
    }
}

/// The one degrade cause worth naming beyond "no daemon answered": a socket path that cannot
/// fit in `sun_path`. It is silent, it is not fixed by starting a daemon, and it is reached by
/// an ordinary long `XDG_CACHE_HOME`. Every other cause (not running, spawn failed, refused
/// handshake) is already covered by the first line, so this says nothing rather than guessing.
pub(crate) fn degrade_reason() -> Option<String> {
    let Ok(client) = Client::from_default() else {
        return Some("No cache root resolves, so there is no socket path to dial.".to_owned());
    };
    let socket = client.socket_path();
    let len = socket.as_os_str().as_encoded_bytes().len();
    if len < SUN_PATH_CAPACITY {
        return None;
    }
    Some(format!(
        "The socket path is {len} bytes, at or over this platform's {SUN_PATH_CAPACITY}-byte \
         sun_path limit, so NO daemon can bind or dial it. Shorten the cache root \
         (XDG_CACHE_HOME): {}",
        socket.display()
    ))
}

/// An engine read answer: the wire success body plus which path produced it.
pub(crate) struct Answer {
    /// Where the answer came from (warm daemon or in-process degrade).
    pub(crate) source: EngineSource,
    /// The wire success body (a `ResponseBody::Links` object), as JSON.
    pub(crate) body: Value,
}

/// Run `mrd links [PATH]`: resolve the workspace for the cwd, answer the corpus edge map
/// (whole-corpus, or one workspace-relative `PATH`) from the resident daemon or the in-process
/// degrade, and print it in the house grammar. Errors The cwd or workspace cannot be resolved,
/// or the degrade path hits a genuine corpus error (see [`answer_links`]).
pub(crate) fn run_command(path_arg: Option<&str>, format: Format) -> Result<(), Fail> {
    let cwd = current_dir()?;
    let resolved = crate::resolve::resolve_runtime(&cwd).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace for {}: {e}",
            cwd.display()
        ))
    })?;
    let answer = answer_links(&resolved.workspace, path_arg)?;
    // Read off the ANSWER, so warm and degrade voice one fact from one source:
    // an enumeration names the population it did not carry (§4.6 `excluded`).
    voice_excluded(&answer.body);

    match format {
        Format::Json => {
            let value = json!({
                "workspace": resolved.workspace.display().to_string(),
                "source": answer.source.label(),
                "links": answer.body,
            });
            println!("{}", serde_json::to_string_pretty(&value).expect("json"));
        }
        Format::Human => {
            println!("workspace {}", resolved.workspace.display());
            println!("  source: {}", answer.source.label());
            render_links_human(&answer.body);
        }
    }
    // A dangling ambient link stays non-refusing at exit 0 — ordinary authoring state. A REFUSED
    // edge is a different fact (a mount relationship that does not hold, or an address outside the
    // grammar), so it carries into the exit triad.
    let refusals = refusal_messages(&answer.body);
    if refusals.is_empty() {
        return Ok(());
    }
    Err(Fail::findings(refusals.join("\n")))
}

/// Voice the enumeration's domain-excluded population on stderr, so machine
/// stdout stays byte-identical to what the wire carried.
///
/// The list rides the answer (§4.6 `excluded`), never a second disk walk here:
/// a face that re-derived it could disagree with the door that served it, which
/// is the door/face split this rule exists to close. Empty on the named form —
/// a named path is served, so nothing was left out (§12.1).
fn voice_excluded(body: &Value) {
    let names: Vec<&str> = body
        .get("excluded")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    if names.is_empty() {
        return;
    }
    eprintln!(
        "mrd: note: {} markdown file(s) under this root are outside the hash domain and are NOT in this listing — {}. They stay addressable by explicit path (`mrd read`, `mrd links <PATH>`); their bytes do not move the fingerprint this answer is stamped with.",
        names.len(),
        names.join(", ")
    );
}

/// How many times the page wrote a refused linkpath.
fn count_of(refusal: &Value) -> u64 {
    refusal.get("count").and_then(Value::as_u64).unwrap_or(1)
}

/// Every refusal message the edge map carries — empty when the answer refused nothing.
fn refusal_messages(body: &Value) -> Vec<String> {
    body.get("files")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(_, edges)| edges.get("refused").and_then(Value::as_object))
        .flat_map(serde_json::Map::values)
        .filter_map(|r| r.get("message").and_then(Value::as_str))
        .map(str::to_owned)
        .collect()
}

/// Print the `links` edge map as an indented human list: each file with a
/// non-empty edge set, its resolved destinations, then its unresolved linkpaths.
fn render_links_human(body: &Value) {
    let Some(files) = body.get("files").and_then(Value::as_object) else {
        return;
    };
    let mut any = false;
    for (file, edges) in files {
        let resolved = edges.get("resolved").and_then(Value::as_object);
        let unresolved = edges.get("unresolved").and_then(Value::as_object);
        let rooted = edges.get("resolved_rooted").and_then(Value::as_object);
        let refused = edges.get("refused").and_then(Value::as_object);
        let empty =
            |m: Option<&serde_json::Map<String, Value>>| m.is_none_or(serde_json::Map::is_empty);
        if empty(resolved) && empty(unresolved) && empty(rooted) && empty(refused) {
            continue;
        }
        any = true;
        println!("  {file}");
        for (dest, count) in resolved.into_iter().flatten() {
            println!("    -> {dest} ({count})");
        }
        // Cross-root destinations print root-qualified: the ambient corpus may hold its own file
        // at the same path, so an unqualified name would read as the wrong document.
        for (root, paths) in rooted.into_iter().flatten() {
            for (dest, count) in paths.as_object().into_iter().flatten() {
                println!("    -> {root}:{dest} ({count})");
            }
        }
        for (link, count) in unresolved.into_iter().flatten() {
            println!("    -> {link} ({count}, unresolved)");
        }
        for (link, refusal) in refused.into_iter().flatten() {
            let tone = refusal
                .get("color")
                .and_then(Value::as_str)
                .unwrap_or("red");
            let reason = refusal
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let count = count_of(refusal);
            println!("    -> {link} ({count}, {tone} {reason})");
        }
    }
    if !any {
        println!("  (no outgoing links)");
    }
}

/// How long to wait for an auto-spawned daemon to bind its socket before degrading. Generous: a
/// cold daemon binds in milliseconds, so this only bounds the pathological "launched but never
/// came up" case.
const SPAWN_READY_TIMEOUT: Duration = Duration::from_secs(5);
/// Poll granularity while waiting for the socket to answer a ping.
const PING_POLL: Duration = Duration::from_millis(25);

/// Answer `links` for `workspace` (optional workspace-relative `path`): dial the resident
/// daemon — auto-spawning it — and on any daemon-path failure degrade to the in-process
/// ephemeral engine.
pub(crate) fn answer_links(workspace: &Path, path: Option<&str>) -> Result<Answer, Fail> {
    if let Some(body) = try_daemon_links(workspace, path)
        && !daemon_answer_needs_the_address_plane(&body)
    {
        return Ok(Answer {
            source: EngineSource::Daemon,
            body,
        });
    }
    let body = in_process_links(workspace, path)?;
    Ok(Answer {
        source: EngineSource::Ephemeral,
        body,
    })
}

/// Does this answer depend on a question the daemon cannot ask? The daemon's warm state is one
/// workspace corpus with no mount authority, so it reports every rooted spelling `unresolved` —
/// wrong for a bound target, a silent non-refusal for an unbound one. The in-process path holds
/// the mount table, so it answers both correctly. Gates on the answer, not a pre-flight scan.
fn daemon_answer_needs_the_address_plane(body: &Value) -> bool {
    body.get("files")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(_, edges)| edges.get("unresolved").and_then(Value::as_object))
        .flat_map(serde_json::Map::keys)
        .any(|link| addr::head_carries_root_separator(link))
}

/// Try the whole daemon path: resolve the socket, ensure a daemon is up (auto-spawn if not),
/// then dial `hello` + `links`. `None` on ANY failure — the caller degrades. `Some(body)` only
/// when the daemon answered `ok:true`.
fn try_daemon_links(workspace: &Path, path: Option<&str>) -> Option<Value> {
    let client = Client::from_default().ok()?;
    ensure_daemon(&client).ok()?;
    dial_links(client.socket_path(), workspace, path)
        .ok()
        .flatten()
}

/// Ensure a daemon answers on `client`'s socket: return early if one already pings, else
/// auto-spawn it detached and poll until it binds or the timeout elapses. Errors The daemon
/// could not be spawned (spawn-impossible), or it was spawned but never became ready within
/// [`SPAWN_READY_TIMEOUT`].
pub(crate) fn ensure_daemon(client: &Client) -> io::Result<()> {
    if client.ping().unwrap_or(false) {
        return Ok(());
    }
    daemon::spawn_detached()?;
    let deadline = Instant::now() + SPAWN_READY_TIMEOUT;
    while Instant::now() < deadline {
        if client.ping().unwrap_or(false) {
            return Ok(());
        }
        std::thread::sleep(PING_POLL);
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "auto-spawned daemon did not become ready",
    ))
}

/// Dial one connection: `hello` binds and warms `workspace` (one round trip), then `links`
/// reads from that binding. `Ok(Some(body))` when the daemon answered `ok:true`; `Ok(None)`
/// when it answered an op error (degrade to the authoritative in-process answer); `Err` on a
/// transport failure.
fn dial_links(socket: &Path, workspace: &Path, path: Option<&str>) -> io::Result<Option<Value>> {
    let stream = UnixStream::connect(socket)?;
    let mut writer = stream.try_clone()?;
    let mut reader = BufReader::new(stream);

    // `hello` with a `workspace` resolves, pins and warms the resident engine, binding this
    // connection to it. A failed handshake degrades — the in-process answer is authoritative.
    let hello = json!({
        "op": "hello",
        "proto": 1,
        "contract": "v3",
        "workspace": workspace.to_string_lossy(),
    });
    if call(&mut writer, &mut reader, &hello)?
        .get("ok")
        .and_then(Value::as_bool)
        != Some(true)
    {
        return Ok(None);
    }

    let mut links = json!({ "op": "links" });
    if let Some(p) = path {
        links["path"] = json!(p);
    }
    let response = call(&mut writer, &mut reader, &links)?;
    if response.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(response.get("body").cloned())
    } else {
        Ok(None)
    }
}

/// Map an engine refusal to the CLI exit triad (shared by `read`/`put`): EVERY engine refusal
/// is the findings leg (exit 1) — `bad_request` included, because a request the ENGINE judges
/// invalid (a §4.4 batch overlap, a multi-line upsert value) is the engine refusing a
/// well-formed invocation, not the CLI refusing the invocation itself. Exit 2 belongs to the
/// CLI's own refusals (flags, stdin), which are minted before any engine contact and never
/// pass through here (dogfood P3-b: the split is what lets a script tell "fix your command
/// line" from "read the engine's message").
///
/// PRIVATE ON PURPOSE. This helper mints the stderr half alone, and a verb that reached it
/// directly published a `--json` refusal with an EMPTY stdout — the defect
/// [`json_refusal`] was created to end and then left at `pin` and `retire mark`. Every
/// module outside this one converts an engine [`ErrorBody`] through [`json_refusal`], which
/// takes the [`Format`] and cannot skip the envelope. Privacy is the enforcement: a new
/// caller that tries to skip it does not compile.
fn refusal_fail(error: &ErrorBody) -> Fail {
    Fail::findings(refusal_text(error))
}

/// An engine refusal AS PROSE, with no exit code attached: the engine's own message where it
/// composed one, [`spelled`] where it did not, plus the [`extras`] a terminal cannot read off
/// the wire.
///
/// Separate from [`refusal_fail`] because the exit code is the CALLER's judgement and the
/// sentence is not. `mrd test --corpus` refuses at exit 2 — its spec declared an edit the engine
/// will not perform, which is a bad input rather than the engine refusing the caller's request —
/// and it still owes the operator the sentence this composes. It used to render the body by hand
/// as `{:?}: {message-or-empty}`, which spelled the code in Rust's `Debug` vocabulary
/// (`NoMatch`, where every other door says `no_match`) and printed NOTHING after the colon for
/// the message-less refusals that are the common case. One owner for the prose; the triad stays
/// with the verb.
pub(crate) fn refusal_text(error: &ErrorBody) -> String {
    let mut text = match &error.message {
        Some(message) => message.clone(),
        None => spelled(error),
    };
    text.push_str(&extras(error));
    text
}

/// The `--json` face's refusal envelope on stdout — `{workspace, error}`, the engine's §8 error
/// body lifted into the v3 vocabulary — beside the human stderr line [`refusal_fail`] returns.
///
/// The law is the FACE's, not one verb's (status.md § teaching rows): a machine consumer cannot
/// tell an absent frame from success with no output, so every leg of a `--json` face that can
/// refuse emits one. Human format prints nothing here and the exit triad is untouched.
pub(crate) fn json_refusal(format: Format, workspace: &Path, error: &ErrorBody) -> Fail {
    if matches!(format, Format::Json) {
        let mut frame = json!({
            "error": serde_json::to_value(error).expect("json"),
        });
        wire_serve::rev::project_response(&mut frame);
        let error_v3 = frame
            .as_object_mut()
            .and_then(|obj| obj.remove("error"))
            .unwrap_or(Value::Null);
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "workspace": workspace.display().to_string(),
                "error": error_v3,
            }))
            .expect("json")
        );
    }
    refusal_fail(error)
}

/// A message-less refusal, spelled out: name the failure, say what did not happen, give the fix.
fn spelled(error: &ErrorBody) -> String {
    let code = serde_json::to_value(error.code)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "error".to_owned());
    // §8 binds `io_error{cause}` and the engine composes real prose into it —
    // the ambiguous-domain refusal names both config files and which one to
    // delete. Rendering the bare token strands the caller at exactly the leg
    // whose remedy is least guessable. The cause is carried verbatim: it is the
    // measured one, and nothing is invented beside it.
    if error.code == wire::ErrorCode::IoError
        && let Some(cause) = &error.cause
    {
        return format!("io_error: {cause}");
    }
    if error.code == wire::ErrorCode::RootMismatch
        && let Some(actual) = &error.actual
    {
        return format!(
            "root_mismatch: the workspace fingerprint your write pinned is not this \
             workspace's current one. {} Fix: re-run with `--if-fingerprint {}` once you \
             have seen what moved, or drop the flag to write unguarded.",
            wire_serve::NO_PARTIAL_WRITE_CLAUSE,
            actual.0
        );
    }
    // The match-law pair (dogfood P3-a): the wire's `matches` count is the
    // whole machine face (§4.4 `not_unique{matches}`), so the human face
    // renders it into the law and its working fix instead of a bare token.
    if error.code == wire::ErrorCode::NoMatch {
        return format!(
            "no_match: `old` occurs {} times in the target — the §4.4 match law wants exactly \
             one occurrence, byte-exact against the current content. {} Fix: re-read the \
             target and copy `old` verbatim (whitespace included); an `if_node_rev`-guarded \
             miss is provably a typo, an unguarded one may be a moved world (§5.2).",
            error.matches.unwrap_or(0),
            wire_serve::NO_PARTIAL_WRITE_CLAUSE
        );
    }
    if error.code == wire::ErrorCode::NotUnique {
        return format!(
            "not_unique: `old` occurs {} times in the target — the §4.4 match law wants \
             exactly one occurrence. {} Fix: extend `old` with surrounding bytes until one \
             occurrence remains, or aim the target at a narrower section.",
            error.matches.unwrap_or(0),
            wire_serve::NO_PARTIAL_WRITE_CLAUSE
        );
    }
    // Dogfood NEW-B: contract §4.4 promises `would_corrupt{family,…}` — the
    // family and the measured cause ride the wire, so the human face names what
    // dies AND the remedy that repairs THIS batch. The remedy is never a fixed
    // string: a remedy for a cause the engine did not measure is a
    // taught-recovery loop, so an unmeasured cause teaches nothing.
    // The `target_identity` family is not spelled here: that refusal carries
    // its own `message` (wire-serve), which every face prefers over this one.
    if error.code == wire::ErrorCode::WouldCorrupt
        && let Some(lost) = &error.lost
    {
        let chains = lost
            .iter()
            .map(|chain| format!("`{}`", wire_serve::display_hpath(chain)))
            .collect::<Vec<_>>()
            .join(", ");
        let remedy = match error.cause.as_deref() {
            Some("heading_destroyed") => {
                " Fix: carry your own newlines — `at:\"end\"` is raw byte concatenation \
                 (§4.4), so inserted text that runs up against a following heading must \
                 end with `\\n` or the heading line glues onto it and stops parsing as a \
                 heading."
            }
            Some("reparented") => {
                " Fix: the text you wrote opens a heading shallow enough to adopt the \
                 sections after it — those headings still parse, their paths just moved. \
                 Deepen that heading's level so it nests under the target, or aim the \
                 edit at the parent whose whole subtree you meant to rewrite."
            }
            // Measured nothing shared: name the loss, teach no cause.
            _ => {
                " Fix: re-read the file and compare the section tree your text would \
                 produce against the one above — the lost paths do not share one cause, \
                 so no single remedy is safe to teach."
            }
        };
        return format!(
            "would_corrupt: committed, this batch would re-parse to a document that loses \
             containment — {chains} would fall out of the section tree. {}{remedy}",
            wire_serve::NO_PARTIAL_WRITE_CLAUSE
        );
    }
    match (&error.expected, &error.actual, &error.path) {
        (Some(expected), Some(actual), _) => {
            format!("{code}: expected {}, actual {}", expected.0, actual.0)
        }
        (_, _, Some(path)) => format!("{code}: {}", path.0),
        _ => code,
    }
}

/// The refusal's extras, rendered for a terminal rather than a wire client: a `cas_mismatch`
/// tells the caller to apply the `diff` extra and resend with `new_fingerprint`, which are wire
/// fields invisible on the human face. Printing them keeps the no-re-read shortcut reachable
/// from a shell.
fn extras(error: &ErrorBody) -> String {
    let mut out = String::new();
    if error.code == wire::ErrorCode::RootMismatch
        && let (Some(expected), Some(actual)) = (&error.expected, &error.actual)
    {
        let _ = write!(out, "\n  pinned:  {}\n  current: {}", expected.0, actual.0);
    }
    if let Some(fingerprint) = &error.new_fingerprint {
        let _ = write!(out, "\n  new_fingerprint: {}", fingerprint.0);
    }
    if let Some(diff) = &error.diff {
        let _ = write!(
            out,
            "\n  diff (apply this to your copy):\n{}",
            diff.trim_end()
        );
    }
    if let Some(content) = &error.new_content {
        let _ = write!(
            out,
            "\n  new_content (that node's current bytes):\n{}",
            content.trim_end()
        );
    }
    out
}

/// One NDJSON round trip on an open connection: write the request line, read one response line,
/// parse it. Errors The write fails, the daemon closes without a response, or the response line
/// is not valid JSON.
pub(crate) fn call(
    writer: &mut UnixStream,
    reader: &mut BufReader<UnixStream>,
    request: &Value,
) -> io::Result<Value> {
    let response = call_line(writer, reader, request)?;
    serde_json::from_str(&response).map_err(io::Error::other)
}

/// The same round trip, answering the response **line** instead of a parsed
/// value. The script entry embeds the splice response in its trace verbatim, and
/// a parse-then-reserialize would sort the object's keys and normalize its
/// whitespace — a second commit-fact shape. Callers that only read fields use
/// [`call`]. Errors The write fails, or the daemon closes without a response.
pub(crate) fn call_line(
    writer: &mut UnixStream,
    reader: &mut BufReader<UnixStream>,
    request: &Value,
) -> io::Result<String> {
    let mut line = serde_json::to_string(request).map_err(io::Error::other)?;
    line.push('\n');
    writer.write_all(line.as_bytes())?;
    writer.flush()?;

    let mut response = String::new();
    if reader.read_line(&mut response)? == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "daemon closed the connection without a response",
        ));
    }
    Ok(response)
}

/// The degrade: build the corpus in-process and answer through the same shared `links` read arm
/// the daemon serves, then re-key it to the v3 vocabulary the CLI negotiated ([`dial_links`]
/// sends `contract:v3`) so warm and degrade answers do not drift.
fn in_process_links(workspace: &Path, path: Option<&str>) -> Result<Value, Fail> {
    let canonical = workspace::canonicalize(workspace).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace {} ({e})",
            workspace.display()
        ))
    })?;
    let root = fs::WorkspaceRoot(canonical);
    let (files, fingerprint) = fs::domain_snapshot(&root)
        .map_err(|e| Fail::tool(format!("cannot read the corpus: {e}")))?;
    let (index, docs, unserved) = fs::build_corpus(files);
    crate::voice_unserved(&unserved);

    let as_of = Root(fingerprint.0);
    let wpath = path.map(|p| WirePath(p.to_owned()));
    // `live_root` samples after the read; for a single in-process snapshot the
    // world does not move, so it equals `as_of` (a legal §10.1 frame).
    let live = as_of.clone();
    // The mount table comes from the same loader the pin plane uses (`walk_cmd::load_mounts_for`),
    // so a cross-root address resolves through the same `resolve_ref` on both planes. Corpora
    // narrow to the roots this answer's link targets name; the table itself is never narrowed.
    // A full-table eager load cost ~27 s CPU on a workspace naming zero roots.
    let mounts =
        crate::walk_cmd::load_mounts_for(&crate::walk_cmd::link_addressed_roots(&docs, path));
    // Carried with the corpus for the same reason `walk` carries it: a face that
    // cannot name its filter cannot tell excluded from missing (§12.1).
    let domain = fs::domain::Domain::load(&root)
        .map_err(|e| Fail::tool(format!("cannot read the hash domain: {e}")))?;
    let corpus = mounts.rooted(&docs, &domain, &root);
    let body = wire_serve::read::links_rooted(
        &root,
        &index,
        &docs,
        &unserved,
        &corpus,
        mounts.set(),
        wpath.as_ref(),
        as_of,
        0,
        || Ok(live),
    )
    .map_err(|e| Fail::tool(render_wire_error(&e)))?;
    let body = serde_json::to_value(&body)
        .map_err(|e| Fail::tool(format!("cannot render the answer: {e}")))?;
    // Run the same lifted projection the daemon runs (`root` → `fingerprint`). It re-keys under
    // `body`, so wrap, project, and unwrap.
    let mut frame = json!({ "body": body });
    wire_serve::rev::project_response(&mut frame);
    Ok(frame
        .as_object_mut()
        .and_then(|obj| obj.remove("body"))
        .unwrap_or(Value::Null))
}

/// Render a wire error body as a one-line diagnostic (the code plus its message
/// or echoed path) for the CLI's stderr.
fn render_wire_error(error: &ErrorBody) -> String {
    let code = serde_json::to_value(error.code)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "error".to_owned());
    match (&error.message, &error.path) {
        // A typed message that already opens with its own code is not prefixed
        // twice: `file_not_found: file_not_found: no file at …` reads as two
        // refusals stacked, and the doubling appears the moment a door that
        // used to answer with a bare code starts carrying a teaching message.
        (Some(message), _) if message.starts_with(&format!("{code}:")) => message.clone(),
        (Some(message), _) => format!("{code}: {message}"),
        (None, Some(path)) => format!("{code}: {}", path.0),
        (None, None) => code,
    }
}
