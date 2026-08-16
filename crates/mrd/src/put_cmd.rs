//! `mrd put` — the batch write verb.
//!
//! ```text
//! mrd put <PATH> [--dry | --validate] [--force] [--actor A] [--now T]
//!         [--if-fingerprint FP] [--scope PATH] [--receipt PATH#ANCHOR] [--json]  < edits.json
//! ```
//!
//! `--scope` narrows the `--if-fingerprint` premise to the named node
//! (wire-contract §5.4): FP is then that node's scoped token from the §4.7
//! mint arm, not the world value, so a disjoint sibling's birth no longer
//! refuses the put. Two walls guard it here, both exit 2 before any engine
//! write: `--scope` without `--if-fingerprint` is half a premise (the §5.4
//! pair law, enforced at parse), and a daemon whose hello does not serve
//! `scoped-guards` cannot check a scoped premise — the taught refusal fires
//! instead of a strict-wall `bad_request` from a field it never negotiated.
//!
//! The two rehearsals are one run and two faces: both send `dry: true` through the same
//! choke-point, so neither can validate anything the other would not. `--dry` is the daemon
//! rehearsal (nothing written). `--validate` is the silent check: nothing on stdout and exit 0
//! when the rehearsal passes, the engine's verbatim refusal at exit 1 when it does not. Passing
//! both is a contradiction (exit 2).
//!
//! The edits ride stdin in the wire §4.4 grammar — a bare JSON array of
//! `{target, edit, if_node_rev?}` where `target` is `{"hpath":[…]}` / `{"anchor":"…"}` /
//! `{"fm_key":"…"}` and `edit` is `{"match":{"old","new"}}` or `{"put":{"at","text"}}` —
//! strict-decoded, so an unknown key is a loud exit 2, never ignored. The array is the value of
//! §4.4's `edits` field, not the request object around it: `id`/`op`/`path` are argv's here, so
//! sending the whole envelope is a type error [`read_stdin_edits`] names explicitly.
//!
//! Every put is a wire `splice` to the running daemon ([`crate::write_ipc`]). There is
//! no in-process publication path: a down daemon is a taught refusal, never a local
//! write. The daemon inherits the CAS guards and the armed-plane gate. A guardless
//! put is a wire client, so fingerprint-or-force applies (`--force` or `if_node_rev`).
//! Rule packs are the empty set, so `verdicts` stay `[]` unless `--force` names a
//! bypass.
//!
//! Exit triad: 0 committed (or a rehearsal that passed) / 1 refused (EVERY engine
//! refusal — `no_match`, `not_unique`, `cas_mismatch`, `root_mismatch`,
//! `guard_required`, `bad_request`, an armed gate refusal — the engine's verbatim
//! message) / 2 bad invocation (the CLI's own refusals, including a down daemon,
//! before any engine write).
//!
//! Under `--json` BOTH legs answer on stdout: a commit the `{workspace, put}`
//! frame, an engine refusal the `{workspace, error}` envelope ([`refusal`]) —
//! never an empty stdout a machine consumer cannot branch on.

use std::io::Read as _;

use serde_json::{Value, json};
use wire::{ErrorBody, Path as WirePath, ReceiptAddr};

use crate::{Fail, Format, current_dir, engine, write_ipc};

/// Run `mrd put <PATH> [flags] < edits.json`. Errors [`Fail`] — exit 2 on a bad invocation (bad
/// flags, malformed stdin JSON, a malformed `--now` — the CLI's own refusals, before any engine
/// contact); exit 1 on every engine refusal, `bad_request` included, message verbatim.
pub(crate) fn dispatch(args: &[String]) -> Result<(), Fail> {
    let parsed = Put::parse(args)?;
    let edits = read_stdin_edits()?;
    // The stdin Value is what rides the wire — re-serializing the decoded
    // Vec<Edit> is a second shape. Decode already proved the §4.4 wall.
    let cwd = current_dir()?;
    let resolved = crate::resolve::resolve_runtime(&cwd).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace for {}: {e}",
            cwd.display()
        ))
    })?;
    let mut request = json!({
        "op": "splice",
        "path": parsed.path,
        "edits": edits,
    });
    if let Some(actor) = &parsed.actor {
        request["actor"] = json!(actor);
    }
    if let Some(now) = &parsed.now {
        request["now"] = json!(now);
    }
    if let Some(fp) = &parsed.if_fingerprint {
        request["if_fingerprint"] = json!(fp);
    }
    if let Some(scope) = &parsed.scope {
        request["scope"] = json!(scope);
    }
    if let Some(receipt) = &parsed.receipt {
        request["receipt"] = json!({"path": receipt.path.0, "anchor": receipt.anchor});
    }
    if parsed.rehearsal() {
        request["dry"] = json!(true);
    }
    if parsed.force {
        request["force"] = json!(true);
    }

    let mut door = write_ipc::connect(&resolved.workspace)?;
    // The cap wall, client half (§3.2): a scoped premise rides only when the
    // connect-time hello advertised the family. Refusing HERE — before any
    // engine write — is the taught refusal; sending anyway would draw the
    // strict wall's `bad_request` for a field this daemon never negotiated.
    if parsed.scope.is_some() && !door.has_cap(engine::SCOPED_GUARDS_CAP) {
        return Err(Fail::tool(format!(
            "--scope names a scoped premise (wire-contract §5.4), but this daemon's hello \
             does not serve the `{}` cap, so it cannot check a premise at that grain — \
             nothing was sent and nothing was written.\n\
             Why: the cap is family-whole discovery honesty (§3.2): a daemon either serves \
             the whole scoped-premise family or refuses every guard-family field at its \
             strict wall; the client refuses first, with this teaching, instead of drawing \
             that `bad_request`.\n\
             Fixes — run whichever fits your case:\n\
               - retry without --scope: `--if-fingerprint` alone is the world-grain premise \
             this daemon does check.\n\
               - when the daemon should serve the family: restart it so a build that \
             advertises `{}` binds the socket, then mint the scoped token again \
             (`fingerprint {{scope}}`, §4.7) — a world token does not become scoped by \
             renaming the flag.",
            engine::SCOPED_GUARDS_CAP,
            engine::SCOPED_GUARDS_CAP,
        )));
    }
    let body = write_ipc::call(&mut door, &request)
        .map_err(|e| refusal(&parsed, &resolved.workspace, &e))?;
    let body = write_ipc::project_body(&body);

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
            let value = json!({
                "workspace": resolved.workspace.display().to_string(),
                "put": body,
            });
            println!("{}", serde_json::to_string_pretty(&value).expect("json"));
        }
        // `--validate` says nothing: the exit code is the whole answer.
        Format::Human if parsed.validate => {}
        Format::Human => print_human(&parsed, &body),
    }
    Ok(())
}

/// An engine refusal, on both faces — [`engine::json_refusal`], which owns the envelope for
/// every `--json` face rather than for this verb alone.
fn refusal(parsed: &Put, workspace: &std::path::Path, error: &ErrorBody) -> Fail {
    engine::json_refusal(parsed.format, workspace, error)
}

/// The human summary: what landed (or was rehearsed), at which fingerprint — and one line per
/// FIRED intent. Without those lines an armed workspace commits identically to an unarmed one
/// on this face, and the operator who armed the plane can only see it fire through `--json`
/// (`put.armed.effects[]`). The line speaks the engine's vocabulary: what the write armed,
/// never that anything was delivered, and the receipt address VERBATIM — it is the pairing key
/// the delivery faces echo as `correlation`.
///
/// `--dry` no longer prints a local unified candidate-diff: that field was
/// in-process-only and never a wire fact. The daemon rehearsal is the authority.
fn print_human(parsed: &Put, body: &Value) {
    let edits = body
        .pointer("/armed/edits")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    if parsed.dry {
        println!(
            "dry run: {} ({edits} edit(s)), nothing written",
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
    let fired = body
        .pointer("/armed/effects")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|envelope| envelope.get("intents").and_then(Value::as_array))
        .flatten();
    for intent in fired {
        let rule = intent.get("rule_id").and_then(Value::as_str).unwrap_or("?");
        let action = intent.get("action").and_then(Value::as_str).unwrap_or("?");
        let receipt = intent.get("receipt").and_then(Value::as_str).unwrap_or("?");
        match intent.get("target").and_then(Value::as_str) {
            Some(target) => println!("  fired: {rule} {action} → {target} (receipt {receipt})"),
            None => println!("  fired: {rule} {action} (receipt {receipt})"),
        }
    }
}

/// The parsed `put` invocation.
struct Put {
    path: String,
    actor: Option<String>,
    now: Option<String>,
    receipt: Option<ReceiptAddr>,
    if_fingerprint: Option<String>,
    /// `--scope`: the node the `--if-fingerprint` premise binds (§5.4). The
    /// pair law holds at parse: present only beside `if_fingerprint`.
    scope: Option<String>,
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
    /// `put`'s argv parse. It holds NO hand-split address of its own any
    /// more — the `--receipt` split moved to [`parse_receipt`], and the
    /// `crates/addr` ingress row moved with it, because that scanner
    /// attributes a needle to its enclosing function.
    ///
    /// HEADROOM IS SINGLE DIGITS: 92 non-comment lines against
    /// `clippy::too_many_lines`' threshold of 100 (97 raw), measured
    /// 2026-08-16. **A new flag arm extracts into its own function rather
    /// than inlining here — and the extracted function gets its own row in
    /// `PINNED` if it splits an address.** Inlining to keep the ingress list
    /// short is the trap: it trips the lint (CI 738) exactly as extracting
    /// without a pin trips the ingress guard (CI 732). See
    /// [`parse_receipt`] for the worked example.
    fn parse(args: &[String]) -> Result<Self, Fail> {
        let mut positional: Option<String> = None;
        let mut actor: Option<String> = None;
        let mut now: Option<String> = None;
        let mut receipt: Option<ReceiptAddr> = None;
        let mut if_fingerprint: Option<String> = None;
        let mut scope: Option<String> = None;
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
                "--scope" => {
                    let value = it
                        .next()
                        .ok_or_else(|| Fail::tool("--scope needs a value".to_owned()))?;
                    scope = Some(value.clone());
                }
                "--receipt" => {
                    let value = it
                        .next()
                        .ok_or_else(|| Fail::tool("--receipt needs a value".to_owned()))?;
                    receipt = Some(parse_receipt(value)?);
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
        // The §5.4 pair law, this face's own wall: a scope names WHERE a
        // premise binds and carries no token, so alone it is half a premise.
        // The engine would refuse the same shape; this refusal costs no dial.
        if scope.is_some() && if_fingerprint.is_none() {
            return Err(Fail::tool(
                "--scope names the premise's node but carries no token — pair it with \
                 --if-fingerprint holding that node's scoped token (minted by the §4.7 \
                 `fingerprint {scope}` arm; wire-contract §5.4). A scope with no \
                 fingerprint is half a premise."
                    .to_owned(),
            ));
        }
        Ok(Put {
            path,
            actor,
            now,
            receipt,
            if_fingerprint,
            scope,
            dry,
            validate,
            force,
            format: if json { Format::Json } else { Format::Human },
        })
    }
}

/// `--receipt PATH#ANCHOR` → the typed address, or the CLI's own refusal
/// (exit 2, before any engine contact). Both parts are required: a half
/// address names no anchor to pair a delivery against.
///
/// A SEPARATE FUNCTION ON PURPOSE, and it must stay pinned. `crates/addr`'s
/// ingress enumeration lists every hand-split address site BY FUNCTION, so
/// this one is registered as `("crates/mrd/src/put_cmd.rs", "fn parse_receipt",
/// Class::CliArgv)`. Inlining it back into `fn parse` satisfies that guard by
/// accident but pushes `fn parse` past `clippy::too_many_lines` (118 > 100) —
/// the two invariants pull opposite ways, and this split plus the pin is the
/// arrangement that satisfies both. CI 732 (unpinned ingress) and CI 738
/// (over-long `fn parse`) are the two reds that mapped the vise.
fn parse_receipt(value: &str) -> Result<ReceiptAddr, Fail> {
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
    Ok(ReceiptAddr {
        path: WirePath(rpath.to_owned()),
        anchor: anchor.to_owned(),
    })
}

/// The working batch every door teaches — ONE spelling shared by the empty-stdin refusal and
/// the malformed-decode refusal, so the taught grammar cannot drift between them.
const WORKING_BATCH: &str = "[{\"target\":{\"hpath\":[{\"h\":\"Title\"}]},\"edit\":{\"match\":{\"old\":\"a\",\"new\":\"b\"}}}]";

/// Read and strict-decode the edits array from stdin (the wire §4.4 grammar).
fn read_stdin_edits() -> Result<Value, Fail> {
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
    let value: Value = serde_json::from_str(&raw)
        .map_err(|e| refuse_stdin("malformed edits JSON on stdin", &e.to_string(), &raw))?;
    // The engine's own edit decoder (§3.2 grain law): the strict wall holds at
    // EVERY object of the batch — the edit object, its shape body, its target,
    // each hpath segment — and names the closed set it checked against. A serde
    // decode here was strict only where the types happened to be untagged, so a
    // typo'd guard field (`if_rev` for `if_node_rev`) was DROPPED and the write
    // armed unguarded. One decoder, both doors, no drift.
    //
    // `ShapeOnly` because the exits differ here and only here: this seam's own
    // refusal is exit 2, and a VALUE law (§2.4's block-id charset) is not this
    // seam's to judge — the engine refuses that one at exit 1 with its
    // structured frame, which is the exit triad `docs/status.md` states.
    wire_serve::decode::decode_edits(&value, wire_serve::decode::Laws::ShapeOnly).map_err(|e| {
        refuse_stdin(
            "the edits on stdin are not the §4.4 batch shape",
            e.message
                .as_deref()
                .unwrap_or("the §4.4 edit grammar was not met"),
            &raw,
        )
    })?;
    if value.as_array().is_none_or(Vec::is_empty) {
        return Err(Fail::tool(format!(
            "put wants a non-empty edits array — a §4.4 batch like {WORKING_BATCH}"
        )));
    }
    Ok(value)
}

/// The one stdin-refusal shape, shared by the JSON-syntax leg and the
/// strict-decode leg: `lead` names WHICH law the bytes missed — the two are not
/// the same news, and calling a batch that parsed cleanly "malformed JSON" tells
/// the caller to go look at a byte that is fine. The decoder's own words then
/// locate the byte or the field, the grammar clause is what the refused caller
/// needs, and the nothing-happened clause is unconditional and one spelling
/// across the family — both legs run before the workspace is resolved and before
/// any splice, so a refusal here has had zero engine contact (exit 2, the CLI's
/// own).
fn refuse_stdin(lead: &str, reason: &str, raw: &str) -> Fail {
    Fail::tool(format!(
        "{lead}: {reason}{}. The §4.4 grammar: target is \
         {{\"hpath\":[{{\"h\":\"Raw Title\"}}]}} / {{\"anchor\":\"block-id\"}} / \
         {{\"fm_key\":\"key\"}}; edit is the NESTED {{\"match\":{{\"old\":\"…\",\"new\":\"…\"}}}} \
         or {{\"put\":{{\"at\":\"end\",\"text\":\"…\"}}}} — a working batch: {WORKING_BATCH}. \
         Nothing was parsed and nothing was written.",
        envelope_hint(raw)
    ))
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
