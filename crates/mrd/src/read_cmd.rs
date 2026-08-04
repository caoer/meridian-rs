//! `mrd read` — the composed read verb (M1 U1, ratified read/put naming).
//!
//! ```text
//! mrd read <PATH>[#FRAG] [--mode toc|sections] [--section SEL] [--json]
//! ```
//!
//! One exchange over the U4a2 composed read op (decision D6): addressing +
//! content + render at ONE engine snapshot. Default mode `toc` answers the
//! section map (dewey ordinal, depth, title, hpath, words, `sec_rev`) plus the
//! rendered text projection; `--section` (repeatable — a heading path, a dewey
//! ordinal, or a `^anchor`) selects sections and implies mode `sections`.
//!
//! Answered by the resident daemon (auto-spawned, the [`crate::engine`]
//! watchman model) or the in-process degrade — BOTH run the same
//! [`wire_serve::read::composed_read`] leaves and the same v3 rev projection,
//! so warm and degrade answers never drift. Human output is `rendered_text`
//! VERBATIM (the projection IS the human face); `--json` wraps the projected
//! body in the house frame.
//!
//! Exit triad: 0 served / 1 the engine refused (the message is the engine's
//! verbatim, golden-pinned string) / 2 bad invocation or `bad_request`.

use std::io::BufReader;
use std::os::unix::net::UnixStream;
use std::path::Path;

use registry::Client;
use serde_json::{Value, json};
use wire::Path as WirePath;

use crate::engine::{self, EngineSource};
use crate::{Fail, Format, current_dir};

/// Run `mrd read <PATH>[#FRAG] [--mode M] [--section SEL] [--json]`.
///
/// # Errors
/// [`Fail`] — exit 2 on a bad invocation or a `bad_request` refusal; exit 1 on
/// any other engine refusal (`ref_not_found` …), message verbatim.
pub(crate) fn dispatch(args: &[String]) -> Result<(), Fail> {
    let parsed = Read::parse(args)?;
    let cwd = current_dir()?;
    let resolved = crate::resolve::resolve_runtime(&cwd).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace for {}: {e}",
            cwd.display()
        ))
    })?;
    let (source, body) = answer_read(&resolved.workspace, &parsed)?;

    match parsed.format {
        Format::Json => {
            let value = json!({
                "workspace": resolved.workspace.display().to_string(),
                "source": source.label(),
                "read": body,
            });
            println!("{}", serde_json::to_string_pretty(&value).expect("json"));
        }
        Format::Human => {
            // The rendered projection IS the human face — verbatim, no header.
            let text = body
                .get("rendered_text")
                .and_then(Value::as_str)
                .unwrap_or_default();
            print!("{text}");
        }
    }
    Ok(())
}

/// The parsed `read` invocation.
struct Read {
    /// The workspace-relative file path (the part before `#`).
    path: String,
    /// The `#FRAG` tail, when given.
    frag: Option<String>,
    /// `--mode`, as given (the engine gates the value).
    mode: Option<String>,
    /// `--section` selectors, in order.
    sections: Vec<String>,
    format: Format,
}

/// A `#Fragment` the user typed → the segment array the wire takes. It scopes
/// a HEADING subtree, so it is parsed as a heading path and nothing else: a
/// `^id` or a dewey ordinal in this position would be a subtree with no
/// descendants, which is a section read, not a scope.
fn frag_segments(frag: &str) -> Vec<wire::HpathSeg> {
    frag.split('/')
        .map(|h| wire::HpathSeg {
            h: h.to_owned(),
            n: None,
        })
        .collect()
}

impl Read {
    fn parse(args: &[String]) -> Result<Self, Fail> {
        let mut positional: Option<String> = None;
        let mut mode: Option<String> = None;
        let mut sections: Vec<String> = Vec::new();
        let mut json = false;
        let mut it = args.iter();
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "--json" => json = true,
                "--mode" => {
                    let value = it
                        .next()
                        .ok_or_else(|| Fail::tool("--mode needs a value".to_owned()))?;
                    mode = Some(value.clone());
                }
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
        // Loud, not silent: the wire would ignore `sections` under mode toc —
        // a human face refuses the contradiction instead (stricter than the Go
        // MCP face, deliberately; this CLI has no byte-parity gate).
        if mode.as_deref() == Some("toc") && !sections.is_empty() {
            return Err(Fail::tool(
                "--section conflicts with --mode toc (sections are a sections-mode selector)"
                    .to_owned(),
            ));
        }
        if mode.is_none() && !sections.is_empty() {
            mode = Some("sections".to_owned());
        }
        let (path, frag) = match full.split_once('#') {
            Some((p, f)) => (p.to_owned(), Some(f.to_owned())),
            None => (full, None),
        };
        Ok(Read {
            path,
            frag,
            mode,
            sections,
            format: if json { Format::Json } else { Format::Human },
        })
    }
}

/// Answer the composed read: dial the resident daemon (auto-spawning it), and
/// on ANY daemon-path failure degrade to the in-process engine — the same
/// leaves, the same projection, only the reported source differs.
fn answer_read(workspace: &Path, r: &Read) -> Result<(EngineSource, Value), Fail> {
    if let Some(body) = try_daemon_read(workspace, r) {
        return Ok((EngineSource::Daemon, body));
    }
    Ok((EngineSource::Ephemeral, in_process_read(workspace, r)?))
}

/// The whole daemon path: socket, ensure-up, `hello` (v3, workspace-bound),
/// then the `read` op. `None` on ANY failure — including an op error, where
/// the in-process recompute is authoritative and mints the SAME typed refusal
/// for the exit triad.
fn try_daemon_read(workspace: &Path, r: &Read) -> Option<Value> {
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
        response.get("body").cloned()
    } else {
        None
    }
}

/// A `#Fragment` the user typed → the segment array the wire takes. It scopes
/// a HEADING subtree, so it is parsed as a heading path and nothing else: a
/// `^id` or a dewey ordinal in this position would be a subtree with no
/// descendants, which is a section read, not a scope.
fn frag_segments(frag: &str) -> Vec<wire::HpathSeg> {
    frag.split('/')
        .map(|h| wire::HpathSeg {
            h: h.to_owned(),
            n: None,
        })
        .collect()
}

impl Read {
    /// The wire `read` request this invocation maps onto. `display_path` is
    /// the PATH exactly as the user typed it (C's U4a2 contract: the engine
    /// renders the caller's spelling, it never invents one).
    fn request(&self) -> Value {
        let mut req = json!({
            "op": "read",
            "path": self.path,
            "display_path": self.path,
        });
        if let Some(mode) = &self.mode {
            req["mode"] = json!(mode);
        }
        // **The human/wire ingress door (U14, U7's ruled direction).** Both
        // selector fields are STRUCTURED on the wire, and this is where a
        // typed string becomes structure — once, at the edge, so nothing
        // inward of the CLI carries a joined address.
        if let Some(frag) = &self.frag {
            req["frag"] = json!(frag_segments(frag));
        }
        if !self.sections.is_empty() {
            let sels: Vec<wire::ReadSel> = self
                .sections
                .iter()
                .map(|s| wire::ReadSel::parse(s))
                .collect();
            req["sections"] = json!(sels);
        }
        req
    }
}

/// The degrade: load the ONE document from disk, answer through the SAME
/// composed-read leaf the daemon serves, then run the SAME v3 vocabulary
/// projection — warm and degrade bodies are byte-identical for the same state.
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
    let params = wire_serve::read::ReadParams {
        mode: r.mode.clone(),
        // The SAME conversion the wire request does — one door, two transports,
        // so warm and degrade cannot diverge on what a selector means.
        frag: r.frag.as_deref().map(frag_segments),
        sections: (!r.sections.is_empty())
            .then(|| r.sections.iter().map(|s| wire::ReadSel::parse(s)).collect()),
        display_path: Some(r.path.clone()),
        // §9 read provenance is the DAEMON's to stamp; the local CLI sends
        // none on both warm and degrade paths (symmetry with the wire call).
        actor: None,
    };
    // S6: no actor (above) and no session store — the local CLI mints no read
    // receipt on either path, and the CLI pin is local-operator-trusted (D16).
    // S10: this degrade path loads ONE document, not the corpus, so it cannot
    // color a pin whose target is another page — it decorates nothing. The
    // decorated face is the daemon's (the host that holds a corpus); `mrd read`
    // serves the stored spelling, which is also the spelling `mrd put` takes.
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
