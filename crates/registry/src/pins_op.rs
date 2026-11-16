//! The `pins` op (`docs/wire-contract.md` § A.13): every `meridian-lock` pin
//! the workspace declares, coloured — what `mrd status`'s lock axis and
//! `mrd check`'s pin plane are computed from.
//!
//! The colour COMPUTER is the leased arm of the one the CLI verbs run
//! ([`view::walk::lock_pin_colors_leased`]), over the warm engine's generation
//! with the mounted roots' resident engines beside it, as `walk`
//! ([`crate::walk_op`]). Read-only: writes nothing, mints no receipt; computed
//! per query, never stored.

use std::path::Path;

use view::walk::{self, LockFacts};
use wire::{ErrorBody, ErrorCode, PinRow, ResponseBody, Root};

use crate::{Registry, WorkspaceEngine};

/// Serve one `pins` call from the pinned warm engine `engine`.
///
/// # Errors
/// `io_error` when the workspace's hash domain cannot be read or walked —
/// never the default domain, and never a pin population narrowed to the
/// hashed corpus without saying so — or when a member the generation holds
/// cannot be served, naming that member.
pub(crate) fn serve(
    registry: &Registry,
    engine: &WorkspaceEngine,
    ws: &Path,
) -> Result<ResponseBody, Box<ErrorBody>> {
    let root = fs::WorkspaceRoot(ws.to_path_buf());
    let io_error = |what: &str, e: &std::io::Error| {
        let mut err = ErrorBody::new(ErrorCode::IoError);
        err.cause = Some(format!("{what}: {e}"));
        Box::new(err)
    };
    let domain =
        fs::domain::Domain::load(&root).map_err(|e| io_error("cannot read the hash domain", &e))?;
    let outside =
        fs::excluded_markdown(&root).map_err(|e| io_error("cannot list the workspace", &e))?;

    // §12.1 pin-population clause: a holder the hash domain excludes is read
    // from disk for its lock rows. One that cannot be read is skipped, as the
    // in-process lane skips it.
    let files: fs::DomainFiles = outside
        .iter()
        .filter_map(|md| Some((md.rel.clone(), std::fs::read(ws.join(&md.rel)).ok()?)))
        .collect();
    let (_index, excluded_holders, _unserved) = fs::build_corpus(files);

    let mounts = wire_serve::mount_corpus::load_mounts_for_from(&engine.lock_roots(), &|root| {
        registry.mounted_corpus(root)
    });
    let corpus = mounts
        .rooted_catalog(engine)
        .with_hash_domain(&domain)
        .with_ambient_disk(&root);
    let colors = walk::lock_pin_colors_leased(&corpus, engine, mounts.set(), &excluded_holders)
        .map_err(wire_serve::read::member_fault)?;

    let pins = colors.into_iter().map(pin_row).collect();
    let excluded = outside
        .into_iter()
        .filter(|md| md.declined)
        .map(|md| md.rel)
        .collect();
    Ok(ResponseBody::Pins {
        root: Root(engine.at_fingerprint.0.clone()),
        pins,
        excluded,
    })
}

/// One coloured pin as its § A.13 wire row — the one projection the door and
/// the CLI's in-process lane share.
#[must_use]
pub fn pin_row(pin: walk::PinColor) -> PinRow {
    let some = |s: String| (!s.is_empty()).then_some(s);
    PinRow {
        holder: pin.src_path,
        object: some(pin.object),
        selector: some(pin.selector),
        hash: pin.blob_sha,
        color: walk::color_tone(&pin.color).to_string(),
        reason: walk::color_reason(&pin.color).map(str::to_owned),
        detail: walk::color_detail(&pin.color),
    }
}
