//! `mrd put` — the batch write verb (M1 U1, ratified read/put naming).
//!
//! ```text
//! mrd put <PATH> [--dry] [--force] [--actor A] [--now T]
//!         [--if-fingerprint FP] [--receipt PATH#ANCHOR] [--json]  < edits.json
//! ```
//!
//! The edits ride STDIN in the wire §4.4 grammar VERBATIM — a JSON array of
//! `{target, edit, if_node_rev?}` where `target` is `{"hpath":[…]}` /
//! `{"anchor":"…"}` / `{"fm_key":"…"}` and `edit` is `{"match":{"old","new"}}`
//! or `{"put":{"at","text"}}` — strict-decoded, so an unknown key is a loud
//! exit 2, never ignored.
//!
//! Every put routes through THE production splice choke-point
//! ([`wire_serve::write::splice`], decision 0002 W1) in-process: the CAS
//! guards, the armed-plane gate (the workspace's OWN law — never caller
//! packs), and the D9 cross-process write flock are all inherited (A-S1); no
//! degrade path exists that could bypass them. Rule packs are the empty set —
//! exactly the production sidecar face (`verdicts` stay `[]`).
//!
//! Exit triad: 0 committed (or `--dry` validated) / 1 refused (`no_match`,
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
        dry: parsed.dry,
        force: parsed.force,
        edits,
        plan_edits: Vec::new(),
        // `mrd put` writes content; the pin verb is `mrd pin`.
        pin: None,
    };
    // seq 0, like the resident daemon (no epoch ring); the emitted DeltaFrame
    // has no subscriber here, so it is dropped with the outcome.
    let outcome =
        splice(&root, 0, &splice_args, &[], None).map_err(|e| engine::refusal_fail(&e))?;

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

    match parsed.format {
        Format::Json => {
            let value = json!({
                "workspace": resolved.workspace.display().to_string(),
                "put": body,
            });
            println!("{}", serde_json::to_string_pretty(&value).expect("json"));
        }
        Format::Human => print_human(&parsed, &body),
    }
    Ok(())
}

/// The human summary: what landed (or was rehearsed), at which fingerprint.
fn print_human(parsed: &Put, body: &Value) {
    let edits = body
        .pointer("/armed/edits")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    if parsed.dry {
        println!(
            "dry run: {} validated ({edits} edit(s)), nothing written",
            parsed.path
        );
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
    dry: bool,
    force: bool,
    format: Format,
}

impl Put {
    fn parse(args: &[String]) -> Result<Self, Fail> {
        let mut positional: Option<String> = None;
        let mut actor: Option<String> = None;
        let mut now: Option<String> = None;
        let mut receipt: Option<ReceiptAddr> = None;
        let mut if_fingerprint: Option<String> = None;
        let mut dry = false;
        let mut force = false;
        let mut json = false;
        let mut it = args.iter();
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "--json" => json = true,
                "--dry" => dry = true,
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
        Ok(Put {
            path,
            actor,
            now,
            receipt,
            if_fingerprint,
            dry,
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
    let edits: Vec<Edit> = serde_json::from_str(&raw)
        .map_err(|e| Fail::tool(format!("malformed edits JSON on stdin: {e}")))?;
    Ok(edits)
}
