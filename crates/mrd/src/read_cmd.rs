//! `mrd read` — the composed read verb.
//!
//! ```text
//! mrd read <PATH>[#FRAG] [--section SEL] [--json]
//! ```
//!
//! One exchange over the composed read op: addressing + content + render at one
//! engine snapshot. With no `--section` the read answers the section map and
//! nothing else — dewey ordinal, depth, raw title, words, `sec_rev`, over the
//! read's fingerprint. `--section` (repeatable — a heading path, a dewey
//! ordinal, or a `^anchor`) is the section read that serves bodies.
//!
//! The `#FRAG` tail goes through the same selector door as `--section`
//! (Law A-2: a fragment is selector bytes): a heading path scopes the whole
//! call as its subtree; a `^id` or dewey spelling names one node, so it rides
//! `sections` and serves that node's body — `path#^id` and
//! `path --section '^id'` are two spellings of one address.
//!
//! Answered by the resident daemon (auto-spawned) or the in-process degrade —
//! both run the same [`wire_serve::read::composed_read`] leaves and the same v3
//! rev projection, so warm and degrade answers never drift. Human output is
//! `rendered_text` verbatim; `--json` wraps the projected body in the house frame.
//!
//! Exit triad: 0 served / 1 the engine refused (EVERY engine refusal,
//! `bad_request` included — the message is the engine's verbatim, golden-pinned
//! string) / 2 bad invocation (the CLI's own refusals, before any engine contact).

use std::io::BufReader;
use std::os::unix::net::UnixStream;
use std::path::Path;

use registry::Client;
use serde_json::{Value, json};
use wire::{ErrorBody, Path as WirePath};

use crate::engine::{self, EngineSource};
use crate::{Fail, Format, current_dir};

/// Run `mrd read <PATH>[FRAG] [--section SEL] [--json]`. Errors [`Fail`] — exit 2 on a bad
/// invocation (the CLI's own refusals, before any engine contact); exit 1 on any engine
/// refusal (`bad_request`, `ref_not_found` …), message verbatim.
pub(crate) fn dispatch(args: &[String]) -> Result<(), Fail> {
    let parsed = Read::parse(args)?;
    let cwd = current_dir()?;
    let resolved = crate::resolve::resolve_runtime(&cwd).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace for {}: {e}",
            cwd.display()
        ))
    })?;
    let (source, mut body) = answer_read(&resolved.workspace, &cwd, &parsed)?;

    match parsed.format {
        Format::Json => {
            drop_duplicated_map(&mut body);
            let value = json!({
                "workspace": resolved.workspace.display().to_string(),
                "source": source.label(),
                "read": body,
            });
            println!("{}", serde_json::to_string_pretty(&value).expect("json"));
        }
        Format::Human => {
            // The rendered projection is the human face — verbatim, no header.
            let text = body
                .get("rendered_text")
                .and_then(Value::as_str)
                .unwrap_or_default();
            print!("{text}");
            // The degrade voice comes after the answer and on stderr, so stdout stays
            // byte-identical to the warm answer.
            engine::voice_degrade(&source);
        }
    }
    Ok(())
}

/// A toc read's `--json` drops `rendered_text`: on a toc read both planes carry the same facts
/// (`toc[]` structured, `rendered_text` the TOON encoding of those same rows — measured at 5473
/// chars of duplication on one page). `rendered_text` survives in `--json` only where a body was
/// requested, where it is prose the raw `sections[].content` rows do not spell the same way.
///
/// Not a wire change (the composed read op still answers both planes) and not the human face
/// (`mrd read` without `--json` prints `rendered_text` verbatim).
fn drop_duplicated_map(body: &mut Value) {
    let is_toc_read = body.get("toc").is_some_and(|t| !t.is_null());
    if let (true, Some(obj)) = (is_toc_read, body.as_object_mut()) {
        obj.remove("rendered_text");
    }
}

/// The parsed `read` invocation.
struct Read {
    /// The workspace-relative file path (the part before `#`).
    path: String,
    /// The `#FRAG` tail, when given.
    frag: Option<String>,
    /// `--section` selectors, in order — non-empty is the section read.
    sections: Vec<String>,
    format: Format,
}

impl Read {
    fn parse(args: &[String]) -> Result<Self, Fail> {
        let mut positional: Option<String> = None;
        let mut sections: Vec<String> = Vec::new();
        let mut json = false;
        let mut it = args.iter();
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "--json" => json = true,
                "--section" => {
                    let value = it
                        .next()
                        .ok_or_else(|| Fail::tool("--section needs a value".to_owned()))?;
                    sections.push(value.clone());
                }
                flag if flag.starts_with('-') => {
                    return Err(Fail::tool(format!("unknown flag: {flag}")));
                }
                value if positional.is_none() => positional = Some(value.to_owned()),
                value => return Err(Fail::tool(format!("unexpected argument: {value}"))),
            }
        }
        let Some(full) = positional else {
            return Err(Fail::tool("read needs a PATH".to_owned()));
        };
        let (path, frag) = match full.split_once('#') {
            Some((p, f)) => (p.to_owned(), Some(f.to_owned())),
            None => (full, None),
        };
        Ok(Read {
            path,
            frag,
            sections,
            format: if json { Format::Json } else { Format::Human },
        })
    }
}

/// What the daemon path answered. The split matters: a refusal is an ANSWER —
/// the warm engine looked and said no, with its teaching (path, message,
/// recovery) — while `Unavailable` means the question never reached an engine
/// at all, the one case where the in-process degrade answers instead.
enum DaemonRead {
    /// `ok:true` — the wire success body.
    Served(Value),
    /// `ok:false` with a well-formed §8 error envelope — the engine's typed
    /// refusal, surfaced verbatim. Degrading past it would remint from a
    /// single-file load that cannot know what the corpus-holding engine said
    /// (e.g. the per-file `invalid_utf8` naming an unserved member).
    Refused(Box<ErrorBody>),
    /// The daemon path itself failed: socket, spawn, handshake, or a frame
    /// that does not parse as the wire vocabulary.
    Unavailable,
    /// The daemon answered the handshake from a build that is not this
    /// client's (0025 socket law). A refusal, never a degrade: melting skew
    /// into `Unavailable` would serve an in-process answer and hide the stale
    /// resident forever. Carries the one-voice message
    /// ([`engine::hello_identity_skew`]).
    Skew(String),
}

/// Answer the composed read: dial the resident daemon (auto-spawning it) and serve its answer —
/// success or typed refusal alike. Only when the daemon is unreachable degrade to the
/// in-process engine — the same leaves, the same projection, only the reported source differs.
fn answer_read(workspace: &Path, cwd: &Path, r: &Read) -> Result<(EngineSource, Value), Fail> {
    match try_daemon_read(workspace, r) {
        DaemonRead::Served(body) => Ok((EngineSource::Daemon, body)),
        DaemonRead::Refused(mut error) => {
            teach_bad_path(workspace, &mut error);
            teach_cwd_respelling(workspace, cwd, &mut error);
            Err(engine::json_refusal(r.format, workspace, &error))
        }
        DaemonRead::Unavailable => {
            Ok((EngineSource::Ephemeral, in_process_read(workspace, cwd, r)?))
        }
        DaemonRead::Skew(message) => Err(Fail::tool(message)),
    }
}

/// The §1 path-law admission the warm decoder applies (`req_path`), mirrored
/// so the two transports cannot drift: before this gate the daemonless read
/// served any absolute spelling — `/etc/hosts` included — that the warm door
/// refuses (dogfood NEW-A, degrade half).
fn violates_path_law(path: &str) -> bool {
    path.is_empty()
        || path.starts_with('/')
        || path
            .split('/')
            .any(|seg| seg.is_empty() || seg == "." || seg == "..")
}

/// The read door's `bad_path` teaching (dogfood NEW-A): the warm decoder's
/// refusal echoes only the offending path, so the face composes the §1 rule
/// and — when the spelling lies inside this workspace — the respell the write
/// door already computes ([`wire_serve::write::relative_respelling`]: one
/// implementation, both doors, so the read door untrains the same mistake the
/// write door untrains). No-op on any other code or an already-taught refusal.
fn teach_bad_path(workspace: &Path, error: &mut ErrorBody) {
    if error.code != wire::ErrorCode::BadPath || error.message.is_some() {
        return;
    }
    let Some(path) = error.path.clone() else {
        return;
    };
    let mut m = format!(
        "{} is not a workspace-relative path — the read door admits only workspace-relative \
         spellings (§1 path law: no absolute path, no `.`/`..`/empty segment). Nothing was \
         read and no rev was minted.",
        path.0
    );
    if let Ok(canonical) = workspace::canonicalize(workspace) {
        let root = fs::WorkspaceRoot(canonical);
        if let Some(rel) = wire_serve::write::relative_respelling(&root, &path.0) {
            use std::fmt::Write as _;
            let _ = write!(
                m,
                " This path lies inside this workspace — respell it as `{rel}`."
            );
        }
    }
    // A refusal carries its recovery when one clearly exists (laws.md § the
    // face-honesty law, clause 3). `mrd read .` is the caller asking to see the
    // corpus at a door that serves one page; `links --json` is the door that
    // answers it. Every other bad path keeps the respelling above and gains no
    // pointer — a wrong pointer is worse than none.
    if crate::names_the_whole_corpus(&path.0) {
        use std::fmt::Write as _;
        let _ = write!(
            m,
            " To list the corpus instead of one page, `mrd links --json` enumerates every file."
        );
    }
    error.message = Some(m);
}

/// The `file_not_found` companion to [`teach_bad_path`]: when the caller's cwd lies inside the
/// workspace and the ref names a file that EXISTS relative to that cwd, the refusal names the
/// workspace-relative spelling verbatim. Teaching only — admission is unchanged, the ref grammar
/// is unchanged, and nothing downstream sees a rewritten path. Reuses the write door's
/// [`wire_serve::write::relative_respelling`] by handing it the joined absolute spelling, so the
/// respell computation has ONE implementation across all three refusals that carry one.
fn teach_cwd_respelling(workspace: &Path, cwd: &Path, error: &mut ErrorBody) {
    use std::fmt::Write as _;
    if error.code != wire::ErrorCode::FileNotFound {
        return;
    }
    let Some(path) = error.path.clone() else {
        return;
    };
    if path.0.starts_with('/') {
        return;
    }
    let candidate = cwd.join(&path.0);
    if !candidate.is_file() {
        return;
    }
    let Ok(canonical) = workspace::canonicalize(workspace) else {
        return;
    };
    let root = fs::WorkspaceRoot(canonical);
    let Some(abs) = candidate.to_str() else {
        return;
    };
    let Some(rel) = wire_serve::write::relative_respelling(&root, abs) else {
        return;
    };
    if rel == path.0 {
        return;
    }
    let m = error.message.get_or_insert_with(String::new);
    let _ = write!(
        m,
        " Did you mean `{rel}`? — that file exists relative to your cwd, and refs are spelled \
         from the workspace root, never from where you stand."
    );
}

/// The whole daemon path: socket, ensure-up, `hello` (v3, workspace-bound), then the `read` op.
fn try_daemon_read(workspace: &Path, r: &Read) -> DaemonRead {
    daemon_read(workspace, r).unwrap_or(DaemonRead::Unavailable)
}

/// Dial the registry socket, spawning a daemon only when nothing answers.
///
/// **A successful connect IS the liveness proof**, so the warm path asks the question once
/// instead of twice. It used to call [`engine::ensure_daemon`] first, whose own liveness test
/// (`Client::ping`) opens a SEPARATE connection — two dials where one answers, and the daemon's
/// accept loop polls a non-blocking listener on a fixed quantum, so each dial waits out its own
/// sleep. That doubled the fixed cost of every warm read: measured 40.2ms → 20.1ms, a 2.00x
/// improvement, on the session receipt `09-11-mentor-8h-perfection-loop`
/// `results/perf-finding-accept-poll-countdown-2026-08-09.md` (a session tree, not this repo).
///
/// The pre-flight is NOT redundant, which is why it is reordered rather than deleted: its answer
/// decides whether to AUTO-SPAWN, and dropping it outright would leave the first read after boot
/// silently degrading in-process forever instead of starting the daemon. The cold path is
/// unchanged — an absent or refusing socket still reaches `ensure_daemon`, which spawns and waits
/// for readiness.
fn connect_or_spawn(client: &Client) -> std::io::Result<UnixStream> {
    if let Ok(stream) = UnixStream::connect(client.socket_path()) {
        return Ok(stream);
    }
    engine::ensure_daemon(client)?;
    UnixStream::connect(client.socket_path())
}

/// `None` on any transport or handshake failure — the caller degrades. An op-level `ok:false`
/// is NOT such a failure: it comes back as [`DaemonRead::Refused`] carrying the engine's frame.
fn daemon_read(workspace: &Path, r: &Read) -> Option<DaemonRead> {
    let client = Client::from_default().ok()?;
    let stream = connect_or_spawn(&client).ok()?;
    let mut writer = stream.try_clone().ok()?;
    let mut reader = BufReader::new(stream);

    let hello = json!({
        "op": "hello",
        "proto": 1,
        "contract": "v3",
        "workspace": workspace.to_string_lossy(),
    });
    let greeted = engine::call(&mut writer, &mut reader, &hello).ok()?;
    if greeted.get("ok").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    // 0025 socket law: identity equality on the hello frame already in hand —
    // no extra round trip, and skew is a refusal, never a degrade.
    if let Err(message) = engine::hello_identity_skew(greeted.get("body"), client.socket_path()) {
        return Some(DaemonRead::Skew(message));
    }

    let response = engine::call(&mut writer, &mut reader, &r.request()).ok()?;
    if response.get("ok").and_then(Value::as_bool) == Some(true) {
        return Some(DaemonRead::Served(response.get("body")?.clone()));
    }
    // A refusal frame that does not parse as the §8 envelope is a broken
    // channel, not a refusal — that one degrades.
    let error: ErrorBody = serde_json::from_value(response.get("error")?.clone()).ok()?;
    Some(DaemonRead::Refused(Box::new(error)))
}

/// The two selector inputs routed to their wire fields — the CLI's half of the
/// one-door law (Law A-2: a fragment is selector bytes). The fragment goes
/// through [`wire::ReadSel::parse`] like every human selector string: a
/// heading path is a subtree scope and rides the `toc` scope; a `^id` or
/// dewey spelling names one node — a section read — so it MOVES onto
/// `sections`, never onto both planes (the engine refuses `toc` + `sections`
/// together). Before this door, `#^id` reached the engine as a heading whose
/// literal text was `^id`, missed, and refused — the anchor lane was never
/// entered (season-1 finding 5, attribution overturned onto the faces).
///
/// With explicit `--section` values beside a fragment, nothing moves: both
/// planes ride as given and the engine's pass-one refusal answers, exactly
/// as before the door (it refuses on presence, before reading any content).
fn route_selectors(
    frag: Option<&str>,
    sections: &[String],
) -> (Option<wire::ReadSel>, Option<Vec<wire::ReadSel>>) {
    let explicit: Option<Vec<wire::ReadSel>> =
        (!sections.is_empty()).then(|| sections.iter().map(|s| wire::ReadSel::parse(s)).collect());
    let Some(frag) = frag else {
        return (None, explicit);
    };
    match wire::ReadSel::parse(frag) {
        sel @ wire::ReadSel::Hpath { .. } => (Some(sel), explicit),
        sel if explicit.is_none() => (None, Some(vec![sel])),
        _ => (
            Some(wire::ReadSel::Hpath {
                hpath: vec![wire::HpathSeg {
                    h: frag.to_owned(),
                    n: None,
                }],
            }),
            explicit,
        ),
    }
}

impl Read {
    /// The wire `read` request this invocation maps onto. `display_path` is the path exactly as
    /// the user typed it — the engine renders the caller's spelling, never invents one.
    fn request(&self) -> Value {
        let mut req = json!({
            "op": "read",
            "path": self.path,
            "display_path": self.path,
        });
        // Both selector fields are structured on the wire; this is where a typed string becomes
        // structure — once, at the edge, through the one selector door, so nothing inward of
        // the CLI carries a joined address.
        let (toc, sections) = route_selectors(self.frag.as_deref(), &self.sections);
        if let Some(toc) = toc {
            req["toc"] = json!(toc);
        }
        if let Some(sections) = sections {
            req["sections"] = json!(sections);
        }
        req
    }
}

/// The degrade: load the one document from disk, answer through the same composed-read leaf the
/// daemon serves, then run the same v3 vocabulary projection — warm and degrade bodies are
/// byte-identical for the same state.
fn in_process_read(workspace: &Path, cwd: &Path, r: &Read) -> Result<Value, Fail> {
    // The same admission the warm decoder runs before any engine contact —
    // without it `fs::load`'s `root.join(path)` resolves an absolute spelling
    // verbatim and serves bytes from outside the root.
    if violates_path_law(&r.path) {
        let mut error = ErrorBody::new(wire::ErrorCode::BadPath);
        error.path = Some(WirePath(r.path.clone()));
        teach_bad_path(workspace, &mut error);
        return Err(engine::json_refusal(r.format, workspace, &error));
    }
    let canonical = workspace::canonicalize(workspace).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace {} ({e})",
            workspace.display()
        ))
    })?;
    let root = fs::WorkspaceRoot(canonical);
    let wpath = WirePath(r.path.clone());
    // The degrade teaches the same respelling the warm path teaches: a refusal that carries the
    // fitted spelling on one transport and not the other trains the habit only half the time.
    let doc = wire_serve::load_doc(&root, &wpath).map_err(|mut e| {
        teach_cwd_respelling(workspace, cwd, &mut e);
        engine::json_refusal(r.format, workspace, &e)
    })?;
    let ambient = wire_serve::ambient_root(&root)
        .map_err(|e| engine::json_refusal(r.format, workspace, &e))?;
    // The same routing the wire request does — one door, two transports,
    // so warm and degrade cannot diverge on what a selector means.
    let (toc, sections) = route_selectors(r.frag.as_deref(), &r.sections);
    let params = wire_serve::read::ReadParams {
        toc,
        sections,
        display_path: Some(r.path.clone()),
        // Read provenance is the daemon's to stamp; the local CLI sends none on
        // both warm and degrade paths (symmetry with the wire call).
        actor: None,
    };
    // No actor and no session store, so the local CLI mints no read receipt on either path. This
    // degrade path loads one document, not the corpus, so it cannot color a pin whose target is
    // another page — the decorated face is the daemon's, and `mrd read` serves the stored
    // spelling, which is also the spelling `mrd put` takes.
    let body = wire_serve::read::composed_read(
        &doc,
        &wpath,
        &ambient,
        &params,
        None,
        &wire_serve::read::NO_DECORATIONS,
    )
    .map_err(|e| engine::json_refusal(r.format, workspace, &e))?;
    let body = serde_json::to_value(&body)
        .map_err(|e| Fail::tool(format!("cannot render the answer: {e}")))?;
    // The same lifted projection the daemon applies for a v3 session
    // (`root` → `fingerprint`), so the degrade JSON never drifts from warm.
    let mut frame = json!({ "body": body });
    wire_serve::rev::project_response(&mut frame);
    Ok(frame
        .as_object_mut()
        .and_then(|obj| obj.remove("body"))
        .unwrap_or(Value::Null))
}

#[cfg(test)]
mod frag_door_tests {
    //! The router alone — both transports call it, so these four facts are
    //! the whole door: heading stays the scope, `^id`/dewey move onto
    //! `sections`, never both planes from one fragment, and an explicit
    //! `--section` beside any fragment leaves the either/or refusal to the
    //! engine.

    use super::route_selectors;

    #[test]
    fn a_heading_fragment_stays_the_whole_call_scope() {
        let (toc, sections) = route_selectors(Some("Alpha/Beta"), &[]);
        let wire::ReadSel::Hpath { hpath } = toc.expect("the heading plane rides") else {
            panic!("a heading fragment routes onto the toc scope's hpath arm");
        };
        let texts: Vec<&str> = hpath.iter().map(|s| s.h.as_str()).collect();
        assert_eq!(texts, ["Alpha", "Beta"]);
        assert!(sections.is_none(), "nothing moved onto sections");
    }

    #[test]
    fn an_anchor_fragment_moves_onto_sections() {
        let (frag, sections) = route_selectors(Some("^goal"), &[]);
        assert!(frag.is_none(), "the frag plane is vacated");
        assert_eq!(
            sections.expect("the section read rides"),
            vec![wire::ReadSel::Anchor {
                anchor: "goal".into()
            }]
        );
    }

    #[test]
    fn a_dewey_fragment_moves_onto_sections() {
        let (frag, sections) = route_selectors(Some("1.1"), &[]);
        assert!(frag.is_none(), "the frag plane is vacated");
        assert_eq!(
            sections.expect("the section read rides"),
            vec![wire::ReadSel::Dewey { n: "1.1".into() }]
        );
    }

    #[test]
    fn explicit_sections_beside_any_fragment_keep_both_planes() {
        for frag in ["Alpha", "^goal", "1.1"] {
            let (f, s) = route_selectors(Some(frag), &["Beta".to_owned()]);
            assert!(
                f.is_some() && s.is_some(),
                "both planes ride for {frag}, so the engine's either/or refusal answers"
            );
        }
    }
}
