//! The CLI write transport: every routine write verb dials the running daemon
//! over authenticated IPC (hello + the socket-law identity check). There is no
//! in-process publication fallback — a down daemon is a taught refusal, never a
//! local `splice` / `remove`.
//!
//! Law: merged-plan §4.7 / §4.11 CLI row (pre-merge ruling 4, "No Daemon Route,
//! B"); authority contract §15. The daemon write door is already the authority;
//! this module is the client half.

use std::path::Path;

use registry::Client;
use serde_json::Value;
use wire::ErrorBody;

use crate::script::wire_host::{Frame, SocketDoor};
use crate::{Fail, engine};

/// The one teaching every write verb prints when the authority cannot be
/// reached. Face and docs share this spelling so a test can match it.
pub(crate) const DAEMON_DOWN: &str = "\
mrd write verbs talk to the running daemon over authenticated IPC; there is no \
direct-publication fallback. The daemon must come up (`mrd daemon`, or the next \
call auto-spawns it). Scripts that used to write with no daemon now need the \
daemon up.";

/// Dial the workspace authority for a write. Auto-spawns the resident daemon
/// the same way the read path does, then refuses — never degrades — if it
/// does not answer.
pub(crate) fn connect(workspace: &Path) -> Result<SocketDoor, Fail> {
    let client = Client::from_default().map_err(|e| {
        Fail::tool(format!(
            "{DAEMON_DOWN} (cannot resolve the daemon socket: {e})"
        ))
    })?;
    engine::ensure_daemon(&client).map_err(|e| {
        Fail::tool(format!(
            "{DAEMON_DOWN} ({e}). {}",
            engine::degrade_reason().unwrap_or_else(|| {
                "Start the daemon, or fix the socket path (XDG_RUNTIME_DIR / HOME / sun_path)."
                    .to_owned()
            })
        ))
    })?;
    SocketDoor::connect(client.socket_path(), workspace)
        .map_err(|e| Fail::tool(format!("{DAEMON_DOWN} (cannot dial the daemon: {e})")))
}

/// Printed to stderr once when the daemon has held a write past the socket's
/// read timeout (the script host's 7 s wall clock, which this door shares).
/// The wait goes on: a write on the wire is a write the daemon is committing,
/// and the only honest result is its answer (card
/// `cli-face-put-false-negative`: before this, the tick returned `os error
/// 35` and exit 1 while the bytes landed).
pub(crate) const STILL_WAITING: &str = "\
mrd: the daemon has not answered this write yet — it is in flight (an armed \
middleware may be running) and may already be committed; still waiting for \
the daemon's answer. Do not interrupt and re-send: a re-send of landed bytes \
is a second write. If you must stop, read the file back before any retry.";

/// One write op on an open door. Success is the projected body; a daemon
/// refusal is the engine's [`ErrorBody`] so the verb's `--json` face stays
/// one envelope.
///
/// The answer is waited for — never abandoned at the wall clock. A transport
/// loss after the frame went out (the daemon closed the connection, the
/// stream tore) is reported as what it is: the outcome is UNKNOWN, the write
/// may have committed, read before any re-send. It is never rendered as a
/// failed write — that was the false negative.
pub(crate) fn call(door: &mut SocketDoor, request: &Value) -> Result<Value, Box<ErrorBody>> {
    let line = door
        .call_until_answered(request, STILL_WAITING)
        .map_err(|e| {
            let mut err = ErrorBody::new(wire::ErrorCode::IoError);
            let target = request
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or("the target");
            err.message = Some(format!(
                "the answer to the write on {target} was lost ({e}) — the frame had reached the \
             daemon, so the write may have committed before the loss: whether the file \
             carries it is UNKNOWN. Read the file back and check for your edit BEFORE any \
             re-send; a re-send of landed bytes is a second write, and a lost answer is not \
             a failed write."
            ));
            err.cause = Some(e.to_string());
            Box::new(err)
        })?;
    let frame = Frame::parse(&line).map_err(|e| {
        let mut err = ErrorBody::new(wire::ErrorCode::IoError);
        err.message = Some(format!("the daemon's write answer would not parse: {e}"));
        err.cause = Some(e.to_string());
        Box::new(err)
    })?;
    if frame.ok {
        let body = frame.body_value("write").map_err(|e| {
            let mut err = ErrorBody::new(wire::ErrorCode::IoError);
            err.message = Some(e.to_string());
            Box::new(err)
        })?;
        // A write reply names what it armed or removed. Hello/identity
        // bodies are ok:true too — treating those as a commit was the
        // silent-success class (pin printed "?" and exited 0).
        let armed_empty = body
            .pointer("/armed/edits")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty);
        if (body.get("armed").is_none()
            && body.get("file_rev_before").is_none()
            && body.get("file_rev_after").is_none())
            || (armed_empty && request.get("pin").is_none())
        {
            let mut err = ErrorBody::new(wire::ErrorCode::IoError);
            err.message = Some(format!(
                "the daemon answered ok but the body is not a write (request={request} body={body})"
            ));
            return Err(Box::new(err));
        }
        return Ok(body);
    }
    if let Some(raw) = frame.error {
        // The daemon's v3 session projects error codes (root_mismatch →
        // fingerprint_mismatch). ErrorBody is the v2 enum; unproject before
        // decode so a world-guard refusal is not an io_error parse miss.
        let mut raw_val: Value = serde_json::from_str(raw.get()).map_err(|e| {
            let mut err = ErrorBody::new(wire::ErrorCode::IoError);
            err.message = Some(format!(
                "the daemon refused the write but the error frame would not parse: {e}"
            ));
            Box::new(err)
        })?;
        if let Some(obj) = raw_val.as_object_mut()
            && let Some(Value::String(code)) = obj.get("code")
        {
            let v2 = match code.as_str() {
                "fingerprint_mismatch" => Some("root_mismatch"),
                "fingerprint_unknown" => Some("root_unknown"),
                _ => None,
            };
            if let Some(v2) = v2 {
                obj.insert("code".into(), Value::String(v2.into()));
            }
        }
        let error: ErrorBody = serde_json::from_value(raw_val).map_err(|e| {
            let mut err = ErrorBody::new(wire::ErrorCode::IoError);
            err.message = Some(format!(
                "the daemon refused the write but the error frame would not parse: {e}"
            ));
            Box::new(err)
        })?;
        Err(Box::new(error))
    } else {
        let mut err = ErrorBody::new(wire::ErrorCode::IoError);
        err.message = Some(format!(
            "the daemon refused the write with no error body: {}",
            line.trim()
        ));
        Err(Box::new(err))
    }
}

/// Lift a v2-shaped splice/remove body into the v3 vocabulary the CLI speaks.
/// Idempotent on a body the daemon already projected.
pub(crate) fn project_body(body: &Value) -> Value {
    let mut frame = serde_json::json!({ "body": body });
    wire_serve::rev::project_response(&mut frame);
    frame
        .as_object_mut()
        .and_then(|obj| obj.remove("body"))
        .unwrap_or(Value::Null)
}
