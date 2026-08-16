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
                "Start the daemon, or fix the socket path (XDG_CACHE_HOME / sun_path).".to_owned()
            })
        ))
    })?;
    SocketDoor::connect(client.socket_path(), workspace)
        .map_err(|e| Fail::tool(format!("{DAEMON_DOWN} (cannot dial the daemon: {e})")))
}

/// One write op on an open door. Success is the projected body; a daemon
/// refusal is the engine's [`ErrorBody`] so the verb's `--json` face stays
/// one envelope.
pub(crate) fn call(door: &mut SocketDoor, request: &Value) -> Result<Value, Box<ErrorBody>> {
    use crate::script::wire_host::Door as _;

    let line = door.call(request).map_err(|e| {
        let mut err = ErrorBody::new(wire::ErrorCode::IoError);
        err.message = Some(format!("the daemon did not answer the write: {e}"));
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
        return frame.body_value("write").map_err(|e| {
            let mut err = ErrorBody::new(wire::ErrorCode::IoError);
            err.message = Some(e.to_string());
            Box::new(err)
        });
    }
    match frame.error {
        Some(raw) => serde_json::from_str(raw.get()).map_err(|e| {
            let mut err = ErrorBody::new(wire::ErrorCode::IoError);
            err.message = Some(format!(
                "the daemon refused the write but the error frame would not parse: {e}"
            ));
            Box::new(err)
        }),
        None => {
            let mut err = ErrorBody::new(wire::ErrorCode::IoError);
            err.message = Some(format!(
                "the daemon refused the write with no error body: {}",
                line.trim()
            ));
            Err(Box::new(err))
        }
    }
}

/// Lift a v2-shaped splice/remove body into the v3 vocabulary the CLI speaks.
/// Idempotent on a body the daemon already projected.
pub(crate) fn project_body(body: Value) -> Value {
    let mut frame = serde_json::json!({ "body": body });
    wire_serve::rev::project_response(&mut frame);
    frame
        .as_object_mut()
        .and_then(|obj| obj.remove("body"))
        .unwrap_or(Value::Null)
}
