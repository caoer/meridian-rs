//! The `mounts` op (`docs/wire-contract.md` § A.5): mount-table discovery —
//! the live root registry, machine-scoped, under per-call config-hash
//! freshness.
//!
//! A projection off [`config::mount::MountTable`] — bind semantics, state
//! words, and refusal text all live there; this module owns only the
//! freshness gate and the wire shape.
//!
//! # The config-hash rebind law (§ A.5)
//! Every call fingerprints the binding file (blake3 over the file bytes)
//! before answering: an unchanged hash serves the derived table; a changed
//! hash re-derives first. No mtime, no TTL, no hello-time snapshot. A table
//! that fails to re-derive after a change refuses `mount_table_invalid` — it
//! never serves the previous table as if current. A table that fails to
//! re-derive while unchanged does not exist by construction: the hash gate
//! re-derives only on change.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use wire::{ErrorBody, ErrorCode, MountRow, NodeRev, ResponseBody};

/// One cached derivation: which binding file, at which bytes, said what.
#[derive(Debug, Clone)]
struct Derived {
    /// The resolved config path the table came from. Part of the gate key:
    /// `MERIDIAN_CONFIG` may re-point the chain between calls, and a
    /// re-pointed path is a changed world even when both files hash alike in
    /// some other slot.
    path: PathBuf,
    /// `blake3(file bytes)[:16]` at derivation; `None` = state A (absent).
    config_rev: Option<String>,
    /// The projected rows, in document order.
    rows: Vec<MountRow>,
}

/// The machine-scoped cache — one per daemon ([`crate::Registry`] holds it).
/// A restart starts cold: one extra derivation, never a wrong answer.
#[derive(Debug, Default)]
pub(crate) struct MountsCache {
    state: Mutex<Option<Derived>>,
}

/// Serve one `mounts` call (§ A.5).
///
/// # Errors
/// `mount_table_invalid{path, message}` (env class) when the resolved binding
/// file cannot be read, parsed, or bound — and there is no unchanged prior
/// derivation to serve.
pub(crate) fn serve(cache: &MountsCache) -> Result<ResponseBody, Box<ErrorBody>> {
    let env = config::Env::from_process();
    let path = config::resolve_path(&env).map_err(|e| invalid(&e.path, &e.to_string()))?;
    // The gate: fingerprint the binding file's bytes before answering.
    let live_rev = match std::fs::read(&path) {
        Ok(bytes) => Some(rev16(&bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            return Err(invalid(
                &path,
                &format!("the binding file cannot be read: {e}"),
            ));
        }
    };
    {
        let state = cache.state.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(d) = state.as_ref()
            && d.path == path
            && d.config_rev == live_rev
        {
            return Ok(body(d));
        }
    }
    // Changed hash (or first call): re-derive. A failure refuses loud and
    // caches nothing — the fix is editing the binding file, not re-dialing.
    let resolution = config::resolve(&env).map_err(|e| invalid(&e.path, &e.to_string()))?;
    let table = resolution
        .bind(&env)
        .map_err(|e| invalid(&e.path, &e.to_string()))?;
    let derived = Derived {
        path,
        // The resolution's own rev — the bytes the table actually derived
        // from; the gate read above is a separate read and may trail a racing
        // writer, and serving its hash would pin the table to bytes it was
        // not derived from.
        config_rev: resolution.file_rev().map(str::to_owned),
        rows: project(&table),
    };
    let out = body(&derived);
    *cache.state.lock().unwrap_or_else(PoisonError::into_inner) = Some(derived);
    Ok(out)
}

/// The § A.5 row projection — the engine's own words verbatim, one spelling
/// across the human line, `--json`, and this wire.
fn project(table: &config::mount::MountTable) -> Vec<MountRow> {
    table
        .mounts()
        .iter()
        .map(|m| MountRow {
            name: m.name().to_owned(),
            state: m.state().word().to_owned(),
            workspace: m.canonical_path().map(|p| p.to_string_lossy().into_owned()),
            primary: m.primary(),
            alias: m.alias().map(ToOwned::to_owned),
        })
        .collect()
}

fn body(d: &Derived) -> ResponseBody {
    ResponseBody::Mounts {
        config_rev: d.config_rev.clone().map(NodeRev),
        mounts: d.rows.clone(),
    }
}

/// `blake3(bytes)[:16]` — the §1 `file_rev` law over the binding file's bytes.
fn rev16(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().as_str()[..16].to_string()
}

/// The § A.5 refusal: `mount_table_invalid{path, message}`, env class. The
/// message is the config plane's own rendered refusal, so the wire caller
/// reads the same words the CLI operator does (Law A-3c rides in it: scope,
/// offending member, and the `Fix:` clause).
fn invalid(path: &Path, message: &str) -> Box<ErrorBody> {
    let mut e = ErrorBody::new(ErrorCode::MountTableInvalid);
    e.path = Some(wire::Path(path.display().to_string()));
    e.message = Some(message.to_string());
    Box::new(e)
}
