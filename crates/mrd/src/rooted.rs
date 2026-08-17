//! The CLI half of the rooted-address lane: `[root:]path` resolved to the
//! named root's bound workspace, ONCE, here — the seam every CLI door that
//! grows a `root:` lane resolves through, so two doors cannot hold two
//! opinions of one ref (the MCP face's one-seam law, `resolveRefIn`).
//!
//! The grammar is the §4.1 colon law (`docs/address-grammar.md`), and the
//! resolution order is the engine's own pin-cross-root door
//! ([`wire_serve`] `resolve_pin_target`): parse → confinement → table. The
//! refusal family is that door's too — `bad_path`, each refusal teaching its
//! own remedy, the unbound-root refusal enumerating what DOES bind — so a
//! rooted spelling refused at the CLI reads like the same spelling refused
//! at the wire. Riding the engine's §8 envelope keeps the `--json` face on
//! its `{workspace, error}` frame and the exit on the refusal leg (exit 1):
//! an address-plane refusal is an ANSWER about this machine's topology,
//! never a malformed invocation.
//!
//! The root reading wins unconditionally (§4.1): a head colon is never
//! reinterpreted as a literal path, so a typo'd root refuses instead of
//! degrading into an ambient lookup — the cross-root misresolve shape the
//! grammar exists to prevent.

use std::path::PathBuf;

use wire::{ErrorBody, ErrorCode};

/// A rooted ref, resolved: the named root and the workspace its mount binds.
pub(crate) struct RootedRef {
    /// The canonical root name the spelling carried.
    pub(crate) name: addr::MountName,
    /// The root's canonical bound path — the workspace the read binds to.
    pub(crate) workspace: PathBuf,
    /// The mount's declared-primary designation, carried from the one table
    /// read so a door that renders the root row (resolve) never opens the
    /// table a second time for it.
    pub(crate) primary: bool,
}

/// Does this spelling enter the rooted lane at all? — the same lexical gate
/// the link plane and the resolver's C-3 guard share.
pub(crate) fn is_rooted(spelling: &str) -> bool {
    addr::head_carries_root_separator(spelling)
}

/// Resolve a rooted spelling (the part before any `#`) to its root and bound
/// workspace. `door` and `consequence` are the caller's own name and §1
/// consequence clause, composed into every refusal exactly as
/// [`crate::path_law`] composes the family teachings.
///
/// # Errors
/// `bad_path` (§8) — a malformed head, an unconfined rel half, an unreadable
/// mount table, or a root this machine does not bind. Parse and resolution
/// refuse alike: the §4.1 law has no fallback to a literal reading.
pub(crate) fn resolve(
    spelling: &str,
    door: &str,
    consequence: &str,
) -> Result<(String, RootedRef), Box<ErrorBody>> {
    let refuse = |message: String| -> Box<ErrorBody> {
        let mut e = ErrorBody::new(ErrorCode::BadPath);
        e.path = Some(wire::Path(spelling.to_owned()));
        e.message = Some(message);
        Box::new(e)
    };
    // Parse: the §4.1 colon law, through the one constructor. The fragment
    // was already split off by the caller, so the selector arm stays empty.
    let parsed = addr::Addr::parse(spelling).map_err(|e| refuse(format!("{e} {consequence}")))?;
    let Some(name) = parsed.root().cloned() else {
        // Unreachable behind [`is_rooted`], but the seam refuses rather than
        // trusting its caller: a rooted lane serving an ambient spelling
        // would be the misresolve defect inverted.
        return Err(refuse(format!(
            "{spelling} carries no root — the rooted lane resolves `root:path` spellings only. \
             {consequence}"
        )));
    };
    let rel = parsed.path().to_owned();
    if !addr::confined(&rel) {
        return Err(refuse(format!(
            "{rel} is not a root-relative path — the rel half of a rooted ref obeys the same \
             §1 path law as any {door} path (no absolute path, no `.`/`..`/empty segment, \
             no second `root:` prefix). {consequence}"
        )));
    }
    // The table, read fresh per call (the currency law the engine's pin door
    // and the MCP face both hold): a stale table would serve yesterday's
    // topology.
    let resolution = config::resolve(&config::Env::from_process()).map_err(|e| {
        refuse(format!(
            "{spelling} names root `{name}`, but this machine's mount table cannot be \
             resolved ({e}), so the name binds to no workspace. {consequence}"
        ))
    })?;
    let table = resolution.bind().map_err(|e| {
        refuse(format!(
            "{spelling} names root `{name}`, but this machine's mount table refuses to bind \
             ({e}), so the name binds to no workspace. {consequence}"
        ))
    })?;
    let Some(mount) = table.by_name(name.as_str()) else {
        let names: Vec<&str> = table
            .mounts()
            .iter()
            .filter(|m| !m.state().refuses())
            .map(config::mount::Mount::name)
            .collect();
        return Err(refuse(format!(
            "{spelling} names root `{name}`, which this machine does not bind (bound roots: \
             {}). {consequence} Fix: declare the mount in ~/MERIDIAN.md (name / path); see \
             [[address-grammar]].",
            if names.is_empty() {
                "none".to_owned()
            } else {
                names.join(", ")
            }
        )));
    };
    if mount.state().refuses() {
        return Err(refuse(format!(
            "{spelling} names root `{name}`, which is declared but does not bind here: \
             {} — {} {consequence}",
            mount.state().word(),
            mount.state().detail()
        )));
    }
    let Some(workspace) = mount.canonical_path() else {
        // A Bound state always carries its canonical path; refuse rather than
        // panic if that invariant ever moves.
        return Err(refuse(format!(
            "{spelling} names root `{name}`, whose mount carries no canonical path. \
             {consequence}"
        )));
    };
    Ok((
        rel,
        RootedRef {
            name,
            workspace: workspace.to_path_buf(),
            primary: mount.primary(),
        },
    ))
}
