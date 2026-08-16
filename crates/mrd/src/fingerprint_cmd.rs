//! `mrd fingerprint` — the standalone §4.7 mint door.
//!
//! ```text
//! mrd fingerprint [PATH | --scope-bytes B64] [--json]
//! ```
//!
//! Bare, it mints the §5.1 world token — the v2-identical base arm, no cap
//! needed. `PATH` mints the NAMED node's scoped token (workspace root, folder,
//! or file leaf); `--scope-bytes B64` (base64url over the raw path bytes)
//! mints a node whose name the UTF-8 `Path` noun cannot carry (wire-contract
//! §5.4). Exactly one spelling — a mint names ONE node (§4.7); both refuse at
//! parse before any dial, mirroring the engine's own mint-pair wall.
//!
//! The riding mint (`mrd read --json` under `scoped-guards`) covers only the
//! read path — a file leaf the read served. Folders and raw-byte names have
//! no servable read, so this door is their one CLI mint home.
//!
//! Walls, all exit 2 before any engine contact: the mint-pair law and the
//! empty `--scope-bytes` at parse; the §1 path law on `PATH` after the
//! workspace resolves (the engine would refuse the same shape message-less);
//! the cap wall after connect — a scoped arm rides only when the hello
//! advertised `scoped-guards` (§3.2 discovery honesty), while the world arm
//! rides regardless. The daemon is the one mint authority: a down daemon is a
//! taught refusal, never an in-process mint.
//!
//! Exit triad: 0 minted / 1 engine refusal (`scope_unresolved`, an
//! undecodable base64url, …— the engine's verbatim frame) / 2 bad
//! invocation. Under `--json` both legs answer on stdout: the served mint as
//! `{workspace, mint}` (the §4.7 body verbatim — `{fingerprint, seq,
//! scope|scope_bytes}`, the request's spelling echoed beside the token), a
//! refusal as the `{workspace, error}` envelope.

use serde_json::{Value, json};
use wire::{ErrorBody, Path as WirePath};

use crate::script::wire_host::{Door as _, Frame, SocketDoor};
use crate::{Fail, Format, current_dir, engine};

/// The one teaching this door prints when the mint authority cannot be
/// reached — the write doors' posture (`write_ipc::DAEMON_DOWN`), spoken in
/// the mint's own voice.
const MINT_DAEMON_DOWN: &str = "\
mrd fingerprint mints through the running daemon (§4.7); there is no \
in-process mint. The daemon must come up (`mrd daemon`, or the next call \
auto-spawns it).";

/// Run `mrd fingerprint [PATH | --scope-bytes B64] [--json]`. Errors
/// [`Fail`] — exit 2 on a bad invocation or an unreachable daemon (the CLI's
/// own refusals, before any mint); exit 1 on every engine refusal, message
/// verbatim.
pub(crate) fn dispatch(args: &[String]) -> Result<(), Fail> {
    let parsed = Mint::parse(args)?;
    let cwd = current_dir()?;
    let resolved = crate::resolve::resolve_runtime(&cwd).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace for {}: {e}",
            cwd.display()
        ))
    })?;
    admit_path(&parsed, &resolved.workspace)?;

    let client = registry::Client::from_default().map_err(|e| {
        Fail::tool(format!(
            "{MINT_DAEMON_DOWN} (cannot resolve the daemon socket: {e})"
        ))
    })?;
    engine::ensure_daemon(&client).map_err(|e| {
        Fail::tool(format!(
            "{MINT_DAEMON_DOWN} ({e}). {}",
            engine::degrade_reason().unwrap_or_else(|| {
                "Start the daemon, or fix the socket path (XDG_CACHE_HOME / sun_path).".to_owned()
            })
        ))
    })?;
    let mut door = SocketDoor::connect(client.socket_path(), &resolved.workspace)
        .map_err(|e| Fail::tool(format!("{MINT_DAEMON_DOWN} (cannot dial the daemon: {e})")))?;

    // The cap wall, client half (§3.2): a scoped mint rides only when the
    // connect-time hello advertised the family. The world arm is the
    // v2-identical base op and rides regardless.
    if parsed.scoped() && !door.has_cap(engine::SCOPED_GUARDS_CAP) {
        return Err(Fail::tool(format!(
            "a scoped mint (PATH or --scope-bytes; wire-contract §4.7) needs the `{}` cap, \
             and this daemon's hello does not serve it — nothing was sent and no token \
             was minted.\n\
             Why: the cap is family-whole discovery honesty (§3.2): a daemon either serves \
             the whole scoped-premise family or refuses every guard-family field at its \
             strict wall; the client refuses first, with this teaching, instead of drawing \
             that `bad_request`.\n\
             Fixes — run whichever fits your case:\n\
               - `mrd fingerprint` bare mints the §5.1 world token this daemon does serve.\n\
               - when the daemon should serve the family: restart it so a build that \
             advertises `{}` binds the socket, then mint again.",
            engine::SCOPED_GUARDS_CAP,
            engine::SCOPED_GUARDS_CAP,
        )));
    }

    let mut request = json!({ "op": "fingerprint" });
    if let Some(path) = &parsed.path {
        request["scope"] = json!(path);
    }
    if let Some(b64) = &parsed.scope_bytes {
        request["scope_bytes"] = json!(b64);
    }
    let body = mint_call(&mut door, &request)
        .map_err(|e| engine::json_refusal(parsed.format, &resolved.workspace, &e))?;

    match parsed.format {
        Format::Json => {
            let value = json!({
                "workspace": resolved.workspace.display().to_string(),
                "mint": body,
            });
            println!("{}", serde_json::to_string_pretty(&value).expect("json"));
        }
        Format::Human => print_human(&body),
    }
    Ok(())
}

/// One mint exchange on the open door. Success is the §4.7 body; a refusal is
/// the engine's [`ErrorBody`] so the `--json` face keeps one envelope. No
/// v2/v3 unprojection here: a mint refuses `scope_unresolved` / `bad_request`
/// / `bad_path`, none of which the projection renames.
fn mint_call(door: &mut SocketDoor, request: &Value) -> Result<Value, Box<ErrorBody>> {
    let io_refusal = |message: String| {
        let mut err = ErrorBody::new(wire::ErrorCode::IoError);
        err.message = Some(message);
        Box::new(err)
    };
    let line = door
        .call(request)
        .map_err(|e| io_refusal(format!("the daemon did not answer the mint: {e}")))?;
    let frame = Frame::parse(&line)
        .map_err(|e| io_refusal(format!("the daemon's mint answer would not parse: {e}")))?;
    if frame.ok {
        let body = frame
            .body_value("fingerprint")
            .map_err(|e| io_refusal(e.to_string()))?;
        // A mint names its token. A hello/identity body is ok:true too —
        // treating one as a mint was the silent-success class the write door
        // already defends against.
        if body.get("fingerprint").is_none() {
            return Err(io_refusal(format!(
                "the daemon answered ok but the body is not a mint (request={request} body={body})"
            )));
        }
        return Ok(body);
    }
    match frame.error {
        Some(raw) => match serde_json::from_str::<ErrorBody>(raw.get()) {
            Ok(error) => Err(Box::new(error)),
            Err(e) => Err(io_refusal(format!(
                "the daemon refused the mint but the error frame would not parse: {e}"
            ))),
        },
        None => Err(io_refusal(format!(
            "the daemon refused the mint with no error body: {}",
            line.trim()
        ))),
    }
}

/// The human face: the token, then the spelling the mint bound (the §4.7
/// desync guard, printed so a caller can never desync what it minted from
/// where), then the seq.
fn print_human(body: &Value) {
    let token = body
        .get("fingerprint")
        .and_then(Value::as_str)
        .unwrap_or("?");
    println!("fingerprint: {token}");
    if let Some(scope) = body.get("scope").and_then(Value::as_str) {
        println!("  scope: {scope}");
    }
    if let Some(b64) = body.get("scope_bytes").and_then(Value::as_str) {
        println!("  scope_bytes: {b64}");
    }
    if let Some(seq) = body.get("seq").and_then(Value::as_u64) {
        println!("  seq: {seq}");
    }
}

/// The §1 path-law wall for the PATH arm, this door's fitting of the family
/// admission (`crate::path_law`): a violating spelling refuses exit 2 before
/// any dial, and the `--json` face keeps the `{workspace, error}` frame —
/// exactly as `put`'s scope wall publishes it. `--scope-bytes` is exempt by
/// design: raw bytes are the names the §1 noun cannot spell.
fn admit_path(parsed: &Mint, workspace: &std::path::Path) -> Result<(), Fail> {
    let Some(path) = &parsed.path else {
        return Ok(());
    };
    if !crate::path_law::violates_path_law(path) {
        return Ok(());
    }
    let message = if path.is_empty() {
        "the mint PATH is empty — it names no node, and an empty value is usually \
         an unquoted shell variable that expanded to nothing. A mint PATH is the \
         workspace-relative path of the node to mint (§1 path law: no absolute \
         path, no `.`/`..`/empty segment; wire-contract §4.7). Nothing was sent \
         and no token was minted."
            .to_owned()
    } else {
        crate::path_law::bad_path_message(
            workspace,
            path,
            "fingerprint mint",
            "Nothing was sent and no token was minted.",
        )
    };
    let mut error = ErrorBody::new(wire::ErrorCode::BadPath);
    error.path = Some(WirePath(path.clone()));
    error.message = Some(message);
    engine::json_error_frame(parsed.format, workspace, &error);
    Err(Fail::tool(engine::render_wire_error(&error)))
}

/// The parsed `fingerprint` invocation.
struct Mint {
    /// The positional PATH — the UTF-8 scoped arm. Absent with
    /// [`Self::scope_bytes`] absent = the world mint.
    path: Option<String>,
    /// `--scope-bytes`: the raw-byte scoped arm, base64url over the raw path
    /// bytes (§5.4). Mutually exclusive with [`Self::path`] at parse.
    scope_bytes: Option<String>,
    format: Format,
}

impl Mint {
    /// Does this invocation name a node — either spelling (§4.7 scoped arm)?
    fn scoped(&self) -> bool {
        self.path.is_some() || self.scope_bytes.is_some()
    }

    /// `fingerprint`'s argv parse. The mint-pair law holds here, before any
    /// dial, with the same posture as the engine's §4.7 wall: a mint names
    /// ONE node, so two spellings refuse.
    fn parse(args: &[String]) -> Result<Self, Fail> {
        let mut path: Option<String> = None;
        let mut scope_bytes: Option<String> = None;
        let mut json = false;
        let mut it = args.iter();
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "--json" => json = true,
                "--scope-bytes" => {
                    let value = it
                        .next()
                        .ok_or_else(|| Fail::tool("--scope-bytes needs a value".to_owned()))?;
                    scope_bytes = Some(value.clone());
                }
                flag if flag.starts_with('-') => {
                    return Err(Fail::tool(format!("unknown flag: {flag}")));
                }
                value if path.is_none() => path = Some(value.to_owned()),
                value => return Err(Fail::tool(format!("unexpected argument: {value}"))),
            }
        }
        if path.is_some() && scope_bytes.is_some() {
            return Err(Fail::tool(
                "a mint names ONE node, and PATH and --scope-bytes are two spellings of \
                 it (wire-contract §4.7) — pass the one that names your path: PATH for \
                 a UTF-8 name, --scope-bytes for raw path bytes."
                    .to_owned(),
            ));
        }
        if scope_bytes.as_deref() == Some("") {
            return Err(Fail::tool(
                "--scope-bytes is empty — it names no node, and an empty value is \
                 usually an unquoted shell variable that expanded to nothing. It takes \
                 base64url over the raw path bytes of the node to mint (wire-contract \
                 §5.4/§4.7). Nothing was sent and no token was minted."
                    .to_owned(),
            ));
        }
        Ok(Mint {
            path,
            scope_bytes,
            format: if json { Format::Json } else { Format::Human },
        })
    }
}
