//! The lock plane's rows for `mrd status` and `mrd check`
//! (`docs/wire-contract.md` § A.13): asked of the resident daemon's `pins`
//! door, or coloured in this process for a workspace no daemon serves. Both
//! lanes hand the verbs one row, [`wire::PinRow`], so everything downstream of
//! the rows is one code path.

use std::io::BufReader;
use std::path::Path;

use registry::Client;
use serde_json::{Value, json};
use wire::PinRow;

use crate::engine::{call, call_through_retries, hello_identity_skew, resident_lost};
use crate::{Fail, Format};

/// The daemon's § A.13 answer.
pub(crate) struct LockPlane {
    /// Every declared pin, coloured: holder path order, then lock order.
    pub(crate) rows: Vec<PinRow>,
    /// The declined class of the markdown the hash domain left out.
    pub(crate) excluded: Vec<String>,
}

/// The daemon lane: `hello` binds and warms `workspace`, then one `pins`.
///
/// `Ok(None)` = no daemon serves this workspace and the in-process lane
/// answers. Once a daemon has accepted the connection it is the answer (§ A.10,
/// the CLI ladder): every other outcome is the rows or an `Err`.
pub(crate) fn from_daemon(workspace: &Path, format: Format) -> Result<Option<LockPlane>, Fail> {
    let dialing = timing::phase("daemon.dial");
    let Ok(client) = Client::from_default() else {
        return Ok(None);
    };
    let Some(stream) = crate::engine::connect_or_spawn(&client)? else {
        return Ok(None);
    };
    let mut writer = stream.try_clone().map_err(|e| resident_lost(&e))?;
    let mut reader = BufReader::new(stream);
    let socket = client.socket_path();

    let hello = json!({
        "op": "hello",
        "proto": 1,
        "contract": "v3",
        "workspace": workspace.to_string_lossy(),
    });
    let greeted = call(&mut writer, &mut reader, socket, &hello).map_err(|e| resident_lost(&e))?;
    if greeted.get("ok").and_then(Value::as_bool) != Some(true) {
        return Ok(None);
    }
    hello_identity_skew(greeted.get("body"), socket).map_err(Fail::tool)?;

    let request = json!({ "op": "pins" });
    let response = call_through_retries(&mut writer, &mut reader, socket, workspace, &request)
        .map_err(|e| resident_lost(&e))?;
    dialing.stop();
    if response.get("ok").and_then(Value::as_bool) != Some(true) {
        let Some(error) = response
            .get("error")
            .and_then(|e| serde_json::from_value::<wire::ErrorBody>(e.clone()).ok())
        else {
            return Err(Fail::tool(format!(
                "the resident daemon refused `pins` with a frame that is not the §8 envelope: \
                 {response}"
            )));
        };
        crate::engine::json_error_frame(format, workspace, &error);
        return Err(Fail::tool(crate::engine::refusal_text(&error)));
    }
    let field = |key: &str| response.get("body").and_then(|body| body.get(key)).cloned();
    let rows = field("pins").and_then(|pins| serde_json::from_value::<Vec<PinRow>>(pins).ok());
    let Some(rows) = rows else {
        return Err(Fail::tool(format!(
            "the resident daemon answered `pins` with a body this build cannot read as the \
             § A.13 rows: {response}"
        )));
    };
    let excluded = field("excluded")
        .and_then(|list| serde_json::from_value(list).ok())
        .unwrap_or_default();
    Ok(Some(LockPlane { rows, excluded }))
}

/// The in-process lane's rows: the daemon's own projection of the same
/// colours.
pub(crate) fn rows(colors: Vec<view::walk::PinColor>) -> Vec<PinRow> {
    colors.into_iter().map(registry::pin_row).collect()
}

/// A row's colour as the human label, in `view`'s one spelling.
pub(crate) fn label(row: &PinRow) -> String {
    view::walk::label_parts(&row.color, row.reason.as_deref(), row.detail.as_deref())
}

/// The ref a row declares, as the lock reader spells it; empty on a
/// lock-refused row.
pub(crate) fn declared_ref(row: &PinRow) -> String {
    match (&row.object, &row.selector) {
        (None, _) => String::new(),
        (Some(object), None) => format!("{object}.md"),
        (Some(object), Some(selector)) => format!("{object}.md#{selector}"),
    }
}

/// Whether the hash domain carries the row's holder. Rows of holders it
/// excludes are part of the pin population (`docs/status.md`, the
/// pin-population clause) and of no anchoring question.
pub(crate) fn holder_in_domain(domain: &fs::domain::Domain, row: &PinRow) -> bool {
    domain.contains(Path::new(&row.holder))
}
