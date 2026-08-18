//! `mrd put` — the batch write verb.
//!
//! ```text
//! mrd put <PATH> [--dry | --validate] [--force] [--actor A] [--now T]
//!         [--if-fingerprint FP] [--scope PATH | --scope-bytes B64]
//!         [--receipt PATH#ANCHOR] [--json]  < edits.json
//! ```
//!
//! `--scope` narrows the `--if-fingerprint` premise to the named node
//! (wire-contract §5.4): FP is then that node's scoped token from the §4.7
//! mint arm, not the world value, so a disjoint sibling's birth no longer
//! refuses the put. Three walls guard it here, all exit 2 before any engine
//! write: `--scope` without `--if-fingerprint` is half a premise (the §5.4
//! pair law, enforced at parse); a `--scope` spelling the §1 path law refuses
//! teaches the family refusal before any dial ([`admit_scope`] — the engine
//! would refuse the same shape message-less, echoing a path that cannot name
//! the flag); and a daemon whose hello does not serve `scoped-guards` cannot
//! check a scoped premise — the taught refusal fires instead of a
//! strict-wall `bad_request` from a field it never negotiated.
//!
//! `--scope` takes the agent-plane `[root:]path` spelling (address-grammar
//! §4.1 colon law): a head-colon scope resolves through the one CLI seam
//! ([`crate::rooted`]) and is accepted exactly when the named root binds the
//! workspace this put writes — the spelling a rooted §4.7 mint echoes pastes
//! beside its token, and the wire carries the rel half only. Any other
//! landing is an address answer at exit 1 (`{workspace, error}` under
//! `--json`): a bound root elsewhere names both workspaces, an unbound root
//! enumerates what does bind, a `#` fragment refuses at path grain — never
//! the §5.5 coverage refusal a literal send used to draw (card
//! put-scope-rejects-rooted-mint-echo). The write TARGET keeps its
//! workspace-relative grammar (§4.5 D11).
//!
//! `--scope-bytes B64` is the same premise for a node whose name the UTF-8
//! `Path` noun cannot carry: B64 is base64url over the raw path bytes, and FP
//! is the token the §4.7 `fingerprint {scope_bytes}` mint echoed. On the wire
//! the pair rides as ONE `guards[]` entry — `scope_bytes` is a top-level
//! field on NO write door (the §5.4 field matrix), and a top-level
//! `if_fingerprint` beside the entry would arm a second, world-grain premise
//! with a token minted for the scoped node. Exactly one of
//! `--scope`/`--scope-bytes` per put ([`premise_flag_walls`]); the pair law
//! and the cap wall hold identically; the §1 path-law wall does not apply
//! (raw bytes are the names that law's noun cannot spell), so the face
//! refuses only the empty spelling and an undecodable base64url is the
//! engine's taught refusal at exit 1.
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
    let mut parsed = Put::parse(args)?;
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
    // The rooted lane on the WRITE TARGET (§4.1 colon law, 2026-08-18
    // rooted-refs-everywhere): a head-colon PATH names a page of the NAMED
    // root, so the write dials the daemon with THAT workspace at the hello and
    // the rel half rides the wire as `path` — the same split the rooted
    // `--scope` already rides ("the rel half rides the wire"). The daemon
    // serves the attached workspace exactly as if the caller stood there, so
    // the TARGET tree's armed gates fire and its receipts land home. A `#`
    // fragment refuses first (this door's own `--scope` stance): a write
    // binds a file, and silently stripping the fragment would write the whole
    // file while the caller named a section.
    let workspace = if crate::rooted::is_rooted(&parsed.path) {
        if parsed.path.contains('#') {
            let mut error = ErrorBody::new(wire::ErrorCode::BadPath);
            error.path = Some(WirePath(parsed.path.clone()));
            error.message = Some(format!(
                "{} carries a `#` fragment, and a write binds at path grain — a fragment \
                 addresses a section, not a file. Name the section in the edit selectors, \
                 not the path. {PUT_CONSEQUENCE}",
                parsed.path
            ));
            return Err(engine::json_refusal(
                parsed.format,
                &resolved.workspace,
                &error,
            ));
        }
        match crate::rooted::resolve(&parsed.path, "put", PUT_CONSEQUENCE) {
            Ok((rel, rooted)) => {
                parsed.display = Some(std::mem::replace(&mut parsed.path, rel));
                rooted.workspace
            }
            // The refusal frames with the workspace the caller stands in —
            // no target workspace exists to name.
            Err(error) => {
                return Err(engine::json_refusal(
                    parsed.format,
                    &resolved.workspace,
                    &error,
                ));
            }
        }
    } else {
        resolved.workspace
    };
    admit_scope(&mut parsed, &workspace)?;
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
    // The raw-byte premise rides as ONE guards[] entry carrying its token:
    // `scope_bytes` is a top-level field on NO write door (§5.4 matrix), and
    // top-level `if_fingerprint` beside the entry would arm a SECOND,
    // world-grain premise with a token minted for the scoped node. The parse
    // walls guarantee the pairing: scope_bytes never rides fingerprint-less
    // and never beside `scope`.
    if let (Some(b64), Some(fp)) = (&parsed.scope_bytes, &parsed.if_fingerprint) {
        request["guards"] = json!([{ "scope_bytes": b64, "fingerprint": fp }]);
    } else {
        if let Some(fp) = &parsed.if_fingerprint {
            request["if_fingerprint"] = json!(fp);
        }
        if let Some(scope) = &parsed.scope {
            request["scope"] = json!(scope);
        }
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
    attach_fields(&parsed, &mut request);

    let mut door = write_ipc::connect(&workspace)?;
    // The cap wall, client half (§3.2): a scoped premise rides only when the
    // connect-time hello advertised the family. Refusing HERE — before any
    // engine write — is the taught refusal; sending anyway would draw the
    // strict wall's `bad_request` for a field this daemon never negotiated.
    let scoped_flag = match (&parsed.scope, &parsed.scope_bytes) {
        (Some(_), _) => Some(("--scope", "scope")),
        (None, Some(_)) => Some(("--scope-bytes", "scope_bytes")),
        (None, None) => None,
    };
    if let Some((flag, arm)) = scoped_flag
        && !door.has_cap(engine::SCOPED_GUARDS_CAP)
    {
        return Err(Fail::tool(format!(
            "{flag} names a scoped premise (wire-contract §5.4), but this daemon's hello \
             does not serve the `{}` cap, so it cannot check a premise at that grain — \
             nothing was sent and nothing was written.\n\
             Why: the cap is family-whole discovery honesty (§3.2): a daemon either serves \
             the whole scoped-premise family or refuses every guard-family field at its \
             strict wall; the client refuses first, with this teaching, instead of drawing \
             that `bad_request`.\n\
             Fixes — run whichever fits your case:\n\
               - retry without {flag}: `--if-fingerprint` alone is the world-grain premise \
             this daemon does check.\n\
               - when the daemon should serve the family: restart it so a build that \
             advertises `{}` binds the socket, then mint the scoped token again \
             (`fingerprint {{{arm}}}`, §4.7) — a world token does not become scoped by \
             renaming the flag.",
            engine::SCOPED_GUARDS_CAP,
            engine::SCOPED_GUARDS_CAP,
        )));
    }
    fields_cap_wall(&parsed, &door)?;
    let body = write_ipc::call(&mut door, &request)
        .map_err(|e| refusal(&parsed, &workspace, &e))?;
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
                parsed.display(),
                serde_json::to_string_pretty(&body["verdicts"]).expect("json")
            )));
        }
    }

    match parsed.format {
        Format::Json => {
            let value = json!({
                "workspace": workspace.display().to_string(),
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

/// Attach the § A.2.1 `fields` passthrough to the frame — only when any
/// `--field` was given, so a fieldless frame's bytes stand.
fn attach_fields(parsed: &Put, request: &mut Value) {
    if parsed.fields.is_empty() {
        return;
    }
    let map: serde_json::Map<String, Value> = parsed
        .fields
        .iter()
        .map(|(k, v)| (k.clone(), json!(v)))
        .collect();
    request["fields"] = Value::Object(map);
}

/// The § A.2.1 cap wall, client half: `fields` rides only when the hello
/// advertised `splice.fields` — refusing here, before any engine write, is
/// the taught refusal; sending anyway would draw the strict wall's
/// `bad_request` for a field this daemon never negotiated.
fn fields_cap_wall(parsed: &Put, door: &crate::script::wire_host::SocketDoor) -> Result<(), Fail> {
    if !parsed.fields.is_empty() && !door.has_cap("splice.fields") {
        return Err(Fail::tool(
            "--field rides the middleware passthrough (wire-contract § A.2.1), but this \
             daemon's hello does not serve the `splice.fields` cap — nothing was sent and \
             nothing was written. Fixes: retry without --field, or restart the daemon so a \
             build that advertises `splice.fields` binds the socket."
                .to_owned(),
        ));
    }
    Ok(())
}

/// One `--field k=v` pair (§ A.2.1 passthrough): split at the FIRST `=`, both
/// halves verbatim — no key vocabulary exists to validate against.
fn parse_field(raw: &str) -> Result<(String, String), Fail> {
    raw.split_once('=')
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .ok_or_else(|| Fail::tool(format!("--field takes k=v, got `{raw}`")))
}

/// An engine refusal, on both faces — [`engine::json_refusal`], which owns the envelope for
/// every `--json` face rather than for this verb alone.
fn refusal(parsed: &Put, workspace: &std::path::Path, error: &ErrorBody) -> Fail {
    engine::json_refusal(parsed.format, workspace, error)
}

/// The put door's §1 consequence clause — what did NOT happen because the
/// refusal fired ([`crate::path_law`] holds the family message; this door
/// states only its own name and consequence).
const PUT_CONSEQUENCE: &str = "Nothing was sent and nothing was written.";

/// The scope admission, two lanes. A head-colon spelling is the agent-plane
/// `[root:]path` address (§4.1 colon law — the root reading wins, never a
/// literal node name) and routes to [`admit_rooted_scope`], which rewrites
/// `parsed.scope` to the rel half on the accepted leg. Every other spelling
/// keeps the §1 path-law wall, this face's half of the family admission
/// (`crate::path_law`; dogfood 88877785): a violating spelling refuses exit 2
/// before any dial. The engine refuses the same shape but message-less — a
/// bare `bad_path` echo that cannot name the flag, and the empty spelling
/// echoes nothing at all. The `--json` face keeps the `{workspace, error}`
/// frame, exactly as the links door's admission publishes it.
fn admit_scope(parsed: &mut Put, workspace: &std::path::Path) -> Result<(), Fail> {
    let Some(scope) = parsed.scope.clone() else {
        return Ok(());
    };
    if crate::rooted::is_rooted(&scope) {
        parsed.scope = Some(admit_rooted_scope(parsed.format, &scope, workspace)?);
        return Ok(());
    }
    if !crate::path_law::violates_path_law(&scope) {
        return Ok(());
    }
    let mut error = ErrorBody::new(wire::ErrorCode::BadPath);
    error.path = Some(WirePath(scope.clone()));
    error.message = Some(crate::path_law::scope_bad_path_message(workspace, &scope));
    engine::json_error_frame(parsed.format, workspace, &error);
    Err(Fail::tool(engine::render_wire_error(&error)))
}

/// The rooted lane for `--scope` (card put-scope-rejects-rooted-mint-echo):
/// the spelling a rooted §4.7 mint echoes, resolved through the one CLI seam
/// ([`crate::rooted::resolve`]) and accepted exactly when the named root
/// binds the workspace THIS PUT WRITES — the premise then names the same
/// node as the stripped spelling, so the rel half is what rides the wire and
/// the §5.4 `scope` field stays workspace-relative. Every other landing is
/// an address answer (exit 1, `{workspace, error}` under `--json`, the
/// seam's `bad_path` family), never premise coverage: before this lane a
/// rooted or unbound-root scope rode the wire as a literal node name and
/// surfaced as the §5.5 "no premise covers" refusal — a coverage answer for
/// an address fault. A `#` fragment refuses at the same wall (the resolve
/// door's path-grain posture): a premise binds a node, not a section, and
/// silently stripping the fragment would bind a premise the caller did not
/// spell.
fn admit_rooted_scope(
    format: Format,
    scope: &str,
    workspace: &std::path::Path,
) -> Result<String, Fail> {
    if scope.contains('#') {
        let mut error = ErrorBody::new(wire::ErrorCode::BadPath);
        error.path = Some(WirePath(scope.to_owned()));
        error.message = Some(format!(
            "--scope {scope} carries a `#` fragment, and a §5.4 premise binds at path \
             grain — a fragment addresses a section, not a node. Re-issue with the bare \
             `[root:]path` spelling of the node the §4.7 mint bound. {PUT_CONSEQUENCE}"
        ));
        return Err(engine::json_refusal(format, workspace, &error));
    }
    let (rel, rooted) = crate::rooted::resolve(scope, "put", PUT_CONSEQUENCE)
        .map_err(|error| engine::json_refusal(format, workspace, &error))?;
    if rooted.workspace.as_path() != workspace {
        let mut error = ErrorBody::new(wire::ErrorCode::BadPath);
        error.path = Some(WirePath(scope.to_owned()));
        error.message = Some(format!(
            "--scope {scope} names root `{name}`, which binds {bound} — but this put \
             writes {ws}, and a §5.4 premise covers only nodes of the workspace being \
             written. {PUT_CONSEQUENCE} Run the put from that root's workspace to pair \
             this scope, or mint a premise for this workspace (`mrd fingerprint <path>`, \
             §4.7).",
            name = rooted.name,
            bound = rooted.workspace.display(),
            ws = workspace.display(),
        ));
        return Err(engine::json_refusal(format, workspace, &error));
    }
    Ok(rel)
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
            parsed.display()
        );
        return;
    }
    let after = body
        .get("fingerprint_after")
        .and_then(Value::as_str)
        .unwrap_or("?");
    println!("committed {} ({edits} edit(s))", parsed.display());
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
    // § A.2.1 middleware intents (`armed.intents`): armed, never delivered —
    // the host realizes them and answers on its own surface.
    let mw = body
        .get("armed")
        .and_then(|armed| armed.get("intents"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten();
    for intent in mw {
        let rule = intent.get("rule_id").and_then(Value::as_str).unwrap_or("?");
        let kind = intent.get("kind").and_then(Value::as_str).unwrap_or("?");
        let to: Vec<&str> = intent
            .get("to")
            .and_then(Value::as_array)
            .map(|xs| xs.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        println!("  intent: {rule} {kind} → {} (host realizes)", to.join(","));
    }
}

/// The parsed `put` invocation.
struct Put {
    path: String,
    /// The rooted lane's typed spelling (`root:rel`) — the write target the
    /// human face echoes, so the caller sees what they wrote. `None` on the
    /// ambient lane. The wire never carries it: `path` is the rel half.
    display: Option<String>,
    actor: Option<String>,
    now: Option<String>,
    receipt: Option<ReceiptAddr>,
    if_fingerprint: Option<String>,
    /// `--scope`: the node the `--if-fingerprint` premise binds (§5.4). The
    /// pair law holds at parse: present only beside `if_fingerprint`.
    scope: Option<String>,
    /// `--scope-bytes`: the same premise node as base64url over the raw path
    /// bytes (§5.4). Pair law at parse; mutually exclusive with `scope`;
    /// rides the wire as one `guards[]` entry, never top-level.
    scope_bytes: Option<String>,
    /// `--field k=v` (repeatable): the § A.2.1 opaque passthrough, delivered
    /// to middleware verbatim as `ctx.fields`. No key is interpreted.
    fields: Vec<(String, String)>,
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

    /// The spelling the faces echo: the caller's own — rooted where the
    /// caller wrote rooted, the bare path otherwise (the read door's law).
    fn display(&self) -> &str {
        self.display.as_deref().unwrap_or(&self.path)
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
        let mut scope_bytes: Option<String> = None;
        let mut fields: Vec<(String, String)> = Vec::new();
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
                "--scope-bytes" => scope_bytes = Some(flag_value(&mut it, "--scope-bytes")?),
                "--field" => fields.push(parse_field(&flag_value(&mut it, "--field")?)?),
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
        premise_flag_walls(
            scope.as_deref(),
            scope_bytes.as_deref(),
            if_fingerprint.as_deref(),
        )?;
        Ok(Put {
            path,
            display: None,
            actor,
            now,
            receipt,
            if_fingerprint,
            scope,
            scope_bytes,
            fields,
            dry,
            validate,
            force,
            format: if json { Format::Json } else { Format::Human },
        })
    }
}

/// One flag's value pulled from argv — the extracted arm the `fn parse`
/// headroom note commands. No address split, so no `PINNED` row.
fn flag_value(it: &mut std::slice::Iter<'_, String>, flag: &str) -> Result<String, Fail> {
    it.next()
        .cloned()
        .ok_or_else(|| Fail::tool(format!("{flag} needs a value")))
}

/// The §5.4 premise-flag walls, this face's own (exit 2, before stdin and
/// before any dial): two spellings name one premise's node twice; a scope
/// names WHERE a premise binds and carries no token, so alone it is half a
/// premise; an empty `--scope-bytes` names nothing (the empty `--scope`
/// refuses at [`admit_scope`], which owns the §1 path-law voice). The engine
/// would refuse every one of these shapes; refusing here costs no dial.
fn premise_flag_walls(
    scope: Option<&str>,
    scope_bytes: Option<&str>,
    if_fingerprint: Option<&str>,
) -> Result<(), Fail> {
    if scope.is_some() && scope_bytes.is_some() {
        return Err(Fail::tool(
            "--scope and --scope-bytes are two spellings of one premise's node \
             (wire-contract §5.4: exactly one per premise) — pass the one that names \
             your path: --scope for a UTF-8 name, --scope-bytes for raw path bytes."
                .to_owned(),
        ));
    }
    if scope.is_some() && if_fingerprint.is_none() {
        return Err(Fail::tool(
            "--scope names the premise's node but carries no token — pair it with \
             --if-fingerprint holding that node's scoped token (minted by the §4.7 \
             `fingerprint {scope}` arm; wire-contract §5.4). A scope with no \
             fingerprint is half a premise."
                .to_owned(),
        ));
    }
    if scope_bytes.is_some() && if_fingerprint.is_none() {
        return Err(Fail::tool(
            "--scope-bytes names the premise's node but carries no token — pair it \
             with --if-fingerprint holding that node's scoped token (minted by the \
             §4.7 `fingerprint {scope_bytes}` arm; wire-contract §5.4). A scope with \
             no fingerprint is half a premise."
                .to_owned(),
        ));
    }
    if scope_bytes == Some("") {
        return Err(Fail::tool(
            "--scope-bytes is empty — it names no node, and an empty value is \
             usually an unquoted shell variable that expanded to nothing. It takes \
             the base64url spelling the §4.7 mint echoed (`fingerprint \
             {scope_bytes}`; wire-contract §5.4). Nothing was sent and nothing was \
             written."
                .to_owned(),
        ));
    }
    Ok(())
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
