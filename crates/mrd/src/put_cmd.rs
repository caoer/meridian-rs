//! `mrd put` — the batch write verb.
//!
//! ```text
//! mrd put <PATH> [--dry | --validate] [--force] [--actor A] [--now T]
//!         [--if-fingerprint FP] [--receipt PATH#ANCHOR] [--json]  < edits.json
//! ```
//!
//! The two rehearsals are one run and two faces: both send `dry: true` through the same
//! choke-point, so neither can validate anything the other would not. `--dry` shows the change —
//! a unified diff from the file's current bytes to the candidate. `--validate` is the silent
//! check: nothing on stdout and exit 0 when the rehearsal passes, the engine's verbatim refusal
//! at exit 1 when it does not. Passing both is a contradiction (exit 2).
//!
//! The edits ride stdin in the wire §4.4 grammar — a bare JSON array of
//! `{target, edit, if_node_rev?}` where `target` is `{"hpath":[…]}` / `{"anchor":"…"}` /
//! `{"fm_key":"…"}` and `edit` is `{"match":{"old","new"}}` or `{"put":{"at","text"}}` —
//! strict-decoded, so an unknown key is a loud exit 2, never ignored. The array is the value of
//! §4.4's `edits` field, not the request object around it: `id`/`op`/`path` are argv's here, so
//! sending the whole envelope is a type error [`read_stdin_edits`] names explicitly.
//!
//! Every put routes through the production splice choke-point ([`wire_serve::write::splice`])
//! in-process, inheriting the CAS guards, the armed-plane gate (the workspace's own law, never
//! caller packs), and the cross-process write flock. Rule packs are the empty set, so `verdicts`
//! stay `[]`.
//!
//! Exit triad: 0 committed (or a rehearsal that passed) / 1 refused (EVERY engine
//! refusal — `no_match`, `not_unique`, `cas_mismatch`, `root_mismatch`,
//! `workspace_busy`, `bad_request`, an armed gate refusal — the engine's verbatim
//! message) / 2 bad invocation (the CLI's own refusals, before any engine contact).
//!
//! Under `--json` BOTH legs answer on stdout: a commit the `{workspace, put}`
//! frame, an engine refusal the `{workspace, error}` envelope ([`refusal`]) —
//! never an empty stdout a machine consumer cannot branch on.

use std::io::Read as _;

use serde_json::{Value, json};
use wire::{Edit, ErrorBody, Path as WirePath, ReceiptAddr, Root};
use wire_serve::write::{SpliceArgs, splice};

use crate::{Fail, Format, current_dir, engine};

/// Run `mrd put <PATH> [flags] < edits.json`. Errors [`Fail`] — exit 2 on a bad invocation (bad
/// flags, malformed stdin JSON, a malformed `--now` — the CLI's own refusals, before any engine
/// contact); exit 1 on every engine refusal, `bad_request` included, message verbatim.
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
    let outcome = splice(&root, None, &splice_args, &[], None)
        .map_err(|e| refusal(&parsed, &resolved.workspace, &e))?;

    // The rehearsal preview renders the candidate the choke-point already built (§4.4 one-reparse
    // law); the CLI holds no second implementation of what a batch does to a document.
    let diff = if parsed.dry {
        rehearsal_diff(&root, &parsed.path, outcome.candidate.as_deref())
            .map_err(|e| refusal(&parsed, &resolved.workspace, &e))?
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

    // The findings leg of the silent check: a passing rehearsal says nothing, so anything the
    // engine found has to leave through a non-zero exit or it is lost.
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
        // `--validate` says nothing: the exit code is the whole answer.
        Format::Human if parsed.validate => {}
        Format::Human => print_human(&parsed, &body, diff.as_deref()),
    }
    Ok(())
}

/// An engine refusal, on both faces. Under `--json` the machine face gets the refusal envelope
/// on stdout — `{workspace, error}`, the engine's error body in the v3 vocabulary (the same
/// lifted projection the commit frame gets) — so a machine consumer branches on the frame,
/// exactly as it does for `mrd script --json` on a fault. The human diagnostic still rides
/// stderr via the returned [`Fail`], and the exit triad is untouched.
fn refusal(parsed: &Put, workspace: &std::path::Path, error: &ErrorBody) -> Fail {
    if matches!(parsed.format, Format::Json) {
        let mut frame = json!({
            "error": serde_json::to_value(error).expect("json"),
        });
        wire_serve::rev::project_response(&mut frame);
        let error_v3 = frame
            .as_object_mut()
            .and_then(|obj| obj.remove("error"))
            .unwrap_or(Value::Null);
        let value = json!({
            "workspace": workspace.display().to_string(),
            "error": error_v3,
        });
        println!("{}", serde_json::to_string_pretty(&value).expect("json"));
    }
    engine::refusal_fail(error)
}

/// The `--dry` diff: the file's current bytes → the candidate the rehearsal built, through the
/// one line-diff renderer in the tree ([`wire_serve::ladder::unified_diff`]). `None` when the
/// batch changes no line. Errors the engine's refusal body, so the caller keeps both faces.
fn rehearsal_diff(
    root: &fs::WorkspaceRoot,
    path: &str,
    candidate: Option<&str>,
) -> Result<Option<String>, Box<ErrorBody>> {
    let Some(candidate) = candidate else {
        return Ok(None);
    };
    let doc = wire_serve::load_doc(root, &WirePath(path.to_owned()))?;
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
    /// `--dry`: rehearse and show the diff.
    dry: bool,
    /// `--validate`: rehearse and say nothing.
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
                    // The wire dispatch strict-decode is not on this path, so validate the §9
                    // format law here, exactly as the server would.
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

/// The working batch every door teaches — ONE spelling shared by the empty-stdin refusal and
/// the malformed-decode refusal, so the taught grammar cannot drift between them.
const WORKING_BATCH: &str = "[{\"target\":{\"hpath\":[{\"h\":\"Title\"}]},\"edit\":{\"match\":{\"old\":\"a\",\"new\":\"b\"}}}]";

/// Read and strict-decode the edits array from stdin (the wire §4.4 grammar).
fn read_stdin_edits() -> Result<Vec<Edit>, Fail> {
    let mut raw = String::new();
    std::io::stdin()
        .read_to_string(&mut raw)
        .map_err(|e| Fail::tool(format!("cannot read edits from stdin: {e}")))?;
    if raw.trim().is_empty() {
        return Err(Fail::tool(format!(
            "put wants the edits JSON on stdin — a §4.4 array like {WORKING_BATCH}"
        )));
    }
    // Shape advice is conditional ([`envelope_hint`]); the nothing-happened clause is not — the
    // decode runs before the workspace is resolved and before any splice, so a refusal here has
    // had zero engine contact. The decoder's own words locate the byte; the grammar clause is
    // what the refused caller actually needs (G-P2-6: serde's variant names taught nothing).
    let edits: Vec<Edit> = serde_json::from_str(&raw).map_err(|e| {
        Fail::tool(format!(
            "malformed edits JSON on stdin: {e}{}. The §4.4 grammar: target is \
             {{\"hpath\":[{{\"h\":\"Raw Title\"}}]}} / {{\"anchor\":\"block-id\"}} / \
             {{\"fm_key\":\"key\"}}; edit is the NESTED {{\"match\":{{\"old\":\"…\",\"new\":\"…\"}}}} \
             or {{\"put\":{{\"at\":\"end\",\"text\":\"…\"}}}} — a working batch: {WORKING_BATCH}. \
             Nothing was parsed and nothing was written.",
            envelope_hint(&raw)
        ))
    })?;
    Ok(edits)
}

/// The one refusal worth naming: stdin carried the whole §4.4 request object instead of the
/// array under its `edits` key. Fires only when the bytes really are that — an object with an
/// `edits` array — so it can never mis-diagnose some other malformed input.
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
