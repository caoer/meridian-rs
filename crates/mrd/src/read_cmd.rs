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
    let (source, mut body) = answer_read(&resolved.workspace, &parsed)?;

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
}

/// Answer the composed read: dial the resident daemon (auto-spawning it) and serve its answer —
/// success or typed refusal alike. Only when the daemon is unreachable degrade to the
/// in-process engine — the same leaves, the same projection, only the reported source differs.
fn answer_read(workspace: &Path, r: &Read) -> Result<(EngineSource, Value), Fail> {
    match try_daemon_read(workspace, r) {
        DaemonRead::Served(body) => Ok((EngineSource::Daemon, body)),
        DaemonRead::Refused(error) => Err(engine::refusal_fail(&error)),
        DaemonRead::Unavailable => Ok((EngineSource::Ephemeral, in_process_read(workspace, r)?)),
    }
}

/// The whole daemon path: socket, ensure-up, `hello` (v3, workspace-bound), then the `read` op.
fn try_daemon_read(workspace: &Path, r: &Read) -> DaemonRead {
    daemon_read(workspace, r).unwrap_or(DaemonRead::Unavailable)
}

/// `None` on any transport or handshake failure — the caller degrades. An op-level `ok:false`
/// is NOT such a failure: it comes back as [`DaemonRead::Refused`] carrying the engine's frame.
fn daemon_read(workspace: &Path, r: &Read) -> Option<DaemonRead> {
    let client = Client::from_default().ok()?;
    engine::ensure_daemon(&client).ok()?;
    let stream = UnixStream::connect(client.socket_path()).ok()?;
    let mut writer = stream.try_clone().ok()?;
    let mut reader = BufReader::new(stream);

    let hello = json!({
        "op": "hello",
        "proto": 1,
        "contract": "v3",
        "workspace": workspace.to_string_lossy(),
    });
    if engine::call(&mut writer, &mut reader, &hello)
        .ok()?
        .get("ok")
        .and_then(Value::as_bool)
        != Some(true)
    {
        return None;
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
/// heading path is a subtree scope and stays `frag`; a `^id` or dewey
/// spelling names one node — a section read — so it MOVES onto `sections`,
/// never onto both planes (the engine refuses `frag` + `sections` together).
/// Before this door, `#^id` reached the engine as a heading whose literal
/// text was `^id`, missed, and refused — the anchor lane was never entered
/// (season-1 finding 5, attribution overturned onto the faces).
///
/// With explicit `--section` values beside a fragment, nothing moves: both
/// planes ride as given and the engine's either/or refusal answers, exactly
/// as before the door (it refuses on presence, before reading any content).
fn route_selectors(
    frag: Option<&str>,
    sections: &[String],
) -> (Option<Vec<wire::HpathSeg>>, Option<Vec<wire::ReadSel>>) {
    let explicit: Option<Vec<wire::ReadSel>> =
        (!sections.is_empty()).then(|| sections.iter().map(|s| wire::ReadSel::parse(s)).collect());
    let Some(frag) = frag else {
        return (None, explicit);
    };
    match wire::ReadSel::parse(frag) {
        wire::ReadSel::Hpath { hpath } => (Some(hpath), explicit),
        sel if explicit.is_none() => (None, Some(vec![sel])),
        _ => (
            Some(vec![wire::HpathSeg {
                h: frag.to_owned(),
                n: None,
            }]),
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
        let (frag, sections) = route_selectors(self.frag.as_deref(), &self.sections);
        if let Some(frag) = frag {
            req["frag"] = json!(frag);
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
fn in_process_read(workspace: &Path, r: &Read) -> Result<Value, Fail> {
    let canonical = workspace::canonicalize(workspace).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace {} ({e})",
            workspace.display()
        ))
    })?;
    let root = fs::WorkspaceRoot(canonical);
    let wpath = WirePath(r.path.clone());
    let doc = wire_serve::load_doc(&root, &wpath).map_err(|e| engine::refusal_fail(&e))?;
    let ambient = wire_serve::ambient_root(&root).map_err(|e| engine::refusal_fail(&e))?;
    // The same routing the wire request does — one door, two transports,
    // so warm and degrade cannot diverge on what a selector means.
    let (frag, sections) = route_selectors(r.frag.as_deref(), &r.sections);
    let params = wire_serve::read::ReadParams {
        frag,
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
    .map_err(|e| engine::refusal_fail(&e))?;
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
        let (frag, sections) = route_selectors(Some("Alpha/Beta"), &[]);
        let hpath = frag.expect("the heading plane rides");
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
