//! `mrd put` — the batch write verb (M1 U1, ratified read/put naming).
//!
//! ```text
//! mrd put <PATH> [--dry | --validate] [--force] [--actor A] [--now T]
//!         [--if-fingerprint FP] [--receipt PATH#ANCHOR] [--json]  < edits.json
//! ```
//!
//! **The two rehearsals are ONE run and TWO faces (D3).** Both send
//! `dry: true` through the same choke-point, so neither can validate anything
//! the other would not. `--dry` SHOWS the change — a unified diff from the
//! file's current bytes to the candidate the rehearsal built. `--validate` is
//! the SILENT check: nothing on stdout and exit 0 when the rehearsal passes,
//! the engine's verbatim refusal at exit 1 when it does not. Passing both is a
//! contradiction (exit 2): one invocation cannot be both the loud and the
//! quiet face.
//!
//! The edits ride STDIN in the wire §4.4 grammar VERBATIM — a BARE JSON array
//! of `{target, edit, if_node_rev?}` where `target` is `{"hpath":[…]}` /
//! `{"anchor":"…"}` / `{"fm_key":"…"}` and `edit` is `{"match":{"old","new"}}`
//! or `{"put":{"at","text"}}` — strict-decoded, so an unknown key is a loud
//! exit 2, never ignored.
//!
//! **The array is the VALUE of §4.4's `edits` field, not the request object
//! around it.** §4.4 documents a wire request, where the array sits under an
//! `"edits"` key beside `id`/`op`/`path`; here those three are argv's, so
//! sending the whole envelope on stdin is a type error the decoder refuses
//! (dogfood 2026-08-04, run 14). [`read_stdin_edits`] names that exact mistake
//! when it sees it, because "expected a sequence" alone leaves a caller
//! re-reading the doc that misled them.
//!
//! Every put routes through THE production splice choke-point
//! ([`wire_serve::write::splice`], decision 0002 W1) in-process: the CAS
//! guards, the armed-plane gate (the workspace's OWN law — never caller
//! packs), and the D9 cross-process write flock are all inherited (A-S1); no
//! degrade path exists that could bypass them. Rule packs are the empty set —
//! exactly the production sidecar face (`verdicts` stay `[]`).
//!
//! Exit triad: 0 committed (or a rehearsal that passed) / 1 refused (`no_match`,
//! `not_unique`, `cas_mismatch`, `root_mismatch`, `workspace_busy`, an armed
//! gate refusal — the engine's verbatim message) / 2 bad invocation.

use std::io::Read as _;

use serde_json::{Value, json};
use wire::{Edit, Path as WirePath, ReceiptAddr, Root};
use wire_serve::write::{SpliceArgs, splice};

use crate::{Fail, Format, current_dir, engine};

/// Run `mrd put <PATH> [flags] < edits.json`.
///
/// # Errors
/// [`Fail`] — exit 2 on a bad invocation (bad flags, malformed stdin JSON, a
/// malformed `--now`, a `bad_request` refusal); exit 1 on any other engine
/// refusal, message verbatim.
pub(crate) fn dispatch(args: &[String]) -> Result<(), Fail> {
    let parsed = Put::parse(args)?;
    let edits = read_stdin_edits()?;
    let cwd = current_dir()?;
    let resolved = crate::resolve::resolve_runtime(&cwd).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace for {}: {e}",
            cwd.display()
        ))
    })?;
    let canonical = workspace::canonicalize(&resolved.workspace).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace {} ({e})",
            resolved.workspace.display()
        ))
    })?;
    let root = fs::WorkspaceRoot(canonical);

    let splice_args = SpliceArgs {
        id: None,
        origin: wire_serve::guard::Origin::InProcess,
        path: WirePath(parsed.path.clone()),
        actor: parsed.actor.clone(),
        now: parsed.now.clone(),
        receipt: parsed.receipt.clone(),
        if_root: parsed.if_fingerprint.clone().map(Root),
        dry: parsed.rehearsal(),
        force: parsed.force,
        edits,
        plan_edits: Vec::new(),
        // `mrd put` writes content; the pin verb is `mrd pin`.
        pin: None,
    };
    // seq 0, like the resident daemon (no epoch ring); the emitted DeltaFrame
    // has no subscriber here, so it is dropped with the outcome.
    let outcome =
        splice(&root, None, &splice_args, &[], None).map_err(|e| engine::refusal_fail(&e))?;

    // D3: the rehearsal preview, rendered from the candidate the choke-point
    // ALREADY built (§4.4 one-reparse law) rather than re-derived here — the
    // CLI holds no second implementation of what a batch does to a document.
    let diff = if parsed.dry {
        rehearsal_diff(&root, &parsed.path, outcome.candidate.as_deref())?
    } else {
        None
    };

    let body = serde_json::to_value(&outcome.body)
        .map_err(|e| Fail::tool(format!("cannot render the answer: {e}")))?;
    // The CLI speaks the v3 vocabulary (`fingerprint`, never bare `root`) —
    // the same lifted projection the daemon applies for a v3 session.
    let mut frame = json!({ "body": body });
    wire_serve::rev::project_response(&mut frame);
    let body = frame
        .as_object_mut()
        .and_then(|obj| obj.remove("body"))
        .unwrap_or(Value::Null);

    // The findings leg of the silent check: a passing rehearsal says nothing,
    // so anything the engine DID find has to leave through a non-zero exit or
    // it is lost. Rule packs are empty on this face, so an unforced rehearsal
    // carries no verdicts and this is the quiet path by construction.
    if parsed.validate {
        let verdicts = body
            .get("verdicts")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        if verdicts > 0 {
            return Err(Fail::findings(format!(
                "validate: {} finding(s) on {}, nothing written:\n{}",
                verdicts,
                parsed.path,
                serde_json::to_string_pretty(&body["verdicts"]).expect("json")
            )));
        }
    }

    match parsed.format {
        Format::Json => {
            let mut value = json!({
                "workspace": resolved.workspace.display().to_string(),
                "put": body,
            });
            if let Some(diff) = &diff {
                value["diff"] = json!(diff);
            }
            println!("{}", serde_json::to_string_pretty(&value).expect("json"));
        }
        // `--validate` passed, so it says NOTHING (D3): the exit code is the
        // whole answer, and a line of reassurance is what makes a silent check
        // stop being one.
        Format::Human if parsed.validate => {}
        Format::Human => print_human(&parsed, &body, diff.as_deref()),
    }
    Ok(())
}

/// The `--dry` diff: the file's CURRENT bytes → the candidate the rehearsal
/// built, through the one line-diff renderer in the tree
/// ([`wire_serve::ladder::unified_diff`], U11's primitive). `None` when the
/// batch changes no line — a rehearsal that would write the same bytes has no
/// change to show, and saying so is the honest answer.
fn rehearsal_diff(
    root: &fs::WorkspaceRoot,
    path: &str,
    candidate: Option<&str>,
) -> Result<Option<String>, Fail> {
    let Some(candidate) = candidate else {
        return Ok(None);
    };
    let doc = wire_serve::load_doc(root, &WirePath(path.to_owned()))
        .map_err(|e| engine::refusal_fail(&e))?;
    Ok(wire_serve::ladder::unified_diff(
        "current",
        "candidate",
        &doc.raw,
        candidate,
    ))
}

/// The human summary: what landed (or was rehearsed), at which fingerprint.
fn print_human(parsed: &Put, body: &Value, diff: Option<&str>) {
    let edits = body
        .pointer("/armed/edits")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    if parsed.dry {
        println!(
            "dry run: {} ({edits} edit(s)), nothing written",
            parsed.path
        );
        match diff {
            Some(diff) => print!("{diff}"),
            None => println!("no change: the candidate is byte-identical"),
        }
        return;
    }
    let after = body
        .get("fingerprint_after")
        .and_then(Value::as_str)
        .unwrap_or("?");
    println!("committed {} ({edits} edit(s))", parsed.path);
    println!("  fingerprint: {after}");
    if let Some(receipt) = body
        .get("receipt")
        .and_then(|r| r.get("anchor"))
        .and_then(Value::as_str)
    {
        println!("  receipt: ^{receipt}");
    }
}

/// The parsed `put` invocation.
struct Put {
    path: String,
    actor: Option<String>,
    now: Option<String>,
    receipt: Option<ReceiptAddr>,
    if_fingerprint: Option<String>,
    /// `--dry`: rehearse and SHOW the diff.
    dry: bool,
    /// `--validate`: rehearse and say nothing (D3).
    validate: bool,
    force: bool,
    format: Format,
}

impl Put {
    /// Either face means the same run: everything except disk.
    fn rehearsal(&self) -> bool {
        self.dry || self.validate
    }
}

impl Put {
    fn parse(args: &[String]) -> Result<Self, Fail> {
        let mut positional: Option<String> = None;
        let mut actor: Option<String> = None;
        let mut now: Option<String> = None;
        let mut receipt: Option<ReceiptAddr> = None;
        let mut if_fingerprint: Option<String> = None;
        let mut dry = false;
        let mut validate = false;
        let mut force = false;
        let mut json = false;
        let mut it = args.iter();
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "--json" => json = true,
                "--dry" => dry = true,
                "--validate" => validate = true,
                "--force" => force = true,
                "--actor" => {
                    let value = it
                        .next()
                        .ok_or_else(|| Fail::tool("--actor needs a value".to_owned()))?;
                    actor = Some(value.clone());
                }
                "--now" => {
                    let value = it
                        .next()
                        .ok_or_else(|| Fail::tool("--now needs a value".to_owned()))?;
                    // The engine reads no wall clock and the wire dispatch
                    // strict-decode is not on this path — validate the §9
                    // format law HERE, exactly as the server would.
                    if !wire::now_is_rfc3339(value) {
                        return Err(Fail::tool(format!(
                            "--now must be RFC 3339 (YYYY-MM-DDTHH:MM:SS[.frac](Z|±HH:MM)): {value}"
                        )));
                    }
                    now = Some(value.clone());
                }
                "--if-fingerprint" => {
                    let value = it
                        .next()
                        .ok_or_else(|| Fail::tool("--if-fingerprint needs a value".to_owned()))?;
                    if_fingerprint = Some(value.clone());
                }
                "--receipt" => {
                    let value = it
                        .next()
                        .ok_or_else(|| Fail::tool("--receipt needs a value".to_owned()))?;
                    let Some((rpath, anchor)) = value.split_once('#') else {
                        return Err(Fail::tool(format!(
                            "--receipt wants PATH#ANCHOR (a block anchor address): {value}"
                        )));
                    };
                    if rpath.is_empty() || anchor.is_empty() {
                        return Err(Fail::tool(format!(
                            "--receipt wants PATH#ANCHOR (both parts non-empty): {value}"
                        )));
                    }
                    receipt = Some(ReceiptAddr {
                        path: WirePath(rpath.to_owned()),
                        anchor: anchor.to_owned(),
                    });
                }
                flag if flag.starts_with('-') => {
                    return Err(Fail::tool(format!("unknown flag: {flag}")));
                }
                value if positional.is_none() => positional = Some(value.to_owned()),
                value => return Err(Fail::tool(format!("unexpected argument: {value}"))),
            }
        }
        let Some(path) = positional else {
            return Err(Fail::tool("put needs a PATH".to_owned()));
        };
        if dry && validate {
            return Err(Fail::tool(
                "--dry and --validate are the two faces of ONE rehearsal: --dry shows the \
                 diff, --validate says nothing. Pass one."
                    .to_owned(),
            ));
        }
        Ok(Put {
            path,
            actor,
            now,
            receipt,
            if_fingerprint,
            dry,
            validate,
            force,
            format: if json { Format::Json } else { Format::Human },
        })
    }
}

/// Read and strict-decode the edits array from stdin (the wire §4.4 grammar).
fn read_stdin_edits() -> Result<Vec<Edit>, Fail> {
    let mut raw = String::new();
    std::io::stdin()
        .read_to_string(&mut raw)
        .map_err(|e| Fail::tool(format!("cannot read edits from stdin: {e}")))?;
    if raw.trim().is_empty() {
        return Err(Fail::tool(
            "put wants the edits JSON on stdin — a §4.4 array like \
             [{\"target\":{\"hpath\":[{\"h\":\"Title\"}]},\"edit\":{\"match\":{\"old\":\"a\",\"new\":\"b\"}}}]"
                .to_owned(),
        ));
    }
    let edits: Vec<Edit> = serde_json::from_str(&raw).map_err(|e| {
        Fail::tool(format!(
            "malformed edits JSON on stdin: {e}{}",
            envelope_hint(&raw)
        ))
    })?;
    Ok(edits)
}

/// The one refusal worth naming: stdin carried the whole §4.4 REQUEST object
/// instead of the array under its `edits` key.
///
/// It fires only when the bytes really are that — an object with an `edits`
/// key — so the hint can never mis-diagnose some other malformed input. Any
/// other shape gets the decoder's own message and nothing added to it.
fn envelope_hint(raw: &str) -> &'static str {
    let is_envelope = serde_json::from_str::<Value>(raw)
        .ok()
        .and_then(|v| v.get("edits").cloned())
        .is_some_and(|edits| edits.is_array());
    if is_envelope {
        " — stdin takes the BARE edits ARRAY, not the wire §4.4 request object: \
         send the value of its \"edits\" field (id / op / path are argv's here)"
    } else {
        ""
    }
}
