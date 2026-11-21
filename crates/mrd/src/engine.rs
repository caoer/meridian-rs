//! The engine-client path: answer an engine read op by dialing the resident daemon —
//! auto-spawning it on first use — and degrading to an in-process ephemeral engine when no
//! daemon serves the workspace. A run never fails for want of a daemon.
//!
//! The degrade is for the ABSENT daemon only (`docs/wire-contract.md` § A.10 / § A.11, the
//! CLI ladder). A daemon that accepted the connection holds — or is building — the corpus
//! this process would build, so an exchange with it that does not complete is refused
//! ([`resident_lost`]), a `retry` refusal is re-asked on the held connection
//! ([`call_through_retries`]) and a socket closed by a daemon still in its stop is refused
//! ([`resident_stopping`]). An in-process build beside it would be paid by every caller at
//! once, exactly when the daemon is slow.

use std::fmt::Write as _;
use std::io::{self, BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use config::mount::DECLARATION_FILENAME;
use registry::{Client, Liveness};
use serde_json::{Value, json};
use wire::{ErrorBody, Path as WirePath, Root};

use crate::{Fail, Format, current_dir, daemon, placement};

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

/// This client's own build identity, baked by `build.rs` (G10): a whole-commit
/// sha, `<sha>-dirty`, or `unknown` (`docs/release.md` §5.1).
const OWN_BUILD: &str = env!("MRD_BUILD_SHA");

/// The socket law, client half (0025, 2026-08-12; `docs/wire-contract.md`
/// §A.3): a LOCAL client on its own cache root compares `hello.identity.build`
/// against its own baked identity, WHOLE token, and refuses on anything but
/// equality. Zero extra round trips — `body` is the hello frame the single
/// dial already received and parsed.
///
/// Absence refuses too, on this LOCAL socket only: a resident daemon that
/// publishes no identity is a build predating the identity token — exactly the
/// stale-resident shape the law exists for (receipt `839fdb38`: a foreign
/// daemon served wording that does not exist in the caller's tree, no error).
/// `hello.identity` stays optional ON THE WIRE; remote peers are not bound.
///
/// ONE voice for every lane (read, links, script host), minted here alone, in
/// the fleet's skew grammar (`child:… daemon:… SKEW`): both identities, the
/// verdict, the reason, and fitted suggestions.
///
/// The teaching register: explain WHY, then suggest
/// fixes each under its applicability condition — never demand one command,
/// because no single command applies to every caller (a caller who does not
/// own the resident must not kill it; a managed install owns its own restart).
/// Teachings address users, so conditions are applicability ("when you own…"),
/// never authority ("only the owner may…").
pub(crate) fn hello_identity_skew(body: Option<&Value>, socket: &Path) -> Result<(), String> {
    let theirs = body
        .and_then(|b| b.get("identity"))
        .and_then(|i| i.get("build"))
        .and_then(Value::as_str);
    if theirs == Some(OWN_BUILD) {
        return Ok(());
    }
    let daemon = theirs.map_or_else(
        || "(no identity published — a build predating the identity token)".to_owned(),
        ToOwned::to_owned,
    );
    let pidfile = socket.with_extension("pid");
    Err(format!(
        "build  child:{OWN_BUILD}  daemon:{daemon}  SKEW — the resident daemon on this socket \
         answers from a build that is not this client's; refusing to serve across builds \
         (docs/wire-contract.md §A.3, the socket law).\n\
         Why: the socket is keyed on the cache root — one root, one socket, one resident \
         daemon — and a resident survives upgrades until something restarts it. An answer \
         served across the skew is computed by a build this client did not make: wrong words, \
         no error — the measured defect the law closes.\n\
         Fixes — run whichever fits your case:\n\
           - when you own the resident (you started it, or it serves your own cache root): \
         restart it — kill the pid in {}; the next call auto-starts the current build.\n\
           - when an install or deploy pipeline manages this daemon: rerun its install step — \
         that step owns the restart duty, and one rerun converges every caller on this socket.\n\
           - when neither is yours: report this skew to whoever operates the daemon, quoting \
         the two builds above.",
        pidfile.display()
    ))
}

/// The `scoped-guards` behavior cap (wire-contract §3.2/§5.4) — ONE spelling
/// for every client lane. The cap is family-whole: a daemon serving it decodes
/// the whole scoped-premise family (splice/script `scope`+`guards[]` and the
/// §4.7 mint arm); a daemon not serving it refuses every guard-family field at
/// the strict wall, so the client half stays dormant until the hello says so.
pub(crate) const SCOPED_GUARDS_CAP: &str = "scoped-guards";

/// Does a parsed hello body advertise `cap`? Absent body, absent `caps`, or a
/// non-array all answer `false` — the dormant default, never a guess.
pub(crate) fn hello_has_cap(body: Option<&Value>, cap: &str) -> bool {
    body.and_then(|b| b.get("caps"))
        .and_then(Value::as_array)
        .is_some_and(|caps| caps.iter().filter_map(Value::as_str).any(|c| c == cap))
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

/// WHY the daemon path did not serve, when this run can say it. `None` when it cannot — the
/// first line of [`voice_degrade`] already names the fact, and a guess is worth less than
/// silence.
///
/// Two causes, in the order an operator can act on them:
///
/// 1. **A socket path that cannot fit in `sun_path`** — silent, and not fixed by starting a
///    daemon, so it outranks anything a spawn attempt reports (under it, a spawn is bound to
///    fail too, and naming the bind limit is what ends the loop). Since the short-sock law the
///    socket is hash-keyed under a short per-user base
///    (`registry::socket_path_for_cache_root`), so a merely long `XDG_CACHE_HOME` no longer
///    reaches this — only a pathological base (deep `HOME`, or a missing one forcing the
///    in-root fallback) still can.
/// 2. **A daemon this process spawned and never heard from** — its own dying words, read back
///    off its lane ([`record_spawn_failure`]). This is not a guess: it is the child's own text.
///
/// A refused handshake and a daemon somebody else's process failed to start are still covered
/// by the first line alone. A daemon this process REACHED, which then failed the exchange, is
/// not a degrade at all: it refuses ([`resident_lost`]).
pub(crate) fn degrade_reason() -> Option<String> {
    let Ok(client) = Client::from_default() else {
        return Some("No cache root resolves, so there is no socket path to dial.".to_owned());
    };
    let socket = client.socket_path();
    let len = socket.as_os_str().as_encoded_bytes().len();
    if len >= SUN_PATH_CAPACITY {
        return Some(format!(
            "The socket path is {len} bytes, at or over this platform's {SUN_PATH_CAPACITY}-byte \
             sun_path limit, so NO daemon can bind or dial it. The socket rides a short per-user \
             base ($XDG_RUNTIME_DIR on Linux, else $HOME/.cache/mrd-run); shorten that base \
             directory: {}",
            socket.display()
        ));
    }
    recorded_fact()
}

/// Why the daemon path did not serve THIS process, in this process's own words — set by
/// [`record_spawn_failure`], read by [`degrade_reason`].
///
/// A process-wide cell rather than a return value, because the lanes that need this fact do
/// not share a call shape: the read lanes swallow `ensure_daemon`'s error into a degrade
/// ([`connect_or_spawn`]) and three interpolate it into a refusal (`write_ipc`,
/// `fingerprint_cmd`, `script::cmd`). [`degrade_reason`] is the seam they all consult, so the
/// fact is recorded once and stated once, in one voice, wherever the run ends up surfacing it.
///
/// Last writer wins: a process that spawns twice is reporting on its latest attempt.
static DEGRADE_FACT: Mutex<Option<String>> = Mutex::new(None);

/// Record why the spawn this process just attempted left it with no daemon. A poisoned lock
/// drops it: a diagnostic is never worth a panic on a path whose whole job is to keep the run
/// alive.
fn record_spawn_failure(reason: String) {
    if let Ok(mut slot) = DEGRADE_FACT.lock() {
        *slot = Some(reason);
    }
}

/// The recorded degrade fact, if this process produced one. A poisoned lock answers `None`:
/// a diagnostic is never worth a panic on the degrade path.
fn recorded_fact() -> Option<String> {
    DEGRADE_FACT.lock().ok()?.clone()
}

/// What to say when a daemon we launched never bound its socket: QUOTE it. Its stderr is the
/// lane beside the socket (`daemon::voice`), so whatever it managed to say on the way down is
/// there, and `mark` is the length that lane had before this spawn — so the quote is this
/// child's words and nobody else's.
///
/// An EMPTY region is reported as such rather than skipped. "It said nothing" is a different
/// finding from "it said this", and it still beats the silence this whole path replaces: it
/// names the file, the timeout, and the fact that the child never reached its first line.
fn never_bound(mark: u64) -> String {
    let lane = daemon::voice_path().map_or_else(
        |_| "no cache root resolves, so it had none".to_owned(),
        |path| path.display().to_string(),
    );
    let secs = SPAWN_READY_TIMEOUT.as_secs();
    daemon::voice_since(mark).map_or_else(
        || {
            format!(
                "The daemon this run auto-spawned never bound its socket within {secs}s, and its \
                 lane ({lane}) carries nothing from it — it died before its first line, or \
                 something killed it."
            )
        },
        |said| {
            format!(
                "The daemon this run auto-spawned never bound its socket within {secs}s. Its own \
                 lane ({lane}) carries its last words: {said}"
            )
        },
    )
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
    // The rooted lane (§4.1 colon law): a head-colon PATH is an agent-plane
    // address, never a literal path — the edge map serves from the NAMED
    // root's bound workspace, exactly as if the caller stood there. The
    // ambient lane resolves from cwd exactly as before.
    let entered = match path_arg {
        Some(p) => crate::rooted::enter(p, "links", "Nothing was served."),
        None => Ok(None),
    };
    let ambient = || -> Result<std::path::PathBuf, Fail> {
        Ok(crate::resolve::resolve_runtime(workspace::Base::Cwd(&cwd))
            .map_err(|e| {
                Fail::tool(format!(
                    "cannot resolve workspace for {}: {e}",
                    cwd.display()
                ))
            })?
            .workspace)
    };
    let (workspace, rooted_rel) = match entered {
        Ok(Some((rel, rooted))) => (rooted.workspace, Some(rel)),
        Ok(None) => (ambient()?, None),
        // The refusal frames with the workspace the caller stands in — no
        // target workspace exists to name.
        Err(error) => {
            let ambient = ambient()?;
            return Err(json_refusal(format, &ambient, &error));
        }
    };
    let path_arg = match &rooted_rel {
        Some(rel) => Some(rel.as_str()),
        None => path_arg,
    };
    // §1 admission at the face, before any engine contact: the warm daemon
    // refuses a violating spelling but that refusal melts into the degrade
    // ([`try_daemon_links`] answers `None`), and the degrade's `load_doc`
    // resolves an absolute spelling verbatim — so this door SERVED a page from
    // outside the root (wire-contract §12.1, the door-family clause). The
    // refusal keeps the `--json` face's `{workspace, error}` frame, exactly as
    // the degrade's own engine-refusal seam publishes it. On the rooted lane
    // the rel half is already confined ([`crate::rooted`]) — a no-op pass.
    if let Some(p) = path_arg
        && crate::path_law::violates_path_law(p)
    {
        let mut error = ErrorBody::new(wire::ErrorCode::BadPath);
        error.path = Some(WirePath(p.to_owned()));
        crate::path_law::teach_bad_path(&workspace, &mut error, "links", "Nothing was served.");
        json_error_frame(format, &workspace, &error);
        return Err(Fail::tool(render_wire_error(&error)));
    }
    let answer = answer_links(&workspace, &cwd, path_arg, format)?;
    // Read off the ANSWER, so warm and degrade voice one fact from one source:
    // an enumeration names the population it did not carry (§4.6 `excluded`).
    voice_excluded(&answer.body);
    // `links` is the lane the never-refusing degrade was measured on, and it
    // reported the degrade as a `source:` LABEL with no cause — which is the
    // "5 seconds slower, then the ephemeral answer" symptom itself, spelled in
    // one word (card `auto-spawned-daemon-dies-silently`). The label stays
    // where it is; this adds the WHY on stderr, the lane `voice_excluded`
    // already speaks on, so both output formats gain it and neither's stdout
    // moves by a byte.
    voice_degrade(&answer.source);

    match format {
        Format::Json => {
            let value = json!({
                "workspace": workspace.display().to_string(),
                "source": answer.source.label(),
                "links": answer.body,
            });
            let rendering = timing::phase("json.render");
            let text = serde_json::to_string_pretty(&value).expect("json");
            rendering.stop();
            let writing = timing::phase("json.write");
            println!("{text}");
            writing.stop();
        }
        Format::Human => {
            println!("workspace {}", workspace.display());
            println!("  source: {}", answer.source.label());
            render_links_human(&answer.body, path_arg.is_some());
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
/// The population rides the answer (§4.6 `excluded`), never a second disk walk
/// here: a face that re-derived it could disagree with the door that served
/// it, which is the door/face split this rule exists to close. Empty on the
/// named form — a named path is served, so nothing was left out (§12.1).
///
/// The VOICE projects that key through the projection's one exclusion
/// predicate before speaking (card walk-law-audit): a member with a
/// dot-prefixed segment ([`fs::domain::dot_segment`], §12.1 rule 2) leaves
/// the count and the sample, so this face can never voice a path the record
/// projection refuses to serve (dogfood F11) — and the prose is capped by the
/// shared spelling ([`crate::voice_excluded_note`]), never an unbounded join
/// (the 2026-08-10 3.1M-character measurement). The wire key underneath stays
/// the complete outside-domain enumeration, and the note points at it as the
/// complete list — on this face, this verb's own `--json` answer.
fn voice_excluded(body: &Value) {
    let declined: Vec<String> = body
        .get("excluded")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|rel| !rel.split('/').any(fs::domain::dot_segment))
        .map(str::to_owned)
        .collect();
    crate::voice_excluded_note(&declined);
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
///
/// `named` is true when the caller addressed ONE path. It decides whether the
/// face owes a withheld-count line (laws.md § the face-honesty law, clause 1):
/// the enumeration form filters and must say so, while a named path is served
/// whole, so nothing was left out and a count line there would report a
/// filtering that did not happen.
fn render_links_human(body: &Value, named: bool) {
    let Some(files) = body.get("files").and_then(Value::as_object) else {
        return;
    };
    let mut any = false;
    let mut shown = 0usize;
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
        shown += 1;
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
        // Verdict then reason, spelled exactly as the refused rows below spell
        // it — `(count, verdict reason)`. A reason rides only where the target
        // is a real file the domain does not carry; a genuine typo keeps the
        // bare `unresolved` it has always had (decision 0034).
        let why = edges.get("unresolved_reason").and_then(Value::as_object);
        for (link, count) in unresolved.into_iter().flatten() {
            match why.and_then(|w| w.get(link)).and_then(Value::as_str) {
                Some(reason) => println!("    -> {link} ({count}, unresolved {reason})"),
                None => println!("    -> {link} ({count}, unresolved)"),
            }
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
    // A named path is served whole (§12.1), so this face filtered nothing and
    // owes no bound. The enumeration form always does.
    if !named {
        voice_population(files, shown);
    }
}

/// State the bound of the enumeration this face just printed: what it withheld,
/// by what criterion, and which face carries the rows — plus the population
/// split that keeps the engine's own bookkeeping out of a content count.
///
/// laws.md § the face-honesty law, clauses 1 and 4. The defect this closes:
/// the loop above SKIPS every edgeless file, so a corpus of 112 with 2 linked
/// files rendered as six lines and a reader concluded the corpus held 2.
/// Enumeration deliberately stays machine-side — the pointer is the answer here,
/// because flooding the human face is the same failure from the other side.
fn voice_population(files: &serde_json::Map<String, Value>, shown: usize) {
    let total = files.len();
    // `mrd init` writes the declaration INTO the corpus it declares, so it lands
    // in this map like any page. Counted and labeled, never silently either way:
    // excluding it hides a filter, and counting it unlabeled lets the engine
    // pollute the content denominator.
    let engine_owned = files
        .keys()
        .filter(|path| {
            Path::new(path)
                .file_name()
                .is_some_and(|name| name == DECLARATION_FILENAME)
        })
        .count();

    let withheld = total.saturating_sub(shown);
    if withheld > 0 {
        println!(
            "  shown {shown} of {total} — {withheld} with no outgoing links not listed; \
             `mrd links --json` enumerates every file"
        );
    }
    if engine_owned > 0 {
        let content = total - engine_owned;
        println!(
            "  {total} files: {content} content + {engine_owned} engine-owned ({DECLARATION_FILENAME})"
        );
    }
}

/// How long to wait for an auto-spawned daemon to bind its socket before degrading. Generous: a
/// cold daemon binds in milliseconds, so this only bounds the pathological "launched but never
/// came up" case.
const SPAWN_READY_TIMEOUT: Duration = Duration::from_secs(5);
/// Poll granularity while waiting for the socket to answer a ping.
const PING_POLL: Duration = Duration::from_millis(25);

/// Answer `links` for `workspace` (optional workspace-relative `path`): dial the resident
/// daemon — auto-spawning it — and degrade to the in-process ephemeral engine when no daemon
/// serves the workspace, or when the daemon's mount table is not this process's
/// ([`daemon_answer_needs_the_address_plane`]). Both lanes answer through one serve
/// (`wire_serve::read::links`), so the body and the exit code do not depend on the lane.
///
/// `format` reaches both lanes so each refusal seam can publish the `--json` face's
/// `{workspace, error}` envelope: the degrade's ([`in_process_links`]), and the daemon lane's
/// `retry` refusal that outlived the cap ([`try_daemon_links`]). Every other op refusal the
/// daemon answers still falls to the degrade, whose refusal seam stays the authoritative one.
pub(crate) fn answer_links(
    workspace: &Path,
    cwd: &Path,
    path: Option<&str>,
    format: Format,
) -> Result<Answer, Fail> {
    let dialing = timing::phase("daemon.dial");
    if let Some(body) = try_daemon_links(workspace, path, format)?
        && !daemon_answer_needs_the_address_plane(&body)
    {
        dialing.stop();
        return Ok(Answer {
            source: EngineSource::Daemon,
            body,
        });
    }
    dialing.stop();
    let body = in_process_links(workspace, cwd, path, format)?;
    Ok(Answer {
        source: EngineSource::Ephemeral,
        body,
    })
}

/// Did the daemon answer from a mount table that is not this process's? The daemon resolves a
/// rooted head against the table of ITS environment (`docs/wire-contract.md` §4.6, Cross-root
/// edges), and a client `MERIDIAN_CONFIG` never reaches it. A head it left `unresolved` that
/// this process's table declares is that skew — a daemon whose table lacks the root, or would
/// not resolve or bind — and the in-process lane then answers from this environment. Gates on
/// the answer, not a pre-flight scan.
///
/// Gated on the table's OWN names ([`addr::head_names_declared_root`]), never the bare lexical
/// `:` — an external URI trips the lexical test too (`[[https://…]]` has `https:` in its head),
/// and a head no table declares is `unresolved` on both lanes, so a rebuild could only repeat
/// the daemon's answer (one such rebuild measured 137 s on an 11.5k-file vault over 6
/// URL-shaped wikilinks).
fn daemon_answer_needs_the_address_plane(body: &Value) -> bool {
    needs_address_plane(body, declared_mount_set)
}

/// [`daemon_answer_needs_the_address_plane`] with the table load injectable. The table is
/// loaded once, and only when at least one unresolved head carries a `:` (the common body has
/// none, and pays nothing); `None` — this process's config would not resolve or its table
/// would not bind — degrades, so the answer under a broken config is the one this
/// environment gives.
fn needs_address_plane(body: &Value, table: impl FnOnce() -> Option<addr::MountSet>) -> bool {
    let mut rooted = unresolved_keys(body)
        .filter(|link| addr::head_carries_root_separator(link))
        .peekable();
    if rooted.peek().is_none() {
        return false;
    }
    let Some(set) = table() else {
        return true;
    };
    rooted.any(|link| addr::head_names_declared_root(link, &set))
}

/// Every `unresolved` key across the body's files — the spellings the daemon could not
/// resolve, verbatim.
fn unresolved_keys(body: &Value) -> impl Iterator<Item = &str> {
    body.get("files")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(_, edges)| edges.get("unresolved").and_then(Value::as_object))
        .flat_map(serde_json::Map::keys)
        .map(String::as_str)
}

/// The mount table's name projection, table-only: a config parse — no per-root corpus is
/// built (`load_mounts_for` narrows the corpora; this needs none at all). `None` when the
/// config will not resolve or the table will not bind; the caller degrades rather than guess
/// at a table it cannot read.
fn declared_mount_set() -> Option<addr::MountSet> {
    let env = config::Env::from_process();
    let resolution = config::resolve(&env).ok()?;
    let table = resolution.bind(&env).ok()?;
    Some(table.projection())
}

/// The daemon lane of `links`, on the CLI ladder (`docs/wire-contract.md` § A.10): dial
/// ([`connect_or_spawn`]), `hello` binds and warms `workspace`, then one `links`.
///
/// `Ok(None)` = the in-process lane answers: no daemon serves this workspace (none accepts
/// the connection, or `hello` refuses the workspace), or the daemon answered an op refusal
/// that is not `retry` — the in-process lane's refusal seam is the authoritative one.
/// `Err` = a daemon accepted the connection and is the answer, and the answer is a refusal:
/// build skew (0025 socket law), an exchange that did not complete, or a `retry` refusal that
/// outlived the cap.
fn try_daemon_links(
    workspace: &Path,
    path: Option<&str>,
    format: Format,
) -> Result<Option<Value>, Fail> {
    let Ok(client) = Client::from_default() else {
        return Ok(None);
    };
    let Some(stream) = connect_or_spawn(&client)? else {
        return Ok(None);
    };
    let socket = client.socket_path();
    let mut writer = stream.try_clone().map_err(|e| resident_lost(&e))?;
    let mut reader = BufReader::new(stream);

    // `hello` with a `workspace` resolves, pins and warms the resident engine, binding this
    // connection to it.
    let hello = json!({
        "op": "hello",
        "proto": 1,
        "contract": "v3",
        "workspace": workspace.to_string_lossy(),
    });
    let greeted = call(&mut writer, &mut reader, socket, &hello).map_err(|e| resident_lost(&e))?;
    if greeted.get("ok").and_then(Value::as_bool) != Some(true) {
        return Ok(None);
    }
    // 0025 socket law: identity equality on the hello frame already in hand.
    hello_identity_skew(greeted.get("body"), socket).map_err(Fail::tool)?;

    let mut links = json!({ "op": "links" });
    if let Some(p) = path {
        links["path"] = json!(p);
    }
    let response = call_through_retries(&mut writer, &mut reader, socket, workspace, &links)
        .map_err(|e| resident_lost(&e))?;
    if response.get("ok").and_then(Value::as_bool) == Some(true) {
        return Ok(response.get("body").cloned());
    }
    match retry_refusal(&response) {
        Some(error) => Err(json_refusal(format, workspace, &error)),
        None => Ok(None),
    }
}

/// Dial the registry socket, spawning a daemon only when nothing answers. `Ok(None)` = no
/// daemon accepts the connection and none came up: the caller's in-process lane answers.
///
/// **A successful connect IS the liveness proof**, so the warm path asks the question once
/// instead of twice. Calling [`ensure_daemon`] first would add its own liveness test
/// (`Client::ping`) on a SEPARATE connection: a second connect, a second connection thread in
/// the daemon and a second round trip on every warm read, where one dial answers both questions.
///
/// The pre-flight runs after the connect, and only when it fails: an absent or refusing socket
/// reaches `ensure_daemon`, which spawns the daemon and waits for readiness. Without it the first
/// read after boot would degrade in-process instead of starting the daemon.
///
/// The stream comes back under the wedge discipline ([`registry::wedge`]): a connect proves
/// only that something is LISTENING, and an unbounded read on a daemon that accepts and never
/// answers parks this process forever (card `registry-client-read-timeout-absent`).
///
/// # Errors
/// The socket refused the connection while its pidfile names a running `mrd`
/// ([`resident_stopping`]), or the accepted stream could not be put under the discipline
/// ([`resident_lost`]).
pub(crate) fn connect_or_spawn(client: &Client) -> Result<Option<UnixStream>, Fail> {
    let socket = client.socket_path();
    let bound = |stream: UnixStream| -> Result<Option<UnixStream>, Fail> {
        registry::wedge::bind(&stream).map_err(|e| resident_lost(&e))?;
        Ok(Some(stream))
    };
    match UnixStream::connect(socket) {
        Ok(stream) => return bound(stream),
        Err(error) => {
            if let Some(stopping) = resident_stopping(socket, &error) {
                return Err(stopping);
            }
        }
    }
    // `ensure_daemon` has recorded the richer fact — the child's own dying words — for
    // `voice_degrade` to state.
    if ensure_daemon(client).is_err() {
        return Ok(None);
    }
    match UnixStream::connect(socket) {
        Ok(stream) => bound(stream),
        Err(_) => Ok(None),
    }
}

/// The refusal for a daemon that accepted the connection and did not complete the exchange
/// (`docs/wire-contract.md` § A.10, the CLI ladder): it outlived the wedge cap or closed
/// mid-request, as `registry::wedge` judged it on the wire. It holds the corpus, so the
/// in-process lane would build the whole corpus beside it, and every caller waiting on the
/// same slow daemon would build one.
pub(crate) fn resident_lost(cause: &io::Error) -> Fail {
    Fail::tool(format!(
        "the resident daemon accepted this call and did not complete it ({cause}); the corpus          is not built in-process beside it — retry"
    ))
}

/// The refusal for a socket that refused the connection while its pidfile names a running
/// `mrd` (`docs/wire-contract.md` § A.11): a daemon in its stop closes its listener first and
/// holds the corpus until its checkpoint save ends, so the in-process lane would build the
/// whole corpus beside it, as in [`resident_lost`]. `None` for every other connect failure —
/// no socket file, or a refused one left by a daemon gone without cleanup — so the caller
/// spawns or degrades.
fn resident_stopping(socket: &Path, error: &io::Error) -> Option<Fail> {
    if error.kind() != io::ErrorKind::ConnectionRefused {
        return None;
    }
    let pid = crate::sql::running_daemon_pid(&socket.with_extension("pid"))?;
    Some(Fail::tool(format!(
        "the resident daemon (pid {pid}) refused this call: its socket is closed while the          process still runs, most often finishing a stop that still holds the corpus; the          corpus is not built in-process beside it — retry"
    )))
}

/// How long a silent wait on a `retry` refusal runs before the caller is told why.
const RETRY_VOICE_AFTER: Duration = Duration::from_secs(2);
/// The pause between two asks of a daemon that answers a `retry` refusal. The refusal is
/// served at config cost (the §3.2 cold gate reads no corpus), so the poll costs the daemon
/// nothing it would notice.
const RETRY_POLL: Duration = Duration::from_millis(100);

/// One round trip ([`call`]) that re-asks while the daemon answers a `retry` refusal
/// (`corpus_warming`, `corpus_race`), until it answers anything else or
/// `registry::wedge::WEDGE_CAP` is spent — then the last refusal comes back for the caller to
/// surface ([`retry_refusal`]).
///
/// The daemon is building the very corpus the in-process lane would build, and started
/// first, so waiting on it is never slower than building beside it and costs this process
/// nothing. The wait is said once on stderr, so a person watching knows what the pause is.
///
/// # Errors
/// As [`call`]: the exchange did not complete.
pub(crate) fn call_through_retries(
    writer: &mut UnixStream,
    reader: &mut BufReader<UnixStream>,
    socket: &Path,
    workspace: &Path,
    request: &Value,
) -> io::Result<Value> {
    let asked = Instant::now();
    let mut voiced = false;
    loop {
        let response = call(writer, reader, socket, request)?;
        let waited = asked.elapsed();
        if retry_refusal(&response).is_none() || waited >= registry::wedge::WEDGE_CAP {
            return Ok(response);
        }
        if !voiced && waited >= RETRY_VOICE_AFTER {
            voiced = true;
            eprintln!(
                "mrd: the resident daemon is still building {} and answers `retry`; waiting                  for it (up to {}s) — the corpus is not built in-process beside it.",
                workspace.display(),
                registry::wedge::WEDGE_CAP.as_secs()
            );
        }
        std::thread::sleep(RETRY_POLL);
    }
}

/// The §8 error body of a `retry`-class refusal frame (`ok:false`, `recovery: "retry"`),
/// including a code a newer daemon introduced; `None` for every other frame.
pub(crate) fn retry_refusal(response: &Value) -> Option<ErrorBody> {
    if response.get("ok").and_then(Value::as_bool) != Some(false) {
        return None;
    }
    let error = response.get("error")?;
    if error.get("recovery")?.as_str()? != "retry" {
        return None;
    }
    serde_json::from_value(error.clone()).ok()
}

/// Ensure a daemon serves `client`'s socket: return early if one is up — answering, or
/// [`Liveness::Busy`]: holding the probe unanswered, which only a live daemon can do — else
/// auto-spawn it detached and poll until it binds or the timeout elapses. A busy daemon is
/// never spawned beside: the spawn can only lose the singleton flock, and the caller's own
/// exchange waits it out under the wedge cap (`registry::wedge`). Errors The daemon could
/// not be spawned (spawn-impossible), or it was spawned but never became ready within
/// [`SPAWN_READY_TIMEOUT`].
pub(crate) fn ensure_daemon(client: &Client) -> io::Result<()> {
    if client.liveness() != Liveness::Down {
        return Ok(());
    }
    // The drain-budget hazard, checked HERE because this is where it is LOUD.
    // The child's identical `debug_assert` (inside `RunningServer::start`) now
    // lands on a lane that is read back — `spawn_detached` gives it a file for
    // its stderr (`daemon::voice`) and the degrade below quotes it — so this is
    // no longer the only place the failure can be seen. It is still the best:
    // it fails before the 5 s wait is spent, it carries this process's exit
    // code, and its stderr is captured directly by whoever ran `mrd`, which is
    // exactly where the subprocess fixtures look.
    //
    // The child inherits this environment, so resolving the config here yields
    // the same one it will build: `Config::resolve` stays the single reader of
    // the budget variable. Debug builds only, so a release client on a tmpfs
    // `XDG_CACHE_HOME` still spawns.
    //
    // A config that does NOT resolve is checked here too, and for the same
    // reason. `Config::resolve` refuses a malformed `MRD_DRAIN_COLD_BUILDS`
    // rather than falling back to the 2 s default (`parse_drain_cold_builds`) —
    // correctly, since the value exists to escape that default. The child
    // re-resolves the same environment and dies with that refusal, and while
    // that refusal now reaches its lane, this arm still says it 5 s earlier and
    // with a nonzero exit. Measured on `5bbd7912e`, before the lane existed:
    // `MRD_DRAIN_COLD_BUILDS=notanumber` gave exit 0, 5.03 s, an ephemeral
    // answer, and no mention of the variable anywhere.
    //
    // Still no new error route: both arms are `debug_assert`, so a release
    // client spawns and degrades exactly as it does today.
    let refusal = match registry::Config::resolve() {
        Ok(config) => config.drain_budget_hazard(),
        Err(e) => Some(format!(
            "the daemon layout this client is about to spawn does not resolve: {e}\n\
             The child would re-resolve the same environment and die with that \
             refusal on a null stderr, leaving a ~{}s degrade to the ephemeral \
             engine as the only symptom.",
            SPAWN_READY_TIMEOUT.as_secs()
        )),
    };
    debug_assert!(refusal.is_none(), "{}", refusal.unwrap_or_default());
    // Taken BEFORE the spawn: the lane is append-only and may already hold an
    // earlier daemon's lines, so only the region added after this mark is the
    // child we are about to launch ([`daemon::voice_since`]).
    let mark = daemon::voice_mark();
    let spawned = match daemon::spawn_detached() {
        Ok(pid) => pid,
        Err(error) => {
            record_spawn_failure(format!(
                "The daemon could not be launched at all ({error}) — nothing was started, so \
                 nothing can answer until it is (check MERIDIAN_DAEMON_BIN if it is set)."
            ));
            return Err(error);
        }
    };
    let deadline = Instant::now() + SPAWN_READY_TIMEOUT;
    while Instant::now() < deadline {
        if client.liveness() != Liveness::Down {
            // Before this client exits: out of its cgroup, the daemon outlives its unit.
            placement::place_spawned(client.socket_path(), spawned);
            return Ok(());
        }
        std::thread::sleep(PING_POLL);
    }
    // The whole point of the lane: the caller degrades (or refuses) in a moment,
    // and now it can say WHY rather than presenting every startup failure alike
    // as "5 seconds slower" (card `auto-spawned-daemon-dies-silently`).
    record_spawn_failure(never_bound(mark));
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "auto-spawned daemon did not become ready",
    ))
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
    json_error_frame(format, workspace, error);
    refusal_fail(error)
}

/// The envelope ALONE, with no exit code attached — the frame half of [`json_refusal`] for the
/// legs whose triad is the verb's own judgement. `mrd repair`'s lock-door refusal is a TOOL
/// failure (exit 2); routing it through [`json_refusal`] would publish the frame and
/// simultaneously tell a scripted caller a pin was unrecoverable (exit 1). Frame and exit are
/// two judgements: this emits one and leaves the other to the caller.
pub(crate) fn json_error_frame(format: Format, workspace: &Path, error: &ErrorBody) {
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
    // `remove_refused` (§ A.3): the message PROMISES "the referrers list" —
    // the human face owes the rows the `--json` face already serves, or the
    // promise dangles (dogfood #5).
    if let Some(referrers) = &error.referrers
        && !referrers.is_empty()
    {
        let _ = write!(out, "\n  referrers:");
        for r in referrers {
            let kind = match r.kind {
                wire::ReferrerKind::Wikilink => "wikilink",
                wire::ReferrerKind::Embed => "embed",
                wire::ReferrerKind::Pin => "pin",
                wire::ReferrerKind::Frontmatter => "frontmatter",
            };
            let key = r.key.as_deref().map_or(String::new(), |k| format!(" {k}"));
            let _ = write!(
                out,
                "\n    {}  {kind}{key}  {} edge{}",
                r.path,
                r.count,
                if r.count == 1 { "" } else { "s" }
            );
        }
    }
    out
}

/// Write one NDJSON request line **under the wedge discipline**
/// ([`registry::wedge::write_all`]): the write ticks, each tick asks `socket`
/// whether the daemon is still alive, and a frame that stalls mid-drain is
/// reported as wedged-or-absent with the commit fact stated — never as a bare
/// `WouldBlock`. The half both round trips share; the halves differ only in
/// what bounds the READ.
fn write_frame(writer: &mut UnixStream, socket: &Path, request: &Value) -> io::Result<()> {
    let mut line = serde_json::to_string(request).map_err(io::Error::other)?;
    line.push('\n');
    registry::wedge::write_all(writer, socket, registry::wedge::WEDGE_CAP, line.as_bytes())
}

/// One NDJSON round trip on an open connection, **under the wedge discipline**
/// ([`registry::wedge`]): write the request line, read one response line with a
/// tick that probes the daemon's liveness, parse it.
///
/// `socket` is the daemon's own path — the probe dials it a second time, which
/// is what lets a slow answer be told from a dead one.
///
/// Errors The write fails, the daemon closes without a response, the daemon
/// died mid-request or is wedged ([`registry::wedge::read_line`]), or the
/// response line is not valid JSON.
pub(crate) fn call(
    writer: &mut UnixStream,
    reader: &mut BufReader<UnixStream>,
    socket: &Path,
    request: &Value,
) -> io::Result<Value> {
    write_frame(writer, socket, request)?;
    let mut response = String::new();
    let read =
        registry::wedge::read_line(reader, socket, registry::wedge::WEDGE_CAP, &mut response)?;
    if read == 0 {
        // The one place a daemon's death mid-request is SEEN: its exit closes
        // the connection (`registry::wedge` § A held connection is the
        // liveness evidence).
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "daemon closed the connection without a response — it died mid-request or dropped \
             this call; the outcome of this call is unknown",
        ));
    }
    serde_json::from_str(&response).map_err(io::Error::other)
}

/// The same round trip, answering the response **line** instead of a parsed
/// value. The script entry embeds the splice response in its trace verbatim, and
/// a parse-then-reserialize would sort the object's keys and normalize its
/// whitespace — a second commit-fact shape. Callers that only read fields use
/// [`call`]. Errors The write fails, or the daemon closes without a response.
///
/// **The READ is unbounded on purpose, and only because its one caller bounds
/// itself.** The WRITE is not: it carries the wedge discipline like every other
/// frame ([`write_frame`]), because a stalled drain is answerable — the frame
/// is not whole on the wire, so nothing was committed — where a stalled ANSWER
/// is not.
/// [`crate::script::wire_host::SocketDoor`] owns the script/write door: it sets
/// the entry's wall clock on the socket at connect and runs its own tick loop
/// (`call_until_answered`), where a tick prints a notice instead of drawing a
/// verdict — a write verb past the wire has no honest answer but the daemon's.
/// Reaching this function from a NEW caller reintroduces the park-forever bug;
/// use [`call`].
pub(crate) fn call_line(
    writer: &mut UnixStream,
    reader: &mut BufReader<UnixStream>,
    socket: &Path,
    request: &Value,
) -> io::Result<String> {
    write_frame(writer, socket, request)?;

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
/// the daemon serves, then re-key it to the v3 vocabulary the CLI negotiated
/// ([`try_daemon_links`] sends `contract:v3`) so warm and degrade answers do not drift.
fn in_process_links(
    workspace: &Path,
    cwd: &Path,
    path: Option<&str>,
    format: Format,
) -> Result<Value, Fail> {
    let canonical = workspace::canonicalize(workspace).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace {} ({e})",
            workspace.display()
        ))
    })?;
    let root = fs::WorkspaceRoot(canonical);
    crate::build_gate::enter();
    let (files, fingerprint) = fs::domain_snapshot(&root)
        .map_err(|e| Fail::tool(format!("cannot read the corpus: {e}")))?;
    let (index, docs, unserved) = fs::build_corpus(files);
    crate::voice_unserved(&unserved);

    let as_of = Root(fingerprint.0);
    let wpath = path.map(|p| WirePath(p.to_owned()));
    // `live_root` samples after the read; for a single in-process snapshot the
    // world does not move, so it equals `as_of` (a legal §10.1 frame).
    let live = as_of.clone();
    let reading = timing::phase("links.read");
    let catalog = model::catalog::EagerCatalog::new(&docs);
    let body = wire_serve::read::links(
        &root,
        &index,
        &catalog,
        &unserved,
        wpath.as_ref(),
        // The mount table comes from the same loader the pin plane uses, so a cross-root
        // address resolves through the same `resolve_ref` on both planes. Corpora narrow to
        // the roots this answer's link targets name; the table itself is never narrowed. A
        // full-table eager load cost ~27 s CPU on a workspace naming zero roots.
        |needed| Ok(crate::walk_cmd::load_mounts_for(needed)),
        as_of,
        0,
        || Ok(live),
    )
    .map_err(|mut e| {
        // THE `--json` FACE'S REFUSAL ENVELOPE, and the exit triad is deliberately NOT moved.
        // This is the one leg of `links` where a `wire::ErrorBody` is the terminal outcome for an
        // object the caller addressed, so it owes the frame (status.md § the `--json` face answers
        // `{workspace, error}` on EVERY leg that can refuse). It stays a TOOL failure at exit 2:
        // `links` spells its OWN finding — a refused edge — as `Fail::findings` at exit 1 below,
        // so routing this engine refusal through `json_refusal` would tell a script the corpus
        // holds a bad edge when the read itself never completed. Frame and exit are two
        // judgements; `json_error_frame` emits one and leaves the other here.
        //
        // The frame names the workspace the CALLER passed, not `canonical`, so a refusal and a
        // success from the same invocation carry the same `workspace` string.
        crate::path_law::teach_cwd_respelling(workspace, cwd, &mut e);
        json_error_frame(format, workspace, &e);
        Fail::tool(render_wire_error(&e))
    })?;
    let body = serde_json::to_value(&body)
        .map_err(|e| Fail::tool(format!("cannot render the answer: {e}")))?;
    // Run the same lifted projection the daemon runs (`root` → `fingerprint`). It re-keys under
    // `body`, so wrap, project, and unwrap.
    let mut frame = json!({ "body": body });
    wire_serve::rev::project_response(&mut frame);
    let body = frame
        .as_object_mut()
        .and_then(|obj| obj.remove("body"))
        .unwrap_or(Value::Null);
    reading.stop();
    Ok(body)
}

/// Render a wire error body as a one-line diagnostic (the code plus its message
/// or echoed path) for the CLI's stderr. `pub(crate)` for the door admissions
/// that refuse as TOOL failures with a composed `ErrorBody` (the links leg
/// above, the put `--scope` wall) — `refusal_text` is the findings-leg twin.
pub(crate) fn render_wire_error(error: &ErrorBody) -> String {
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

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wire::{ErrorBody, ErrorCode, Referrer, ReferrerKind};

    use super::{needs_address_plane, refusal_text};

    /// A body whose one file leaves `links` unresolved.
    fn body_with_unresolved(links: &[&str]) -> serde_json::Value {
        let unresolved: serde_json::Map<String, serde_json::Value> =
            links.iter().map(|l| ((*l).to_owned(), json!(1))).collect();
        json!({ "files": { "notes/a.md": { "unresolved": unresolved } } })
    }

    /// URL-shaped wikilinks no table declares keep the daemon's answer — the
    /// exact bodies that used to cost a whole-corpus ephemeral rebuild.
    #[test]
    fn undeclared_url_heads_keep_the_daemon_answer() {
        let body = body_with_unresolved(&[
            "https://example.com",
            "meridian://x/y",
            "mailto:someone@host",
        ]);
        assert!(
            !needs_address_plane(&body, || Some(addr::MountSet::new([]))),
            "no declared root in any head — the ambient `unresolved` stands",
        );
    }

    /// A head the table declares still degrades: the daemon's `unresolved` is
    /// wrong for a bound target and hides the refusal for an unreachable one.
    #[test]
    fn declared_heads_still_take_the_address_plane() {
        let sessions = addr::MountName::parse("sessions").unwrap();
        let body = body_with_unresolved(&["https://example.com", "sessions:24-01/notes.md"]);
        assert!(
            needs_address_plane(&body, || Some(addr::MountSet::new([sessions]))),
            "a declared head is the address plane's to answer",
        );
    }

    /// An unreadable table keeps the OLD posture: any rooted head degrades.
    /// A broken config must never silently widen what the daemon may answer.
    #[test]
    fn an_unreadable_table_degrades_every_rooted_head() {
        let body = body_with_unresolved(&["https://example.com"]);
        assert!(needs_address_plane(&body, || None));
    }

    /// No rooted head anywhere: served from the daemon, and the table is
    /// never loaded at all.
    #[test]
    fn ambient_bodies_never_load_the_table() {
        let body = body_with_unresolved(&["plain-page", "a/b.md"]);
        assert!(!needs_address_plane(&body, || {
            panic!("no rooted head — the table load must not run")
        }));
    }

    /// The human face keeps the message's promise: a `remove_refused` renders
    /// the `referrers` rows the `--json` face serves — each referring file,
    /// its edge kind, and its edge count (dogfood #5: the promise used to
    /// dangle with no list under it).
    #[test]
    fn remove_refused_renders_the_referrers_list() {
        let mut e = ErrorBody::new(ErrorCode::RemoveRefused);
        e.message = Some("refused: notes/dead.md still has 4 inbound references".to_owned());
        e.referrers = Some(vec![
            Referrer {
                path: "notes/fan.md".to_owned(),
                kind: ReferrerKind::Wikilink,
                key: None,
                count: 2,
            },
            Referrer {
                path: "notes/gallery.md".to_owned(),
                kind: ReferrerKind::Embed,
                key: None,
                count: 1,
            },
            Referrer {
                path: "notes/lock-holder.md".to_owned(),
                kind: ReferrerKind::Pin,
                key: None,
                count: 1,
            },
            Referrer {
                path: "notes/source-holder.md".to_owned(),
                kind: ReferrerKind::Frontmatter,
                key: Some("source".to_owned()),
                count: 1,
            },
        ]);
        let text = refusal_text(&e);
        assert!(
            text.contains("referrers:"),
            "the list is announced under the message:\n{text}"
        );
        assert!(
            text.contains("notes/fan.md  wikilink  2 edges"),
            "each row names file, kind, count:\n{text}"
        );
        assert!(
            text.contains("notes/gallery.md  embed  1 edge"),
            "the singular count reads as one edge:\n{text}"
        );
        assert!(
            text.contains("notes/lock-holder.md  pin  1 edge"),
            "the ambient pin plane renders beside the link kinds:\n{text}"
        );
        assert!(
            text.contains("notes/source-holder.md  frontmatter source  1 edge"),
            "a frontmatter row names the key beside its kind:\n{text}"
        );
    }

    /// A refusal with no `referrers` extra renders exactly as before — the
    /// block is `remove_refused`'s alone.
    #[test]
    fn other_refusals_carry_no_referrers_block() {
        let mut e = ErrorBody::new(ErrorCode::FileNotFound);
        e.message = Some("file_not_found: no file at notes/gone.md".to_owned());
        let text = refusal_text(&e);
        assert!(
            !text.contains("referrers"),
            "no invented block on other codes:\n{text}"
        );
    }
}
