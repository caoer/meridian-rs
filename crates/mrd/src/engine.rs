//! The engine-client path (decision 0002 §3, P1): answer an engine read op by
//! dialing the resident daemon — auto-spawning it on first use (the watchman
//! model) — and degrading to an in-process ephemeral engine when the daemon is
//! unavailable.
//!
//! # The degrade is the universal catch
//! A run NEVER fails for want of a daemon. Every daemon-path failure — no cache
//! root, a spawn that cannot launch or never binds, a dropped connection, a
//! refused handshake, or a daemon-side op error — falls through to
//! [`in_process_links`], which builds the corpus from disk with U1's builder
//! ([`fs::build_corpus`]) and answers through the SAME shared read arm the
//! daemon serves ([`wire_serve::read::links`]), then applies the SAME v3
//! vocabulary projection ([`wire_serve::rev`]) the daemon applies for a
//! `contract:v3` session. One read projection + one rev projection, two state
//! sources, so the warm answer and the degrade answer never drift (both speak
//! `fingerprint`); only the reported [`EngineSource`] differs.

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

/// An engine read answer: the wire success body plus which path produced it.
pub(crate) struct Answer {
    /// Where the answer came from (warm daemon or in-process degrade).
    pub(crate) source: EngineSource,
    /// The wire success body (a `ResponseBody::Links` object), as JSON.
    pub(crate) body: Value,
}

/// Run `mrd links [PATH]`: resolve the workspace for the cwd, answer the corpus
/// edge map (whole-corpus, or one workspace-relative `PATH`) from the resident
/// daemon or the in-process degrade, and print it in the house grammar.
///
/// # Errors
/// The cwd or workspace cannot be resolved, or the degrade path hits a genuine
/// corpus error (see [`answer_links`]).
pub(crate) fn run_command(path_arg: Option<&str>, format: Format) -> Result<(), Fail> {
    let cwd = current_dir()?;
    let resolved = crate::resolve::resolve_runtime(&cwd).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace for {}: {e}",
            cwd.display()
        ))
    })?;
    let answer = answer_links(&resolved.workspace, path_arg)?;

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
    // **Q3(a) — the asymmetry, stated rather than smoothed.** An ambient
    // dangling link stays first-class and non-refusing at exit 0: it is an
    // ordinary authoring state in a working vault. A REFUSED edge is a
    // different fact — the author believed a mount relationship that does not
    // hold, or wrote something outside the address grammar — and the exit code
    // says so. The refusals are already rendered above; this only carries the
    // finding into the exit triad, the same way a red walk row does.
    let refusals = refusal_messages(&answer.body);
    if refusals.is_empty() {
        return Ok(());
    }
    Err(Fail::findings(refusals.join("\n")))
}

/// How many times the page wrote a refused linkpath.
fn count_of(refusal: &Value) -> u64 {
    refusal.get("count").and_then(Value::as_u64).unwrap_or(1)
}

/// Every refusal the edge map carries, as the teaching text a reader acts on —
/// empty when the answer refused nothing.
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
        // A cross-root destination is printed ROOT-QUALIFIED, and that is not
        // cosmetic: the ambient corpus may hold its own file at the same path,
        // so an unqualified name would read as the wrong document.
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

/// How long to wait for an auto-spawned daemon to bind its socket before
/// degrading. Generous: a cold daemon binds in milliseconds, so this only bounds
/// the pathological "launched but never came up" case.
const SPAWN_READY_TIMEOUT: Duration = Duration::from_secs(5);
/// Poll granularity while waiting for the socket to answer a ping.
const PING_POLL: Duration = Duration::from_millis(25);

/// Answer `links` for `workspace` (optional workspace-relative `path`): dial the
/// resident daemon — auto-spawning it — and on any daemon-path failure degrade
/// to the in-process ephemeral engine.
///
/// # Errors
/// Only a genuine corpus failure in the DEGRADE path surfaces (the workspace is
/// gone, unreadable, or holds a non-UTF-8 file, or the requested `path` is not
/// in the corpus) — the same error the daemon would raise. A missing daemon is
/// never an error.
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

/// **The U21 degrade gate: does this answer depend on a question the daemon
/// cannot ask?**
///
/// The daemon's warm state is ONE workspace corpus and NO mount authority
/// (`query::links_with`), so it reports every rooted spelling `unresolved` —
/// which for a bound, readable, present target is a wrong answer, and for an
/// unbound one is a silent non-refusal. The in-process path holds the mount
/// table and the mounted corpora, so it can answer both correctly. This decides
/// which of the two the user gets.
///
/// **It gates on the ANSWER, not on a pre-flight scan of the corpus, and the
/// coverage is exact rather than heuristic.** A spelling reaches the daemon's
/// `unresolved` map precisely when `resolve_linkpath` returned `None`, and that
/// function's C-3 guard IS [`addr::head_carries_root_separator`] — the same
/// function, not a second predicate that agrees. So every spelling this gate
/// could care about is already sitting in `unresolved`, by construction, and an
/// ordinary single-root corpus pays one map walk and never degrades.
///
/// **Why the LEXICAL predicate and not `may_carry_cross_root`.** That one gates
/// on `Addr::parse` succeeding, which is right for the question it asks — a
/// WRITE-side question, *"does this address have a stored form?"*, and an
/// unparseable address has none. Ours is a READ-side question, *"could the
/// address plane change this answer?"*, and a REFUSAL is a changed answer. Five
/// spellings slip the parse-gated form — `Sessions:notes.md` (uppercase root),
/// `My Notes:draft.md`, `a:b:c.md`, `:notes.md`, `sessions:` — and each would
/// then be served warm as `unresolved` at exit 0 while in-process refuses it.
/// One address, two answers, decided by which path served it: the C-4 defect
/// the address grammar exists to prevent, rebuilt inside the fix for it.
fn daemon_answer_needs_the_address_plane(body: &Value) -> bool {
    body.get("files")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(_, edges)| edges.get("unresolved").and_then(Value::as_object))
        .flat_map(serde_json::Map::keys)
        .any(|link| addr::head_carries_root_separator(link))
}

/// Try the whole daemon path: resolve the socket, ensure a daemon is up
/// (auto-spawn if not), then dial `hello` + `links`. `None` on ANY failure — the
/// caller degrades. `Some(body)` only when the daemon answered `ok:true`.
fn try_daemon_links(workspace: &Path, path: Option<&str>) -> Option<Value> {
    let client = Client::from_default().ok()?;
    ensure_daemon(&client).ok()?;
    dial_links(client.socket_path(), workspace, path)
        .ok()
        .flatten()
}

/// Ensure a daemon answers on `client`'s socket: return early if one already
/// pings, else auto-spawn it detached and poll until it binds or the timeout
/// elapses.
///
/// # Errors
/// The daemon could not be spawned (spawn-impossible), or it was spawned but
/// never became ready within [`SPAWN_READY_TIMEOUT`].
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

/// Dial one connection: `hello` binds and warms `workspace` (one round trip),
/// then `links` reads from that binding. `Ok(Some(body))` when the daemon
/// answered `ok:true`; `Ok(None)` when it answered an op error (degrade to the
/// authoritative in-process answer); `Err` on a transport failure.
fn dial_links(socket: &Path, workspace: &Path, path: Option<&str>) -> io::Result<Option<Value>> {
    let stream = UnixStream::connect(socket)?;
    let mut writer = stream.try_clone()?;
    let mut reader = BufReader::new(stream);

    // `hello` with a `workspace` resolves + pins + warms the resident engine and
    // binds this connection to it (decision 0002 §4). A failed handshake (deny
    // ceiling, unknown rev) degrades — the in-process answer is authoritative.
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

/// Map an engine refusal to the CLI exit triad (shared by `read`/`put`):
/// `bad_request` is a bad invocation (exit 2); every other refusal is a
/// finding (exit 1). When the engine minted a `message`, it is printed
/// VERBATIM — the refusal strings are golden-pinned (U0), never reworded —
/// and the extras that message NAMES are printed under it ([`extras`]), so a
/// terminal reader can follow the instruction it was given. A message-less
/// refusal is spelled here, in the house pattern: the failure, the
/// nothing-happened clause, one fix line.
pub(crate) fn refusal_fail(error: &ErrorBody) -> Fail {
    let mut text = match &error.message {
        Some(message) => message.clone(),
        None => spelled(error),
    };
    text.push_str(&extras(error));
    if error.code == wire::ErrorCode::BadRequest {
        Fail::tool(text)
    } else {
        Fail::findings(text)
    }
}

/// A message-less refusal, written the way the good refusals in this tree are
/// written: name the failure, say what did NOT happen, give the fix.
///
/// `root_mismatch` is the one that reaches a person often — the world-grain
/// guard on a write — so it gets the full pattern with its own recovery
/// command. Any other message-less code falls back to the code plus the §8
/// comparison tokens; inventing a fix line for a refusal we cannot name would
/// teach a recovery that may not exist.
fn spelled(error: &ErrorBody) -> String {
    let code = serde_json::to_value(error.code)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "error".to_owned());
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
    match (&error.expected, &error.actual, &error.path) {
        (Some(expected), Some(actual), _) => {
            format!("{code}: expected {}, actual {}", expected.0, actual.0)
        }
        (_, _, Some(path)) => format!("{code}: {}", path.0),
        _ => code,
    }
}

/// The refusal's own extras, rendered for a TERMINAL rather than a wire client.
///
/// A `cas_mismatch` refusal tells the caller to apply the `diff` extra and
/// resend with `new_fingerprint`; on the human face those are wire response
/// fields nobody can see, so the instruction was unfollowable exactly where it
/// was offered. Printing them under the message is what makes the ladder's
/// no-re-read shortcut reachable from a shell.
///
/// The diff and the new content ride UNINDENTED beneath their headers — both
/// are meant to be copied or piped, and an indent would corrupt them.
///
/// Every branch here is reachable from a terminal, and that is a standing
/// requirement rather than an observation: a refusal no input can produce is
/// dead code with a green test guarding it. `ErrorBody::changed` was printed
/// here until it was checked — nothing in the write path ever sets it, and
/// `registry::root_mismatch_wire_shape` pins a served `root_mismatch` to
/// `expected` + `actual` with NO `changed` — so that line was removed rather
/// than left as prose about a field the engine does not send.
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
        let _ = write!(out, "\n  diff (apply this to your copy):\n{}", diff.trim_end());
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

/// One NDJSON round trip on an open connection: write the request line, read one
/// response line, parse it.
///
/// # Errors
/// The write fails, the daemon closes without a response, or the response line
/// is not valid JSON.
pub(crate) fn call(
    writer: &mut UnixStream,
    reader: &mut BufReader<UnixStream>,
    request: &Value,
) -> io::Result<Value> {
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
    serde_json::from_str(&response).map_err(io::Error::other)
}

/// The degrade: build the corpus in-process with U1's builder and answer through
/// the shared `links` read arm — the SAME read projection the daemon serves — then
/// re-key it to the v3 vocabulary the CLI negotiated ([`dial_links`] sends
/// `contract:v3`), so the answer is byte-identical to the warm one for the same
/// corpus (`fingerprint`, never `root`).
///
/// # Errors
/// The workspace cannot be canonicalized (gone), the corpus cannot be read, a
/// corpus file is non-UTF-8, or the wire read arm refuses (e.g. the requested
/// `path` is not in the corpus).
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
    let (index, docs) =
        fs::build_corpus(files).map_err(|e| Fail::tool(format!("cannot build the corpus: {e}")))?;

    let as_of = Root(fingerprint.0);
    let wpath = path.map(|p| WirePath(p.to_owned()));
    // `live_root` samples after the read; for a single in-process snapshot the
    // world does not move, so it equals `as_of` (a legal §10.1 frame).
    let live = as_of.clone();
    // **The whole reason this path exists after U21.** The mount table and one
    // corpus per bound root come from `walk_cmd::load_mounts` — the SAME
    // assembly the pin plane uses (S3-R59, one owner), so a cross-root address
    // resolves through the same `resolve_ref` on both planes and cannot answer
    // two ways.
    let mounts = crate::walk_cmd::load_mounts();
    let corpus = mounts.rooted(&docs);
    let body = wire_serve::read::links_rooted(
        &index,
        &docs,
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
    // The CLI negotiated `contract:v3` on the warm path, so the degrade answer
    // must speak the same vocabulary — run the SAME lifted projection the daemon
    // runs (`root` → `fingerprint`) so warm and degrade never drift. The
    // projection re-keys under `body`, so wrap, project, and unwrap.
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
        (Some(message), _) => format!("{code}: {message}"),
        (None, Some(path)) => format!("{code}: {}", path.0),
        (None, None) => code,
    }
}
